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
//! Data          every fill → every body → every picked highlight
//!               → every value label, in SSOT series order
//!               (`data_render::composite`), scissored to the data area
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

/// The most rects a panel's data is drawn in: a piece per cut plus one on
/// each axis (#200).
const MAX_DATA_PIECES: usize =
    (crate::config::MAX_AXIS_BREAKS + 1) * (crate::config::MAX_AXIS_BREAKS + 1);

/// An empty rect, filling the unused pieces.
const NO_RECT: Rect = Rect {
    x: 0,
    y: 0,
    width: 0,
    height: 0,
};

/// The rects a panel's data slot is drawn in (#200): its data area, or on a
/// broken axis one rect per piece, so nothing is drawn in a gap. Each draw
/// of the data slot is scissored to each rect in turn; a mark crossing a gap
/// is drawn once per piece it reaches and shows only inside them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DataPieces {
    rects: [Rect; MAX_DATA_PIECES],
    count: usize,
}

impl DataPieces {
    /// The whole data area, one piece.
    pub(crate) fn whole(area: Rect) -> Self {
        let mut rects = [NO_RECT; MAX_DATA_PIECES];
        rects[0] = area;
        Self { rects, count: 1 }
    }

    /// The pieces of `area` that `config`'s broken axes leave
    /// ([`crate::layout::BreakMap`]): the bands of `bottom_x` across and of
    /// `left_y` up, mirrored on an inverted axis, rounded to whole pixels.
    pub(crate) fn of(config: &crate::config::Config, area: Rect) -> Self {
        let bands = |axis: &crate::config::AxisOptions, len: u32| {
            let map = crate::layout::BreakMap::of(axis);
            let mut out = [(0u32, len); crate::config::MAX_AXIS_BREAKS + 1];
            let mut count = 0;
            for (a, b) in map.pieces() {
                let (a, b) = if axis.inverted {
                    (1.0 - b, 1.0 - a)
                } else {
                    (a, b)
                };
                let px = |t: f64| (t.clamp(0.0, 1.0) * f64::from(len)).round() as u32;
                if px(b) > px(a) {
                    out[count] = (px(a), px(b));
                    count += 1;
                }
            }
            (out, count)
        };
        let (xs, nx) = bands(&config.bottom_x, area.width);
        let (ys, ny) = bands(&config.left_y, area.height);
        if nx <= 1 && ny <= 1 {
            return Self::whole(area);
        }
        let mut pieces = Self {
            rects: [NO_RECT; MAX_DATA_PIECES],
            count: 0,
        };
        for &(x0, x1) in &xs[..nx] {
            // A y band is measured up from the bottom of the area.
            for &(y0, y1) in &ys[..ny] {
                pieces.rects[pieces.count] = Rect {
                    x: area.x + x0,
                    y: area.y + (area.height - y1),
                    width: x1 - x0,
                    height: y1 - y0,
                };
                pieces.count += 1;
            }
        }
        pieces
    }

    pub(crate) fn rects(&self) -> &[Rect] {
        &self.rects[..self.count]
    }

    /// The same pieces relative to `(x, y)`; `None` when a piece starts
    /// before it.
    pub(crate) fn relative_to(&self, x: u32, y: u32) -> Option<Self> {
        let mut out = *self;
        for rect in &mut out.rects[..out.count] {
            rect.x = rect.x.checked_sub(x)?;
            rect.y = rect.y.checked_sub(y)?;
        }
        Some(out)
    }
}

/// The order a frame draws its panels' slots in: every panel's
/// [`Slot::Below`] with its data, then every panel's [`Slot::Above`].
pub(crate) const FRAME_SLOTS: [Slot; 2] = [Slot::Below, Slot::Above];

/// A full-screen textured layer of one slot: the pipeline that blits a
/// panel-sized texture and the texture's bind group.
pub(crate) struct PanelChrome<'a> {
    /// The grid texture: what the chart emits into [`crate::sink::Slot::Below`];
    /// `None` draws the slot's data without it (the vector export's data
    /// image).
    pub below: Option<AxisLayer<'a>>,
    /// The annotation spans, when the layer draws any.
    pub annotation_underlay: Option<AxisLayer<'a>>,
    /// The decoration texture: what the chart emits into
    /// [`crate::sink::Slot::Above`]; `None` when the chrome is not drawn.
    pub above: Option<AxisLayer<'a>>,
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
    data: &DataPieces,
    chrome: &PanelChrome<'_>,
    series_list: &[SeriesLayers<'_>],
    cached_bodies: Option<AxisLayer<'_>>,
    slot: Slot,
) {
    let Some(panel_clamped) = clamp_rect_to_target(panel_rect, target_size) else {
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
    let data_scissor = |pass: &mut wgpu::RenderPass<'_>, piece: Rect| {
        pass.set_scissor_rect(piece.x, piece.y, piece.width, piece.height);
    };
    let pieces = || {
        data.rects()
            .iter()
            .filter_map(|&piece| clamp_rect_to_target(piece, target_size))
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
            if let Some(layer) = &chrome.below {
                blit(pass, layer);
            }
            if let Some(layer) = &chrome.annotation_underlay {
                blit(pass, layer);
            }

            // The data slot, scissored to the data area (to each of its
            // pieces on a broken axis): every fill, then every body, then
            // every picked highlight, each in SSOT series order.
            match cached_bodies {
                Some(layer) => {
                    // The composite was clipped to the data area when it was
                    // drawn; clipping its blit again would apply the data
                    // scissor twice at the edge samples, which a direct draw
                    // applies once.
                    panel_scissor(pass);
                    blit(pass, &layer);
                }
                None => {
                    for piece in pieces() {
                        data_scissor(pass, piece);
                        crate::data_render::issue_series_bodies(pass, series_list);
                    }
                }
            }
            for piece in pieces() {
                data_scissor(pass, piece);
                for s in series_list {
                    crate::data_render::issue_series_picked(pass, s);
                }
                // The value labels over all of the panel's data and its
                // picked highlights, never in the cached bodies: a label
                // drag or a selected label redraws only them.
                for s in series_list {
                    crate::data_render::issue_series_value_labels(pass, s);
                }
            }
        }
        Slot::Above => {
            // Slot::Above: the decoration, then the annotation overlay on top
            // of everything.
            panel_scissor(pass);
            if let Some(layer) = &chrome.above {
                blit(pass, layer);
            }
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
        below: Some(grid),
        annotation_underlay,
        above: Some(decoration),
        annotation_overlay,
    };
    for slot in FRAME_SLOTS {
        draw_chart_panel(
            pass,
            target_size,
            panel_rect,
            // A host-issued panel draws its whole data area.
            &DataPieces::whole(data_area),
            &chrome,
            series_list,
            None,
            slot,
        );
    }
}
