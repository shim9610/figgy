//! Pure text rendering onto a skia `Canvas`.
//!
//! No window or event-loop dependency — the caller passes in a `Canvas` and
//! we draw into it. Liberation Sans (SIL OFL 1.1) is bundled in four styles,
//! so rendering always succeeds even on systems without the requested font:
//! if `RichText.font` is empty or unresolvable, the bundled font is used.

use std::sync::OnceLock;

use skia_safe::{Canvas, Color4f, Font, FontMgr, FontStyle, Paint, Rect, Typeface};

use crate::color::Color;
use crate::text::{greek_char, RichSegment, RichText};

// Bundled fonts (Liberation Sans, SIL OFL 1.1 — see fonts/LICENSE-LiberationSans.txt).

static EMBEDDED_REGULAR: &[u8] =
    include_bytes!("../fonts/LiberationSans-Regular.ttf");
static EMBEDDED_BOLD: &[u8] =
    include_bytes!("../fonts/LiberationSans-Bold.ttf");
static EMBEDDED_ITALIC: &[u8] =
    include_bytes!("../fonts/LiberationSans-Italic.ttf");
static EMBEDDED_BOLD_ITALIC: &[u8] =
    include_bytes!("../fonts/LiberationSans-BoldItalic.ttf");

static TF_REGULAR: OnceLock<Typeface> = OnceLock::new();
static TF_BOLD: OnceLock<Typeface> = OnceLock::new();
static TF_ITALIC: OnceLock<Typeface> = OnceLock::new();
static TF_BOLD_ITALIC: OnceLock<Typeface> = OnceLock::new();

fn load_typeface(bytes: &'static [u8]) -> Typeface {
    FontMgr::new()
        .new_from_data(bytes, None)
        .expect("corrupted bundled font")
}

fn embedded_typeface(bold: bool, italic: bool) -> Typeface {
    let (cell, bytes) = match (bold, italic) {
        (false, false) => (&TF_REGULAR, EMBEDDED_REGULAR),
        (true, false) => (&TF_BOLD, EMBEDDED_BOLD),
        (false, true) => (&TF_ITALIC, EMBEDDED_ITALIC),
        (true, true) => (&TF_BOLD_ITALIC, EMBEDDED_BOLD_ITALIC),
    };
    cell.get_or_init(|| load_typeface(bytes)).clone()
}

/// Resolve a typeface, falling back to the bundled font. Always succeeds.
/// 1) If `family` is non-empty, query the system FontMgr.
/// 2) Otherwise (or on miss), return the bundled Liberation Sans variant.
fn resolve_typeface_by(family: &str, bold: bool, italic: bool) -> Typeface {
    if !family.is_empty() {
        let fm = FontMgr::new();
        let style = match (bold, italic) {
            (false, false) => FontStyle::normal(),
            (true, false) => FontStyle::bold(),
            (false, true) => FontStyle::italic(),
            (true, true) => FontStyle::bold_italic(),
        };
        if let Some(tf) = fm.match_family_style(family, style) {
            return tf;
        }
    }
    embedded_typeface(bold, italic)
}

fn resolve_typeface(rt_font: &str, seg: &RichSegment) -> Typeface {
    resolve_typeface_by(rt_font, seg.bold, seg.italic)
}

// Rendering coefficients.

const SUB_SUPER_SIZE_RATIO: f32 = 0.65;
const SUPERSCRIPT_Y_RATIO: f32 = -0.5; // shift up from baseline
const SUBSCRIPT_Y_RATIO: f32 = 0.25; // shift down from baseline
const UNDERLINE_THICKNESS_RATIO: f32 = 0.06;
const UNDERLINE_Y_OFFSET_RATIO: f32 = 0.1; // below baseline
/// Horizontal kerning gap inserted before a super/sub segment, as a fraction
/// of the base font_size. Prevents visual collision from italic overhang or
/// size mismatch with the preceding glyph.
const SUB_SUPER_KERN_RATIO: f32 = 0.08;

// Public rendering API.

/// Draw `rt` at `origin` (left edge, baseline). The caller is responsible for
/// measuring and alignment — this function does not align.
pub fn draw_rich_text(canvas: &Canvas, rt: &RichText, origin: (f32, f32)) {
    let (mut pen_x, baseline) = origin;
    let paint = make_paint(&rt.color);

    for seg in &rt.segments {
        let size = segment_font_size(seg, rt.font_size);
        let tf = resolve_typeface(&rt.font, seg);
        let font = Font::from_typeface(tf, Some(size));

        let ch = if seg.greek {
            greek_char(seg.text)
        } else {
            seg.text
        };
        let mut buf = [0u8; 4];
        let s: &str = ch.encode_utf8(&mut buf);

        pen_x += segment_kern_before(seg, rt.font_size);

        let y = baseline + segment_y_offset(seg, rt.font_size);
        let (advance, _bounds) = font.measure_str(s, Some(&paint));
        canvas.draw_str(s, (pen_x, y), &font, &paint);

        if seg.underline {
            let uy = y + size * UNDERLINE_Y_OFFSET_RATIO;
            let uh = (size * UNDERLINE_THICKNESS_RATIO).max(1.0);
            canvas.draw_rect(Rect::from_xywh(pen_x, uy, advance, uh), &paint);
        }

        pen_x += advance;
    }
}

