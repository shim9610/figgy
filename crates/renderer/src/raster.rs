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
    FilterQuality, PathBuilder, Pixmap, PixmapPaint, PremultipliedColorU8, Stroke, StrokeDash,
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
        Self {
            color: ts_color(color),
            style: PaintStyle::Fill,
            stroke_width: 1.0,
            dash: None,
        }
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
        self.dash = if pattern.is_empty() {
            None
        } else {
            Some(pattern.to_vec())
        };
        self
    }

    /// Raise the stroke width to at least `min` px (no-op for fills). The
    /// sketch stroker uses this: a wobbled 1 px hairline drifts across pixel
    /// rows and alternates between sharp and 50/50-blurred coverage, reading
    /// as a broken line — a hand-drawn pen stroke needs some body.
    pub fn with_min_stroke_width(mut self, min: f32) -> Self {
        if matches!(self.style, PaintStyle::Stroke) {
            self.stroke_width = self.stroke_width.max(min);
        }
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
            dash: self
                .dash
                .as_ref()
                .and_then(|d| StrokeDash::new(d.clone(), 0.0)),
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

    /// Mutable access to the premultiplied RGBA8 pixels — used by style
    /// post-processing (the constellation glow bloom and space backdrop)
    /// that operates on raw bytes between draw calls.
    pub(crate) fn pixels_mut(&mut self) -> &mut [u8] {
        self.pix.data_mut()
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.pix.width(), self.pix.height())
    }

    /// The pixmap's bounds mapped back through the current transform, as
    /// `(min_x, min_y, max_x, max_y)` in drawing coordinates. Anything drawn
    /// outside this box is clipped away, so loops whose work grows with the
    /// drawn geometry can skip it. `None` for a non-invertible transform.
    pub(crate) fn visible_bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let inv = self.ts.invert()?;
        let (w, h) = (self.pix.width() as f32, self.pix.height() as f32);
        let mut corners = [
            tiny_skia::Point::from_xy(0.0, 0.0),
            tiny_skia::Point::from_xy(w, 0.0),
            tiny_skia::Point::from_xy(0.0, h),
            tiny_skia::Point::from_xy(w, h),
        ];
        inv.map_points(&mut corners);
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in corners {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        Some((x0, y0, x1, y1))
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
        // Exact quarter turns keep pixel axes orthogonal. sin/cos rounding at
        // 90 degrees would otherwise perturb half-pixel baseline snapping.
        let basis = match degrees.rem_euclid(360.0) {
            0.0 => Some((1.0, 0.0, 0.0, 1.0)),
            90.0 => Some((0.0, 1.0, -1.0, 0.0)),
            180.0 => Some((-1.0, 0.0, 0.0, -1.0)),
            270.0 => Some((0.0, -1.0, 1.0, 0.0)),
            _ => None,
        };
        let rotation = if let Some((sx, ky, kx, sy)) = basis {
            Transform::from_row(
                sx,
                ky,
                kx,
                sy,
                cx - sx * cx - kx * cy,
                cy - ky * cx - sy * cy,
            )
        } else {
            Transform::from_rotate_at(degrees, cx, cy)
        };
        self.ts = self.ts.pre_concat(rotation);
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

    /// Stroke a connected polyline through `points` as **one** path: vertices
    /// get proper line joins and a dash pattern runs continuously along the
    /// whole path instead of restarting at every vertex (which a chain of
    /// [`Self::draw_line`] calls would do). Non-finite points split the path
    /// into separate subpaths — the gap semantics the rest of figgy uses.
    pub fn draw_polyline(&mut self, points: &[(f32, f32)], paint: &Paint) {
        let mut pb = PathBuilder::new();
        let mut pen_down = false;
        for &(x, y) in points {
            if !x.is_finite() || !y.is_finite() {
                pen_down = false;
                continue;
            }
            if pen_down {
                pb.line_to(x, y);
            } else {
                pb.move_to(x, y);
                pen_down = true;
            }
        }
        let Some(path) = pb.finish() else { return };
        self.pix.stroke_path(
            &path,
            &paint.shader_paint(),
            &paint.stroke_params(),
            self.ts,
            None,
        );
    }

    /// [`Self::draw_polyline`] but with a smooth Catmull–Rom spline through
    /// the points (cubic Bézier segments) instead of straight joins — the
    /// pen-stroke look the sketch stroker needs: a straight polyline through
    /// noise samples reads as angular facets, a spline reads as a drawn
    /// curve. Same subpath split on non-finite points; dashes run along the
    /// curve continuously.
    pub fn draw_polyline_smooth(&mut self, points: &[(f32, f32)], paint: &Paint) {
        let mut pb = PathBuilder::new();
        crate::sketch::smooth_cubics(points, |segment| match segment {
            crate::sketch::PathSegment::Move(p) => pb.move_to(p.0, p.1),
            crate::sketch::PathSegment::Line(p) => pb.line_to(p.0, p.1),
            crate::sketch::PathSegment::Cubic(c1, c2, p) => {
                pb.cubic_to(c1.0, c1.1, c2.0, c2.1, p.0, p.1)
            }
        });
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
                let Some(rect) = tiny_skia::Rect::from_xywh(x, y, w, h) else {
                    return;
                };
                // tiny-skia 0.11's anti-aliased `fill_rect` always goes through
                // `hairline_aa::fill_dot8`, which trims the fractional edge
                // column and row and then asserts that a whole-pixel interior is
                // left. A rect that straddles a pixel boundary without covering a
                // full interior column has none — `x = 10.5, w = 0.999` panics —
                // and the assert is a `debug_assert!(false)`, so it takes the
                // whole raster down in a debug build. Under two pixels in either
                // direction, fill the rect as a path instead: the general
                // scanline filler has no such fast path and handles any size. A
                // colourbar band, a 1 px separator, or a thin selection outline
                // must not be able to kill a raster.
                if w < 2.0 || h < 2.0 {
                    let path = PathBuilder::from_rect(rect);
                    self.pix.fill_path(
                        &path,
                        &paint.shader_paint(),
                        tiny_skia::FillRule::Winding,
                        self.ts,
                        None,
                    );
                    return;
                }
                self.pix
                    .fill_rect(rect, &paint.shader_paint(), self.ts, None);
            }
            PaintStyle::Stroke => {
                let Some(rect) = tiny_skia::Rect::from_xywh(x, y, w, h) else {
                    return;
                };
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

    /// Fill a closed polygon through `points` (non-zero winding). Fewer than
    /// three finite points draw nothing.
    pub fn fill_polygon(&mut self, points: &[(f32, f32)], paint: &Paint) {
        if points.len() < 3 || points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(points[0].0, points[0].1);
        for &(x, y) in &points[1..] {
            pb.line_to(x, y);
        }
        pb.close();
        let Some(path) = pb.finish() else { return };
        self.pix.fill_path(
            &path,
            &paint.shader_paint(),
            tiny_skia::FillRule::Winding,
            self.ts,
            None,
        );
    }

    /// The current transform's translation, or `None` when it rotates/scales.
    pub fn translation(&self) -> Option<(f32, f32)> {
        let t = self.ts;
        if t.sx == 1.0 && t.sy == 1.0 && t.kx == 0.0 && t.ky == 0.0 {
            Some((t.tx, t.ty))
        } else {
            None
        }
    }

    /// A unit quarter-turn plus translation expressed in the unrotated glyph
    /// axes (R^-1 * t). This lets swash handle the local subpixel phase once.
    /// General rotations/scales retain the filtered fallback.
    pub(crate) fn orthogonal_text_transform(&self) -> Option<(u8, f32, f32)> {
        let t = self.ts;
        let turn = match (t.sx, t.ky, t.kx, t.sy) {
            (1.0, 0.0, 0.0, 1.0) => 0,
            (0.0, 1.0, -1.0, 0.0) => 1,
            (-1.0, 0.0, 0.0, -1.0) => 2,
            (0.0, -1.0, 1.0, 0.0) => 3,
            _ => return None,
        };
        Some((turn, t.sx * t.tx + t.ky * t.ty, t.kx * t.tx + t.sy * t.ty))
    }

    /// Direct integer-position src-over blit of a glyph alpha mask tinted
    /// with `color`. No filtering — the mask must already be rasterized at
    /// the right subpixel phase (swash render offset). This is the crisp
    /// text path; AA happens exactly once, in the glyph rasterizer.
    pub fn blit_mask(&mut self, ix: i32, iy: i32, w: u32, h: u32, alpha: &[u8], color: &Color) {
        self.blit_mask_orthogonal(ix, iy, w, h, alpha, color, 0);
    }

    /// Rotate pixel cells by an exact quarter turn, without filtering or a
    /// temporary glyph pixmap. Coordinates already include local translation.
    pub(crate) fn blit_mask_orthogonal(
        &mut self,
        ix: i32,
        iy: i32,
        w: u32,
        h: u32,
        alpha: &[u8],
        color: &Color,
        turn: u8,
    ) {
        if w == 0 || h == 0 || alpha.len() < (w as usize * h as usize) {
            return;
        }
        let c = ts_color(color);
        let (cr, cg, cb, ca) = (c.red(), c.green(), c.blue(), c.alpha());
        let dst_w = self.pix.width() as i32;
        let dst_h = self.pix.height() as i32;
        let px = self.pix.pixels_mut();

        for row in 0..h as i32 {
            for col in 0..w as i32 {
                let x = ix + col;
                let y = iy + row;
                // Map pixel cells, not their top-left points: a reversed axis
                // needs -1 so [x, x+1) becomes [-x-1, -x).
                let (dx, dy) = match turn {
                    0 => (x, y),
                    1 => (-y - 1, x),
                    2 => (-x - 1, -y - 1),
                    3 => (y, -x - 1),
                    _ => unreachable!("quarter turn must be in 0..4"),
                };
                if dx < 0 || dx >= dst_w || dy < 0 || dy >= dst_h {
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

    /// Filtered glyph blit for general transforms. Text at unit quarter turns
    /// uses `blit_mask_orthogonal` instead, preserving rasterized coverage.
    pub fn draw_mask(&mut self, x: f32, y: f32, w: u32, h: u32, alpha: &[u8], color: &Color) {
        if w == 0 || h == 0 || alpha.len() < (w as usize * h as usize) {
            return;
        }
        let Some(mut glyph) = Pixmap::new(w, h) else {
            return;
        };
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
        self.pix
            .draw_pixmap(0, 0, glyph.as_ref(), &paint, full_ts, None);
    }
}
