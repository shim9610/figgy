//! The grid and axes of a polar chart (`Config.polar`): rings at the radius
//! ticks, spokes at the angle ticks, the outer ring as the angle axis with
//! its labels around it, and the ray at `radial_label_angle_degrees` as the
//! radius axis with its labels beside it. Rings and arcs are polylines on
//! the canvas, with a segment count that follows the radius so an export at
//! a higher scale stays round. The geometry is `Config::polar_geometry`, the
//! same circle the GPU transform draws against.

use super::*;
use crate::polar::{PolarGeometry, PolarOptions};
use crate::text_render::TextMetrics;

/// Arcs are flattened into chords of about this length in pixels.
const ARC_CHORD_PX: f32 = 3.0;
const MIN_ARC_SEGMENTS: usize = 12;
const MAX_ARC_SEGMENTS: usize = 2048;

/// The screen sweep an angular range covers: start angle and signed sweep in
/// radians, counter-clockwise positive, y up.
fn sweep(polar: &PolarOptions, axis: &AxisOptions) -> (f64, f64) {
    match polar.sector {
        Some([start, end]) => {
            let a0 = polar.screen_angle(start);
            (a0, polar.screen_angle(end) - a0)
        }
        None => {
            let _ = axis;
            (
                polar.screen_angle(0.0),
                std::f64::consts::TAU * polar.direction(),
            )
        }
    }
}

/// A screen point at `angle` radians and `radius` pixels from the centre.
fn point(geometry: &PolarGeometry, angle: f64, radius: f32) -> (f32, f32) {
    (
        geometry.center[0] + angle.cos() as f32 * radius,
        geometry.center[1] - angle.sin() as f32 * radius,
    )
}

/// Stroke the arc of `radius` over `(start, sweep)` as chords.
#[allow(clippy::too_many_arguments)]
fn stroke_arc(
    canvas: &mut Canvas,
    geometry: &PolarGeometry,
    radius: f32,
    start: f64,
    sweep: f64,
    paint: &Paint,
    stroker: &DecoStroker,
    tag: &str,
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
        stroker.stroke_segment(canvas, previous, next, paint, tag);
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
    inner + (geometry.radius - inner) * axis_fraction(value, axis)
}

/// Whether an angle-axis `value` draws: inside the sector, and on a full
/// circle not on the same spoke as an earlier value in `earlier`.
fn angle_drawn(polar: &PolarOptions, value: f64, earlier: &[f64]) -> bool {
    if !polar.contains_angle(value) {
        return false;
    }
    if polar.sector.is_some() {
        return true;
    }
    let turn = std::f64::consts::TAU / polar.angle_unit.radians_per_unit();
    !earlier.iter().any(|other| {
        let turns = (value - other) / turn;
        (turns - turns.round()).abs() * turn < 1e-6 * turn.max(1.0)
    })
}

/// Rings at the radius ticks and spokes at the angle ticks, from the grid
/// options: `*_y` fields are rings, `*_x` fields are spokes.
pub(super) fn draw_polar_grid(canvas: &mut Canvas, config: &Config, da: &DataArea, fp: FontPolicy) {
    let (Some(polar), Some(geometry)) = (&config.polar, config.polar_geometry()) else {
        return;
    };
    let g = &config.grid;
    let stroker = DecoStroker::from_style(&config.draw_style);
    let (start, sweep) = sweep(polar, &config.bottom_x);
    let inner = geometry.radius * polar.inner_radius_fraction;
    let rings = |canvas: &mut Canvas, values: Vec<f64>, paint: Paint, prefix: &str| {
        for (i, value) in values.into_iter().enumerate() {
            let radius = ring_radius(polar, &geometry, &config.left_y, value);
            if radius > inner + 0.5 && radius <= geometry.radius + 0.5 {
                let tag = format!("{prefix}_{i}");
                stroke_arc(
                    canvas, &geometry, radius, start, sweep, &paint, &stroker, &tag,
                );
            }
        }
    };
    let spokes = |canvas: &mut Canvas, values: Vec<f64>, paint: Paint, prefix: &str| {
        for (i, value) in values.iter().enumerate() {
            if !angle_drawn(polar, *value, &values[..i]) {
                continue;
            }
            let angle = polar.screen_angle(*value);
            let tag = format!("{prefix}_{i}");
            stroker.stroke_segment(
                canvas,
                point(&geometry, angle, inner),
                point(&geometry, angle, geometry.radius),
                &paint,
                &tag,
            );
        }
    };
    // Minor first so major paints over it, like the rectangular grid.
    if g.show_minor_x {
        spokes(
            canvas,
            minor_tick_values_for_axis(&config.bottom_x, Side::Bottom, da, fp),
            stroke_paint(&g.minor_x_color, g.minor_x_width, &g.minor_x_style),
            "grid_minor_x",
        );
    }
    if g.show_minor_y {
        rings(
            canvas,
            minor_tick_values_for_axis(&config.left_y, Side::Left, da, fp),
            stroke_paint(&g.minor_y_color, g.minor_y_width, &g.minor_y_style),
            "grid_minor_y",
        );
    }
    if g.show_major_x {
        spokes(
            canvas,
            major_tick_values_for_axis(&config.bottom_x, Side::Bottom, da, fp),
            stroke_paint(&g.major_x_color, g.major_x_width, &g.major_x_style),
            "grid_major_x",
        );
    }
    if g.show_major_y {
        rings(
            canvas,
            major_tick_values_for_axis(&config.left_y, Side::Left, da, fp),
            stroke_paint(&g.major_y_color, g.major_y_width, &g.major_y_style),
            "grid_major_y",
        );
    }
}

