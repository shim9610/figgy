//! The SVG subset the vector export writes (#48), read back part by part,
//! and the comparison of read-back parts with the parts the export was
//! handed (`renderer::written_document_parts`, a `Parts` recording of the
//! same emit). `pdf_structure.rs` reads the PDF into the same [`ReadPart`]s.
#![allow(dead_code)]

use renderer::layout::{Rect, RectF};
use renderer::{DrawnPart, WrittenPart};

/// One part read back from a file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReadPart {
    /// A straight stroke, a rectangle (fill or border) or a triangle, with
    /// the stroke width the file paints it with (`None` for a fill).
    Part(DrawnPart, Option<f32>),
    /// A curved stroke (the sketch stroker's wobble): its end points and
    /// width.
    Curve {
        first: [f32; 2],
        last: [f32; 2],
        width: f32,
    },
    /// The start of a text block: the block's transform `[a b c d e f]`.
    Text([f32; 6]),
    /// A glyph's pen relative to its text block's origin.
    Glyph([f32; 2]),
    /// A filled rectangle inside a text block, relative to its origin.
    TextRect(RectF),
    TextEnd,
    Image(Rect),
}

/// A path being read: its subpaths' points, whether each closes and
/// whether it has a curve.
#[derive(Default)]
pub struct Subpath {
    pub points: Vec<[f32; 2]>,
    pub closed: bool,
    pub curved: bool,
}

/// Classify one painted subpath the way the writers build them.
pub fn classify(sub: &Subpath, stroke: Option<f32>, in_text: bool) -> ReadPart {
    if sub.curved {
        return ReadPart::Curve {
            first: sub.points[0],
            last: *sub.points.last().unwrap(),
            width: stroke.expect("only sketch strokes are curved outside glyphs"),
        };
    }
    let p = &sub.points;
    match (p.len(), sub.closed) {
        (2, false) => ReadPart::Part(
            DrawnPart::Segment {
                a: p[0],
                b: p[1],
                width: stroke.unwrap_or(0.0),
            },
            stroke,
        ),
        (4, true) => {
            let x0 = p.iter().map(|q| q[0]).fold(f32::MAX, f32::min);
            let y0 = p.iter().map(|q| q[1]).fold(f32::MAX, f32::min);
            let x1 = p.iter().map(|q| q[0]).fold(f32::MIN, f32::max);
            let y1 = p.iter().map(|q| q[1]).fold(f32::MIN, f32::max);
            let r = RectF {
                x: x0,
                y: y0,
                width: x1 - x0,
                height: y1 - y0,
            };
            if in_text && stroke.is_none() {
                ReadPart::TextRect(r)
            } else {
                ReadPart::Part(DrawnPart::Rect(r), stroke)
            }
        }
        (3, true) => ReadPart::Part(DrawnPart::Triangle([p[0], p[1], p[2]]), stroke),
        other => panic!("unexpected subpath {other:?}: {:?}", sub.points),
    }
}

/// The tokens of SVG path data as the writer spells it: command letters and
/// numbers.
fn path_data(d: &str) -> Vec<Subpath> {
    let mut subs: Vec<Subpath> = Vec::new();
    let mut numbers: Vec<f32> = Vec::new();
    let mut command = ' ';
    let flush = |command: char, numbers: &mut Vec<f32>, subs: &mut Vec<Subpath>| {
        match command {
            'M' => subs.push(Subpath {
                points: vec![[numbers[0], numbers[1]]],
                ..Subpath::default()
            }),
            'L' => subs
                .last_mut()
                .unwrap()
                .points
                .push([numbers[0], numbers[1]]),
            'C' => {
                let s = subs.last_mut().unwrap();
                s.points.push([numbers[4], numbers[5]]);
                s.curved = true;
            }
            'Z' => subs.last_mut().unwrap().closed = true,
            ' ' => {}
            c => panic!("unexpected path command {c}"),
        }
        numbers.clear();
    };
    let mut token = String::new();
    for ch in d.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphabetic() || ch == ' ' {
            if !token.is_empty() {
                numbers.push(token.parse().expect("path number"));
                token.clear();
            }
            if ch.is_ascii_alphabetic() {
                flush(command, &mut numbers, &mut subs);
                command = ch;
            }
        } else {
            token.push(ch);
        }
    }
    flush(command, &mut numbers, &mut subs);
    subs
}

