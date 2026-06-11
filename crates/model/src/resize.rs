//! Resize policy — `Resizable`, PPT-style 8-handle resizing for selected
//! elements.
//!
//! A `Resizable` element's selection box grows eight square handles (four
//! corners + four edge midpoints). Dragging a handle resizes the element; as
//! with [`Draggable`](crate::drag::Draggable), the actual mutation routes
//! through [`Config::nudge`] only — implementations just map each handle to
//! the nudge elements its horizontal / vertical motion drives. For the data
//! area that mapping is its boundary edges ([`Element::DataAreaEdge`]):
//! dragging the east handle moves the right boundary, the north-west corner
//! moves the left and top boundaries, and so on — margins shift, the data
//! area follows, and ticks/data stay aligned by construction.

use crate::config::Config;
use crate::layout::{Element, NudgeReject, NudgeResult, RectF, Side};
use crate::select::{DataAreaElement, Selectable, SELECTION_PADDING};
use crate::text::MeasureText;

/// Edge length of a square resize handle, px.
pub const HANDLE_SIZE: f32 = 8.0;

/// One of the eight resize handles, compass-named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeHandle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

pub const ALL_HANDLES: [ResizeHandle; 8] = [
    ResizeHandle::NW,
    ResizeHandle::N,
    ResizeHandle::NE,
    ResizeHandle::E,
    ResizeHandle::SE,
    ResizeHandle::S,
    ResizeHandle::SW,
    ResizeHandle::W,
];

/// The eight handle squares for a selection rect: corners + edge midpoints,
/// each centered on its anchor point.
pub fn handle_rects(rect: &RectF) -> [(ResizeHandle, RectF); 8] {
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
    let (cx, cy) = (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    let half = HANDLE_SIZE * 0.5;
    let square = |x: f32, y: f32| RectF {
        x: x - half,
        y: y - half,
        width: HANDLE_SIZE,
        height: HANDLE_SIZE,
    };
    [
        (ResizeHandle::NW, square(x0, y0)),
        (ResizeHandle::N, square(cx, y0)),
        (ResizeHandle::NE, square(x1, y0)),
        (ResizeHandle::E, square(x1, cy)),
        (ResizeHandle::SE, square(x1, y1)),
        (ResizeHandle::S, square(cx, y1)),
        (ResizeHandle::SW, square(x0, y1)),
        (ResizeHandle::W, square(x0, cy)),
    ]
}

/// A selectable element whose extent can be adjusted by dragging the eight
/// handles on its selection box.
pub trait Resizable: Selectable {
    /// The nudge elements a handle's (horizontal, vertical) motion drives —
    /// the only thing an implementation defines. `None` components are
    /// discarded (an edge-midpoint handle resizes along one dimension).
    fn resize_targets(&self, handle: ResizeHandle) -> (Option<Element>, Option<Element>);

    /// Default handle geometry: eight squares on the selection box (element
    /// bounds + [`SELECTION_PADDING`]).
    fn resize_handles(
        &self,
        cfg: &Config,
        measure: &dyn MeasureText,
    ) -> Option<[(ResizeHandle, RectF); 8]> {
        let rect = self.bounds(cfg, measure)?.expanded(SELECTION_PADDING);
        Some(handle_rects(&rect))
    }

    /// Default handle hit test.
    fn hit_resize_handle(
        &self,
        cfg: &Config,
        measure: &dyn MeasureText,
        x: f32,
        y: f32,
    ) -> Option<ResizeHandle> {
        self.resize_handles(cfg, measure)?
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|(h, _)| *h)
    }

    /// Default resize pipeline: each motion component routes through
    /// [`Config::nudge`] on its target element, so resize obeys exactly the
    /// same feasibility rules as dragging those elements directly.
    fn resize_by(
        &self,
        cfg: &mut Config,
        handle: ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let (h_target, v_target) = self.resize_targets(handle);
        let mut moved = false;
        let mut attempted = false;
        if dx != 0.0
            && let Some(e) = h_target
        {
            attempted = true;
            moved |= cfg.nudge(e, dx, 0.0) == NudgeResult::Moved;
        }
        if dy != 0.0
            && let Some(e) = v_target
        {
            attempted = true;
            moved |= cfg.nudge(e, 0.0, dy) == NudgeResult::Moved;
        }
        if moved || !attempted {
            NudgeResult::Moved
        } else {
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        }
    }
}

