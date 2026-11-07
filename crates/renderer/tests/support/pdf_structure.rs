//! The PDF the vector export writes (#48), read back: the header, the
//! cross-reference table and every object at its offset, the trailer, the
//! object graph from the catalog to the page's content and resources, each
//! image XObject with its soft mask, and the inflated content stream's
//! operators as the [`ReadPart`]s of `svg_parts.rs`.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Read as _;

use renderer::layout::Rect;

use super::svg_parts::{ReadPart, Subpath, classify};

/// One object of the file: its dictionary text (or value) and its raw
/// stream.
pub struct Object {
    pub dict: String,
    pub stream: Option<Vec<u8>>,
}

/// An image XObject decoded: its size and straight-alpha RGBA from the
/// RGB image and its soft mask.
pub struct PdfImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct PdfFile {
    /// The MediaBox size in points.
    pub page: [f32; 2],
    pub objects: BTreeMap<u32, Object>,
    /// The inflated content stream.
    pub content: Vec<u8>,
    /// The page's images by resource name.
    pub images: BTreeMap<String, PdfImage>,
    /// The alpha bytes of the page's ExtGStates.
    pub alphas: Vec<u8>,
    /// Bytes of the file outside the image streams.
    pub other_bytes: usize,
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

/// The value after `key` in a dictionary text: up to the next `/`, `>>`
/// or `[` unless it starts one.
fn value<'a>(dict: &'a str, key: &str) -> Option<&'a str> {
    let at = dict.find(&format!("{key} "))? + key.len() + 1;
    let rest = &dict[at..];
    let end = rest
        .find(|c| c == '/' || c == '>' || c == ']')
        .unwrap_or(rest.len());
    Some(rest[..end].trim())
}

/// The object number of an indirect reference `n 0 R`.
fn reference(v: &str) -> u32 {
    let mut it = v.split(' ');
    let n = it.next().unwrap().parse().unwrap();
    assert_eq!((it.next(), it.next()), (Some("0"), Some("R")), "{v}");
    n
}

fn inflate(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .expect("a deflated stream");
    out
}

/// Undo the PNG predictors (`/Predictor 15`) of `rows` rows of `stride`
/// bytes with `bpp` bytes per pixel.
fn unpredict(data: &[u8], stride: usize, rows: usize, bpp: usize) -> Vec<u8> {
    assert_eq!(data.len(), rows * (stride + 1), "predicted rows");
    let mut out = vec![0u8; rows * stride];
    for y in 0..rows {
        let filter = data[y * (stride + 1)];
        for x in 0..stride {
            let raw = data[y * (stride + 1) + 1 + x];
            let a = if x >= bpp {
                out[y * stride + x - bpp]
            } else {
                0
            };
            let b = if y > 0 { out[(y - 1) * stride + x] } else { 0 };
            let c = if x >= bpp && y > 0 {
                out[(y - 1) * stride + x - bpp]
            } else {
                0
            };
            let predicted = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => {
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
                f => panic!("PNG filter {f}"),
            };
            out[y * stride + x] = raw.wrapping_add(predicted);
        }
    }
    out
}

