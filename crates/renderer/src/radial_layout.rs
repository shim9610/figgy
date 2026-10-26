//! The radial frame's layout: its sectors, and the one placement of each of
//! its decorations (the title, the plot area and every slice label with its
//! leader line), [`emit_decoration`], which the label raster draws with and
//! the frame's `HitMap<RadialChart>` answers hits, selection boxes and drag
//! containment from.
use super::*;
use crate::data_render::MaterialGpu;
use crate::decoration::scaled_rich_text;
use crate::decoration_render::CanvasSink;
use crate::raster::Canvas;
use crate::sketch::DecoStroker;
use crate::text_render::{FontPolicy, measure_plain_text, measure_rich_text};
use model::layout::{NudgeReject, NudgeResult, RectF};
use model::resize::{Resizable, ResizeHandle};
use model::select::{Selectable, SelectionGeometry};
use model::sink::{DrawnPart, Parts, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use model::{Color, DecorationElement, DecorationId, Draggable, HitMap, MeasureText, TextExtents};
use std::f32::consts::TAU;

pub(super) struct Layout {
    pub sectors: Vec<Sector>,
    pub groups: [[f32; 4]; 2],
    pub holes: [f32; 2],
    pub annotations: Vec<u8>,
    pub decorations: HitMap<RadialChart>,
    pub targets: Vec<RadialTarget>,
    /// Anchor space for the annotation layer: the plot area with no value
    /// axes, so only ratio anchors can be placed on a radial chart.
    pub space: crate::annotation::PixelSpace,
}
/// The radial layout's text measure: a text measured at the frame's scale
/// and expressed in logical pixels, as the label raster draws it.
struct FrameMeasure {
    scale: f32,
}
impl MeasureText for FrameMeasure {
    fn measure_rich(&self, rt: &model::text::RichText) -> TextExtents {
        let m = measure_rich_text(&scaled_rich_text(rt, self.scale), FontPolicy::Standard);
        TextExtents {
            width: m.width / self.scale,
            ascent: m.ascent / self.scale,
            descent: m.descent / self.scale,
        }
    }
}
/// The height of the radial title band: the room a default title needs
/// above the circles (70 logical pixels), grown for a taller title, or the
/// plain 24 pixel margin without a title.
fn title_band(title: &model::config::ChartTitleOptions, measure: &FrameMeasure) -> f32 {
    if !title.visible || title.text.segments.is_empty() {
        return 24.;
    }
    let m = measure.measure_rich(&title.text);
    let baseline = 42f32.max(m.ascent + 12.);
    70f32.max(baseline + m.descent + 20.)
}
fn width(c: &RadialChart, text: &str, size: f32) -> f32 {
    measure_plain_text(
        text,
        &c.font_family,
        size,
        false,
        false,
        FontPolicy::Standard,
    )
    .width
}
/// The drawn lines of a slice's label: its SSOT text split at its line
/// breaks. The layout draws these strings and formats nothing.
fn text_lines(slice: &RadialSlice) -> std::str::Split<'_, char> {
    slice.text.as_deref().unwrap_or_default().split('\n')
}
fn projected(s: &Sector, angle: f32, r: f32) -> (f32, f32) {
    let mid = s.arc[0] + s.arc[1] * 0.5;
    let cs = s.arc[2].cos();
    let sn = s.arc[2].sin();
    (
        s.geometry[0] + s.geometry[2] * (r * angle.cos() + s.detail[0] * mid.cos()),
        s.geometry[1]
            + s.geometry[2] * ((r * angle.sin() + s.detail[0] * mid.sin()) * cs - s.arc[3] * sn),
    )
}

/// The frame a radial chart is laid out in, in logical pixels: the canvas,
/// the title band at its top, the automatic plot area below it and the
/// plot area the chart resolves from it.
#[derive(Clone, Copy)]
struct Frame {
    canvas: RectF,
    scale: f32,
    band: f32,
    automatic: RectF,
    area: RectF,
}
fn frame(chart: &RadialChart, size: (u32, u32), scale: f32) -> Result<Frame, RadialError> {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let band = title_band(&chart.resolved_title(), &FrameMeasure { scale });
    let bottom = h - 24.0;
    let automatic = RectF {
        x: 24.,
        y: band,
        width: w - 48.,
        height: bottom - band,
    };
    let area = chart
        .decorations
        .resolve_data_area(automatic)
        .map_err(RadialError::Invalid)?;
    Ok(Frame {
        canvas: RectF {
            x: 0.,
            y: 0.,
            width: w,
            height: h,
        },
        scale,
        band,
        automatic,
        area,
    })
}

