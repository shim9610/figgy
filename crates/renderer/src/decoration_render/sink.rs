//! The canvas sink: the one place decoration parts become pixels, and the
//! selection box raster.
//!
//! Every decoration, axis, polar, colourbar and annotation element places its
//! parts through [`Sink`]; this sink paints them onto the CPU raster, one
//! canvas per [`Slot`]. The bounds, containment and selection sinks of
//! `model::select` run over the same emits, so the raster and the hit
//! geometry cannot disagree.

use crate::color::Color;
use crate::config::ColorBarOptions;
use crate::decoration_render::draw_colorbar_strip;
use crate::layout::RectF;
use crate::raster::{Canvas, Paint};
use crate::select::{SelectionBox, SelectionOutline, part_outline};
use crate::sink::{Border, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use crate::sketch::DecoStroker;
use crate::text_render::{FontPolicy, draw_plain_text, draw_rich_text};

/// Paints emitted parts onto up to two canvases: the one below the data and
/// the one above. A slot without a canvas drops its parts, which is how a
/// single-layer raster is produced from the same emit.
///
/// Coordinates arrive in the chart's logical frame and land at
/// `(p - origin) * scale`; text is drawn at `scale` times its size, so a
/// raster at a physical scale draws the same layout the hit map answers in
/// logical pixels.
pub(crate) struct CanvasSink<'a> {
    below: Option<&'a mut Canvas>,
    above: Option<&'a mut Canvas>,
    stroker: DecoStroker,
    policy: FontPolicy,
    origin: [f32; 2],
    scale: f32,
}

impl<'a> CanvasSink<'a> {
    /// A sink in the raster's own frame (origin zero, scale one).
    pub(crate) fn new(
        below: Option<&'a mut Canvas>,
        above: Option<&'a mut Canvas>,
        stroker: DecoStroker,
        policy: FontPolicy,
    ) -> Self {
        Self::placed(below, above, stroker, policy, [0.0, 0.0], 1.0)
    }

    /// A sink whose canvases cover the logical rectangle starting at
    /// `origin`, at `scale` physical pixels per logical pixel.
    pub(crate) fn placed(
        below: Option<&'a mut Canvas>,
        above: Option<&'a mut Canvas>,
        stroker: DecoStroker,
        policy: FontPolicy,
        origin: [f32; 2],
        scale: f32,
    ) -> Self {
        Self {
            below,
            above,
            stroker,
            policy,
            origin,
            scale,
        }
    }

    fn canvas(&mut self, slot: Slot) -> Option<&mut Canvas> {
        match slot {
            Slot::Below => self.below.as_deref_mut(),
            Slot::Above => self.above.as_deref_mut(),
        }
    }

    fn point(&self, p: [f32; 2]) -> (f32, f32) {
        (
            (p[0] - self.origin[0]) * self.scale,
            (p[1] - self.origin[1]) * self.scale,
        )
    }

    fn place_rect(&self, r: RectF) -> RectF {
        RectF {
            x: (r.x - self.origin[0]) * self.scale,
            y: (r.y - self.origin[1]) * self.scale,
            width: r.width * self.scale,
            height: r.height * self.scale,
        }
    }

    /// A stroke paint at the sink's scale. Dash patterns have at most eight
    /// entries, scaled into a fixed buffer.
    fn stroke_paint(&self, color: &Color, width: f32, dash: &[f32]) -> Paint {
        let paint = Paint::stroke(color, width * self.scale);
        if dash.is_empty() {
            return paint;
        }
        if self.scale == 1.0 {
            return paint.with_dash(dash);
        }
        let mut scaled = [0.0f32; 8];
        let n = dash.len().min(scaled.len());
        for (slot, value) in scaled.iter_mut().zip(dash) {
            *slot = value * self.scale;
        }
        paint.with_dash(&scaled[..n])
    }
}

