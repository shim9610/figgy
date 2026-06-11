//! Pure text rendering onto the CPU raster [`Canvas`](crate::raster::Canvas).
//!
//! No window or event-loop dependency — the caller passes in a canvas and we
//! draw into it. Liberation Sans (SIL OFL 1.1) is bundled in four styles, so
//! rendering always succeeds even on systems without the requested font: if
//! `RichText.font` is empty or unresolvable, the bundled font is used.
//!
//! Layering: the segment layout policy (super/subscript scaling and shifts,
//! kerning gaps, underline bars, greek mapping, the measure envelope) lives
//! here and is backend-independent. The glyph layer below it — metrics and
//! alpha-mask rasterization — is fontdb (font lookup) + swash (scaling), all
//! pure Rust, so the whole stack compiles for wasm32-unknown-unknown.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use swash::scale::{Render, ScaleContext, Source};
use swash::FontRef;

use crate::color::Color;
use crate::raster::Canvas;
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

fn embedded_bytes(bold: bool, italic: bool) -> &'static [u8] {
    match (bold, italic) {
        (false, false) => EMBEDDED_REGULAR,
        (true, false) => EMBEDDED_BOLD,
        (false, true) => EMBEDDED_ITALIC,
        (true, true) => EMBEDDED_BOLD_ITALIC,
    }
}

/// System font database (native targets only — on wasm there is no system to
/// scan; hosts register fonts at runtime via [`register_font_bytes`]).
#[cfg(not(target_arch = "wasm32"))]
fn font_db() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// Runtime-registered fonts — works on every target (fontdb itself compiles
/// on wasm; only the system *scan* is native-only). Registered families win
/// over system fonts so native and wasm resolve identically once a host has
/// registered its fonts.
fn registered_db() -> &'static Mutex<fontdb::Database> {
    static DB: OnceLock<Mutex<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| Mutex::new(fontdb::Database::new()))
}

/// Register a font (TTF/OTF/TTC bytes) for `RichText.font` family lookup.
/// Returns the family names the file declares, or an error when the bytes
/// parse to no usable face. Re-registering a family replaces nothing —
/// fontdb keeps both and the query picks the best style match.
pub fn register_font_bytes(bytes: Vec<u8>) -> Result<Vec<String>, String> {
    let mut db = registered_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let before: Vec<fontdb::ID> = db.faces().map(|f| f.id).collect();
    db.load_font_data(bytes);
    let mut families: Vec<String> = db
        .faces()
        .filter(|f| !before.contains(&f.id))
        .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
        .collect();
    families.dedup();
    if families.is_empty() {
        return Err("font data contains no usable face".to_string());
    }
    // A family that previously missed (and was cached as the bundled
    // fallback) may now resolve — drop the resolution cache. Glyph caches
    // key on each font's own CacheKey, so they stay valid.
    resolved_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    Ok(families)
}

fn lookup_registered_font(family: &str, bold: bool, italic: bool) -> Option<(&'static [u8], u32)> {
    let db = registered_db().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
        stretch: fontdb::Stretch::Normal,
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
    };
    let id = db.query(&query)?;
    db.with_face_data(id, |data, index| {
        (&*Box::leak(data.to_vec().into_boxed_slice()), index)
    })
}

/// Bundled fonts parsed exactly once — `FontRef::from_index` mints a fresh
/// `CacheKey` per call, and a stable key is what makes the glyph caches below
/// actually hit.
fn embedded_font(bold: bool, italic: bool) -> FontRef<'static> {
    static REGULAR: OnceLock<FontRef<'static>> = OnceLock::new();
    static BOLD: OnceLock<FontRef<'static>> = OnceLock::new();
    static ITALIC: OnceLock<FontRef<'static>> = OnceLock::new();
    static BOLD_ITALIC: OnceLock<FontRef<'static>> = OnceLock::new();
    let cell = match (bold, italic) {
        (false, false) => &REGULAR,
        (true, false) => &BOLD,
        (false, true) => &ITALIC,
        (true, true) => &BOLD_ITALIC,
    };
    *cell.get_or_init(|| {
        FontRef::from_index(embedded_bytes(bold, italic), 0).expect("corrupted bundled font")
    })
}