/// Where one circle of the chart sits: its slices, shape, start angle and
/// total, and its centre and radius in logical pixels. Group 0 is the main
/// circle, group 1 the detail circle of a split.
struct Circle<'a> {
    slices: &'a [RadialSlice],
    kind: RadialKind,
    start: f32,
    total: f64,
    center: [f32; 2],
    radius: f32,
    max_explode: f32,
}
fn circle<'a>(chart: &'a RadialChart, area: RectF, g: usize) -> Result<Circle<'a>, RadialError> {
    let (slices, kind, start) = match (g, &chart.split) {
        (0, _) => (
            chart.slices.as_slice(),
            chart.kind,
            chart.start_angle_degrees.rem_euclid(360.0).to_radians(),
        ),
        (_, Some(split)) => (
            split.children.as_slice(),
            split.kind,
            -std::f32::consts::FRAC_PI_2,
        ),
        (_, None) => return Err(RadialError::Invalid("radial chart has no detail circle")),
    };
    let (top, bottom) = (area.y, area.y + area.height);
    let total = slices.iter().map(|s| s.value).sum::<f64>();
    let (left, right) = if chart.split.is_none() {
        (area.x, area.x + area.width)
    } else if g == 0 {
        (area.x, area.x + (area.width + 48.) * 0.6 - 48.)
    } else {
        (area.x + (area.width + 48.) * 0.6, area.x + area.width)
    };
    let max_explode = slices
        .iter()
        .filter(|s| s.value > 0.0)
        .map(|s| s.explode)
        .fold(0.0, f32::max);
    // The automatic area already reserves 24 logical pixels at both
    // outer edges (and either side of the split). Keep the original
    // default geometry while leaving room for all eight resize handles.
    let mut margin = 0.0;
    let outside =
        |slice: &RadialSlice| slice.labels.unwrap_or(chart.labels) == RadialLabels::Outside;
    if slices.iter().any(outside) {
        let max_label = slices
            .iter()
            .filter(|s| s.value > 0.0 && outside(s))
            .flat_map(|s| text_lines(s))
            .map(|line| width(chart, line, chart.font_size))
            .fold(0.0, f32::max);
        margin = max_label + 12.0;
    }
    let sn = chart.style.tilt_degrees.to_radians().sin();
    // Reserve the maximum lift even at rest: interaction must not resize the chart.
    let max_lift = slices
        .iter()
        .map(|s| s.style.as_ref().unwrap_or(&chart.style).hover_lift)
        .fold(0.0, f32::max);
    let max_depth = slices
        .iter()
        .map(|s| s.style.as_ref().unwrap_or(&chart.style).depth)
        .fold(chart.style.depth, f32::max);
    let shadow_margin = if slices
        .iter()
        .any(|s| s.style.as_ref().unwrap_or(&chart.style).shadow)
    {
        0.55
    } else {
        0.25
    };
    let extent = slices
        .iter()
        .filter(|s| s.value > 0.0)
        .map(|s| {
            let style = s.style.as_ref().unwrap_or(&chart.style);
            let tilt = style.tilt_degrees.to_radians();
            2.0 * tilt.cos() * (1.0 + max_explode)
                + (max_depth + max_lift) * tilt.sin()
                + shadow_margin
        })
        .fold(0.0, f32::max);
    // Labels cannot consume the entire chart. Keep the existing geometry
    // when it fits, and cap their reserved margin on a crowded canvas.
    margin = margin.min(((right - left) * 0.5 - 40.0 * (1.0 + max_explode)).max(0.));
    let radius =
        ((right - left - 2.0 * margin) / (2.0 * (1.0 + max_explode))).min((bottom - top) / extent);
    let radius = if g == 1 { radius * 0.76 } else { radius };
    if radius < 32.0 {
        return Err(RadialError::Invalid("radial data area is too small"));
    }
    let center = [
        (left + right) * 0.5,
        (top + bottom) * 0.5 + chart.style.depth * sn * radius * 0.5,
    ];
    Ok(Circle {
        slices,
        kind,
        start,
        total,
        center,
        radius,
        max_explode,
    })
}