/// One tag: its name (with a leading `/` for a closing tag) and attributes.
struct Tag<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, &'a str)>,
}

impl<'a> Tag<'a> {
    fn get(&self, name: &str) -> Option<&'a str> {
        self.attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }
    fn num(&self, name: &str) -> f32 {
        self.get(name)
            .unwrap_or_else(|| panic!("<{}> without {name}", self.name))
            .parse()
            .unwrap()
    }
}

fn tags(svg: &str) -> Vec<Tag<'_>> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find('<') {
        let end = start + rest[start..].find('>').expect("unterminated tag");
        let inner = rest[start + 1..end].trim_end_matches('/');
        rest = &rest[end + 1..];
        if inner.starts_with('?') {
            continue;
        }
        let (name, mut attrs_text) = inner.split_once(' ').unwrap_or((inner, ""));
        let mut attrs = Vec::new();
        while let Some(eq) = attrs_text.find("=\"") {
            let key = attrs_text[..eq].trim();
            let value_start = eq + 2;
            let value_end = value_start + attrs_text[value_start..].find('"').unwrap();
            attrs.push((key, &attrs_text[value_start..value_end]));
            attrs_text = &attrs_text[value_end + 1..];
        }
        out.push(Tag { name, attrs });
    }
    out
}

/// The transform of a text block's `transform` attribute:
/// `[rotate(d cx cy) ]translate(x y)`.
fn text_transform(t: &str) -> [f32; 6] {
    let args = |s: &str| -> Vec<f32> {
        s.split(' ')
            .filter(|v| !v.is_empty())
            .map(|v| v.parse().unwrap())
            .collect()
    };
    let (rotation, translate) = match t.split_once(") ") {
        Some((r, tr)) => (Some(args(r.trim_start_matches("rotate("))), tr),
        None => (None, t),
    };
    let o = args(
        translate
            .trim_start_matches("translate(")
            .trim_end_matches(')'),
    );
    block_transform([o[0], o[1]], rotation.map(|r| (r[0], [r[1], r[2]])))
}

/// A text block's transform from its origin and rotation: the rotation
/// (exact at quarter turns, as the canvas's) after the translation.
pub fn block_transform(origin: [f32; 2], rotation: Option<(f32, [f32; 2])>) -> [f32; 6] {
    let Some((degrees, [cx, cy])) = rotation else {
        return [1.0, 0.0, 0.0, 1.0, origin[0], origin[1]];
    };
    let (cos, sin) = match degrees.rem_euclid(360.0) {
        0.0 => (1.0, 0.0),
        90.0 => (0.0, 1.0),
        180.0 => (-1.0, 0.0),
        270.0 => (0.0, -1.0),
        _ => {
            let r = (degrees as f64).to_radians();
            (r.cos() as f32, r.sin() as f32)
        }
    };
    let (e, f) = (cx - cos * cx + sin * cy, cy - sin * cx - cos * cy);
    [
        cos,
        sin,
        -sin,
        cos,
        cos * origin[0] - sin * origin[1] + e,
        sin * origin[0] + cos * origin[1] + f,
    ]
}

/// An SVG file read back.
pub struct SvgFile {
    /// The `viewBox` size: the export's physical pixels.
    pub size: (u32, u32),
    /// `width` and `height` in points.
    pub page: [f32; 2],
    pub parts: Vec<ReadPart>,
    /// Each image's pixels and PNG bytes, in order.
    pub images: Vec<(Rect, Vec<u8>)>,
    /// Glyph outlines in `<defs>` and the `<use>`s of them.
    pub glyph_defs: usize,
    pub glyph_uses: usize,
    /// Bytes of the file outside the images' data URIs.
    pub other_bytes: usize,
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn decode_base64(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let value = |c: u8| BASE64.iter().position(|b| *b == c).expect("base64 digit") as u32;
    for chunk in s.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|c| **c == b'=').count();
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= if *c == b'=' { 0 } else { value(*c) } << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    out
}

