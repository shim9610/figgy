//! Drag policy — `Draggable`, position adjustment for selected elements.
//!
//! `Draggable` extends [`Selectable`]: only objects
//! the user can select can be dragged. Implementations name the nudge-path
//! [`Element`] they move. The default [`Draggable::drag_by`] routes every
//! position change through [`Config::nudge`], so a drag can never bypass that
//! path: the same feasibility check (`NudgeReject::OutOfBounds` keeps
//! elements inside the chart area) and the same rotated-frame offset mapping
//! apply whether the move came from a drag, a keyboard nudge, or
//! programmatic adjustment. [`AxisElement`] and [`AxisLabelElement`] also
//! override [`Draggable::drag_from`] to pass the pointer position on to
//! [`Config::nudge_from`], because their polar moves depend on where they
//! were grabbed. Overlay-layer and subplot elements override `drag_by` to
//! nudge their own panel's `Config` and write the result back.
//!
//! Axes are draggable too, with their own rule inside the same path. In a
//! Cartesian chart [`Element::Axis`] *detaches* the axis: it moves only
//! perpendicular to its direction (y-axes horizontally, x-axes vertically)
//! via `line_offset`, while the data area, grid, and data transform stay put.
//! Tick positions along the axis therefore stay aligned with the data. The
//! data area itself is draggable as a whole ([`Element::DataArea`]): it
//! translates without resizing by shifting opposite margins in tandem, and it
//! is also resizable via its handles ([`crate::resize::Resizable`]).
//!
//! In a polar chart ([`Config::polar`]) the axes follow the polar frame
//! instead. Dragging the radius axis (`Element::Axis(Side::Left)`) turns its
//! ray around the centre (`radial_label_angle_degrees`); the angle axis, the
//! outer ring, has no move and the drag is rejected. The angle tick labels
//! move along their spokes and the radius tick labels in the ray frame. With
//! the grab point from [`Draggable::drag_from`] these moves follow the
//! pointer; without it the ray turns as if grabbed on the outer ring and the
//! angle labels move along the spoke at the zero angle.

use crate::config::Config;
use crate::layout::{Element, NudgeResult};
use crate::select::{
    AxisElement, AxisLabelElement, AxisTitleElement, ChartTitleElement, ColorBarAxisElement,
    ColorBarElement, ColorBarLabelElement, ColorBarTitleElement, DataAreaElement, LegendElement,
    Selectable,
};
use crate::text::MeasureText;

/// State that owns position changes: [`Config`] for Cartesian and polar
/// charts, [`DecorationLayout`](crate::decoration::DecorationLayout) for
/// the radial chart's decorations and [`AnnotationLayer`](crate::layer::AnnotationLayer)
/// for annotations. All three share the hit-test and drag policy of this
/// module.
pub trait DragConfig {
    type Element;
    /// Move `element` by a screen delta. `measure` lays the element out so
    /// containment is checked on what is drawn ([`Config::nudge`]); states
    /// whose elements carry their measured bounds ignore it.
    fn nudge(
        &mut self,
        element: Self::Element,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult;
    /// [`Self::nudge`] with the pointer position `at` (chart-local logical
    /// pixels) the delta was measured from. Elements whose rule depends on
    /// where they were grabbed read it (a polar chart's ray turns around the
    /// centre, its angle labels move along the spoke under the pointer);
    /// the default ignores it.
    fn nudge_at(
        &mut self,
        element: Self::Element,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let _ = at;
        self.nudge(element, measure, dx, dy)
    }
}
impl DragConfig for Config {
    type Element = Element;
    fn nudge(
        &mut self,
        element: Element,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        Config::nudge(self, element, measure, dx, dy)
    }
    fn nudge_at(
        &mut self,
        element: Element,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        Config::nudge_from(self, element, measure, Some(at), dx, dy)
    }
}

/// A selectable object whose position can be adjusted by drag & drop.
pub trait Draggable<C: DragConfig = Config>: Selectable<C> {
    /// The nudge-path element this drag target moves. Movement itself
    /// happens in [`Self::drag_by`] and [`Self::drag_from`]; an element whose
    /// move depends on the grab point overrides `drag_from` to route it
    /// through [`DragConfig::nudge_at`].
    fn nudge_element(&self) -> C::Element;