/// The sector of slice `index` of circle `g`, in physical pixels, or `None`
/// for a slice without a value. Both of its angles come from the same
/// cumulative sum, so no f32 wedge increments accumulate and the last slice
/// leaves no rounding hole.
fn sector(
    chart: &RadialChart,
    c: &Circle<'_>,
    g: usize,
    index: usize,
    scale: f32,
) -> Option<Sector> {
    let slice = &c.slices[index];
    if slice.value == 0.0 {
        return None;
    }
    let before = c.slices[..index].iter().map(|s| s.value).sum::<f64>();
    let a = c.start + (before / c.total) as f32 * TAU;
    let b = c.start + ((before + slice.value) / c.total) as f32 * TAU;
    let style = slice.style.as_ref().unwrap_or(&chart.style);
    let target = RadialTarget { group: g, index };
    let amount = chart.interaction.amount(target);
    let radius_px = c.radius * scale;
    let tilt = style.tilt_degrees.to_radians();
    let vertical_lift = style.hover_lift * amount;
    // How far the lifted top face moves up the screen, in physical pixels.
    // Computed once here; the inside labels, the shader's moving inside
    // glyphs and their hit regions all read it from `effects[3]`.
    let label_lift = vertical_lift * tilt.sin() * radius_px;
    Some(Sector {
        geometry: [
            c.center[0] * scale,
            c.center[1] * scale,
            radius_px,
            c.kind.inner_radius(),
        ],
        arc: [a, b - a, tilt, style.depth],
        color: [slice.color.r, slice.color.g, slice.color.b, 1.0],
        material: MaterialGpu::pack(
            style.material,
            style.roughness,
            style.texture_strength,
            style.texture_scale,
            style.gloss,
            index as f32 * 13.17,
            style.texture_angle_degrees.rem_euclid(360.0).to_radians(),
            amount * style.hover_brightness,
        ),
        rounding: [style.inner_corner, style.outer_corner, vertical_lift, 0.0],
        outline: [
            if style.outline.rim { 1.0 } else { 0.0 },
            if style.outline.separators { 1.0 } else { 0.0 },
            if style.outline.emphasis { amount } else { 0.0 },
            style.outline.width * scale,
        ],
        outline_color: [
            style.outline.color.r,
            style.outline.color.g,
            style.outline.color.b,
            style.outline.color.a,
        ],
        light: [
            style.light[0],
            style.light[1],
            style.light[2],
            if style.shadow { 1.0 } else { 0.0 },
        ],
        effects: [
            0.0,
            0.0,
            if slice.labels.unwrap_or(chart.labels) == RadialLabels::Inside
                && !raster_moves_inside_labels(chart)
            {
                1.0
            } else {
                0.0
            },
            label_lift,
        ],
        detail: [
            slice.explode,
            style.gap_degrees.to_radians().min((b - a) * 0.2),
            style.bevel,
            0.0,
        ],
    })
}

/// One line of plain label text, `size` logical pixels high, with its
/// baseline origin at `(x, y)`, measured once at the frame's scale.
#[allow(clippy::too_many_arguments)]
fn plain_line(
    sink: &mut dyn Sink,
    family: &str,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    color: Color,
    scale: f32,
) {
    if text.is_empty() || color.a == 0. {
        return;
    }
    let m = measure_plain_text(
        text,
        family,
        size * scale,
        false,
        false,
        FontPolicy::Standard,
    );
    sink.text(
        Slot::Above,
        &PlacedText {
            text: TextRef::Plain {
                text,
                family,
                size,
                color,
            },
            origin: (x, y),
            extents: TextExtents {
                width: m.width / scale,
                ascent: m.ascent / scale,
                descent: m.descent / scale,
            },
            rotation: None,
        },
    );
}

