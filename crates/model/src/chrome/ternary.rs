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
pub use crate::ternary::{component_side, side_component};

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

/// How far the tick labels of edge `k` reach past the edge along its
/// outward normal, as [`emit_ternary_labels`] places them: the line offset,
/// the tick gap, the largest extent along the normal of a label of any
/// major (the drawn labels are those), and the label offset along it.
/// `None` when the labels are hidden or the chart is not ternary. The
/// edge's title keeps the title gap past this (#272).
pub(crate) fn ternary_label_reach(
    config: &Config,
    k: usize,
    measure: &dyn MeasureText,
) -> Option<f32> {
    let g = config.ternary_geometry()?;
    let axis = ternary_axis(config, k);
    let ls = &axis.label_style;
    if !ls.visible || !ls.label_visible {
        return None;
    }
    let n = g.outward_normal(k);
    let gap = super::polar::outward_tick(axis) + ls.font_size.max(0.0) / 3.0;
    // Every major the axis walks, past the corner's 0 as the labels skip
    // it, so the widest of them bounds the band without a list.
    crate::tick::major_ticks(axis)
        .filter(|value| fraction(axis, *value).is_some_and(|f| f > 1e-6))
        .map(|value| {
            let (_, m) = super::polar::tick_text(axis, value, measure);
            n[0].abs() * m.width + n[1].abs() * (m.ascent + m.descent)
        })
        .fold(None, |widest: Option<f32>, extent| {
            Some(widest.map_or(extent, |w| w.max(extent)))
        })
        .map(|widest| axis.line_offset + gap + widest + ls.label_offset_y.max(0.0))
}

/// The ternary grid below the data: for each component, a line of constant
/// value at each tick of its axis, from edge to edge (minor ticks first, so
/// major lines paint over them). The x family takes the grid's x styles, y
/// and c its y styles.
pub fn emit_ternary_grid(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
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
                major_tick_values_for_axis(axis, side, &plot, measure)
            } else {
                minor_tick_values_for_axis(axis, side, &plot, measure)
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
pub fn emit_ternary_axes(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    for k in 0..3 {
        emit_ternary_axis_strokes(config, da, k, measure, sink);
    }
    for k in 0..3 {
        emit_ternary_labels(config, da, k, measure, sink);
    }
}

/// Edge `k` and its ticks (`Element::Axis` of its side): the edge moved out
/// by the axis' `line_offset` along its outward normal, the ticks along the
/// normal (outside, inside or both).
pub fn emit_ternary_axis_strokes(
    config: &Config,
    da: &DataArea,
    k: usize,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let Some(g) = config.ternary_geometry() else {
        return;
    };
    let axis = ternary_axis(config, k);
    let offset = axis.line_offset;
    let point = |f: f32, by: f32| {
        let (x, y) = edge_point(&g, k, f, offset + by);
        [x, y]
    };
    super::emit_edge_axis_strokes(
        axis,
        component_side(k),
        da,
        measure,
        sink,
        (point(0.0, 0.0), point(1.0, 0.0)),
        |value| fraction(axis, value),
        point,
    );
}

/// Edge `k`'s tick labels (`Element::AxisLabel` of its side), but its 0:
/// centred
/// outside the tick ends along the edge's outward normal, upright, moved by
/// the label offsets read along the edge (`label_offset_x`) and along the
/// normal (`label_offset_y`).
pub fn emit_ternary_labels(
    config: &Config,
    da: &DataArea,
    k: usize,
    measure: &dyn MeasureText,
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
    let majors = major_tick_values_for_axis(axis, side, &data_area_rect(da), measure);
    let n = g.outward_normal(k);
    let along = g.edge_direction(k);
    // A third of the font as clearance past the tick, as the polar ring's
    // labels keep.
    let gap = super::polar::outward_tick(axis) + ls.font_size.max(0.0) / 3.0;
    for value in majors {
        // A corner shows its own component's 100 %: the 0 of the edge that
        // starts there would sit on it.
        let Some(f) = fraction(axis, value).filter(|f| *f > 1e-6) else {
            continue;
        };
        let (text, m) = super::polar::tick_text(axis, value, measure);
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
