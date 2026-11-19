//! Polar category labels (#309) and category group headers (#203).

use super::*;

impl Config {
    /// How many polar category labels [`HitMap::for_active_layer`]
    /// registers as their own elements (#309), on the chart first and then
    /// on each subplot in order: a host that keeps a hit map rebuilds it
    /// when this sequence changes, as when the active layer does.
    pub fn polar_category_label_counts(&self) -> impl Iterator<Item = usize> + '_ {
        fn own(config: &Config) -> usize {
            if config.polar().is_some() {
                config.bottom_x.categories.as_deref().map_or(0, <[_]>::len)
            } else {
                0
            }
        }
        std::iter::once(own(self)).chain(self.subplots.iter().map(|subplot| own(&subplot.config)))
    }
}

/// The label of category `index` on a polar chart's categorical angle axis
/// (#309): hit and outlined where it is drawn, dragged into its own
/// `Category::label_offset`. Registered over the angle labels as a whole,
/// so a click on a label selects that label.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryLabelElement {
    pub index: usize,
}

/// The category group headers of the categorical `top_x` or `bottom_x`
/// (#203), outlined and hit header by header, dragged into the axis'
/// `category_groups.offset`.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryGroupLabelElement {
    pub side: Side,
}

impl Selectable for CategoryGroupLabelElement {
    fn element_id(&self) -> String {
        format!("category_groups_{}", side_tag(&self.side))
    }

    /// The headers the renderer draws past the axis' tick labels
    /// ([`crate::chrome::emit_part_of`]).
    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(
                panel,
                &Element::CategoryGroupLabel(self.side.clone()),
                measure,
                sink,
            )
        })
    }

    /// Each header (and its bracket) is outlined and hit where it is drawn.
    fn outlines_parts(&self, _: &Config) -> bool {
        true
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for CategoryLabelElement {
    fn element_id(&self) -> String {
        format!("category_label_{}", self.index)
    }

    /// The label as the renderer draws it ([`crate::chrome::emit_part_of`]).
    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::CategoryLabel(self.index), measure, sink)
        })
    }

    /// The label is outlined and hit where it is drawn.
    fn outlines_parts(&self, _: &Config) -> bool {
        true
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }

    /// The ring every angle label starts from: grabbed while this label is
    /// selected, it moves them all together ([`Resizable`] below).
    fn as_resizable(&self) -> Option<&dyn Resizable> {
        Some(self)
    }

    fn selection_ring(&self, cfg: &Config) -> Option<([f32; 2], f32)> {
        on_drawn_panel(cfg, crate::chrome::polar_angle_label_ring)
    }
}

/// How far, px, a press may land from a selected category label's ring and
/// still grab it (#309): the stroke grab width.
pub const RING_GRAB_HALF_WIDTH: f32 = STROKE_GRAB_HALF_WIDTH;

/// A selected polar category label's ring (#309) is its one handle: no
/// squares, a press within [`RING_GRAB_HALF_WIDTH`] of the circle, and a
/// drag that moves the angle labels as a whole along their spokes
/// (`Element::AxisLabel(Side::Bottom)`), the label's own offset unchanged.
impl Resizable for CategoryLabelElement {
    fn resize_targets(&self, _: ResizeHandle) -> (Option<Element>, Option<Element>) {
        (Some(Element::AxisLabel(Side::Bottom)), None)
    }

    fn resize_handles(
        &self,
        _: &Config,
        _: &dyn MeasureText,
        _: &mut dyn FnMut(ResizeHandle, RectF),
    ) {
    }

    /// Near the ring and not on a drawn angle label, so a press on a label
    /// where the ring passes under it still selects that label.
    fn hit_resize_handle(
        &self,
        cfg: &Config,
        measure: &dyn MeasureText,
        x: f32,
        y: f32,
    ) -> Option<ResizeHandle> {
        let (center, radius) = self.selection_ring(cfg)?;
        let distance = (x - center[0]).hypot(y - center[1]);
        if (distance - radius).abs() > RING_GRAB_HALF_WIDTH {
            return None;
        }
        let mut on_label = false;
        on_drawn_panel(cfg, |panel| {
            emit_element(
                panel,
                &Element::AxisLabel(Side::Bottom),
                measure,
                &mut Parts(&mut |part| on_label |= part_contains(&part, x, y)),
            )
        });
        (!on_label).then_some(ResizeHandle::N)
    }

    fn resize_by(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        _: ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        cfg.nudge(Element::AxisLabel(Side::Bottom), measure, dx, dy)
    }

    fn resize_from(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        _: ResizeHandle,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        cfg.nudge_at(Element::AxisLabel(Side::Bottom), measure, at, dx, dy)
    }
}