/// Resolved `FontRef` per (family, bold, italic). System faces are copied out
/// of fontdb once and leaked — font data lives for the process lifetime
/// anyway (the bundled statics set the precedent) — and the parsed `FontRef`
/// is cached so its `CacheKey` stays stable for the glyph caches.
fn resolved_cache() -> &'static Mutex<HashMap<(String, bool, bool), FontRef<'static>>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, bool, bool), FontRef<'static>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(not(target_arch = "wasm32"))]
fn lookup_system_font(family: &str, bold: bool, italic: bool) -> Option<(&'static [u8], u32)> {
    let db = font_db();
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
        stretch: fontdb::Stretch::Normal,
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
    };
    let id = db.query(&query)?;
    db.with_face_data(id, |data, index| {
        (&*Box::leak(data.to_vec().into_boxed_slice()), index)
    })
}

#[cfg(target_arch = "wasm32")]
fn lookup_system_font(_family: &str, _bold: bool, _italic: bool) -> Option<(&'static [u8], u32)> {
    None
}

/// Resolve a font, falling back to the bundled font. Always succeeds.
/// 1) If `family` is non-empty, query runtime-registered fonts (all targets),
///    then the system font database (native only).
/// 2) Otherwise (or on miss), return the bundled Liberation Sans variant.
fn resolve_font(family: &str, bold: bool, italic: bool) -> FontRef<'static> {
    if family.is_empty() {
        return embedded_font(bold, italic);
    }
    let key = (family.to_string(), bold, italic);
    if let Some(hit) = resolved_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
        return *hit;
    }
    let resolved = lookup_registered_font(family, bold, italic)
        .or_else(|| lookup_system_font(family, bold, italic))
        .and_then(|(bytes, index)| FontRef::from_index(bytes, index as usize))
        .unwrap_or_else(|| embedded_font(bold, italic));
    resolved_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key, resolved);
    resolved
}

// Glyph primitives (swash) — everything below is cached per glyph because the
// decoration layer re-rasterizes on every interaction frame. Without caching,
// each glyph re-parses font tables and rebuilds a hinting `Scaler` per frame,
// which is what dominated the raster cost (measured ~4 ms/frame for a
// labelled panel; cached it drops to table lookups + blits).

/// Per-glyph extents in skia's y-down baseline convention:
/// `ink_top` is negative above the baseline, `ink_bottom` positive below.
#[derive(Clone, Copy)]
struct GlyphExtents {
    advance: f32,
    ink_top: f32,
    ink_bottom: f32,
}

/// Rendered glyph alpha mask. `left`/`top` are the pen-relative placement
/// (top positive above the baseline, swash convention).
struct GlyphMask {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    data: Box<[u8]>,
}

type MetricsKey = (swash::CacheKey, u32, char);
/// Mask key: metrics key + horizontal subpixel bin (quarter-pixel phases).
type MaskKey = (swash::CacheKey, u32, char, u8);

thread_local! {
    static SCALE_CTX: RefCell<ScaleContext> = RefCell::new(ScaleContext::new());
    static METRICS_CACHE: RefCell<HashMap<MetricsKey, GlyphExtents>> =
        RefCell::new(HashMap::new());
    static MASK_CACHE: RefCell<HashMap<MaskKey, Option<GlyphMask>>> =
        RefCell::new(HashMap::new());
}

fn glyph_extents(font: FontRef<'static>, size: f32, ch: char) -> GlyphExtents {
    let key = (font.key, size.to_bits(), ch);
    if let Some(hit) = METRICS_CACHE.with(|c| c.borrow().get(&key).copied()) {
        return hit;
    }
    let glyph = font.charmap().map(ch);
    let advance = font.glyph_metrics(&[]).scale(size).advance_width(glyph);
    let (ink_top, ink_bottom) = SCALE_CTX.with(|cx| {
        let mut cx = cx.borrow_mut();
        let mut scaler = cx.builder(font).size(size).hint(true).build();
        match scaler.scale_outline(glyph) {
            // zeno bounds are y-up; flip into the y-down baseline convention.
            Some(outline) => {
                let b = outline.bounds();
                (-b.max.y, -b.min.y)
            }
            None => (0.0, 0.0),
        }
    });
    let extents = GlyphExtents { advance, ink_top, ink_bottom };
    METRICS_CACHE.with(|c| c.borrow_mut().insert(key, extents));
    extents
}

