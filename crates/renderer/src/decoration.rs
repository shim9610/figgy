#![doc = include_str!("../INTERACTION.md")]
//! Renderer-measured decoration hit maps. Data picking remains on the chart frame.
//! Coordinates and drag deltas are chart-local logical pixels, not framebuffer pixels.
use crate::raster::{Canvas, Paint};
use crate::text_render::{draw_plain_text, measure_plain_text, draw_rich_text, measure_rich_text};
use crate::{CpuTextMeasure, FontPolicy, HitId, HitMap};
use model::{
    DecorationElement, DecorationId, DecorationLayout,
    layout::{NudgeResult, RectF},
};


pub(crate) fn scaled_rich_text(text: &model::text::RichText, scale: f32) -> model::text::RichText {
    let mut text = text.clone();
    text.font_size *= scale;
    for segment in &mut text.segments {
        if let Some(size) = &mut segment.font_size { *size *= scale; }
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
        Some(self.map.get(id)?.as_draggable()?.drag_by(layout, dx, dy))
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
        Some(
            self.map
                .get(id)?
                .as_resizable()?
                .resize_by(layout, handle, dx, dy),
        )
    }
    pub(crate) fn draw_selection(&self, canvas: &mut Canvas, scale: f32) {
        let Some((id, _)) = self
            .entries
            .iter()
            .find(|(_, e)| Some(&e.id) == self.state.selected.as_ref())
        else {
            return;
        };
        if let Some(mut selection) = self.selection_box(*id) {
            selection.rect.x *= scale;
            selection.rect.y *= scale;
            selection.rect.width *= scale;
            selection.rect.height *= scale;
            selection.stroke_width *= scale;
            for handle in &mut selection.handles {
                handle.x *= scale;
                handle.y *= scale;
                handle.width *= scale;
                handle.height *= scale;
            }
            crate::axis_render::draw_selection_boxes(canvas, &[selection]);
        }
    }
}

