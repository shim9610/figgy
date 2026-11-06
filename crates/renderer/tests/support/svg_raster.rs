//! A test-side rasterizer of the SVG the vector export writes (#234,
//! approved on #234 as item 11): it paints the file's own elements, in
//! document order, with tiny-skia, independently of the renderer's canvas
//! sink. The composite tests compare its picture of a file with the PNG
//! export of the same document, so the vector parts themselves, not a
//! raster of the series they come from, are what is checked.
//!
//! It reads the subset `vector_export/svg.rs` writes: `<clipPath>`
//! rectangles and the `<g clip-path>` groups that use them, `<path>`
//! elements with `fill`, `stroke`, `stroke-width`, `stroke-dasharray`,
//! `fill-opacity` / `stroke-opacity` and absolute `M`, `L`, `C`, `Z`
//! commands, text groups (`rotate(a cx cy)` then `translate(x y)`) whose
//! `<use>`s place glyph outlines from `<defs>`, and `<image>`s with a
//! base64 PNG. SVG's own defaults apply: butt caps and miter joins.

use std::collections::HashMap;
use tiny_skia::{
    FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Rect as SkRect, Stroke, StrokeDash,
    Transform,
};

/// One tag: its name and attributes, and whether it closes or is
/// self-closing.
struct Tag<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, &'a str)>,
    closing: bool,
    empty: bool,
}

impl<'a> Tag<'a> {
    fn get(&self, key: &str) -> Option<&'a str> {
        self.attrs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    fn num(&self, key: &str) -> f32 {
        self.get(key).map_or(0.0, |v| v.parse().expect("a number"))
    }
}

/// The tags of `svg` in order.
fn tags(svg: &str) -> Vec<Tag<'_>> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>').expect("a closed tag") + start;
        let body = &rest[start + 1..end];
        rest = &rest[end + 1..];
        if body.starts_with('?') || body.starts_with('!') {
            continue;
        }
        let closing = body.starts_with('/');
        let empty = body.ends_with('/');
        let body = body.trim_start_matches('/').trim_end_matches('/');
        let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
        let name = &body[..name_end];
        let mut attrs = Vec::new();
        let mut at = &body[name_end..];
        while let Some(eq) = at.find("=\"") {
            let key = at[..eq].trim();
            let value_start = eq + 2;
            let value_end = at[value_start..].find('"').expect("a quoted value") + value_start;
            attrs.push((key, &at[value_start..value_end]));
            at = &at[value_end + 1..];
        }
        out.push(Tag {
            name,
            attrs,
            closing,
            empty,
        });
    }
    out
}

/// The `#rrggbb` colour with an opacity, as straight RGBA.
fn color(hex: &str, opacity: Option<&str>) -> Option<[u8; 4]> {
    let hex = hex.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("a hex colour");
    let alpha = opacity.map_or(1.0, |o| o.parse::<f32>().expect("an opacity"));
    Some([byte(0), byte(2), byte(4), (alpha * 255.0).round() as u8])
}

/// The path data `d` (absolute `M`, `L`, `C`, `Z`).
fn path(d: &str) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let mut numbers = Vec::new();
    let mut command = ' ';
    let flush = |pb: &mut PathBuilder, command: char, n: &mut Vec<f32>| {
        match command {
            'M' => {
                for p in n.chunks_exact(2) {
                    pb.move_to(p[0], p[1]);
                }
            }
            'L' => {
                for p in n.chunks_exact(2) {
                    pb.line_to(p[0], p[1]);
                }
            }
            'C' => {
                for p in n.chunks_exact(6) {
                    pb.cubic_to(p[0], p[1], p[2], p[3], p[4], p[5]);
                }
            }
            'Z' => pb.close(),
            _ => {}
        }
        n.clear();
    };
    let mut token = String::new();
    let push_token = |token: &mut String, numbers: &mut Vec<f32>| {
        if !token.is_empty() {
            numbers.push(token.parse().expect("a path number"));
            token.clear();
        }
    };
    for ch in d.chars() {
        match ch {
            'M' | 'L' | 'C' | 'Z' => {
                push_token(&mut token, &mut numbers);
                flush(&mut pb, command, &mut numbers);
                command = ch;
                if ch == 'Z' {
                    flush(&mut pb, 'Z', &mut numbers);
                    command = ' ';
                }
            }
            ' ' | ',' => push_token(&mut token, &mut numbers),
            '-' if !token.is_empty() && !token.ends_with('e') => {
                push_token(&mut token, &mut numbers);
                token.push(ch);
            }
            _ => token.push(ch),
        }
    }
    push_token(&mut token, &mut numbers);
    flush(&mut pb, command, &mut numbers);
    pb.finish()
}

