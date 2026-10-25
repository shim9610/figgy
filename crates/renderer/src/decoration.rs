#![doc = include_str!("../INTERACTION.md")]
//! Renderer-measured decoration hit maps. Data picking remains on the chart frame.
//! Coordinates and drag deltas are chart-local logical pixels, not framebuffer pixels.
use crate::{CpuTextMeasure, HitId, HitMap};
use model::{
    DecorationElement, DecorationLayout,
    layout::{NudgeResult, RectF},
    select::SelectionGeometry,
};

pub(crate) fn scaled_rich_text(text: &model::text::RichText, scale: f32) -> model::text::RichText {
    let mut text = text.clone();
    text.font_size *= scale;
    for segment in &mut text.segments {
        if let Some(size) = &mut segment.font_size {
            *size *= scale;
        }
    }
    text
}

pub struct DecorationMap {
    state: DecorationLayout,
    map: HitMap<DecorationLayout>,
    entries: Vec<(HitId, DecorationElement)>,
    size: (u32, u32),
}
impl DecorationMap {
    pub fn hit_map(&self) -> &HitMap<DecorationLayout> {
        &self.map
    }
    pub fn layout(&self) -> &DecorationLayout {
        &self.state
    }
    /// The common hit-map contract: a hit can be queried for identity, bounds,
    /// selection_box and as_draggable. Nothing is selected implicitly.
    pub fn hit_test(&self, position: [f32; 2]) -> Option<HitId> {
        if position.iter().any(|v| !v.is_finite())
            || position[0] < 0.
            || position[1] < 0.
            || position[0] >= self.size.0 as f32
            || position[1] >= self.size.1 as f32
        {
            return None;
        }
        self.map.hit_test(
            &self.state,
            &CpuTextMeasure::for_style(&Default::default()),
            position[0],
            position[1],
        )
    }
    pub fn element(&self, id: HitId) -> Option<&DecorationElement> {
        self.entries.iter().find(|(i, _)| *i == id).map(|(_, e)| e)
    }
    pub fn entries(&self) -> impl Iterator<Item = (HitId, &DecorationElement)> {
        self.entries.iter().map(|(id, e)| (*id, e))
    }
    pub fn bounds(&self, id: HitId) -> Option<RectF> {
        self.map
            .get(id)?
            .bounds(&self.state, &CpuTextMeasure::for_style(&Default::default()))
    }
    pub fn selection_box(&self, id: HitId) -> Option<crate::SelectionBox> {
        self.map.selection_box(
            id,
            &self.state,
            &CpuTextMeasure::for_style(&Default::default()),
        )
    }
    /// Update the chart's SSOT through the common Draggable policy, then prepare
    /// a new frame. Reuse this map only until size/data/font/layout changes.
    pub fn drag_by(
        &self,
        layout: &mut DecorationLayout,
        id: HitId,
        dx: f32,
        dy: f32,
    ) -> Option<NudgeResult> {
        Some(self.map.get(id)?.as_draggable()?.drag_by(
            layout,
            &CpuTextMeasure::for_style(&Default::default()),
            dx,
            dy,
        ))
    }
    pub fn hit_resize_handle(
        &self,
        id: HitId,
        position: [f32; 2],
    ) -> Option<crate::resize::ResizeHandle> {
        self.map.get(id)?.as_resizable()?.hit_resize_handle(
            &self.state,
            &CpuTextMeasure::for_style(&Default::default()),
            position[0],
            position[1],
        )
    }
    pub fn resize_by(
        &self,
        layout: &mut DecorationLayout,
        id: HitId,
        handle: crate::resize::ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> Option<NudgeResult> {
        Some(self.map.get(id)?.as_resizable()?.resize_by(
            layout,
            &CpuTextMeasure::for_style(&Default::default()),
            handle,
            dx,
            dy,
        ))
    }
}
