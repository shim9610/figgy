//! The vector export: SVG and PDF files of a chart whose decorations are
//! vector content over a raster data layer (#48).
//!
//! **One emit, more sinks.** The file's decorations come from the same
//! placement pass the raster draws with: [`emit_chart_chrome`] for every
//! panel, [`emit_annotation_layer`] for the annotation layer, the radial
//! chart's [`emit_decoration`](crate::radial) for its labels and title. The
//! emits run on the same scaled, placed document as the PNG export
//! ([`Renderer::prepare_export`]), and [`VectorSink`] hands each part to a
//! [`VectorOut`], the SVG or PDF writer, instead of painting it. So the
//! decorations in the file are the drawn ones by construction.
//!
//! **Order.** The writer reproduces the compositor's z-order
//! (`renderer/composite.rs`): for each panel its [`Slot::Below`] parts, the
//! annotation underlay on the base panel, and the data image of its panel
//! group; then for each panel its [`Slot::Above`] parts, with the annotation
//! overlay after the topmost panel of the parent's area. A [`SlotFilter`]
//! keeps one slot of an emit, which runs once per slot. A group is a panel
//! with the following panels whose `Below` pass places nothing (a
//! [`SlotFilter`] over a counting sink probes it): overlay panels emit no
//! grid, so a plain or overlay document has one data image and each subplot
//! adds one ([`Passes`]).
//!
//! **Data.** The data layer is drawn by the GPU from the column pool, so it
//! is embedded as an image: [`Renderer::paint_export`] paints one group with
//! the chrome off and the image is cropped to the group's data areas. The
//! colourbar ramp and the Milkyway backdrop are images too, painted by the
//! raster's own functions; the Milkyway glow is raster-only and omitted.
//!
//! **DataLess series** (#234). An energy-level series is drawn on the CPU
//! between its panel's GPU data and its value labels. The file writes it
//! from the emit its raster uses, as vector parts clipped to each data
//! piece, between one image of the GPU steps before it and one of those
//! after it ([`DataLess`], [`DataRun`]), so the file stacks as the PNG does.
//!
//! One user unit is one physical pixel of the export; the page is the
//! logical size at 1 px = 0.75 pt.

use std::ops::Range;

use swash::FontRef;

use crate::annotation::{RasterRequest, emit_annotation_layer, pixel_space_for_config};
use crate::axis_render::{draw_space_background, emit_chart_chrome};
use crate::color::Color;
use crate::config::{ColorBarOptions, Config, DrawStyle};
use crate::data_config::SeriesConfig;
use crate::data_render::clamp_rect_to_target;
use crate::decoration_render::draw_colorbar_strip;
use crate::error::{FiggyError, Result};
use crate::layout::{Rect, RectF};
use crate::raster::Canvas;
use crate::renderer::{DataSteps, ExportFrame, RasterImage, Renderer};
use crate::sink::{Border, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use crate::sketch::{DecoStroker, PathSegment};
use crate::text_render::{FontPolicy, GlyphSink, place_plain_text, place_rich_text};

mod pdf;
mod svg;
pub(crate) use pdf::PdfOut;
pub(crate) use svg::SvgOut;

/// The page size in points of a `logical` pixel size: 1 px = 0.75 pt.
pub(crate) fn page_points(logical: (u32, u32)) -> [f32; 2] {
    [logical.0 as f32 * 0.75, logical.1 as f32 * 0.75]
}

/// How a path is painted. Strokes have butt caps, miter joins with limit 4
/// and the dash pattern given, as the raster's tiny-skia strokes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PathPaint<'a> {
    Fill(Color),
    Stroke {
        color: Color,
        width: f32,
        dash: &'a [f32],
    },
}

/// A vector file being written: the SVG or PDF writer. Coordinates are in
/// the export's physical pixels, y down. Paths are written segment by
/// segment between [`Self::begin_path`] and [`Self::end_path`]; a path with
/// a non-finite coordinate is dropped.
pub(crate) trait VectorOut {
    fn begin_path(&mut self, paint: PathPaint<'_>);
    fn segment(&mut self, segment: PathSegment);
    fn end_path(&mut self);
    /// Start a block of text at `origin`, rotated by `rotation` (degrees
    /// clockwise about a centre) when given: glyphs and rules until
    /// [`Self::end_text`] are relative to `origin`.
    fn begin_text(&mut self, origin: [f32; 2], rotation: Option<(f32, [f32; 2])>);
    /// The outline of glyph `glyph` of `font` at `size`, its pen at `pen`
    /// relative to the text origin.
    fn glyph(&mut self, font: FontRef<'static>, glyph: u16, size: f32, pen: [f32; 2], color: Color);
    fn end_text(&mut self);
    /// Clip what follows, until [`Self::pop_clip`], to `rect`.
    fn push_clip(&mut self, rect: Rect);
    fn pop_clip(&mut self);
    /// A straight-alpha image with its top-left pixel at `origin`, one image
    /// pixel per user unit.
    fn image(&mut self, origin: [i32; 2], image: &RasterImage) -> Result<()>;
}

/// Write the axis-aligned rectangle `r` as a closed subpath.
pub(crate) fn rect_path(out: &mut dyn VectorOut, r: RectF) {
    out.segment(PathSegment::Move((r.x, r.y)));
    out.segment(PathSegment::Line((r.x + r.width, r.y)));
    out.segment(PathSegment::Line((r.x + r.width, r.y + r.height)));
    out.segment(PathSegment::Line((r.x, r.y + r.height)));
    out.segment(PathSegment::Close);
}

/// Whether a rectangle has an area to paint.
fn has_area(r: RectF) -> bool {
    r.x.is_finite()
        && r.y.is_finite()
        && r.width.is_finite()
        && r.height.is_finite()
        && r.width > 0.0
        && r.height > 0.0
}

/// The dash pattern tiny-skia strokes with: an even number (at least two)
/// of non-negative lengths with a positive sum, or solid.
fn valid_dash(dash: &[f32]) -> &[f32] {
    let sum: f32 = dash.iter().sum();
    if dash.len() >= 2 && dash.len() % 2 == 0 && dash.iter().all(|d| *d >= 0.0) && sum > 0.0 {
        dash
    } else {
        &[]
    }
}

/// Keeps the parts of one slot and drops the other: an emit runs once per
/// slot, so the writer keeps no per-slot buffer.
pub(crate) struct SlotFilter<'a> {
    pub(crate) slot: Slot,
    pub(crate) inner: &'a mut dyn Sink,
}

impl Sink for SlotFilter<'_> {
    fn text(&mut self, slot: Slot, text: &PlacedText<'_>) {
        if slot == self.slot {
            self.inner.text(slot, text);
        }
    }
    fn stroke(&mut self, slot: Slot, a: [f32; 2], b: [f32; 2], style: &StrokeStyle<'_>) {
        if slot == self.slot {
            self.inner.stroke(slot, a, b, style);
        }
    }
    fn rect(&mut self, slot: Slot, rect: RectF, fill: Option<Color>, border: Option<Border<'_>>) {
        if slot == self.slot {
            self.inner.rect(slot, rect, fill, border);
        }
    }
    fn triangle(&mut self, slot: Slot, points: [[f32; 2]; 3], color: Color) {
        if slot == self.slot {
            self.inner.triangle(slot, points, color);
        }
    }
    fn path(&mut self, slot: Slot, segments: &[PathSegment], style: &StrokeStyle<'_>) {
        if slot == self.slot {
            self.inner.path(slot, segments, style);
        }
    }
    fn polygon(
        &mut self,
        slot: Slot,
        points: &[[f32; 2]],
        fill: Option<Color>,
        border: Option<&StrokeStyle<'_>>,
    ) {
        if slot == self.slot {
            self.inner.polygon(slot, points, fill, border);
        }
    }
    fn ramp(&mut self, slot: Slot, rect: RectF, bar: &ColorBarOptions) {
        if slot == self.slot {
            self.inner.ramp(slot, rect, bar);
        }
    }
}

/// Counts the parts that paint something: the group probe.
#[derive(Default)]
struct PaintCount(usize);

impl Sink for PaintCount {
    fn text(&mut self, _: Slot, _: &PlacedText<'_>) {
        self.0 += 1;
    }
    fn stroke(&mut self, _: Slot, _: [f32; 2], _: [f32; 2], style: &StrokeStyle<'_>) {
        self.0 += usize::from(style.color.a > 0.0);
    }
    fn rect(&mut self, _: Slot, _: RectF, fill: Option<Color>, border: Option<Border<'_>>) {
        self.0 += usize::from(fill.is_some_and(|c| c.a > 0.0) || border.is_some());
    }
    fn triangle(&mut self, _: Slot, _: [[f32; 2]; 3], color: Color) {
        self.0 += usize::from(color.a > 0.0);
    }
    fn path(&mut self, _: Slot, _: &[PathSegment], style: &StrokeStyle<'_>) {
        self.0 += usize::from(style.color.a > 0.0);
    }
    fn polygon(
        &mut self,
        _: Slot,
        _: &[[f32; 2]],
        fill: Option<Color>,
        border: Option<&StrokeStyle<'_>>,
    ) {
        self.0 += usize::from(
            fill.is_some_and(|c| c.a > 0.0)
                || border.is_some_and(|b| b.color.a > 0.0 && b.width > 0.0),
        );
    }
    fn ramp(&mut self, _: Slot, _: RectF, _: &ColorBarOptions) {
        self.0 += 1;
    }
}

