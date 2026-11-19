//! Elements of a subplot's own chart and the subplot frame.

use super::*;

/// An element of subplot `subplot`'s own chart, registered while that
/// subplot (or one of its overlays) is the active layer.
///
/// It measures against the subplot's chart ([`Config::subplot_config`]) and
/// applies a drag or resize to that chart through the element's own policy,
/// then stores the result in `subplots[subplot].config`, so every rule of the
/// single chart (data-area margins, axis offsets, overlay axes, legends)
/// holds unchanged inside a subplot. The id is the inner id prefixed with
/// `subplot_<n>:`.
pub struct SubplotElement {
    pub subplot: usize,
    pub(super) inner: Box<dyn Selectable + Send + Sync>,
    /// Registered on the base layer, where the subplot shares the pointer
    /// with the parent and the other subplots (#205).
    pub(super) routed: bool,
}

impl std::fmt::Debug for SubplotElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubplotElement")
            .field("subplot", &self.subplot)
            .field("inner", &self.inner.element_id())
            .finish()
    }
}

impl SubplotElement {
    /// Run `edit` on the subplot's chart and store what it moved. The linked
    /// axis ranges stay their targets' and the subplot's own active layer
    /// and annotation layer are not part of the edit.
    fn edit(&self, cfg: &mut Config, edit: impl FnOnce(&mut Config) -> NudgeResult) -> NudgeResult {
        let Some(mut chart) = cfg.subplot_config(self.subplot) else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let result = edit(&mut chart);
        if result == NudgeResult::Moved {
            let stored = &mut cfg.subplots[self.subplot].config;
            // The chart was edited at its drawn text sizes (#292); the
            // stored ones stay.
            chart.scale_text_sizes(Some(stored), 1.0);
            // Every field of the edited chart is named: a new one is stored
            // from the edit unless it is one `subplot_config` overrides.
            let Config {
                chart_area,
                top_x,
                bottom_x,
                left_y,
                right_y,
                chart_title,
                grid,
                legend,
                picked_data,
                colorbar,
                draw_style,
                coordinates,
                overlays,
                subplots,
                // `subplot_config` resets these two; the stored ones stay.
                active_layer: _,
                annotations: _,
                bar_groups,
                line_offsets,
                plot_background,
                background,
            } = chart;
            *stored = Config {
                chart_area,
                top_x,
                bottom_x,
                left_y,
                right_y,
                chart_title,
                grid,
                legend,
                picked_data,
                colorbar,
                draw_style,
                coordinates,
                overlays,
                subplots,
                active_layer: stored.active_layer,
                annotations: std::mem::replace(
                    &mut stored.annotations,
                    crate::layer::AnnotationLayer::empty(),
                ),
                bar_groups,
                line_offsets,
                plot_background,
                background,
            };
        }
        result
    }
}

impl Selectable for SubplotElement {
    fn element_id(&self) -> String {
        format!("subplot_{}:{}", self.subplot, self.inner.element_id())
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        match cfg.subplot_config(self.subplot) {
            Some(chart) => self.inner.emit(&chart, measure, sink),
            None => true,
        }
    }

    fn outlines_parts(&self, cfg: &Config) -> bool {
        cfg.subplot_config(self.subplot)
            .is_some_and(|chart| self.inner.outlines_parts(&chart))
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        self.inner.as_draggable().map(|_| self as &dyn Draggable)
    }

    fn as_resizable(&self) -> Option<&dyn Resizable> {
        self.inner.as_resizable().map(|_| self as &dyn Resizable)
    }

    fn selection_ring(&self, cfg: &Config) -> Option<([f32; 2], f32)> {
        self.inner
            .selection_ring(&cfg.subplot_config(self.subplot)?)
    }

    fn owns_point(&self, cfg: &Config, x: f32, y: f32) -> bool {
        !self.routed || cfg.subplot_at(x, y) == Some(self.subplot)
    }

    fn is_background(&self) -> bool {
        self.inner.is_background()
    }
}

impl Draggable for SubplotElement {
    fn nudge_element(&self) -> Element {
        self.inner
            .as_draggable()
            .map_or(Element::DataArea, |drag| drag.nudge_element())
    }

    fn drag_by(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let Some(drag) = self.inner.as_draggable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        self.edit(cfg, |chart| drag.drag_by(chart, measure, dx, dy))
    }

