//! Where the chart's chrome goes (#160): the grid, the axes with their
//! ticks and labels, the colourbar's axis and labels, and the titles and
//! legend through [`crate::select::emit_element`], placed into a
//! [`Sink`] in the chart frame. Tick planning, formatting and label pruning
//! live here, measured through [`MeasureText`]; a renderer paints what the
//! sink receives, and selection, hit testing and drag containment read the
//! same placement under any measure.
//!
//! Public for the renderer crate, which paints it; not a host API, and with
//! no compatibility promise.

use crate::config::{AxisOptions, AxisScale, Config, LabelStyle, TickVisibility};
use crate::format::LabelFormat;
use crate::layout::{
    DataArea, Element, RectF, Side, axis_anchor, axis_offset, label_origin, label_rect,
    point_on_rect_side, visible_colorbar_rect,
};
use crate::select::emit_element;
use crate::sink::{PlacedText, Sink, Slot, StrokeStyle, TextRef};
use crate::text::{MeasureText, RichSegment, RichText, TextAlign, TextExtents};

mod tick_plan;
pub mod time;
pub use tick_plan::*;
mod grid;
pub use grid::*;
mod line;
pub use line::*;
mod tick_labels;
pub use tick_labels::*;
mod tick_format;
pub use tick_format::*;
mod side_tag;
pub use side_tag::*;
mod box3d;
mod parallel;
mod polar;
mod ternary;

/// Place every decoration of `config` into `sink`: the grid below the data,
/// then above it the axes with their ticks and labels, the colourbar, the
/// axis titles, the chart title and the legend. The font policy is the
/// text twin of the stroke strategy: sketch mode forces the bundled
/// handwritten face (with per-character fallback for glyphs it lacks),
/// threaded through every measure and placement so layout and raster always
/// agree. The elements the model places go through
/// [`emit_element`], with the renderer's measure.
pub fn emit_chart_chrome(config: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    let Ok(da) = config.data_area() else { return };
    // The Cartesian walks take the plot rectangle.
    let plot = data_area_rect(&da);

    if config.polar().is_some() {
        polar::emit_polar_grid(config, &da, measure, sink);
        // The angle axis is the outer ring, the radius axis the labelled
        // ray; the top and right axes are not drawn.
        polar::emit_polar_axes(config, &da, measure, sink);
    } else if config.ternary().is_some() {
        // The triangle's edges are the three components' axes; the top
        // axis is not drawn.
        ternary::emit_ternary_grid(config, &da, measure, sink);
        ternary::emit_ternary_axes(config, &da, measure, sink);
    } else if config.view_3d().is_some() {
        // The box's back panes carry the grid and three of its edges the
        // x, y and z axes; the top axis is not drawn.
        box3d::emit_box_grid(config, &da, measure, sink);
        box3d::emit_box_axes(config, &da, measure, sink);
    } else if config.parallel().is_some() {
        // The category axes name the parallel axes; each parallel axis
        // draws its own line, ticks and labels at its category. No grid:
        // the axes are the structure, and `left_y` is the normalized band,
        // not drawn (#141).
        for (axis, side) in [(&config.top_x, Side::Top), (&config.bottom_x, Side::Bottom)] {
            let cross = crate::layout::axis_cross(config, &side);
            emit_axis(axis, side, &plot, cross, measure, sink);
        }
        parallel::emit_parallel_axes(config, &da, measure, sink);
    } else {
        emit_grid(config, &plot, measure, sink);
        for (axis, side) in [
            (&config.top_x, Side::Top),
            (&config.bottom_x, Side::Bottom),
            (&config.left_y, Side::Left),
            (&config.right_y, Side::Right),
        ] {
            let cross = crate::layout::axis_cross(config, &side);
            emit_axis(axis, side, &plot, cross, measure, sink);
        }
    }

    emit_colorbar(config, &da, measure, sink);

    for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
        emit_element(config, &Element::AxisTitle(side), measure, sink);
    }
    emit_element(config, &Element::ChartTitle, measure, sink);
    emit_element(config, &Element::Legend, measure, sink);
}