/// Read `svg`, asserting the structure the writer gives it: a balanced
/// document, every `<use>` naming a defined glyph, every image a PNG data
/// URI.
pub fn read(svg: &str) -> SvgFile {
    assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg "));
    assert!(svg.ends_with("</svg>\n"));
    let mut size = (0, 0);
    let mut page = [0.0; 2];
    let mut parts = Vec::new();
    let mut images = Vec::new();
    let mut defined = std::collections::BTreeSet::new();
    let mut used = std::collections::BTreeSet::new();
    let mut glyph_uses = 0;
    let mut image_bytes = 0;
    // The open groups: true for a text block, false for a clip.
    let mut groups: Vec<bool> = Vec::new();
    let mut in_defs = false;
    let mut in_clip_path = false;
    for tag in tags(svg) {
        match tag.name {
            "svg" => {
                let view: Vec<u32> = tag
                    .get("viewBox")
                    .unwrap()
                    .split(' ')
                    .map(|v| v.parse().unwrap())
                    .collect();
                assert_eq!(&view[..2], &[0, 0]);
                size = (view[2], view[3]);
                page = [
                    tag.get("width")
                        .unwrap()
                        .trim_end_matches("pt")
                        .parse()
                        .unwrap(),
                    tag.get("height")
                        .unwrap()
                        .trim_end_matches("pt")
                        .parse()
                        .unwrap(),
                ];
            }
            "defs" => in_defs = true,
            "/defs" => in_defs = false,
            "clipPath" => in_clip_path = true,
            "/clipPath" => in_clip_path = false,
            "rect" => assert!(in_clip_path, "a <rect> outside a clip path"),
            "path" if in_defs => {
                let id = tag.get("id").expect("a glyph definition has an id");
                assert!(defined.insert(id.to_owned()), "glyph {id} defined twice");
            }
            "path" => {
                let in_text = groups.iter().any(|t| *t);
                let stroke = (tag.get("fill") == Some("none")).then(|| tag.num("stroke-width"));
                for sub in path_data(tag.get("d").unwrap()) {
                    parts.push(classify(&sub, stroke, in_text));
                }
            }
            "g" => match tag.get("transform") {
                Some(t) => {
                    groups.push(true);
                    parts.push(ReadPart::Text(text_transform(t)));
                }
                None => {
                    assert!(
                        tag.get("clip-path").is_some(),
                        "a <g> is a text block or a clip"
                    );
                    groups.push(false);
                }
            },
            "/g" => {
                if groups.pop().expect("balanced groups") {
                    parts.push(ReadPart::TextEnd);
                }
            }
            "use" => {
                let href = tag.get("xlink:href").unwrap().trim_start_matches('#');
                used.insert(href.to_owned());
                glyph_uses += 1;
                assert!(groups.last() == Some(&true), "a glyph outside a text block");
                parts.push(ReadPart::Glyph([tag.num("x"), tag.num("y")]));
            }
            "image" => {
                let rect = Rect {
                    x: tag.num("x") as u32,
                    y: tag.num("y") as u32,
                    width: tag.num("width") as u32,
                    height: tag.num("height") as u32,
                };
                let uri = tag.get("xlink:href").unwrap();
                let data = uri
                    .strip_prefix("data:image/png;base64,")
                    .expect("a PNG data URI");
                image_bytes += data.len();
                parts.push(ReadPart::Image(rect));
                images.push((rect, decode_base64(data)));
            }
            "/svg" => {}
            other => panic!("unexpected element <{other}>"),
        }
    }
    assert!(groups.is_empty(), "unbalanced groups");
    assert!(used.is_subset(&defined), "a <use> names an undefined glyph");
    SvgFile {
        size,
        page,
        parts,
        images,
        glyph_defs: defined.len(),
        glyph_uses,
        other_bytes: svg.len() - image_bytes,
    }
}

/// How far two points are apart.
fn far(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).abs().max((a[1] - b[1]).abs())
}

