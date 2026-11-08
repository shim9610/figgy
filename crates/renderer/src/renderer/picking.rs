//! GPU picking on registered charts: the pick request, the per-series
//! pick descriptors, the pick plan, the picker state machine and the
//! pick_chart entry points.

use super::*;

mod data_pick;
mod descriptors;
mod energy_levels;
mod entry;
mod picker;
mod plan;

pub(super) use descriptors::*;
pub(super) use energy_levels::*;
pub(super) use picker::*;
pub(super) use plan::*;

/// Host-space input for one renderer-owned exact GPU pick.
///
/// The renderer derives the transform and data-area clip from its authoritative
/// chart config. Hosts provide only the physical canvas position and the panel
/// placement used for the current display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuPickRequest {
    pub canvas_position_px: [f32; 2],
    /// Where the whole document ([`Config::document_area`]) is displayed, as
    /// [`fit_display_panel`] places it. Without subplots this is the
    /// displayed chart panel.
    pub display_panel_px: Rect,
    pub display_scale: f32,
    pub max_distance_px: f32,
}

/// `request` as the pick of `panel` (the active layer's panel as displayed)
/// answers it: unchanged, except that a cursor inside a gap of a broken
/// axis (#200), inside the data area but in none of its pieces, where
/// nothing is drawn, picks as a cursor far off the chart with no reach, so
/// it hits nothing, and a cursor in a shifted piece of a `Panels` axis is
/// moved back by the piece's shift. Both the point pick and the typed data
/// pick go through it. The second value is how far the cursor moved: the
/// data-area clip the picker checks the cursor against moves with it, so
/// a moved cursor is inside the moved clip exactly when it was inside the
/// data area.
fn pick_request_outside_gaps(
    panel: &Config,
    request: GpuPickRequest,
) -> (GpuPickRequest, [f32; 2]) {
    let Ok(area) = panel.data_area() else {
        return (request, [0.0; 2]);
    };
    let pieces = super::composite::DataPieces::of(panel, area.0);
    if pieces.rects().len() <= 1 {
        return (request, [0.0; 2]);
    }
    let [x, y] = request.canvas_position_px;
    let inside = |r: &Rect| {
        x >= r.x as f32
            && x < (r.x + r.width) as f32
            && y >= r.y as f32
            && y < (r.y + r.height) as f32
    };
    if !inside(&area.0) {
        return (request, [0.0; 2]);
    }
    // A piece of a `Panels` axis is drawn at the panel's transform shifted
    // by its shift (`DataPieces`), so the cursor is picked shifted back.
    if let Some((_, shift)) = pieces
        .rects()
        .iter()
        .zip(pieces.shifts())
        .find(|(rect, _)| inside(rect))
    {
        let offset = [
            -(shift[0] * f64::from(area.0.width)) as f32,
            (shift[1] * f64::from(area.0.height)) as f32,
        ];
        let request = GpuPickRequest {
            canvas_position_px: [x + offset[0], y + offset[1]],
            ..request
        };
        return (request, offset);
    }
    let request = GpuPickRequest {
        canvas_position_px: [-1.0e6, -1.0e6],
        max_distance_px: 0.0,
        ..request
    };
    (request, [0.0; 2])
}
