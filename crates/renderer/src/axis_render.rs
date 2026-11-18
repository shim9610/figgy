//! Axis renderer. Takes a `Config` and places axes, ticks, labels, and
//! titles through the sink; the raster entry points paint them onto the CPU
//! raster [`Canvas`] with the canvas sink. No data is drawn here — only axis
//! chrome.
//!
//! Text rendering is delegated to [`crate::text_render`].

use crate::raster::Canvas;

use crate::config::{AxisOptions, AxisScale, Config, LogTickPolicy};
use crate::layout::Side;
use crate::render_warning::{RenderWarning, WarningAxis};
use crate::select::SelectionBox;
use crate::sketch::DecoStroker;
use crate::text_render::{CpuTextMeasure, FontPolicy};

pub(crate) use crate::data_render::{bake_contour_label_atlas, contour_label_texts};

use crate::decoration_render::*;
pub use crate::decoration_render::{draw_selection_box, draw_selection_boxes};

mod space;
pub(crate) use space::*;

mod warnings;
pub(crate) use warnings::*;

pub(crate) use crate::chrome::*;

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
/// `selection` comes from `SelectionGeometry::selection_box(cfg, …)` and is
/// in absolute chart-surface coordinates; this function shifts it into the
/// chart_area-relative raster frame.
pub fn try_raster_chart_layer_to_rgba_with_selection(
    config: &Config,
    layer: AxisLayerKind,
    selection: &[SelectionBox],
) -> crate::Result<Vec<u8>> {
    try_raster_chart_layer_to_rgba_with_selection_at(config, layer, selection, 1.0)
}

/// [`try_raster_chart_layer_to_rgba_with_selection`] for a `config` whose
/// pixel values carry the display or export `scale` (#205): what the
/// renderer rasters for a placed or exported document, where a broken
/// axis' piece frames snap to whole logical pixels and then scale, so a 2x
/// raster is the 1x raster doubled. The plain entry is this one at scale 1.
pub fn try_raster_chart_layer_to_rgba_with_selection_at(
    config: &Config,
    layer: AxisLayerKind,
    selection: &[SelectionBox],
    scale: f32,
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
        AxisLayerKind::Grid => draw_layers(Some(&mut canvas), None, &raster_cfg, scale)?,
        AxisLayerKind::Decoration => draw_layers(None, Some(&mut canvas), &raster_cfg, scale)?,
        AxisLayerKind::All => {
            draw_layers(Some(&mut canvas), None, &raster_cfg, scale)?;
            draw_layers(None, Some(&mut canvas), &raster_cfg, scale)?;
        }
    }
    if matches!(layer, AxisLayerKind::Decoration | AxisLayerKind::All) {
        finish_decoration_layer(&mut canvas, config, &raster_cfg, selection);
    }

    Ok(canvas.into_rgba())
}

/// The grid and decoration rasters of one panel from one placement pass:
/// what [`try_raster_chart_layer_to_rgba_with_selection`] produces for
/// `Grid` and `Decoration`, laid out once. With `grid` false the slot below
/// the data is not rastered (its buffer comes back empty): a panel whose
/// grid texture the caller fills otherwise, such as the cached Milkyway
/// backdrop, skips that work.
pub(crate) fn raster_panel_layers(
    config: &Config,
    selection: &[SelectionBox],
    grid: bool,
    scale: f32,
) -> crate::Result<(Vec<u8>, Vec<u8>)> {
    let raster_cfg = raster_frame_config(config)?;
    let w = raster_cfg.chart_area.0.width;
    let h = raster_cfg.chart_area.0.height;
    let allocation_failed = || crate::FiggyError::RasterWrapFailed {
        reason: format!("raster target allocation failed for {w}x{h}"),
    };
    let mut below = if grid {
        Some(Canvas::new(w, h).ok_or_else(allocation_failed)?)
    } else {
        None
    };
    let mut above = Canvas::new(w, h).ok_or_else(allocation_failed)?;
    draw_layers(below.as_mut(), Some(&mut above), &raster_cfg, scale)?;
    finish_decoration_layer(&mut above, config, &raster_cfg, selection);
    Ok((
        below.map(Canvas::into_rgba).unwrap_or_default(),
        above.into_rgba(),
    ))
}

/// The style post-processing and the selection overlay of a decoration
/// raster: the Milkyway glow runs before the selection so interaction chrome
/// stays crisp, and the selection boxes move from absolute chart-surface
/// coordinates into the raster frame.
fn finish_decoration_layer(
    canvas: &mut Canvas,
    config: &Config,
    raster_cfg: &Config,
    selection: &[SelectionBox],
) {
    if let crate::config::DrawStyle::Milkyway(c) = &raster_cfg.draw_style {
        apply_decoration_glow(canvas, c.glow);
    }
    let ox = config.chart_area.0.x as f32;
    let oy = config.chart_area.0.y as f32;
    for b in selection {
        draw_selection_box(canvas, b, [ox, oy], 1.0);
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

/// Paint the chart's chrome into the canvases of its two slots, either of
/// which may be absent: the one placement pass of every decoration, through
/// [`emit_chart_chrome`]. The constellation style's deep-space backdrop is
/// painted first into the slot below the data, in place of the grid.
/// `scale` is the scale `config`'s pixels carry ([`emit_chart_chrome`]).
fn draw_layers(
    below: Option<&mut Canvas>,
    above: Option<&mut Canvas>,
    config: &Config,
    scale: f32,
) -> crate::Result<()> {
    let mut below = below;
    if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style
        && let Some(canvas) = below.as_deref_mut()
    {
        draw_space_background(canvas, c.seed, c.nebula, c.dust);
    }
    let backdrop = matches!(config.draw_style, crate::config::DrawStyle::Milkyway(_));
    let stroker = DecoStroker::from_style(&config.draw_style);
    let fp = FontPolicy::for_style(&config.draw_style);
    let mut sink = CanvasSink::new(if backdrop { None } else { below }, above, stroker, fp);
    emit_chart_chrome(config, &CpuTextMeasure::with_policy(fp), &mut sink, scale);
    sink.finish()
}

/// Grid layer — only the parts that should sit below the data layer.
///
/// The constellation style repurposes this slot as its deep-space backdrop
/// (the compositing order already puts it under the data): grid lines are
/// not drawn in that mode — the style declares its own background instead.
pub fn draw_grid_layer(canvas: &mut Canvas, config: &Config) -> crate::Result<()> {
    draw_layers(Some(canvas), None, config, 1.0)
}

/// Decoration layer — axis lines, tick labels, axis titles, chart title,
/// legend. Drawn above the data so data never overlaps axis chrome.
pub fn draw_decoration_layer(canvas: &mut Canvas, config: &Config) -> crate::Result<()> {
    draw_layers(None, Some(canvas), config, 1.0)
}

/// Draw the full axis chrome (grid + decoration) into the regions specified
/// by `config`. If `data_area()` fails, this is a no-op. One canvas cannot
/// back both slots at once, so this entry runs the placement once per slot;
/// `raster_panel_layers` is the single pass the renderer uses.
pub fn draw_axes(canvas: &mut Canvas, config: &Config) -> crate::Result<()> {
    draw_grid_layer(canvas, config)?;
    draw_decoration_layer(canvas, config)
}

#[cfg(test)]
mod tests;