    /// Default drag pipeline: apply the pointer delta through
    /// [`DragConfig::nudge`] ([`Config::nudge`] for chart elements). Returns
    /// [`NudgeResult::Rejected`] (and leaves `cfg` untouched) when the move
    /// would leave the chart area.
    ///
    /// Call this per pointer-move with the frame delta; the move accumulates
    /// in the config fields the element is drawn from (its `offset_x` /
    /// `offset_y`, an axis' `line_offset`, the tick labels'
    /// `label_offset_x` / `label_offset_y`, the data-area margins or a polar
    /// chart's `radial_label_angle_degrees`), so a drop needs no extra commit
    /// step.
    ///
    /// `measure` lays the element out, so the move is contained on the
    /// element's drawn parts (the renderer's `CpuTextMeasure`; a measure
    /// that cannot place the element rejects its drag).
    fn drag_by(&self, cfg: &mut C, measure: &dyn MeasureText, dx: f32, dy: f32) -> NudgeResult {
        cfg.nudge(self.nudge_element(), measure, dx, dy)
    }

    /// [`Self::drag_by`] with the pointer position `at` the delta was
    /// measured from, for hosts that have it. Elements whose move depends on
    /// where they were grabbed route it through [`DragConfig::nudge_at`];
    /// the default is [`Self::drag_by`].
    fn drag_from(
        &self,
        cfg: &mut C,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let _ = at;
        self.drag_by(cfg, measure, dx, dy)
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

    fn drag_from(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        cfg.nudge_at(self.nudge_element(), measure, at, dx, dy)
    }
}

impl Draggable for crate::select::ParallelAxisLabelElement {
    fn nudge_element(&self) -> Element {
        Element::ParallelAxisLabel(self.axis)
    }

    fn drag_from(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        cfg.nudge_at(self.nudge_element(), measure, at, dx, dy)
    }
}

impl Draggable for AxisElement {
    fn nudge_element(&self) -> Element {
        Element::Axis(self.side.clone())
    }

