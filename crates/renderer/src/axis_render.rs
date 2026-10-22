//! Axis renderer. Takes a `Config` and draws axes, ticks, labels, and titles
//! onto the CPU raster [`Canvas`]. No data is drawn here — only axis chrome.
//!
//! Text rendering is delegated to [`crate::text_render`].

use crate::raster::{Canvas, Paint};

use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, Config, LabelStyle, TickVisibility};
use crate::format::LabelFormat;
use crate::layout::{
    DataArea, DrawnPart, RectF, Side, axis_anchor, axis_offset, point_on_rect_side,
};
use crate::line::LineStylePreset;
use crate::render_warning::{RenderWarning, SpacingRejection, WarningAxis};
use crate::select::{SELECTION_PADDING, SelectionBox, SelectionOutline};
use crate::sketch::DecoStroker;
use crate::text_render::{
    FontPolicy, draw_plain_text, draw_rich_text, measure_plain_text, measure_rich_text,
};

pub(crate) use crate::data_render::contour_label_atlas::{
    bake_contour_label_atlas, contour_label_texts,
};

pub use crate::decoration_render::draw_selection_boxes;
use crate::decoration_render::*;

mod space;
pub(crate) use space::*;

mod tick_plan;
pub(crate) use tick_plan::*;

mod grid;
use grid::*;

mod line;
pub(crate) use line::*;

// Public entry.

/// Which layer of the axis raster to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisLayerKind {
    /// Grid lines + minor ticks only. Should be composited *below* data so
    /// data is not obscured.
    Grid,
    /// Axis lines, tick labels, axis titles, chart title, legend. Composited
    /// *above* data.
    Decoration,
    /// Both, in one layer (legacy single-pass). New code should use the
    /// separate Grid / Decoration layers.
    All,
}

/// Rasterize the axes into an RGBA8 premultiplied buffer matching
/// `config.chart_area` in size.
///
/// Used as the **CPU raster** that the renderer uploads to a wgpu texture.
/// The returned buffer is `width * height * 4` bytes; each pixel is
/// `[R, G, B, A]` with premultiplied alpha.
///
/// Returns [`crate::FiggyError::InvalidChartArea`] /
/// [`crate::FiggyError::RasterWrapFailed`] instead of panicking.
pub fn try_raster_chart_to_rgba(config: &Config) -> crate::Result<Vec<u8>> {
    try_raster_chart_layer_to_rgba(config, AxisLayerKind::All)
}

/// Rasterize a single layer — used to composite grid below and decoration
/// above the GPU data layer.
pub fn try_raster_chart_layer_to_rgba(
    config: &Config,
    layer: AxisLayerKind,
) -> crate::Result<Vec<u8>> {
    try_raster_chart_layer_to_rgba_with_selection(config, layer, &[])
}

/// [`try_raster_chart_layer_to_rgba`] plus selection highlight boxes drawn on
/// top (Decoration / All layers only — selection never sits under the data).
///
/// `selection` comes from `Selectable::selection_box(cfg, …)` and is in
/// absolute chart-surface coordinates; this function shifts it into the
/// chart_area-relative raster frame.
pub fn try_raster_chart_layer_to_rgba_with_selection(
    config: &Config,
    layer: AxisLayerKind,
    selection: &[SelectionBox],
) -> crate::Result<Vec<u8>> {
    let raster_cfg = raster_frame_config(config)?;
    let w = raster_cfg.chart_area.0.width;
    let h = raster_cfg.chart_area.0.height;

    let Some(mut canvas) = Canvas::new(w, h) else {
        return Err(crate::FiggyError::RasterWrapFailed {
            reason: format!("raster target allocation failed for {w}x{h}"),
        });
    };
    match layer {
        AxisLayerKind::Grid => draw_grid_layer(&mut canvas, &raster_cfg),
        AxisLayerKind::Decoration => draw_decoration_layer(&mut canvas, &raster_cfg),
        AxisLayerKind::All => draw_axes(&mut canvas, &raster_cfg),
    }
    // Milkyway: the axis chrome reads as line-light — bloom it. Runs
    // BEFORE the selection overlay so interaction chrome stays crisp.
    if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style
        && matches!(layer, AxisLayerKind::Decoration | AxisLayerKind::All)
    {
        apply_decoration_glow(&mut canvas, c.glow);
    }
    if !selection.is_empty() && matches!(layer, AxisLayerKind::Decoration | AxisLayerKind::All) {
        let ox = config.chart_area.0.x as f32;
        let oy = config.chart_area.0.y as f32;
        let local: Vec<SelectionBox> = selection
            .iter()
            .map(|b| SelectionBox {
                rect: b.rect.translated(-ox, -oy),
                handles: b.handles.iter().map(|h| h.translated(-ox, -oy)).collect(),
                ..b.clone()
            })
            .collect();
        let fp = FontPolicy::for_style(&raster_cfg.draw_style);
        draw_chart_selection_boxes(&mut canvas, &local, &raster_cfg, fp);
    }

    Ok(canvas.into_rgba())
}

