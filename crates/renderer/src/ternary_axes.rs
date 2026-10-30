//! The grid and axes of a ternary chart (`Config.ternary`): the triangle's
//! three edges as the axes of the three components (`bottom_x` x along the
//! bottom edge from R to L, `left_y` y along the left edge from L to T,
//! `right_y` c along the right edge from T to R), each with its ticks and
//! labels outside the triangle, and grid lines of constant x, y and c
//! across it. The geometry is `Config::ternary_geometry`, the triangle the
//! GPU transform places the samples in; every part goes through the sink,
//! so drawing, hit regions, selection boxes and the vector exports share
//! one placement.

use super::*;
use crate::sink::{Sink, Slot, StrokeStyle};
use crate::ternary::{TernaryGeometry, ternary_axis};
pub(super) use crate::ternary::{component_side, side_component};

/// The fraction of the triangle value `value` of `axis` stands for.
fn fraction(axis: &AxisOptions, value: f64) -> Option<f32> {
    let f = (value - axis.min) / (axis.max - axis.min);
    (f.is_finite() && (-1e-9..=1.0 + 1e-9).contains(&f)).then_some(f.clamp(0.0, 1.0) as f32)
}

/// The point at fraction `f` along edge `k` (from its 0 to its 100 %
/// corner), moved out by `offset` pixels along the edge's outward normal.
fn edge_point(g: &TernaryGeometry, k: usize, f: f32, offset: f32) -> (f32, f32) {
    let [a, b] = g.edge(k);
    let n = g.outward_normal(k);
    (
        a[0] + (b[0] - a[0]) * f + n[0] * offset,
        a[1] + (b[1] - a[1]) * f + n[1] * offset,
    )
}

/// The ternary grid below the data: for each component, a line of constant
/// value at each tick of its axis, from edge to edge (minor ticks first, so
/// major lines paint over them). The x family takes the grid's x styles, y
/// and c its y styles.
pub(super) fn emit_ternary_grid(
    config: &Config,
    da: &DataArea,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let Some(g) = config.ternary_geometry() else {
        return;
    };
    let grid = &config.grid;
    let plot = data_area_rect(da);
    for major in [false, true] {
        for k in 0..3 {
            let axis = ternary_axis(config, k);
            let (show, color, width, style) = match (k, major) {
                (0, false) => (
                    grid.show_minor_x,
                    grid.minor_x_color,
                    grid.minor_x_width,
                    grid.minor_x_style,
                ),
                (0, true) => (
                    grid.show_major_x,
                    grid.major_x_color,
                    grid.major_x_width,
                    grid.major_x_style,
                ),
                (_, false) => (
                    grid.show_minor_y,
                    grid.minor_y_color,
                    grid.minor_y_width,
                    grid.minor_y_style,
                ),
                (_, true) => (
                    grid.show_major_y,
                    grid.major_y_color,
                    grid.major_y_width,
                    grid.major_y_style,
                ),
            };
            if !show {
                continue;
            }
            let side = component_side(k);
            let values = if major {
                major_tick_values_for_axis(axis, side, &plot, fp)
            } else {
                minor_tick_values_for_axis(axis, side, &plot, fp)
            };
            let prefix = match (k, major) {
                (_, true) => "grid_major",
                (_, false) => "grid_minor",
            };
            let family = ["x", "y", "c"][k];
            for (i, value) in values.into_iter().enumerate() {
                let Some(f) = fraction(axis, value) else {
                    continue;
                };
                // The edges are drawn by the axes; a line at 0 or 100 % is
                // an edge or a corner.
                if f <= 1e-6 || f >= 1.0 - 1e-6 {
                    continue;
                }
                // Constant component k at f: from where the next component
                // takes the rest to where the one after it does.
                let mut a = [0.0f32; 3];
                let mut b = [0.0f32; 3];
                a[k] = f;
                b[k] = f;
                a[(k + 1) % 3] = 1.0 - f;
                b[(k + 2) % 3] = 1.0 - f;
                let tag = format!("{prefix}_{family}_{i}");
                sink.stroke(
                    Slot::Below,
                    g.point(a),
                    g.point(b),
                    &StrokeStyle {
                        width,
                        color,
                        dash: style.pattern(),
                        tag: &tag,
                    },
                );
            }
        }
    }
}