/// Read `pdf`, asserting its structure.
pub fn read(pdf: &[u8]) -> PdfFile {
    assert!(pdf.starts_with(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n"), "header");
    assert!(pdf.ends_with(b"%%EOF\n"), "end of file marker");
    let tail = find(pdf, b"startxref\n", 0).expect("startxref") + 10;
    let digits = pdf[tail..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    let xref: usize = std::str::from_utf8(&pdf[tail..tail + digits])
        .unwrap()
        .parse()
        .unwrap();
    let table = std::str::from_utf8(&pdf[xref..tail - 10]).expect("an ASCII table and trailer");
    let mut lines = table.lines();
    assert_eq!(lines.next(), Some("xref"));
    let count: usize = lines
        .next()
        .and_then(|l| l.strip_prefix("0 "))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(lines.next(), Some("0000000000 65535 f "));
    let mut offsets = Vec::new();
    for _ in 1..count {
        let line = lines.next().unwrap();
        assert!(
            line.len() == 19 && line.ends_with(" 00000 n "),
            "entry {line:?}"
        );
        offsets.push(line[..10].parse::<usize>().unwrap());
    }
    assert_eq!(lines.next(), Some("trailer"));
    let trailer = lines.next().unwrap();
    assert_eq!(
        value(trailer, "/Size").unwrap().parse::<usize>().unwrap(),
        count
    );
    let root = reference(value(trailer, "/Root").unwrap());

    // Every object at its offset: a dictionary with an optional stream, or
    // a bare value (a stream's length).
    let mut objects = BTreeMap::new();
    let mut lengths = BTreeMap::new();
    for (i, &offset) in offsets.iter().enumerate() {
        let number = i as u32 + 1;
        let head = format!("{number} 0 obj\n");
        assert!(
            pdf[offset..].starts_with(head.as_bytes()),
            "object {number}"
        );
        let body = offset + head.len();
        let end = find(pdf, b"endobj\n", body).unwrap();
        if pdf[body..].starts_with(b"<<") {
            // The dictionary ends where its nesting closes.
            let mut depth = 0;
            let mut j = body;
            loop {
                if pdf[j..].starts_with(b"<<") {
                    depth += 1;
                    j += 2;
                } else if pdf[j..].starts_with(b">>") {
                    depth -= 1;
                    j += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            let dict = std::str::from_utf8(&pdf[body..j]).unwrap().to_owned();
            let stream = if pdf[j..].starts_with(b"\nstream\n") {
                let start = j + 8;
                let length = reference(value(&dict, "/Length").unwrap());
                lengths.insert(number, (length, start));
                Some(start)
            } else {
                assert_eq!(&pdf[j..j + 8], b"\nendobj\n", "object {number}");
                None
            };
            objects.insert(number, (dict, stream));
        } else {
            let text = std::str::from_utf8(&pdf[body..end])
                .unwrap()
                .trim()
                .to_owned();
            objects.insert(number, (text, None));
        }
    }
    let objects: BTreeMap<u32, Object> = objects
        .iter()
        .map(|(number, (dict, stream))| {
            let stream = stream.map(|start| {
                let (length_object, _) = lengths[number];
                let length: usize = objects[&length_object].0.parse().unwrap();
                assert!(
                    pdf[start + length..].starts_with(b"\nendstream\nendobj\n"),
                    "object {number}: stream length"
                );
                pdf[start..start + length].to_vec()
            });
            (
                *number,
                Object {
                    dict: dict.clone(),
                    stream,
                },
            )
        })
        .collect();

    // The graph: catalog, pages, the page, its content and resources.
    let catalog = &objects[&root].dict;
    assert!(catalog.contains("/Type /Catalog"));
    let pages_number = reference(value(catalog, "/Pages").unwrap());
    let pages = &objects[&pages_number].dict;
    assert!(pages.contains("/Type /Pages") && pages.contains("/Count 1"));
    let kids = &pages[pages.find("/Kids [").unwrap() + 7..];
    let page_number = reference(&kids[..kids.find(']').unwrap()]);
    let page = &objects[&page_number].dict;
    assert!(page.contains("/Type /Page "));
    assert_eq!(reference(value(page, "/Parent").unwrap()), pages_number);
    let media = &page[page.find("/MediaBox [0 0 ").unwrap() + 15..];
    let media: Vec<f32> = media[..media.find(']').unwrap()]
        .split(' ')
        .filter(|v| !v.is_empty())
        .map(|v| v.parse().unwrap())
        .collect();
    let content_number = reference(value(page, "/Contents").unwrap());
    let content = inflate(objects[&content_number].stream.as_ref().unwrap());
    assert!(
        objects[&content_number]
            .dict
            .contains("/Filter /FlateDecode")
    );

    // The images: an RGB image and its DeviceGray soft mask of one size.
    let xobjects = &page[page.find("/XObject <<").unwrap() + 11..];
    let xobjects = &xobjects[..xobjects.find(">>").unwrap()];
    let mut images = BTreeMap::new();
    let mut image_bytes = 0;
    let mut names = xobjects.split('/').filter(|s| !s.trim().is_empty());
    for entry in names.by_ref() {
        let (name, target) = entry.trim().split_once(' ').unwrap();
        let image = &objects[&reference(target)];
        let mask_number = reference(value(&image.dict, "/SMask").expect("a soft mask"));
        let mask = &objects[&mask_number];
        let int = |dict: &str, key: &str| value(dict, key).unwrap().parse::<u32>().unwrap();
        let (w, h) = (int(&image.dict, "/Width"), int(&image.dict, "/Height"));
        assert_eq!(
            (int(&mask.dict, "/Width"), int(&mask.dict, "/Height")),
            (w, h)
        );
        assert!(
            image.dict.contains("/Subtype /Image") && image.dict.contains("/ColorSpace /DeviceRGB")
        );
        assert!(
            mask.dict.contains("/Subtype /Image") && mask.dict.contains("/ColorSpace /DeviceGray")
        );
        for (dict, colors) in [(&image.dict, 3), (&mask.dict, 1)] {
            assert!(dict.contains(&format!(
                "/DecodeParms << /Predictor 15 /Colors {colors} /BitsPerComponent 8 /Columns {w} >>"
            )));
        }
        let raw = image.stream.as_ref().unwrap();
        let raw_mask = mask.stream.as_ref().unwrap();
        image_bytes += raw.len() + raw_mask.len();
        let rgb = unpredict(&inflate(raw), w as usize * 3, h as usize, 3);
        let alpha = unpredict(&inflate(raw_mask), w as usize, h as usize, 1);
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for (px, a) in rgb.chunks_exact(3).zip(&alpha) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], *a]);
        }
        images.insert(
            name.to_owned(),
            PdfImage {
                width: w,
                height: h,
                rgba,
            },
        );
    }

    // One ExtGState per alpha byte, its fill and stroke alpha that byte.
    let mut alphas = Vec::new();
    if let Some(at) = page.find("/ExtGState <<") {
        let mut rest = &page[at + 13..];
        while let Some(start) = rest.find("/A") {
            let name_end = start + rest[start..].find(' ').unwrap();
            let byte: u8 = rest[start + 2..name_end].parse().unwrap();
            let dict_end = name_end + rest[name_end..].find(">>").unwrap();
            let dict = &rest[name_end..dict_end];
            let want = byte as f32 / 255.0;
            for key in ["/ca", "/CA"] {
                let got: f32 = value(dict, key).unwrap().parse().unwrap();
                assert!((got - want).abs() < 6e-4, "alpha {byte}: {key} {got}");
            }
            alphas.push(byte);
            rest = &rest[dict_end + 2..];
        }
    }
    PdfFile {
        page: [media[0], media[1]],
        objects,
        content,
        images,
        alphas,
        other_bytes: pdf.len() - image_bytes,
    }
}

/// What a saved graphics state of the content stream is.
#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    /// Not known yet: the operators after `q` tell.
    Open,
    Clip,
    Image,
    Text,
    Glyph,
}