/// The inside label of slice `index` of circle `g`: its lines on the
/// slice's face, at the face's lifted place, or at its unlifted place when
/// `at_rest` (the raster while the shader lifts the inside glyphs).
fn emit_inside(
    chart: &RadialChart,
    f: &Frame,
    c: &Circle<'_>,
    g: usize,
    index: usize,
    at_rest: bool,
    sink: &mut dyn Sink,
) {
    let scale = f.scale;
    let Some(s) = sector(chart, c, g, index, scale) else {
        return;
    };
    let slice = &c.slices[index];
    let mid = s.arc[0] + s.arc[1] * 0.5;
    let r = (s.geometry[3] + 1.0) * 0.5;
    let (x, mut y) = projected(&s, mid, r.max(0.6));
    if !at_rest {
        y -= s.effects[3];
    }
    let fs = chart.font_size * scale;
    let line_count = text_lines(slice).count();
    let text_width = text_lines(slice)
        .map(|s| width(chart, s, fs))
        .fold(0.0, f32::max);
    // Keep inside glyphs anchored to their slice, so the cached hover
    // translation and the hit region still agree. Fit the text to the
    // available face instead of rejecting the chart.
    let available =
        (s.arc[1] * r * s.geometry[2] * s.arc[2].cos()).min(s.geometry[2] * (1.0 - s.geometry[3]));
    let fit = (available * 1.05 / text_width.max(1.))
        .min(available / (fs * line_count as f32 * 1.2))
        .min(1.);
    let fs = (fs * fit).max(scale);
    let luminance = s.color[0] * 0.2126 + s.color[1] * 0.7152 + s.color[2] * 0.0722;
    let color = slice.label_color.unwrap_or(if luminance < 0.5 {
        crate::Color::WHITE
    } else {
        chart.label_color
    });
    let offset = chart
        .decorations
        .effective_offset(&DecorationId::RadialLabel(RadialTarget { group: g, index }));
    for (i, line) in text_lines(slice).enumerate() {
        plain_line(
            sink,
            &chart.font_family,
            line,
            (x - width(chart, line, fs) * 0.5) / scale + offset[0],
            (y + (i as f32 * 1.2 - (line_count - 1) as f32 * 0.6 + 0.45) * fs) / scale + offset[1],
            fs / scale,
            color,
            scale,
        );
    }
}

/// The outside labels of circle `g` on one side, in stacking order: each
/// label's leader anchor (physical pixels) and slice index, sorted by the
/// anchor's height, ties in slice order. At most [`MAX_RADIAL_SLICES`].
fn side_labels(
    chart: &RadialChart,
    c: &Circle<'_>,
    g: usize,
    left_side: bool,
    scale: f32,
) -> ([((f32, f32), usize); MAX_RADIAL_SLICES], usize) {
    let mut labels = [((0.0, 0.0), 0); MAX_RADIAL_SLICES];
    let mut n = 0;
    for (index, slice) in c.slices.iter().enumerate() {
        if slice.labels.unwrap_or(chart.labels) != RadialLabels::Outside {
            continue;
        }
        let Some(s) = sector(chart, c, g, index, scale) else {
            continue;
        };
        let mid = s.arc[0] + s.arc[1] * 0.5;
        if (mid.cos() < 0.0) != left_side {
            continue;
        }
        let mut anchor = projected(&s, mid, 1.015);
        // The front silhouette is the bottom of the extruded wall, not the
        // top rim: leaders must not cross the visible face.
        if mid.sin() > 0.0 {
            anchor.1 += s.arc[3] * s.arc[2].sin() * s.geometry[2];
        }
        labels[n] = (anchor, index);
        n += 1;
    }
    labels[..n].sort_unstable_by(|a, b| a.0.1.total_cmp(&b.0.1).then(a.1.cmp(&b.1)));
    (labels, n)
}

/// The colour of the leader lines.
const LEADER: Color = Color {
    r: 0.48,
    g: 0.53,
    b: 0.6,
    a: 0.75,
};

/// The outside labels of circle `g` on one side, each its leader line
/// (from the slice's rim to an elbow beside the circle, then to the text)
/// and its lines, stacked so they keep apart: all of them, or only slice
/// `only`'s. The leader starts at the slice and follows the label's offset
/// from its elbow on: it is a part of the label.
fn emit_outside(
    chart: &RadialChart,
    f: &Frame,
    c: &Circle<'_>,
    g: usize,
    left_side: bool,
    only: Option<usize>,
    sink: &mut dyn Sink,
) {
    let scale = f.scale;
    let (labels, n) = side_labels(chart, c, g, left_side, scale);
    let labels = &labels[..n];
    let (top, bottom) = (f.area.y, f.area.y + f.area.height);
    let intervals = n.saturating_sub(1) as f32;
    let fs = (chart.font_size * scale)
        .min((bottom - top) * scale / (2.4 + 2.7 * intervals))
        .max(scale);
    let min = top * scale + fs;
    let max = (bottom * scale - fs * 1.4).max(min);
    let step = (fs * 2.7).min((max - min) / intervals.max(1.));
    let mut ys = [0.0f32; MAX_RADIAL_SLICES];
    let mut prev = min - step;
    for ((anchor, _), y) in labels.iter().zip(&mut ys) {
        *y = anchor.1.clamp(min, max).max(prev + step);
        prev = *y;
    }
    if let Some(last) = ys[..n].last().copied() {
        let shift = (last - max).max(0.0);
        for y in &mut ys[..n] {
            *y -= shift;
        }
    }
    let elbow = (c.center[0]
        + if left_side {
            -c.radius * (1.0 + c.max_explode) - 12.0
        } else {
            c.radius * (1.0 + c.max_explode) + 12.0
        })
        * scale;
    let leader = StrokeStyle {
        width: 1.0,
        color: LEADER,
        dash: &[],
        tag: "radial_leader",
    };
    let logical = |x: f32, y: f32| [x / scale, y / scale];
    for ((anchor, index), y) in labels.iter().zip(ys) {
        if only.is_some_and(|only| only != *index) {
            continue;
        }
        let slice = &c.slices[*index];
        let target = RadialTarget {
            group: g,
            index: *index,
        };
        let tx = elbow + if left_side { -8.0 * scale } else { 8.0 * scale };
        let offset = chart
            .decorations
            .effective_offset(&DecorationId::RadialLabel(target));
        let (ox, oy) = (offset[0] * scale, offset[1] * scale);
        let elbow = logical(elbow + ox, y + oy);
        sink.stroke(Slot::Above, logical(anchor.0, anchor.1), elbow, &leader);
        sink.stroke(Slot::Above, elbow, logical(tx + ox, y + oy), &leader);
        for (i, line) in text_lines(slice).enumerate() {
            let x = if left_side {
                tx - width(chart, line, fs)
            } else {
                tx
            };
            plain_line(
                sink,
                &chart.font_family,
                line,
                x / scale + offset[0],
                (y + (i as f32 * 1.2 - 0.15) * fs) / scale + offset[1],
                fs / scale,
                slice.label_color.unwrap_or(chart.label_color),
                scale,
            );
        }
    }
}