    fn drag_from(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        cfg.nudge_at(self.nudge_element(), measure, at, dx, dy)
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

impl Draggable for ColorBarElement {
    fn nudge_element(&self) -> Element {
        Element::ColorBar
    }
}

impl Draggable for ColorBarAxisElement {
    fn nudge_element(&self) -> Element {
        Element::ColorBarAxis
    }
}

impl Draggable for ColorBarLabelElement {
    fn nudge_element(&self) -> Element {
        Element::ColorBarLabel
    }
}

impl Draggable for ColorBarTitleElement {
    fn nudge_element(&self) -> Element {
        Element::ColorBarTitle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;
    use crate::layout::{ChartArea, NudgeReject, Rect, Side};
    use crate::select::SelectionGeometry;
    use crate::select::tests::LaysOut;
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
    fn drag_matches_direct_nudge_path() {
        // The title must draw something: an empty one cannot be grabbed and
        // would be rejected on both paths.
        let mut via_drag = cfg_800x600();
        via_drag.left_y.title_option.text.segments = rich_segments_from_text("Y");
        let mut via_nudge = via_drag.clone();
        let el = AxisTitleElement { side: Side::Left };
        assert_eq!(
            el.drag_by(&mut via_drag, &LaysOut, 4.0, -2.0),
            NudgeResult::Moved
        );
        assert_eq!(
            via_nudge.nudge(Element::AxisTitle(Side::Left), &LaysOut, 4.0, -2.0),
            NudgeResult::Moved
        );
        assert_eq!(via_drag, via_nudge);
    }

    #[test]
    fn left_axis_title_drag_respects_rotated_frame() {
        // Screen-space drag (dx, dy) on the rotated left title must land as
        // local (-dy, dx) — same as the nudge path's inverse rotation.
        let mut cfg = cfg_800x600();
        cfg.left_y.title_option.text.segments = rich_segments_from_text("Y");
        let el = AxisTitleElement { side: Side::Left };
        let r = el.drag_by(&mut cfg, &LaysOut, 3.0, 7.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.left_y.title_option.offset_x, -7.0);
        assert_eq!(cfg.left_y.title_option.offset_y, 3.0);
    }

    #[test]
    fn axis_label_drag_moves_label_offsets() {
        let mut cfg = cfg_800x600();
        let el = AxisLabelElement { side: Side::Bottom };
        let r = el.drag_by(&mut cfg, &LaysOut, 2.0, 5.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.bottom_x.label_style.label_offset_x, 2.0);
        assert_eq!(cfg.bottom_x.label_style.label_offset_y, 5.0);
    }

    // The hit-map round trip hosts use: hit → as_draggable → drag_by.
    #[test]
    fn as_draggable_exposes_drag_through_hitmap() {
        use crate::select::HitMap;
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
        assert_eq!(
            drag.drag_by(&mut cfg, &LaysOut, 4.0, 2.0),
            NudgeResult::Moved
        );
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

        let r = DataAreaElement.drag_by(&mut cfg, &LaysOut, 5.0, 3.0);
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
        let right_margin = cfg.right_y.out_margin; // +50 px past the right margin overflows

        let r = DataAreaElement.drag_by(&mut cfg, &LaysOut, right_margin + 50.0, 3.0);
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
        let r = DataAreaElement.drag_by(&mut cfg, &LaysOut, 1e6, -1e6);
        assert_eq!(
            r,
            NudgeResult::Rejected(crate::layout::NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, before);
    }

    /// The bar drags like the legend: the delta lands in its own offsets, and
    /// the *band* does not move — so the data area does not reflow out from
    /// under the pointer mid-drag.
    #[test]
    fn dragging_the_colorbar_moves_its_offsets_and_not_the_layout() {
        use crate::layout::Side;
        use crate::select::ColorBarElement;

        let mut cfg = cfg_800x600();
        cfg.colorbar = Some(crate::default::default_colorbar_options());
        let margins_before = cfg.margins();
        let area_before = cfg.data_area().expect("data area");

        assert_eq!(
            ColorBarElement.drag_by(&mut cfg, &LaysOut, 7.0, -3.0),
            NudgeResult::Moved
        );
        let bar = cfg.colorbar.as_ref().expect("colourbar");
        assert_eq!((bar.offset_x, bar.offset_y), (7.0, -3.0));
        assert_eq!(cfg.margins(), margins_before);
        assert_eq!(cfg.data_area().expect("data area"), area_before);

        // Deltas accumulate, and the drawn rect follows them.
        let da = cfg.data_area().expect("data area");
        let anchored = {
            let mut probe = cfg.clone();
            let bar = probe.colorbar.as_mut().expect("colourbar");
            bar.offset_x = 0.0;
            bar.offset_y = 0.0;
            crate::layout::colorbar_rect(
                &probe.chart_area,
                &da,
                probe.chart_title.top_margin,
                probe.colorbar.as_ref().expect("colourbar"),
            )
        };
        ColorBarElement.drag_by(&mut cfg, &LaysOut, 1.0, 1.0);
        let bar = cfg.colorbar.as_ref().expect("colourbar");
        assert_eq!((bar.offset_x, bar.offset_y), (8.0, -2.0));
        let moved =
            crate::layout::colorbar_rect(&cfg.chart_area, &da, cfg.chart_title.top_margin, bar);
        assert_eq!(moved.x, anchored.x + 8.0);
        assert_eq!(moved.y, anchored.y - 2.0);

        // A drag that would leave the chart area is rejected, state untouched.
        let before = cfg.clone();
        assert_eq!(
            ColorBarElement.drag_by(&mut cfg, &LaysOut, 10_000.0, 0.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, before);

        // With no colourbar there is nothing to drag.
        let mut bare = cfg_800x600();
        bare.colorbar = None;
        assert_eq!(
            ColorBarElement.drag_by(&mut bare, &LaysOut, 1.0, 1.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        let _ = Side::Right;
    }

    #[test]
    fn colorbar_detail_drags_update_only_the_z_axis_fields() {
        let mut cfg = cfg_800x600();
        let mut bar = crate::default::default_colorbar_options();
        bar.side = Side::Right;
        bar.axis.title_option.visible = true;
        bar.axis.title_option.text =
            crate::text::RichText::plain("z", crate::color::Color::BLACK, 12.0, "");
        cfg.colorbar = Some(bar);
        let area_before = cfg.data_area().expect("data area");
        let strip_offsets_before = {
            let bar = cfg.colorbar.as_ref().unwrap();
            (bar.offset_x, bar.offset_y)
        };

        assert_eq!(
            ColorBarAxisElement.drag_by(&mut cfg, &LaysOut, 6.0, 99.0),
            NudgeResult::Moved
        );
        assert_eq!(cfg.colorbar.as_ref().unwrap().axis.line_offset, 6.0);

        assert_eq!(
            ColorBarLabelElement.drag_by(&mut cfg, &LaysOut, 4.0, -2.0),
            NudgeResult::Moved
        );
        let labels = &cfg.colorbar.as_ref().unwrap().axis.label_style;
        assert_eq!((labels.label_offset_x, labels.label_offset_y), (4.0, -2.0));

        assert_eq!(
            ColorBarTitleElement.drag_by(&mut cfg, &LaysOut, 3.0, 7.0),
            NudgeResult::Moved
        );
        let title = &cfg.colorbar.as_ref().unwrap().axis.title_option;
        // Right-side text is rotated +90°: screen (dx,dy) → local (dy,-dx).
        assert_eq!((title.offset_x, title.offset_y), (7.0, -3.0));

        let bar = cfg.colorbar.as_ref().unwrap();
        assert_eq!((bar.offset_x, bar.offset_y), strip_offsets_before);
        assert_eq!(cfg.data_area().expect("data area"), area_before);
    }

    fn polar_cfg() -> Config {
        let mut cfg = cfg_800x600();
        cfg.polar = Some(crate::polar::PolarOptions::default());
        cfg
    }

    /// The ray follows the pointer's angle around the centre; without a
    /// pointer it turns as if grabbed on the outer ring. The ring itself
    /// has no move.
    #[test]
    fn polar_ray_drag_turns_it_and_the_ring_is_rejected() {
        let mut cfg = polar_cfg();
        let geometry = cfg.polar_geometry().unwrap();
        let (cx, cy, r) = (geometry.center[0], geometry.center[1], geometry.radius);
        let ray = AxisElement { side: Side::Left };
        // Grab the upward ray at half radius and move to 45 degrees.
        let at = [cx, cy - r * 0.5];
        let to = [cx + r * 0.5 * 0.7071, cy - r * 0.5 * 0.7071];
        assert_eq!(
            ray.drag_from(&mut cfg, &LaysOut, at, to[0] - at[0], to[1] - at[1]),
            NudgeResult::Moved
        );
        let angle = cfg.polar.as_ref().unwrap().radial_label_angle_degrees;
        assert!((angle - 45.0).abs() < 0.5, "{angle}");
        // Without a pointer, a drag across the ray by r/2 turns by 0.5 rad.
        let mut cfg = polar_cfg();
        assert_eq!(
            ray.drag_by(&mut cfg, &LaysOut, -r * 0.5, 0.0),
            NudgeResult::Moved
        );
        let angle = cfg.polar.as_ref().unwrap().radial_label_angle_degrees;
        assert!(
            (angle - (90.0 + 0.5f64.to_degrees())).abs() < 0.5,
            "{angle}"
        );
        // A full turn wraps.
        let mut cfg = polar_cfg();
        cfg.nudge_from(
            Element::Axis(Side::Left),
            &LaysOut,
            Some([cx, cy - r]),
            0.0,
            2.0 * r,
        );
        let angle = cfg.polar.as_ref().unwrap().radial_label_angle_degrees;
        assert!((angle - 270.0).abs() < 0.5, "{angle}");
        let before = cfg.clone();
        assert_eq!(
            AxisElement { side: Side::Bottom }.drag_from(
                &mut cfg,
                &LaysOut,
                [cx + r, cy],
                5.0,
                5.0
            ),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, before);
        assert_eq!(cfg.left_y.line_offset, 0.0, "no detach in polar mode");
    }

    /// Angle labels move along the spoke under the pointer, radius labels
    /// in the ray frame.
    #[test]
    fn polar_label_drags_use_the_spoke_and_the_ray_frame() {
        let mut cfg = polar_cfg();
        let geometry = cfg.polar_geometry().unwrap();
        let (cx, cy, r) = (geometry.center[0], geometry.center[1], geometry.radius);
        let labels = AxisLabelElement { side: Side::Bottom };
        // Grab the label at 180 degrees and drag left: outward there.
        assert_eq!(
            labels.drag_from(&mut cfg, &LaysOut, [cx - r - 12.0, cy], -10.0, 0.0),
            NudgeResult::Moved
        );
        assert!((cfg.bottom_x.label_style.label_offset_y - 10.0).abs() < 1e-5);
        assert_eq!(cfg.bottom_x.label_style.label_offset_x, 0.0);
        // Grab the label at 90 degrees and drag down: inward there; the
        // tangential part is dropped.
        assert_eq!(
            labels.drag_from(&mut cfg, &LaysOut, [cx, cy - r - 12.0], 30.0, 4.0),
            NudgeResult::Moved
        );
        assert!((cfg.bottom_x.label_style.label_offset_y - 6.0).abs() < 1e-5);
        // Without a pointer the spoke at the zero angle (right) is used.
        assert_eq!(
            labels.drag_by(&mut cfg, &LaysOut, 5.0, 0.0),
            NudgeResult::Moved
        );
        assert!((cfg.bottom_x.label_style.label_offset_y - 11.0).abs() < 1e-5);
        // Far past the chart area the move is rejected.
        assert_eq!(
            labels.drag_from(&mut cfg, &LaysOut, [cx - r - 12.0, cy], -2000.0, 0.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );

        // Radius labels: the ray points up, so screen up is `along` and
        // screen left is `across` (the label side).
        let radius_labels = AxisLabelElement { side: Side::Left };
        assert_eq!(
            radius_labels.drag_from(&mut cfg, &LaysOut, [cx - 20.0, cy - r * 0.5], -7.0, -9.0),
            NudgeResult::Moved
        );
        assert!(
            (cfg.left_y.label_style.label_offset_x - 9.0).abs() < 1e-5,
            "along"
        );
        assert!(
            (cfg.left_y.label_style.label_offset_y - 7.0).abs() < 1e-5,
            "across"
        );
        // The same screen drag on a rightward ray reads the other way round.
        let mut cfg = polar_cfg();
        cfg.polar.as_mut().unwrap().radial_label_angle_degrees = 0.0;
        assert_eq!(
            radius_labels.drag_by(&mut cfg, &LaysOut, -7.0, -9.0),
            NudgeResult::Moved
        );
        assert!(
            (cfg.left_y.label_style.label_offset_x + 7.0).abs() < 1e-5,
            "along"
        );
        assert!(
            (cfg.left_y.label_style.label_offset_y - 9.0).abs() < 1e-5,
            "across"
        );
        // Other elements take the plain path with or without a pointer.
        let mut cfg = polar_cfg();
        let title = ChartTitleElement;
        assert_eq!(
            title.drag_from(&mut cfg, &LaysOut, [10.0, 10.0], 4.0, 3.0),
            NudgeResult::Moved
        );
        assert_eq!(cfg.chart_title.offset_x, 4.0);
        assert_eq!(cfg.chart_title.offset_y, 3.0);
    }
}