    fn drag_from(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let Some(drag) = self.inner.as_draggable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        // A subplot's chart keeps document coordinates, so the pointer
        // passes through unchanged.
        self.edit(cfg, |chart| drag.drag_from(chart, measure, at, dx, dy))
    }
}

impl Resizable for SubplotElement {
    fn resize_targets(
        &self,
        handle: crate::resize::ResizeHandle,
    ) -> (Option<Element>, Option<Element>) {
        self.inner
            .as_resizable()
            .map_or((None, None), |resize| resize.resize_targets(handle))
    }

    fn resize_by(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        handle: crate::resize::ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let Some(resize) = self.inner.as_resizable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        self.edit(cfg, |chart| {
            resize.resize_by(chart, measure, handle, dx, dy)
        })
    }

    fn resize_from(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        handle: crate::resize::ResizeHandle,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let Some(resize) = self.inner.as_resizable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        self.edit(cfg, |chart| {
            resize.resize_from(chart, measure, handle, at, dx, dy)
        })
    }

    fn resize_handles(
        &self,
        cfg: &Config,
        measure: &dyn MeasureText,
        each: &mut dyn FnMut(ResizeHandle, RectF),
    ) {
        if let (Some(resize), Some(chart)) =
            (self.inner.as_resizable(), cfg.subplot_config(self.subplot))
        {
            resize.resize_handles(&chart, measure, each);
        }
    }

    /// The inner element's own hit rule on the subplot's chart, so a
    /// handle that is not a square (a category label's ring, #309) is
    /// grabbed inside a subplot too.
    fn hit_resize_handle(
        &self,
        cfg: &Config,
        measure: &dyn MeasureText,
        x: f32,
        y: f32,
    ) -> Option<ResizeHandle> {
        let resize = self.inner.as_resizable()?;
        resize.hit_resize_handle(&cfg.subplot_config(self.subplot)?, measure, x, y)
    }
}

/// Subplot `subplot`'s frame on the base layer (#205): its `chart_area`,
/// selected by a press on the subplot's plot or anywhere in the frame no
/// decoration of it covers. A drag moves the subplot and its handles resize
/// it, by whole pixels ([`Element::SubplotFrame`]), inside the parent chart
/// or outside it. The id is `subplot_<n>:frame`.
#[derive(Debug, Clone, PartialEq)]
pub struct SubplotFrameElement {
    pub subplot: usize,
}

impl Selectable for SubplotFrameElement {
    fn element_id(&self) -> String {
        format!("subplot_{}:frame", self.subplot)
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        emit_element(cfg, &Element::SubplotFrame(self.subplot), measure, sink)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }

    fn as_resizable(&self) -> Option<&dyn Resizable> {
        Some(self)
    }

    fn owns_point(&self, cfg: &Config, x: f32, y: f32) -> bool {
        cfg.subplot_at(x, y) == Some(self.subplot)
    }

    /// The frame is selected by a press on the subplot's plot or anywhere
    /// in it no decoration covers: a background.
    fn is_background(&self) -> bool {
        true
    }
}

impl Draggable for SubplotFrameElement {
    fn nudge_element(&self) -> Element {
        Element::SubplotFrame(self.subplot)
    }

    fn drag_from(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        cfg.nudge_from(self.nudge_element(), measure, Some(at), dx, dy)
    }
}

impl Resizable for SubplotFrameElement {
    fn resize_targets(&self, handle: ResizeHandle) -> (Option<Element>, Option<Element>) {
        use ResizeHandle::*;
        let edge = |side| Some(Element::SubplotFrameEdge(self.subplot, side));
        let horizontal = match handle {
            NW | W | SW => edge(Side::Left),
            NE | E | SE => edge(Side::Right),
            N | S => None,
        };
        let vertical = match handle {
            NW | N | NE => edge(Side::Top),
            SW | S | SE => edge(Side::Bottom),
            E | W => None,
        };
        (horizontal, vertical)
    }

    fn resize_from(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        handle: ResizeHandle,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let (horizontal, vertical) = self.resize_targets(handle);
        let mut moved = false;
        if let Some(edge) = horizontal {
            moved |= cfg.nudge_from(edge, measure, Some(at), dx, 0.0) == NudgeResult::Moved;
        }
        if let Some(edge) = vertical {
            moved |= cfg.nudge_from(edge, measure, Some(at), 0.0, dy) == NudgeResult::Moved;
        }
        if moved {
            NudgeResult::Moved
        } else {
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        }
    }
}
