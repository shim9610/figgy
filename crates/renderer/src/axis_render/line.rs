//! Axis strokes: the axis line and its tick marks, and the value-to-screen
//! mapping along an axis that every axis element shares.

use super::*;

pub(super) fn draw_axis_segment(
    canvas: &mut Canvas,
    p0: (f32, f32),
    p1: (f32, f32),
    paint: &Paint,
    stroker: &DecoStroker,
    tag: &str,
) {
    stroker.stroke_segment(canvas, p0, p1, paint, tag);
}

pub(crate) fn draw_precise_axis_segment(
    canvas: &mut Canvas,
    p0: (f32, f32),
    p1: (f32, f32),
    color: Color,
    width: f32,
) {
    draw_axis_segment(
        canvas,
        p0,
        p1,
        &Paint::stroke(&color, width),
        &DecoStroker::Precise,
        "bounded_axis",
    );
}

pub(super) fn draw_axis(
    canvas: &mut Canvas,
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    stroker: &DecoStroker,
    fp: FontPolicy,
) {
    // Detached-axis offset: shift the whole axis chrome (line + ticks +
    // labels) perpendicular to the axis. The data area and grid stay put;
    // tick positions along the axis are unaffected.
    let (off_x, off_y) = axis_offset(side.clone(), axis.line_offset);
    let detached = off_x != 0.0 || off_y != 0.0;
    if detached {
        canvas.save();
        canvas.translate(off_x, off_y);
    }

    // 1) Axis line and 2) major / minor ticks, from the walk `drawn_parts`
    // reports.
    let majors = major_tick_values_for_axis(axis, side.clone(), da, fp);
    let paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
    each_axis_stroke(axis, side.clone(), da, fp, &majors, |stroke, a, b| {
        // Tick stroker tags share one running index per side (majors first,
        // minors after) so every tick gets its own wobble shape.
        let tag = match stroke {
            AxisStroke::Line => format!("axis_{}", side_tag(&side)),
            AxisStroke::Tick(i) => format!("tick_{}_{i}", side_tag(&side)),
        };
        draw_axis_segment(canvas, a, b, &paint, stroker, &tag);
    });

    // 3) Major tick labels
    each_axis_tick_label(axis, side.clone(), da, fp, &majors, |text, pos| {
        draw_tick_text(canvas, &text, pos, side.clone(), axis, fp);
    });

    if detached {
        canvas.restore();
    }
}

/// Where `value` falls along its axis, as a fraction of the axis' screen length
/// measured from the axis' visual start, `inverted` included.
///
/// **Not clamped**: a tick outside the range lands outside the axis, which is
/// what puts a stale tick visibly off the end instead of piled at it. The
/// colourbar's `ColorBarOptions::normalized_z` is the clamped z→colour twin, and
/// `colorbar_ticks_match_the_strip_ramp` pins the two to agree in range.
pub(crate) fn axis_fraction(value: f64, axis: &AxisOptions) -> f32 {
    let t = match axis.scale {
        AxisScale::Linear => {
            let range = axis.max - axis.min;
            if range == 0.0 {
                0.0
            } else {
                (value - axis.min) / range
            }
        }
        AxisScale::Logarithmic => {
            let (min, max) = crate::chart::guarded_log_range(axis.min, axis.max);
            let log_min = min.log10();
            let log_max = max.log10();
            let range = log_max - log_min;
            if range == 0.0 || value <= 0.0 {
                0.0
            } else {
                (value.log10() - log_min) / range
            }
        }
    };
    let t = if axis.inverted { 1.0 - t } else { t };
    t as f32
}

pub(super) fn data_area_rect(da: &DataArea) -> RectF {
    RectF {
        x: da.x as f32,
        y: da.y as f32,
        width: da.width as f32,
        height: da.height as f32,
    }
}

pub(crate) fn value_to_screen(
    value: f64,
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
) -> (f32, f32) {
    point_on_rect_side(axis_fraction(value, axis), &side, &data_area_rect(da))
}