/// A raster canvas as a straight-alpha image: the canvas is premultiplied,
/// the export's images are not (the PNG export's conversion).
pub(crate) fn canvas_image(canvas: Canvas) -> RasterImage {
    let (width, height) = canvas.size();
    let mut rgba = canvas.into_rgba();
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        if a != 0 && a != 255 {
            let a_f = a as f32 / 255.0;
            for c in &mut px[..3] {
                *c = ((*c as f32 / a_f).round().clamp(0.0, 255.0)) as u8;
            }
        }
    }
    RasterImage {
        width,
        height,
        rgba,
    }
}

/// Cut `image` down to `area` in place (`area` inside the image).
pub(crate) fn crop_in_place(image: &mut RasterImage, area: Rect) {
    let width = image.width as usize;
    let row = area.width as usize * 4;
    for y in 0..area.height as usize {
        let src = ((area.y as usize + y) * width + area.x as usize) * 4;
        image.rgba.copy_within(src..src + row, y * row);
    }
    image.rgba.truncate(row * area.height as usize);
    image.width = area.width;
    image.height = area.height;
}

/// The intersection of two pixel rectangles, `None` when empty.
fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    (x1 > x0 && y1 > y0).then(|| Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// The pixels of the image a colourbar strip `r` (output pixels) is
/// written as: the pixels it covers, cut to `clip`.
fn ramp_area(r: RectF, clip: Rect) -> Option<Rect> {
    if !has_area(r) {
        return None;
    }
    let x0 = r.x.floor().max(0.0);
    let y0 = r.y.floor().max(0.0);
    let x1 = (r.x + r.width).ceil().min(u32::MAX as f32);
    let y1 = (r.y + r.height).ceil().min(u32::MAX as f32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let covered = Rect {
        x: x0 as u32,
        y: y0 as u32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    };
    intersect(covered, clip)
}

/// Hands every placed part to a [`VectorOut`]: the vector twin of the
/// canvas sink (`decoration_render::CanvasSink`), with the same mapping
/// `(p - origin) * scale`, the same stroke widths, width floors, dashes and
/// sketch curves, and the same text placement. Text glyphs are written at
/// their exact pens (the raster snaps them to its pixel grid).
pub(crate) struct VectorSink<'a> {
    out: &'a mut dyn VectorOut,
    stroker: DecoStroker,
    policy: FontPolicy,
    origin: [f32; 2],
    scale: f32,
    /// The rectangle the raster's texture covers: a ramp image is cut to it.
    clip: Rect,
    /// The origin of the text being written, in output pixels.
    text_origin: [f32; 2],
    /// The first image that could not be written.
    pub(crate) error: Option<FiggyError>,
}

impl<'a> VectorSink<'a> {
    pub(crate) fn new(
        out: &'a mut dyn VectorOut,
        stroker: DecoStroker,
        policy: FontPolicy,
        origin: [f32; 2],
        scale: f32,
        clip: Rect,
    ) -> Self {
        Self {
            out,
            stroker,
            policy,
            origin,
            scale,
            clip,
            text_origin: [0.0, 0.0],
            error: None,
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

    /// Stroke `points` with `color` at `width` logical pixels and `dash`:
    /// the precise stroker's straight path (`closed` for a rectangle
    /// outline), or the sketch stroker's wobble curve through
    /// `sketch(amplitude, wavelength, seed)`.
    fn stroke_path(
        &mut self,
        color: Color,
        width: f32,
        dash: &[f32],
        precise: &[(f32, f32)],
        closed: bool,
        sketch: impl FnOnce(f32, f32, u32) -> Vec<(f32, f32)>,
        tag: &str,
    ) {
        // `Paint::stroke` raises a width under one pixel to one.
        let width = (width * self.scale).max(1.0);
        let mut scaled = [0.0f32; 8];
        let dash = if dash.is_empty() || self.scale == 1.0 {
            dash
        } else {
            let n = dash.len().min(scaled.len());
            for (slot, value) in scaled.iter_mut().zip(dash) {
                *slot = value * self.scale;
            }
            &scaled[..n]
        };
        let dash = valid_dash(dash);
        match self.stroker {
            DecoStroker::Precise => {
                self.out
                    .begin_path(PathPaint::Stroke { color, width, dash });
                for (i, &p) in precise.iter().enumerate() {
                    self.out.segment(if i == 0 {
                        PathSegment::Move(p)
                    } else {
                        PathSegment::Line(p)
                    });
                }
                if closed {
                    self.out.segment(PathSegment::Close);
                }
                self.out.end_path();
            }
            DecoStroker::Sketch {
                amplitude_px,
                wavelength_px,
                seed,
            } => {
                let points = sketch(
                    amplitude_px,
                    wavelength_px,
                    seed ^ crate::sketch::fnv1a(tag),
                );
                let width = width.max(crate::sketch::SKETCH_MIN_STROKE_PX);
                self.out
                    .begin_path(PathPaint::Stroke { color, width, dash });
                let out = &mut *self.out;
                crate::sketch::smooth_cubics(&points, |segment| out.segment(segment));
                self.out.end_path();
            }
        }
    }
}

impl Sink for VectorSink<'_> {
    fn text(&mut self, _slot: Slot, text: &PlacedText<'_>) {
        let origin = self.point([text.origin.0, text.origin.1]);
        let rotation = text.rotation.map(|(degrees, center)| {
            let c = self.point([center.0, center.1]);
            (degrees, [c.0, c.1])
        });
        self.text_origin = [origin.0, origin.1];
        self.out.begin_text([origin.0, origin.1], rotation);
        let (scale, policy) = (self.scale, self.policy);
        match text.text {
            TextRef::Plain {
                text,
                family,
                size,
                color,
            } => place_plain_text(
                self,
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
                    place_rich_text(self, rich, origin, policy);
                } else {
                    let scaled = crate::decoration::scaled_rich_text(rich, scale);
                    place_rich_text(self, &scaled, origin, policy);
                }
            }
        }
        self.out.end_text();
    }

    fn stroke(&mut self, _slot: Slot, a: [f32; 2], b: [f32; 2], style: &StrokeStyle<'_>) {
        if style.color.a <= 0.0 {
            return;
        }
        let (p0, p1) = (self.point(a), self.point(b));
        self.stroke_path(
            style.color,
            style.width,
            style.dash,
            &[p0, p1],
            false,
            |amplitude, wavelength, seed| {
                crate::sketch::sketch_line(p0, p1, amplitude, wavelength, seed)
            },
            style.tag,
        );
    }

    fn rect(&mut self, _slot: Slot, rect: RectF, fill: Option<Color>, border: Option<Border<'_>>) {
        let r = self.place_rect(rect);
        if !has_area(r) {
            return;
        }
        if let Some(fill) = fill.filter(|c| c.a > 0.0) {
            self.out.begin_path(PathPaint::Fill(fill));
            rect_path(self.out, r);
            self.out.end_path();
        }
        if let Some(b) = border {
            let corners = [
                (r.x, r.y),
                (r.x + r.width, r.y),
                (r.x + r.width, r.y + r.height),
                (r.x, r.y + r.height),
            ];
            self.stroke_path(
                b.color,
                b.width,
                &[],
                &corners,
                true,
                |amplitude, wavelength, seed| {
                    crate::sketch::sketch_rect_outline(
                        r.x, r.y, r.width, r.height, amplitude, wavelength, seed,
                    )
                },
                b.tag,
            );
        }
    }

    fn triangle(&mut self, _slot: Slot, points: [[f32; 2]; 3], color: Color) {
        if color.a <= 0.0 {
            return;
        }
        self.out.begin_path(PathPaint::Fill(color));
        for (i, p) in points.iter().enumerate() {
            let p = self.point(*p);
            self.out.segment(if i == 0 {
                PathSegment::Move(p)
            } else {
                PathSegment::Line(p)
            });
        }
        self.out.segment(PathSegment::Close);
        self.out.end_path();
    }

    fn path(&mut self, _slot: Slot, segments: &[PathSegment], style: &StrokeStyle<'_>) {
        if style.color.a <= 0.0 {
            return;
        }
        // As the canvas: one path, a width under a pixel raised to one, the
        // dash scaled, the shape as placed in every style.
        let width = (style.width * self.scale).max(1.0);
        let mut scaled = [0.0f32; 8];
        let n = style.dash.len().min(scaled.len());
        for (slot, value) in scaled.iter_mut().zip(style.dash) {
            *slot = value * self.scale;
        }
        let dash = valid_dash(&scaled[..n]);
        self.out.begin_path(PathPaint::Stroke {
            color: style.color,
            width,
            dash,
        });
        for segment in segments {
            let mapped = match *segment {
                PathSegment::Move(p) => PathSegment::Move(self.point([p.0, p.1])),
                PathSegment::Line(p) => PathSegment::Line(self.point([p.0, p.1])),
                PathSegment::Cubic(c1, c2, p) => PathSegment::Cubic(
                    self.point([c1.0, c1.1]),
                    self.point([c2.0, c2.1]),
                    self.point([p.0, p.1]),
                ),
                PathSegment::Close => PathSegment::Close,
            };
            self.out.segment(mapped);
        }
        self.out.end_path();
    }

    fn polygon(
        &mut self,
        _slot: Slot,
        points: &[[f32; 2]],
        fill: Option<Color>,
        border: Option<&StrokeStyle<'_>>,
    ) {
        if points.len() < 3 {
            return;
        }
        // The placed points, one list per shape written.
        let pts: Vec<(f32, f32)> = points.iter().map(|p| self.point(*p)).collect();
        if let Some(fill) = fill.filter(|c| c.a > 0.0) {
            self.out.begin_path(PathPaint::Fill(fill));
            for (i, p) in pts.iter().enumerate() {
                self.out.segment(if i == 0 {
                    PathSegment::Move(*p)
                } else {
                    PathSegment::Line(*p)
                });
            }
            self.out.segment(PathSegment::Close);
            self.out.end_path();
        }
        if let Some(b) = border.filter(|b| b.color.a > 0.0 && b.width > 0.0) {
            let closed: Vec<(f32, f32)> = pts.iter().copied().chain(pts.first().copied()).collect();
            self.stroke_path(
                b.color,
                b.width,
                b.dash,
                &pts,
                true,
                |amplitude, wavelength, seed| {
                    crate::sketch::sketch_polyline(&closed, amplitude, wavelength, seed)
                },
                b.tag,
            );
        }
    }

    /// The strip painted by the raster's own ramp painter into a canvas of
    /// the pixels it covers, cut to the panel, so its bands are the PNG's.
    fn ramp(&mut self, _slot: Slot, rect: RectF, bar: &ColorBarOptions) {
        let r = self.place_rect(rect);
        let Some(area) = ramp_area(r, self.clip) else {
            return;
        };
        let Some(mut canvas) = Canvas::new(area.width, area.height) else {
            return;
        };
        canvas.translate(-(area.x as f32), -(area.y as f32));
        draw_colorbar_strip(&mut canvas, bar, &r);
        let image = canvas_image(canvas);
        if let Err(e) = self.out.image([area.x as i32, area.y as i32], &image) {
            self.error.get_or_insert(e);
        }
    }
}

impl GlyphSink for VectorSink<'_> {
    fn paints(&self) -> bool {
        true
    }

    fn glyph(
        &mut self,
        font: FontRef<'static>,
        glyph: u16,
        size: f32,
        pen: (f32, f32),
        color: &Color,
    ) {
        let pen = [pen.0 - self.text_origin[0], pen.1 - self.text_origin[1]];
        self.out.glyph(font, glyph, size, pen, *color);
    }

    fn rect(&mut self, r: RectF, color: &Color) {
        let r = RectF {
            x: r.x - self.text_origin[0],
            y: r.y - self.text_origin[1],
            ..r
        };
        if !has_area(r) {
            return;
        }
        self.out.begin_path(PathPaint::Fill(*color));
        rect_path(self.out, r);
        self.out.end_path();
    }
}

