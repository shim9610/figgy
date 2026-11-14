//! The grid and axes of a polar chart (`Config.polar`): rings at the radius
//! ticks, spokes at the angle ticks, the outer ring as the angle axis with
//! its labels around it, and the ray at `radial_label_angle_degrees` as the
//! radius axis with its labels beside it. Rings and arcs are polylines on
//! the canvas, with a segment count that follows the radius so an export at
//! a higher scale stays round; a polygon grid (`GridShape::Polygon`) joins
//! each ring at the categories of a categorical angle axis instead. The
//! geometry is `Config::polar_geometry`, the same circle the GPU transform
//! draws against, and the angles are `Config::polar_angle_map`, the same
//! mapping: a categorical angle axis puts its spokes and labels at its
//! categories.

use super::*;
use crate::color::Color;
use crate::line::LineStylePreset;
use crate::polar::{AngleMap, GridShape, PolarGeometry, PolarOptions};
use crate::sink::{PlacedText, Sink, Slot, StrokeStyle, TextRef};

/// Arcs are flattened into chords of about this length in pixels.
const ARC_CHORD_PX: f32 = 3.0;
const MIN_ARC_SEGMENTS: usize = 12;
const MAX_ARC_SEGMENTS: usize = 2048;

/// The screen sweep the angle axis covers: start angle and signed sweep in
/// radians, counter-clockwise positive, y up.
fn sweep(map: &AngleMap) -> (f64, f64) {
    match map.window {
        Some([start, end]) => {
            let a0 = map.screen_angle(start);
            (a0, map.screen_angle(end) - a0)
        }
        None => (
            map.screen_angle(0.0),
            std::f64::consts::TAU * map.direction(),
        ),
    }
}

/// The ring shape of a polar chart: `Some(n)` to join every ring at the `n`
/// categories of its angle axis (a polygon grid), `None` for circles.
fn polygon_corners(polar: &PolarOptions, axis: &AxisOptions) -> Option<usize> {
    (polar.grid_shape() == GridShape::Polygon)
        .then(|| axis.category_count())
        .flatten()
        .filter(|n| *n > 0)
}

/// Place the ring of `radius` over the angle axis: the arc of the sweep
/// (`stroke_arc`), or with `corners` the chords between the category angles
/// `0, 1, ..., n` (the last one back on the first on a full circle, on the
/// sector end in a sector).
#[allow(clippy::too_many_arguments)]
fn stroke_ring(
    sink: &mut dyn Sink,
    slot: Slot,
    geometry: &PolarGeometry,
    map: &AngleMap,
    corners: Option<usize>,
    radius: f32,
    style: &StrokeStyle<'_>,
) {
    let Some(n) = corners else {
        let (start, sweep) = sweep(map);
        stroke_arc(sink, slot, geometry, radius, start, sweep, style);
        return;
    };
    if radius <= 0.0 {
        return;
    }
    let mut previous = point(geometry, map.screen_angle(0.0), radius);
    for k in 1..=n {
        let next = point(geometry, map.screen_angle(k as f64), radius);
        sink.stroke(slot, [previous.0, previous.1], [next.0, next.1], style);
        previous = next;
    }
}

/// A screen point at `angle` radians and `radius` pixels from the centre.
fn point(geometry: &PolarGeometry, angle: f64, radius: f32) -> (f32, f32) {
    (
        geometry.center[0] + angle.cos() as f32 * radius,
        geometry.center[1] - angle.sin() as f32 * radius,
    )
}

/// Place the arc of `radius` over `(start, sweep)` as chords.
#[allow(clippy::too_many_arguments)]
fn stroke_arc(
    sink: &mut dyn Sink,
    slot: Slot,
    geometry: &PolarGeometry,
    radius: f32,
    start: f64,
    sweep: f64,
    style: &StrokeStyle<'_>,
) {
    if radius <= 0.0 || sweep == 0.0 {
        return;
    }
    let length = (radius as f64 * sweep.abs()) as f32;
    let segments =
        ((length / ARC_CHORD_PX).ceil() as usize).clamp(MIN_ARC_SEGMENTS, MAX_ARC_SEGMENTS);
    let mut previous = point(geometry, start, radius);
    for i in 1..=segments {
        let angle = start + sweep * i as f64 / segments as f64;
        let next = point(geometry, angle, radius);
        sink.stroke(slot, [previous.0, previous.1], [next.0, next.1], style);
        previous = next;
    }
}

/// The pixel radius of a radius-axis `value`: the inner ring at the axis
/// start, the outer ring at its end.
fn ring_radius(
    polar: &PolarOptions,
    geometry: &PolarGeometry,
    axis: &AxisOptions,
    value: f64,
) -> f32 {
    let inner = geometry.radius * polar.inner_radius_fraction;
    inner + (geometry.radius - inner) * crate::layout::axis_fraction(value, axis, geometry.radius)
}

/// Whether an angle-axis `value` draws: inside the sector, and on a full
/// circle not on the same spoke as an earlier value in `earlier`.
fn angle_drawn(map: &AngleMap, value: f64, earlier: &[f64]) -> bool {
    if !map.contains(value) {
        return false;
    }
    if map.window.is_some() {
        return true;
    }
    let turn = map.values_per_turn();
    !earlier.iter().any(|other| {
        let turns = (value - other) / turn;
        (turns - turns.round()).abs() * turn < 1e-6 * turn.max(1.0)
    })
}