/// A text group's transform: `rotate(a cx cy) translate(x y)` or
/// `translate(x y)`.
fn group_transform(t: &str) -> Transform {
    let numbers = |s: &str| -> Vec<f32> {
        s.split(|c: char| c == ' ' || c == ',')
            .filter(|n| !n.is_empty())
            .map(|n| n.parse().expect("a transform number"))
            .collect()
    };
    let mut ts = Transform::identity();
    let mut rest = t;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim();
        let close = rest[open..].find(')').expect("a closed transform") + open;
        let n = numbers(&rest[open + 1..close]);
        ts = match name {
            "rotate" => ts.pre_concat(Transform::from_rotate_at(n[0], n[1], n[2])),
            "translate" => ts.pre_translate(n[0], n[1]),
            other => panic!("an unknown transform {other}"),
        };
        rest = &rest[close + 1..];
    }
    ts
}

/// A base64 decoder for the images' data URIs.
fn base64(data: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("a base64 character"),
        }
    };
    let mut out = Vec::new();
    for chunk in data.as_bytes().chunks(4) {
        let n = chunk.iter().filter(|&&c| c != b'=').count();
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= if c == b'=' { 0 } else { value(c) } << (18 - 6 * i);
        }
        out.extend_from_slice(&acc.to_be_bytes()[1..n]);
    }
    out
}

/// A PNG's pixels as a premultiplied pixmap.
fn png_pixmap(bytes: &[u8]) -> Pixmap {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().expect("a PNG");
    let mut rgba = vec![0; reader.output_buffer_size().expect("a PNG size")];
    let info = reader.next_frame(&mut rgba).expect("PNG pixels");
    let mut pixmap = Pixmap::new(info.width, info.height).expect("an image pixmap");
    for (px, c) in pixmap.pixels_mut().iter_mut().zip(rgba.chunks_exact(4)) {
        *px = tiny_skia::ColorU8::from_rgba(c[0], c[1], c[2], c[3]).premultiply();
    }
    pixmap
}

/// The clip mask of a rectangle.
fn rect_mask(width: u32, height: u32, rect: [f32; 4]) -> Mask {
    let mut mask = Mask::new(width, height).expect("a clip mask");
    if let Some(r) = SkRect::from_xywh(rect[0], rect[1], rect[2], rect[3]) {
        mask.fill_path(
            &PathBuilder::from_rect(r),
            FillRule::Winding,
            false,
            Transform::identity(),
        );
    }
    mask
}

