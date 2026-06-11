//! CPU raster canvas — a thin shim over `tiny-skia` exposing the small
//! drawing subset the axis/text renderers need (lines with dash patterns,
//! rects, circles, glyph alpha-mask blits, and a save/translate/rotate
//! transform stack mirroring the skia canvas model this code was written
//! against).
//!
//! Output is premultiplied RGBA8 — the same contract the wgpu texture upload
//! path always consumed. Pure Rust; compiles on every target including
//! wasm32-unknown-unknown.

use tiny_skia::{
    FilterQuality, Pixmap, PixmapPaint, PathBuilder, PremultipliedColorU8, Stroke, StrokeDash,
    Transform,
};

use crate::color::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaintStyle {
    Fill,
    Stroke,
}

/// Solid-color paint: fill or stroke (+ optional dash pattern).
#[derive(Debug, Clone)]
pub struct Paint {
    color: tiny_skia::Color,
    style: PaintStyle,
    stroke_width: f32,
    dash: Option<Vec<f32>>,
}

fn ts_color(c: &Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(
        c.r.clamp(0.0, 1.0),
        c.g.clamp(0.0, 1.0),
        c.b.clamp(0.0, 1.0),
        c.a.clamp(0.0, 1.0),
    )
    .unwrap_or(tiny_skia::Color::BLACK)
}

impl Paint {
    pub fn fill(color: &Color) -> Self {
        Self { color: ts_color(color), style: PaintStyle::Fill, stroke_width: 1.0, dash: None }
    }

    pub fn stroke(color: &Color, width: f32) -> Self {
        Self {
            color: ts_color(color),
            style: PaintStyle::Stroke,
            stroke_width: width.max(1.0),
            dash: None,
        }
    }

    /// Attach a dash pattern (`[on, off, …]`, even length). Empty = solid.
    pub fn with_dash(mut self, pattern: &[f32]) -> Self {
        self.dash = if pattern.is_empty() { None } else { Some(pattern.to_vec()) };
        self
    }

    fn shader_paint(&self) -> tiny_skia::Paint<'static> {
        let mut p = tiny_skia::Paint::default();
        p.set_color(self.color);
        p.anti_alias = true;
        p
    }

    fn stroke_params(&self) -> Stroke {
        Stroke {
            width: self.stroke_width,
            dash: self.dash.as_ref().and_then(|d| StrokeDash::new(d.clone(), 0.0)),
            ..Stroke::default()
        }
    }
}

/// Raster target with a skia-style transform stack. Starts fully transparent.
pub struct Canvas {
    pix: Pixmap,
    ts: Transform,
    stack: Vec<Transform>,
}