pub(crate) struct DecorationPainter<'a> {
    state: &'a DecorationLayout,
    size: (u32, u32),
    pub scale: f32,
    enabled: bool,
    entries: Vec<DecorationElement>,
}
impl<'a> DecorationPainter<'a> {
    pub fn new(state: &'a DecorationLayout, size: (u32, u32), scale: f32, enabled: bool) -> Self {
        Self {
            state,
            size,
            scale,
            enabled,
            entries: vec![],
        }
    }
    pub fn shift_hit(&mut self, id: &DecorationId, dx: f32, dy: f32) {
        if let Some(e) = self.entries.iter_mut().find(|e| &e.id == id) {
            e.base_bounds = e.base_bounds.translated(dx, dy);
        }
    }
    pub fn offset(&self, id: &DecorationId) -> [f32; 2] {
        self.state.effective_offset(id)
    }
    pub fn add(&mut self, id: DecorationId, rect: RectF) {
        if rect.width <= 0. || rect.height <= 0. {
            return;
        }
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            let q = e.base_bounds;
            let x = q.x.min(rect.x);
            let y = q.y.min(rect.y);
            e.base_bounds = RectF {
                x,
                y,
                width: (q.x + q.width).max(rect.x + rect.width) - x,
                height: (q.y + q.height).max(rect.y + rect.height) - y,
            };
        } else {
            self.entries.push(DecorationElement {
                id,
                base_bounds: rect,
                canvas_size: self.size,
                followers: vec![],
            });
        }
    }
    pub fn text(
        &mut self,
        id: DecorationId,
        canvas: &mut Canvas,
        family: &str,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: crate::Color,
    ) {
        if text.is_empty() || color.a == 0. {
            return;
        }
        let m = measure_plain_text(
            text,
            family,
            size * self.scale,
            false,
            false,
            FontPolicy::Standard,
        );
        self.add(
            id.clone(),
            RectF {
                x,
                y: y - m.ascent / self.scale,
                width: m.width / self.scale,
                height: (m.ascent + m.descent) / self.scale,
            },
        );
        let p = self.offset(&id);
        if self.enabled {
            draw_plain_text(
                canvas,
                text,
                ((x + p[0]) * self.scale, (y + p[1]) * self.scale),
                color,
                family,
                size * self.scale,
                false,
                false,
                FontPolicy::Standard,
            );
        }
    }
    pub fn rich_text(&mut self, id: DecorationId, canvas: &mut Canvas,
        text: &model::text::RichText, x: f32, baseline: f32) {
        if text.segments.is_empty() { return; }
        let text = scaled_rich_text(text, self.scale);
        let m = measure_rich_text(&text, FontPolicy::Standard);
        self.add(id.clone(), RectF { x, y: baseline - m.ascent / self.scale,
            width: m.width / self.scale, height: m.height() / self.scale });
        let p = self.offset(&id);
        if self.enabled {
            draw_rich_text(canvas, &text,
                ((x + p[0]) * self.scale, (baseline + p[1]) * self.scale), FontPolicy::Standard);
        }
    }
    /// A -90 degree axis title. Offsets remain screen-space, so horizontal
    /// pointer motion always moves the visible text horizontally.
    pub fn rotated_text(
        &mut self,
        id: DecorationId,
        canvas: &mut Canvas,
        family: &str,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: crate::Color,
    ) {
        let m = measure_plain_text(
            text,
            family,
            size * self.scale,
            false,
            false,
            FontPolicy::Standard,
        );
        self.add(
            id.clone(),
            RectF {
                x: x - m.ascent / self.scale,
                y: y - m.width / self.scale * 0.5,
                width: (m.ascent + m.descent) / self.scale,
                height: m.width / self.scale,
            },
        );
        let p = self.offset(&id);
        if self.enabled {
            canvas.save();
            canvas.translate(p[0] * self.scale, p[1] * self.scale);
            canvas.rotate_at(-90., x * self.scale, y * self.scale);
            draw_plain_text(
                canvas,
                text,
                (x * self.scale - m.width * 0.5, y * self.scale),
                color,
                family,
                size * self.scale,
                false,
                false,
                FontPolicy::Standard,
            );
            canvas.restore();
        }
    }
    pub fn rect(
        &mut self,
        id: DecorationId,
        canvas: &mut Canvas,
        rect: RectF,
        color: crate::Color,
    ) {
        if color.a == 0. {
            return;
        }
        self.add(id.clone(), rect);
        let p = self.offset(&id);
        if self.enabled {
            canvas.draw_rect(
                (rect.x + p[0]) * self.scale,
                (rect.y + p[1]) * self.scale,
                rect.width * self.scale,
                rect.height * self.scale,
                &Paint::fill(&color),
            );
        }
    }
    pub fn axis(
        &mut self,
        id: DecorationId,
        canvas: &mut Canvas,
        a: (f32, f32),
        b: (f32, f32),
        color: crate::Color,
    ) {
        if color.a == 0. {
            return;
        }
        let rect = RectF {
            x: a.0.min(b.0) - 3.,
            y: a.1.min(b.1) - 3.,
            width: (a.0 - b.0).abs() + 6.,
            height: (a.1 - b.1).abs() + 6.,
        };
        self.add(id.clone(), rect);
        let p = self.offset(&id);
        if self.enabled {
            crate::axis_render::draw_precise_axis_segment(
                canvas,
                ((a.0 + p[0]) * self.scale, (a.1 + p[1]) * self.scale),
                ((b.0 + p[0]) * self.scale, (b.1 + p[1]) * self.scale),
                color,
                self.scale,
            );
        }
    }
    pub fn finish(mut self) -> DecorationMap {
        // Background axes first; foreground text/legend wins at overlaps.
        self.entries.sort_by_key(|e| match e.id {
            DecorationId::DataArea => 0,
            DecorationId::Axis(_) => 1,
            DecorationId::AxisLabel(_) => 2,
            _ => 3,
        });
        let labels: Vec<_> = self
            .entries
            .iter()
            .filter_map(|e| match &e.id {
                DecorationId::AxisLabel(side) => Some((side.clone(), e.id.clone(), e.base_bounds)),
                _ => None,
            })
            .collect();
        for e in &mut self.entries {
            if let DecorationId::Axis(side) = &e.id {
                e.followers = labels
                    .iter()
                    .filter(|(s, _, _)| s == side)
                    .map(|(_, id, r)| (id.clone(), *r))
                    .collect();
            }
        }
        let mut map = HitMap::for_state();
        let entries = self
            .entries
            .into_iter()
            .map(|e| (map.register(e.clone()), e))
            .collect();
        DecorationMap {
            state: self.state.clone(),
            map,
            entries,
            size: self.size,
        }
    }
}