/// Rasterize (or fetch) the glyph mask for a quarter-pixel horizontal phase
/// and hand it to `use_mask`. AA happens exactly once, here in swash — the
/// subpixel offset is baked into the rasterization instead of resampling the
/// finished mask, which is what kept text crisp under skia.
fn with_glyph_mask(
    font: FontRef<'static>,
    size: f32,
    ch: char,
    xbin: u8,
    use_mask: impl FnOnce(&GlyphMask),
) {
    let key = (font.key, size.to_bits(), ch, xbin);
    MASK_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry = cache.entry(key).or_insert_with(|| {
            let glyph = font.charmap().map(ch);
            SCALE_CTX.with(|cx| {
                let mut cx = cx.borrow_mut();
                let mut scaler = cx.builder(font).size(size).hint(true).build();
                let mut render = Render::new(&[Source::Outline]);
                render.offset(swash::zeno::Vector::new(xbin as f32 * 0.25, 0.0));
                render.render(&mut scaler, glyph).and_then(|image| {
                    let p = image.placement;
                    if p.width == 0 || p.height == 0 {
                        return None;
                    }
                    Some(GlyphMask {
                        left: p.left,
                        top: p.top,
                        width: p.width,
                        height: p.height,
                        data: image.data.into_boxed_slice(),
                    })
                })
            })
        });
        if let Some(mask) = entry {
            use_mask(mask);
        }
    });
}

/// Rasterize one glyph and blit it with `color` at (`pen_x`, `baseline`).
/// Returns the advance.
fn draw_char(
    canvas: &mut Canvas,
    font: FontRef<'static>,
    size: f32,
    ch: char,
    pen_x: f32,
    baseline: f32,
    color: &Color,
) -> f32 {
    let advance = glyph_extents(font, size, ch).advance;

    match canvas.translation() {
        // Crisp path: snap to the pixel grid, carry the sub-pixel x phase
        // into the rasterizer, and blit without any filtering.
        Some((tx, ty)) => {
            let gx = pen_x + tx;
            let gy = (baseline + ty).round() as i32;
            let mut ix = gx.floor() as i32;
            let mut xbin = ((gx - gx.floor()) * 4.0).round() as u8;
            if xbin == 4 {
                xbin = 0;
                ix += 1;
            }
            with_glyph_mask(font, size, ch, xbin, |m| {
                canvas.blit_mask(ix + m.left, gy - m.top, m.width, m.height, &m.data, color);
            });
        }
        // Rotated text (axis titles): resampling through the transform is
        // unavoidable; render at phase 0.
        None => {
            with_glyph_mask(font, size, ch, 0, |m| {
                canvas.draw_mask(
                    pen_x + m.left as f32,
                    baseline - m.top as f32,
                    m.width,
                    m.height,
                    &m.data,
                    color,
                );
            });
        }
    }
    advance
}

// Rendering coefficients.

const SUB_SUPER_SIZE_RATIO: f32 = 0.65;
const SUPERSCRIPT_Y_RATIO: f32 = -0.5; // shift up from baseline
const SUBSCRIPT_Y_RATIO: f32 = 0.25; // shift down from baseline
const UNDERLINE_THICKNESS_RATIO: f32 = 0.06;
const UNDERLINE_Y_OFFSET_RATIO: f32 = 0.1; // below baseline
// Rule segments (legend line marks): bar center sits where dash glyphs put
// their ink (~0.3 em above the baseline) so rules and shape glyphs align.
const RULE_Y_RATIO: f32 = 0.3; // above baseline
const RULE_THICKNESS_RATIO: f32 = 0.09;
/// Horizontal kerning gap inserted before a super/sub segment, as a fraction
/// of the base font_size. Prevents visual collision from italic overhang or
/// size mismatch with the preceding glyph.
const SUB_SUPER_KERN_RATIO: f32 = 0.08;

// Line splitting — a `'\n'` segment ends the current line. Both drawing and
// measurement share this so multi-line text (legend labels, titles) lays out
// identically everywhere.

fn split_lines(rt: &RichText) -> Vec<&[RichSegment]> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, seg) in rt.segments.iter().enumerate() {
        if seg.text == '\n' {
            lines.push(&rt.segments[start..i]);
            start = i + 1;
        }
    }
    lines.push(&rt.segments[start..]);
    lines
}