/// Rings at the radius ticks and spokes at the angle ticks, from the grid
/// options: `*_y` fields are rings, `*_x` fields are spokes. Below the data.
pub fn emit_polar_grid(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let (Some(polar), Some(geometry), Some(map)) = (
        config.polar(),
        config.polar_geometry(),
        config.polar_angle_map(),
    ) else {
        return;
    };
    let g = &config.grid;
    let corners = polygon_corners(polar, &config.bottom_x);
    let inner = geometry.radius * polar.inner_radius_fraction;
    let rings = |sink: &mut dyn Sink,
                 values: Vec<f64>,
                 color: &Color,
                 width: f32,
                 style: &LineStylePreset,
                 prefix: &str| {
        for (i, value) in values.into_iter().enumerate() {
            let radius = ring_radius(polar, &geometry, &config.left_y, value);
            if radius > inner + 0.5 && radius <= geometry.radius + 0.5 {
                let tag = format!("{prefix}_{i}");
                stroke_ring(
                    sink,
                    Slot::Below,
                    &geometry,
                    &map,
                    corners,
                    radius,
                    &StrokeStyle {
                        width,
                        color: *color,
                        dash: style.pattern(),
                        tag: &tag,
                    },
                );
            }
        }
    };
    let spokes = |sink: &mut dyn Sink,
                  values: Vec<f64>,
                  color: &Color,
                  width: f32,
                  style: &LineStylePreset,
                  prefix: &str| {
        for (i, value) in values.iter().enumerate() {
            if !angle_drawn(&map, *value, &values[..i]) {
                continue;
            }
            let angle = map.screen_angle(*value);
            let tag = format!("{prefix}_{i}");
            let a = point(&geometry, angle, inner);
            let b = point(&geometry, angle, geometry.radius);
            sink.stroke(
                Slot::Below,
                [a.0, a.1],
                [b.0, b.1],
                &StrokeStyle {
                    width,
                    color: *color,
                    dash: style.pattern(),
                    tag: &tag,
                },
            );
        }
    };
    // Minor first so major paints over it, like the rectangular grid.
    if g.show_minor_x {
        spokes(
            sink,
            minor_tick_values_for_axis(
                &config.bottom_x,
                Side::Bottom,
                &data_area_rect(da),
                measure,
            ),
            &g.minor_x_color,
            g.minor_x_width,
            &g.minor_x_style,
            "grid_minor_x",
        );
    }
    if g.show_minor_y {
        rings(
            sink,
            minor_tick_values_for_axis(&config.left_y, Side::Left, &data_area_rect(da), measure),
            &g.minor_y_color,
            g.minor_y_width,
            &g.minor_y_style,
            "grid_minor_y",
        );
    }
    if g.show_major_x {
        spokes(
            sink,
            major_tick_values_for_axis(
                &config.bottom_x,
                Side::Bottom,
                &data_area_rect(da),
                measure,
            ),
            &g.major_x_color,
            g.major_x_width,
            &g.major_x_style,
            "grid_major_x",
        );
    }
    if g.show_major_y {
        rings(
            sink,
            major_tick_values_for_axis(&config.left_y, Side::Left, &data_area_rect(da), measure),
            &g.major_y_color,
            g.major_y_width,
            &g.major_y_style,
            "grid_major_y",
        );
    }
}

/// One tick label's text and extents, in the axis' label format: the text
/// every numeric tick label of every axis family is drawn, measured, hit
/// and exported with. With a [`AxisOptions::unit`] (#273) it prints the
/// shown value of the tick at data value `value`
/// ([`crate::tick::shown_tick_value`]) against the shown range.
pub fn tick_text(
    axis: &AxisOptions,
    value: f64,
    measure: &dyn MeasureText,
) -> (NumericTickText, TextExtents) {
    let ls = &axis.label_style;
    let shown = crate::tick::shown_tick_value(axis, value);
    if matches!(ls.format, LabelFormat::Power) {
        let text = format_tick_power(shown, ls.significant_digits, ls);
        let metrics = measure.measure_rich(&text);
        (NumericTickText::Power(text), metrics)
    } else {
        let (min, max) = axis.shown_range();
        let text = format_tick_value(
            shown,
            &ls.format,
            ls.significant_digits,
            &axis.scale,
            effective_major_spacing(axis),
            min,
            max,
        );
        let metrics = measure.measure_plain(&text, &ls.label_font, ls.font_size);
        (NumericTickText::Plain(text), metrics)
    }
}

/// The baseline origin of `text` centred on `center`: the one formula the
/// polar labels are placed, boxed and pruned with.
fn centered_origin(center: (f32, f32), m: TextExtents) -> (f32, f32) {
    (
        center.0 - m.width * 0.5,
        center.1 + (m.ascent - m.descent) * 0.5,
    )
}

/// The box of `text` centred on `center` ([`label_rect`] at its origin).
fn centered_rect(center: (f32, f32), m: TextExtents) -> RectF {
    label_rect(centered_origin(center, m), m)
}

/// Place `text` centred on `center`: the label offset fields are read in
/// polar frames (`PolarOptions::angle_label_offset`,
/// `PolarOptions::radius_label_offset`) by the walks, not as screen x / y.
pub fn place_centered(
    sink: &mut dyn Sink,
    text: TickText<'_>,
    center: (f32, f32),
    m: TextExtents,
    ls: &LabelStyle,
) {
    let placed = PlacedText {
        text: match text {
            TickText::Plain(text) => TextRef::Plain {
                text,
                family: &ls.label_font,
                size: ls.font_size,
                color: ls.color,
            },
            TickText::Power(rt) => TextRef::Rich(rt),
        },
        origin: centered_origin(center, m),
        extents: m,
        rotation: None,
    };
    sink.text(Slot::Above, &placed);
}

/// The room text keeps from tick labels of font size `label_font` beside
/// it: auto_layout's title gap (#270), half the label font, at least
/// [`LABEL_GAP`](crate::layout::LABEL_GAP). Polar radius labels keep it
/// from the angle labels and ternary edge titles from their labels (#272).
pub(crate) fn label_title_gap(label_font: f32) -> f32 {
    crate::layout::LABEL_GAP.max(0.5 * label_font.max(0.0))
}