// The passes of a document.

/// One pass of a document's vector export, in writing order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pass {
    /// Panel `i`'s parts below the data, or its Milkyway backdrop.
    Below(usize),
    /// The annotation layer's spans, under the base panel's data.
    Underlay,
    /// The data image of the panel group `range`.
    Data(Range<usize>),
    /// Panel `i`'s parts above the data.
    Above(usize),
    /// The annotation layer's lines, arrows and text, after the topmost
    /// panel of the parent's area.
    Overlay,
}

/// Whether panel `index` of `document` (in [`Config::panel_layers`] order)
/// is the first panel of its area, the one that owns the Milkyway backdrop:
/// the base panel and each subplot, not their overlays or line offset
/// members.
fn owns_backdrop(document: &Config, index: usize) -> bool {
    let charts = std::iter::once(document).chain(document.subplots.iter().map(|s| &s.config));
    let mut start = 0;
    for chart in charts {
        if index == start {
            return true;
        }
        let members = chart
            .line_offsets
            .as_ref()
            .map_or(0, |offsets| offsets.series.len());
        start += 1 + chart.overlays.len() + members;
        if index < start {
            return false;
        }
    }
    false
}

/// Whether panel `index`'s `Below` pass places anything: its Milkyway
/// backdrop, or a part of its chrome below the data (the probe).
fn below_places(document: &Config, panels: &[Config], index: usize) -> bool {
    let panel = &panels[index];
    if matches!(panel.draw_style, DrawStyle::Milkyway(_)) {
        // The backdrop replaces the grid; no chrome part goes below.
        return owns_backdrop(document, index);
    }
    let mut count = PaintCount::default();
    emit_chart_chrome(
        panel,
        &crate::text_render::CpuTextMeasure::for_style(&panel.draw_style),
        &mut SlotFilter {
            slot: Slot::Below,
            inner: &mut count,
        },
    );
    count.0 > 0
}

/// The end of the panel group starting at `start`.
fn group_end(document: &Config, panels: &[Config], start: usize) -> usize {
    let mut end = start + 1;
    while end < panels.len() && !below_places(document, panels, end) {
        end += 1;
    }
    end
}

/// The topmost panel drawn in the parent's area: where the PNG lifts the
/// annotation overlay (`frame::lift_annotation_overlays`).
fn overlay_panel(panels: &[Config]) -> usize {
    let base = panels[0].chart_area.0;
    (0..panels.len())
        .rev()
        .find(|&i| panels[i].chart_area.0 == base)
        .unwrap_or(0)
}

/// The data area panel `panel` draws its data slot in (the prepared
/// item's `data_area`).
fn panel_data_area(panel: &Config) -> Rect {
    panel
        .data_area()
        .map(|area| area.0)
        .unwrap_or(panel.chart_area.0)
}

/// How far past its data area a data image reaches: the GPU's coverage at
/// the data scissor's edge can reach the next pixel (lavapipe paints the
/// column left of the scissor), and the image keeps every pixel the PNG
/// export has.
const DATA_IMAGE_MARGIN: u32 = 1;

/// The pixels of the data image of panels `range`: the union of their data
/// areas, each grown by [`DATA_IMAGE_MARGIN`] inside its panel, inside the
/// target; `None` when it is empty.
pub(crate) fn group_image_area(
    panels: &[Config],
    range: Range<usize>,
    size: (u32, u32),
) -> Option<Rect> {
    let target = Rect {
        x: 0,
        y: 0,
        width: size.0,
        height: size.1,
    };
    let mut union: Option<Rect> = None;
    for panel in &panels[range] {
        let data = panel_data_area(panel);
        let x0 = data.x.saturating_sub(DATA_IMAGE_MARGIN);
        let y0 = data.y.saturating_sub(DATA_IMAGE_MARGIN);
        let grown = Rect {
            x: x0,
            y: y0,
            width: (data.x + data.width).saturating_add(DATA_IMAGE_MARGIN) - x0,
            height: (data.y + data.height).saturating_add(DATA_IMAGE_MARGIN) - y0,
        };
        let Some(area) = intersect(grown, panel.chart_area.0).and_then(|a| intersect(a, target))
        else {
            continue;
        };
        union = Some(match union {
            None => area,
            Some(u) => {
                let x0 = u.x.min(area.x);
                let y0 = u.y.min(area.y);
                let x1 = (u.x + u.width).max(area.x + area.width);
                let y1 = (u.y + u.height).max(area.y + area.height);
                Rect {
                    x: x0,
                    y: y0,
                    width: x1 - x0,
                    height: y1 - y0,
                }
            }
        });
    }
    union
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    Start,
    Below {
        start: usize,
        end: usize,
        next: usize,
    },
    Underlay {
        start: usize,
        end: usize,
    },
    Data {
        start: usize,
        end: usize,
    },
    Above {
        next: usize,
    },
    Overlay {
        next: usize,
    },
    Done,
}

/// The passes of a document in writing order, computed as they are asked
/// for: panel groups from the probe, then the `Above` passes with the
/// annotation overlay. The document and its panels are passed to every
/// [`Self::next`], so nothing is borrowed between passes.
pub(crate) struct Passes {
    stage: Stage,
}

impl Passes {
    pub(crate) fn new() -> Self {
        Self {
            stage: Stage::Start,
        }
    }