/// Assert that `read` holds exactly the parts of `expected`, in order,
/// within `tolerance` pixels: a stroke's ends, written width (the raster's
/// floor of one pixel) and, for a sketch stroke, its ends within the
/// wobble `amplitude`; a rectangle's or triangle's corners; a text block's
/// transform from its origin and rotation; each glyph pen and text rule.
pub fn assert_parts_match(
    read: &[ReadPart],
    expected: &[WrittenPart],
    tolerance: f32,
    amplitude: f32,
    context: &str,
) {
    assert_eq!(
        read.len(),
        expected.len(),
        "{context}: {} parts read, {} written; first difference at {:?}",
        read.len(),
        expected.len(),
        read.iter()
            .zip(expected)
            .position(|(r, e)| std::mem::discriminant(r) != kind_of(e))
    );
    for (i, (r, e)) in read.iter().zip(expected).enumerate() {
        let fail = || panic!("{context}: part {i}: read {r:?}, written {e:?}");
        let near = |a: f32, b: f32| (a - b).abs() <= tolerance;
        match (r, e) {
            (ReadPart::Part(read, width), WrittenPart::Part(written)) => match (read, written) {
                (
                    DrawnPart::Segment { a, b, .. },
                    DrawnPart::Segment {
                        a: ea,
                        b: eb,
                        width: ew,
                    },
                ) => {
                    if far(*a, *ea) > tolerance
                        || far(*b, *eb) > tolerance
                        || !width.is_some_and(|w| near(w, ew.max(1.0)))
                    {
                        fail();
                    }
                }
                (DrawnPart::Rect(a), DrawnPart::Rect(b)) => {
                    if !(near(a.x, b.x)
                        && near(a.y, b.y)
                        && near(a.width, b.width)
                        && near(a.height, b.height))
                    {
                        fail();
                    }
                }
                (DrawnPart::Triangle(a), DrawnPart::Triangle(b)) => {
                    if a.iter().zip(b).any(|(p, q)| far(*p, *q) > tolerance) {
                        fail();
                    }
                }
                _ => fail(),
            },
            (
                ReadPart::Curve { first, last, width },
                WrittenPart::Part(DrawnPart::Segment { a, b, width: ew }),
            ) => {
                let reach = amplitude + tolerance;
                if far(*first, *a) > reach
                    || far(*last, *b) > reach
                    || *width < ew.max(1.4) - tolerance
                {
                    fail();
                }
            }
            (ReadPart::Curve { width, .. }, WrittenPart::Part(DrawnPart::Rect(_))) => {
                // A sketched rectangle outline: one closed wobble.
                if *width < 1.4 - tolerance {
                    fail();
                }
            }
            (ReadPart::Text(t), WrittenPart::Text { origin, rotation }) => {
                let want = block_transform(*origin, *rotation);
                if t[..4]
                    .iter()
                    .zip(&want[..4])
                    .any(|(a, b)| (a - b).abs() > 1e-3)
                    || !near(t[4], want[4])
                    || !near(t[5], want[5])
                {
                    fail();
                }
            }
            (ReadPart::Glyph(p), WrittenPart::Glyph(q)) => {
                if far(*p, *q) > tolerance {
                    fail();
                }
            }
            (ReadPart::TextRect(a), WrittenPart::TextRect(b)) => {
                if !(near(a.x, b.x)
                    && near(a.y, b.y)
                    && near(a.width, b.width)
                    && near(a.height, b.height))
                {
                    fail();
                }
            }
            (ReadPart::TextEnd, WrittenPart::TextEnd) => {}
            (ReadPart::Image(a), WrittenPart::Image(b)) => {
                if a != b {
                    fail();
                }
            }
            _ => fail(),
        }
    }
}

/// The read-back kind a written part comes back as (a sketch stroke reads
/// as a curve; the first mismatch report only).
fn kind_of(e: &WrittenPart) -> std::mem::Discriminant<ReadPart> {
    std::mem::discriminant(&match e {
        WrittenPart::Part(p) => ReadPart::Part(*p, None),
        WrittenPart::Text { .. } => ReadPart::Text([0.0; 6]),
        WrittenPart::Glyph(p) => ReadPart::Glyph(*p),
        WrittenPart::TextRect(r) => ReadPart::TextRect(*r),
        WrittenPart::TextEnd => ReadPart::TextEnd,
        WrittenPart::Image(r) => ReadPart::Image(*r),
    })
}