/// The outward tick extent of an axis: how far its ticks reach past the
/// line, which is where its labels start.
pub fn outward_tick(axis: &AxisOptions) -> f32 {
    match axis.tick {
        TickVisibility::None | TickVisibility::Inside => 0.0,
        TickVisibility::Outside | TickVisibility::Both => axis.major_tick_length.max(0.0),
    }
}

/// The polar chrome above the data: the angle axis (`bottom_x`) as the outer
/// ring with its ticks, its labels, then the radius axis (`left_y`) as the
/// ray at the radial label angle with its ticks and labels.
pub fn emit_polar_axes(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    emit_polar_ring(config, da, measure, sink);
    emit_polar_angle_labels(config, da, measure, sink);
    emit_polar_radius_strokes(config, da, measure, sink);
    emit_polar_radius_labels(config, da, measure, sink);
}

/// The outer ring and its ticks: the data boundary, which no element owns
/// (the angle axis is not selectable).
fn emit_polar_ring(config: &Config, da: &DataArea, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    let (Some(polar), Some(geometry), Some(map)) = (
        config.polar(),
        config.polar_geometry(),
        config.polar_angle_map(),
    ) else {
        return;
    };
    let axis = &config.bottom_x;
    let dash = axis.line_style.pattern();
    if axis.line_visible {
        stroke_ring(
            sink,
            Slot::Above,
            &geometry,
            &map,
            polygon_corners(polar, axis),
            geometry.radius,
            &StrokeStyle {
                width: axis.line_width,
                color: axis.line_color,
                dash,
                tag: "axis_bottom",
            },
        );
    }
    let majors = major_tick_values_for_axis(axis, Side::Bottom, &data_area_rect(da), measure);
    let minors = minor_tick_values_for_axis(axis, Side::Bottom, &data_area_rect(da), measure);
    if axis.tick != TickVisibility::None {
        for (i, (value, length)) in majors
            .iter()
            .map(|v| (*v, axis.major_tick_length))
            .chain(minors.iter().map(|v| (*v, axis.minor_tick_length)))
            .enumerate()
        {
            let earlier = if i < majors.len() {
                &majors[..i]
            } else {
                &minors[..i - majors.len()]
            };
            if !angle_drawn(&map, value, earlier) {
                continue;
            }
            let angle = map.screen_angle(value);
            let (inside, outside) = match axis.tick {
                TickVisibility::None => continue,
                TickVisibility::Outside => (geometry.radius, geometry.radius + length),
                TickVisibility::Inside => (geometry.radius - length, geometry.radius),
                TickVisibility::Both => (geometry.radius - length, geometry.radius + length),
            };
            let tag = format!("tick_bottom_{i}");
            let a = point(&geometry, angle, inside);
            let b = point(&geometry, angle, outside);
            sink.stroke(
                Slot::Above,
                [a.0, a.1],
                [b.0, b.1],
                &StrokeStyle {
                    width: axis.line_width,
                    color: axis.line_color,
                    dash,
                    tag: &tag,
                },
            );
        }
    }
}

/// The box of everything a polar chart draws around its disc: the outer
/// ring (or the sector's arc) with the angle axis' outward ticks, the angle
/// and radius labels, and the radius axis' ray and ticks, whether or not
/// the ring itself is stroked. `None` when the chart is not polar.
fn polar_chrome_bounds(config: &Config, measure: &dyn MeasureText) -> Option<RectF> {
    let (Some(polar), Some(geometry), Some(map)) = (
        config.polar(),
        config.polar_geometry(),
        config.polar_angle_map(),
    ) else {
        return None;
    };
    let da = config.data_area().ok()?;
    let mut bounds: Option<RectF> = None;
    let mut grow = |part: crate::sink::DrawnPart| {
        let r = part.extent();
        bounds = Some(bounds.map_or(r, |b| crate::sink::union(b, r)));
    };
    let mut parts = crate::sink::Parts(&mut grow);
    stroke_ring(
        &mut parts,
        Slot::Above,
        &geometry,
        &map,
        polygon_corners(polar, &config.bottom_x),
        geometry.radius + outward_tick(&config.bottom_x),
        &StrokeStyle {
            width: 0.0,
            color: Color::BLACK,
            dash: &[],
            tag: "polar_bounds",
        },
    );
    emit_polar_angle_labels(config, &da, measure, &mut parts);
    emit_polar_radius_strokes(config, &da, measure, &mut parts);
    emit_polar_radius_labels(config, &da, measure, &mut parts);
    bounds
}

/// Where a polar chart's axis title on `side` goes (#272): `base` is the
/// band placement the title would have without its offsets
/// ([`axis_title_placement`](crate::layout::axis_title_placement)), and the
/// result is the move that puts it beside the disc instead, the title gap
/// past everything drawn around the disc ([`polar_chrome_bounds`]): the
/// radius axis' title (`Side::Left`) left of it and centred on it, the
/// angle axis' title (`Side::Bottom`) below it and centred on it. The
/// caller adds the move to the placement with the offsets, so a drag moves
/// the title as it does on a rectangular chart. The gap is the title gap
/// of the larger of the two axes' label fonts. `None` when the chart is not
/// polar or for another side.
pub(crate) fn polar_title_shift(
    config: &Config,
    side: &Side,
    base: &crate::layout::TitlePlacement,
    extents: TextExtents,
    measure: &dyn MeasureText,
) -> Option<(f32, f32)> {
    let bounds = polar_chrome_bounds(config, measure)?;
    let font = config
        .left_y
        .label_style
        .font_size
        .max(config.bottom_x.label_style.font_size);
    let gap = label_title_gap(font);
    match side {
        Side::Left => {
            // The left title turns about its centre, which is the centre of
            // its box: `height` across, `width` along.
            let center = base.rotation_center;
            let to = (
                bounds.x - gap - extents.height() * 0.5,
                bounds.y + bounds.height * 0.5,
            );
            Some((to.0 - center.0, to.1 - center.1))
        }
        Side::Bottom => {
            let top = base.origin.1 - extents.ascent;
            let left = base.origin.0;
            let to = (
                bounds.x + bounds.width * 0.5 - extents.width * 0.5,
                bounds.y + bounds.height + gap,
            );
            Some((to.0 - left, to.1 - top))
        }
        Side::Top | Side::Right => None,
    }
}