/// The title, centred in its band across the canvas and moved by its
/// offset in `decorations`. A title wider than the canvas runs off it; the
/// canvas clips it.
fn emit_title(chart: &RadialChart, f: &Frame, sink: &mut dyn Sink) {
    let mut title = chart.resolved_title();
    let offset = chart
        .decorations
        .effective_offset(&DecorationId::ChartTitle);
    (title.offset_x, title.offset_y) = (offset[0], offset[1]);
    model::select::emit_chart_title_in(
        &title,
        f.canvas,
        f.band,
        &FrameMeasure { scale: f.scale },
        sink,
    );
}

/// The one placement of decoration `id` of `chart` on a canvas of logical
/// `size` at `scale`, in logical pixels: the plot area (a rectangle that
/// paints nothing), the title, or a slice label with its leader line. The
/// label raster draws with it and the frame's hit map answers from it.
/// Returns `false` when the chart cannot be laid out at that size.
pub(super) fn emit_decoration(
    chart: &RadialChart,
    size: (u32, u32),
    scale: f32,
    id: &DecorationId,
    sink: &mut dyn Sink,
) -> bool {
    let Ok(f) = frame(chart, size, scale) else {
        return false;
    };
    match id {
        DecorationId::DataArea => sink.rect(Slot::Above, f.area, None, None),
        DecorationId::ChartTitle => emit_title(chart, &f, sink),
        DecorationId::RadialLabel(t) => {
            let Ok(c) = circle(chart, f.area, t.group) else {
                return false;
            };
            let Some(slice) = c.slices.get(t.index) else {
                return true;
            };
            match slice.labels.unwrap_or(chart.labels) {
                RadialLabels::Inside => emit_inside(chart, &f, &c, t.group, t.index, false, sink),
                RadialLabels::Outside => {
                    let Some(s) = sector(chart, &c, t.group, t.index, scale) else {
                        return true;
                    };
                    let left = (s.arc[0] + s.arc[1] * 0.5).cos() < 0.0;
                    emit_outside(chart, &f, &c, t.group, left, Some(t.index), sink);
                }
                RadialLabels::None => {}
            }
        }
        _ => {}
    }
    true
}