/// Paint `svg` (written by the vector export) into straight RGBA of its
/// `viewBox` size, every element in document order.
pub fn rasterize(svg: &str) -> renderer::RasterImage {
    let tags = tags(svg);
    let root = tags.iter().find(|t| t.name == "svg").expect("an <svg>");
    let view: Vec<f32> = root
        .get("viewBox")
        .expect("a viewBox")
        .split(' ')
        .map(|n| n.parse().expect("a viewBox number"))
        .collect();
    let (width, height) = (view[2] as u32, view[3] as u32);
    let mut pixmap = Pixmap::new(width, height).expect("a pixmap");
    let mut glyphs: HashMap<&str, tiny_skia::Path> = HashMap::new();
    let mut clip_rects: HashMap<String, [f32; 4]> = HashMap::new();
    // The open groups: a clip (its mask) or a text transform.
    enum Group {
        Clip(Mask),
        Text(Transform),
        Other,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut in_defs = false;
    let mut clip_id: Option<String> = None;
    for tag in &tags {
        let mask = groups.iter().rev().find_map(|g| match g {
            Group::Clip(m) => Some(m),
            _ => None,
        });
        let text = groups.iter().rev().find_map(|g| match g {
            Group::Text(t) => Some(*t),
            _ => None,
        });
        match (tag.name, tag.closing) {
            ("defs", false) => in_defs = !tag.empty,
            ("defs", true) => in_defs = false,
            ("clipPath", false) => clip_id = tag.get("id").map(str::to_string),
            ("clipPath", true) => clip_id = None,
            ("rect", false) if clip_id.is_some() => {
                let id = clip_id.clone().expect("a clip id");
                clip_rects.insert(
                    id,
                    [
                        tag.num("x"),
                        tag.num("y"),
                        tag.num("width"),
                        tag.num("height"),
                    ],
                );
            }
            ("g", false) => {
                let group = if let Some(clip) = tag.get("clip-path") {
                    let id = clip.trim_start_matches("url(#").trim_end_matches(')');
                    let rect = clip_rects[id];
                    Group::Clip(rect_mask(width, height, rect))
                } else if let Some(t) = tag.get("transform") {
                    Group::Text(group_transform(t))
                } else {
                    Group::Other
                };
                if !tag.empty {
                    groups.push(group);
                }
            }
            ("g", true) => {
                groups.pop();
            }
            ("path", false) if in_defs => {
                if let (Some(id), Some(p)) = (tag.get("id"), tag.get("d").and_then(path)) {
                    glyphs.insert(id, p);
                }
            }
            ("path", false) => {
                let Some(p) = tag.get("d").and_then(path) else {
                    continue;
                };
                let mut paint = Paint {
                    anti_alias: true,
                    ..Paint::default()
                };
                match tag.get("stroke") {
                    Some(stroke) if tag.get("fill") == Some("none") => {
                        let Some(c) = color(stroke, tag.get("stroke-opacity")) else {
                            continue;
                        };
                        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
                        let dash = tag.get("stroke-dasharray").and_then(|d| {
                            let v: Vec<f32> =
                                d.split(' ').map(|n| n.parse().expect("a dash")).collect();
                            StrokeDash::new(v, 0.0)
                        });
                        let stroke = Stroke {
                            width: tag.num("stroke-width"),
                            dash,
                            ..Stroke::default()
                        };
                        pixmap.stroke_path(&p, &paint, &stroke, Transform::identity(), mask);
                    }
                    _ => {
                        let Some(c) = tag
                            .get("fill")
                            .and_then(|f| color(f, tag.get("fill-opacity")))
                        else {
                            continue;
                        };
                        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
                        pixmap.fill_path(
                            &p,
                            &paint,
                            FillRule::Winding,
                            Transform::identity(),
                            mask,
                        );
                    }
                }
            }
            ("use", false) => {
                let id = tag
                    .get("xlink:href")
                    .expect("a glyph reference")
                    .trim_start_matches('#');
                let Some(glyph) = glyphs.get(id) else {
                    continue;
                };
                let Some(c) = tag
                    .get("fill")
                    .and_then(|f| color(f, tag.get("fill-opacity")))
                else {
                    continue;
                };
                let mut paint = Paint {
                    anti_alias: true,
                    ..Paint::default()
                };
                paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
                let at = text
                    .unwrap_or_default()
                    .pre_translate(tag.num("x"), tag.num("y"));
                pixmap.fill_path(glyph, &paint, FillRule::Winding, at, mask);
            }
            ("image", false) => {
                let href = tag.get("xlink:href").expect("an image source");
                let data = href
                    .strip_prefix("data:image/png;base64,")
                    .expect("a PNG data URI");
                let image = png_pixmap(&base64(data));
                pixmap.draw_pixmap(
                    tag.num("x") as i32,
                    tag.num("y") as i32,
                    image.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    mask,
                );
            }
            _ => {}
        }
    }
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for px in pixmap.pixels() {
        let c = px.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    renderer::RasterImage {
        width,
        height,
        rgba,
    }
}

/// Whether two pictures of one document are the same within `reach`
/// pixels: every inked pixel (alpha over half) of each has a painted pixel
/// (alpha over 16/255) of the other within `reach` pixels, and inside a
/// paint (a pixel whose eight neighbours both pictures cover fully, the
/// PNG's in one colour) their colours agree within `tolerance` per
/// channel. An independent rasterizer's antialiasing, its
/// unhinted glyph outlines against the renderer's hinted glyph raster (a
/// one-pixel stem can land as two faint rows) and SVG's butt caps differ
/// from the PNG by a pixel or so at edges; a missing, misplaced or
/// miscoloured part does not pass.
pub fn assert_same_picture(
    png: &renderer::RasterImage,
    file: &renderer::RasterImage,
    reach: u32,
    tolerance: u8,
    context: &str,
) {
    assert_eq!(
        (png.width, png.height),
        (file.width, file.height),
        "{context}: size"
    );
    let (w, h) = (png.width, png.height);
    if let Ok(dir) = std::env::var("FIGGY_PICTURE_DUMP") {
        let name: String = context
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        for (img, which) in [(png, "png"), (file, "file")] {
            let bytes = renderer::encode_png(img).expect("a PNG");
            std::fs::write(format!("{dir}/{name}.{which}.png"), bytes).expect("a dump");
        }
    }
    let inked = |img: &renderer::RasterImage, x: u32, y: u32| {
        img.rgba[((y * w + x) * 4 + 3) as usize] > 127
    };
    let painted =
        |img: &renderer::RasterImage, x: u32, y: u32| img.rgba[((y * w + x) * 4 + 3) as usize] > 16;
    let near = |img: &renderer::RasterImage, x: u32, y: u32, reach: u32| {
        (y.saturating_sub(reach)..=(y + reach).min(h - 1)).any(|py| {
            (x.saturating_sub(reach)..=(x + reach).min(w - 1)).any(|px| painted(img, px, py))
        })
    };
    // The inside of one paint: the pixel and its eight neighbours fully
    // covered in both pictures, the PNG's nine of one colour. A covered
    // pixel where two paints' edges meet mixes them by each rasterizer's
    // own coverage, so only an inside has one right colour.
    let inside = |x: u32, y: u32| {
        if x == 0 || y == 0 || x + 1 >= w || y + 1 >= h {
            return false;
        }
        let at = |px: u32, py: u32| ((py * w + px) * 4) as usize;
        let centre = at(x, y);
        (y - 1..=y + 1).all(|py| {
            (x - 1..=x + 1).all(|px| {
                let (i, j) = (at(px, py), at(px, py));
                png.rgba[i + 3] == 255
                    && file.rgba[j + 3] == 255
                    && (0..3).all(|k| png.rgba[i + k].abs_diff(png.rgba[centre + k]) <= tolerance)
            })
        })
    };
    // Counted, not stopped at the first: the PR reports every count, so a
    // small shift shows in the numbers before it fails.
    let (mut ink, mut compared) = (0usize, 0usize);
    let (mut outside, mut beyond_one) = (0usize, 0usize);
    let (mut off_colour, mut over_two) = (0usize, 0usize);
    let mut first: Option<String> = None;
    for y in 0..h {
        for x in 0..w {
            for (a, b, which) in [(png, file, "PNG"), (file, png, "file")] {
                if inked(a, x, y) {
                    if !near(b, x, y, reach) {
                        outside += 1;
                        first.get_or_insert_with(|| {
                            format!("the {which} inks ({x}, {y}) and the other paints nothing within {reach} px")
                        });
                    }
                    beyond_one += usize::from(!near(b, x, y, 1));
                }
            }
            ink += usize::from(inked(png, x, y));
            if inside(x, y) {
                let i = ((y * w + x) * 4) as usize;
                let d = (0..3)
                    .map(|k| png.rgba[i + k].abs_diff(file.rgba[i + k]))
                    .max()
                    .unwrap_or(0);
                if d > tolerance {
                    off_colour += 1;
                    first.get_or_insert_with(|| format!("the colour at ({x}, {y}) differs by {d}"));
                }
                over_two += usize::from(d > 2);
                compared += 1;
            }
        }
    }
    eprintln!(
        "picture {context}: {ink} inked px; outside {reach} px: {outside}, colour over {tolerance}/255: {off_colour} \
         (of {compared} compared); beyond 1 px: {beyond_one}, colour over 2/255: {over_two}"
    );
    assert!(
        outside == 0 && off_colour == 0,
        "{context}: {outside} px outside {reach} px, {off_colour} px over {tolerance}/255; first: {}",
        first.unwrap_or_default()
    );
    assert!(ink > 0, "{context}: the PNG draws nothing to compare");
    assert!(
        compared > 0,
        "{context}: no solid ink to compare colours on"
    );
}