/// Place the parts of decoration `element` this module lays out: tick
/// labels and axis strokes of the four axes and the colourbar, in
/// `config`'s chart frame, with the detached-axis offset applied. Each case
/// runs the same emit the raster draws with. Elements placed elsewhere
/// ([`crate::select::emit_element`]) return `false`.
pub fn emit_part_of(
    config: &Config,
    element: &Element,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) -> bool {
    let Ok(da) = config.data_area() else {
        return matches!(
            element,
            Element::AxisLabel(_)
                | Element::ColorBarLabel
                | Element::Axis(_)
                | Element::ColorBarAxis
        );
    };
    let axis_of = |side: &Side| match side {
        Side::Top => &config.top_x,
        Side::Bottom => &config.bottom_x,
        Side::Left => &config.left_y,
        Side::Right => &config.right_y,
    };
    match element {
        Element::AxisLabel(side) if config.polar().is_some() => match side {
            Side::Bottom => polar::emit_polar_angle_labels(config, &da, measure, sink),
            Side::Left => polar::emit_polar_radius_labels(config, &da, measure, sink),
            Side::Top | Side::Right => {}
        },
        Element::Axis(side) if config.polar().is_some() => {
            // The angle axis is the outer ring, the data boundary: no
            // element owns it.
            if *side == Side::Left {
                polar::emit_polar_radius_strokes(config, &da, measure, sink);
            }
        }
        Element::AxisLabel(side) if config.ternary().is_some() => {
            if let Some(k) = ternary::side_component(side) {
                ternary::emit_ternary_labels(config, &da, k, measure, sink);
            }
        }
        Element::Axis(side) if config.ternary().is_some() => {
            if let Some(k) = ternary::side_component(side) {
                ternary::emit_ternary_axis_strokes(config, &da, k, measure, sink);
            }
        }
        Element::AxisLabel(side) if config.view_3d().is_some() => {
            if let Some(k) = box3d::side_component(side) {
                box3d::emit_box_labels(config, &da, k, measure, sink);
            }
        }
        // A parallel chart's value axes are its parallel axes; `left_y` and
        // `right_y` are not drawn (#141).
        Element::AxisLabel(side) if config.parallel().is_some() => match side {
            Side::Top | Side::Bottom => {
                emit_axis_tick_labels(
                    axis_of(side),
                    side.clone(),
                    &data_area_rect(&da),
                    measure,
                    sink,
                );
            }
            Side::Left | Side::Right => {}
        },
        Element::Axis(side) if config.parallel().is_some() => match side {
            Side::Top | Side::Bottom => emit_axis_strokes(
                axis_of(side),
                side.clone(),
                &data_area_rect(&da),
                crate::layout::axis_cross(config, side),
                measure,
                sink,
            ),
            Side::Left | Side::Right => {}
        },
        Element::ParallelAxisLabel(i) => {
            parallel::emit_parallel_labels(config, &da, *i, measure, sink);
        }
        Element::Axis(side) if config.view_3d().is_some() => {
            if let Some(k) = box3d::side_component(side) {
                box3d::emit_box_axis_strokes(config, &da, k, measure, sink);
            }
        }
        Element::AxisLabel(side) => {
            emit_axis_tick_labels(
                axis_of(side),
                side.clone(),
                &data_area_rect(&da),
                measure,
                sink,
            );
        }
        Element::Axis(side) => {
            emit_axis_strokes(
                axis_of(side),
                side.clone(),
                &data_area_rect(&da),
                crate::layout::axis_cross(config, side),
                measure,
                sink,
            );
        }
        Element::ColorBarLabel => {
            if let Some((bar, rect)) = drawn_colorbar(config, &da) {
                emit_colorbar_tick_labels(&bar.axis, bar.side.clone(), &rect, measure, sink);
            }
        }
        Element::ColorBarAxis => {
            if let Some((bar, rect)) = drawn_colorbar(config, &da) {
                emit_colorbar_axis_strokes(&bar.axis, bar.side.clone(), &rect, sink);
            }
        }
        _ => return false,
    }
    true
}

/// The stroke of an axis line or tick of `axis`.
fn axis_stroke_style<'t>(axis: &AxisOptions, tag: &'t str) -> StrokeStyle<'t> {
    StrokeStyle {
        width: axis.line_width,
        color: axis.line_color,
        dash: axis.line_style.pattern(),
        tag,
    }
}