/// The angle axis' tick labels (`Element::AxisLabel(Side::Bottom)`).
pub fn emit_polar_angle_labels(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let (Some(geometry), Some(map)) = (config.polar_geometry(), config.polar_angle_map()) else {
        return;
    };
    let axis = &config.bottom_x;
    let majors = major_tick_values_for_axis(axis, Side::Bottom, &data_area_rect(da), measure);
    let ls = &axis.label_style;
    each_angle_label(
        &map,
        &geometry,
        axis,
        &majors,
        measure,
        |text, center, m| {
            place_centered(sink, text, center, m, ls);
        },
    );
}

/// The radius axis' ray and ticks (`Element::Axis(Side::Left)`), from the
/// walk [`each_radius_axis_stroke`] reports.
pub fn emit_polar_radius_strokes(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let (Some(polar), Some(geometry)) = (config.polar(), config.polar_geometry()) else {
        return;
    };
    let axis = &config.left_y;
    let majors = major_tick_values_for_axis(axis, Side::Left, &data_area_rect(da), measure);
    let dash = axis.line_style.pattern();
    each_radius_axis_stroke(
        polar,
        &geometry,
        axis,
        da,
        measure,
        &majors,
        |tick, a, b| {
            let tag = match tick {
                None => "axis_left".to_string(),
                Some(i) => format!("tick_left_{i}"),
            };
            sink.stroke(
                Slot::Above,
                [a.0, a.1],
                [b.0, b.1],
                &StrokeStyle {
                    width: axis.line_width,
                    color: axis.line_color,
                    dash,
                    tag: &tag,
                },
            );
        },
    );
}

/// The radius axis' tick labels (`Element::AxisLabel(Side::Left)`).
pub fn emit_polar_radius_labels(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let (Some(polar), Some(geometry)) = (config.polar(), config.polar_geometry()) else {
        return;
    };
    let axis = &config.left_y;
    let majors = major_tick_values_for_axis(axis, Side::Left, &data_area_rect(da), measure);
    let ls = &axis.label_style;
    // The angle labels the radius labels keep their distance from (#272):
    // the ones `emit_polar_angle_labels` draws, from the same walk.
    let map = config.polar_angle_map();
    let angle_majors = map
        .map(|_| {
            major_tick_values_for_axis(&config.bottom_x, Side::Bottom, &data_area_rect(da), measure)
        })
        .unwrap_or_default();
    let angle_boxes = |each: &mut dyn FnMut(RectF)| {
        if let Some(map) = &map {
            each_angle_label(
                map,
                &geometry,
                &config.bottom_x,
                &angle_majors,
                measure,
                |_, center, m| each(centered_rect(center, m)),
            );
        }
    };
    each_radius_label(
        polar,
        &geometry,
        axis,
        &majors,
        measure,
        &angle_boxes,
        |text, center, m| {
            place_centered(sink, text.as_tick_text(), center, m, ls);
        },
    );
}

/// How far `rect` moves along the unit vector `toward` so that it keeps
/// `gap` from every box `boxes` reports: 0 when it already does. Each box
/// it meets is passed on the side the move reaches first, and a box once
/// passed stays passed (the move only goes one way), so the walk ends after
/// at most one pass more than there are boxes.
fn clearance_along(
    rect: RectF,
    toward: (f32, f32),
    gap: f32,
    boxes: &dyn Fn(&mut dyn FnMut(RectF)),
) -> f32 {
    const EPS: f32 = 1e-4;
    if toward.0.abs() < EPS && toward.1.abs() < EPS {
        return 0.0;
    }
    let mut travel = 0.0f32;
    loop {
        let mut moved = false;
        boxes(&mut |other| {
            let at = RectF {
                x: rect.x + toward.0 * travel,
                y: rect.y + toward.1 * travel,
                ..rect
            };
            let (x0, y0) = (other.x - gap, other.y - gap);
            let (x1, y1) = (other.x + other.width + gap, other.y + other.height + gap);
            let meets = at.x < x1 && at.x + at.width > x0 && at.y < y1 && at.y + at.height > y0;
            if !meets {
                return;
            }
            // The move that clears the box across x, or across y.
            let pass = |lo: f32, hi: f32, start: f32, end: f32, d: f32| {
                if d > EPS {
                    (hi - start) / d
                } else if d < -EPS {
                    (end - lo) / -d
                } else {
                    f32::INFINITY
                }
            };
            let across_x = pass(x0, x1, at.x, at.x + at.width, toward.0);
            let across_y = pass(y0, y1, at.y, at.y + at.height, toward.1);
            let need = across_x.min(across_y);
            if need.is_finite() && need > 0.0 {
                travel += need;
                moved = true;
            }
        });
        if !moved {
            return travel;
        }
    }
}

/// The text of the angle-axis label at `value`, with its extents: the
/// category label of a categorical axis (`None` past its categories), else
/// the value in the axis' label format.
fn angle_label_text<'a>(
    axis: &'a AxisOptions,
    value: f64,
    measure: &dyn MeasureText,
) -> Option<(AngleLabelText<'a>, TextExtents)> {
    let ls = &axis.label_style;
    match axis.categories.as_deref() {
        Some(categories) => {
            let label = categories.get(value as usize)?.label.as_str();
            let m = measure.measure_plain(label, &ls.label_font, ls.font_size);
            Some((AngleLabelText::Category(label), m))
        }
        None => {
            let (text, m) = tick_text(axis, value, measure);
            Some((AngleLabelText::Numeric(text), m))
        }
    }
}