    /// The next pass of `document`, whose expanded panels are `panels`.
    pub(crate) fn next(&mut self, document: &Config, panels: &[Config]) -> Option<Pass> {
        let n = panels.len();
        let annotations = document.annotations.visible;
        loop {
            match self.stage {
                Stage::Start => {
                    if n == 0 {
                        self.stage = Stage::Done;
                        continue;
                    }
                    self.stage = Stage::Below {
                        start: 0,
                        end: group_end(document, panels, 0),
                        next: 0,
                    };
                }
                Stage::Below { start, end, next } => {
                    if next < end {
                        self.stage = Stage::Below {
                            start,
                            end,
                            next: next + 1,
                        };
                        if below_places(document, panels, next) {
                            return Some(Pass::Below(next));
                        }
                        continue;
                    }
                    self.stage = if start == 0 && annotations {
                        Stage::Underlay { start, end }
                    } else {
                        Stage::Data { start, end }
                    };
                }
                Stage::Underlay { start, end } => {
                    self.stage = Stage::Data { start, end };
                    return Some(Pass::Underlay);
                }
                Stage::Data { start, end } => {
                    self.stage = if end < n {
                        Stage::Below {
                            start: end,
                            end: group_end(document, panels, end),
                            next: end,
                        }
                    } else {
                        Stage::Above { next: 0 }
                    };
                    return Some(Pass::Data(start..end));
                }
                Stage::Above { next } => {
                    if next >= n {
                        self.stage = Stage::Done;
                        continue;
                    }
                    self.stage = if annotations && next == overlay_panel(panels) {
                        Stage::Overlay { next: next + 1 }
                    } else {
                        Stage::Above { next: next + 1 }
                    };
                    return Some(Pass::Above(next));
                }
                Stage::Overlay { next } => {
                    self.stage = Stage::Above { next };
                    return Some(Pass::Overlay);
                }
                Stage::Done => return None,
            }
        }
    }
}

/// The annotation layer of `document` as the raster draws it
/// (`frame::annotation_raster_for_chart`).
fn annotation_request(document: &Config) -> RasterRequest<'_> {
    let area = document.chart_area.0;
    RasterRequest {
        layer: &document.annotations,
        space: pixel_space_for_config(document),
        origin: [area.x as f32, area.y as f32],
        size: (area.width.max(1), area.height.max(1)),
        scale: 1.0,
        selected: None,
        policy: FontPolicy::for_style(&document.draw_style),
    }
}

/// Run every non-data pass's emit into `sink`: the chrome of a panel for
/// its slot, or the annotation layer's slot. What the writer writes for the
/// pass, part for part; the Milkyway backdrop is not a part.
pub(crate) fn emit_pass(document: &Config, panels: &[Config], pass: &Pass, sink: &mut dyn Sink) {
    match *pass {
        Pass::Below(i) | Pass::Above(i) => {
            let panel = &panels[i];
            let slot = if matches!(pass, Pass::Below(_)) {
                Slot::Below
            } else {
                Slot::Above
            };
            if slot == Slot::Below && matches!(panel.draw_style, DrawStyle::Milkyway(_)) {
                return;
            }
            emit_chart_chrome(
                panel,
                &crate::text_render::CpuTextMeasure::for_style(&panel.draw_style),
                &mut SlotFilter { slot, inner: sink },
            );
        }
        Pass::Underlay | Pass::Overlay => {
            let slot = if *pass == Pass::Underlay {
                Slot::Below
            } else {
                Slot::Above
            };
            emit_annotation_layer(
                &annotation_request(document),
                &mut SlotFilter { slot, inner: sink },
                &mut |_| {},
            );
        }
        Pass::Data(_) => {}
    }
}

/// The rectangle a pass's raster covers in the PNG, which the writer clips
/// the pass to: the panel, or the parent's area for the annotation layer.
pub(crate) fn pass_clip(document: &Config, panels: &[Config], pass: &Pass) -> Rect {
    match *pass {
        Pass::Below(i) | Pass::Above(i) => panels[i].chart_area.0,
        _ => document.chart_area.0,
    }
}

/// Write the non-data pass `pass` of `document` into `out`, clipped to the
/// raster it stands in for.
fn write_pass(
    out: &mut dyn VectorOut,
    document: &Config,
    panels: &[Config],
    pass: &Pass,
) -> Result<()> {
    let clip = pass_clip(document, panels, pass);
    out.push_clip(clip);
    let result = (|| {
        let style = match *pass {
            Pass::Below(i) | Pass::Above(i) => &panels[i].draw_style,
            _ => &document.draw_style,
        };
        if let (Pass::Below(_), DrawStyle::Milkyway(c)) = (pass, style) {
            // The deep-space backdrop, painted by the raster's own function
            // into the panel's pixels.
            let Some(mut canvas) = Canvas::new(clip.width.max(1), clip.height.max(1)) else {
                return Ok(());
            };
            draw_space_background(&mut canvas, c.seed, c.nebula, c.dust);
            return out.image([clip.x as i32, clip.y as i32], &canvas_image(canvas));
        }
        // Annotations stroke precisely in every style, as their raster does.
        let stroker = match pass {
            Pass::Underlay | Pass::Overlay => DecoStroker::Precise,
            _ => DecoStroker::from_style(style),
        };
        let mut sink = VectorSink::new(
            out,
            stroker,
            FontPolicy::for_style(style),
            [0.0, 0.0],
            1.0,
            clip,
        );
        emit_pass(document, panels, pass, &mut sink);
        sink.error.map_or(Ok(()), Err)
    })();
    out.pop_clip();
    result
}

// The DataLess series of a data pass (#234).

/// The DataLess series of a document's panels: the series the data slot
/// draws as a CPU layer between its GPU steps (`renderer::DataSteps`). The
/// vector export writes them as vector parts from the same emit as their
/// raster, between images of the GPU steps around them, so the file stacks
/// as the PNG does: for each panel of a group its fills, bodies and picked
/// highlights, its DataLess series, then its value labels.
///
/// The steps of a group are numbered in drawing order, three per panel:
/// `3 * panel` the GPU data, `3 * panel + 1` the CPU layer and
/// `3 * panel + 2` the value labels. A panel that draws a DataLess series
/// cuts the group there; each [`DataRun`] between two cuts is one image.
#[derive(Clone, Copy)]
pub(crate) struct DataLess<'a> {
    document: &'a Config,
    panels: &'a [Config],
    series: &'a [SeriesConfig],
}

/// A run of a panel group's data steps between two CPU steps, as step
/// numbers (see [`DataLess`]), both bounds excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DataRun {
    after: i64,
    before: i64,
}

impl DataRun {
    /// Every GPU step of a group.
    const WHOLE: Self = Self {
        after: -1,
        before: i64::MAX,
    };

    fn holds(&self, step: i64) -> bool {
        self.after < step && step < self.before
    }

    /// The steps panel `panel` draws in this run. The CPU step is never
    /// drawn: it is written as vector parts.
    pub(crate) fn steps(&self, panel: usize) -> DataSteps {
        let first = 3 * panel as i64;
        DataSteps {
            gpu: self.holds(first),
            cpu: false,
            labels: self.holds(first + 2),
        }
    }
}

impl<'a> DataLess<'a> {
    pub(crate) fn new(
        document: &'a Config,
        panels: &'a [Config],
        series: &'a [SeriesConfig],
    ) -> Self {
        Self {
            document,
            panels,
            series,
        }
    }

    /// The series panel `panel` draws, in series order.
    fn of_panel(&self, panel: usize) -> impl Iterator<Item = &'a SeriesConfig> + 'a {
        let (document, count) = (self.document, self.panels.len());
        self.series.iter().filter(move |cfg| {
            crate::renderer::series_panel_index(document, count, &cfg.series_id) == panel
        })
    }

    /// Whether panel `panel` draws a DataLess series: a CPU step.
    fn cuts(&self, panel: usize) -> bool {
        self.of_panel(panel).any(SeriesConfig::is_dataless)
    }

    /// Whether panel `panel` draws a series on the GPU.
    fn paints(&self, panel: usize) -> bool {
        self.of_panel(panel).any(|cfg| !cfg.is_dataless())
    }

    /// Whether a panel of `range` draws a DataLess series.
    fn cuts_any(&self, range: &Range<usize>) -> bool {
        range.clone().any(|panel| self.cuts(panel))
    }

    /// The first panel of `range` whose CPU step comes after step `after`.
    fn next_cut(&self, range: &Range<usize>, after: i64) -> Option<usize> {
        range
            .clone()
            .find(|&panel| 3 * panel as i64 + 1 > after && self.cuts(panel))
    }

    /// Whether `run` of the group `range` draws anything on the GPU: a run
    /// with no GPU series in its steps writes no image.
    fn run_paints(&self, range: &Range<usize>, run: DataRun) -> bool {
        range.clone().any(|panel| {
            let steps = run.steps(panel);
            (steps.gpu || steps.labels) && self.paints(panel)
        })
    }

    /// Emit the DataLess series of panel `panel` (placed at `scale` export
    /// pixels per logical pixel) into the sink `sink` makes for each of the
    /// panel's data pieces, as the frame blits their rasters: piece by
    /// piece, each piece's series in series order.
    fn emit_panel(
        &self,
        panel: usize,
        pieces: &[Rect],
        scale: f32,
        mut each_piece: impl FnMut(Rect, &mut dyn FnMut(&mut dyn Sink)) -> Result<()>,
    ) -> Result<()> {
        let config = &self.panels[panel];
        for &piece in pieces {
            each_piece(piece, &mut |sink: &mut dyn Sink| {
                for cfg in self.of_panel(panel) {
                    crate::renderer::emit_dataless(
                        config,
                        cfg,
                        |levels| crate::renderer::energy_level_records(levels, self.series),
                        scale,
                        sink,
                    );
                }
            })?;
        }
        Ok(())
    }
}

/// One image of a data pass: the panels `range` painted with the steps of
/// `run`, cropped to `area`.
pub(crate) struct DataImage {
    pub(crate) range: Range<usize>,
    pub(crate) area: Rect,
    run: DataRun,
}

