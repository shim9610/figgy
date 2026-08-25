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
use crate::select::{ColorBarElement, DataAreaElement, SELECTION_PADDING, Selectable};
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
    fn resize_by(&self, cfg: &mut Config, handle: ResizeHandle, dx: f32, dy: f32) -> NudgeResult {
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

impl Resizable for ColorBarElement {
    /// Each handle drives the strip edges it touches — the same compass mapping
    /// the data area uses. Which of `thickness_px` / `length_frac` an edge
    /// changes depends on the bar's orientation, and that resolution belongs to
    /// nudge: a handle only knows screen directions.
    fn resize_targets(&self, handle: ResizeHandle) -> (Option<Element>, Option<Element>) {
        use ResizeHandle::*;
        let horizontal = match handle {
            NW | W | SW => Some(Element::ColorBarEdge(Side::Left)),
            NE | E | SE => Some(Element::ColorBarEdge(Side::Right)),
            N | S => None,
        };
        let vertical = match handle {
            NW | N | NE => Some(Element::ColorBarEdge(Side::Top)),
            SW | S | SE => Some(Element::ColorBarEdge(Side::Bottom)),
            E | W => None,
        };
        (horizontal, vertical)
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
            TextExtents {
                width: rt.segments.len() as f32 * 8.0,
                ascent: 10.0,
                descent: 3.0,
            }
        }
    }

    fn cfg_800x600() -> Config {
        let mut cfg = default_config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        });
        cfg
    }

    #[test]
    fn handles_sit_on_selection_box_corners_and_midpoints() {
        let cfg = cfg_800x600();
        let b = DataAreaElement
            .bounds(&cfg, &FixedMeasure)
            .unwrap()
            .expanded(SELECTION_PADDING);
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
        assert!(
            DataAreaElement
                .hit_resize_handle(&cfg, &FixedMeasure, -100.0, -100.0)
                .is_none()
        );
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

    // ── Colourbar ──────────────────────────────────────────────────────────

    fn cfg_with_colorbar(side: Side) -> Config {
        let mut cfg = cfg_800x600();
        let mut bar = crate::default::default_colorbar_options();
        bar.side = side;
        cfg.colorbar = Some(bar);
        cfg
    }

    fn bar_of(cfg: &Config) -> &crate::config::ColorBarOptions {
        cfg.colorbar.as_ref().expect("colourbar")
    }

    fn colorbar_bounds(cfg: &Config) -> RectF {
        ColorBarElement
            .bounds(cfg, &FixedMeasure)
            .expect("strip bounds")
    }

    fn colorbar_short_edges(rect: RectF, side: &Side) -> (f32, f32) {
        match side {
            Side::Left => (rect.x + rect.width, rect.x),
            Side::Right => (rect.x, rect.x + rect.width),
            Side::Top => (rect.y + rect.height, rect.y),
            Side::Bottom => (rect.y, rect.y + rect.height),
        }
    }

    fn assert_near(actual: f32, expected: f32, context: &str) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "{context}: got {actual}, expected {expected}"
        );
    }

    /// Which dimension a handle drives follows the *bar's* orientation, not the
    /// handle: on a vertical bar the side handles are thickness and the end
    /// handles are length; on a horizontal bar it is the other way round.
    #[test]
    fn colorbar_handles_drive_thickness_or_length_by_orientation() {
        for (side, thickness_handle, length_handle) in [
            (Side::Right, ResizeHandle::W, ResizeHandle::N),
            (Side::Left, ResizeHandle::E, ResizeHandle::S),
            (Side::Top, ResizeHandle::S, ResizeHandle::E),
            (Side::Bottom, ResizeHandle::N, ResizeHandle::W),
        ] {
            let base = cfg_with_colorbar(side.clone());
            let (t0, l0) = (bar_of(&base).thickness_px, bar_of(&base).length_frac);

            // The thickness handle changes thickness and leaves length alone.
            let mut cfg = base.clone();
            assert_eq!(
                ColorBarElement.resize_by(&mut cfg, thickness_handle, -4.0, -4.0),
                NudgeResult::Moved,
                "{side:?} {thickness_handle:?}"
            );
            assert_ne!(bar_of(&cfg).thickness_px, t0, "{side:?} thickness");
            assert_eq!(bar_of(&cfg).length_frac, l0, "{side:?} length untouched");

            // And the length handle the other way.
            let mut cfg = base.clone();
            assert_eq!(
                ColorBarElement.resize_by(&mut cfg, length_handle, -4.0, -4.0),
                NudgeResult::Moved,
                "{side:?} {length_handle:?}"
            );
            assert_eq!(
                bar_of(&cfg).thickness_px,
                t0,
                "{side:?} thickness untouched"
            );
            assert_ne!(bar_of(&cfg).length_frac, l0, "{side:?} length");
        }
    }

    /// Dragging an edge outward grows the bar; inward shrinks it. Stated on the
    /// drawn rect, because that is what the user is dragging.
    #[test]
    fn dragging_a_colorbar_edge_outward_grows_it() {
        // Vertical bar, west handle: dragging left (away from the strip) widens.
        let mut cfg = cfg_with_colorbar(Side::Right);
        let before = colorbar_bounds(&cfg);
        ColorBarElement.resize_by(&mut cfg, ResizeHandle::W, -6.0, 0.0);
        let after = colorbar_bounds(&cfg);
        assert!(
            after.width > before.width,
            "west drag left must widen: {} -> {}",
            before.width,
            after.width
        );

        let mut cfg = cfg_with_colorbar(Side::Right);
        ColorBarElement.resize_by(&mut cfg, ResizeHandle::W, 6.0, 0.0);
        assert!(
            colorbar_bounds(&cfg).width < before.width,
            "west drag right narrows"
        );
    }

    /// Every thickness handle follows the pointer on screen. The edge opposite
    /// the grabbed handle stays fixed, including when the grabbed edge is the
    /// strip's band-anchored outer edge.
    #[test]
    fn colorbar_thickness_handles_track_the_pointer_on_all_sides() {
        for (side, outer_handle, outer_dx, outer_dy) in [
            (Side::Left, ResizeHandle::W, -6.0, 0.0),
            (Side::Right, ResizeHandle::E, 6.0, 0.0),
            (Side::Top, ResizeHandle::N, 0.0, -6.0),
            (Side::Bottom, ResizeHandle::S, 0.0, 6.0),
        ] {
            let pointer_delta = if outer_dx != 0.0 { outer_dx } else { outer_dy };

            let mut cfg = cfg_with_colorbar(side.clone());
            let before = colorbar_bounds(&cfg);
            let (inner_before, outer_before) = colorbar_short_edges(before, &side);
            assert_eq!(
                ColorBarElement.resize_by(&mut cfg, outer_handle, outer_dx, outer_dy),
                NudgeResult::Moved,
                "{side:?} outer handle"
            );
            let after = colorbar_bounds(&cfg);
            let (inner_after, outer_after) = colorbar_short_edges(after, &side);
            assert_near(inner_after, inner_before, &format!("{side:?} inner edge"));
            assert_near(
                outer_after,
                outer_before + pointer_delta,
                &format!("{side:?} outer edge"),
            );

            let inner_handle = match side {
                Side::Left => ResizeHandle::E,
                Side::Right => ResizeHandle::W,
                Side::Top => ResizeHandle::S,
                Side::Bottom => ResizeHandle::N,
            };
            let mut cfg = cfg_with_colorbar(side.clone());
            let before = colorbar_bounds(&cfg);
            let (inner_before, outer_before) = colorbar_short_edges(before, &side);
            assert_eq!(
                ColorBarElement.resize_by(&mut cfg, inner_handle, -outer_dx, -outer_dy),
                NudgeResult::Moved,
                "{side:?} inner handle"
            );
            let after = colorbar_bounds(&cfg);
            let (inner_after, outer_after) = colorbar_short_edges(after, &side);
            assert_near(
                inner_after,
                inner_before - pointer_delta,
                &format!("{side:?} inner edge"),
            );
            assert_near(outer_after, outer_before, &format!("{side:?} outer edge"));
        }
    }

    /// With a centre anchor both ends move by half a size change, so the size
    /// has to change by twice the drag for the dragged edge to stay under the
    /// pointer. With `Start` / `End` one end is pinned and the factor is 1.
    #[test]
    fn a_colorbar_end_handle_tracks_the_pointer_under_its_anchor() {
        use crate::config::BarAlign;

        for (align, factor) in [
            (BarAlign::Center, 2.0f32),
            (BarAlign::Start, 1.0),
            (BarAlign::End, 1.0),
        ] {
            let mut cfg = cfg_with_colorbar(Side::Right);
            cfg.colorbar.as_mut().expect("colourbar").align = align.clone();
            let da = cfg.data_area().expect("data area");
            let before = bar_of(&cfg).length_frac;

            let drag = -10.0f32; // north handle, upward = grow
            ColorBarElement.resize_by(&mut cfg, ResizeHandle::N, 0.0, drag);
            let grew = (bar_of(&cfg).length_frac - before) * da.height as f32;
            assert!(
                (grew - factor * -drag).abs() < 1e-3,
                "{align:?}: grew {grew} px, expected {}",
                factor * -drag
            );
        }
    }

    /// The size range is the one `Config::validate` states, and a handle drag
    /// cannot leave it: rejected, with the config untouched.
    #[test]
    fn colorbar_resize_stops_at_the_valid_range() {
        // Length cannot exceed the data area.
        let mut cfg = cfg_with_colorbar(Side::Right);
        cfg.colorbar.as_mut().expect("colourbar").length_frac = 1.0;
        let before = cfg.clone();
        assert_eq!(
            ColorBarElement.resize_by(&mut cfg, ResizeHandle::N, 0.0, -5.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, before);

        // Thickness cannot go negative.
        let mut cfg = cfg_with_colorbar(Side::Right);
        cfg.colorbar.as_mut().expect("colourbar").thickness_px = 2.0;
        let before = cfg.clone();
        assert_eq!(
            ColorBarElement.resize_by(&mut cfg, ResizeHandle::W, 50.0, 0.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, before);

        // A rejected outer-edge resize rolls back its paired offset as well as
        // the thickness; otherwise the bar would jump despite the rejection.
        let mut cfg = cfg_with_colorbar(Side::Right);
        let before = cfg.clone();
        assert_eq!(
            ColorBarElement.resize_by(&mut cfg, ResizeHandle::E, 1e6, 0.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, before);

        // And the result always satisfies `validate`.
        let mut cfg = cfg_with_colorbar(Side::Right);
        for _ in 0..40 {
            ColorBarElement.resize_by(&mut cfg, ResizeHandle::NW, -3.0, -3.0);
            assert!(cfg.validate().is_ok(), "resize left an invalid config");
        }
    }

    /// A corner handle drives both dimensions in one gesture.
    #[test]
    fn a_colorbar_corner_handle_resizes_both_dimensions() {
        let mut cfg = cfg_with_colorbar(Side::Right);
        let (t0, l0) = (bar_of(&cfg).thickness_px, bar_of(&cfg).length_frac);
        assert_eq!(
            ColorBarElement.resize_by(&mut cfg, ResizeHandle::NW, -5.0, -5.0),
            NudgeResult::Moved
        );
        assert!(bar_of(&cfg).thickness_px > t0);
        assert!(bar_of(&cfg).length_frac > l0);
    }

    /// The outer component of a corner handle obeys the same pointer-tracking
    /// rule while its long-axis component resizes in the same gesture.
    #[test]
    fn a_colorbar_outer_corner_tracks_both_pointer_components() {
        let mut cfg = cfg_with_colorbar(Side::Right);
        let before = colorbar_bounds(&cfg);
        assert_eq!(
            ColorBarElement.resize_by(&mut cfg, ResizeHandle::NE, 5.0, -7.0),
            NudgeResult::Moved
        );
        let after = colorbar_bounds(&cfg);
        assert_near(after.x, before.x, "opposite thickness edge");
        assert_near(
            after.x + after.width,
            before.x + before.width + 5.0,
            "outer thickness edge",
        );
        assert_near(after.y, before.y - 7.0, "north length edge");
    }
}
