use super::*;
use crate::data_render::MaterialGpu;
use crate::decoration::scaled_rich_text;
use crate::decoration::{DecorationMap, DecorationPainter};
use crate::raster::{Canvas, Paint};
use crate::text_render::{FontPolicy, measure_plain_text, measure_rich_text};
use model::{DecorationId, MeasureText, TextExtents};
use std::f32::consts::TAU;

pub(super) struct Layout {
    pub sectors: Vec<Sector>,
    pub groups: [[f32; 4]; 2],
    pub holes: [f32; 2],
    pub annotations: Vec<u8>,
    pub decorations: DecorationMap,
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
    let mut decorations = DecorationPainter::new(&chart.decorations, size, scale, annotate);
    let mut out = Layout {
        sectors: Vec::new(),
        groups: [[0.0; 4]; 2],
        holes: [0.0; 2],
        annotations: Vec::new(),
        decorations: DecorationPainter::new(&chart.decorations, size, scale, false).finish(),
        targets: Vec::new(),
        space: crate::annotation::PixelSpace {
            panel: crate::layout::RectF {
                x: 0.,
                y: 0.,
                width: w,
                height: h,
            },
            plot: crate::layout::RectF {
                x: 0.,
                y: 0.,
                width: w,
                height: h,
            },
            x: crate::annotation::AxisSpace::NONE,
            y: crate::annotation::AxisSpace::NONE,
        },
    };
    let title = chart.resolved_title();
    let measure = FrameMeasure { scale };
    let band = title_band(&title, &measure);
    let top = band;
    let bottom = h - 24.0;
    let automatic_area = crate::layout::RectF {
        x: 24.,
        y: top,
        width: w - 48.,
        height: bottom - top,
    };
    let area = chart
        .decorations
        .resolve_data_area(automatic_area)
        .map_err(RadialError::Invalid)?;
    decorations.add(DecorationId::DataArea, automatic_area);
    out.space.plot = area;
    let (top, bottom) = (area.y, area.y + area.height);