impl Canvas {
    /// `None` when either dimension is zero.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        Some(Self {
            pix: Pixmap::new(width, height)?,
            ts: Transform::identity(),
            stack: Vec::new(),
        })
    }

    /// Consume into premultiplied RGBA8 bytes (`width * height * 4`).
    pub fn into_rgba(self) -> Vec<u8> {
        self.pix.take()
    }

    // Transform stack — same accumulation semantics as a skia canvas: a
    // transform pushed later applies to drawn coordinates first.

    pub fn save(&mut self) {
        self.stack.push(self.ts);
    }

    pub fn restore(&mut self) {
        if let Some(ts) = self.stack.pop() {
            self.ts = ts;
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.ts = self.ts.pre_concat(Transform::from_translate(dx, dy));
    }

    pub fn rotate_at(&mut self, degrees: f32, cx: f32, cy: f32) {
        self.ts = self.ts.pre_concat(Transform::from_rotate_at(degrees, cx, cy));
    }

    // Primitives.

    pub fn draw_line(&mut self, p0: (f32, f32), p1: (f32, f32), paint: &Paint) {
        let mut pb = PathBuilder::new();
        pb.move_to(p0.0, p0.1);
        pb.line_to(p1.0, p1.1);
        let Some(path) = pb.finish() else { return };
        self.pix.stroke_path(
            &path,
            &paint.shader_paint(),
            &paint.stroke_params(),
            self.ts,
            None,
        );
    }

    pub fn draw_rect(&mut self, x: f32, y: f32, w: f32, h: f32, paint: &Paint) {
        match paint.style {
            PaintStyle::Fill => {
                let Some(rect) = tiny_skia::Rect::from_xywh(x, y, w, h) else { return };
                self.pix.fill_rect(rect, &paint.shader_paint(), self.ts, None);
            }
            PaintStyle::Stroke => {
                let Some(rect) = tiny_skia::Rect::from_xywh(x, y, w, h) else { return };
                let path = PathBuilder::from_rect(rect);
                self.pix.stroke_path(
                    &path,
                    &paint.shader_paint(),
                    &paint.stroke_params(),
                    self.ts,
                    None,
                );
            }
        }
    }

    pub fn draw_circle(&mut self, cx: f32, cy: f32, r: f32, paint: &Paint) {
        let Some(path) = PathBuilder::from_circle(cx, cy, r) else { return };
        match paint.style {
            PaintStyle::Fill => {
                self.pix.fill_path(
                    &path,
                    &paint.shader_paint(),
                    tiny_skia::FillRule::Winding,
                    self.ts,
                    None,
                );
            }
            PaintStyle::Stroke => {
                self.pix.stroke_path(
                    &path,
                    &paint.shader_paint(),
                    &paint.stroke_params(),
                    self.ts,
                    None,
                );
            }
        }
    }

    /// The current transform's translation, or `None` when it rotates/scales.
    /// Text uses this to pick the resample-free integer blit path.
    pub fn translation(&self) -> Option<(f32, f32)> {
        let t = self.ts;
        if t.sx == 1.0 && t.sy == 1.0 && t.kx == 0.0 && t.ky == 0.0 {
            Some((t.tx, t.ty))
        } else {
            None
        }
    }

    /// Direct integer-position src-over blit of a glyph alpha mask tinted
    /// with `color`. No filtering — the mask must already be rasterized at
    /// the right subpixel phase (swash render offset). This is the crisp
    /// text path; AA happens exactly once, in the glyph rasterizer.
    pub fn blit_mask(&mut self, ix: i32, iy: i32, w: u32, h: u32, alpha: &[u8], color: &Color) {
        if w == 0 || h == 0 || alpha.len() < (w as usize * h as usize) {
            return;
        }
        let c = ts_color(color);
        let (cr, cg, cb, ca) = (c.red(), c.green(), c.blue(), c.alpha());
        let dst_w = self.pix.width() as i32;
        let dst_h = self.pix.height() as i32;
        let px = self.pix.pixels_mut();

        for row in 0..h as i32 {
            let dy = iy + row;
            if dy < 0 || dy >= dst_h {
                continue;
            }
            for col in 0..w as i32 {
                let dx = ix + col;
                if dx < 0 || dx >= dst_w {
                    continue;
                }
                let a = alpha[(row * w as i32 + col) as usize];
                if a == 0 {
                    continue;
                }
                let af = (a as f32 / 255.0) * ca;
                let sa = (af * 255.0 + 0.5) as u16;
                let sr = ((cr * af * 255.0 + 0.5) as u16).min(sa);
                let sg = ((cg * af * 255.0 + 0.5) as u16).min(sa);
                let sb = ((cb * af * 255.0 + 0.5) as u16).min(sa);

                let di = (dy * dst_w + dx) as usize;
                let d = px[di];
                let inv = 255 - sa;
                let or = (sr + d.red() as u16 * inv / 255).min(255) as u8;
                let og = (sg + d.green() as u16 * inv / 255).min(255) as u8;
                let ob = (sb + d.blue() as u16 * inv / 255).min(255) as u8;
                let oa = (sa + d.alpha() as u16 * inv / 255).min(255) as u8;
                if let Some(p) =
                    PremultipliedColorU8::from_rgba(or.min(oa), og.min(oa), ob.min(oa), oa)
                {
                    px[di] = p;
                }
            }
        }
    }

    /// Transformed glyph blit — only for rotated text (axis titles), where
    /// resampling is unavoidable. Translation-only text must use
    /// [`Self::blit_mask`] instead.
    pub fn draw_mask(&mut self, x: f32, y: f32, w: u32, h: u32, alpha: &[u8], color: &Color) {
        if w == 0 || h == 0 || alpha.len() < (w as usize * h as usize) {
            return;
        }
        let Some(mut glyph) = Pixmap::new(w, h) else { return };
        let c = ts_color(color);
        let (cr, cg, cb, ca) = (c.red(), c.green(), c.blue(), c.alpha());
        let px = glyph.pixels_mut();
        for (i, &a) in alpha.iter().take(px.len()).enumerate() {
            if a == 0 {
                continue;
            }
            let af = (a as f32 / 255.0) * ca;
            let pa = (af * 255.0 + 0.5) as u8;
            let pr = ((cr * af * 255.0 + 0.5) as u8).min(pa);
            let pg = ((cg * af * 255.0 + 0.5) as u8).min(pa);
            let pb = ((cb * af * 255.0 + 0.5) as u8).min(pa);
            if let Some(p) = PremultipliedColorU8::from_rgba(pr, pg, pb, pa) {
                px[i] = p;
            }
        }

        let paint = PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        let full_ts = self.ts.pre_concat(Transform::from_translate(x, y));
        self.pix.draw_pixmap(0, 0, glyph.as_ref(), &paint, full_ts, None);
    }
}