/// One tick label's text and extents, in the axis' label format.
fn tick_text(axis: &AxisOptions, value: f64, fp: FontPolicy) -> (NumericTickText, TextMetrics) {
    let ls = &axis.label_style;
    if matches!(ls.format, LabelFormat::Power) {
        let text = format_tick_power(value, ls.significant_digits, ls);
        let metrics = measure_rich_text(&text, fp);
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
        let metrics = measure_plain_text(&text, &ls.label_font, ls.font_size, false, false, fp);
        (NumericTickText::Plain(text), metrics)
    }
}

/// Draw `text` centred on `center` plus `offset`: the label offset fields
/// are read in polar frames (`PolarOptions::angle_label_offset`,
/// `PolarOptions::radius_label_offset`), not as screen x / y.
fn draw_centered(
    canvas: &mut Canvas,
    text: &NumericTickText,
    center: (f32, f32),
    offset: (f32, f32),
    m: TextMetrics,
    ls: &LabelStyle,
    fp: FontPolicy,
) {
    let origin = (
        center.0 - m.width * 0.5 + offset.0,
        center.1 + (m.ascent - m.descent) * 0.5 + offset.1,
    );
    match text {
        NumericTickText::Plain(text) => draw_plain_text(
            canvas,
            text,
            origin,
            ls.color,
            &ls.label_font,
            ls.font_size,
            false,
            false,
            fp,
        ),
        NumericTickText::Power(text) => draw_rich_text(canvas, text, origin, fp),
    }
}

/// The outward tick extent of an axis: how far its ticks reach past the
/// line, which is where its labels start.
fn outward_tick(axis: &AxisOptions) -> f32 {
    match axis.tick {
        TickVisibility::None | TickVisibility::Inside => 0.0,
        TickVisibility::Outside | TickVisibility::Both => axis.major_tick_length.max(0.0),
    }
}