/// Which stroke of an axis a segment is: its line, or tick `i` in the
/// running per-axis index (majors first, minors after).
#[derive(Clone, Copy)]
pub(crate) enum AxisStroke {
    Line,
    Tick(usize),
}

/// Every segment [`draw_axis`] strokes for `axis` on `side`, before the
/// detached-axis translate: the line, then the major and minor ticks.
/// Drawing and [`drawn_parts`] both walk this.
pub(super) fn each_axis_stroke(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
    majors: &[f64],
    mut each: impl FnMut(AxisStroke, (f32, f32), (f32, f32)),
) {
    if axis.line_visible {
        let (p0, p1) = axis_anchor(side.clone(), da);
        each(AxisStroke::Line, p0, p1);
    }
    if axis.tick == TickVisibility::None {
        return;
    }
    let minors = minor_tick_values_for_axis(axis, side.clone(), da, fp);
    let ticks = majors
        .iter()
        .map(|v| (*v, axis.major_tick_length))
        .chain(minors.iter().map(|v| (*v, axis.minor_tick_length)));
    for (i, (value, length)) in ticks.enumerate() {
        let pos = value_to_screen(value, axis, side.clone(), da);
        if let Some((a, b)) = tick_segment(pos, side.clone(), length, &axis.tick) {
            each(AxisStroke::Tick(i), a, b);
        }
    }
}

/// Every segment [`draw_colorbar`] strokes for the bar's axis along the strip
/// `rect`, before the detached-axis translate: the line, then the ticks.
pub(crate) fn each_colorbar_axis_stroke(
    axis: &AxisOptions,
    side: Side,
    rect: &RectF,
    majors: &[f64],
    mut each: impl FnMut(AxisStroke, (f32, f32), (f32, f32)),
) {
    // The bar's axis runs along the strip's outer long edge — the same side
    // of the strip as the bar is of the chart, which is what makes
    // `tick_segment`'s "outward = away from the data area" come out right.
    if axis.line_visible {
        let low = point_on_rect_side(0.0, &side, rect);
        let high = point_on_rect_side(1.0, &side, rect);
        each(AxisStroke::Line, low, high);
    }
    if axis.tick == TickVisibility::None {
        return;
    }
    // `major_tick_values` / `minor_tick_values` are the axis walkers, so a
    // logarithmic bar gets decades and 2a..9a minors with no extra code here.
    let minors = minor_tick_values(axis);
    let ticks = majors
        .iter()
        .map(|v| (*v, axis.major_tick_length))
        .chain(minors.iter().map(|v| (*v, axis.minor_tick_length)));
    for (i, (value, length)) in ticks.enumerate() {
        let pos = point_on_rect_side(axis_fraction(value, axis), &side, rect);
        if let Some((a, b)) = tick_segment(pos, side.clone(), length, &axis.tick) {
            each(AxisStroke::Tick(i), a, b);
        }
    }
}

/// The segment a tick at `pos` is stroked along, or `None` when ticks are
/// hidden. Drawing and [`drawn_parts`] both use it.
pub(super) fn tick_segment(
    pos: (f32, f32),
    side: Side,
    length: f32,
    visibility: &TickVisibility,
) -> Option<((f32, f32), (f32, f32))> {
    // outward direction = away from the data area.
    let (dx_out, dy_out) = match side {
        Side::Top => (0.0, -1.0),
        Side::Bottom => (0.0, 1.0),
        Side::Left => (-1.0, 0.0),
        Side::Right => (1.0, 0.0),
    };
    let outside = (pos.0 + dx_out * length, pos.1 + dy_out * length);
    let inside = (pos.0 - dx_out * length, pos.1 - dy_out * length);
    match visibility {
        TickVisibility::None => None,
        TickVisibility::Outside => Some((pos, outside)),
        TickVisibility::Inside => Some((pos, inside)),
        TickVisibility::Both => Some((inside, outside)),
    }
}

// Skia Paint helpers.

pub(crate) fn stroke_paint(color: &Color, width: f32, style: &LineStylePreset) -> Paint {
    Paint::stroke(color, width).with_dash(style.pattern())
}
