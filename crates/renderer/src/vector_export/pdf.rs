//! The PDF writer: a single-page PDF 1.4 file written by hand, its streams
//! deflated by `flate2`.
//!
//! The page's MediaBox is the logical size in points; its content stream
//! starts with the transform from the export's physical pixels (y down) to
//! the page, so every operator after it works in pixels like the SVG.
//! Paths are written with their colour and alpha set first (alpha through
//! one ExtGState per alpha byte the page uses); a text block is a saved
//! graphics state translated to its origin and rotated about its centre,
//! each glyph's outline inline at its pen; an image is an RGB XObject with
//! a DeviceGray soft mask, both deflated with the PNG Paeth predictor; a
//! clip is a saved state with a rectangle clip.
//!
//! Images are written as objects while the page is drawn; the content
//! stream, the page, its parents, the cross-reference table and the
//! trailer follow when the page is finished. Every stream's length is an
//! indirect object written after the stream, so each stream is deflated
//! straight into the file.

use std::io::Write as _;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use swash::FontRef;

use super::svg::channel;
use super::{PathPaint, VectorOut};
use crate::color::Color;
use crate::error::{FiggyError, Result};
use crate::layout::Rect;
use crate::renderer::RasterImage;
use crate::sketch::PathSegment;

/// Write `v` with at most three decimals, trailing zeros dropped.
fn push_num(out: &mut Vec<u8>, v: f32) {
    let start = out.len();
    let _ = write!(out, "{v:.3}");
    if out[start..].contains(&b'.') {
        while out.last() == Some(&b'0') {
            out.pop();
        }
        if out.last() == Some(&b'.') {
            out.pop();
        }
    }
    if &out[start..] == b"-0" {
        out.truncate(start);
        out.push(b'0');
    }
}

fn push_nums(out: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        push_num(out, *v);
        out.push(b' ');
    }
}

