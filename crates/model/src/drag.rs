//! Drag policy — `Draggable`, position adjustment for selected elements.
//!
//! `Draggable` extends [`Selectable`](crate::select::Selectable): only objects
//! the user can select can be dragged. Implementations provide a single
//! mapping — which nudge-path [`Element`] they move — and nothing else. The
//! default [`Draggable::drag_by`] routes every position change through
//! [`Config::nudge`], so a drag can never bypass that path: the same
//! feasibility check (`NudgeReject::OutOfBounds` keeps elements inside the
//! chart area) and the same rotated-frame offset mapping apply whether the
//! move came from a drag, a keyboard nudge, or programmatic adjustment.
//!
//! Axes are draggable too, with their own rule inside the same path:
//! [`Element::Axis`] *detaches* the axis — it moves only perpendicular to its
//! direction (y-axes horizontally, x-axes vertically) via `line_offset`,
//! while the data area, grid, and data transform stay put. Tick positions
//! along the axis therefore stay aligned with the data. The data area itself
//! is draggable as a whole ([`Element::DataArea`]): it translates without
//! resizing by shifting opposite margins in tandem, and it is also resizable
//! via its handles ([`crate::resize::Resizable`]).

use crate::config::Config;
use crate::layout::{Element, NudgeResult};
use crate::select::{
    AxisElement, AxisLabelElement, AxisTitleElement, ChartTitleElement, DataAreaElement,
    LegendElement, Selectable,
};

/// A selectable object whose position can be adjusted by drag & drop.
pub trait Draggable: Selectable {
    /// The nudge-path element this drag target moves. This mapping is the
    /// only thing an implementation defines; movement itself happens in
    /// [`Self::drag_by`].
    fn nudge_element(&self) -> Element;

    /// Default drag pipeline: apply the pointer delta through
    /// [`Config::nudge`]. Returns [`NudgeResult::Rejected`] (and leaves `cfg`
    /// untouched) when the move would leave the chart area.
    ///
    /// Call this per pointer-move with the frame delta; the accumulated
    /// offset lands in the element's `offset_x` / `offset_y` config fields,
    /// so a drop needs no extra commit step.
    fn drag_by(&self, cfg: &mut Config, dx: f32, dy: f32) -> NudgeResult {
        cfg.nudge(self.nudge_element(), dx, dy)
    }
}

impl Draggable for ChartTitleElement {
    fn nudge_element(&self) -> Element {
        Element::ChartTitle
    }
}

impl Draggable for AxisTitleElement {
    fn nudge_element(&self) -> Element {
        Element::AxisTitle(self.side.clone())
    }
}

impl Draggable for AxisLabelElement {
    fn nudge_element(&self) -> Element {
        Element::AxisLabel(self.side.clone())
    }
}

impl Draggable for AxisElement {
    fn nudge_element(&self) -> Element {
        Element::Axis(self.side.clone())
    }
}

impl Draggable for LegendElement {
    fn nudge_element(&self) -> Element {
        Element::Legend
    }
}