/// Cells of one line, split at `'\t'` segments (the separators themselves
/// are never rendered). A line without tabs is one cell.
fn split_cells(line: &[RichSegment]) -> Vec<&[RichSegment]> {
    let mut cells = Vec::new();
    let mut start = 0;
    for (i, seg) in line.iter().enumerate() {
        if seg.text == '\t' {
            cells.push(&line[start..i]);
            start = i + 1;
        }
    }
    cells.push(&line[start..]);
    cells
}

/// Tab-column layout shared by measure + draw. `'\t'` behaves like a table
/// column separator: column k is as wide as the widest cell k across every
/// line of the document. This is what gives legends equal-width symbol
/// fields — labels behind `—` and `–●–` start at the same x.
struct DocLayout<'a> {
    line_cells: Vec<Vec<&'a [RichSegment]>>,
    /// Per line: ascent/descent = max over its cells, width = end of the
    /// last cell in column coordinates.
    line_metrics: Vec<TextMetrics>,
    col_starts: Vec<f32>,
}

fn layout_doc(rt: &RichText) -> DocLayout<'_> {
    let line_cells: Vec<Vec<&[RichSegment]>> =
        split_lines(rt).into_iter().map(split_cells).collect();

    let cell_metrics: Vec<Vec<TextMetrics>> = line_cells
        .iter()
        .map(|cells| cells.iter().map(|c| measure_line(c, rt)).collect())
        .collect();

    let n_cols = line_cells.iter().map(|c| c.len()).max().unwrap_or(1);
    let mut col_starts = Vec::with_capacity(n_cols);
    let mut acc = 0.0f32;
    for k in 0..n_cols {
        col_starts.push(acc);
        let col_w = cell_metrics
            .iter()
            .filter_map(|m| m.get(k))
            .map(|m| m.width)
            .fold(0.0, f32::max);
        acc += col_w;
    }

    let line_metrics = cell_metrics
        .iter()
        .map(|cells| {
            let last = cells.len().saturating_sub(1);
            let width = col_starts.get(last).copied().unwrap_or(0.0)
                + cells.last().map(|m| m.width).unwrap_or(0.0);
            let ascent = cells.iter().map(|m| m.ascent).fold(0.0, f32::max);
            let descent = cells.iter().map(|m| m.descent).fold(0.0, f32::max);
            TextMetrics { width, ascent, descent }
        })
        .collect();

    DocLayout { line_cells, line_metrics, col_starts }
}

/// Envelope of one line. Empty lines fall back to a font-size-derived height
/// so blank lines still advance the baseline.
fn measure_line(segments: &[RichSegment], rt: &RichText) -> TextMetrics {
    let mut total_w = 0.0f32;
    let mut max_ascent = 0.0f32;
    let mut max_descent = 0.0f32;

    for seg in segments {
        let base = segment_base_size(seg, rt);
        let size = segment_font_size(seg, base);
        let y_offset = segment_y_offset(seg, base);

        let (advance, seg_top, mut seg_bottom) = if seg.rule {
            // Drawn rule: a bar centered on the symbol axis. Width is the
            // fixed field; vertical extent is the bar thickness only.
            let field = seg.field_em.unwrap_or(1.0) * base;
            let t = (base * RULE_THICKNESS_RATIO).max(1.0);
            let center = y_offset - base * RULE_Y_RATIO;
            (field, center - t * 0.5, center + t * 0.5)
        } else {
            let font = resolve_font(&rt.font, seg.bold, seg.italic);
            let ch = if seg.greek { greek_char(seg.text) } else { seg.text };
            let g = glyph_extents(font, size, ch);
            // A fixed field replaces both the kern and the glyph advance.
            let adv = match seg.field_em {
                Some(f) => f * base,
                None => segment_kern_before(seg, base) + g.advance,
            };
            (adv, y_offset + g.ink_top, y_offset + g.ink_bottom)
        };

        if seg.underline {
            let underline_top = y_offset + size * UNDERLINE_Y_OFFSET_RATIO;
            let underline_thickness = (size * UNDERLINE_THICKNESS_RATIO).max(1.0);
            seg_bottom = seg_bottom.max(underline_top + underline_thickness);
        }

        max_ascent = max_ascent.max(-seg_top);
        max_descent = max_descent.max(seg_bottom);
        total_w += advance;
    }

    if segments.is_empty() || (max_ascent == 0.0 && max_descent == 0.0) {
        // Blank line: keep vertical rhythm from the nominal font size.
        max_ascent = rt.font_size * 0.75;
        max_descent = rt.font_size * 0.25;
    }
    TextMetrics { width: total_w, ascent: max_ascent, descent: max_descent }
}