fn write_failed(what: &str, e: std::io::Error) -> FiggyError {
    FiggyError::RasterWrapFailed {
        reason: format!("pdf {what}: {e}"),
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

/// How deep text blocks, clips and glyphs nest their saved states.
const MAX_DEPTH: usize = 16;

/// Objects per image: its soft mask, the mask's length, the image and the
/// image's length.
const OBJECTS_PER_IMAGE: u32 = 4;

/// The PDF writer. [`Self::finish`] returns the file.
pub(crate) struct PdfOut {
    /// The file: the header and the image objects as they are written.
    out: Vec<u8>,
    /// The page's content stream, deflated into the file when finished.
    content: Vec<u8>,
    /// The file offset of every object, in object number order.
    offsets: Vec<u32>,
    /// The alpha bytes the page paints with: one ExtGState each.
    alphas: [bool; 256],
    /// The alpha of the current graphics state and of each saved one.
    alpha: [u8; MAX_DEPTH],
    depth: usize,
    images: u32,
    /// The content offset of the path being written and whether it is
    /// still valid, and whether it is a fill.
    path: Option<(usize, bool, bool)>,
    page: [f32; 2],
}

impl PdfOut {
    /// A page of `size` physical pixels shown at `page` points.
    pub(crate) fn new(size: (u32, u32), page: [f32; 2]) -> Self {
        let mut out = Vec::with_capacity(64 * 1024);
        out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        let mut content = Vec::with_capacity(64 * 1024);
        // Pixels, y down, onto the page, y up; tiny-skia's miter limit.
        push_nums(
            &mut content,
            &[
                page[0] / size.0.max(1) as f32,
                0.0,
                0.0,
                -page[1] / size.1.max(1) as f32,
                0.0,
                page[1],
            ],
        );
        content.extend_from_slice(b"cm\n4 M\n");
        Self {
            out,
            content,
            offsets: Vec::new(),
            alphas: [false; 256],
            alpha: [255; MAX_DEPTH],
            depth: 0,
            images: 0,
            path: None,
            page,
        }
    }

    /// Start object `number` (the next one): record its offset.
    fn begin_object(&mut self, number: u32) -> Result<()> {
        debug_assert_eq!(number as usize, self.offsets.len() + 1);
        let offset = u32::try_from(self.out.len()).map_err(|_| FiggyError::RasterWrapFailed {
            reason: "pdf output exceeds 4 GiB".into(),
        })?;
        self.offsets.push(offset);
        let _ = write!(self.out, "{number} 0 obj\n");
        Ok(())
    }

    /// Write stream object `number` with dictionary entries `dict`, its
    /// data deflated from `fill`, and its length as object `number + 1`.
    fn stream_object(
        &mut self,
        number: u32,
        dict: impl FnOnce(&mut Vec<u8>),
        fill: impl FnOnce(&mut ZlibEncoder<&mut Vec<u8>>) -> std::io::Result<()>,
    ) -> Result<()> {
        self.begin_object(number)?;
        self.out.extend_from_slice(b"<<");
        dict(&mut self.out);
        let _ = write!(
            self.out,
            " /Filter /FlateDecode /Length {} 0 R >>\nstream\n",
            number + 1
        );
        let start = self.out.len();
        let mut encoder = ZlibEncoder::new(&mut self.out, Compression::best());
        fill(&mut encoder).map_err(|e| write_failed("deflate", e))?;
        encoder.finish().map_err(|e| write_failed("deflate", e))?;
        let length = self.out.len() - start;
        self.out.extend_from_slice(b"\nendstream\nendobj\n");
        self.begin_object(number + 1)?;
        let _ = write!(self.out, "{length}\nendobj\n");
        Ok(())
    }

    /// Set the alpha of the current graphics state to `a` (both the fill and
    /// the stroke alpha).
    fn set_alpha(&mut self, a: f32) {
        let byte = channel(a);
        if self.alpha[self.depth] == byte {
            return;
        }
        self.alpha[self.depth] = byte;
        self.alphas[byte as usize] = true;
        let _ = write!(self.content, "/A{byte} gs\n");
    }

    fn save(&mut self) {
        self.content.extend_from_slice(b"q\n");
        if self.depth + 1 < MAX_DEPTH {
            self.alpha[self.depth + 1] = self.alpha[self.depth];
        }
        self.depth = (self.depth + 1).min(MAX_DEPTH - 1);
    }

    fn restore(&mut self) {
        self.content.extend_from_slice(b"Q\n");
        self.depth = self.depth.saturating_sub(1);
    }

    fn color(&mut self, c: Color, op: &[u8]) {
        push_nums(
            &mut self.content,
            &[
                channel(c.r) as f32 / 255.0,
                channel(c.g) as f32 / 255.0,
                channel(c.b) as f32 / 255.0,
            ],
        );
        self.content.extend_from_slice(op);
        self.content.push(b'\n');
    }

    fn push_segment(&mut self, segment: PathSegment) {
        let content = &mut self.content;
        match segment {
            PathSegment::Move(p) => {
                push_nums(content, &[p.0, p.1]);
                content.extend_from_slice(b"m\n");
            }
            PathSegment::Line(p) => {
                push_nums(content, &[p.0, p.1]);
                content.extend_from_slice(b"l\n");
            }
            PathSegment::Cubic(c1, c2, p) => {
                push_nums(content, &[c1.0, c1.1, c2.0, c2.1, p.0, p.1]);
                content.extend_from_slice(b"c\n");
            }
            PathSegment::Close => content.extend_from_slice(b"h\n"),
        }
    }

    /// The finished file: the content stream, the page and its parents,
    /// the cross-reference table and the trailer.
    pub(crate) fn finish(mut self) -> Result<Vec<u8>> {
        let images = self.images * OBJECTS_PER_IMAGE;
        let (content, page, pages, catalog) = (images + 1, images + 3, images + 4, images + 5);
        let stream = std::mem::take(&mut self.content);
        self.stream_object(content, |_| {}, |encoder| encoder.write_all(&stream))?;
        drop(stream);

        self.begin_object(page)?;
        let _ = write!(
            self.out,
            "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 "
        );
        let page_size = self.page;
        push_nums(&mut self.out, &page_size);
        self.out.extend_from_slice(b"] /Resources << /XObject <<");
        for k in 1..=self.images {
            let _ = write!(self.out, " /Im{k} {} 0 R", k * OBJECTS_PER_IMAGE - 1);
        }
        self.out.extend_from_slice(b" >> /ExtGState <<");
        for (byte, used) in self.alphas.iter().enumerate() {
            if *used {
                let _ = write!(self.out, " /A{byte} << /Type /ExtGState /ca ");
                push_num(&mut self.out, byte as f32 / 255.0);
                self.out.extend_from_slice(b" /CA ");
                push_num(&mut self.out, byte as f32 / 255.0);
                self.out.extend_from_slice(b" >>");
            }
        }
        let _ = write!(self.out, " >> >> /Contents {content} 0 R >>\nendobj\n");
        self.begin_object(pages)?;
        let _ = write!(
            self.out,
            "<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>\nendobj\n"
        );
        self.begin_object(catalog)?;
        let _ = write!(
            self.out,
            "<< /Type /Catalog /Pages {pages} 0 R >>\nendobj\n"
        );

        let xref = self.out.len();
        let _ = write!(
            self.out,
            "xref\n0 {}\n0000000000 65535 f \n",
            self.offsets.len() + 1
        );
        for offset in &self.offsets {
            let _ = write!(self.out, "{offset:010} 00000 n \n");
        }
        let _ = write!(
            self.out,
            "trailer\n<< /Size {} /Root {catalog} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            self.offsets.len() + 1
        );
        Ok(self.out)
    }
}

/// The PNG Paeth predictor of one sample from its left, upper and
/// upper-left neighbours.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Deflate the `channels` (offsets into each RGBA pixel) of `image`, each
/// row Paeth-filtered (PNG predictor 15, filter type 4), through a fixed
/// buffer.
fn write_predicted(
    encoder: &mut ZlibEncoder<&mut Vec<u8>>,
    image: &RasterImage,
    channels: &[usize],
) -> std::io::Result<()> {
    let w = image.width as usize;
    let px = |x: usize, y: usize, c: usize| image.rgba[(y * w + x) * 4 + c];
    let mut buf = [0u8; 4096];
    let mut n = 0;
    let mut push = |encoder: &mut ZlibEncoder<&mut Vec<u8>>, byte: u8| -> std::io::Result<()> {
        buf[n] = byte;
        n += 1;
        if n == buf.len() {
            encoder.write_all(&buf)?;
            n = 0;
        }
        Ok(())
    };
    for y in 0..image.height as usize {
        push(encoder, 4)?;
        for x in 0..w {
            for &c in channels {
                let a = if x > 0 { px(x - 1, y, c) } else { 0 };
                let b = if y > 0 { px(x, y - 1, c) } else { 0 };
                let ul = if x > 0 && y > 0 {
                    px(x - 1, y - 1, c)
                } else {
                    0
                };
                push(encoder, px(x, y, c).wrapping_sub(paeth(a, b, ul)))?;
            }
        }
    }
    encoder.write_all(&buf[..n])
}

impl VectorOut for PdfOut {
    fn begin_path(&mut self, paint: PathPaint<'_>) {
        let fill = match paint {
            PathPaint::Fill(color) => {
                self.set_alpha(color.a);
                self.color(color, b"rg");
                true
            }
            PathPaint::Stroke { color, width, dash } => {
                self.set_alpha(color.a);
                self.color(color, b"RG");
                push_num(&mut self.content, width);
                self.content.extend_from_slice(b" w [");
                for (i, d) in dash.iter().enumerate() {
                    if i > 0 {
                        self.content.push(b' ');
                    }
                    push_num(&mut self.content, *d);
                }
                self.content.extend_from_slice(b"] 0 d\n");
                false
            }
        };
        self.path = Some((self.content.len(), true, fill));
    }

    fn segment(&mut self, segment: PathSegment) {
        let Some((_, valid, _)) = self.path.as_mut() else {
            return;
        };
        *valid &= segment_is_finite(&segment);
        if *valid {
            self.push_segment(segment);
        }
    }

    fn end_path(&mut self) {
        let Some((start, valid, fill)) = self.path.take() else {
            return;
        };
        if !valid || self.content.len() == start {
            self.content.truncate(start);
            return;
        }
        self.content
            .extend_from_slice(if fill { b"f\n" } else { b"S\n" });
    }

    fn begin_text(&mut self, origin: [f32; 2], rotation: Option<(f32, [f32; 2])>) {
        self.save();
        if let Some((degrees, [cx, cy])) = rotation {
            // tiny-skia's rotation about a centre, with its exact quarter
            // turns (`Canvas::rotate_at`).
            let (cos, sin) = match degrees.rem_euclid(360.0) {
                0.0 => (1.0, 0.0),
                90.0 => (0.0, 1.0),
                180.0 => (-1.0, 0.0),
                270.0 => (0.0, -1.0),
                _ => {
                    let r = degrees.to_radians();
                    (r.cos(), r.sin())
                }
            };
            push_nums(
                &mut self.content,
                &[
                    cos,
                    sin,
                    -sin,
                    cos,
                    cx - cos * cx + sin * cy,
                    cy - sin * cx - cos * cy,
                ],
            );
            self.content.extend_from_slice(b"cm\n");
        }
        self.content.extend_from_slice(b"1 0 0 1 ");
        push_nums(&mut self.content, &origin);
        self.content.extend_from_slice(b"cm\n");
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
        self.save();
        self.content.extend_from_slice(b"1 0 0 1 ");
        push_nums(&mut self.content, &pen);
        self.content.extend_from_slice(b"cm\n");
        self.set_alpha(color.a);
        self.color(color, b"rg");
        let start = self.content.len();
        crate::text_render::glyph_outline(font, glyph, size, &mut |segment| {
            if segment_is_finite(&segment) {
                self.push_segment(segment);
            }
        });
        if self.content.len() > start {
            self.content.extend_from_slice(b"f\n");
        }
        self.restore();
    }

    fn end_text(&mut self) {
        self.restore();
    }

    fn push_clip(&mut self, rect: Rect) {
        self.save();
        let _ = write!(
            self.content,
            "{} {} {} {} re W n\n",
            rect.x, rect.y, rect.width, rect.height
        );
    }

    fn pop_clip(&mut self) {
        self.restore();
    }

    fn image(&mut self, origin: [i32; 2], image: &RasterImage) -> Result<()> {
        if image.width == 0 || image.height == 0 {
            return Ok(());
        }
        let number = self.images * OBJECTS_PER_IMAGE + 1;
        let (w, h) = (image.width, image.height);
        let header = |out: &mut Vec<u8>, colors: u32| {
            let _ = write!(
                out,
                " /Type /XObject /Subtype /Image /Width {w} /Height {h} /BitsPerComponent 8 \
                 /DecodeParms << /Predictor 15 /Colors {colors} /BitsPerComponent 8 /Columns {w} >>"
            );
        };
        self.stream_object(
            number,
            |out| {
                header(out, 1);
                out.extend_from_slice(b" /ColorSpace /DeviceGray");
            },
            |encoder| write_predicted(encoder, image, &[3]),
        )?;
        self.stream_object(
            number + 2,
            |out| {
                header(out, 3);
                let _ = write!(out, " /ColorSpace /DeviceRGB /SMask {number} 0 R");
            },
            |encoder| write_predicted(encoder, image, &[0, 1, 2]),
        )?;
        self.images += 1;
        self.save();
        push_nums(
            &mut self.content,
            &[
                w as f32,
                0.0,
                0.0,
                -(h as f32),
                origin[0] as f32,
                (origin[1] + h as i32) as f32,
            ],
        );
        let _ = write!(self.content, "cm /Im{} Do\n", self.images);
        self.restore();
        Ok(())
    }
}
