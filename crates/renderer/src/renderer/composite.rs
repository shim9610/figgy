//! The compositor: the z-order of a frame's chart panels.
//!
//! A renderer owns its elements' placement, raster and sinks; a slot is a z
//! position in the composite, declared by each element where it is emitted
//! ([`crate::sink::Slot`]); this file alone knows the order the slots are
//! drawn in. No renderer knows it.
//!
//! One panel, in slot order:
//!
//! ```text
//! Slot::Below   the grid texture (space background, grid, polar rings and spokes)
//!               → the annotation underlay (spans)
//! Data          every fill → every body → every picked highlight,
//!               in SSOT series order (`data_render::composite`),
//!               scissored to the data area
//! Slot::Above   the decoration texture (axis lines and ticks, tick labels,
//!               titles, legend, colourbar, selection boxes)
//!               → the annotation overlay (lines, arrows, text)
//! ```
//!
//! A frame of several panels (the base, overlay and subplot panels of a
//! layered document) draws the panels in [`FRAME_SLOTS`] order: the `Below`
//! slot and the data of every panel first, in panel order, then the `Above`
//! slot of every panel, in panel order. So on every layer the decoration is
//! above the data, as on a single-layer chart: an overlay's data never covers
//! the base panel's axes and labels. The selection boxes are inside the
//! decoration texture of the topmost panel, and the document's annotation
//! overlay is on the topmost panel of the parent's area
//! (`frame::lift_annotation_overlays`), so both stay above every layer's
//! decoration.
//!
//! The textures stay per layer so SSOT invalidation keeps refreshing one
//! layer at a time: the raster of the two chrome slots is one placement pass
//! (`axis_render::raster_panel_layers`), the annotation raster another, and
//! the data slot is issued from the series layers or blitted from the
//! resident composite that caches exactly its fills and bodies.

use crate::data_render::{AxisLayer, SeriesLayers, clamp_rect_to_target};
use crate::layout::Rect;
use crate::sink::Slot;

/// The order a frame draws its panels' slots in: every panel's
/// [`Slot::Below`] with its data, then every panel's [`Slot::Above`].
pub(crate) const FRAME_SLOTS: [Slot; 2] = [Slot::Below, Slot::Above];

/// A full-screen textured layer of one slot: the pipeline that blits a
/// panel-sized texture and the texture's bind group.
pub(crate) struct PanelChrome<'a> {
    /// The grid texture: what the chart emits into [`crate::sink::Slot::Below`].
    pub below: AxisLayer<'a>,
    /// The annotation spans, when the layer draws any.
    pub annotation_underlay: Option<AxisLayer<'a>>,
    /// The decoration texture: what the chart emits into
    /// [`crate::sink::Slot::Above`].
    pub above: AxisLayer<'a>,
    /// The annotation lines, arrows and text, when the layer draws any.
    pub annotation_overlay: Option<AxisLayer<'a>>,
}

/// Draw one chart panel's `slot`: [`Slot::Below`] draws the grid, the
/// annotation underlay and the data slot, [`Slot::Above`] the decoration and
/// the annotation overlay. Both in turn draw the whole panel; a frame of
/// several panels draws each slot for every panel ([`FRAME_SLOTS`]). The
/// function configures viewport and scissor itself, so callers only supply
/// rects.
///
/// `target_size` is the pixel size of the current color attachment;
/// `panel_rect` / `data_area` are clamped to it to avoid wgpu validation
/// errors when a panel partially exits the surface. The data slot's fills
/// and bodies come from `cached_bodies` when given: a panel-sized texture
/// of exactly what `issue_series_bodies` draws. The picked highlights
/// always come from `series_list`, so a selection never needs the bodies
/// drawn again. `series_list` and `cached_bodies` are read by the `Below`
/// slot only.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_chart_panel(
    pass: &mut wgpu::RenderPass<'_>,
    target_size: (u32, u32),
    panel_rect: Rect,
    data_area: Rect,
    chrome: &PanelChrome<'_>,
    series_list: &[SeriesLayers<'_>],
    cached_bodies: Option<AxisLayer<'_>>,
    slot: Slot,
) {
    let Some(panel_clamped) = clamp_rect_to_target(panel_rect, target_size) else {
        return;
    };
    let Some(data_clamped) = clamp_rect_to_target(data_area, target_size) else {
        return;
    };

    pass.set_viewport(
        panel_clamped.x as f32,
        panel_clamped.y as f32,
        panel_clamped.width as f32,
        panel_clamped.height as f32,
        0.0,
        1.0,
    );
    let panel_scissor = |pass: &mut wgpu::RenderPass<'_>| {
        pass.set_scissor_rect(
            panel_clamped.x,
            panel_clamped.y,
            panel_clamped.width,
            panel_clamped.height,
        );
    };
    let data_scissor = |pass: &mut wgpu::RenderPass<'_>| {
        pass.set_scissor_rect(
            data_clamped.x,
            data_clamped.y,
            data_clamped.width,
            data_clamped.height,
        );
    };
    let blit = |pass: &mut wgpu::RenderPass<'_>, layer: &AxisLayer<'_>| {
        pass.set_pipeline(layer.pipeline);
        pass.set_bind_group(0, layer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    };

    match slot {
        Slot::Below => {
            // Slot::Below: the grid, then the annotation spans over it.
            panel_scissor(pass);
            blit(pass, &chrome.below);
            if let Some(layer) = &chrome.annotation_underlay {
                blit(pass, layer);
            }

            // The data slot, scissored to the data area: every fill, then
            // every body, then every picked highlight, each in SSOT series
            // order.
            data_scissor(pass);
            match cached_bodies {
                Some(layer) => {
                    // The composite was clipped to the data area when it was
                    // drawn; clipping its blit again would apply the data
                    // scissor twice at the edge samples, which a direct draw
                    // applies once.
                    panel_scissor(pass);
                    blit(pass, &layer);
                    data_scissor(pass);
                }
                None => crate::data_render::issue_series_bodies(pass, series_list),
            }
            for s in series_list {
                crate::data_render::issue_series_picked(pass, s);
            }
        }
        Slot::Above => {
            // Slot::Above: the decoration, then the annotation overlay on top
            // of everything.
            panel_scissor(pass);
            blit(pass, &chrome.above);
            if let Some(layer) = &chrome.annotation_overlay {
                blit(pass, layer);
            }
        }
    }
}

/// The panel draw in slot order without a cached data slot: the panel draw of a
/// host that issues every series itself.
#[allow(clippy::too_many_arguments)]
pub fn draw_chart_panel_columnar(
    pass: &mut wgpu::RenderPass<'_>,
    target_size: (u32, u32),
    panel_rect: Rect,
    data_area: Rect,
    grid: AxisLayer<'_>,
    annotation_underlay: Option<AxisLayer<'_>>,
    series_list: &[SeriesLayers<'_>],
    decoration: AxisLayer<'_>,
    annotation_overlay: Option<AxisLayer<'_>>,
) {
    let chrome = PanelChrome {
        below: grid,
        annotation_underlay,
        above: decoration,
        annotation_overlay,
    };
    for slot in FRAME_SLOTS {
        draw_chart_panel(
            pass,
            target_size,
            panel_rect,
            data_area,
            &chrome,
            series_list,
            None,
            slot,
        );
    }
}