/// Baseline-to-baseline advance between two lines. Ink envelopes alone make
/// descender-less lines collapse together, so clamp to a nominal line height
/// derived from the font size. Shared by draw + measure.
fn line_advance(prev: &TextMetrics, this: &TextMetrics, rt: &RichText) -> f32 {
    (prev.descent + this.ascent).max(rt.font_size * 1.15)
}

fn draw_line_segments(
    canvas: &mut Canvas,
    segments: &[RichSegment],
    rt: &RichText,
    origin: (f32, f32),
) {
    let (mut pen_x, baseline) = origin;
    for seg in segments {
        let base = segment_base_size(seg, rt);
        let size = segment_font_size(seg, base);
        let color = seg.color.unwrap_or(rt.color);
        let y = baseline + segment_y_offset(seg, base);

        let advance = if seg.rule {
            // Drawn rule filling its fixed field, centered on the symbol
            // axis — the exact-length line stroke of legend marks.
            let field = seg.field_em.unwrap_or(1.0) * base;
            let t = (base * RULE_THICKNESS_RATIO).max(1.0);
            let ry = y - base * RULE_Y_RATIO - t * 0.5;
            canvas.draw_rect(pen_x, ry, field, t, &crate::raster::Paint::fill(&color));
            field
        } else {
            let font = resolve_font(&rt.font, seg.bold, seg.italic);
            let ch = if seg.greek { greek_char(seg.text) } else { seg.text };
            match seg.field_em {
                Some(f) => {
                    // Center the glyph ink inside the fixed field.
                    let field = f * base;
                    let g = glyph_extents(font, size, ch);
                    let inset = (field - g.advance) * 0.5;
                    draw_char(canvas, font, size, ch, pen_x + inset, y, &color);
                    field
                }
                None => {
                    pen_x += segment_kern_before(seg, base);
                    draw_char(canvas, font, size, ch, pen_x, y, &color)
                }
            }
        };

        if seg.underline {
            let uy = y + size * UNDERLINE_Y_OFFSET_RATIO;
            let uh = (size * UNDERLINE_THICKNESS_RATIO).max(1.0);
            canvas.draw_rect(pen_x, uy, advance, uh, &crate::raster::Paint::fill(&color));
        }

        pen_x += advance;
    }
}

// Public rendering API.

/// Draw `rt` at `origin` (left edge, baseline **of the first line**). The
/// caller is responsible for measuring and alignment — this function does
/// not align. `'\n'` segments start a new line; subsequent baselines advance
/// by the previous line's descent + the next line's ascent.
pub fn draw_rich_text(canvas: &mut Canvas, rt: &RichText, origin: (f32, f32)) {
    let doc = layout_doc(rt);
    let (x, mut baseline) = origin;

    for (i, cells) in doc.line_cells.iter().enumerate() {
        if i > 0 {
            baseline += line_advance(&doc.line_metrics[i - 1], &doc.line_metrics[i], rt);
        }
        for (k, cell) in cells.iter().enumerate() {
            draw_line_segments(canvas, cell, rt, (x + doc.col_starts[k], baseline));
        }
    }
}

/// Measurement result returned by [`measure_rich_text`].
///
/// - `width`: widest line's total advance.
/// - `ascent`: first line's rise above its baseline (positive).
/// - `descent`: everything below the first baseline (positive) — includes
///   super/sub y-offsets, underline bars, and **all subsequent lines** when
///   the text contains `'\n'` segments.
///
/// `height() = ascent + descent` is the full vertical envelope, so block
/// centering math works unchanged for multi-line text.
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

/// CPU-raster implementation of the model crate's [`MeasureText`] contract.
///
/// Inject this wherever the model needs glyph extents — element bounds for
/// `Selectable::selection_box`, hit tests, etc. — so model-side geometry uses
/// the same measurements the renderer draws with.
pub struct CpuTextMeasure;

impl crate::text::MeasureText for CpuTextMeasure {
    fn measure_rich(&self, rt: &RichText) -> crate::text::TextExtents {
        let m = measure_rich_text(rt);
        crate::text::TextExtents {
            width: m.width,
            ascent: m.ascent,
            descent: m.descent,
        }
    }
}