/// [`draw_selection_boxes`] in one chart's raster: a selection of polar tick
/// labels ([`SelectionOutline::TickLabels`]) boxes each label where `config`
/// draws it, instead of one box around the whole ring.
fn draw_chart_selection_boxes(
    canvas: &mut Canvas,
    boxes: &[SelectionBox],
    config: &Config,
    fp: FontPolicy,
) {
    for b in boxes {
        match &b.outline {
            SelectionOutline::TickLabels(side) if config.polar.is_some() => {
                let outline = Paint::stroke(&b.color, b.stroke_width);
                polar::polar_tick_label_rects(config, side.clone(), fp, |label| {
                    let r = label.expanded(SELECTION_PADDING);
                    canvas.draw_rect(r.x, r.y, r.width, r.height, &outline);
                });
            }
            _ => draw_selection_boxes(canvas, std::slice::from_ref(b)),
        }
    }
}

/// `config` in the CPU raster's frame: the raster buffer covers exactly the
/// chart area, so the chart area moves to the origin. [`tick_warnings`] derives
/// from the same frame as the raster, so the two cannot disagree about the
/// data area.
fn raster_frame_config(config: &Config) -> crate::Result<Config> {
    use crate::layout::{ChartArea, Rect};

    let w = config.chart_area.0.width;
    let h = config.chart_area.0.height;
    if w == 0 || h == 0 {
        return Err(crate::FiggyError::InvalidChartArea {
            width: w,
            height: h,
        });
    }

    let mut raster_cfg = config.clone();
    raster_cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    });
    Ok(raster_cfg)
}

/// Panic-on-error wrapper around [`try_raster_chart_to_rgba`]. New code
/// should prefer the fallible version; this exists for the binary demos.
pub fn raster_chart_to_rgba(config: &Config) -> Vec<u8> {
    try_raster_chart_to_rgba(config).expect("raster_chart_to_rgba failed")
}

/// Grid layer — only the parts that should sit below the data layer.
///
/// The constellation style repurposes this slot as its deep-space backdrop
/// (the compositing order already puts it under the data): grid lines are
/// not drawn in that mode — the style declares its own background instead.
pub fn draw_grid_layer(canvas: &mut Canvas, config: &Config) {
    if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style {
        draw_space_background(canvas, c.seed, c.nebula, c.dust);
        return;
    }
    let Ok(da) = config.data_area() else { return };
    let fp = FontPolicy::for_style(&config.draw_style);
    draw_grid(canvas, config, &da, fp);
}

/// Decoration layer — axis lines, tick labels, axis titles, chart title,
/// legend. Drawn above the data so data never overlaps axis chrome.
pub fn draw_decoration_layer(canvas: &mut Canvas, config: &Config) {
    let Ok(da) = config.data_area() else { return };

    // The decoration stroke strategy is derived once per layer entry and
    // threaded down: `Precise` keeps every draw below on
    // the plain pre-stroker canvas calls. The font policy is its text twin —
    // sketch mode forces the bundled handwritten face (with per-character
    // fallback for glyphs it lacks), threaded through every measure + draw so
    // layout and raster always agree.
    let stroker = DecoStroker::from_style(&config.draw_style);
    let fp = FontPolicy::for_style(&config.draw_style);

    if config.polar.is_some() {
        // The angle axis is the outer ring, the radius axis the labelled
        // ray; the top and right axes are not drawn.
        polar::draw_polar_axes(canvas, config, &da, &stroker, fp);
    } else {
        draw_axis(canvas, &config.top_x, Side::Top, &da, &stroker, fp);
        draw_axis(canvas, &config.bottom_x, Side::Bottom, &da, &stroker, fp);
        draw_axis(canvas, &config.left_y, Side::Left, &da, &stroker, fp);
        draw_axis(canvas, &config.right_y, Side::Right, &da, &stroker, fp);
    }

    draw_colorbar(canvas, config, &da, &stroker, fp);

    if config.polar.is_none() {
        draw_axis_title(canvas, config, &da, Side::Top, fp);
    }
    draw_axis_title(canvas, config, &da, Side::Bottom, fp);
    draw_axis_title(canvas, config, &da, Side::Left, fp);
    if config.polar.is_none() {
        draw_axis_title(canvas, config, &da, Side::Right, fp);
    }

    if config.chart_title.visible {
        draw_chart_title(canvas, config, fp);
    }

    if config.legend.visible && !config.legend.content.segments.is_empty() {
        draw_legend(canvas, config, &da, &stroker, fp);
    }
}