impl DataImage {
    /// The steps the panel at `index` of `range` paints.
    pub(crate) fn steps(&self, index: usize) -> DataSteps {
        self.run.steps(self.range.start + index)
    }
}

/// The group a data pass is writing: its panels, its image area and the
/// step its next run starts after.
struct GroupRuns {
    range: Range<usize>,
    area: Option<Rect>,
    after: i64,
}

/// Writes a document's passes in order. A data pass is handed back to the
/// driver, which paints that panel group on the GPU and waits for it
/// ([`write_document`], or its `_blocking` twin); every other pass is
/// written here. A group with DataLess series is handed back as one image
/// per run of GPU steps, with the series written between them (#234).
pub(crate) struct DocumentWriter {
    passes: Passes,
    group: Option<GroupRuns>,
    /// The panel whose DataLess series are written before the next image.
    cut: Option<usize>,
}

impl DocumentWriter {
    pub(crate) fn new() -> Self {
        Self {
            passes: Passes::new(),
            group: None,
            cut: None,
        }
    }

    /// Write passes into `out` up to the next data image, and return it;
    /// `None` once every pass is written.
    pub(crate) fn next_data(
        &mut self,
        frame: &ExportFrame,
        series: &[SeriesConfig],
        out: &mut dyn VectorOut,
    ) -> Result<Option<DataImage>> {
        let dataless = DataLess::new(frame.chart.config(), frame.panels(), series);
        loop {
            if let Some(panel) = self.cut.take() {
                write_dataless(out, frame, dataless, panel)?;
            }
            if let Some(group) = self.group.as_mut() {
                let after = group.after;
                let cut = dataless.next_cut(&group.range, after);
                let run = DataRun {
                    after,
                    before: cut.map_or(i64::MAX, |panel| 3 * panel as i64 + 1),
                };
                let (range, area) = (group.range.clone(), group.area);
                match cut {
                    Some(panel) => {
                        group.after = run.before;
                        self.cut = Some(panel);
                    }
                    None => self.group = None,
                }
                if let Some(area) = area
                    && dataless.run_paints(&range, run)
                {
                    return Ok(Some(DataImage { range, area, run }));
                }
                continue;
            }
            let Some(pass) = self.passes.next(frame.chart.config(), frame.panels()) else {
                return Ok(None);
            };
            match pass {
                Pass::Data(range) => {
                    let area = group_image_area(frame.panels(), range.clone(), frame.size);
                    if dataless.cuts_any(&range) {
                        self.group = Some(GroupRuns {
                            range,
                            area,
                            after: DataRun::WHOLE.after,
                        });
                    } else if let Some(area) = area {
                        return Ok(Some(DataImage {
                            range,
                            area,
                            run: DataRun::WHOLE,
                        }));
                    }
                }
                pass => write_pass(out, frame.chart.config(), frame.panels(), &pass)?,
            }
        }
    }

    /// Write a data pass's painted group image, cropped to its area.
    pub(crate) fn write_image(
        out: &mut dyn VectorOut,
        area: Rect,
        mut image: RasterImage,
    ) -> Result<()> {
        crop_in_place(&mut image, area);
        out.image([area.x as i32, area.y as i32], &image)
    }
}

/// Write the DataLess series of panel `panel` of `frame` as vector parts,
/// clipped to each of the panel's data pieces as their raster is.
fn write_dataless(
    out: &mut dyn VectorOut,
    frame: &ExportFrame,
    dataless: DataLess<'_>,
    panel: usize,
) -> Result<()> {
    let config = &frame.panels()[panel];
    let policy = FontPolicy::for_style(&config.draw_style);
    let pieces = frame.data_pieces(panel);
    dataless.emit_panel(panel, pieces, frame.scale, |piece, emit| {
        let Some(piece) = clamp_rect_to_target(piece, frame.size) else {
            return Ok(());
        };
        out.push_clip(piece);
        // The rasters are placed precisely in every style.
        let mut sink = VectorSink::new(
            out,
            DecoStroker::Precise,
            policy,
            [0.0, 0.0],
            frame.scale,
            config.chart_area.0,
        );
        emit(&mut sink);
        let error = sink.error.take();
        out.pop_clip();
        error.map_or(Ok(()), Err)
    })
}

/// The clear colour of a data group's image: transparent, so the vector
/// content under it shows.
const DATA_IMAGE_CLEAR: Color = Color::new(0.0, 0.0, 0.0, 0.0);

/// Write the document of `frame` into `out`: every pass in order, the data
/// images painted by the GPU one run of a panel group at a time, and the
/// DataLess series of `series` between them.
pub(crate) async fn write_document(
    renderer: &Renderer,
    frame: &mut ExportFrame,
    series: &[SeriesConfig],
    out: &mut dyn VectorOut,
) -> Result<()> {
    let mut writer = DocumentWriter::new();
    while let Some(data) = writer.next_data(frame, series, out)? {
        let image = renderer
            .paint_export(
                frame,
                data.range.clone(),
                false,
                DATA_IMAGE_CLEAR,
                &|index| data.steps(index),
            )
            .await?;
        DocumentWriter::write_image(out, data.area, image)?;
    }
    Ok(())
}

/// [`write_document`], polling the device on this thread for each data
/// image's readback.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_document_blocking(
    renderer: &Renderer,
    frame: &mut ExportFrame,
    series: &[SeriesConfig],
    out: &mut dyn VectorOut,
) -> Result<()> {
    let mut writer = DocumentWriter::new();
    while let Some(data) = writer.next_data(frame, series, out)? {
        let image = renderer.paint_export_blocking(
            frame,
            data.range.clone(),
            false,
            DATA_IMAGE_CLEAR,
            &|index| data.steps(index),
        )?;
        DocumentWriter::write_image(out, data.area, image)?;
    }
    Ok(())
}

/// Paint `clear` under everything, as the PNG export's clear colour.
pub(crate) fn write_background(out: &mut dyn VectorOut, size: (u32, u32), clear: Color) {
    if clear.a <= 0.0 {
        return;
    }
    out.begin_path(PathPaint::Fill(clear));
    rect_path(
        out,
        RectF {
            x: 0.0,
            y: 0.0,
            width: size.0 as f32,
            height: size.1 as f32,
        },
    );
    out.end_path();
}

/// A vector file format: its writer, made for a page of `size` physical
/// pixels shown at `page` points, and the file it finishes into.
pub(crate) trait VectorFile: VectorOut + Sized {
    type Output;
    fn create(size: (u32, u32), page: [f32; 2]) -> Self;
    fn finish_file(self) -> Result<Self::Output>;
}

impl VectorFile for SvgOut {
    type Output = String;
    fn create(size: (u32, u32), page: [f32; 2]) -> Self {
        SvgOut::new(size, page)
    }
    fn finish_file(self) -> Result<String> {
        Ok(self.finish())
    }
}

impl VectorFile for PdfOut {
    type Output = Vec<u8>;
    fn create(size: (u32, u32), page: [f32; 2]) -> Self {
        PdfOut::new(size, page)
    }
    fn finish_file(self) -> Result<Vec<u8>> {
        self.finish()
    }
}

/// The file of `frame`'s document over `clear`, in format `F`.
pub(crate) async fn document_file<F: VectorFile>(
    renderer: &Renderer,
    frame: &mut ExportFrame,
    series: &[SeriesConfig],
    clear: Color,
) -> Result<F::Output> {
    let mut out = F::create(frame.size, page_points(frame.logical));
    write_background(&mut out, frame.size, clear);
    write_document(renderer, frame, series, &mut out).await?;
    out.finish_file()
}

/// [`document_file`], polling the device on this thread.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn document_file_blocking<F: VectorFile>(
    renderer: &Renderer,
    frame: &mut ExportFrame,
    series: &[SeriesConfig],
    clear: Color,
) -> Result<F::Output> {
    let mut out = F::create(frame.size, page_points(frame.logical));
    write_background(&mut out, frame.size, clear);
    write_document_blocking(renderer, frame, series, &mut out)?;
    out.finish_file()
}

// Test support: what the writers are handed, part by part.

/// One part of a vector export, in writing order and output pixels: what
/// [`written_document_parts`] records from the same passes the writers run,
/// for the read-back tests of the SVG and PDF (`cpu-twin` only).
#[cfg(feature = "cpu-twin")]
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub enum WrittenPart {
    /// A stroke, a rectangle's fill or border, or a triangle, as
    /// [`crate::sink::Parts`] stores it (a stroke's width as emitted).
    Part(crate::sink::DrawnPart),
    /// The start of a text block: its origin and rotation.
    Text {
        origin: [f32; 2],
        rotation: Option<(f32, [f32; 2])>,
    },
    /// A glyph's pen, relative to the text origin.
    Glyph([f32; 2]),
    /// A rule, dash, band or underline of the text, relative to its origin.
    TextRect(RectF),
    /// The end of a text block.
    TextEnd,
    /// An image's pixels: a data image, a ramp or a backdrop.
    Image(Rect),
}