/// Interpret `file`'s content operators into the parts they paint, in
/// order, and check the stream's own rules: the page transform first,
/// balanced saves, every `gs` naming one of the page's ExtGStates and every
/// `Do` one of its images, drawn at its pixel size.
pub fn content_parts(file: &PdfFile, size: (u32, u32)) -> Vec<ReadPart> {
    let text = std::str::from_utf8(&file.content).expect("an ASCII content stream");
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut operands: Vec<f32> = Vec::new();
    let mut name: Option<&str> = None;
    let mut states: Vec<State> = Vec::new();
    let mut parts = Vec::new();
    let mut subs: Vec<Subpath> = Vec::new();
    let mut width = 1.0f32;
    let mut first_cm = true;
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        i += 1;
        if let Ok(v) = t.parse::<f32>() {
            operands.push(v);
            continue;
        }
        if let Some(n) = t.strip_prefix('/') {
            name = Some(n);
            continue;
        }
        let in_text = states.contains(&State::Text);
        let in_glyph = states.contains(&State::Glyph);
        match t {
            "cm" => {
                let m: [f32; 6] = operands[..].try_into().expect("six operands");
                if first_cm {
                    // Pixels, y down, onto the page.
                    first_cm = false;
                    let want = [
                        file.page[0] / size.0 as f32,
                        0.0,
                        0.0,
                        -file.page[1] / size.1 as f32,
                        0.0,
                        file.page[1],
                    ];
                    for (a, b) in m.iter().zip(want) {
                        assert!((a - b).abs() < 1e-3, "page transform {m:?}");
                    }
                } else {
                    let top = states.last_mut().expect("cm inside a saved state");
                    match *top {
                        State::Open if tokens.get(i + 1) == Some(&"Do") => {
                            *top = State::Image;
                            let (w, h) = (m[0], -m[3]);
                            assert!(m[1] == 0.0 && m[2] == 0.0 && w > 0.0 && h > 0.0);
                            parts.push(ReadPart::Image(Rect {
                                x: m[4] as u32,
                                y: (m[5] - h) as u32,
                                width: w as u32,
                                height: h as u32,
                            }));
                        }
                        State::Open if in_text => {
                            *top = State::Glyph;
                            assert_eq!(&m[..4], &[1.0, 0.0, 0.0, 1.0]);
                            parts.push(ReadPart::Glyph([m[4], m[5]]));
                        }
                        State::Open => {
                            *top = State::Text;
                            // An optional rotation, then the translation.
                            let mut block = m;
                            if m[..4] != [1.0, 0.0, 0.0, 1.0] {
                                let n: Vec<f32> = tokens[i..i + 6]
                                    .iter()
                                    .map(|v| v.parse().unwrap())
                                    .collect();
                                assert_eq!(tokens[i + 6], "cm");
                                assert_eq!(&n[..4], &[1.0, 0.0, 0.0, 1.0]);
                                i += 7;
                                // The rotation after the translation.
                                block[4] = m[0] * n[4] + m[2] * n[5] + m[4];
                                block[5] = m[1] * n[4] + m[3] * n[5] + m[5];
                            }
                            parts.push(ReadPart::Text(block));
                        }
                        s => panic!("cm in a {s:?} state"),
                    }
                }
            }
            "q" => states.push(State::Open),
            "Q" => {
                if states.pop().expect("balanced saves") == State::Text {
                    parts.push(ReadPart::TextEnd);
                }
            }
            "re" => {
                assert_eq!(tokens.get(i), Some(&"W"));
                assert_eq!(tokens.get(i + 1), Some(&"n"));
                i += 2;
                let top = states.last_mut().unwrap();
                assert_eq!(*top, State::Open, "a clip opens its own state");
                *top = State::Clip;
            }
            "gs" => {
                let n = name.take().expect("a named ExtGState");
                let byte: u8 = n.strip_prefix('A').unwrap().parse().unwrap();
                assert!(file.alphas.contains(&byte), "undefined /{n}");
            }
            "Do" => {
                let n = name.take().expect("a named image");
                let image = &file.images[n];
                let Some(ReadPart::Image(r)) = parts.last() else {
                    panic!("Do without its placement")
                };
                assert_eq!((r.width, r.height), (image.width, image.height));
            }
            "rg" | "RG" => {}
            "w" => width = operands[0],
            "d" => {}
            "M" => assert_eq!(operands, [4.0], "tiny-skia's miter limit"),
            "m" => subs.push(Subpath {
                points: vec![[operands[0], operands[1]]],
                ..Subpath::default()
            }),
            "l" => subs
                .last_mut()
                .unwrap()
                .points
                .push([operands[0], operands[1]]),
            "c" => {
                let s = subs.last_mut().unwrap();
                s.points.push([operands[4], operands[5]]);
                s.curved = true;
            }
            "h" => subs.last_mut().unwrap().closed = true,
            "f" | "S" => {
                let stroke = (t == "S").then_some(width);
                if !in_glyph {
                    for sub in &subs {
                        parts.push(classify(sub, stroke, in_text));
                    }
                }
                subs.clear();
            }
            "[" | "]" | "[]" => {
                // A dash array: its numbers are operands of `d`.
                continue;
            }
            other if other.starts_with('[') || other.ends_with(']') => {
                // `[a` or `b]` of a dash array.
                if let Ok(v) = other.trim_matches(|c| c == '[' || c == ']').parse::<f32>() {
                    operands.push(v);
                }
                continue;
            }
            other => panic!("unexpected operator {other}"),
        }
        operands.clear();
    }
    assert!(states.is_empty(), "unbalanced saves");
    parts
}