impl Sink for CanvasSink<'_> {
    fn text(&mut self, slot: Slot, text: &PlacedText<'_>) {
        let origin = self.point([text.origin.0, text.origin.1]);
        let rotation = text
            .rotation
            .map(|(degrees, center)| (degrees, self.point([center.0, center.1])));
        let (scale, policy) = (self.scale, self.policy);
        let Some(canvas) = self.canvas(slot) else {
            return;
        };
        if let Some((degrees, center)) = rotation {
            canvas.save();
            canvas.rotate_at(degrees, center.0, center.1);
        }
        match text.text {
            TextRef::Plain {
                text,
                family,
                size,
                color,
            } => draw_plain_text(
                canvas,
                text,
                origin,
                color,
                family,
                size * scale,
                false,
                false,
                policy,
            ),
            TextRef::Rich(rich) => {
                if scale == 1.0 {
                    draw_rich_text(canvas, rich, origin, policy);
                } else {
                    let scaled = crate::decoration::scaled_rich_text(rich, scale);
                    draw_rich_text(canvas, &scaled, origin, policy);
                }
            }
        }
        if rotation.is_some() {
            canvas.restore();
        }
    }

    fn stroke(&mut self, slot: Slot, a: [f32; 2], b: [f32; 2], style: &StrokeStyle<'_>) {
        if style.color.a <= 0.0 {
            return;
        }
        let paint = self.stroke_paint(&style.color, style.width, style.dash);
        let (p0, p1) = (self.point(a), self.point(b));
        let stroker = self.stroker;
        if let Some(canvas) = self.canvas(slot) {
            stroker.stroke_segment(canvas, p0, p1, &paint, style.tag);
        }
    }

    fn rect(&mut self, slot: Slot, rect: RectF, fill: Option<Color>, border: Option<Border<'_>>) {
        let r = self.place_rect(rect);
        let border_paint = border.map(|b| (self.stroke_paint(&b.color, b.width, &[]), b.tag));
        let stroker = self.stroker;
        let Some(canvas) = self.canvas(slot) else {
            return;
        };
        if let Some(fill) = fill.filter(|c| c.a > 0.0) {
            canvas.draw_rect(r.x, r.y, r.width, r.height, &Paint::fill(&fill));
        }
        if let Some((paint, tag)) = border_paint {
            stroker.stroke_rect_outline(canvas, r.x, r.y, r.width, r.height, &paint, tag);
        }
    }

    fn triangle(&mut self, slot: Slot, points: [[f32; 2]; 3], color: Color) {
        if color.a <= 0.0 {
            return;
        }
        let pts = [
            self.point(points[0]),
            self.point(points[1]),
            self.point(points[2]),
        ];
        if let Some(canvas) = self.canvas(slot) {
            canvas.fill_polygon(&pts, &Paint::fill(&color));
        }
    }

    fn polygon(
        &mut self,
        slot: Slot,
        points: &[[f32; 2]],
        fill: Option<Color>,
        border: Option<&StrokeStyle<'_>>,
    ) {
        // The placed points, one list per shape drawn.
        let pts: Vec<(f32, f32)> = points.iter().map(|p| self.point(*p)).collect();
        let border_paint = border
            .filter(|b| b.color.a > 0.0 && b.width > 0.0)
            .map(|b| (self.stroke_paint(&b.color, b.width, b.dash), b.tag));
        let stroker = self.stroker;
        let Some(canvas) = self.canvas(slot) else {
            return;
        };
        if let Some(fill) = fill.filter(|c| c.a > 0.0) {
            canvas.fill_polygon(&pts, &Paint::fill(&fill));
        }
        if let Some((paint, tag)) = border_paint {
            stroker.stroke_polygon_outline(canvas, &pts, &paint, tag);
        }
    }

    fn ramp(&mut self, slot: Slot, rect: RectF, bar: &ColorBarOptions) {
        let r = self.place_rect(rect);
        if let Some(canvas) = self.canvas(slot) {
            draw_colorbar_strip(canvas, bar, &r);
        }
    }
}

/// Draw one selection highlight box, the raster realization of the model's
/// selection policy. `b` is in the chart frame; the canvas shows the chart
/// from `origin` at `scale`, so every coordinate and the stroke width are
/// mapped through `(p - origin) * scale`. A box that outlines its parts
/// strokes each part's [`part_outline`] (the shape its hit test answers
/// within, computed in the chart frame and then mapped, so the two agree at
/// every scale). Resize handles (when present) are drawn as white squares
/// with the selection color as border, slide-editor style.
pub fn draw_selection_box(canvas: &mut Canvas, b: &SelectionBox, origin: [f32; 2], scale: f32) {
    let map = |p: [f32; 2]| ((p[0] - origin[0]) * scale, (p[1] - origin[1]) * scale);
    let rect = |r: RectF| {
        let (x, y) = map([r.x, r.y]);
        (x, y, r.width * scale, r.height * scale)
    };
    let outline = Paint::stroke(&b.color(), b.stroke_width() * scale);
    match b.outline() {
        SelectionOutline::Rect => {
            let (x, y, w, h) = rect(b.rect());
            canvas.draw_rect(x, y, w, h, &outline);
        }
        SelectionOutline::Parts(parts) => {
            // Each part's corners and the closing point, one list per
            // part: a quad, a triangle or a shape's polygon.
            for part in parts {
                let corners = part_outline(part);
                let corners = corners.corners();
                let pts: Vec<(f32, f32)> = corners
                    .iter()
                    .chain(corners.first())
                    .map(|c| map(*c))
                    .collect();
                canvas.draw_polyline(&pts, &outline);
            }
        }
    }

    if b.handles().is_empty() {
        return;
    }
    let fill = Paint::fill(&Color::WHITE);
    for h in b.handles() {
        let (x, y, w, h) = rect(*h);
        canvas.draw_rect(x, y, w, h, &fill);
        canvas.draw_rect(x, y, w, h, &outline);
    }
}

/// Draw selection highlight boxes, each as [`draw_selection_box`].
pub fn draw_selection_boxes(
    canvas: &mut Canvas,
    boxes: &[SelectionBox],
    origin: [f32; 2],
    scale: f32,
) {
    for b in boxes {
        draw_selection_box(canvas, b, origin, scale);
    }
}
