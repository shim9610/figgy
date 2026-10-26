//! The SVG writer: one `String`, glyph outlines once each in `<defs>`.
//!
//! The document is SVG 1.1: a `viewBox` in the export's physical pixels and
//! a `width` / `height` in points at the logical size. Paths carry their
//! paint as attributes (`fill` / `stroke`, alpha as `fill-opacity` /
//! `stroke-opacity`); a text block is a `<g>` translated to its origin and
//! rotated about its centre, its glyphs `<use>` references to their outline
//! in `<defs>`, one per distinct face, size and glyph; an image is an
//! `<image>` with a base64 PNG data URI; a clip is a `<g>` with a
//! `<clipPath>` rectangle.

use std::collections::HashMap;
use std::fmt::Write as _;

use swash::FontRef;

use super::{PathPaint, VectorOut};
use crate::color::Color;
use crate::error::Result;
use crate::layout::Rect;
use crate::renderer::{RasterImage, encode_png};
use crate::sketch::PathSegment;

/// Write `v` with at most three decimals, trailing zeros dropped.
pub(crate) fn push_num(out: &mut String, v: f32) {
    let start = out.len();
    let _ = write!(out, "{v:.3}");
    if out[start..].contains('.') {
        while out.ends_with('0') {
            out.pop();
        }
        if out.ends_with('.') {
            out.pop();
        }
    }
    if &out[start..] == "-0" {
        out.truncate(start);
        out.push('0');
    }
}