impl Resizable for DataAreaElement {
    /// Each handle drives the boundary edges it touches.
    fn resize_targets(&self, handle: ResizeHandle) -> (Option<Element>, Option<Element>) {
        use ResizeHandle::*;
        let horizontal = match handle {
            NW | W | SW => Some(Element::DataAreaEdge(Side::Left)),
            NE | E | SE => Some(Element::DataAreaEdge(Side::Right)),
            N | S => None,
        };
        let vertical = match handle {
            NW | N | NE => Some(Element::DataAreaEdge(Side::Top)),
            SW | S | SE => Some(Element::DataAreaEdge(Side::Bottom)),
            E | W => None,
        };
        (horizontal, vertical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;
    use crate::layout::{ChartArea, Rect};
    use crate::text::{RichText, TextExtents};

    struct FixedMeasure;
    impl MeasureText for FixedMeasure {
        fn measure_rich(&self, rt: &RichText) -> TextExtents {
            TextExtents { width: rt.segments.len() as f32 * 8.0, ascent: 10.0, descent: 3.0 }
        }
    }

    fn cfg_800x600() -> Config {
        let mut cfg = default_config();
        cfg.chart_area = ChartArea(Rect { x: 0, y: 0, width: 800, height: 600 });
        cfg
    }

    #[test]
    fn handles_sit_on_selection_box_corners_and_midpoints() {
        let cfg = cfg_800x600();
        let b = DataAreaElement.bounds(&cfg, &FixedMeasure).unwrap().expanded(SELECTION_PADDING);
        let handles = DataAreaElement.resize_handles(&cfg, &FixedMeasure).unwrap();
        assert_eq!(handles.len(), 8);

        let (_, nw) = handles[0];
        assert_eq!(nw.x + HANDLE_SIZE * 0.5, b.x);
        assert_eq!(nw.y + HANDLE_SIZE * 0.5, b.y);
        let (_, e) = handles[3];
        assert_eq!(e.x + HANDLE_SIZE * 0.5, b.x + b.width);
        assert_eq!(e.y + HANDLE_SIZE * 0.5, b.y + b.height * 0.5);
    }

    #[test]
    fn hit_resize_handle_finds_corner() {
        let cfg = cfg_800x600();
        let handles = DataAreaElement.resize_handles(&cfg, &FixedMeasure).unwrap();
        let (kind, rect) = handles[4]; // SE
        let hit = DataAreaElement
            .hit_resize_handle(
                &cfg,
                &FixedMeasure,
                rect.x + rect.width * 0.5,
                rect.y + rect.height * 0.5,
            )
            .unwrap();
        assert_eq!(hit, kind);
        assert!(DataAreaElement.hit_resize_handle(&cfg, &FixedMeasure, -100.0, -100.0).is_none());
    }

    // Dragging the east handle left shrinks the data area by growing the
    // right margin — and only horizontally (the dy component is discarded).
    #[test]
    fn east_handle_resizes_via_right_axis() {
        let mut cfg = cfg_800x600();
        let da_before = cfg.data_area().unwrap();
        let r = DataAreaElement.resize_by(&mut cfg, ResizeHandle::E, -10.0, 99.0);
        assert_eq!(r, NudgeResult::Moved);
        let da_after = cfg.data_area().unwrap();
        assert_eq!(da_after.width, da_before.width - 10);
        assert_eq!(da_after.height, da_before.height);
        assert_eq!(da_after.x, da_before.x);
    }

    // A corner handle drives both boundary axes at once.
    #[test]
    fn nw_corner_resizes_left_and_top() {
        let mut cfg = cfg_800x600();
        let da_before = cfg.data_area().unwrap();
        let r = DataAreaElement.resize_by(&mut cfg, ResizeHandle::NW, 6.0, 4.0);
        assert_eq!(r, NudgeResult::Moved);
        let da_after = cfg.data_area().unwrap();
        assert_eq!(da_after.x, da_before.x + 6);
        assert_eq!(da_after.y, da_before.y + 4);
        assert_eq!(da_after.width, da_before.width - 6);
        assert_eq!(da_after.height, da_before.height - 4);
    }

    // Resizing past the chart edge is rejected and leaves the config intact.
    #[test]
    fn resize_rejected_when_margin_would_go_negative() {
        let mut cfg = cfg_800x600();
        let before = cfg.clone();
        let r = DataAreaElement.resize_by(&mut cfg, ResizeHandle::E, 1e6, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }
}
