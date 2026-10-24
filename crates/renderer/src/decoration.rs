#![doc = include_str!("../INTERACTION.md")]
//! Renderer-measured decoration hit maps. Data picking remains on the chart frame.
//! Coordinates and drag deltas are chart-local logical pixels, not framebuffer pixels.
use crate::decoration_render::CanvasSink;
use crate::raster::Canvas;
use crate::sketch::DecoStroker;
use crate::text_render::measure_plain_text;
use crate::{CpuTextMeasure, FontPolicy, HitId, HitMap};
use model::{
    DecorationElement, DecorationId, DecorationLayout,
    color::Color,
    config::ColorBarOptions,
    layout::{NudgeResult, RectF},
    select::{STROKE_GRAB_HALF_WIDTH, SelectionGeometry},
    sink::{Border, DrawnPart, PlacedText, Sink, Slot, StrokeStyle, TextRef},
    text::TextExtents,
};

/// Bounded, deterministic placement of automatic label rectangles. Manual
/// offsets are applied afterwards by DecorationPainter and never fed back here.
pub(crate) struct LabelPlacer {
    bounds: RectF,
    axes: [bool; 2],
    placed: Vec<RectF>,
}
impl LabelPlacer {
    pub fn new(bounds: RectF, axes: [bool; 2]) -> Self {
        Self {
            bounds,
            axes,
            placed: Vec::new(),
        }
    }

    pub fn place(&mut self, preferred: RectF) -> RectF {
        let clamp = |r: RectF| RectF {
            x: r.x.clamp(
                self.bounds.x,
                self.bounds.x + (self.bounds.width - r.width).max(0.),
            ),
            y: r.y.clamp(
                self.bounds.y,
                self.bounds.y + (self.bounds.height - r.height).max(0.),
            ),
            ..r
        };
        // Search from the nearest in-bounds position. Starting at a clipped
        // statistic far outside the viewport would collapse every candidate
        // onto the same edge.
        let origin = clamp(preferred);
        let mut best = origin;
        let mut best_score = (f32::INFINITY, f32::INFINITY);
        // At most nine candidates, including the original position. A crowded
        // canvas still yields a frame; there is no unbounded relaxation loop.
        for (dx, dy) in [
            (0., 0.),
            (0., 1.),
            (0., -1.),
            (1., 0.),
            (-1., 0.),
            (0., 2.),
            (0., -2.),
            (2., 0.),
            (-2., 0.),
        ] {
            let x = origin.x
                + if self.axes[0] {
                    dx * (preferred.width + 4.)
                } else {
                    0.
                };
            let y = origin.y
                + if self.axes[1] {
                    dy * (preferred.height + 4.)
                } else {
                    0.
                };
            let r = clamp(RectF { x, y, ..preferred });
            let overlap: f32 = self
                .placed
                .iter()
                .map(|q| {
                    ((r.x + r.width + 3.).min(q.x + q.width + 3.) - r.x.max(q.x)).max(0.)
                        * ((r.y + r.height + 2.).min(q.y + q.height + 2.) - r.y.max(q.y)).max(0.)
                })
                .sum();
            let distance = (r.x - preferred.x).powi(2) + (r.y - preferred.y).powi(2);
            if (overlap, distance) < best_score {
                best = r;
                best_score = (overlap, distance);
            }
            if overlap == 0. && distance == 0. {
                break;
            }
        }
        self.placed.push(best);
        best
    }
}

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
    pub(crate) fn draw_selection(&self, canvas: &mut Canvas, scale: f32) {
        let Some((id, _)) = self
            .entries
            .iter()
            .find(|(_, e)| Some(&e.id) == self.state.selected.as_ref())
        else {
            return;
        };
        if let Some(selection) = self.selection_box(*id) {
            crate::axis_render::draw_selection_box(canvas, &selection, [0.0, 0.0], scale);
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
    /// Keep the recorded bounds of `id` inside `to`: an element the canvas
    /// clips (a radial title wider than the canvas) is hit, selected and
    /// dragged as what the canvas shows.
    pub fn clip_hit(&mut self, id: &DecorationId, to: RectF) {
        if let Some(e) = self.entries.iter_mut().find(|e| &e.id == id) {
            let q = e.base_bounds;
            let (x0, y0) = (q.x.max(to.x), q.y.max(to.y));
            let x1 = (q.x + q.width).min(to.x + to.width);
            let y1 = (q.y + q.height).min(to.y + to.height);
            e.base_bounds = RectF {
                x: x0,
                y: y0,
                width: (x1 - x0).max(0.),
                height: (y1 - y0).max(0.),
            };
        }
    }
    pub fn place_text(
        &self,
        placer: &mut LabelPlacer,
        family: &str,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
    ) -> (f32, f32) {
        let m = measure_plain_text(
            text,
            family,
            size * self.scale,
            false,
            false,
            FontPolicy::Standard,
        );
        let preferred = RectF {
            x,
            y: y - m.ascent / self.scale,
            width: m.width / self.scale,
            height: (m.ascent + m.descent) / self.scale,
        };
        let placed = placer.place(preferred);
        (x + placed.x - preferred.x, y + placed.y - preferred.y)
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
    /// The sink an element's parts are placed through: every part is
    /// recorded in `id`'s bounds and, when the painter draws, painted on
    /// `canvas` at the element's effective offset.
    pub fn on<'c>(&'c mut self, id: DecorationId, canvas: &'c mut Canvas) -> ElementSink<'c, 'a> {
        ElementSink {
            painter: self,
            id: Some(id),
            canvas,
        }
    }
    /// The sink for chrome that is no element, such as grid lines and radar
    /// spokes: parts are painted at zero offset and recorded nowhere.
    pub fn chrome<'c>(&'c mut self, canvas: &'c mut Canvas) -> ElementSink<'c, 'a> {
        ElementSink {
            painter: self,
            id: None,
            canvas,
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

/// One element (or chrome) being placed by a bounded chart: the bounded
/// canvas-plus-bounds sink. Every part is recorded in the element's bounds
/// (chrome records nothing) and, when the painter draws, painted through a
/// [`CanvasSink`] so a part placed at logical `p` lands at
/// `(p + offset) * scale`. The caller chose the canvas, so the slot only
/// tells the canvas sink where to find it.
pub(crate) struct ElementSink<'c, 'a> {
    painter: &'c mut DecorationPainter<'a>,
    id: Option<DecorationId>,
    canvas: &'c mut Canvas,
}

impl ElementSink<'_, '_> {
    fn record(&mut self, part: DrawnPart) {
        if let Some(id) = &self.id {
            self.painter.add(id.clone(), part.extent());
        }
    }

    /// The canvas sink a part is painted through, `None` when the painter
    /// only measures.
    fn paint(&mut self, slot: Slot) -> Option<CanvasSink<'_>> {
        if !self.painter.enabled {
            return None;
        }
        let origin = match &self.id {
            Some(id) => {
                let offset = self.painter.offset(id);
                [-offset[0], -offset[1]]
            }
            None => [0.0, 0.0],
        };
        let canvas = &mut *self.canvas;
        let (below, above) = match slot {
            Slot::Below => (Some(canvas), None),
            Slot::Above => (None, Some(canvas)),
        };
        Some(CanvasSink::placed(
            below,
            above,
            DecoStroker::Precise,
            FontPolicy::Standard,
            origin,
            self.painter.scale,
        ))
    }

    /// Plain text at baseline origin `(x, y)`, `size` logical pixels high,
    /// measured once at the painter's scale.
    pub fn plain_text(
        &mut self,
        family: &str,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
    ) {
        if text.is_empty() || color.a == 0. {
            return;
        }
        let extents = self.plain_extents(family, text, size);
        self.text(
            Slot::Above,
            &PlacedText {
                text: TextRef::Plain {
                    text,
                    family,
                    size,
                    color,
                },
                origin: (x, y),
                extents,
                rotation: None,
            },
        );
    }

    /// A -90 degree axis title centred on `(x, y)`. Offsets remain
    /// screen-space, so horizontal pointer motion always moves the visible
    /// text horizontally.
    pub fn rotated_text(
        &mut self,
        family: &str,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
    ) {
        let extents = self.plain_extents(family, text, size);
        self.text(
            Slot::Above,
            &PlacedText {
                text: TextRef::Plain {
                    text,
                    family,
                    size,
                    color,
                },
                origin: (x - extents.width * 0.5, y),
                extents,
                rotation: Some((-90.0, (x, y))),
            },
        );
    }

    /// A filled rectangle, such as a legend swatch.
    pub fn fill(&mut self, rect: RectF, color: Color) {
        if color.a == 0. {
            return;
        }
        self.rect(Slot::Above, rect, Some(color), None);
    }

    /// A one logical pixel wide line: an element's axis above the data, or
    /// chrome such as a grid line or a radar spoke below it.
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], color: Color) {
        if color.a == 0. {
            return;
        }
        let slot = if self.id.is_some() {
            Slot::Above
        } else {
            Slot::Below
        };
        self.stroke(
            slot,
            a,
            b,
            &StrokeStyle {
                width: 1.0,
                color,
                dash: &[],
                tag: "bounded_axis",
            },
        );
    }

    fn plain_extents(&self, family: &str, text: &str, size: f32) -> TextExtents {
        let scale = self.painter.scale;
        let m = measure_plain_text(
            text,
            family,
            size * scale,
            false,
            false,
            FontPolicy::Standard,
        );
        TextExtents {
            width: m.width / scale,
            ascent: m.ascent / scale,
            descent: m.descent / scale,
        }
    }
}