/// Measurement result returned by [`measure_rich_text`].
///
/// - `width`: total advance (sum of per-segment skia `measure_str` advances).
/// - `ascent`: largest distance above baseline (positive).
/// - `descent`: largest distance below baseline (positive); includes super/sub
///   y-offset and the bottom of any underline bar.
///
/// `height() = ascent + descent` is the vertical envelope across all segments.
#[derive(Debug, Clone, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

impl TextMetrics {
    pub fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// Compute the real rendered bounding box of `rt` using skia measurements.
/// Mirrors `draw_rich_text` for segment handling, styles, sub/superscripts,
/// and underline.
pub fn measure_rich_text(rt: &RichText) -> TextMetrics {
    let mut total_w = 0.0f32;
    let mut max_ascent = 0.0f32;
    let mut max_descent = 0.0f32;

    for seg in &rt.segments {
        let size = segment_font_size(seg, rt.font_size);
        let tf = resolve_typeface(&rt.font, seg);
        let font = Font::from_typeface(tf, Some(size));

        let ch = if seg.greek {
            greek_char(seg.text)
        } else {
            seg.text
        };
        let mut buf = [0u8; 4];
        let s: &str = ch.encode_utf8(&mut buf);

        // skia top-down coords: top<0 above baseline, bottom>0 below.
        let (advance, bounds) = font.measure_str(s, None);
        let y_offset = segment_y_offset(seg, rt.font_size);

        // Top/bottom of this segment relative to the overall baseline.
        let seg_top = y_offset + bounds.top;
        let mut seg_bottom = y_offset + bounds.bottom;

        if seg.underline {
            let underline_top = y_offset + size * UNDERLINE_Y_OFFSET_RATIO;
            let underline_thickness = (size * UNDERLINE_THICKNESS_RATIO).max(1.0);
            seg_bottom = seg_bottom.max(underline_top + underline_thickness);
        }

        // Convert ascent/descent to positive distances.
        max_ascent = max_ascent.max(-seg_top);
        max_descent = max_descent.max(seg_bottom);
        total_w += segment_kern_before(seg, rt.font_size) + advance;
    }

    TextMetrics {
        width: total_w,
        ascent: max_ascent.max(0.0),
        descent: max_descent.max(0.0),
    }
}

// Private helpers.

fn make_paint(color: &Color) -> Paint {
    let c4 = Color4f::new(color.r, color.g, color.b, color.a);
    let mut paint = Paint::new(c4, None);
    paint.set_anti_alias(true);
    paint
}


fn segment_font_size(seg: &RichSegment, base: f32) -> f32 {
    if seg.superscript || seg.subscript {
        base * SUB_SUPER_SIZE_RATIO
    } else {
        base
    }
}

fn segment_y_offset(seg: &RichSegment, base: f32) -> f32 {
    if seg.superscript {
        base * SUPERSCRIPT_Y_RATIO
    } else if seg.subscript {
        base * SUBSCRIPT_Y_RATIO
    } else {
        0.0
    }
}

/// Kerning gap added to pen_x just before drawing `seg`. Super/sub segments
/// differ in size and y-offset from the preceding glyph and need a small gap
/// to avoid visual overhang collisions.
fn segment_kern_before(seg: &RichSegment, base: f32) -> f32 {
    if seg.superscript || seg.subscript {
        base * SUB_SUPER_KERN_RATIO
    } else {
        0.0
    }
}

// Plain (single-style) text rendering — used for tick labels etc.
// Shares the same system/bundle fallback as RichText via `resolve_typeface_by`.

/// Draw single-style text at `origin` (left, baseline).
#[allow(clippy::too_many_arguments)]
pub fn draw_plain_text(
    canvas: &Canvas,
    text: &str,
    origin: (f32, f32),
    color: Color,
    family: &str,
    size: f32,
    bold: bool,
    italic: bool,
) {
    let paint = make_paint(&color);
    let tf = resolve_typeface_by(family, bold, italic);
    let font = Font::from_typeface(tf, Some(size));
    canvas.draw_str(text, origin, &font, &paint);
}

/// Return the advance width and measured vertical extent of single-style text.
pub fn measure_plain_text(
    text: &str,
    family: &str,
    size: f32,
    bold: bool,
    italic: bool,
) -> TextMetrics {
    let tf = resolve_typeface_by(family, bold, italic);
    let font = Font::from_typeface(tf, Some(size));
    let (advance, bounds) = font.measure_str(text, None);
    TextMetrics {
        width: advance,
        ascent: (-bounds.top).max(0.0),
        descent: bounds.bottom.max(0.0),
    }
}