/// The recording sink of [`WrittenPart`]s: the parts a [`VectorSink`]
/// writes, in the same order and mapping, through [`crate::sink::Parts`].
#[cfg(feature = "cpu-twin")]
pub(crate) struct PartsTwin<'a> {
    out: &'a mut dyn FnMut(WrittenPart),
    policy: FontPolicy,
    origin: [f32; 2],
    scale: f32,
    clip: Rect,
    text_origin: [f32; 2],
}

#[cfg(feature = "cpu-twin")]
impl<'a> PartsTwin<'a> {
    pub(crate) fn new(
        out: &'a mut dyn FnMut(WrittenPart),
        policy: FontPolicy,
        origin: [f32; 2],
        scale: f32,
        clip: Rect,
    ) -> Self {
        Self {
            out,
            policy,
            origin,
            scale,
            clip,
            text_origin: [0.0, 0.0],
        }
    }

    fn map(&self, p: [f32; 2]) -> [f32; 2] {
        [
            (p[0] - self.origin[0]) * self.scale,
            (p[1] - self.origin[1]) * self.scale,
        ]
    }

    /// Record `emit`'s parts as `Parts` stores them, mapped to output pixels.
    fn parts(&mut self, emit: impl FnOnce(&mut dyn Sink)) {
        let (origin, scale) = (self.origin, self.scale);
        let map = |p: [f32; 2]| [(p[0] - origin[0]) * scale, (p[1] - origin[1]) * scale];
        let out = &mut *self.out;
        emit(&mut crate::sink::Parts(&mut |part| {
            use crate::sink::DrawnPart;
            out(WrittenPart::Part(match part {
                DrawnPart::Segment { a, b, width } => DrawnPart::Segment {
                    a: map(a),
                    b: map(b),
                    width: width * scale,
                },
                DrawnPart::Rect(r) | DrawnPart::Text(r) => {
                    let o = map([r.x, r.y]);
                    DrawnPart::Rect(RectF {
                        x: o[0],
                        y: o[1],
                        width: r.width * scale,
                        height: r.height * scale,
                    })
                }
                DrawnPart::Triangle(t) => DrawnPart::Triangle([map(t[0]), map(t[1]), map(t[2])]),
                DrawnPart::Polygon { points, width } => DrawnPart::Polygon {
                    points: points.iter().map(|p| map(*p)).collect(),
                    width: width * scale,
                },
                DrawnPart::Path { points, width } => DrawnPart::Path {
                    points: points.iter().map(|p| map(*p)).collect(),
                    width: width * scale,
                },
            }));
        }));
    }
}

#[cfg(feature = "cpu-twin")]
impl Sink for PartsTwin<'_> {
    fn text(&mut self, _slot: Slot, text: &PlacedText<'_>) {
        let origin = self.map([text.origin.0, text.origin.1]);
        let rotation = text
            .rotation
            .map(|(degrees, c)| (degrees, self.map([c.0, c.1])));
        self.text_origin = origin;
        (self.out)(WrittenPart::Text { origin, rotation });
        let (scale, policy, pen) = (self.scale, self.policy, (origin[0], origin[1]));
        match text.text {
            TextRef::Plain {
                text,
                family,
                size,
                color,
            } => place_plain_text(
                self,
                text,
                pen,
                color,
                family,
                size * scale,
                false,
                false,
                policy,
            ),
            TextRef::Rich(rich) if scale == 1.0 => place_rich_text(self, rich, pen, policy),
            TextRef::Rich(rich) => place_rich_text(
                self,
                &crate::decoration::scaled_rich_text(rich, scale),
                pen,
                policy,
            ),
        }
        (self.out)(WrittenPart::TextEnd);
    }

    fn stroke(&mut self, slot: Slot, a: [f32; 2], b: [f32; 2], style: &StrokeStyle<'_>) {
        if style.color.a > 0.0 {
            self.parts(|sink| sink.stroke(slot, a, b, style));
        }
    }

    fn rect(&mut self, slot: Slot, rect: RectF, fill: Option<Color>, border: Option<Border<'_>>) {
        let o = self.map([rect.x, rect.y]);
        let mapped = RectF {
            x: o[0],
            y: o[1],
            width: rect.width * self.scale,
            height: rect.height * self.scale,
        };
        if !has_area(mapped) {
            return;
        }
        if fill.is_some_and(|c| c.a > 0.0) {
            self.parts(|sink| sink.rect(slot, rect, fill, None));
        }
        if border.is_some() {
            self.parts(|sink| sink.rect(slot, rect, None, border));
        }
    }

    fn triangle(&mut self, slot: Slot, points: [[f32; 2]; 3], color: Color) {
        if color.a > 0.0 {
            self.parts(|sink| sink.triangle(slot, points, color));
        }
    }

    fn path(&mut self, slot: Slot, segments: &[PathSegment], style: &StrokeStyle<'_>) {
        if style.color.a <= 0.0 {
            return;
        }
        // One part per subpath, as a file reads back.
        let mut rest = segments;
        while let Some(start) = rest.iter().position(|s| matches!(s, PathSegment::Move(_))) {
            let after = &rest[start + 1..];
            let end = after
                .iter()
                .position(|s| matches!(s, PathSegment::Move(_)))
                .map_or(rest.len(), |k| start + 1 + k);
            let subpath = &rest[start..end];
            self.parts(|sink| sink.path(slot, subpath, style));
            rest = &rest[end..];
        }
    }

    fn polygon(
        &mut self,
        slot: Slot,
        points: &[[f32; 2]],
        fill: Option<Color>,
        border: Option<&StrokeStyle<'_>>,
    ) {
        if fill.is_some_and(|c| c.a > 0.0)
            || border.is_some_and(|b| b.color.a > 0.0 && b.width > 0.0)
        {
            self.parts(|sink| sink.polygon(slot, points, fill, border));
        }
    }

    fn ramp(&mut self, _slot: Slot, rect: RectF, _bar: &ColorBarOptions) {
        let o = self.map([rect.x, rect.y]);
        let r = RectF {
            x: o[0],
            y: o[1],
            width: rect.width * self.scale,
            height: rect.height * self.scale,
        };
        if let Some(area) = ramp_area(r, self.clip) {
            (self.out)(WrittenPart::Image(area));
        }
    }
}

#[cfg(feature = "cpu-twin")]
impl GlyphSink for PartsTwin<'_> {
    fn paints(&self) -> bool {
        true
    }

    fn glyph(&mut self, _: FontRef<'static>, _: u16, size: f32, pen: (f32, f32), _: &Color) {
        let pen = [pen.0 - self.text_origin[0], pen.1 - self.text_origin[1]];
        if pen[0].is_finite() && pen[1].is_finite() && size.is_finite() && size > 0.0 {
            (self.out)(WrittenPart::Glyph(pen));
        }
    }

    fn rect(&mut self, r: RectF, _: &Color) {
        let r = RectF {
            x: r.x - self.text_origin[0],
            y: r.y - self.text_origin[1],
            ..r
        };
        if has_area(r) {
            (self.out)(WrittenPart::TextRect(r));
        }
    }
}