/// An angle label's text: a category's label, borrowed, or a formatted
/// value.
enum AngleLabelText<'a> {
    Category(&'a str),
    Numeric(NumericTickText),
}

impl AngleLabelText<'_> {
    fn as_tick_text(&self) -> TickText<'_> {
        match self {
            AngleLabelText::Category(label) => TickText::Plain(label),
            AngleLabelText::Numeric(text) => text.as_tick_text(),
        }
    }
}

/// The angle axis' tick labels as [`emit_polar_angle_labels`] places them: `place`
/// gets each kept label's text, its centre on screen and its extents. A
/// categorical axis' labels are its category labels at the categories (its
/// major ticks); an empty label keeps its slot in the stride and draws
/// nothing.
fn each_angle_label(
    map: &AngleMap,
    geometry: &PolarGeometry,
    axis: &AxisOptions,
    majors: &[f64],
    measure: &dyn MeasureText,
    mut place: impl FnMut(TickText<'_>, (f32, f32), TextExtents),
) {
    let ls = &axis.label_style;
    if !(ls.visible && ls.label_visible) {
        return;
    }
    let (_, sweep) = sweep(map);
    // Keep every k-th label when the labels would overlap around the
    // ring: the same 1/2/5 stride rule the straight axes use.
    let gap = ls.font_size * 0.5;
    let circumference = (geometry.radius as f64 * sweep.abs()) as f32;
    let mut total = 0.0f32;
    let mut count = 0usize;
    for (i, value) in majors.iter().enumerate() {
        if angle_drawn(map, *value, &majors[..i])
            && let Some((_, m)) = angle_label_text(axis, *value, measure)
        {
            total += m.width + gap;
            count += 1;
        }
    }
    let mut stride = 1usize;
    while count > 0 && total / stride as f32 > circumference.max(1.0) {
        let decade = 10usize.pow(stride.ilog10());
        stride = match stride / decade {
            1 => 2 * decade,
            2 => 5 * decade,
            _ => 10 * decade,
        };
    }
    let band = outward_tick(axis) + 4.0;
    let radial = PolarOptions::angle_label_offset(ls);
    let mut drawn = 0usize;
    for (i, value) in majors.iter().enumerate() {
        if !angle_drawn(map, *value, &majors[..i]) {
            continue;
        }
        let Some((text, m)) = angle_label_text(axis, *value, measure) else {
            continue;
        };
        drawn += 1;
        if (drawn - 1) % stride != 0 {
            continue;
        }
        let text = text.as_tick_text();
        if matches!(text, TickText::Plain("")) {
            continue;
        }
        let angle = map.screen_angle(*value);
        let (dx, dy) = (angle.cos() as f32, -(angle.sin() as f32));
        // The label box sits just outside the ring along the spoke.
        let reach = band + dx.abs() * m.width * 0.5 + dy.abs() * m.height() * 0.5;
        let center = (
            geometry.center[0] + dx * (geometry.radius + reach),
            geometry.center[1] + dy * (geometry.radius + reach),
        );
        // Along the spoke, so every label keeps its distance to the ring.
        place(text, (center.0 + dx * radial, center.1 + dy * radial), m);
    }
}

/// The radius axis' tick labels as [`emit_polar_radius_labels`] places them, beside
/// the ray: `place` gets each label's text, centre and extents. A label
/// that would come within the gap of an angle label (`angle_boxes`, the
/// boxes of the drawn angle labels) moves inward along the ray until it
/// keeps the gap (#272); the dragged offset applies after that.
fn each_radius_label(
    polar: &PolarOptions,
    geometry: &PolarGeometry,
    axis: &AxisOptions,
    majors: &[f64],
    measure: &dyn MeasureText,
    angle_boxes: &dyn Fn(&mut dyn FnMut(RectF)),
    mut place: impl FnMut(&NumericTickText, (f32, f32), TextExtents),
) {
    let inner = geometry.inner_radius;
    let ray = polar.radial_label_angle_degrees.to_radians();
    let [dx, dy] = polar.ray_direction();
    let dir = (dx, dy);
    let [sx, sy] = polar.ray_side();
    let side = (sx, sy);
    let ls = &axis.label_style;
    if !(ls.visible && ls.label_visible) {
        return;
    }
    let offset = polar.radius_label_offset(ls);
    let band = outward_tick(axis) + 4.0;
    // The room a radius label keeps from an angle label.
    let gap = label_title_gap(ls.font_size);
    // Where a label is drawn: beside its tick and half a line inward, then
    // further inward along the ray while it would come within the gap of
    // an angle label (the outermost one, below the angle label on the
    // ray's spoke), then moved by the dragged offset.
    let center = |position: (f32, f32), m: TextExtents| {
        let reach = band + side.0.abs() * m.width * 0.5 + side.1.abs() * m.height() * 0.5;
        let inward = m.height() * 0.5;
        let placed = (
            position.0 + side.0 * reach - dir.0 * inward,
            position.1 + side.1 * reach - dir.1 * inward,
        );
        let clear = clearance_along(centered_rect(placed, m), (-dir.0, -dir.1), gap, angle_boxes);
        (
            placed.0 - dir.0 * clear + offset[0],
            placed.1 - dir.1 * clear + offset[1],
        )
    };
    // Thinned where the drawn boxes collide, so the ray's angle alone does
    // not change which labels stay.
    let labels = prune_numeric_labels(
        axis,
        measure,
        majors,
        |value| point(geometry, ray, ring_radius(polar, geometry, axis, value)),
        |position, m| centered_rect(center(position, m), m),
    );
    let radius_of = |position: (f32, f32)| {
        (position.0 - geometry.center[0]).hypot(position.1 - geometry.center[1])
    };
    let drawn = |radius: f32| radius >= inner - 0.5 && radius <= geometry.radius + 0.5;
    // The innermost drawn label, skipped when the chart hides it (#259).
    let first = polar
        .hide_first_radius_label
        .then(|| {
            labels
                .iter()
                .map(|label| radius_of(label.position))
                .filter(|radius| drawn(*radius))
                .fold(f32::INFINITY, f32::min)
        })
        .filter(|radius| radius.is_finite());
    for label in &labels {
        let radius = radius_of(label.position);
        if !drawn(radius) || first == Some(radius) {
            continue;
        }
        let m = extents(&label.text, ls, measure);
        place(&label.text, center(label.position, m), m);
    }
}

/// Every segment the radius axis is stroked along: the ray from the inner
/// radius to the rim (`None`), then tick `i` across it on the label side
/// (`Some(i)`, majors first).
fn each_radius_axis_stroke(
    polar: &crate::polar::PolarOptions,
    geometry: &crate::polar::PolarGeometry,
    axis: &crate::config::AxisOptions,
    da: &DataArea,
    measure: &dyn MeasureText,
    majors: &[f64],
    mut each: impl FnMut(Option<usize>, (f32, f32), (f32, f32)),
) {
    let inner = geometry.inner_radius;
    let ray = polar.radial_label_angle_degrees.to_radians();
    // Labels sit on the left of the ray seen from the centre
    // (`PolarOptions::ray_side`).
    let [sx, sy] = polar.ray_side();
    let side = (sx, sy);
    if axis.line_visible {
        each(
            None,
            point(geometry, ray, inner),
            point(geometry, ray, geometry.radius),
        );
    }
    if axis.tick == TickVisibility::None {
        return;
    }
    let minors = minor_tick_values_for_axis(axis, Side::Left, &data_area_rect(da), measure);
    for (i, (value, length)) in majors
        .iter()
        .map(|v| (*v, axis.major_tick_length))
        .chain(minors.iter().map(|v| (*v, axis.minor_tick_length)))
        .enumerate()
    {
        let radius = ring_radius(polar, geometry, axis, value);
        if radius < inner - 0.5 || radius > geometry.radius + 0.5 {
            continue;
        }
        let at = point(geometry, ray, radius);
        let (a, b) = match axis.tick {
            TickVisibility::None => continue,
            TickVisibility::Outside => (0.0, length),
            TickVisibility::Inside => (-length, 0.0),
            TickVisibility::Both => (-length, length),
        };
        each(
            Some(i),
            (at.0 + side.0 * a, at.1 + side.1 * a),
            (at.0 + side.0 * b, at.1 + side.1 * b),
        );
    }
}

fn extents(text: &NumericTickText, ls: &LabelStyle, measure: &dyn MeasureText) -> TextExtents {
    match text {
        NumericTickText::Plain(text) => measure.measure_plain(text, &ls.label_font, ls.font_size),
        NumericTickText::Power(text) => measure.measure_rich(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Element, auto_layout};
    use crate::polar::{AngleUnit, PolarOptions};

    /// Half an em per character, 0.8 em above the baseline and 0.2 below.
    struct HalfEm;
    impl MeasureText for HalfEm {
        fn measure_plain(&self, text: &str, _family: &str, size: f32) -> TextExtents {
            TextExtents {
                width: text.chars().count() as f32 * size * 0.5,
                ascent: size * 0.8,
                descent: size * 0.2,
            }
        }
        fn measure_rich(&self, text: &crate::text::RichText) -> TextExtents {
            let size = text.font_size;
            TextExtents {
                width: text.segments.len() as f32 * size * 0.5,
                ascent: size * 0.8,
                descent: size * 0.2,
            }
        }
    }

    /// The text boxes `element` of `config` draws.
    fn boxes(config: &Config, element: Element) -> Vec<RectF> {
        let mut out = Vec::new();
        crate::select::on_drawn_panel(config, |panel| {
            emit_element(
                panel,
                &element,
                &HalfEm,
                &mut crate::sink::Parts(&mut |part| {
                    if let crate::sink::DrawnPart::Text(r) = part {
                        out.push(r);
                    }
                }),
            );
        });
        out
    }

    /// How far apart `a` and `b` are: the larger of their gaps across x and
    /// across y, negative when they overlap.
    fn apart(a: &RectF, b: &RectF) -> f32 {
        let x = (b.x - (a.x + a.width)).max(a.x - (b.x + b.width));
        let y = (b.y - (a.y + a.height)).max(a.y - (b.y + b.height));
        x.max(y)
    }

    /// A 960 x 640 polar chart of `polar` over the angles `angles` labelled
    /// every `angle_step` in `format` and the radii `0..radius` every
    /// `radius_step`.
    fn polar_chart(
        polar: PolarOptions,
        angles: (f64, f64, f64),
        format: LabelFormat,
        radius: (f64, f64),
    ) -> Config {
        let mut config = crate::defaults::config();
        config.chart_area.0.width = 960;
        config.chart_area.0.height = 640;
        config.bottom_x.min = angles.0;
        config.bottom_x.max = angles.1;
        config.bottom_x.major_spacing = angles.2;
        config.bottom_x.label_style.format = format;
        config.left_y.min = 0.0;
        config.left_y.max = radius.0;
        config.left_y.major_spacing = radius.1;
        config.set_polar(Some(polar));
        config
    }

    // The radius axis prints its unit (#273) through the one numeric label
    // text: `0, 200, …, 1000` on the rings `0, 2, …, 10` of the data.
    #[test]
    fn the_radius_axis_prints_its_unit() {
        let mut plain = polar_chart(
            crate::defaults::polar_options(),
            (0.0, 360.0, 30.0),
            LabelFormat::Degrees,
            (10.0, 2.0),
        );
        plain.left_y.label_style.format = LabelFormat::Decimal;
        plain.left_y.label_style.significant_digits = 1;
        let mut converted = plain.clone();
        converted.left_y.unit = Some(crate::config::AxisUnit::new(100.0, 0.0));
        converted.left_y.major_spacing = 200.0;
        assert_eq!(converted.validate_chart(), Ok(()));
        let (a, b) = (
            boxes(&plain, Element::AxisLabel(Side::Left)),
            boxes(&converted, Element::AxisLabel(Side::Left)),
        );
        assert_eq!(a.len(), b.len());
        assert!(a.len() >= 3, "{a:?}");
        let size = plain.left_y.label_style.font_size;
        let widest = |r: &[RectF]| r.iter().map(|r| r.width).fold(0.0f32, f32::max);
        // "10" against "1000", half an em per character.
        assert_eq!(widest(&a), 2.0 * 0.5 * size);
        assert_eq!(widest(&b), 4.0 * 0.5 * size);
    }

    /// The polar example's scenes and the radar gallery's circular one: an
    /// antenna pattern (ray up through the 90° label), a compass rose (ray
    /// at 45° under its label), a sector in radians, a clockwise heatmap
    /// with a hole, and a radar document with value labels.
    #[allow(deprecated)]
    fn example_charts() -> Vec<(&'static str, Config)> {
        use std::f64::consts::PI;
        let degrees = || LabelFormat::Degrees;
        let mut charts = vec![
            (
                "antenna",
                polar_chart(
                    crate::defaults::polar_options(),
                    (0.0, 360.0, 30.0),
                    degrees(),
                    (1.0, 0.25),
                ),
            ),
            (
                "compass",
                polar_chart(
                    PolarOptions {
                        zero_angle_degrees: 90.0,
                        clockwise: true,
                        radial_label_angle_degrees: 45.0,
                        ..crate::defaults::polar_options()
                    },
                    (0.0, 360.0, 45.0),
                    degrees(),
                    (3.0, 1.0),
                ),
            ),
            (
                "sector",
                polar_chart(
                    PolarOptions {
                        angle_unit: AngleUnit::Radians,
                        sector: Some([-PI / 2.0, PI / 2.0]),
                        inner_radius_fraction: 0.2,
                        radial_label_angle_degrees: 90.0,
                        ..crate::defaults::polar_options()
                    },
                    (-PI / 2.0, PI / 2.0, PI / 4.0),
                    LabelFormat::Radians,
                    (2.0, 0.5),
                ),
            ),
            (
                "heatmap",
                polar_chart(
                    PolarOptions {
                        zero_angle_degrees: 90.0,
                        clockwise: true,
                        inner_radius_fraction: 0.08,
                        radial_label_angle_degrees: 90.0,
                        ..crate::defaults::polar_options()
                    },
                    (0.0, 360.0, 30.0),
                    degrees(),
                    (6.0, 1.0),
                ),
            ),
        ];
        let document = crate::radar::RadarChart {
            categories: ["Speed", "Safety", "Size", "Docs", "Tests", "Ergonomics"]
                .into_iter()
                .map(|s| crate::categorical::Category::new(s.to_lowercase(), s))
                .collect(),
            series: vec![crate::radar::RadarSeries::new(
                "v1",
                "Release 1",
                vec![6., 8., 4., 5., 7., 6.5],
                crate::Color::BLACK,
            )],
            font_size: 18.,
            value_range: Some([0.0, 10.0]),
            labels: crate::radar::RadarLabels::Values,
            ..crate::defaults::radar_chart()
        };
        let (mut radar, _, _) = Config::from_radar(&document);
        radar.chart_area.0.width = 960;
        radar.chart_area.0.height = 640;
        charts.push(("radar", radar));
        charts
    }

    /// Every radius (ring) label keeps the title gap from every angle
    /// (category) label, before and after auto_layout (#272): the 1.00 under
    /// the 90° label, the 3.00 beside the 45° one, the radar's 10 under its
    /// top category.
    #[test]
    fn radius_labels_keep_the_gap_from_the_angle_labels() {
        for (name, chart) in example_charts() {
            for config in [chart.clone(), auto_layout(&chart, &HalfEm)] {
                let radius = boxes(&config, Element::AxisLabel(Side::Left));
                let angle = boxes(&config, Element::AxisLabel(Side::Bottom));
                assert!(radius.len() >= 3 && angle.len() >= 3, "{name}");
                let gap = label_title_gap(config.left_y.label_style.font_size);
                for r in &radius {
                    for a in &angle {
                        assert!(apart(r, a) >= gap - 1e-3, "{name}: {r:?} {a:?}");
                    }
                }
            }
        }
    }

    /// A polar chart's axis titles sit beside its disc (#272): the radius
    /// title the title gap left of everything drawn around the disc, the
    /// angle title the gap below it, both centred on it and clear of every
    /// label, before and after auto_layout, and a title offset moves the
    /// title from there.
    #[test]
    fn polar_titles_sit_the_title_gap_beside_the_disc() {
        for (name, mut chart) in example_charts() {
            for (axis, text) in [
                (&mut chart.bottom_x, "Azimuth"),
                (&mut chart.left_y, "Gain"),
            ] {
                let title = &mut axis.title_option;
                title.visible = true;
                title.text = crate::text::RichText::plain(text, title.text.color, 16.0, "");
            }
            for config in [chart.clone(), auto_layout(&chart, &HalfEm)] {
                let bounds = polar_chrome_bounds(&config, &HalfEm).unwrap();
                let gap = label_title_gap(
                    config
                        .left_y
                        .label_style
                        .font_size
                        .max(config.bottom_x.label_style.font_size),
                );
                let labels: Vec<RectF> = [Side::Left, Side::Bottom]
                    .into_iter()
                    .flat_map(|side| boxes(&config, Element::AxisLabel(side)))
                    .collect();
                let left = boxes(&config, Element::AxisTitle(Side::Left));
                let bottom = boxes(&config, Element::AxisTitle(Side::Bottom));
                let (left, bottom) = (left[0], bottom[0]);
                assert!(
                    (bounds.x - (left.x + left.width) - gap).abs() < 0.5,
                    "{name}: {left:?} beside {bounds:?}"
                );
                assert!(
                    (bottom.y - (bounds.y + bounds.height) - gap).abs() < 0.5,
                    "{name}: {bottom:?} below {bounds:?}"
                );
                let middle = |r: &RectF| (r.x + r.width * 0.5, r.y + r.height * 0.5);
                assert!((middle(&left).1 - middle(&bounds).1).abs() < 0.5, "{name}");
                assert!(
                    (middle(&bottom).0 - middle(&bounds).0).abs() < 0.5,
                    "{name}"
                );
                for label in &labels {
                    assert!(apart(&left, label) >= gap - 1e-3, "{name}: {label:?}");
                    assert!(apart(&bottom, label) >= gap - 1e-3, "{name}: {label:?}");
                }
                // auto_layout writes no title offset on a polar chart.
                assert_eq!(
                    config.left_y.title_option.offset_y,
                    chart.left_y.title_option.offset_y
                );
            }
            let mut moved = chart.clone();
            moved.bottom_x.title_option.offset_x = 7.0;
            moved.bottom_x.title_option.offset_y = -3.0;
            let at = boxes(&chart, Element::AxisTitle(Side::Bottom))[0];
            let to = boxes(&moved, Element::AxisTitle(Side::Bottom))[0];
            assert!((to.x - at.x - 7.0).abs() < 1e-3 && (to.y - at.y + 3.0).abs() < 1e-3);
        }
    }

    /// A colour bar on a polar chart keeps auto_layout's title gap between
    /// its labels and its title, as on a rectangular chart (#270, #272).
    #[test]
    fn a_polar_colorbar_title_keeps_the_title_gap() {
        for polar in [false, true] {
            let mut config = crate::defaults::config();
            config.chart_area.0.width = 1200;
            config.chart_area.0.height = 800;
            if polar {
                config.set_polar(Some(crate::defaults::polar_options()));
            }
            let mut bar = crate::defaults::color_bar_options();
            bar.visible = true;
            bar.gap_px = 24.0;
            bar.thickness_px = 16.0;
            bar.length_frac = 0.6;
            bar.side = Side::Right;
            bar.axis.min = -30.0;
            bar.axis.max = 0.0;
            bar.axis.major_spacing = 5.0;
            bar.axis.label_style.font_size = 18.0;
            bar.axis.tick = TickVisibility::None;
            bar.offset_x = -110.0;
            bar.axis.title_option.visible = true;
            bar.axis.title_option.text =
                crate::text::RichText::plain("Energy (dB)", crate::Color::BLACK, 19.0, "");
            config.colorbar = Some(bar);
            let config = auto_layout(&config, &HalfEm);
            let labels = boxes(&config, Element::ColorBarLabel);
            let title = boxes(&config, Element::ColorBarTitle)[0];
            let right = labels
                .iter()
                .map(|r| r.x + r.width)
                .fold(f32::NEG_INFINITY, f32::max);
            let gap = label_title_gap(18.0);
            assert!(
                title.x - right >= gap - 1e-3,
                "polar {polar}: {}",
                title.x - right
            );
            assert!(
                title.x - right <= gap + 1.0,
                "polar {polar}: {}",
                title.x - right
            );
        }
    }

    /// `hide_first_radius_label` drops the innermost ring's label and keeps
    /// every other one where it was (#259).
    #[test]
    fn hiding_the_first_radius_label_drops_only_the_innermost() {
        struct Measure;
        impl crate::text::MeasureText for Measure {
            fn measure_plain(
                &self,
                text: &str,
                _family: &str,
                size: f32,
            ) -> crate::text::TextExtents {
                crate::text::TextExtents {
                    width: text.chars().count() as f32 * size * 0.5,
                    ascent: size * 0.8,
                    descent: size * 0.2,
                }
            }
            fn measure_rich(&self, _text: &crate::text::RichText) -> crate::text::TextExtents {
                TextExtents {
                    width: 0.0,
                    ascent: 0.0,
                    descent: 0.0,
                }
            }
        }
        let labels = |config: &Config| {
            let da = config.data_area().unwrap();
            let mut rects = Vec::new();
            let mut each = |part: crate::sink::DrawnPart| {
                if let crate::sink::DrawnPart::Text(rect) = part {
                    rects.push(rect);
                }
            };
            emit_polar_radius_labels(config, &da, &Measure, &mut crate::sink::Parts(&mut each));
            rects
        };
        let mut config = crate::defaults::config();
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 360.0;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        config.set_polar(Some(crate::defaults::polar_options()));
        let all = labels(&config);
        assert!(all.len() >= 3, "{all:?}");
        let mut hidden = config.clone();
        hidden.polar_mut().unwrap().hide_first_radius_label = true;
        let kept = labels(&hidden);
        assert_eq!(kept.len(), all.len() - 1);
        let center = config.polar_geometry().unwrap().center;
        let distance = |r: &crate::layout::RectF| {
            (r.x + r.width * 0.5 - center[0]).hypot(r.y + r.height * 0.5 - center[1])
        };
        let innermost = all
            .iter()
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .unwrap();
        assert!(!kept.contains(innermost));
        assert!(kept.iter().all(|r| all.contains(r)));
    }
}