impl Sink for ElementSink<'_, '_> {
    fn text(&mut self, slot: Slot, text: &PlacedText<'_>) {
        self.record(DrawnPart::Text(text.rect()));
        if let Some(mut sink) = self.paint(slot) {
            sink.text(slot, text);
        }
    }

    fn stroke(&mut self, slot: Slot, a: [f32; 2], b: [f32; 2], style: &StrokeStyle<'_>) {
        // A thin line keeps the grab band its hit test answers within.
        self.record(DrawnPart::Segment {
            a,
            b,
            width: style.width.max(2.0 * STROKE_GRAB_HALF_WIDTH),
        });
        if let Some(mut sink) = self.paint(slot) {
            sink.stroke(slot, a, b, style);
        }
    }

    fn rect(&mut self, slot: Slot, rect: RectF, fill: Option<Color>, border: Option<Border<'_>>) {
        self.record(DrawnPart::Rect(rect));
        if let Some(mut sink) = self.paint(slot) {
            sink.rect(slot, rect, fill, border);
        }
    }

    fn triangle(&mut self, slot: Slot, points: [[f32; 2]; 3], color: Color) {
        self.record(DrawnPart::Triangle(points));
        if let Some(mut sink) = self.paint(slot) {
            sink.triangle(slot, points, color);
        }
    }

    fn ramp(&mut self, slot: Slot, rect: RectF, bar: &ColorBarOptions) {
        self.record(DrawnPart::Rect(rect));
        if let Some(mut sink) = self.paint(slot) {
            sink.ramp(slot, rect, bar);
        }
    }
}