/// Every part [`Renderer::export_svg`] and
/// [`Renderer::export_pdf`] write for `chart` and its `series` at `scale`
/// (the clear colour's background aside), in writing order: the parts of
/// the same passes over the same scaled, placed document, recorded through
/// [`crate::sink::Parts`] instead of written, with the DataLess series
/// between the data images of their group (#234). Test support
/// (`cpu-twin`).
#[cfg(feature = "cpu-twin")]
#[doc(hidden)]
pub fn written_document_parts(
    chart: &crate::chart::Chart,
    series: &[SeriesConfig],
    scale: f32,
    out: &mut dyn FnMut(WrittenPart),
) -> Result<()> {
    let scale = crate::renderer::clamp_export_scale(scale);
    let orig = chart.config().document_area();
    let size = (
        ((orig.width as f32) * scale).round().max(1.0) as u32,
        ((orig.height as f32) * scale).round().max(1.0) as u32,
    );
    let target = Rect {
        x: 0,
        y: 0,
        width: size.0,
        height: size.1,
    };
    let document = crate::renderer::place_document(chart.config(), scale, target);
    let panels = crate::renderer::expand_for_render(&document)?;
    let dataless = DataLess::new(&document, &panels, series);
    let mut passes = Passes::new();
    while let Some(pass) = passes.next(&document, &panels) {
        match &pass {
            Pass::Data(range) => {
                let area = group_image_area(&panels, range.clone(), size);
                if !dataless.cuts_any(range) {
                    if let Some(area) = area {
                        out(WrittenPart::Image(area));
                    }
                    continue;
                }
                let mut after = DataRun::WHOLE.after;
                loop {
                    let cut = dataless.next_cut(range, after);
                    let run = DataRun {
                        after,
                        before: cut.map_or(i64::MAX, |panel| 3 * panel as i64 + 1),
                    };
                    if let Some(area) = area
                        && dataless.run_paints(range, run)
                    {
                        out(WrittenPart::Image(area));
                    }
                    let Some(panel) = cut else {
                        break;
                    };
                    let config = &panels[panel];
                    let pieces = crate::renderer::DataPieces::of(
                        config,
                        config
                            .data_area()
                            .map_or(config.chart_area.0, |area| area.0),
                    );
                    let policy = FontPolicy::for_style(&config.draw_style);
                    dataless.emit_panel(panel, pieces.rects(), scale, |piece, emit| {
                        if clamp_rect_to_target(piece, size).is_some() {
                            let mut twin =
                                PartsTwin::new(out, policy, [0.0, 0.0], scale, config.chart_area.0);
                            emit(&mut twin);
                        }
                        Ok(())
                    })?;
                    after = run.before;
                }
            }
            Pass::Below(i) if matches!(panels[*i].draw_style, DrawStyle::Milkyway(_)) => {
                out(WrittenPart::Image(panels[*i].chart_area.0));
            }
            _ => {
                let style = match pass {
                    Pass::Below(i) | Pass::Above(i) => &panels[i].draw_style,
                    _ => &document.draw_style,
                };
                let clip = pass_clip(&document, &panels, &pass);
                let mut twin =
                    PartsTwin::new(out, FontPolicy::for_style(style), [0.0, 0.0], 1.0, clip);
                emit_pass(&document, &panels, &pass, &mut twin);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;

    /// A dashed cubic from (10, 40) to (70, 40), bulging up to y 10.
    const ARC: [PathSegment; 2] = [
        PathSegment::Move((10.0, 40.0)),
        PathSegment::Cubic((10.0, 0.0), (70.0, 0.0), (70.0, 40.0)),
    ];

    fn arc_style() -> StrokeStyle<'static> {
        StrokeStyle {
            width: 1.5,
            color: Color::from_rgb8(255, 0, 0),
            dash: &[4.0, 2.0],
            tag: "arc",
        }
    }

    // A stroked path (#234) is one SVG path at the sink's scale: its width
    // and dash scaled, its cubic kept a cubic, so the dash runs along the
    // whole curve.
    #[test]
    fn a_path_is_written_as_one_scaled_dashed_curve() {
        let mut out = SvgOut::new((160, 100), page_points((80, 50)));
        let area = Rect {
            x: 0,
            y: 0,
            width: 160,
            height: 100,
        };
        let mut sink = VectorSink::new(
            &mut out,
            DecoStroker::Precise,
            FontPolicy::default(),
            [0.0, 0.0],
            2.0,
            area,
        );
        sink.path(Slot::Above, &ARC, &arc_style());
        assert!(sink.error.is_none());
        let svg = out.finish();
        let path = svg
            .lines()
            .find(|line| line.contains("stroke=\"#ff0000\""))
            .expect("the path");
        assert!(path.contains("stroke-width=\"3\""), "{path}");
        assert!(path.contains("stroke-dasharray=\"8 4\""), "{path}");
        assert!(path.contains("M20 80C20 0 140 0 140 80"), "{path}");
        assert_eq!(svg.matches("stroke=\"#ff0000\"").count(), 1, "one path");
    }

    // The canvas strokes the same curve: ink where the cubic passes at its
    // middle (40, 10), none at the chord's middle (40, 40) under it.
    #[test]
    fn a_path_is_stroked_on_the_canvas_along_its_curve() {
        let mut canvas = Canvas::new(80, 50).unwrap();
        let mut sink = crate::decoration_render::CanvasSink::new(
            None,
            Some(&mut canvas),
            DecoStroker::Precise,
            FontPolicy::default(),
        );
        let solid = StrokeStyle {
            dash: &[],
            width: 3.0,
            ..arc_style()
        };
        sink.path(Slot::Above, &ARC, &solid);
        // A slot without a canvas paints nothing.
        sink.path(Slot::Below, &ARC, &solid);
        sink.finish().unwrap();
        let rgba = canvas.into_rgba();
        let alpha = |x: usize, y: usize| rgba[(y * 80 + x) * 4 + 3];
        assert!(alpha(40, 10) > 200, "ink on the curve");
        assert_eq!(alpha(40, 40), 0, "no ink on the chord");
    }

    // The geometry sinks bound a path by every point it names: the Bézier
    // lies in its control points' hull.
    #[test]
    fn a_path_part_is_bounded_by_its_control_points() {
        let mut parts = Vec::new();
        crate::sink::Parts(&mut |part| parts.push(part)).path(Slot::Above, &ARC, &arc_style());
        let [part] = parts.as_slice() else {
            panic!("one part: {parts:?}")
        };
        let extent = part.extent();
        assert_eq!(
            (extent.x, extent.y, extent.width, extent.height),
            (9.25, -0.75, 61.5, 41.5)
        );
    }

    /// The non-data passes of `config` written as an SVG document.
    fn chrome_svg(config: &Config) -> String {
        let area = config.chart_area.0;
        let size = (area.x + area.width, area.y + area.height);
        let mut out = SvgOut::new(size, page_points(size));
        let panels = std::slice::from_ref(config);
        let mut passes = Passes::new();
        while let Some(pass) = passes.next(config, panels) {
            if !matches!(pass, Pass::Data(_)) {
                write_pass(&mut out, config, panels, &pass).unwrap();
            }
        }
        out.finish()
    }

    // The default chart's chrome writes a document whose groups balance,
    // with its glyphs defined once and used where the text is placed.
    #[test]
    fn the_chrome_passes_write_a_balanced_document() {
        let svg = chrome_svg(&default_config());
        assert!(svg.starts_with("<?xml"));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert_eq!(svg.matches("<g").count(), svg.matches("</g>").count());
        assert!(svg.contains("<path"));
        let uses = svg.matches("<use ").count();
        let defs = svg.matches("<path id=\"g").count();
        assert!(
            uses > 0 && defs > 0 && defs <= uses,
            "{defs} defs, {uses} uses"
        );
    }

    // The default chart's chrome as a PDF: the header, every object at the
    // offset its cross-reference entry names, and the trailer's root.
    #[test]
    fn the_chrome_passes_write_a_well_formed_pdf() {
        let config = default_config();
        let area = config.chart_area.0;
        let size = (area.x + area.width, area.y + area.height);
        let mut out = PdfOut::new(size, page_points(size));
        let panels = std::slice::from_ref(&config);
        let mut passes = Passes::new();
        while let Some(pass) = passes.next(&config, panels) {
            if !matches!(pass, Pass::Data(_)) {
                write_pass(&mut out, &config, panels, &pass).unwrap();
            }
        }
        let pdf = out.finish().unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4\n"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let find = |needle: &[u8], from: usize| {
            pdf[from..]
                .windows(needle.len())
                .position(|w| w == needle)
                .map(|i| i + from)
        };
        let tail = find(b"startxref\n", 0).unwrap() + 10;
        let digits = pdf[tail..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let xref: usize = std::str::from_utf8(&pdf[tail..tail + digits])
            .unwrap()
            .parse()
            .unwrap();
        let table = std::str::from_utf8(&pdf[xref..tail - 10]).unwrap();
        assert!(table.starts_with("xref\n0 "));
        let offsets: Vec<usize> = table
            .lines()
            .skip(3)
            .take_while(|l| l.ends_with(" n "))
            .map(|l| l[..10].parse().unwrap())
            .collect();
        assert!(!offsets.is_empty());
        for (i, offset) in offsets.iter().enumerate() {
            let head = format!("{} 0 obj\n", i + 1);
            assert!(
                pdf[*offset..].starts_with(head.as_bytes()),
                "object {}",
                i + 1
            );
        }
        let text = String::from_utf8_lossy(&pdf[xref..]);
        assert!(text.contains("/Root"));
        assert!(find(b"/Type /Catalog", 0).is_some());
    }

    // Every pass of a single panel comes once, in compositor order.
    #[test]
    fn a_single_panel_document_has_one_data_image_between_its_slots() {
        let config = default_config();
        let panels = std::slice::from_ref(&config);
        let mut passes = Passes::new();
        let mut order = Vec::new();
        while let Some(pass) = passes.next(&config, panels) {
            order.push(pass);
        }
        let data = order
            .iter()
            .position(|p| matches!(p, Pass::Data(_)))
            .expect("one data pass");
        assert_eq!(order[data], Pass::Data(0..1));
        assert!(
            order[..data]
                .iter()
                .all(|p| matches!(p, Pass::Below(0) | Pass::Underlay))
        );
        assert!(
            order[data + 1..]
                .iter()
                .all(|p| matches!(p, Pass::Above(0) | Pass::Overlay))
        );
        assert!(order.contains(&Pass::Above(0)));
    }

    /// Documents for the pass tests: one panel with annotations shown and
    /// hidden, the Milkyway style, and a layered document (two parent
    /// overlays, 50 line offset members and a colourbar on the parent, three
    /// subplots with two overlays each, the last with 5 members of its own),
    /// in the precise and the Milkyway style.
    fn pass_documents() -> Vec<Config> {
        use crate::data_config::LineOffsets;
        use crate::{OverlayLayer, Subplot};
        let milkyway = DrawStyle::Milkyway(Default::default());
        let area = |x: u32, y: u32, width: u32, height: u32| {
            crate::layout::ChartArea(Rect {
                x,
                y,
                width,
                height,
            })
        };
        let overlay = |id: String| {
            let mut overlay = OverlayLayer::default();
            overlay.y_axis.min = 0.0;
            overlay.y_axis.max = 1.0;
            overlay.y_axis.out_margin = 40.0;
            overlay.series = vec![id];
            overlay
        };
        let members = |prefix: &str, n: usize| LineOffsets {
            series: (0..n).map(|i| format!("{prefix}{i}")).collect(),
            step_y: 1.0,
            step_x: 0.0,
        };
        let single = default_config();
        let mut hidden = default_config();
        hidden.annotations.visible = false;
        let mut sky = default_config();
        sky.draw_style = milkyway;
        let mut layered = default_config();
        layered.chart_area = area(0, 0, 640, 360);
        layered.overlays = (0..2).map(|k| overlay(format!("po{k}"))).collect();
        layered.line_offsets = Some(members("m", 50));
        layered.colorbar = Some(crate::default::default_colorbar_options());
        layered.subplots = (0..3)
            .map(|i| {
                let mut subplot = Subplot::default();
                subplot.config.chart_area = area(220 * i, 360, 220, 220);
                subplot.config.overlays = (0..2).map(|n| overlay(format!("s{i}o{n}"))).collect();
                subplot.series = vec![format!("s{i}")];
                if i == 2 {
                    let offsets = members("k", 5);
                    subplot.series.extend(offsets.series.iter().cloned());
                    subplot.config.line_offsets = Some(offsets);
                }
                subplot
            })
            .collect();
        let mut layered_sky = layered.clone();
        layered_sky.draw_style = milkyway;
        layered_sky.annotations.visible = false;
        for subplot in &mut layered_sky.subplots {
            subplot.config.draw_style = milkyway;
        }
        vec![single, hidden, sky, layered, layered_sky]
    }

    /// PASS_INDEX_IN_PANELS and RASTER_IMAGE_RGBA_LEN (#152): over the
    /// expanded panels of every pass document at 1x and 2x (and over their
    /// first panel, their first half and no panels), `Passes` names panels
    /// below `panels.len()` only, its data ranges partition the panels in
    /// order, and every panel has one `Above` pass. Each non-data pass
    /// emits, clips and writes as SVG and PDF; each data pass's image area
    /// lies in the target, and an image of the target's size cut to it by
    /// `crop_in_place` holds `width * height * 4` bytes and writes. The
    /// written parts read back for the same documents.
    #[test]
    fn every_pass_indexes_its_panels_and_the_data_passes_partition_them() {
        for (w, h) in [(1, 1), (3, 5), (2049, 3)] {
            let image = canvas_image(Canvas::new(w, h).unwrap());
            assert_eq!((image.width, image.height), (w, h));
            assert_eq!(image.rgba.len(), (w * h * 4) as usize);
        }
        for document in pass_documents() {
            for scale in [1.0f32, 2.0] {
                let orig = document.document_area();
                let size = (
                    ((orig.width as f32) * scale).round().max(1.0) as u32,
                    ((orig.height as f32) * scale).round().max(1.0) as u32,
                );
                let target = Rect {
                    x: 0,
                    y: 0,
                    width: size.0,
                    height: size.1,
                };
                let placed = crate::renderer::place_document(&document, scale, target);
                let panels = crate::renderer::expand_for_render(&placed).unwrap();
                let expected = 1
                    + placed.overlays.len()
                    + placed.line_offsets.as_ref().map_or(0, |o| o.series.len())
                    + placed
                        .subplots
                        .iter()
                        .map(|s| {
                            1 + s.config.overlays.len()
                                + s.config.line_offsets.as_ref().map_or(0, |o| o.series.len())
                        })
                        .sum::<usize>();
                assert_eq!(panels.len(), expected);
                let slices: [&[Config]; 4] =
                    [&panels, &panels[..1], &panels[..panels.len() / 2], &[]];
                for slice in slices {
                    let n = slice.len();
                    let mut svg = SvgOut::new(size, page_points(size));
                    let mut pdf = PdfOut::new(size, page_points(size));
                    let mut passes = Passes::new();
                    let (mut data_end, mut above) = (0, 0);
                    while let Some(pass) = passes.next(&placed, slice) {
                        match &pass {
                            Pass::Below(i) => assert!(*i < n, "{pass:?} of {n}"),
                            Pass::Above(i) => {
                                assert_eq!(*i, above, "{pass:?} of {n}");
                                above += 1;
                            }
                            Pass::Data(range) => {
                                assert_eq!(range.start, data_end, "{pass:?} of {n}");
                                assert!(range.start < range.end && range.end <= n);
                                data_end = range.end;
                                let Some(area) = group_image_area(slice, range.clone(), size)
                                else {
                                    continue;
                                };
                                assert!(area.width > 0 && area.height > 0);
                                assert!(area.x + area.width <= size.0);
                                assert!(area.y + area.height <= size.1);
                                let mut image = RasterImage {
                                    width: size.0,
                                    height: size.1,
                                    rgba: vec![96; (size.0 * size.1 * 4) as usize],
                                };
                                crop_in_place(&mut image, area);
                                assert_eq!((image.width, image.height), (area.width, area.height));
                                assert_eq!(
                                    image.rgba.len(),
                                    (area.width * area.height * 4) as usize
                                );
                                let origin = [area.x as i32, area.y as i32];
                                svg.image(origin, &image).unwrap();
                                pdf.image(origin, &image).unwrap();
                                continue;
                            }
                            Pass::Underlay | Pass::Overlay => assert!(n > 0),
                        }
                        let clip = pass_clip(&placed, slice, &pass);
                        assert!(clip.width > 0 && clip.height > 0, "{pass:?}");
                        emit_pass(&placed, slice, &pass, &mut PaintCount::default());
                        write_pass(&mut svg, &placed, slice, &pass).unwrap();
                        write_pass(&mut pdf, &placed, slice, &pass).unwrap();
                    }
                    assert_eq!(data_end, n);
                    assert_eq!(above, n);
                    assert!(passes.next(&placed, slice).is_none());
                    assert!(svg.finish().ends_with("</svg>\n"));
                    pdf.finish().unwrap();
                }
                #[cfg(feature = "cpu-twin")]
                {
                    let chart = crate::chart::Chart::new(document.clone());
                    let mut parts = 0usize;
                    written_document_parts(&chart, &[], scale, &mut |_| parts += 1).unwrap();
                    assert!(parts > 0);
                }
            }
        }
    }

    /// PASS_INDEX_IN_PANELS and RASTER_IMAGE_RGBA_LEN (#152), on the GPU:
    /// the export frame's panels are the slice every pass of the document
    /// loop indexes, and painting a data group does not change them; each
    /// painted image is the target's size with `width * height * 4` bytes,
    /// and cut to its group's area it keeps that relation. Every pass
    /// document at 1x and 2x, then the SVG and PDF files of the same
    /// frames.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_export_frame_keeps_its_panels_through_the_document_loop() {
        let (device, queue) = crate::data_render::shared_device().unwrap();
        let mut renderer = Renderer::try_new(
            crate::RendererDevice::new(device, queue).expect("figgy poll thread"),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        let clear = Color::new(0.0, 0.0, 0.0, 0.0);
        for document in pass_documents() {
            let chart = crate::chart::Chart::new(document);
            for scale in [1.0f32, 2.0] {
                let mut frame = renderer
                    .prepare_export(&chart, &[], scale, crate::renderer::ExportScope::Document)
                    .unwrap();
                let n = frame.panels().len();
                let size = frame.size;
                let mut passes = Passes::new();
                let mut painted = 0;
                while let Some(pass) = passes.next(frame.chart.config(), frame.panels()) {
                    let Pass::Data(range) = pass else {
                        assert_eq!(frame.panels().len(), n);
                        continue;
                    };
                    assert!(range.end <= n);
                    let area = group_image_area(frame.panels(), range.clone(), size);
                    let mut image = crate::test_gpu::block_on(renderer.paint_export(
                        &mut frame,
                        range,
                        false,
                        clear,
                        &|_| DataSteps::ALL,
                    ))
                    .unwrap();
                    assert_eq!(frame.panels().len(), n, "a paint changed the panels");
                    assert_eq!((image.width, image.height), size);
                    assert_eq!(image.rgba.len(), (size.0 * size.1 * 4) as usize);
                    if let Some(area) = area {
                        crop_in_place(&mut image, area);
                        assert_eq!(image.rgba.len(), (area.width * area.height * 4) as usize);
                    }
                    painted += 1;
                }
                assert!(painted > 0);
                drop(frame);
                renderer.end_gpu_frame();
                let mut frame = renderer
                    .prepare_export(&chart, &[], scale, crate::renderer::ExportScope::Document)
                    .unwrap();
                let svg = crate::test_gpu::block_on(document_file::<SvgOut>(
                    &renderer,
                    &mut frame,
                    &[],
                    clear,
                ));
                assert!(svg.unwrap().ends_with("</svg>\n"));
                assert_eq!(frame.panels().len(), n);
                let pdf = crate::test_gpu::block_on(document_file::<PdfOut>(
                    &renderer,
                    &mut frame,
                    &[],
                    clear,
                ));
                assert!(pdf.unwrap().ends_with(b"%%EOF\n"));
                assert_eq!(frame.panels().len(), n);
                drop(frame);
                renderer.end_gpu_frame();
            }
        }
    }
}