/// A colour channel as a byte, the raster's rounding of a clamped value.
pub(crate) fn channel(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn push_color(out: &mut String, c: Color) {
    let _ = write!(
        out,
        "#{:02x}{:02x}{:02x}",
        channel(c.r),
        channel(c.g),
        channel(c.b)
    );
}

/// ` name="value"` with the colour's alpha, when it is not opaque.
fn push_opacity(out: &mut String, name: &str, c: Color) {
    let a = c.a.clamp(0.0, 1.0);
    if a < 1.0 {
        let _ = write!(out, " {name}=\"");
        push_num(out, a);
        out.push('"');
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Append `bytes` in standard base64 with padding.
pub(crate) fn push_base64(out: &mut String, bytes: &[u8]) {
    out.reserve(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(BASE64[(n >> 18) as usize & 63] as char);
        out.push(BASE64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            BASE64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64[n as usize & 63] as char
        } else {
            '='
        });
    }
}

/// Append the path data of `segment`.
fn push_segment(out: &mut String, segment: PathSegment) {
    let point = |out: &mut String, p: (f32, f32)| {
        push_num(out, p.0);
        out.push(' ');
        push_num(out, p.1);
    };
    match segment {
        PathSegment::Move(p) => {
            out.push('M');
            point(out, p);
        }
        PathSegment::Line(p) => {
            out.push('L');
            point(out, p);
        }
        PathSegment::Cubic(c1, c2, p) => {
            out.push('C');
            point(out, c1);
            out.push(' ');
            point(out, c2);
            out.push(' ');
            point(out, p);
        }
        PathSegment::Close => out.push('Z'),
    }
}

fn segment_is_finite(segment: &PathSegment) -> bool {
    let f = |p: &(f32, f32)| p.0.is_finite() && p.1.is_finite();
    match segment {
        PathSegment::Move(p) | PathSegment::Line(p) => f(p),
        PathSegment::Cubic(a, b, c) => f(a) && f(b) && f(c),
        PathSegment::Close => true,
    }
}

/// Glyph definitions: face, size bits and glyph id to the `<defs>` id.
type GlyphKey = (swash::CacheKey, u32, u16);

/// An opaque stroke's paint: colour, width and dash, compared bit for bit.
#[derive(Clone, Copy, PartialEq)]
struct StrokeKey {
    color: [u32; 3],
    width: u32,
    dash: [u32; 8],
    dash_len: usize,
}

impl StrokeKey {
    /// The key of `paint` when it is an opaque stroke: its subpaths can join
    /// the previous path of the same paint without changing a pixel, since
    /// opaque overlaps cover the same either way.
    fn of(paint: &PathPaint<'_>) -> Option<Self> {
        let PathPaint::Stroke { color, width, dash } = *paint else {
            return None;
        };
        if color.a < 1.0 || dash.len() > 8 {
            return None;
        }
        let mut bits = [0u32; 8];
        for (b, d) in bits.iter_mut().zip(dash) {
            *b = d.to_bits();
        }
        Some(Self {
            color: [color.r.to_bits(), color.g.to_bits(), color.b.to_bits()],
            width: width.to_bits(),
            dash: bits,
            dash_len: dash.len(),
        })
    }
}

/// The path being written.
struct OpenPath {
    /// Where its text starts: its `<path`, or its first subpath when it
    /// joined the previous path.
    start: usize,
    valid: bool,
    segments: usize,
    joined: bool,
    key: Option<StrokeKey>,
}

const PATH_END: &str = "\"/>\n";

/// The SVG writer. [`Self::finish`] returns the document.
pub(crate) struct SvgOut {
    /// The document: the header, then the body as it is written.
    svg: String,
    /// Where `<defs>` goes: right after the header.
    defs_at: usize,
    /// The glyph outlines, written once each.
    defs: String,
    glyphs: HashMap<GlyphKey, u32>,
    path: Option<OpenPath>,
    /// The last path's stroke key and where its text ends: a path of the
    /// same opaque stroke right after it joins it as more subpaths.
    last: Option<(StrokeKey, usize)>,
    clips: u32,
}

impl SvgOut {
    /// A document of `size` physical pixels shown at `page` points.
    pub(crate) fn new(size: (u32, u32), page: [f32; 2]) -> Self {
        let mut svg = String::with_capacity(64 * 1024);
        svg.push_str(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" \
             xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\" width=\"",
        );
        push_num(&mut svg, page[0]);
        svg.push_str("pt\" height=\"");
        push_num(&mut svg, page[1]);
        let _ = write!(svg, "pt\" viewBox=\"0 0 {} {}\">\n", size.0, size.1);
        let defs_at = svg.len();
        Self {
            svg,
            defs_at,
            defs: String::new(),
            glyphs: HashMap::new(),
            path: None,
            last: None,
            clips: 0,
        }
    }

    /// The finished document, the glyph `<defs>` in front of the body.
    pub(crate) fn finish(mut self) -> String {
        if !self.defs.is_empty() {
            self.defs.insert_str(0, "<defs>\n");
            self.defs.push_str("</defs>\n");
            self.svg.insert_str(self.defs_at, &self.defs);
        }
        self.svg.push_str("</svg>\n");
        self.svg
    }

    /// The `<defs>` id of a glyph's outline, written on first use.
    fn glyph_id(&mut self, font: FontRef<'static>, glyph: u16, size: f32) -> u32 {
        let key = (font.key, size.to_bits(), glyph);
        if let Some(id) = self.glyphs.get(&key) {
            return *id;
        }
        let id = self.glyphs.len() as u32 + 1;
        self.glyphs.insert(key, id);
        let _ = write!(self.defs, "<path id=\"g{id}\" d=\"");
        let defs = &mut self.defs;
        crate::text_render::glyph_outline(font, glyph, size, &mut |segment| {
            if segment_is_finite(&segment) {
                push_segment(defs, segment);
            }
        });
        self.defs.push_str("\"/>\n");
        id
    }
}

impl VectorOut for SvgOut {
    fn begin_path(&mut self, paint: PathPaint<'_>) {
        let key = StrokeKey::of(&paint);
        if let (Some(key), Some((last, end))) = (key, self.last)
            && key == last
            && end == self.svg.len()
        {
            self.svg.truncate(end - PATH_END.len());
            self.path = Some(OpenPath {
                start: self.svg.len(),
                valid: true,
                segments: 0,
                joined: true,
                key: Some(key),
            });
            return;
        }
        self.path = Some(OpenPath {
            start: self.svg.len(),
            valid: true,
            segments: 0,
            joined: false,
            key,
        });
        let svg = &mut self.svg;
        svg.push_str("<path");
        match paint {
            PathPaint::Fill(color) => {
                svg.push_str(" fill=\"");
                push_color(svg, color);
                svg.push('"');
                push_opacity(svg, "fill-opacity", color);
            }
            PathPaint::Stroke { color, width, dash } => {
                svg.push_str(" fill=\"none\" stroke=\"");
                push_color(svg, color);
                svg.push_str("\" stroke-width=\"");
                push_num(svg, width);
                svg.push('"');
                push_opacity(svg, "stroke-opacity", color);
                if !dash.is_empty() {
                    svg.push_str(" stroke-dasharray=\"");
                    for (i, d) in dash.iter().enumerate() {
                        if i > 0 {
                            svg.push(' ');
                        }
                        push_num(svg, *d);
                    }
                    svg.push('"');
                }
            }
        }
        svg.push_str(" d=\"");
    }

    fn segment(&mut self, segment: PathSegment) {
        let Some(path) = self.path.as_mut() else {
            return;
        };
        path.valid &= segment_is_finite(&segment);
        if path.valid {
            push_segment(&mut self.svg, segment);
            path.segments += 1;
        }
    }

    fn end_path(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if !path.valid || path.segments == 0 {
            self.svg.truncate(path.start);
            if path.joined {
                // The previous path, as it was.
                self.svg.push_str(PATH_END);
            }
            return;
        }
        self.svg.push_str(PATH_END);
        self.last = path.key.map(|key| (key, self.svg.len()));
    }

    fn begin_text(&mut self, origin: [f32; 2], rotation: Option<(f32, [f32; 2])>) {
        let svg = &mut self.svg;
        svg.push_str("<g transform=\"");
        if let Some((degrees, center)) = rotation {
            svg.push_str("rotate(");
            push_num(svg, degrees);
            svg.push(' ');
            push_num(svg, center[0]);
            svg.push(' ');
            push_num(svg, center[1]);
            svg.push_str(") ");
        }
        svg.push_str("translate(");
        push_num(svg, origin[0]);
        svg.push(' ');
        push_num(svg, origin[1]);
        svg.push_str(")\">\n");
    }

    fn glyph(
        &mut self,
        font: FontRef<'static>,
        glyph: u16,
        size: f32,
        pen: [f32; 2],
        color: Color,
    ) {
        if !(pen[0].is_finite() && pen[1].is_finite() && size.is_finite() && size > 0.0) {
            return;
        }
        let id = self.glyph_id(font, glyph, size);
        let svg = &mut self.svg;
        let _ = write!(svg, "<use xlink:href=\"#g{id}\" x=\"");
        push_num(svg, pen[0]);
        svg.push_str("\" y=\"");
        push_num(svg, pen[1]);
        svg.push_str("\" fill=\"");
        push_color(svg, color);
        svg.push('"');
        push_opacity(svg, "fill-opacity", color);
        svg.push_str("/>\n");
    }

    fn end_text(&mut self) {
        self.svg.push_str("</g>\n");
    }

    fn push_clip(&mut self, rect: Rect) {
        self.clips += 1;
        let n = self.clips;
        let _ = write!(
            self.svg,
            "<clipPath id=\"c{n}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>\n\
             <g clip-path=\"url(#c{n})\">\n",
            rect.x, rect.y, rect.width, rect.height
        );
    }

    fn pop_clip(&mut self) {
        self.svg.push_str("</g>\n");
    }

    fn image(&mut self, origin: [i32; 2], image: &RasterImage) -> Result<()> {
        if image.width == 0 || image.height == 0 {
            return Ok(());
        }
        let png = encode_png(image)?;
        let _ = write!(
            self.svg,
            "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" \
             xlink:href=\"data:image/png;base64,",
            origin[0], origin[1], image.width, image.height
        );
        push_base64(&mut self.svg, &png);
        self.svg.push_str("\"/>\n");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_keep_three_decimals_without_trailing_zeros() {
        let mut s = String::new();
        for v in [1.0f32, 0.5, -0.0001, 12.3456, -3.25, 100.0] {
            push_num(&mut s, v);
            s.push(' ');
        }
        assert_eq!(s, "1 0.5 0 12.346 -3.25 100 ");
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            let mut s = String::new();
            push_base64(&mut s, input.as_bytes());
            assert_eq!(s, expected);
        }
    }
}