/// Compute the real rendered bounding box of `rt` using glyph measurements.
/// Mirrors `draw_rich_text` for segment handling, styles, sub/superscripts,
/// underline, and `'\n'` line breaks.
pub fn measure_rich_text(rt: &RichText) -> TextMetrics {
    if rt.segments.is_empty() {
        return TextMetrics { width: 0.0, ascent: 0.0, descent: 0.0 };
    }

    let metrics = layout_doc(rt).line_metrics;
    let width = metrics.iter().map(|m| m.width).fold(0.0, f32::max);
    let ascent = metrics[0].ascent;
    // Everything below the first baseline: accumulate the same baseline
    // advances `draw_rich_text` uses, then the last line's own descent.
    let mut below = 0.0f32;
    for i in 1..metrics.len() {
        below += line_advance(&metrics[i - 1], &metrics[i], rt);
    }
    let descent = below + metrics.last().map(|m| m.descent).unwrap_or(0.0);

    TextMetrics { width, ascent, descent }
}

// Private helpers.

/// Effective base size of a segment: the per-segment `font_size` override
/// when present, the document-level `RichText.font_size` otherwise. The
/// sub/superscript ratio (and the derived y-offset / kerning) apply on top.
fn segment_base_size(seg: &RichSegment, rt: &RichText) -> f32 {
    seg.font_size.unwrap_or(rt.font_size)
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
// Shares the same system/bundle fallback as RichText via `resolve_font`.

/// Draw single-style text at `origin` (left, baseline).
#[allow(clippy::too_many_arguments)]
pub fn draw_plain_text(
    canvas: &mut Canvas,
    text: &str,
    origin: (f32, f32),
    color: Color,
    family: &str,
    size: f32,
    bold: bool,
    italic: bool,
) {
    let font = resolve_font(family, bold, italic);
    let (mut pen_x, baseline) = origin;
    for ch in text.chars() {
        pen_x += draw_char(canvas, font, size, ch, pen_x, baseline, &color);
    }
}

/// Return the advance width and measured vertical extent of single-style text.
pub fn measure_plain_text(
    text: &str,
    family: &str,
    size: f32,
    bold: bool,
    italic: bool,
) -> TextMetrics {
    let font = resolve_font(family, bold, italic);
    let mut width = 0.0f32;
    let mut max_ascent = 0.0f32;
    let mut max_descent = 0.0f32;
    for ch in text.chars() {
        let g = glyph_extents(font, size, ch);
        width += g.advance;
        max_ascent = max_ascent.max(-g.ink_top);
        max_descent = max_descent.max(g.ink_bottom);
    }
    TextMetrics { width, ascent: max_ascent, descent: max_descent }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::rich_segments_from_text;

    // The bundled-font glyph path must produce real ink: positive metrics
    // from measurement and non-transparent pixels from drawing.
    #[test]
    fn plain_text_measures_and_rasterizes_ink() {
        let m = measure_plain_text("123", "", 18.0, false, false);
        assert!(m.width > 10.0, "advance too small: {}", m.width);
        assert!(m.ascent > 5.0, "ascent too small: {}", m.ascent);

        let mut canvas = Canvas::new(60, 30).unwrap();
        draw_plain_text(
            &mut canvas, "123", (5.0, 22.0), Color::BLACK, "", 18.0, false, false,
        );
        let rgba = canvas.into_rgba();
        let inked = rgba.chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(inked > 20, "expected glyph pixels, got {inked}");
    }

    // Subscript segments shift below the baseline: 'V₀' must grow descent
    // versus plain 'V0' while keeping the digit smaller than full size.
    #[test]
    fn rich_text_subscript_drops_below_baseline() {
        let mk = |s: &str| RichText {
            segments: rich_segments_from_text(s),
            color: Color::BLACK,
            font_size: 18.0,
            font: String::new(),
        };
        let plain = measure_rich_text(&mk("V0"));
        let sub = measure_rich_text(&mk("V₀"));
        assert!(sub.descent > plain.descent, "subscript must extend below the baseline");
        assert!(sub.width < plain.width, "subscript digit renders at reduced size");
    }

    // register_font_bytes makes a family resolvable at runtime: before
    // registration an unknown family falls back to the embedded font, after
    // registration it resolves to the registered face (distinct CacheKey).
    // Liberation Sans bytes double as the fixture under an alias-free name.
    #[test]
    fn register_font_bytes_resolves_new_family() {
        assert!(register_font_bytes(vec![1, 2, 3]).is_err(), "garbage must not register");

        let fallback = resolve_font("Liberation Sans", false, false);
        let families = register_font_bytes(EMBEDDED_REGULAR.to_vec()).unwrap();
        assert_eq!(families, ["Liberation Sans"]);

        let resolved = resolve_font("Liberation Sans", false, false);
        let embedded = embedded_font(false, false);
        assert_ne!(
            resolved.key, embedded.key,
            "registered face must win over the embedded fallback"
        );
        // Note: `fallback` may have come from the system DB on native hosts
        // that ship Liberation Sans; only the embedded-vs-registered relation
        // is asserted, plus that pre-registration resolution succeeded.
        let _ = fallback;
    }

    // Every legend symbol form must measure EXACTLY the same width at the
    // same font size — fixed fields, not glyph metrics, set the advance.
    #[test]
    fn legend_symbol_forms_share_exact_width() {
        use crate::config::{symbol_segments, LegendEntryKind};
        let mk = |kind: &LegendEntryKind| RichText {
            segments: symbol_segments(kind, Color::BLACK),
            color: Color::BLACK,
            font_size: 16.0,
            font: String::new(),
        };
        let line = measure_rich_text(&mk(&LegendEntryKind::Line));
        let scatter = measure_rich_text(&mk(&LegendEntryKind::Scatter));
        let combo = measure_rich_text(&mk(&LegendEntryKind::LineScatter));
        assert!((line.width - 32.0).abs() < 0.01, "field = 2.0 em × 16 px, got {}", line.width);
        assert!((line.width - scatter.width).abs() < 0.01);
        assert!((line.width - combo.width).abs() < 0.01);

        // The rule must actually ink (nearly) the whole field — a drawn bar,
        // not a short dash glyph centered in space.
        let mut canvas = Canvas::new(48, 32).unwrap();
        draw_rich_text(&mut canvas, &mk(&LegendEntryKind::Line), (4.0, 20.0));
        let rgba = canvas.into_rgba();
        let ink_cols: Vec<u32> = (0..48u32)
            .filter(|&x| (0..32u32).any(|y| rgba[((y * 48 + x) * 4 + 3) as usize] > 0))
            .collect();
        let span = ink_cols.last().unwrap() - ink_cols.first().unwrap() + 1;
        assert!(
            (31..=33).contains(&span),
            "rule ink must span the whole 32 px field, got {span}"
        );
    }

    // '\t' segments are table column separators: cells after the tab start
    // at the same x on every line — the widest pre-tab cell sets the column
    // width. This is what equalizes legend symbol fields.
    #[test]
    fn rich_text_tab_aligns_columns_across_lines() {
        let mk = |s: &str| RichText {
            segments: rich_segments_from_text(s),
            color: Color::BLACK,
            font_size: 16.0,
            font: String::new(),
        };
        // Line 1 pre-tab cell ("WWW") is much wider than line 2's ("i").
        let rt = mk("WWW\tX\ni\tX");
        let m = measure_rich_text(&rt);
        let wide = measure_rich_text(&mk("WWW"));
        let x_w = measure_rich_text(&mk("X"));
        assert!(
            (m.width - (wide.width + x_w.width)).abs() < 0.5,
            "doc width {} must be widest cell0 {} + cell1 {}",
            m.width, wide.width, x_w.width
        );

        // Drawing: the leftmost ink of each line's second cell must agree.
        let mut canvas = Canvas::new(120, 60).unwrap();
        draw_rich_text(&mut canvas, &rt, (4.0, 18.0));
        let rgba = canvas.into_rgba();
        let leftmost_ink = |y0: u32, y1: u32| -> Option<u32> {
            (0..120u32).find(|&x| {
                (y0..y1).any(|y| rgba[((y * 120 + x) * 4 + 3) as usize] > 0)
            })
        };
        // Second-cell x ranges: scan only right of the widest first cell.
        let cut = (4.0 + wide.width - 0.5) as u32;
        let second_ink = |y0: u32, y1: u32| -> Option<u32> {
            (cut..120u32).find(|&x| {
                (y0..y1).any(|y| rgba[((y * 120 + x) * 4 + 3) as usize] > 0)
            })
        };
        let line1_x = second_ink(4, 22).expect("line 1 second cell ink");
        let line2_x = second_ink(24, 45).expect("line 2 second cell ink");
        assert!(
            line1_x.abs_diff(line2_x) <= 1,
            "second-column ink must align: line1 at {line1_x}, line2 at {line2_x}"
        );
        let _ = leftmost_ink;
    }

    // '\n' segments break lines: the envelope grows vertically, width is the
    // widest line, and drawing puts ink in two vertically separated bands.
    #[test]
    fn rich_text_newline_breaks_lines() {
        let mk = |s: &str| RichText {
            segments: rich_segments_from_text(s),
            color: Color::BLACK,
            font_size: 16.0,
            font: String::new(),
        };
        let one = measure_rich_text(&mk("abc"));
        let two = measure_rich_text(&mk("abc\nde"));
        assert!(two.height() > one.height() * 1.7, "second line must add a full line");
        assert!((two.width - one.width).abs() < 0.01, "width is the widest line (abc)");

        let mut canvas = Canvas::new(60, 60).unwrap();
        draw_rich_text(&mut canvas, &mk("ab\nab"), (5.0, 18.0));
        let rgba = canvas.into_rgba();
        let row_has_ink = |y: u32| {
            (0..60u32).any(|x| rgba[((y * 60 + x) * 4 + 3) as usize] > 0)
        };
        // Ink near the first baseline, a gap is not guaranteed (descenders),
        // but there must be ink well below one line's extent too.
        assert!(row_has_ink(12), "first line ink");
        assert!((30..55).any(row_has_ink), "second line ink");
    }

    // A per-segment color override must draw ink in that color (glyph and
    // underline both), not the RichText-level default.
    #[test]
    fn rich_text_segment_color_override_inks_in_override_color() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let mut segments = rich_segments_from_text("ll");
        segments[0].color = Some(red);
        segments[0].underline = true;
        let rt = RichText {
            segments,
            color: Color::BLACK, // document default stays black
            font_size: 24.0,
            font: String::new(),
        };

        let mut canvas = Canvas::new(60, 40).unwrap();
        draw_rich_text(&mut canvas, &rt, (5.0, 30.0));
        let rgba = canvas.into_rgba();
        let red_ink = rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 200 && p[0] > 150 && p[1] < 50 && p[2] < 50)
            .count();
        let black_ink = rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 200 && p[0] < 50 && p[1] < 50 && p[2] < 50)
            .count();
        assert!(red_ink > 5, "override segment must ink red, got {red_ink} px");
        assert!(black_ink > 5, "non-override segment keeps the default, got {black_ink} px");
    }

    // A per-segment font_size override must change metrics: wider advance and
    // taller ascent than the same text at the document size.
    #[test]
    fn rich_text_segment_font_size_override_changes_metrics() {
        let mk = |override_first: bool| {
            let mut segments = rich_segments_from_text("AA");
            if override_first {
                segments[0].font_size = Some(36.0);
            }
            RichText {
                segments,
                color: Color::BLACK,
                font_size: 16.0,
                font: String::new(),
            }
        };
        let plain = measure_rich_text(&mk(false));
        let overridden = measure_rich_text(&mk(true));
        assert!(
            overridden.width > plain.width,
            "bigger segment must widen the line: {} !> {}",
            overridden.width,
            plain.width
        );
        assert!(
            overridden.ascent > plain.ascent,
            "bigger segment must raise the envelope: {} !> {}",
            overridden.ascent,
            plain.ascent
        );
    }

    // Superscript segments must lift the envelope above the base ascent and
    // rich drawing must produce ink too.
    #[test]
    fn rich_text_superscript_extends_envelope() {
        let plain = RichText {
            segments: rich_segments_from_text("10"),
            color: Color::BLACK,
            font_size: 18.0,
            font: String::new(),
        };
        let sup = RichText {
            segments: rich_segments_from_text("10³"), // '³' maps to a superscript segment
            color: Color::BLACK,
            font_size: 18.0,
            font: String::new(),
        };
        let m_plain = measure_rich_text(&plain);
        let m_sup = measure_rich_text(&sup);
        assert!(m_sup.ascent > m_plain.ascent, "superscript must raise the envelope");

        let mut canvas = Canvas::new(80, 40).unwrap();
        draw_rich_text(&mut canvas, &sup, (5.0, 30.0));
        let rgba = canvas.into_rgba();
        assert!(rgba.chunks_exact(4).any(|p| p[3] > 0));
    }
}
