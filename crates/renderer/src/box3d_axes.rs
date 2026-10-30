//! The grid and axes of a 3D chart (`Config.view_3d`): grid lines of each
//! axis on the box's three back panes for the current camera, and the
//! three axes on the box edges `Config::box_axis_edge` chooses (x and y on
//! the floor edges nearest the eye, z on the left-most vertical edge), each
//! with its ticks and upright labels outside the box. Every point is
//! projected through `Config::project_3d`, the camera the GPU draws with,
//! and every part goes through the sink, so drawing, hit regions, selection
//! outlines and the vector exports share one placement.

use super::*;
use crate::sink::{Sink, Slot, StrokeStyle};
use crate::view3d::{BoxEdge, box_axis};

pub(super) use crate::ternary::side_component;

/// The axis fraction of `value` on `axis`, `0..=1` inside its range, read
/// as the GPU reads it: logarithmic and inverted axes included.
fn fraction(axis: &AxisOptions, value: f64) -> Option<f64> {
    let (lo, hi, v) = match axis.scale {
        AxisScale::Logarithmic => {
            let (lo, hi) = crate::chart::guarded_log_range(axis.min, axis.max);
            if value <= 0.0 {
                return None;
            }
            (lo.log10(), hi.log10(), value.log10())
        }
        _ => {
            let (lo, hi) = axis.data_range();
            (lo, hi, value)
        }
    };
    let f = (v - lo) / (hi - lo);
    let f = if axis.inverted { 1.0 - f } else { f };
    (f.is_finite() && (-1e-9..=1.0 + 1e-9).contains(&f)).then_some(f.clamp(0.0, 1.0))
}

fn stroke_style<'t>(axis: &AxisOptions, tag: &'t str) -> StrokeStyle<'t> {
    StrokeStyle {
        width: axis.line_width,
        color: axis.line_color,
        dash: axis.line_style.pattern(),
        tag,
    }
}

/// The box grid below the data: on each back pane, a line of constant
/// value at each major tick of the pane's two axes, from edge to edge,
/// pane borders included. The x family takes the grid's x styles, y and z
/// its y styles.
pub(super) fn emit_box_grid(config: &Config, da: &DataArea, fp: FontPolicy, sink: &mut dyn Sink) {
    let Some(panes) = config.box_back_panes() else {
        return;
    };
    let grid = &config.grid;
    let plot = data_area_rect(da);
    for k in 0..3 {
        let (show, color, width, style) = if k == 0 {
            (
                grid.show_major_x,
                grid.major_x_color,
                grid.major_x_width,
                grid.major_x_style,
            )
        } else {
            (
                grid.show_major_y,
                grid.major_y_color,
                grid.major_y_width,
                grid.major_y_style,
            )
        };
        if !show {
            continue;
        }
        let axis = box_axis(config, k);
        let values = major_tick_values_for_axis(axis, box_side(k), &plot, fp);
        for (i, value) in values.into_iter().enumerate() {
            let Some(f) = fraction(axis, value) else {
                continue;
            };
            // The line of constant `k` runs across each back pane that
            // contains direction `k`: the panes across the two others.
            for across in (0..3).filter(|j| *j != k) {
                let along = 3 - k - across;
                let mut a = [0.0f64; 3];
                a[k] = f;
                a[across] = panes[across];
                let mut b = a;
                a[along] = 0.0;
                b[along] = 1.0;
                let (Some(pa), Some(pb)) = (config.project_3d(a), config.project_3d(b)) else {
                    continue;
                };
                let family = ["x", "y", "z"][k];
                let tag = format!("grid_major_{family}_{i}_{across}");
                sink.stroke(
                    Slot::Below,
                    [pa[0] as f32, pa[1] as f32],
                    [pb[0] as f32, pb[1] as f32],
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

/// The axis side of box direction `k`: x `Bottom`, y `Left`, z `Right`.
fn box_side(k: usize) -> Side {
    crate::ternary::component_side(k)
}

/// The box chrome above the data: each axis edge with its ticks, then each
/// axis' tick labels.
pub(super) fn emit_box_axes(config: &Config, da: &DataArea, fp: FontPolicy, sink: &mut dyn Sink) {
    for k in 0..3 {
        emit_box_axis_strokes(config, da, k, fp, sink);
    }
    for k in 0..3 {
        emit_box_labels(config, da, k, fp, sink);
    }
}

/// Axis `k`'s edge and ticks (`Element::Axis` of its side): the edge moved
/// out by `line_offset` along its normal, the ticks along the normal.
pub(super) fn emit_box_axis_strokes(
    config: &Config,
    da: &DataArea,
    k: usize,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let Some(edge) = config.box_axis_edge(k) else {
        return;
    };
    let axis = box_axis(config, k);
    let side = box_side(k);
    let tag_side = side_tag(&side);
    if axis.line_visible {
        let tag = format!("axis_{tag_side}");
        sink.stroke(Slot::Above, edge.a, edge.b, &stroke_style(axis, &tag));
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
        let a = tick_point(&edge, f, from);
        let b = tick_point(&edge, f, to);
        let tag = format!("tick_{tag_side}_{i}");
        sink.stroke(Slot::Above, a, b, &stroke_style(axis, &tag));
    }
}

/// The pixel point at fraction `f` of `edge`, `offset` along its normal.
fn tick_point(edge: &BoxEdge, f: f64, offset: f32) -> [f32; 2] {
    edge.point(f as f32, offset)
}

/// Axis `k`'s tick labels (`Element::AxisLabel` of its side): centred past
/// the tick ends along the edge's normal, upright, moved by the label
/// offsets read along the edge (`label_offset_x`) and along the normal
/// (`label_offset_y`).
pub(super) fn emit_box_labels(
    config: &Config,
    da: &DataArea,
    k: usize,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let Some(edge) = config.box_axis_edge(k) else {
        return;
    };
    let axis = box_axis(config, k);
    let ls = &axis.label_style;
    if !ls.visible || !ls.label_visible {
        return;
    }
    let majors = major_tick_values_for_axis(axis, box_side(k), &data_area_rect(da), fp);
    let n = edge.normal;
    let along = edge.direction();
    let gap = super::polar::outward_tick(axis) + ls.font_size.max(0.0) / 3.0;
    // The x and y edges meet at the floor corner nearest the eye; the x
    // label there keeps it, as a ternary corner shows one label.
    let shared = (k == 1)
        .then(|| config.box_axis_edge(0))
        .flatten()
        .map(|x_edge| x_edge.t0[1]);
    for value in majors {
        let Some(f) =
            fraction(axis, value).filter(|f| shared.is_none_or(|corner| (f - corner).abs() > 1e-6))
        else {
            continue;
        };
        let (text, m) = super::polar::tick_text(axis, value, fp);
        let half = n[0].abs() * m.width * 0.5 + n[1].abs() * (m.ascent + m.descent) * 0.5;
        let at = edge.point(f as f32, gap + half);
        let center = (
            at[0] + along[0] * ls.label_offset_x + n[0] * ls.label_offset_y,
            at[1] + along[1] * ls.label_offset_x + n[1] * ls.label_offset_y,
        );
        super::polar::place_centered(sink, text.as_tick_text(), center, m, ls);
    }
}
