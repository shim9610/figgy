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

/// One tick label's text and extents, in the axis' label format.
pub fn tick_text(
    axis: &AxisOptions,
    value: f64,
    measure: &dyn MeasureText,
) -> (NumericTickText, TextExtents) {
    let ls = &axis.label_style;
    if matches!(ls.format, LabelFormat::Power) {
        let text = format_tick_power(value, ls.significant_digits, ls);
        let metrics = measure.measure_rich(&text);
        (NumericTickText::Power(text), metrics)
    } else {
        let text = format_tick_value(
            value,
            &ls.format,
            ls.significant_digits,
            &axis.scale,
            effective_major_spacing(axis),
            axis.min,
            axis.max,
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
    each_radius_label(
        polar,
        &geometry,
        axis,
        &majors,
        measure,
        |text, center, m| {
            place_centered(sink, text.as_tick_text(), center, m, ls);
        },
    );
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
/// the ray: `place` gets each label's text, centre and extents.
fn each_radius_label(
    polar: &PolarOptions,
    geometry: &PolarGeometry,
    axis: &AxisOptions,
    majors: &[f64],
    measure: &dyn MeasureText,
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
    // Where a label is drawn: beside its tick and half a line inward, so the
    // outermost label stays clear of the angle label on the same spoke.
    let center = |position: (f32, f32), m: TextExtents| {
        let reach = band + side.0.abs() * m.width * 0.5 + side.1.abs() * m.height() * 0.5;
        let inward = m.height() * 0.5;
        (
            position.0 + side.0 * reach - dir.0 * inward + offset[0],
            position.1 + side.1 * reach - dir.1 * inward + offset[1],
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
        let mut config = crate::config::Config::omitted();
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 360.0;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        config.set_polar(Some(crate::polar::PolarOptions::default()));
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
