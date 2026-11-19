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
use crate::color::Color;
use crate::config::ColorBarOptions;
use crate::sink::{Border, PathSegment};

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
    let parallel = config.parallel()?;
    parallel
        .axes
        .get(i)
        .map(|axis| axis.axis_options(&config.left_y))
}

/// The parallel chrome above the data: every axis's line and ticks, then
/// every axis's tick labels.
pub fn emit_parallel_axes(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let Some(parallel) = config.parallel() else {
        return;
    };
    for i in 0..parallel.axes.len() {
        emit_parallel_axis_strokes(config, da, i, measure, sink);
    }
    for i in 0..parallel.axes.len() {
        emit_parallel_labels(config, da, i, measure, sink);
    }
}

/// Axis `i`'s line and ticks, tagged `paxis_{i}` and `ptick_{i}_{k}`.
pub fn emit_parallel_axis_strokes(
    config: &Config,
    da: &DataArea,
    i: usize,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let Some(axis) = axis_options(config, i) else {
        return;
    };
    let plot = axis_plot(config, &data_area_rect(da), i);
    let (off_x, off_y) = axis_offset(Side::Left, axis.line_offset);
    let majors = major_tick_values_for_axis(&axis, Side::Left, &plot, measure);
    let dash = axis.line_style.pattern();
    // A parallel axis has no breaks, so the frame snap's scale is unused.
    each_axis_stroke(
        &axis,
        Side::Left,
        &plot,
        None,
        None,
        measure,
        &majors,
        1.0,
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
                // A parallel axis has no breaks (a parallel chart refuses
                // them), so its walk never reports one.
                AxisStroke::Break(k, end) => format!("pbreak_{i}_{k}_{end}"),
                AxisStroke::Frame(k, end) => format!("pframe_{i}_{k}_{end}"),
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

/// The side the axis names (`bottom_x`'s labels) are placed on: the top
/// with [`crate::parallel::ParallelOptions::names_on_top`], the bottom
/// otherwise.
pub(super) fn names_side(config: &Config) -> Side {
    if config.parallel().is_some_and(|p| p.names_on_top) {
        Side::Top
    } else {
        Side::Bottom
    }
}

/// Axis `i`'s tick labels (`Element::ParallelAxisLabel(i)`): to the left of
/// its line, as a left axis' are, moved by the axis's `label_offset`, each
/// over its backdrop box when the chart has a
/// [`crate::parallel::ParallelOptions::label_backdrop`].
pub fn emit_parallel_labels(
    config: &Config,
    da: &DataArea,
    i: usize,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let Some(axis) = axis_options(config, i) else {
        return;
    };
    let plot = axis_plot(config, &data_area_rect(da), i);
    match config.parallel().and_then(|p| p.label_backdrop) {
        Some(fill) => {
            let mut backed = Backdrop { fill, inner: sink };
            emit_axis_tick_labels(&axis, Side::Left, &plot, measure, &mut backed);
        }
        None => emit_axis_tick_labels(&axis, Side::Left, &plot, measure, sink),
    }
}

/// A sink that puts a box of `fill` under every text it passes on: the
/// text's rectangle grown by [`crate::parallel::LABEL_BACKDROP_PAD`] of its
/// font size on each side. Every other part passes through unchanged.
struct Backdrop<'a> {
    fill: Color,
    inner: &'a mut dyn Sink,
}

impl Sink for Backdrop<'_> {
    fn text(&mut self, slot: Slot, text: &PlacedText<'_>) {
        let size = match text.text {
            TextRef::Plain { size, .. } => size,
            TextRef::Rich(rich) => rich.font_size,
            TextRef::ScaledRich { text, scale } => text.font_size * scale,
        };
        let pad = size * crate::parallel::LABEL_BACKDROP_PAD;
        let rect = text.rect();
        let boxed = RectF {
            x: rect.x - pad,
            y: rect.y - pad,
            width: rect.width + 2.0 * pad,
            height: rect.height + 2.0 * pad,
        };
        self.inner.rect(slot, boxed, Some(self.fill), None);
        self.inner.text(slot, text);
    }
    fn stroke(&mut self, slot: Slot, a: [f32; 2], b: [f32; 2], style: &StrokeStyle<'_>) {
        self.inner.stroke(slot, a, b, style);
    }
    fn rect(&mut self, slot: Slot, rect: RectF, fill: Option<Color>, border: Option<Border<'_>>) {
        self.inner.rect(slot, rect, fill, border);
    }
    fn triangle(&mut self, slot: Slot, points: [[f32; 2]; 3], color: Color) {
        self.inner.triangle(slot, points, color);
    }
    fn path(&mut self, slot: Slot, segments: &[PathSegment], style: &StrokeStyle<'_>) {
        self.inner.path(slot, segments, style);
    }
    fn polygon(
        &mut self,
        slot: Slot,
        points: &[[f32; 2]],
        fill: Option<Color>,
        border: Option<&StrokeStyle<'_>>,
    ) {
        self.inner.polygon(slot, points, fill, border);
    }
    fn ramp(&mut self, slot: Slot, rect: RectF, bar: &ColorBarOptions) {
        self.inner.ramp(slot, rect, bar);
    }
}