    let canvas_rect = crate::layout::RectF {
        x: 0.,
        y: 0.,
        width: w,
        height: h,
    };
    // Centred in its band across the canvas; a title wider than the canvas
    // is clipped by it, and its hit region is what the canvas shows.
    model::select::emit_chart_title_in(
        &title,
        canvas_rect,
        band,
        &measure,
        &mut decorations.on(DecorationId::ChartTitle, &mut canvas),
    );
    decorations.clip_hit(&DecorationId::ChartTitle, canvas_rect);
    let mut groups = vec![(
        &chart.slices,
        chart.kind,
        chart.start_angle_degrees.rem_euclid(360.0).to_radians(),
    )];
    if let Some(split) = &chart.split {
        groups.push((&split.children, split.kind, -std::f32::consts::FRAC_PI_2));
    }
    for (g, (slices, kind, start)) in groups.into_iter().enumerate() {
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
        let tilt = chart.style.tilt_degrees.to_radians();
        let cs = tilt.cos();
        let sn = tilt.sin();
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
        let radius = ((right - left - 2.0 * margin) / (2.0 * (1.0 + max_explode)))
            .min((bottom - top) / extent);
        let radius = if g == 1 { radius * 0.76 } else { radius };
        if radius < 32.0 {
            return Err(RadialError::Invalid("radial data area is too small"));
        }
        let center = [
            (left + right) * 0.5,
            (top + bottom) * 0.5 + chart.style.depth * sn * radius * 0.5,
        ];
        out.groups[g] = [center[0] * scale, center[1] * scale, radius * scale, cs];
        out.holes[g] = kind.inner_radius();
        let mut cumulative = 0.0f64;
        let first = out.sectors.len();
        for (index, slice) in slices.iter().enumerate() {
            if slice.value == 0.0 {
                continue;
            }
            let a = start + (cumulative / total) as f32 * TAU;
            cumulative += slice.value;
            // Derive both endpoints from the same cumulative sum; no accumulated
            // f32 wedge increments and no final-slice rounding hole.
            let b = start + (cumulative / total) as f32 * TAU;
            let style = slice.style.as_ref().unwrap_or(&chart.style);
            let target = RadialTarget { group: g, index };
            let amount = chart.interaction.amount(target);
            let radius_px = radius * scale;
            let tilt = style.tilt_degrees.to_radians();
            let vertical_lift = style.hover_lift * amount;
            // How far the lifted top face moves up the screen, in physical
            // pixels. Computed once here; the inside-label code below, the
            // shader's moving inside glyphs and their hit map all read it from
            // `effects[3]`.
            let label_lift = vertical_lift * tilt.sin() * radius_px;
            out.targets.push(target);
            out.sectors.push(Sector {
                geometry: [
                    center[0] * scale,
                    center[1] * scale,
                    radius_px,
                    kind.inner_radius(),
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
            });
        }
        let sectors = &out.sectors[first..];
        let positive: Vec<_> = slices.iter().filter(|s| s.value > 0.0).collect();
        {
            for ((s, slice), target) in sectors.iter().zip(&positive).zip(&out.targets[first..]) {
                if slice.labels.unwrap_or(chart.labels) != RadialLabels::Inside {
                    continue;
                }
                let mid = s.arc[0] + s.arc[1] * 0.5;
                let r = (s.geometry[3] + 1.0) * 0.5;
                let (x, mut y) = projected(s, mid, r.max(0.6));
                let lift = s.effects[3];
                if raster_moves_inside_labels(chart) {
                    y -= lift;
                }
                let fs = chart.font_size * scale;
                let line_count = text_lines(slice).count();
                let text_width = text_lines(slice)
                    .map(|s| width(chart, s, fs))
                    .fold(0.0, f32::max);
                // Keep inside glyphs anchored to their slice, so the cached
                // hover translation and measured hit map still agree. Fit the
                // text to the available face instead of rejecting the chart.
                let available = (s.arc[1] * r * s.geometry[2] * s.arc[2].cos())
                    .min(s.geometry[2] * (1.0 - s.geometry[3]));
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
                for (i, line) in text_lines(slice).enumerate() {
                    decorations
                        .on(DecorationId::RadialLabel(*target), &mut canvas)
                        .plain_text(
                            &chart.font_family,
                            line,
                            (x - width(chart, line, fs) * 0.5) / scale,
                            (y + (i as f32 * 1.2 - (line_count - 1) as f32 * 0.6 + 0.45) * fs)
                                / scale,
                            fs / scale,
                            color,
                        );
                }
                if !raster_moves_inside_labels(chart) {
                    decorations.shift_hit(&DecorationId::RadialLabel(*target), 0., -lift / scale);
                }
            }
        }
        {
            for left_side in [true, false] {
                let mut labels: Vec<_> = sectors
                    .iter()
                    .zip(&positive)
                    .zip(&out.targets[first..])
                    .filter(|((_, slice), _)| outside(slice))
                    .filter(|((s, _), _)| ((s.arc[0] + s.arc[1] * 0.5).cos() < 0.0) == left_side)
                    .map(|((s, slice), target)| {
                        let mid = s.arc[0] + s.arc[1] * 0.5;
                        let mut anchor = projected(s, mid, 1.015);
                        // The front silhouette is the bottom of the extruded wall,
                        // not the top rim: leaders must not cross the visible face.
                        if mid.sin() > 0.0 {
                            anchor.1 += s.arc[3] * s.arc[2].sin() * s.geometry[2];
                        }
                        (anchor, slice, *target)
                    })
                    .collect();
                labels.sort_by(|a, b| a.0.1.total_cmp(&b.0.1));
                let intervals = labels.len().saturating_sub(1) as f32;
                let fs = (chart.font_size * scale)
                    .min((bottom - top) * scale / (2.4 + 2.7 * intervals))
                    .max(scale);
                let min = top * scale + fs;
                let max = (bottom * scale - fs * 1.4).max(min);
                let step = (fs * 2.7).min((max - min) / intervals.max(1.));
                let mut ys = Vec::new();
                let mut prev = min - step;
                for (anchor, _, _) in &labels {
                    let y = anchor.1.clamp(min, max).max(prev + step);
                    ys.push(y);
                    prev = y;
                }
                if let Some(last) = ys.last().copied() {
                    let shift = (last - max).max(0.0);
                    for y in &mut ys {
                        *y -= shift;
                    }
                }
                let elbow = (center[0]
                    + if left_side {
                        -radius * (1.0 + max_explode) - 12.0
                    } else {
                        radius * (1.0 + max_explode) + 12.0
                    })
                    * scale;
                let rule = Paint::stroke(&crate::Color::new(0.48, 0.53, 0.6, 0.75), scale);
                for ((anchor, slice, target), y) in labels.into_iter().zip(ys) {
                    let tx = elbow + if left_side { -8.0 * scale } else { 8.0 * scale };
                    let offset = decorations
                        .offset(&DecorationId::RadialLabel(target))
                        .map(|v| v * scale);
                    if annotate {
                        let elbow = (elbow + offset[0], y + offset[1]);
                        canvas.draw_line(anchor, elbow, &rule);
                        canvas.draw_line(elbow, (tx + offset[0], y + offset[1]), &rule);
                    }
                    for (i, line) in text_lines(slice).enumerate() {
                        let x = if left_side {
                            tx - width(chart, line, fs)
                        } else {
                            tx
                        };
                        decorations
                            .on(DecorationId::RadialLabel(target), &mut canvas)
                            .plain_text(
                                &chart.font_family,
                                line,
                                x / scale,
                                (y + (i as f32 * 1.2 - 0.15) * fs) / scale,
                                fs / scale,
                                slice.label_color.unwrap_or(chart.label_color),
                            );
                    }
                }
            }
        }
    }
    if let Some(split) = &chart.split {
        let parent = chart.slices[..split.slice_index]
            .iter()
            .filter(|s| s.value > 0.0)
            .count();
        let sector = &out.sectors[parent];
        let other = out.groups[1];
        let paint = Paint::stroke(&crate::Color::new(0.55, 0.58, 0.62, 0.65), scale);
        for (a, dy) in [(sector.arc[0], -0.8), (sector.arc[0] + sector.arc[1], 0.8)] {
            canvas.draw_line(
                projected(sector, a, 1.025),
                (
                    other[0] - other[2] * 0.6,
                    other[1] + other[2] * other[3] * dy,
                ),
                &paint,
            );
        }
    }
    out.decorations = decorations.finish();
    if annotate {
        out.decorations.draw_selection(&mut canvas, scale);
        out.annotations = canvas.into_rgba();
    }
    Ok(out)
}

#[cfg(test)]
mod title_tests {
    use super::*;
    use model::{Color, DecorationId};
    fn bounds(map: &DecorationMap, title: bool) -> Option<crate::layout::RectF> {
        map.entries()
            .find(|(_, e)| {
                if title {
                    e.id == DecorationId::ChartTitle
                } else {
                    matches!(e.id, DecorationId::RadialLabel(_))
                }
            })
            .and_then(|(id, _)| map.bounds(id))
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
            let tb = bounds(&original.decorations, true).unwrap();
            let lb = bounds(&original.decorations, false).unwrap();
            let title_ink = crop(&original.annotations, tb, scale);
            assert!(title_ink.chunks_exact(4).any(|p| p[3] != 0));
            let saved = c.clone();
            c.font_size *= 1.25;
            c.label_color = Color::new(0.8, 0.1, 0.1, 1.);
            let labels = layout(&c, (800, 520), scale, true).unwrap();
            assert_eq!(bounds(&labels.decorations, true), Some(tb));
            assert_eq!(
                crop(&labels.annotations, tb, scale),
                title_ink,
                "label edits changed title pixels"
            );
            assert!(bounds(&labels.decorations, false).unwrap().height > lb.height);
            c = saved.clone();
            c.chart_title.as_mut().unwrap().text.font_size += 6.;
            let sized = layout(&c, (800, 520), scale, true).unwrap();
            assert!(bounds(&sized.decorations, true).unwrap().height > tb.height);
            let label = bounds(&sized.decorations, false).unwrap();
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
            assert!(
                bounds(
                    &layout(&c, (800, 520), scale, true).unwrap().decorations,
                    true
                )
                .is_none()
            );
            c = saved;
        }
    }
}