/// An axis drawn along an edge that is not a side of the plot rectangle (a
/// ternary triangle edge, a 3D box edge): the edge `line` when the axis line
/// is visible, then the major and minor ticks along the edge's outward
/// normal. `fraction` places a tick value on the edge (`None` skips it) and
/// `point(f, offset)` is the pixel point at fraction `f` of the edge, moved
/// `offset` pixels along its normal.
#[allow(clippy::too_many_arguments)]
fn emit_edge_axis_strokes(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
    line: ([f32; 2], [f32; 2]),
    fraction: impl Fn(f64) -> Option<f32>,
    point: impl Fn(f32, f32) -> [f32; 2],
) {
    let tag_side = side_tag(&side);
    if axis.line_visible {
        let tag = format!("axis_{tag_side}");
        sink.stroke(Slot::Above, line.0, line.1, &axis_stroke_style(axis, &tag));
    }
    if axis.tick == TickVisibility::None {
        return;
    }
    let plot = data_area_rect(da);
    let majors = major_tick_values_for_axis(axis, side.clone(), &plot, measure);
    let minors = minor_tick_values_for_axis(axis, side, &plot, measure);
    for (i, (value, length)) in majors
        .iter()
        .map(|v| (*v, axis.major_tick_length))
        .chain(minors.iter().map(|v| (*v, axis.minor_tick_length)))
        .enumerate()
    {
        let Some(f) = fraction(value) else {
            continue;
        };
        let (from, to) = match axis.tick {
            TickVisibility::None => continue,
            TickVisibility::Outside => (0.0, length),
            TickVisibility::Inside => (-length, 0.0),
            TickVisibility::Both => (-length, length),
        };
        let tag = format!("tick_{tag_side}_{i}");
        sink.stroke(
            Slot::Above,
            point(f, from),
            point(f, to),
            &axis_stroke_style(axis, &tag),
        );
    }
}

/// The colourbar: its strip and border (the model's `Element::ColorBar`
/// placement), its axis line and ticks, its tick labels and its title (the
/// model's `Element::ColorBarTitle` placement).
///
/// A no-op when there is no colourbar or it is hidden — and `colorbar_parts`
/// has then reserved no band for it, so there is nothing to leave blank.
pub fn emit_colorbar(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let Some((bar, rect)) = drawn_colorbar(config, da) else {
        return;
    };
    emit_element(config, &Element::ColorBar, measure, sink);
    emit_colorbar_axis_strokes(&bar.axis, bar.side.clone(), &rect, sink);
    emit_colorbar_tick_labels(&bar.axis, bar.side.clone(), &rect, measure, sink);
    emit_element(config, &Element::ColorBarTitle, measure, sink);
}

/// The colourbar axis' line and ticks along the strip `rect`
/// (`Element::ColorBarAxis`). Like a chart axis, the colourbar axis chrome
/// may detach perpendicular to its direction; the strip and title stay put,
/// and the labels share the same translation.
pub fn emit_colorbar_axis_strokes(
    axis: &AxisOptions,
    side: Side,
    rect: &RectF,
    sink: &mut dyn Sink,
) {
    let (off_x, off_y) = axis_offset(side.clone(), axis.line_offset);
    let majors = major_tick_values(axis);
    let dash = axis.line_style.pattern();
    each_colorbar_axis_stroke(axis, side, rect, &majors, |stroke, a, b| {
        let tag = match stroke {
            AxisStroke::Line => "axis_colorbar".to_string(),
            AxisStroke::Tick(i) => format!("tick_colorbar_{i}"),
            // A colourbar axis has no breaks (validation refuses them), so
            // its walk never reports one.
            AxisStroke::Break(k, end) => format!("axis_break_colorbar_{k}_{end}"),
            AxisStroke::Frame(k, end) => format!("axis_frame_colorbar_{k}_{end}"),
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

/// The colourbar and its strip rect when it is drawn
/// ([`crate::layout::visible_colorbar_rect`]). The renderer's
/// `tick_warnings` reports the bar's axis only then.
pub fn drawn_colorbar<'a>(
    config: &'a Config,
    _da: &DataArea,
) -> Option<(&'a crate::config::ColorBarOptions, RectF)> {
    visible_colorbar_rect(config)
}