/// The ternary chrome above the data: each edge with its ticks, then each
/// edge's tick labels.
pub(super) fn emit_ternary_axes(
    config: &Config,
    da: &DataArea,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    for k in 0..3 {
        emit_ternary_axis_strokes(config, da, k, fp, sink);
    }
    for k in 0..3 {
        emit_ternary_labels(config, da, k, fp, sink);
    }
}

/// The stroke of an edge or a tick of `axis`.
fn stroke_style<'t>(axis: &AxisOptions, tag: &'t str) -> StrokeStyle<'t> {
    StrokeStyle {
        width: axis.line_width,
        color: axis.line_color,
        dash: axis.line_style.pattern(),
        tag,
    }
}

/// Edge `k` and its ticks (`Element::Axis` of its side): the edge moved out
/// by the axis' `line_offset` along its outward normal, the ticks along the
/// normal (outside, inside or both).
pub(super) fn emit_ternary_axis_strokes(
    config: &Config,
    da: &DataArea,
    k: usize,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let Some(g) = config.ternary_geometry() else {
        return;
    };
    let axis = ternary_axis(config, k);
    let side = component_side(k);
    let tag_side = side_tag(&side);
    let offset = axis.line_offset;
    if axis.line_visible {
        let a = edge_point(&g, k, 0.0, offset);
        let b = edge_point(&g, k, 1.0, offset);
        let tag = format!("axis_{tag_side}");
        sink.stroke(
            Slot::Above,
            [a.0, a.1],
            [b.0, b.1],
            &stroke_style(axis, &tag),
        );
    }
    if axis.tick == TickVisibility::None {
        return;
    }
    let plot = data_area_rect(da);
    let majors = major_tick_values_for_axis(axis, side.clone(), &plot, fp);
    let minors = minor_tick_values_for_axis(axis, side, &plot, fp);
    for (i, (value, length)) in majors
        .iter()
        .map(|v| (*v, axis.major_tick_length))
        .chain(minors.iter().map(|v| (*v, axis.minor_tick_length)))
        .enumerate()
    {
        let Some(f) = fraction(axis, value) else {
            continue;
        };
        let (from, to) = match axis.tick {
            TickVisibility::None => continue,
            TickVisibility::Outside => (0.0, length),
            TickVisibility::Inside => (-length, 0.0),
            TickVisibility::Both => (-length, length),
        };
        let a = edge_point(&g, k, f, offset + from);
        let b = edge_point(&g, k, f, offset + to);
        let tag = format!("tick_{tag_side}_{i}");
        sink.stroke(
            Slot::Above,
            [a.0, a.1],
            [b.0, b.1],
            &stroke_style(axis, &tag),
        );
    }
}

/// Edge `k`'s tick labels (`Element::AxisLabel` of its side), but its 0:
/// centred
/// outside the tick ends along the edge's outward normal, upright, moved by
/// the label offsets read along the edge (`label_offset_x`) and along the
/// normal (`label_offset_y`).
pub(super) fn emit_ternary_labels(
    config: &Config,
    da: &DataArea,
    k: usize,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let Some(g) = config.ternary_geometry() else {
        return;
    };
    let axis = ternary_axis(config, k);
    let ls = &axis.label_style;
    if !ls.visible || !ls.label_visible {
        return;
    }
    let side = component_side(k);
    let majors = major_tick_values_for_axis(axis, side, &data_area_rect(da), fp);
    let n = g.outward_normal(k);
    let [a, b] = g.edge(k);
    let len = (b[0] - a[0]).hypot(b[1] - a[1]).max(f32::EPSILON);
    let along = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
    // A third of the font as clearance past the tick, as the polar ring's
    // labels keep.
    let gap = super::polar::outward_tick(axis) + ls.font_size.max(0.0) / 3.0;
    for value in majors {
        // A corner shows its own component's 100 %: the 0 of the edge that
        // starts there would sit on it.
        let Some(f) = fraction(axis, value).filter(|f| *f > 1e-6) else {
            continue;
        };
        let (text, m) = super::polar::tick_text(axis, value, fp);
        // Half the label's extent along the normal, so its nearest edge
        // keeps the gap whatever the edge's slope.
        let half = n[0].abs() * m.width * 0.5 + n[1].abs() * (m.ascent + m.descent) * 0.5;
        let at = edge_point(&g, k, f, axis.line_offset + gap + half);
        let center = (
            at.0 + along[0] * ls.label_offset_x + n[0] * ls.label_offset_y,
            at.1 + along[1] * ls.label_offset_x + n[1] * ls.label_offset_y,
        );
        super::polar::place_centered(sink, text.as_tick_text(), center, m, ls);
    }
}