/// Draw the full axis chrome (grid + decoration) into the regions specified
/// by `config`. If `data_area()` fails, this is a no-op.
pub fn draw_axes(canvas: &mut Canvas, config: &Config) {
    draw_grid_layer(canvas, config);
    draw_decoration_layer(canvas, config);
}

#[path = "bounded_axes.rs"]
mod bounded;
#[path = "polar_axes.rs"]
mod polar;
pub(crate) use bounded::{
    BoundedAxisDecorations, BoundedLegend, draw_bounded_decorations, draw_bounded_legend,
    plain_number, plan_bounded_legend,
};

/// Call `each` with every part of decoration `element` that
/// [`draw_decoration_layer`] draws for `config`, in `config`'s chart frame
/// with the detached-axis offset applied, and return `true`: one
/// [`DrawnPart::Text`] per tick label of an axis or the colourbar, and the
/// line and ticks of an axis or the colourbar's axis as
/// [`DrawnPart::Segment`]s. Other elements return `false`. This is the
/// renderer's [`crate::text::MeasureText::drawn_parts`]; each case walks the
/// same function its drawing walks.
pub(crate) fn drawn_parts(
    config: &Config,
    element: &crate::layout::Element,
    fp: FontPolicy,
    each: &mut dyn FnMut(DrawnPart),
) -> bool {
    use crate::layout::{Element, label_rect};

    let segment =
        |a: (f32, f32), b: (f32, f32), (dx, dy): (f32, f32), width: f32| DrawnPart::Segment {
            a: [a.0 + dx, a.1 + dy],
            b: [b.0 + dx, b.1 + dy],
            width,
        };
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
        Element::AxisLabel(side) if config.polar.is_some() => {
            polar::polar_tick_label_rects(config, side.clone(), fp, |r| each(DrawnPart::Text(r)));
        }
        Element::Axis(side) if config.polar.is_some() => {
            polar::polar_axis_segments(config, side.clone(), fp, |a, b, width| {
                each(segment(a, b, (0.0, 0.0), width));
            });
        }
        Element::AxisLabel(side) => {
            let axis = axis_of(side);
            let majors = major_tick_values_for_axis(axis, side.clone(), &da, fp);
            let (dx, dy) = axis_offset(side.clone(), axis.line_offset);
            each_axis_tick_label(axis, side.clone(), &da, fp, &majors, |text, pos| {
                let (origin, m) = tick_label_placement(&text, pos, side.clone(), axis, fp);
                each(DrawnPart::Text(label_rect(origin, m).translated(dx, dy)));
            });
        }
        Element::Axis(side) => {
            let axis = axis_of(side);
            let majors = major_tick_values_for_axis(axis, side.clone(), &da, fp);
            let offset = axis_offset(side.clone(), axis.line_offset);
            each_axis_stroke(axis, side.clone(), &da, fp, &majors, |_, a, b| {
                each(segment(a, b, offset, axis.line_width));
            });
        }
        Element::ColorBarLabel => {
            if let Some((bar, rect)) = drawn_colorbar(config, &da) {
                let axis = &bar.axis;
                let majors = major_tick_values(axis);
                let (dx, dy) = axis_offset(bar.side.clone(), axis.line_offset);
                each_colorbar_tick_label(
                    axis,
                    bar.side.clone(),
                    &rect,
                    fp,
                    &majors,
                    |text, pos| {
                        let (origin, m) =
                            tick_label_placement(&text, pos, bar.side.clone(), axis, fp);
                        each(DrawnPart::Text(label_rect(origin, m).translated(dx, dy)));
                    },
                );
            }
        }
        Element::ColorBarAxis => {
            if let Some((bar, rect)) = drawn_colorbar(config, &da) {
                let axis = &bar.axis;
                let majors = major_tick_values(axis);
                let offset = axis_offset(bar.side.clone(), axis.line_offset);
                each_colorbar_axis_stroke(axis, bar.side.clone(), &rect, &majors, |_, a, b| {
                    each(segment(a, b, offset, axis.line_width));
                });
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests;