impl Draggable for DataAreaElement {
    fn nudge_element(&self) -> Element {
        Element::DataArea
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;
    use crate::layout::{ChartArea, NudgeReject, Rect, Side};
    use crate::text::rich_segments_from_text;

    fn cfg_800x600() -> Config {
        let mut cfg = default_config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        });
        cfg.chart_title.text.segments = rich_segments_from_text("Title");
        cfg
    }

    #[test]
    fn drag_moves_chart_title_offset() {
        let mut cfg = cfg_800x600();
        let r = ChartTitleElement.drag_by(&mut cfg, 5.0, 3.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.chart_title.offset_x, 5.0);
        assert_eq!(cfg.chart_title.offset_y, 3.0);
    }

    #[test]
    fn drag_matches_direct_nudge_path() {
        let mut via_drag = cfg_800x600();
        let mut via_nudge = cfg_800x600();
        let el = AxisTitleElement { side: Side::Left };
        el.drag_by(&mut via_drag, 4.0, -2.0);
        via_nudge.nudge(Element::AxisTitle(Side::Left), 4.0, -2.0);
        assert_eq!(via_drag, via_nudge);
    }

    #[test]
    fn rejected_drag_leaves_config_unchanged() {
        let mut cfg = cfg_800x600();
        let before = cfg.clone();
        let r = ChartTitleElement.drag_by(&mut cfg, -1e6, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }

    #[test]
    fn left_axis_title_drag_respects_rotated_frame() {
        // Screen-space drag (dx, dy) on the rotated left title must land as
        // local (-dy, dx) — same as the nudge path's inverse rotation.
        let mut cfg = cfg_800x600();
        cfg.left_y.title_option.text.segments = rich_segments_from_text("Y");
        let el = AxisTitleElement { side: Side::Left };
        let r = el.drag_by(&mut cfg, 3.0, 7.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.left_y.title_option.offset_x, -7.0);
        assert_eq!(cfg.left_y.title_option.offset_y, 3.0);
    }

    // Dragging a y-axis applies only the x component (perpendicular rule)
    // and detaches the axis: line_offset moves, the data area does not — so
    // tick positions along the axis stay aligned with the data.
    #[test]
    fn y_axis_drag_detaches_horizontally_only() {
        let mut cfg = cfg_800x600();
        let da_before = cfg.data_area().unwrap();

        let el = AxisElement { side: Side::Left };
        let r = el.drag_by(&mut cfg, 12.0, -40.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.left_y.line_offset, 12.0);
        assert_eq!(cfg.data_area().unwrap(), da_before);
    }

    #[test]
    fn x_axis_drag_detaches_vertically_only() {
        let mut cfg = cfg_800x600();
        let da_before = cfg.data_area().unwrap();
        let el = AxisElement { side: Side::Bottom };
        let r = el.drag_by(&mut cfg, -40.0, -6.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.bottom_x.line_offset, -6.0);
        assert_eq!(cfg.data_area().unwrap(), da_before);
    }

    #[test]
    fn axis_label_drag_moves_label_offsets() {
        let mut cfg = cfg_800x600();
        let el = AxisLabelElement { side: Side::Bottom };
        let r = el.drag_by(&mut cfg, 2.0, 5.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.bottom_x.label_style.label_offset_x, 2.0);
        assert_eq!(cfg.bottom_x.label_style.label_offset_y, 5.0);
    }

    // The hit-map round trip hosts use: hit → as_draggable → drag_by.
    #[test]
    fn as_draggable_exposes_drag_through_hitmap() {
        use crate::select::{HitMap, Selectable};
        use crate::text::{MeasureText, RichText, TextExtents};

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

        let mut cfg = cfg_800x600();
        let map = HitMap::standard_chart();

        // Hit the chart title and drag it via the trait-object route.
        let tb = ChartTitleElement.bounds(&cfg, &FixedMeasure).unwrap();
        let id = map
            .hit_test(&cfg, &FixedMeasure, tb.x + 1.0, tb.y + 1.0)
            .unwrap();
        let drag = map
            .get(id)
            .unwrap()
            .as_draggable()
            .expect("title is draggable");
        assert_eq!(drag.drag_by(&mut cfg, 4.0, 2.0), NudgeResult::Moved);
        assert_eq!(cfg.chart_title.offset_x, 4.0);

        // The data area is draggable too (whole-area translation).
        let db = crate::select::DataAreaElement
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        let id2 = map
            .hit_test(
                &cfg,
                &FixedMeasure,
                db.x + db.width * 0.5,
                db.y + db.height * 0.5,
            )
            .unwrap();
        assert!(map.get(id2).unwrap().as_draggable().is_some());
    }

    // Dragging the data area translates it without resizing: opposite margins
    // shift in tandem, size is preserved, and ticks/data follow by derivation.
    // Deltas stay inside the default top/right margins (8 px).
    #[test]
    fn data_area_drag_translates_without_resize() {
        let mut cfg = cfg_800x600();
        let da_before = cfg.data_area().unwrap();

        let r = DataAreaElement.drag_by(&mut cfg, 5.0, 3.0);
        assert_eq!(r, NudgeResult::Moved);
        let da_after = cfg.data_area().unwrap();
        assert_eq!(da_after.x, da_before.x + 5);
        assert_eq!(da_after.y, da_before.y + 3);
        assert_eq!(da_after.width, da_before.width);
        assert_eq!(da_after.height, da_before.height);
    }

    // A blocked axis slides: the invalid horizontal component clamps while
    // the valid vertical one still applies.
    #[test]
    fn data_area_drag_clamps_per_axis() {
        let mut cfg = cfg_800x600();
        let da_before = cfg.data_area().unwrap();
        let right_margin = cfg.right_y.out_margin; // default 8 — +50 px overflows

        let r = DataAreaElement.drag_by(&mut cfg, right_margin + 50.0, 3.0);
        assert_eq!(r, NudgeResult::Moved);
        let da_after = cfg.data_area().unwrap();
        assert_eq!(
            da_after.x, da_before.x,
            "blocked horizontal component must not apply"
        );
        assert_eq!(da_after.y, da_before.y + 3);
    }

    #[test]
    fn data_area_drag_rejected_when_fully_blocked() {
        let mut cfg = cfg_800x600();
        let before = cfg.clone();
        let r = DataAreaElement.drag_by(&mut cfg, 1e6, -1e6);
        assert_eq!(
            r,
            NudgeResult::Rejected(crate::layout::NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, before);
    }
}
