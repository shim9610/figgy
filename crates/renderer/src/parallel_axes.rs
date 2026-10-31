//! The axes of a parallel coordinates chart (`Config.parallel`, #141): one
//! vertical axis per category of `bottom_x`, at the category's position,
//! each with its own range ([`crate::parallel::ParallelAxis`]), its line and
//! ticks walked as a left axis is (`each_axis_stroke`) and its tick labels
//! placed as a left axis' are (`emit_axis_tick_labels`), so hover, hit,
//! selection and drag are the Cartesian tick labels'. The categorical
//! `bottom_x` keeps the axis names, their drag and the title; the `left_y`
//! axis spans the normalized band and is not drawn. Every part goes through
//! the sink, so drawing, hit regions, selection boxes and the vector exports
//! share one placement.

use super::*;

/// The plot rectangle whose left edge is category `i`'s x: the axis walks
/// (`Side::Left`) read their x from it and their y span from the plot. The
/// x keeps its fraction, so the ticks and labels scale exactly with the
/// chart (the oracle's I5); only the line snaps, in
/// [`emit_parallel_axis_strokes`].
fn axis_plot(config: &Config, plot: &RectF, i: usize) -> RectF {
    let (x, _) = value_to_screen(i as f64, &config.bottom_x, Side::Bottom, plot);
    RectF {
        x,
        y: plot.y,
        width: (plot.x + plot.width - x).max(1.0),
        height: plot.height,
    }
}

/// The axis options of parallel axis `i`: the left axis' look over the
/// axis's own range and label offset.
fn axis_options(config: &Config, i: usize) -> Option<AxisOptions> {
    let parallel = config.parallel.as_ref()?;
    parallel
        .axes
        .get(i)
        .map(|axis| axis.axis_options(&config.left_y))
}

/// The parallel chrome above the data: every axis's line and ticks, then
/// every axis's tick labels.
pub(super) fn emit_parallel_axes(
    config: &Config,
    da: &DataArea,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let Some(parallel) = config.parallel.as_ref() else {
        return;
    };
    for i in 0..parallel.axes.len() {
        emit_parallel_axis_strokes(config, da, i, fp, sink);
    }
    for i in 0..parallel.axes.len() {
        emit_parallel_labels(config, da, i, fp, sink);
    }
}

/// Axis `i`'s line and ticks, tagged `paxis_{i}` and `ptick_{i}_{k}`.
pub(super) fn emit_parallel_axis_strokes(
    config: &Config,
    da: &DataArea,
    i: usize,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let Some(axis) = axis_options(config, i) else {
        return;
    };
    let plot = axis_plot(config, &data_area_rect(da), i);
    let (off_x, off_y) = axis_offset(Side::Left, axis.line_offset);
    let majors = major_tick_values_for_axis(&axis, Side::Left, &plot, fp);
    let dash = axis.line_style.pattern();
    each_axis_stroke(
        &axis,
        Side::Left,
        &plot,
        None,
        fp,
        &majors,
        |stroke, mut a, mut b| {
            let tag = match stroke {
                AxisStroke::Line => {
                    // The line lands on the centre of the category's pixel
                    // column, as `axis_anchor` snaps a crossing axis, so it
                    // draws one pixel wide at every category position.
                    let x = plot.x.floor() + 0.5;
                    a.0 = x;
                    b.0 = x;
                    format!("paxis_{i}")
                }
                AxisStroke::Tick(k) => format!("ptick_{i}_{k}"),
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
        },
    );
}

/// Axis `i`'s tick labels (`Element::ParallelAxisLabel(i)`): to the left of
/// its line, as a left axis' are, moved by the axis's `label_offset`.
pub(super) fn emit_parallel_labels(
    config: &Config,
    da: &DataArea,
    i: usize,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let Some(axis) = axis_options(config, i) else {
        return;
    };
    let plot = axis_plot(config, &data_area_rect(da), i);
    emit_axis_tick_labels(&axis, Side::Left, &plot, fp, sink);
}