/// The angle axis (`bottom_x`) as the outer ring with its ticks and labels,
/// and the radius axis (`left_y`) as the ray at the radial label angle.
pub(super) fn draw_polar_axes(
    canvas: &mut Canvas,
    config: &Config,
    da: &DataArea,
    stroker: &DecoStroker,
    fp: FontPolicy,
) {
    let (Some(polar), Some(geometry)) = (&config.polar, config.polar_geometry()) else {
        return;
    };
    let (start, sweep) = sweep(polar, &config.bottom_x);
    let inner = geometry.inner_radius;

    // Angle axis.
    let axis = &config.bottom_x;
    let paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
    if axis.line_visible {
        stroke_arc(
            canvas,
            &geometry,
            geometry.radius,
            start,
            sweep,
            &paint,
            stroker,
            "axis_bottom",
        );
    }
    let majors = major_tick_values_for_axis(axis, Side::Bottom, da, fp);
    let minors = minor_tick_values_for_axis(axis, Side::Bottom, da, fp);
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
            if !angle_drawn(polar, value, earlier) {
                continue;
            }
            let angle = polar.screen_angle(value);
            let (inside, outside) = match axis.tick {
                TickVisibility::None => continue,
                TickVisibility::Outside => (geometry.radius, geometry.radius + length),
                TickVisibility::Inside => (geometry.radius - length, geometry.radius),
                TickVisibility::Both => (geometry.radius - length, geometry.radius + length),
            };
            let tag = format!("tick_bottom_{i}");
            stroker.stroke_segment(
                canvas,
                point(&geometry, angle, inside),
                point(&geometry, angle, outside),
                &paint,
                &tag,
            );
        }
    }
    let ls = &axis.label_style;
    if ls.visible && ls.label_visible {
        // Keep every k-th label when the labels would overlap around the
        // ring: the same 1/2/5 stride rule the straight axes use.
        let gap = ls.font_size * 0.5;
        let circumference = (geometry.radius as f64 * sweep.abs()) as f32;
        let mut total = 0.0f32;
        let mut count = 0usize;
        for (i, value) in majors.iter().enumerate() {
            if angle_drawn(polar, *value, &majors[..i]) {
                total += tick_text(axis, *value, fp).1.width + gap;
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
            if !angle_drawn(polar, *value, &majors[..i]) {
                continue;
            }
            drawn += 1;
            if (drawn - 1) % stride != 0 {
                continue;
            }
            let (text, m) = tick_text(axis, *value, fp);
            let angle = polar.screen_angle(*value);
            let (dx, dy) = (angle.cos() as f32, -(angle.sin() as f32));
            // The label box sits just outside the ring along the spoke.
            let reach = band + dx.abs() * m.width * 0.5 + dy.abs() * m.height() * 0.5;
            let center = (
                geometry.center[0] + dx * (geometry.radius + reach),
                geometry.center[1] + dy * (geometry.radius + reach),
            );
            // Along the spoke, so every label keeps its distance to the ring.
            draw_centered(canvas, &text, center, (dx * radial, dy * radial), m, ls, fp);
        }
    }

    // Radius axis along the radial label ray.
    let axis = &config.left_y;
    let ray = polar.radial_label_angle_degrees.to_radians();
    let dir = (ray.cos() as f32, -(ray.sin() as f32));
    // Labels sit on the left of the ray seen from the centre.
    let side = (dir.1, -dir.0);
    let paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
    if axis.line_visible {
        stroker.stroke_segment(
            canvas,
            point(&geometry, ray, inner),
            point(&geometry, ray, geometry.radius),
            &paint,
            "axis_left",
        );
    }
    let majors = major_tick_values_for_axis(axis, Side::Left, da, fp);
    let minors = minor_tick_values_for_axis(axis, Side::Left, da, fp);
    if axis.tick != TickVisibility::None {
        for (i, (value, length)) in majors
            .iter()
            .map(|v| (*v, axis.major_tick_length))
            .chain(minors.iter().map(|v| (*v, axis.minor_tick_length)))
            .enumerate()
        {
            let radius = ring_radius(polar, &geometry, axis, value);
            if radius < inner - 0.5 || radius > geometry.radius + 0.5 {
                continue;
            }
            let at = point(&geometry, ray, radius);
            let (a, b) = match axis.tick {
                TickVisibility::None => continue,
                TickVisibility::Outside => (0.0, length),
                TickVisibility::Inside => (-length, 0.0),
                TickVisibility::Both => (-length, length),
            };
            let tag = format!("tick_left_{i}");
            stroker.stroke_segment(
                canvas,
                (at.0 + side.0 * a, at.1 + side.1 * a),
                (at.0 + side.0 * b, at.1 + side.1 * b),
                &paint,
                &tag,
            );
        }
    }
    let ls = &axis.label_style;
    if ls.visible && ls.label_visible {
        let offset = polar.radius_label_offset(ls);
        let labels = visible_numeric_labels(axis, Side::Left, fp, &majors, |value| {
            point(&geometry, ray, ring_radius(polar, &geometry, axis, value))
        });
        let band = outward_tick(axis) + 4.0;
        for label in &labels {
            let (dx, dy) = (
                label.position.0 - geometry.center[0],
                label.position.1 - geometry.center[1],
            );
            let radius = dx.hypot(dy);
            if radius < inner - 0.5 || radius > geometry.radius + 0.5 {
                continue;
            }
            let m = extents(&label.text, ls, fp);
            let reach = band + side.0.abs() * m.width * 0.5 + side.1.abs() * m.height() * 0.5;
            // Beside the tick and half a line inward, so the outermost label
            // stays clear of the angle label on the same spoke.
            let inward = m.height() * 0.5;
            let center = (
                label.position.0 + side.0 * reach - dir.0 * inward,
                label.position.1 + side.1 * reach - dir.1 * inward,
            );
            draw_centered(
                canvas,
                &label.text,
                center,
                (offset[0], offset[1]),
                m,
                ls,
                fp,
            );
        }
    }
}

fn extents(text: &NumericTickText, ls: &LabelStyle, fp: FontPolicy) -> TextMetrics {
    match text {
        NumericTickText::Plain(text) => {
            measure_plain_text(text, &ls.label_font, ls.font_size, false, false, fp)
        }
        NumericTickText::Power(text) => measure_rich_text(text, fp),
    }
}