/// One decoration of a radial frame in its `HitMap<RadialChart>`: the plot
/// area, the title or a slice label. It carries the frame's logical size
/// and scale; its parts are placed from the chart by [`emit_decoration`] on
/// every query, so its hit region, selection box and drag follow what is
/// drawn. It lays itself out with the renderer's own text measure, so the
/// `measure` the traits pass is not read.
#[derive(Clone, Debug)]
pub(crate) struct RadialDecoration {
    id: DecorationId,
    size: (u32, u32),
    scale: f32,
    /// The automatic plot area of the frame: the region the plot area
    /// resolves, moves and resizes in.
    automatic: RectF,
}
impl RadialDecoration {
    /// The element of the decorations' drag policy: the decoration with its
    /// drawn bounds before its offset. Dragging moves its text (the leader
    /// line keeps its start on the slice), so a move is contained on the
    /// text it draws, intersected with the canvas.
    fn element(&self, chart: &RadialChart) -> Option<DecorationElement> {
        let base = match self.id {
            DecorationId::DataArea => self.automatic,
            _ => {
                let mut text: Option<RectF> = None;
                emit_decoration(
                    chart,
                    self.size,
                    self.scale,
                    &self.id,
                    &mut Parts(&mut |part| {
                        if let DrawnPart::Text(r) = part {
                            text = Some(text.map_or(r, |u| model::sink::union(u, r)));
                        }
                    }),
                );
                let shown = intersection(
                    text?,
                    RectF {
                        x: 0.,
                        y: 0.,
                        width: self.size.0 as f32,
                        height: self.size.1 as f32,
                    },
                )?;
                let offset = chart.decorations.effective_offset(&self.id);
                shown.translated(-offset[0], -offset[1])
            }
        };
        Some(DecorationElement {
            id: self.id.clone(),
            base_bounds: base,
            canvas_size: self.size,
        })
    }
}
/// The overlap of two rectangles, `None` when they do not overlap.
fn intersection(a: RectF, b: RectF) -> Option<RectF> {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    (x1 > x0 && y1 > y0).then_some(RectF {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}
impl Selectable<RadialChart> for RadialDecoration {
    fn emit(&self, chart: &RadialChart, _: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        emit_decoration(chart, self.size, self.scale, &self.id, sink)
    }
    /// An outside label is its leader line and its text: it is outlined,
    /// and hit, part by part.
    fn outlines_parts(&self, chart: &RadialChart) -> bool {
        match &self.id {
            DecorationId::RadialLabel(t) => chart
                .slice(*t)
                .is_some_and(|s| s.labels.unwrap_or(chart.labels) == RadialLabels::Outside),
            _ => false,
        }
    }
    fn as_draggable(&self) -> Option<&dyn Draggable<RadialChart>> {
        Some(self)
    }
    fn as_resizable(&self) -> Option<&dyn Resizable<RadialChart>> {
        (self.id == DecorationId::DataArea).then_some(self)
    }
    fn element_id(&self) -> String {
        self.id.element_id()
    }
}
impl Draggable<RadialChart> for RadialDecoration {
    /// The element without measured bounds: [`Self::drag_by`] measures them
    /// on the chart it moves.
    fn nudge_element(&self) -> DecorationElement {
        DecorationElement {
            id: self.id.clone(),
            base_bounds: self.automatic,
            canvas_size: self.size,
        }
    }
    fn drag_by(
        &self,
        chart: &mut RadialChart,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        match self.element(chart) {
            Some(element) => model::drag::DragConfig::nudge(chart, element, measure, dx, dy),
            None => NudgeResult::Rejected(NudgeReject::OutOfBounds),
        }
    }
}
impl Resizable<RadialChart> for RadialDecoration {
    fn resize_targets(
        &self,
        handle: ResizeHandle,
    ) -> (Option<DecorationElement>, Option<DecorationElement>) {
        self.nudge_element().resize_targets(handle)
    }
    /// Both edges move together or not at all, as on a Cartesian chart.
    fn resize_by(
        &self,
        chart: &mut RadialChart,
        measure: &dyn MeasureText,
        handle: ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        self.nudge_element()
            .resize_by(&mut chart.decorations, measure, handle, dx, dy)
    }
}

/// The colour of the split connectors.
const CONNECTOR: Color = Color {
    r: 0.55,
    g: 0.58,
    b: 0.62,
    a: 0.65,
};

/// Chrome, no element: the two connectors from the split slice of a split
/// chart to its detail circle `other` (centre, radius and tilt, physical
/// pixels), from the frame's `sectors`. Nothing without a split.
pub(super) fn emit_split_connectors(
    chart: &RadialChart,
    sectors: &[Sector],
    other: [f32; 4],
    scale: f32,
    sink: &mut dyn Sink,
) {
    let Some(split) = &chart.split else {
        return;
    };
    let parent = chart.slices[..split.slice_index]
        .iter()
        .filter(|s| s.value > 0.0)
        .count();
    let Some(sector) = sectors.get(parent) else {
        return;
    };
    let style = StrokeStyle {
        width: 1.0,
        color: CONNECTOR,
        dash: &[],
        tag: "radial_split_connector",
    };
    for (a, dy) in [(sector.arc[0], -0.8), (sector.arc[0] + sector.arc[1], 0.8)] {
        let from = projected(sector, a, 1.025);
        let to = (
            other[0] - other[2] * 0.6,
            other[1] + other[2] * other[3] * dy,
        );
        sink.stroke(
            Slot::Above,
            [from.0 / scale, from.1 / scale],
            [to.0 / scale, to.1 / scale],
            &style,
        );
    }
}

pub(super) fn layout(
    chart: &RadialChart,
    size: (u32, u32),
    scale: f32,
    annotate: bool,
) -> Result<Layout, RadialError> {
    let w = size.0 as f32;
    let h = size.1 as f32;
    let mut canvas = Canvas::new(
        if annotate {
            (w * scale).round() as u32
        } else {
            1
        },
        if annotate {
            (h * scale).round() as u32
        } else {
            1
        },
    )
    .ok_or(RadialError::Invalid("could not allocate label canvas"))?;
    let f = frame(chart, size, scale)?;
    let mut out = Layout {
        sectors: Vec::new(),
        groups: [[0.0; 4]; 2],
        holes: [0.0; 2],
        annotations: Vec::new(),
        decorations: HitMap::for_state(),
        targets: Vec::new(),
        space: crate::annotation::PixelSpace {
            panel: f.canvas,
            plot: f.area,
            x: crate::annotation::AxisSpace::NONE,
            y: crate::annotation::AxisSpace::NONE,
        },
    };
    let element = |id: DecorationId| RadialDecoration {
        id,
        size,
        scale,
        automatic: f.automatic,
    };
    // Back to front: the plot area first, then the title and the labels in
    // the order they are drawn; a later element wins where two overlap.
    let measure = FrameMeasure { scale };
    let register = |map: &mut HitMap<RadialChart>, id: DecorationId| {
        let e = element(id);
        if e.bounds(chart, &measure).is_some() {
            map.register(e);
        }
    };
    register(&mut out.decorations, DecorationId::DataArea);
    register(&mut out.decorations, DecorationId::ChartTitle);
    let mut paint = CanvasSink::placed(
        None,
        Some(&mut canvas),
        DecoStroker::Precise,
        FontPolicy::Standard,
        [0.0, 0.0],
        scale,
    );
    if annotate {
        emit_title(chart, &f, &mut paint);
    }
    // While the shader lifts the inside glyphs, they are rastered at rest.
    let inside_at_rest = !raster_moves_inside_labels(chart);
    for g in 0..if chart.split.is_some() { 2 } else { 1 } {
        let c = circle(chart, f.area, g)?;
        out.groups[g] = [
            c.center[0] * scale,
            c.center[1] * scale,
            c.radius * scale,
            chart.style.tilt_degrees.to_radians().cos(),
        ];
        out.holes[g] = c.kind.inner_radius();
        for index in 0..c.slices.len() {
            if let Some(s) = sector(chart, &c, g, index, scale) {
                out.targets.push(RadialTarget { group: g, index });
                out.sectors.push(s);
            }
        }
        for (index, slice) in c.slices.iter().enumerate() {
            if slice.value > 0.0 && slice.labels.unwrap_or(chart.labels) == RadialLabels::Inside {
                if annotate {
                    emit_inside(chart, &f, &c, g, index, inside_at_rest, &mut paint);
                }
                register(
                    &mut out.decorations,
                    DecorationId::RadialLabel(RadialTarget { group: g, index }),
                );
            }
        }
        for left_side in [true, false] {
            if annotate {
                emit_outside(chart, &f, &c, g, left_side, None, &mut paint);
            }
            let (labels, n) = side_labels(chart, &c, g, left_side, scale);
            for (_, index) in &labels[..n] {
                register(
                    &mut out.decorations,
                    DecorationId::RadialLabel(RadialTarget {
                        group: g,
                        index: *index,
                    }),
                );
            }
        }
    }
    if annotate {
        emit_split_connectors(chart, &out.sectors, out.groups[1], scale, &mut paint);
    }
    drop(paint);
    if annotate {
        // The highlight of the selected decoration is its selection box,
        // from the same parts its hit test answers from.
        if let Some(selected) = &chart.decorations.selected
            && let Some(id) = out.decorations.find(&selected.element_id())
            && let Some(selection) = out.decorations.selection_box(id, chart, &measure)
        {
            crate::axis_render::draw_selection_box(&mut canvas, &selection, [0.0, 0.0], scale);
        }
        out.annotations = canvas.into_rgba();
    }
    Ok(out)
}

#[cfg(test)]
mod title_tests {
    use super::*;
    use model::{Color, DecorationId};
    /// The text the title or the first label draws, at its place in `l`'s
    /// frame: the union of its text parts, so a label is measured without
    /// its leader line.
    fn bounds(l: &Layout, c: &RadialChart, title: bool) -> Option<crate::layout::RectF> {
        let id = if title {
            DecorationId::ChartTitle
        } else {
            DecorationId::RadialLabel(RadialTarget::main(0))
        };
        let element = l.decorations.get(l.decorations.find(&id.element_id())?)?;
        let mut text: Option<RectF> = None;
        element.emit(
            c,
            &crate::CpuTextMeasure::default(),
            &mut Parts(&mut |part| {
                if let DrawnPart::Text(r) = part {
                    text = Some(text.map_or(r, |u| model::sink::union(u, r)));
                }
            }),
        );
        text
    }
    fn crop(bytes: &[u8], r: crate::layout::RectF, scale: f32) -> Vec<u8> {
        let width = (800. * scale) as usize;
        let height = (520. * scale) as usize;
        let front = &bytes[bytes.len() - width * height * 4..];
        let x0 = (r.x * scale).floor().max(0.) as usize;
        let x1 = ((r.x + r.width) * scale).ceil().min(width as f32) as usize;
        let y0 = (r.y * scale).floor().max(0.) as usize;
        let y1 = ((r.y + r.height) * scale).ceil().min(height as f32) as usize;
        (y0..y1)
            .flat_map(|y| {
                front[(y * width + x0) * 4..(y * width + x1) * 4]
                    .iter()
                    .copied()
            })
            .collect()
    }
    #[test]
    fn title_and_label_styles_render_independently() {
        let mut c = RadialChart {
            slices: vec![
                RadialSlice::new("A", 30., Color::BLACK),
                RadialSlice::new("B", 70., Color::BLACK),
            ],
            ..Default::default()
        };
        c.title = "Independent title".into();
        c.format_labels();
        for scale in [1., 2.] {
            let legacy = layout(&c, (800, 520), scale, true).unwrap();
            c.normalize_title();
            let original = layout(&c, (800, 520), scale, true).unwrap();
            assert_eq!(
                legacy.annotations, original.annotations,
                "normalization must preserve appearance"
            );
            let tb = bounds(&original, &c, true).unwrap();
            let lb = bounds(&original, &c, false).unwrap();
            let title_ink = crop(&original.annotations, tb, scale);
            assert!(title_ink.chunks_exact(4).any(|p| p[3] != 0));
            let saved = c.clone();
            c.font_size *= 1.25;
            c.label_color = Color::new(0.8, 0.1, 0.1, 1.);
            let labels = layout(&c, (800, 520), scale, true).unwrap();
            assert_eq!(bounds(&labels, &c, true), Some(tb));
            assert_eq!(
                crop(&labels.annotations, tb, scale),
                title_ink,
                "label edits changed title pixels"
            );
            assert!(bounds(&labels, &c, false).unwrap().height > lb.height);
            c = saved.clone();
            c.chart_title.as_mut().unwrap().text.font_size += 6.;
            let sized = layout(&c, (800, 520), scale, true).unwrap();
            assert!(bounds(&sized, &c, true).unwrap().height > tb.height);
            let label = bounds(&sized, &c, false).unwrap();
            assert!(
                (label.height - lb.height).abs() < 0.01 && (label.width - lb.width).abs() < 0.01,
                "title edit resized label glyphs"
            );
            c = saved.clone();
            c.chart_title.as_mut().unwrap().text.color = Color::new(0.1, 0.7, 0.1, 1.);
            let colored = layout(&c, (800, 520), scale, true).unwrap();
            assert_ne!(crop(&colored.annotations, tb, scale), title_ink);
            assert_eq!(
                crop(&colored.annotations, lb, scale),
                crop(&original.annotations, lb, scale),
                "title color changed label pixels"
            );
            c = saved.clone();
            c.chart_title.as_mut().unwrap().text.segments[0].bold = true;
            c.chart_title.as_mut().unwrap().text.segments[1].superscript = true;
            let rich = layout(&c, (800, 520), scale, true).unwrap();
            assert_ne!(
                crop(&rich.annotations, tb, scale),
                title_ink,
                "rich text is actually painted"
            );
            c.chart_title.as_mut().unwrap().visible = false;
            let hidden = layout(&c, (800, 520), scale, true).unwrap();
            assert!(bounds(&hidden, &c, true).is_none());
            c = saved;
        }
    }
}
