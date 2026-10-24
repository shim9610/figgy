//! Axis strokes: the axis line and its tick marks, placed through the sink,
//! and the value-to-screen mapping along an axis that every axis element
//! shares.

use super::*;

/// One axis: its line and ticks, then its tick labels, every part moved by
/// the detached-axis offset (`axis.line_offset`, perpendicular to the
/// axis). The data area and grid stay put; tick positions along the axis are
/// unaffected.
pub(super) fn emit_axis(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    emit_axis_strokes(axis, side.clone(), da, fp, sink);
    emit_axis_tick_labels(axis, side, da, fp, sink);
}

/// The line and the major and minor ticks of `axis` on `side`
/// (`Element::Axis`), from the walk [`each_axis_stroke`] reports, offset by
/// the detached-axis translation. Tick stroker tags share one running index
/// per side (majors first, minors after) so every tick gets its own wobble
/// shape.
pub(crate) fn emit_axis_strokes(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let (off_x, off_y) = axis_offset(side.clone(), axis.line_offset);
    let majors = major_tick_values_for_axis(axis, side.clone(), da, fp);
    let dash = axis.line_style.pattern();
    each_axis_stroke(axis, side.clone(), da, fp, &majors, |stroke, a, b| {
        let tag = match stroke {
            AxisStroke::Line => format!("axis_{}", side_tag(&side)),
            AxisStroke::Tick(i) => format!("tick_{}_{i}", side_tag(&side)),
        };
        sink.stroke(
            Slot::Above,
            [a.0 + off_x, a.1 + off_y],
            [b.0 + off_x, b.1 + off_y],
            &StrokeStyle {
                width: axis.line_width,
                color: axis.line_color,
                dash,
                tag: &tag,
            },
        );
    });
}

/// The tick labels of `axis` on `side` (`Element::AxisLabel`): every label
/// [`each_axis_tick_label`] walks, placed at its tick and moved by the
/// detached-axis translation; a categorical axis' labels come from
/// [`emit_category_tick_labels`].
pub(crate) fn emit_axis_tick_labels(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    if let Some(categories) = &axis.categories {
        emit_category_tick_labels(axis, categories, side, da, fp, sink);
        return;
    }
    let majors = major_tick_values_for_axis(axis, side.clone(), da, fp);
    let offset = axis_offset(side.clone(), axis.line_offset);
    each_axis_tick_label(axis, side.clone(), da, fp, &majors, |text, pos| {
        place_tick_text(&text, pos, side.clone(), axis, fp, offset, sink);
    });
}

/// The tick labels of a categorical `axis` on `side`, part of the same
/// `Element::AxisLabel` as numeric labels: `categories[i].label` at category
/// `i` ([`each_category_tick_label`]), placed by [`place_tick_text`] in the
/// axis' label style and moved by the detached-axis translation, so hover,
/// hit, selection, both drag offsets and the picked highlight are the
/// numeric labels'.
pub(crate) fn emit_category_tick_labels(
    axis: &AxisOptions,
    categories: &[crate::config::Category],
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let offset = axis_offset(side.clone(), axis.line_offset);
    each_category_tick_label(
        axis,
        categories,
        side.clone(),
        fp,
        |value| value_to_screen(value, axis, side.clone(), da),
        |text, pos| place_tick_text(&text, pos, side.clone(), axis, fp, offset, sink),
    );
}

/// Where `value` falls along its axis ([`crate::layout::axis_fraction`]).
pub(crate) fn axis_fraction(value: f64, axis: &AxisOptions) -> f32 {
    crate::layout::axis_fraction(value, axis)
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

/// Every segment [`emit_axis_strokes`] places for `axis` on `side`, before
/// the detached-axis translation: the line, then the major and minor ticks.
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

/// Every segment the colourbar's axis is stroked along the strip `rect`,
/// before the detached-axis translation: the line, then the ticks.
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
/// hidden.
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
