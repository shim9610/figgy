//! Pure text rendering onto the CPU raster [`Canvas`].
//!
//! No window or event-loop dependency — the caller passes in a canvas and we
//! draw into it. Liberation Sans (SIL OFL 1.1) is bundled in four styles, so
//! rendering always succeeds even on systems without the requested font: if
//! `RichText.font` is empty or unresolvable, the bundled font is used. A
//! character the resolved face cannot draw falls back per character to a face
//! that can (registered faces, the bundled face, then system faces on native),
//! so it does not render as a `.notdef` box.
//!
//! Layering: the segment layout policy (super/subscript scaling and shifts,
//! kerning gaps, underline bars, greek mapping, the measure envelope) lives
//! here and is backend-independent. The glyph layer below it — metrics and
//! alpha-mask rasterization — is fontdb (font lookup) + swash (scaling), all
//! pure Rust, so the whole stack compiles for wasm32-unknown-unknown.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(test)]
pub(crate) static FONT_REGISTRATION_TEST_LOCK: Mutex<()> = Mutex::new(());

use swash::FontRef;
use swash::scale::{Render, ScaleContext, Source};

use crate::color::Color;
use crate::layout::RectF;
use crate::raster::Canvas;
use crate::text::{RichSegment, RichText, greek_char};

type FontLookupKey = (String, bool, bool);
/// Per-character fallback decisions depend only on the character and the
/// requested style: the chain after the primary face is the same whichever
/// family missed.
type FallbackKey = (char, bool, bool);
/// One fontdb face parsed once over stable (leaked) backing bytes. Keeping the
/// parsed `FontRef` keeps one swash `CacheKey` per face, so every character
/// and thread that resolves to the face shares its scaler and glyph caches.
type FaceBacking = FontRef<'static>;
type FaceBackingCache = HashMap<fontdb::ID, FaceBacking>;

struct ResolvedFontCache {
    generation: u64,
    fonts: HashMap<FontLookupKey, FontRef<'static>>,
    /// Face drawing each character its primary face cannot draw; `None`
    /// records that no face can, so the chain is not walked again.
    fallbacks: HashMap<FallbackKey, Option<FontRef<'static>>>,
}

impl ResolvedFontCache {
    fn new() -> Self {
        Self {
            generation: 0,
            fonts: HashMap::new(),
            fallbacks: HashMap::new(),
        }
    }

    /// Forget every decision made under an older font generation: a newly
    /// registered face may now win a family lookup or draw a character.
    fn revalidate(&mut self) {
        let generation = font_generation();
        if self.generation != generation {
            self.generation = generation;
            self.fonts.clear();
            self.fallbacks.clear();
        }
    }
}

// Bundled fonts (SIL OFL 1.1):
//   - Liberation Sans — the default face (fonts/LICENSE-LiberationSans.txt).
//   - Comic Neue — the handwritten face forced by the sketch draw style
//     (fonts/LICENSE-ComicNeue.txt).

static EMBEDDED_REGULAR: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");
static EMBEDDED_BOLD: &[u8] = include_bytes!("../fonts/LiberationSans-Bold.ttf");
static EMBEDDED_ITALIC: &[u8] = include_bytes!("../fonts/LiberationSans-Italic.ttf");
static EMBEDDED_BOLD_ITALIC: &[u8] = include_bytes!("../fonts/LiberationSans-BoldItalic.ttf");
static HANDWRITTEN_REGULAR: &[u8] = include_bytes!("../fonts/ComicNeue-Regular.ttf");
static HANDWRITTEN_BOLD: &[u8] = include_bytes!("../fonts/ComicNeue-Bold.ttf");

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
struct RegisteredFonts {
    db: fontdb::Database,
    /// Exact font bytes to the families reported by their first registration.
    ///
    /// `HashMap` hashes for lookup but still compares the complete `Vec<u8>`
    /// on collisions, so this is not a hash-only identity check. The `Arc`
    /// allocation is shared with fontdb's binary source instead of retaining
    /// a second copy solely for deduplication.
    families_by_bytes: HashMap<Arc<Vec<u8>>, Vec<String>>,
    /// Stable swash backing bytes per fontdb face.
    ///
    /// A runtime registration invalidates the thread-local family lookup, but
    /// it must not copy and leak an already-resolved face again afterward.
    /// Family lookups and per-character fallback share it.
    resolved_faces: FaceBackingCache,
}

impl RegisteredFonts {
    fn new() -> Self {
        Self {
            db: fontdb::Database::new(),
            families_by_bytes: HashMap::new(),
            resolved_faces: HashMap::new(),
        }
    }

    fn register(
        &mut self,
        bytes: Vec<u8>,
        generation: u64,
    ) -> Result<(Vec<String>, Option<u64>), String> {
        if let Some(families) = self.families_by_bytes.get(&bytes) {
            return Ok((families.clone(), None));
        }

        let bytes = Arc::new(bytes);
        let (families, successor) =
            load_registered_font(&mut self.db, Arc::clone(&bytes), generation)?;
        self.families_by_bytes.insert(bytes, families.clone());
        Ok((families, Some(successor)))
    }

    fn lookup(&mut self, family: &str, bold: bool, italic: bool) -> Option<FaceBacking> {
        let id = query_font(&self.db, family, bold, italic)?;
        stable_face_backing(&self.db, &mut self.resolved_faces, id)
    }

    /// Registered face that draws `ch`: registered families in registration
    /// order (the first family name each face reports, first seen first),
    /// each through the same style matching as a family lookup.
    fn fallback(&mut self, ch: char, bold: bool, italic: bool) -> Option<FaceBacking> {
        // Faces are never removed, so fontdb iterates them in insertion
        // (registration) order.
        let mut families: Vec<&str> = Vec::new();
        for face in self.db.faces() {
            if let Some((name, _)) = face.families.first()
                && !families.contains(&name.as_str())
            {
                families.push(name);
            }
        }
        let id = first_face_drawing(&self.db, families, ch, bold, italic)?;
        stable_face_backing(&self.db, &mut self.resolved_faces, id)
    }
}

fn registered_fonts() -> &'static Mutex<RegisteredFonts> {
    static FONTS: OnceLock<Mutex<RegisteredFonts>> = OnceLock::new();
    FONTS.get_or_init(|| Mutex::new(RegisteredFonts::new()))
}

fn load_registered_font(
    db: &mut fontdb::Database,
    bytes: Arc<Vec<u8>>,
    generation: u64,
) -> Result<(Vec<String>, u64), String> {
    let successor = generation
        .checked_add(1)
        .ok_or_else(|| "font generation counter exhausted".to_string())?;
    let source: Arc<dyn AsRef<[u8]> + Send + Sync> = bytes;
    let ids = db.load_font_source(fontdb::Source::Binary(source));
    let mut families: Vec<String> = ids
        .iter()
        .filter_map(|id| db.face(*id))
        .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
        .collect();
    families.dedup();
    if families.is_empty() {
        return Err("font data contains no usable face".to_string());
    }
    Ok((families, successor))
}

/// Register a font (TTF/OTF/TTC bytes) for `RichText.font` family lookup.
/// Returns the family names the file declares, or an error when the bytes
/// parse to no usable face. Registering the exact same bytes again is
/// idempotent; different font files declaring the same family still coexist.
/// A registered face also supplies characters the configured family cannot
/// draw (per-character fallback), so registering a font can change text
/// whose `font` names another family.
pub fn register_font_bytes(bytes: Vec<u8>) -> Result<Vec<String>, String> {
    let mut fonts = registered_fonts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let generation = font_generation();
    let (families, successor) = fonts.register(bytes, generation)?;
    let Some(successor) = successor else {
        return Ok(families);
    };
    // A family that previously missed (and was cached as the bundled
    // fallback) may now resolve, and a character no face could draw may now
    // have one — invalidate every thread's resolution and fallback caches
    // via the generation counter. Glyph caches key on each font's own
    // CacheKey, so they stay valid.
    FONT_GENERATION.store(successor, std::sync::atomic::Ordering::Release);
    Ok(families)
}

/// Family name of the bundled default face. Naming it always resolves to a
/// Liberation Sans face (registered, installed, or the bundled copy, in normal
/// resolution order), so it is always available; which copy is drawn is not
/// guaranteed to be the bundled one.
const BUNDLED_FAMILY: &str = "Liberation Sans";

/// Whether `family` names a face the text stack can actually draw with,
/// rather than one it silently replaces with the bundled Liberation Sans.
///
/// `RichText.font` never fails to render: a family that is neither registered
/// ([`register_font_bytes`]) nor installed (native system font database) falls
/// back to the bundled face. Hosts that want to report a typo or a missing font
/// check the name here first. Matching is the one resolution uses (fontdb's
/// exact, case-sensitive family name), so `true` means resolution finds a face
/// for the family. The empty string and `"Liberation Sans"` both name the
/// bundled face and are always available, on wasm included; apart from those,
/// on wasm only registered fonts count, because there is no system font
/// database. Generic CSS names such as `"sans-serif"` are not
/// family names and report `false`, because resolution falls back for them.
///
/// Reads registered fonts under the registry mutex resolution already takes,
/// and the system database through its immutable `OnceLock`; no new lock.
pub fn font_family_available(family: &str) -> bool {
    if family.is_empty() || family == BUNDLED_FAMILY {
        return true;
    }
    let registered = registered_fonts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if query_font(&registered.db, family, false, false).is_some() {
        return true;
    }
    drop(registered);
    system_family_available(family)
}

#[cfg(not(target_arch = "wasm32"))]
fn system_family_available(family: &str) -> bool {
    query_font(font_db(), family, false, false).is_some()
}

#[cfg(target_arch = "wasm32")]
fn system_family_available(_family: &str) -> bool {
    false
}

fn lookup_registered_font(family: &str, bold: bool, italic: bool) -> Option<FaceBacking> {
    registered_fonts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .lookup(family, bold, italic)
}

fn lookup_registered_fallback(ch: char, bold: bool, italic: bool) -> Option<FaceBacking> {
    registered_fonts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .fallback(ch, bold, italic)
}

/// Stable backing for fontdb face `id`: from `faces`, or by copying and
/// leaking the face's bytes once. Registered and system lookups — family
/// resolution and per-character fallback alike — all go through here, so a
/// face is leaked at most once per face id.
fn stable_face_backing(
    db: &fontdb::Database,
    faces: &mut FaceBackingCache,
    id: fontdb::ID,
) -> Option<FaceBacking> {
    if let Some(face) = faces.get(&id) {
        return Some(*face);
    }
    let face = db.with_face_data(id, |data, index| {
        // Parse before copying, so a face swash cannot read is never leaked.
        FontRef::from_index(data, index as usize)?;
        let bytes: &'static [u8] = Box::leak(data.to_vec().into_boxed_slice());
        FontRef::from_index(bytes, index as usize)
    })??;
    faces.insert(id, face);
    Some(face)
}

/// Whether `face` can draw `ch`: its charmap maps `ch` to a real glyph
/// rather than `.notdef` (glyph 0), and the face has outlines. Glyphs
/// rasterize from outline sources only, so a bitmap-only face (colour emoji)
/// would map the character yet draw nothing.
fn draws(face: FontRef<'_>, ch: char) -> bool {
    const OUTLINE_TABLES: [&[u8; 4]; 3] = [b"glyf", b"CFF ", b"CFF2"];
    face.charmap().map(ch) != 0
        && OUTLINE_TABLES
            .iter()
            .any(|tag| face.table(swash::tag_from_bytes(tag)).is_some())
}

/// First of `families`, in the order given, whose face for the requested
/// style (fontdb's CSS matching, as in a family lookup) draws `ch`. Probing
/// only borrows each candidate's data (registered bytes or a temporary file
/// mapping), so nothing is copied or leaked here.
fn first_face_drawing<'a>(
    db: &fontdb::Database,
    families: impl IntoIterator<Item = &'a str>,
    ch: char,
    bold: bool,
    italic: bool,
) -> Option<fontdb::ID> {
    families
        .into_iter()
        .filter_map(|family| query_font(db, family, bold, italic))
        .find(|&id| {
            db.with_face_data(id, |data, index| {
                FontRef::from_index(data, index as usize).is_some_and(|face| draws(face, ch))
            })
            .unwrap_or(false)
        })
}

fn query_font(db: &fontdb::Database, family: &str, bold: bool, italic: bool) -> Option<fontdb::ID> {
    let families = [fontdb::Family::Name(family)];
    let query = fontdb::Query {
        families: &families,
        weight: if bold {
            fontdb::Weight::BOLD
        } else {
            fontdb::Weight::NORMAL
        },
        stretch: fontdb::Stretch::Normal,
        style: if italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        },
    };
    db.query(&query)
}

/// Per-chart text policy, derived from `Config.draw_style`. Threaded
/// explicitly through every measure/draw entry point so layout and raster
/// always agree on glyph metrics.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum FontPolicy {
    /// Normal resolution: `RichText.font` family → registered fonts →
    /// system DB (native) → bundled Liberation Sans. A character that face
    /// cannot draw falls back PER CHARACTER to registered faces, the bundled
    /// Liberation Sans, then system faces (native) that can.
    #[default]
    Standard,
    /// Sketch mode: every glyph the bundled handwritten face (Comic Neue)
    /// covers is drawn with it — regardless of the configured family. Glyphs
    /// it lacks (CJK, Greek, …) fall back PER CHARACTER to the standard
    /// chain, so registered CJK fonts keep working inside sketch charts.
    Handwritten,
}

impl FontPolicy {
    /// The font side of a draw style: sketch forces the handwritten face.
    pub fn for_style(style: &crate::config::DrawStyle) -> Self {
        match style {
            crate::config::DrawStyle::Sketch(_) => FontPolicy::Handwritten,
            _ => FontPolicy::Standard,
        }
    }
}

/// Bundled handwritten face (Comic Neue). Italic intentionally maps onto the
/// upright variants — the hand-drawn look keeps one voice, and bundling four
/// more faces is wasm weight for no chart-visible gain.
fn handwritten_font(bold: bool) -> FontRef<'static> {
    static REGULAR: OnceLock<FontRef<'static>> = OnceLock::new();
    static BOLD: OnceLock<FontRef<'static>> = OnceLock::new();
    let (cell, bytes) = if bold {
        (&BOLD, HANDWRITTEN_BOLD)
    } else {
        (&REGULAR, HANDWRITTEN_REGULAR)
    };
    *cell.get_or_init(|| FontRef::from_index(bytes, 0).expect("corrupted bundled font"))
}

/// Resolve the font for ONE glyph under `policy`. The handwritten policy
/// checks the glyph's coverage first and falls back to the standard chain
/// per character. The standard chain is the family's resolved face, then,
/// when that face cannot draw the character, the per-character fallback
/// chain ([`fallback_font`]). Only a character no face can draw keeps the
/// primary face's `.notdef` box. Measurement and drawing both resolve glyphs
/// here, so layout and raster agree on the face.
fn glyph_font(
    policy: FontPolicy,
    family: &str,
    bold: bool,
    italic: bool,
    ch: char,
) -> FontRef<'static> {
    if policy == FontPolicy::Handwritten {
        let hand = handwritten_font(bold);
        if draws(hand, ch) {
            return hand;
        }
    }
    let primary = resolve_font(family, bold, italic);
    if draws(primary, ch) {
        return primary;
    }
    fallback_font(ch, bold, italic).unwrap_or(primary)
}

/// Face that draws `ch` when the primary face cannot, or `None` when no face
/// can. Decided once per (char, bold, italic) per thread per font
/// generation: a miss probes candidate faces, a repeat is one map lookup.
fn fallback_font(ch: char, bold: bool, italic: bool) -> Option<FontRef<'static>> {
    RESOLVED.with(|cell| {
        let mut cache = cell.borrow_mut();
        cache.revalidate();
        *cache
            .fallbacks
            .entry((ch, bold, italic))
            .or_insert_with(|| find_fallback(ch, bold, italic))
    })
}

/// The chain after the primary face: (a) runtime-registered faces, (b) the
/// bundled Liberation Sans of the same style, (c) on native only, installed
/// system faces. Anything resolvable on every target (registered, bundled)
/// therefore resolves identically on native and wasm; installed system fonts
/// are a native-only last resort, consistent with family resolution, which
/// already consults system fonts on native. The registry and system face
/// caches are locked one at a time, never nested.
fn find_fallback(ch: char, bold: bool, italic: bool) -> Option<FontRef<'static>> {
    lookup_registered_fallback(ch, bold, italic)
        .or_else(|| Some(embedded_font(bold, italic)).filter(|face| draws(*face, ch)))
        .or_else(|| lookup_system_fallback(ch, bold, italic))
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

/// Bumped when `register_font_bytes` accepts previously unseen file contents.
/// Exact duplicate registration is idempotent. Thread-local resolution caches
/// compare against this value and clear themselves, so a newly registered
/// family wins over a previously cached fallback on every thread without
/// cross-thread locking on the hot path.
static FONT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Current runtime-font content stamp for renderer cache invalidation.
pub(crate) fn font_generation() -> u64 {
    FONT_GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

thread_local! {
    /// Resolved `FontRef` per (family, bold, italic) and fallback face per
    /// (char, bold, italic), per thread — the hot path of measure/draw
    /// (called per segment, including during pointer hit-testing) stays
    /// lock-free. Stable backing bytes are cached by fontdb face id in the
    /// process-wide registered/system face caches, so thread-local misses
    /// reuse the same backing instead of copying it.
    static RESOLVED: RefCell<ResolvedFontCache> = RefCell::new(ResolvedFontCache::new());
}

/// Stable backing for a system face id. Family lookups and fallback
/// decisions share this cache, so a system face's bytes are copied at most
/// once however many families, characters or threads resolve to it.
#[cfg(not(target_arch = "wasm32"))]
fn system_face_backing(id: fontdb::ID) -> Option<FaceBacking> {
    static RESOLVED_SYSTEM_FACES: OnceLock<Mutex<FaceBackingCache>> = OnceLock::new();
    let mut resolved = RESOLVED_SYSTEM_FACES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    stable_face_backing(font_db(), &mut resolved, id)
}

#[cfg(not(target_arch = "wasm32"))]
fn lookup_system_font(family: &str, bold: bool, italic: bool) -> Option<FaceBacking> {
    system_face_backing(query_font(font_db(), family, bold, italic)?)
}

#[cfg(target_arch = "wasm32")]
fn lookup_system_font(_family: &str, _bold: bool, _italic: bool) -> Option<FaceBacking> {
    None
}

/// System family names (the first name each face reports) in sorted order,
/// so the fallback choice does not depend on font install order. Computed
/// once: the system database never changes after its scan.
#[cfg(not(target_arch = "wasm32"))]
fn system_fallback_families() -> &'static [&'static str] {
    static FAMILIES: OnceLock<Vec<&'static str>> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let mut families: Vec<&'static str> = font_db()
            .faces()
            .filter_map(|face| face.families.first().map(|(name, _)| name.as_str()))
            .collect();
        families.sort_unstable();
        families.dedup();
        families
    })
}

/// System face that draws `ch`. Probing runs without the face cache lock:
/// the database never changes after its scan, and only the chosen face is
/// copied.
#[cfg(not(target_arch = "wasm32"))]
fn lookup_system_fallback(ch: char, bold: bool, italic: bool) -> Option<FaceBacking> {
    let families = system_fallback_families().iter().copied();
    system_face_backing(first_face_drawing(font_db(), families, ch, bold, italic)?)
}

#[cfg(target_arch = "wasm32")]
fn lookup_system_fallback(_ch: char, _bold: bool, _italic: bool) -> Option<FaceBacking> {
    None
}

/// Resolve a family's primary face, falling back to the bundled font.
/// Always succeeds. Characters the face cannot draw are handled per glyph by
/// [`glyph_font`].
/// 1) If `family` is non-empty, query runtime-registered fonts (all targets),
///    then the system font database (native only).
/// 2) Otherwise (or on miss), return the bundled Liberation Sans variant.
fn resolve_font(family: &str, bold: bool, italic: bool) -> FontRef<'static> {
    if family.is_empty() {
        return embedded_font(bold, italic);
    }
    RESOLVED.with(|cell| {
        let mut cache = cell.borrow_mut();
        cache.revalidate();
        let key = (family.to_string(), bold, italic);
        if let Some(hit) = cache.fonts.get(&key) {
            return *hit;
        }
        let resolved = lookup_registered_font(family, bold, italic)
            .or_else(|| lookup_system_font(family, bold, italic))
            .unwrap_or_else(|| embedded_font(bold, italic));
        cache.fonts.insert(key, resolved);
        resolved
    })
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

/// True for the bundled handwritten face (either weight). It rasterizes with
/// two deviations from the standard faces:
/// - UNHINTED — grid-fitting visibly disfigures Comic Neue's irregular
///   strokes into lumpy outlines; unhinted keeps the drawn shapes smooth.
/// - Slightly EMBOLDENED — the face is thin at chart label sizes, and the
///   sketch stroker draws lines with pen body; a small synthetic embolden
///   gives the glyphs the same ink weight.
fn is_handwritten(font: FontRef<'static>) -> bool {
    font.key == handwritten_font(false).key || font.key == handwritten_font(true).key
}

/// Hinting (TrueType grid-fitting) is small-text technology: above this
/// size it adds nothing visually, but it still EXECUTES the font's hint
/// bytecode per (glyph, ppem) — and exotic hint programs in system fonts
/// can catastrophically displace outline points at specific pixel sizes
/// (field report: macOS-resolved font blew up 'e'/'t' to several times
/// their size at font_size 51 / retina 102 only; FreeType tracks the same
/// class upstream). Cutting hinting off above the threshold removes the
/// bug class for every font; chart labels stay below it and keep their
/// grid fit.
const HINT_MAX_SIZE_PX: f32 = 32.0;

fn font_hinting(font: FontRef<'static>, size: f32) -> bool {
    !is_handwritten(font) && size <= HINT_MAX_SIZE_PX
}

/// Synthetic embolden strength (px) for the handwritten face — ~2% of the
/// font size, i.e. ≈0.4 px at typical label sizes.
fn font_embolden(font: FontRef<'static>, size: f32) -> f32 {
    if is_handwritten(font) {
        size * 0.02
    } else {
        0.0
    }
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
        let mut scaler = cx
            .builder(font)
            .size(size)
            .hint(font_hinting(font, size))
            .build();
        match scaler.scale_outline(glyph) {
            // zeno bounds are y-up; flip into the y-down baseline convention.
            Some(outline) => {
                let b = outline.bounds();
                (-b.max.y, -b.min.y)
            }
            None => (0.0, 0.0),
        }
    });
    let extents = GlyphExtents {
        advance,
        ink_top,
        ink_bottom,
    };
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
                let mut scaler = cx
                    .builder(font)
                    .size(size)
                    .hint(font_hinting(font, size))
                    .build();
                let mut render = Render::new(&[Source::Outline]);
                render.offset(swash::zeno::Vector::new(xbin as f32 * 0.25, 0.0));
                let embolden = font_embolden(font, size);
                if embolden > 0.0 {
                    render.embolden(embolden);
                }
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

    match canvas.orthogonal_text_transform() {
        // Crisp path: snap to the pixel grid, carry the sub-pixel x phase
        // into the rasterizer, and blit without any filtering.
        Some((turn, tx, ty)) => {
            let gx = pen_x + tx;
            // Half-up snapping is invariant under integer translation, including
            // the negative local coordinates of a rotated canvas.
            let gy = (baseline + ty + 0.5).floor() as i32;
            let mut ix = gx.floor() as i32;
            let mut xbin = ((gx - gx.floor()) * 4.0).round() as u8;
            if xbin == 4 {
                xbin = 0;
                ix += 1;
            }
            with_glyph_mask(font, size, ch, xbin, |m| {
                canvas.blit_mask_orthogonal(
                    ix + m.left,
                    gy - m.top,
                    m.width,
                    m.height,
                    &m.data,
                    color,
                    turn,
                );
            });
        }
        // General transforms (non-right-angle rotation or scaling) retain
        // the filtered fallback. Axis titles use the crisp path above.
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
// Fill band under a rule (legend mark of a filled line series): from the
// rule's center down to this many em above the baseline, so it stays inside
// the glyph ascent and never changes the line's measured extents.
const RULE_BAND_BOTTOM_RATIO: f32 = 0.05;
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

fn layout_doc(rt: &RichText, policy: FontPolicy) -> DocLayout<'_> {
    let line_cells: Vec<Vec<&[RichSegment]>> =
        split_lines(rt).into_iter().map(split_cells).collect();

    let cell_metrics: Vec<Vec<TextMetrics>> = line_cells
        .iter()
        .map(|cells| cells.iter().map(|c| measure_line(c, rt, policy)).collect())
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
            TextMetrics {
                width,
                ascent,
                descent,
            }
        })
        .collect();

    DocLayout {
        line_cells,
        line_metrics,
        col_starts,
    }
}

/// Envelope of one line. Empty lines fall back to a font-size-derived height
/// so blank lines still advance the baseline.
fn measure_line(segments: &[RichSegment], rt: &RichText, policy: FontPolicy) -> TextMetrics {
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
            let ch = if seg.greek {
                greek_char(seg.text)
            } else {
                seg.text
            };
            let font = glyph_font(policy, &rt.font, seg.bold, seg.italic, ch);
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
    TextMetrics {
        width: total_w,
        ascent: max_ascent,
        descent: max_descent,
    }
}

/// Width of the whole block: its widest line. Measurement reports it, and
/// drawing aligns every line inside it, so the two cannot disagree.
fn block_width(line_metrics: &[TextMetrics]) -> f32 {
    line_metrics.iter().map(|m| m.width).fold(0.0, f32::max)
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
    policy: FontPolicy,
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
            // The band goes under the rule first, so a dashed rule's gaps
            // show the band and a solid rule covers its top edge.
            if let Some(band) = &seg.rule_band {
                let top = y - base * RULE_Y_RATIO;
                let bottom = y - base * RULE_BAND_BOTTOM_RATIO;
                canvas.draw_rect(
                    pen_x,
                    top,
                    field,
                    (bottom - top).max(1.0),
                    &crate::raster::Paint::fill(band),
                );
            }
            if let Some(pattern) = &seg.rule_dash {
                draw_dashed_rule(
                    canvas,
                    RectF {
                        x: pen_x,
                        y: ry,
                        width: field,
                        height: t,
                    },
                    &color,
                    pattern,
                    base,
                );
            } else {
                canvas.draw_rect(pen_x, ry, field, t, &crate::raster::Paint::fill(&color));
            }
            field
        } else {
            let ch = if seg.greek {
                greek_char(seg.text)
            } else {
                seg.text
            };
            let font = glyph_font(policy, &rt.font, seg.bold, seg.italic, ch);
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

fn draw_dashed_rule(canvas: &mut Canvas, rect: RectF, color: &Color, dash_em: &[f32], base: f32) {
    let pattern: Vec<f32> = dash_em
        .iter()
        .filter_map(|v| {
            let px = *v * base;
            (px.is_finite() && px > 0.0).then_some(px)
        })
        .collect();
    if pattern.is_empty() {
        canvas.draw_rect(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            &crate::raster::Paint::fill(color),
        );
        return;
    }

    let mut offset = 0.0;
    let mut pattern_i = 0usize;
    let mut ink = true;
    while offset < rect.width {
        let len = pattern[pattern_i % pattern.len()].min(rect.width - offset);
        if ink {
            canvas.draw_rect(
                rect.x + offset,
                rect.y,
                len,
                rect.height,
                &crate::raster::Paint::fill(color),
            );
        }
        offset += len;
        pattern_i += 1;
        ink = !ink;
    }
}

#[cfg(test)]
mod dashed_rule_tests {
    use super::*;
    use crate::text::TextAlign;

    // The band of a filled line's legend mark is painted under the rule in
    // the band colour, inside the rule's fixed field, and leaves the measured
    // width and height of the mark unchanged.
    #[test]
    fn rule_band_fills_under_the_rule_without_changing_the_symbol_metrics() {
        let band = Color::new(0.0, 0.0, 1.0, 1.0);
        let make = |band: Option<Color>| RichText {
            segments: vec![
                RichSegment::rule(2.0, Some(Color::new(1.0, 0.0, 0.0, 1.0))).with_rule_band(band),
            ],
            color: Color::BLACK,
            font_size: 16.0,
            font: String::new(),
            align: TextAlign::Left,
        };
        let plain = make(None);
        let banded = make(Some(band));
        assert_eq!(
            measure_rich_text(&plain, FontPolicy::Standard),
            measure_rich_text(&banded, FontPolicy::Standard)
        );

        let mut canvas = Canvas::new(48, 32).unwrap();
        draw_rich_text(&mut canvas, &banded, (4.0, 24.0), FontPolicy::Standard);
        let rgba = canvas.into_rgba();
        let px =
            |x: u32, y: u32| &rgba[((y * 48 + x) * 4) as usize..((y * 48 + x) * 4 + 4) as usize];
        // Rule center: 0.3 em above the baseline, i.e. y ≈ 24 − 4.8.
        let rule = px(20, 19);
        assert!(rule[0] > 200 && rule[2] < 60, "rule row is red: {rule:?}");
        // Halfway between the rule and the baseline: the band.
        let in_band = px(20, 22);
        assert!(
            in_band[2] > 200 && in_band[0] < 60,
            "band row is blue: {in_band:?}"
        );
        // Below the baseline and outside the field: nothing.
        assert_eq!(px(20, 26)[3], 0);
        assert_eq!(px(40, 22)[3], 0);
        let mut plain_canvas = Canvas::new(48, 32).unwrap();
        draw_rich_text(&mut plain_canvas, &plain, (4.0, 24.0), FontPolicy::Standard);
        assert_eq!(
            plain_canvas.into_rgba()[((22 * 48 + 20) * 4 + 3) as usize],
            0
        );
    }

    #[test]
    fn dashed_rule_renders_visible_gaps_inside_fixed_field() {
        let rt = RichText {
            segments: vec![RichSegment::dashed_rule(
                2.0,
                Some(Color::BLACK),
                vec![0.25, 0.25],
            )],
            color: Color::BLACK,
            font_size: 16.0,
            font: String::new(),
            align: TextAlign::Left,
        };
        let mut canvas = Canvas::new(48, 32).unwrap();
        draw_rich_text(&mut canvas, &rt, (4.0, 20.0), FontPolicy::Standard);
        let rgba = canvas.into_rgba();
        let ink_cols: Vec<u32> = (0..48u32)
            .filter(|&x| (0..32u32).any(|y| rgba[((y * 48 + x) * 4 + 3) as usize] > 0))
            .collect();
        let first = *ink_cols.first().unwrap();
        let last = *ink_cols.last().unwrap();
        let gap =
            (first..=last).any(|x| !(0..32u32).any(|y| rgba[((y * 48 + x) * 4 + 3) as usize] > 0));
        assert!(
            gap,
            "dashed legend rule should leave at least one transparent gap"
        );
    }
}

// Public rendering API.

/// Draw `rt` at `origin` (left edge of the block, baseline **of the first
/// line**). The caller is responsible for measuring and placing the block —
/// this function does not position it. `'\n'` segments start a new line;
/// subsequent baselines advance by the previous line's descent + the next
/// line's ascent. Inside the block, which is as wide as its widest line, each
/// line is offset by `rt.align` ([`TextAlign::line_offset`]); its tab-column
/// cells move with it.
///
/// [`TextAlign::line_offset`]: crate::text::TextAlign::line_offset
pub fn draw_rich_text(canvas: &mut Canvas, rt: &RichText, origin: (f32, f32), policy: FontPolicy) {
    let doc = layout_doc(rt, policy);
    let (x, mut baseline) = origin;
    let block_width = block_width(&doc.line_metrics);

    for (i, cells) in doc.line_cells.iter().enumerate() {
        if i > 0 {
            baseline += line_advance(&doc.line_metrics[i - 1], &doc.line_metrics[i], rt);
        }
        let line_x = x + rt.align.line_offset(block_width, doc.line_metrics[i].width);
        for (k, cell) in cells.iter().enumerate() {
            draw_line_segments(
                canvas,
                cell,
                rt,
                (line_x + doc.col_starts[k], baseline),
                policy,
            );
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
pub use crate::text::TextExtents as TextMetrics;

/// CPU-raster implementation of the model crate's [`crate::MeasureText`] contract.
///
/// Inject this wherever the model needs glyph extents — element bounds for
/// `Selectable::selection_box`, hit tests, etc. — so model-side geometry uses
/// the same measurements the renderer draws with. Build it with
/// [`CpuTextMeasure::for_style`] so sketch-mode charts measure with the same
/// handwritten face they draw with; `default()` measures the standard face.
#[derive(Clone, Copy, Default)]
pub struct CpuTextMeasure {
    policy: FontPolicy,
}

impl CpuTextMeasure {
    /// Measure with the font the given draw style actually renders.
    pub fn for_style(style: &crate::config::DrawStyle) -> Self {
        Self {
            policy: FontPolicy::for_style(style),
        }
    }
}

impl crate::text::MeasureText for CpuTextMeasure {
    fn measure_rich(&self, rt: &RichText) -> crate::text::TextExtents {
        measure_rich_text(rt, self.policy)
    }
}

/// Compute the real rendered bounding box of `rt` using glyph measurements.
/// Mirrors `draw_rich_text` for segment handling, styles, sub/superscripts,
/// underline, and `'\n'` line breaks.
pub fn measure_rich_text(rt: &RichText, policy: FontPolicy) -> TextMetrics {
    if rt.segments.is_empty() {
        return TextMetrics {
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
        };
    }

    let metrics = layout_doc(rt, policy).line_metrics;
    let width = block_width(&metrics);
    let ascent = metrics[0].ascent;
    // Everything below the first baseline: accumulate the same baseline
    // advances `draw_rich_text` uses, then the last line's own descent.
    let mut below = 0.0f32;
    for i in 1..metrics.len() {
        below += line_advance(&metrics[i - 1], &metrics[i], rt);
    }
    let descent = below + metrics.last().map(|m| m.descent).unwrap_or(0.0);

    TextMetrics {
        width,
        ascent,
        descent,
    }
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
// Shares the same font resolution and per-glyph fallback as RichText via
// `glyph_font`.

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
    policy: FontPolicy,
) {
    let (mut pen_x, baseline) = origin;
    for ch in text.chars() {
        let font = glyph_font(policy, family, bold, italic, ch);
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
    policy: FontPolicy,
) -> TextMetrics {
    let mut width = 0.0f32;
    let mut max_ascent = 0.0f32;
    let mut max_descent = 0.0f32;
    for ch in text.chars() {
        let font = glyph_font(policy, family, bold, italic, ch);
        let g = glyph_extents(font, size, ch);
        width += g.advance;
        max_ascent = max_ascent.max(-g.ink_top);
        max_descent = max_descent.max(g.ink_bottom);
    }
    TextMetrics {
        width,
        ascent: max_ascent,
        descent: max_descent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{TextAlign, rich_segments_from_text};

    fn measure_rich_text_std(rt: &RichText) -> TextMetrics {
        measure_rich_text(rt, FontPolicy::Standard)
    }

    // The handwritten policy must force the bundled Comic Neue for covered
    // glyphs — different metrics from Liberation prove the swap — while
    // uncovered glyphs (CJK) fall back per character to the standard chain,
    // so sketch charts never render tofu and CJK metrics stay identical.
    #[test]
    fn handwritten_policy_swaps_covered_glyphs_and_falls_back_per_char() {
        // A registration between the two glyph-level lookups below would
        // redo the fallback decision for '가' with a fresh `FontRef`.
        let _font_registration = FONT_REGISTRATION_TEST_LOCK
            .lock()
            .expect("font registration test lock poisoned");
        let std_m = measure_plain_text("Agile 42", "", 20.0, false, false, FontPolicy::Standard);
        let hand_m =
            measure_plain_text("Agile 42", "", 20.0, false, false, FontPolicy::Handwritten);
        assert!(
            (std_m.width - hand_m.width).abs() > 0.5,
            "handwritten policy must change latin advance: std {} vs hand {}",
            std_m.width,
            hand_m.width
        );

        // Comic Neue has no Hangul coverage — per-char fallback must yield
        // the exact standard-chain metrics.
        let std_k = measure_plain_text("가나다", "", 20.0, false, false, FontPolicy::Standard);
        let hand_k = measure_plain_text("가나다", "", 20.0, false, false, FontPolicy::Handwritten);
        assert_eq!(
            std_k, hand_k,
            "uncovered glyphs must fall back per character"
        );

        // Glyph-level: covered chars resolve to the handwritten face, the
        // rest to the standard chain (including its per-glyph fallback).
        assert_eq!(
            glyph_font(FontPolicy::Handwritten, "", false, false, 'A').key,
            handwritten_font(false).key,
        );
        assert_eq!(
            glyph_font(FontPolicy::Handwritten, "", false, false, '가').key,
            glyph_font(FontPolicy::Standard, "", false, false, '가').key,
        );
        // Bold maps to the bundled bold face (real weight, not synthetic).
        assert_ne!(handwritten_font(true).key, handwritten_font(false).key);
    }

    // FontPolicy derives from the chart draw style: sketch forces the
    // handwritten face, precise keeps the standard chain.
    #[test]
    fn font_policy_follows_draw_style() {
        use crate::config::{DrawStyle, SketchOptions};
        assert_eq!(
            FontPolicy::for_style(&DrawStyle::Precise),
            FontPolicy::Standard
        );
        assert_eq!(
            FontPolicy::for_style(&DrawStyle::Sketch(SketchOptions::default())),
            FontPolicy::Handwritten,
        );
    }

    // The bundled-font glyph path must produce real ink: positive metrics
    // from measurement and non-transparent pixels from drawing.
    /// Glyph ink must scale ~linearly with the requested size across the
    /// hinting cutoff — the regression shape of the macOS field report
    /// where a hint program blew specific glyphs ('e', 't') up to several
    /// times their size at one specific ppem (51 / retina 102). A healthy
    /// pipeline keeps mask_height / size inside loose cap-height bounds at
    /// EVERY size; a hinting blowup lands far outside them.
    #[test]
    fn glyph_ink_stays_proportional_across_sizes() {
        for ch in ['e', 't', 'R', 'z'] {
            for size in [12.0_f32, 24.0, 32.0, 33.0, 40.0, 51.0, 64.0, 102.0] {
                let font = glyph_font(FontPolicy::Standard, "", false, false, ch);
                let mut seen = None;
                with_glyph_mask(font, size, ch, 0, |m| seen = Some((m.width, m.height)));
                let (w, h) = seen.expect("basic Latin glyph must rasterize");
                let rel_h = h as f32 / size;
                let rel_w = w as f32 / size;
                assert!(
                    (0.2..=1.2).contains(&rel_h) && (0.1..=1.2).contains(&rel_w),
                    "glyph '{ch}' at size {size}: mask {w}x{h} is out of proportion \
                     (rel {rel_w:.2}x{rel_h:.2}) — hinting blowup?"
                );
            }
        }
    }

    #[test]
    fn plain_text_measures_and_rasterizes_ink() {
        let m = measure_plain_text("123", "", 18.0, false, false, FontPolicy::Standard);
        assert!(m.width > 10.0, "advance too small: {}", m.width);
        assert!(m.ascent > 5.0, "ascent too small: {}", m.ascent);

        let mut canvas = Canvas::new(60, 30).unwrap();
        draw_plain_text(
            &mut canvas,
            "123",
            (5.0, 22.0),
            Color::BLACK,
            "",
            18.0,
            false,
            false,
            FontPolicy::Standard,
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
            align: TextAlign::Left,
        };
        let plain = measure_rich_text_std(&mk("V0"));
        let sub = measure_rich_text_std(&mk("V₀"));
        assert!(
            sub.descent > plain.descent,
            "subscript must extend below the baseline"
        );
        assert!(
            sub.width < plain.width,
            "subscript digit renders at reduced size"
        );
    }

    // register_font_bytes makes a family resolvable at runtime: before
    // registration an unknown family falls back to the embedded font, after
    // registration it resolves to the registered face (distinct CacheKey).
    // Liberation Sans bytes double as the fixture under an alias-free name.
    #[test]
    fn register_font_bytes_resolves_new_family_and_publishes_generation() {
        let _font_registration = FONT_REGISTRATION_TEST_LOCK
            .lock()
            .expect("font registration test lock poisoned");
        let before_invalid = font_generation();
        assert!(
            register_font_bytes(vec![1, 2, 3]).is_err(),
            "garbage must not register"
        );
        assert_eq!(
            font_generation(),
            before_invalid,
            "parse failure must not publish a font generation"
        );

        let fallback = resolve_font("Liberation Sans", false, false);
        let before_success = font_generation();
        let families = register_font_bytes(EMBEDDED_REGULAR.to_vec()).unwrap();
        assert_eq!(families, ["Liberation Sans"]);
        assert_eq!(
            font_generation(),
            before_success.checked_add(1).unwrap(),
            "successful registration must publish exactly one generation"
        );
        let after_success = font_generation();
        let duplicate = register_font_bytes(EMBEDDED_REGULAR.to_vec()).unwrap();
        assert_eq!(duplicate, families);
        assert_eq!(
            font_generation(),
            after_success,
            "registering identical bytes must not publish another generation"
        );

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

    #[test]
    fn font_family_available_reports_what_resolution_finds() {
        let _font_registration = FONT_REGISTRATION_TEST_LOCK
            .lock()
            .expect("font registration test lock poisoned");
        assert!(font_family_available(""), "empty names the bundled face");
        assert!(font_family_available(BUNDLED_FAMILY));
        assert!(!font_family_available("Definitely Not A Font"));
        assert!(
            !font_family_available("liberation sans"),
            "matching is resolution's exact, case-sensitive family match"
        );
        assert!(!font_family_available("sans-serif"));

        // A registered family becomes available.
        let families = register_font_bytes(HANDWRITTEN_REGULAR.to_vec()).unwrap();
        assert_eq!(families, ["Comic Neue"]);
        assert!(font_family_available("Comic Neue"));
        // And it is the face resolution now returns, not the fallback.
        assert_ne!(
            resolve_font("Comic Neue", false, false).key,
            embedded_font(false, false).key
        );
    }

    #[test]
    fn font_generation_exhaustion_is_fail_closed_before_registry_mutation() {
        let mut fonts = RegisteredFonts::new();

        let error = fonts
            .register(EMBEDDED_REGULAR.to_vec(), u64::MAX)
            .expect_err("exhausted generation must reject registration");

        assert_eq!(error, "font generation counter exhausted");
        assert_eq!(fonts.db.faces().count(), 0);
        assert!(fonts.families_by_bytes.is_empty());
    }

    #[test]
    fn identical_font_bytes_are_stored_once() {
        let mut fonts = RegisteredFonts::new();
        let bytes = EMBEDDED_REGULAR.to_vec();

        let (first_families, first_generation) = fonts.register(bytes.clone(), 41).unwrap();
        let face_count = fonts.db.faces().count();
        let (second_families, second_generation) = fonts.register(bytes, 42).unwrap();

        assert_eq!(first_families, ["Liberation Sans"]);
        assert_eq!(second_families, first_families);
        assert_eq!(first_generation, Some(42));
        assert_eq!(second_generation, None);
        assert_eq!(fonts.families_by_bytes.len(), 1);
        assert_eq!(
            fonts.db.faces().count(),
            face_count,
            "identical bytes must not add duplicate fontdb faces"
        );

        let first_face = fonts.lookup("Liberation Sans", false, false).unwrap();
        let resolved_count = fonts.resolved_faces.len();
        let second_face = fonts.lookup("Liberation Sans", false, false).unwrap();
        assert_eq!(second_face.data.as_ptr(), first_face.data.as_ptr());
        assert_eq!(
            second_face.key, first_face.key,
            "one parsed face, one cache key"
        );
        assert_eq!(
            fonts.resolved_faces.len(),
            resolved_count,
            "resolving the same face again must reuse its stable backing bytes"
        );
    }

    const STYLES: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];
    const FALLBACK_SIZE: f32 = 24.0;
    /// A noncharacter no face maps, so it draws the primary face's `.notdef`.
    const NOTDEF_TEXT: &str = "\u{10FFFF}";

    /// Draw single-style `runs` of (text, left x) onto one fresh canvas, in
    /// order, on a shared baseline.
    fn draw_runs(
        runs: &[(&str, f32)],
        family: &str,
        bold: bool,
        italic: bool,
        policy: FontPolicy,
    ) -> Vec<u8> {
        let mut canvas = Canvas::new(160, 40).unwrap();
        for &(text, x) in runs {
            draw_plain_text(
                &mut canvas,
                text,
                (x, 28.0),
                Color::BLACK,
                family,
                FALLBACK_SIZE,
                bold,
                italic,
                policy,
            );
        }
        canvas.into_rgba()
    }

    fn has_ink(rgba: &[u8]) -> bool {
        rgba.chunks_exact(4).any(|p| p[3] > 0)
    }

    // A character a registered Latin-only family lacks (Comic Neue has no
    // Greek) takes a face that draws it: measurement advances exactly as far
    // as drawing does, and the ink is not Comic Neue's `.notdef` box.
    #[test]
    fn missing_glyph_in_registered_family_measures_as_it_draws() {
        let _font_registration = FONT_REGISTRATION_TEST_LOCK
            .lock()
            .expect("font registration test lock poisoned");
        let families = register_font_bytes(HANDWRITTEN_REGULAR.to_vec()).unwrap();
        assert_eq!(families, ["Comic Neue"]);
        for (bold, italic) in STYLES {
            let policy = FontPolicy::Standard;
            let measure = |text: &str| {
                measure_plain_text(text, "Comic Neue", FALLBACK_SIZE, bold, italic, policy)
            };
            let draw = |runs: &[(&str, f32)]| draw_runs(runs, "Comic Neue", bold, italic, policy);
            let alpha = measure("α");
            assert!(
                draw(&[("αX", 4.0)]) == draw(&[("α", 4.0), ("X", 4.0 + alpha.width)]),
                "the pen after 'α' (bold={bold} italic={italic}) must sit where \
                 measurement ends it"
            );
            let mut segments = rich_segments_from_text("α");
            segments[0].bold = bold;
            segments[0].italic = italic;
            let rich = RichText {
                segments,
                color: Color::BLACK,
                font_size: FALLBACK_SIZE,
                font: "Comic Neue".to_string(),
                align: crate::text::TextAlign::Left,
            };
            assert_eq!(measure_rich_text(&rich, policy).width, alpha.width);

            let drawn = draw(&[("α", 4.0)]);
            assert!(has_ink(&drawn), "fallback 'α' must ink");
            assert!(
                drawn != draw(&[(NOTDEF_TEXT, 4.0)]),
                "'α' (bold={bold} italic={italic}) must not be the .notdef box"
            );
            assert_ne!(alpha, measure(NOTDEF_TEXT));
        }
    }

    // The per-glyph fallback sits behind the handwritten face: under the
    // sketch policy every glyph Comic Neue covers keeps the bundled
    // handwritten face whatever the family — including a registered copy of
    // Comic Neue, which rasterizes differently (hinted, not emboldened).
    #[test]
    fn sketch_policy_keeps_comic_neue_for_covered_glyphs() {
        let _font_registration = FONT_REGISTRATION_TEST_LOCK
            .lock()
            .expect("font registration test lock poisoned");
        register_font_bytes(HANDWRITTEN_REGULAR.to_vec()).unwrap();
        let text = "Agile 42";
        let policy = FontPolicy::Handwritten;
        for (bold, italic) in STYLES {
            let measure = |family: &str| {
                measure_plain_text(text, family, FALLBACK_SIZE, bold, italic, policy)
            };
            let draw = |family: &str| draw_runs(&[(text, 4.0)], family, bold, italic, policy);
            let standard =
                measure_plain_text(text, "", FALLBACK_SIZE, bold, italic, FontPolicy::Standard);
            assert_ne!(measure(""), standard, "fixture: sketch swaps the face");
            for family in ["Comic Neue", BUNDLED_FAMILY, "Definitely Not A Font"] {
                assert_eq!(measure(family), measure(""), "{family:?} bold={bold}");
                assert!(
                    draw(family) == draw(""),
                    "{family:?} (bold={bold} italic={italic}) must keep the handwritten face"
                );
            }
        }
    }

    // A fallback decision is redone when a registration publishes a new font
    // generation. Only this test registers the Liberation Sans italic faces:
    // first the italic face is the registered match for a bold-italic 'α'
    // (style before weight), then the bold-italic face takes over.
    #[test]
    fn registering_a_covering_face_changes_the_fallback() {
        let _font_registration = FONT_REGISTRATION_TEST_LOCK
            .lock()
            .expect("font registration test lock poisoned");
        register_font_bytes(HANDWRITTEN_REGULAR.to_vec()).unwrap();
        let measure = |family: &str, bold: bool, italic: bool| {
            measure_plain_text(
                "α",
                family,
                FALLBACK_SIZE,
                bold,
                italic,
                FontPolicy::Standard,
            )
        };
        let draw = |family: &str, bold: bool, italic: bool| {
            draw_runs(&[("α", 4.0)], family, bold, italic, FontPolicy::Standard)
        };
        let italic = register_font_bytes(EMBEDDED_ITALIC.to_vec()).unwrap();
        assert_eq!(italic, [BUNDLED_FAMILY]);
        let before = measure("Comic Neue", true, true);
        assert_eq!(
            before,
            measure("", false, true),
            "the italic face draws 'α'"
        );
        assert!(draw("Comic Neue", true, true) == draw("", false, true));

        let bold_italic = register_font_bytes(EMBEDDED_BOLD_ITALIC.to_vec()).unwrap();
        assert_eq!(bold_italic, [BUNDLED_FAMILY]);
        let after = measure("Comic Neue", true, true);
        assert_ne!(after, before, "the new registration must be seen");
        assert_eq!(
            after,
            measure("", true, true),
            "the bold-italic face draws 'α'"
        );
        assert!(draw("Comic Neue", true, true) == draw("", true, true));
    }

    // A zero-width space inside a Comic Neue run (which lacks U+200B) takes
    // a face's empty glyph instead of Comic Neue's `.notdef` box.
    #[test]
    fn zero_width_space_in_comic_neue_draws_no_box() {
        let _font_registration = FONT_REGISTRATION_TEST_LOCK
            .lock()
            .expect("font registration test lock poisoned");
        register_font_bytes(HANDWRITTEN_REGULAR.to_vec()).unwrap();
        for (family, policy) in [
            ("Comic Neue", FontPolicy::Standard),
            ("Comic Neue", FontPolicy::Handwritten),
            ("", FontPolicy::Handwritten),
        ] {
            let measure =
                |text: &str| measure_plain_text(text, family, FALLBACK_SIZE, false, false, policy);
            let draw = |text: &str| draw_runs(&[(text, 4.0)], family, false, false, policy);
            assert_eq!(
                measure("a\u{200B}b"),
                measure("ab"),
                "{family:?} {policy:?}"
            );
            assert!(
                draw("a\u{200B}b") == draw("ab"),
                "{family:?} {policy:?}: U+200B must draw nothing"
            );
        }
    }

    /// Whether `font` maps `ch` to a glyph with an outline. A test-local
    /// check, independent of the coverage helper the fallback chain uses.
    #[cfg(not(target_arch = "wasm32"))]
    fn outline_drawn(font: FontRef<'_>, ch: char, scale: &mut ScaleContext) -> bool {
        let glyph = font.charmap().map(ch);
        glyph != 0
            && scale
                .builder(font)
                .size(FALLBACK_SIZE)
                .build()
                .scale_outline(glyph)
                .is_some_and(|outline| !outline.is_empty())
    }

    /// Whether any face installed on this host draws `ch`, in any style.
    /// This runs its own system scan, so it does not depend on the fallback
    /// chain: if the chain's system step breaks, the test fails instead of
    /// skipping.
    #[cfg(not(target_arch = "wasm32"))]
    fn installed_face_draws(ch: char) -> bool {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut scale = ScaleContext::new();
        db.faces().any(|face| {
            db.with_face_data(face.id, |data, index| {
                FontRef::from_index(data, index as usize)
                    .is_some_and(|font| outline_drawn(font, ch, &mut scale))
            })
            .unwrap_or(false)
        })
    }

    // U+207B (superscript minus) is missing from every bundled face, and
    // tests register only bundled faces, so with the default font only an
    // installed system face can draw it. Installed fonts vary across CI
    // hosts: the assertion runs only when some installed face draws it.
    // The chain picks one face per family per requested style (CSS
    // matching), and the covering face may be another style, so at least
    // one of the four styles must draw a real glyph.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn superscript_minus_draws_from_an_installed_face_when_one_covers_it() {
        let _font_registration = FONT_REGISTRATION_TEST_LOCK
            .lock()
            .expect("font registration test lock poisoned");
        let ch = '\u{207B}';
        let mut scale = ScaleContext::new();
        let bundled = [
            EMBEDDED_REGULAR,
            EMBEDDED_BOLD,
            EMBEDDED_ITALIC,
            EMBEDDED_BOLD_ITALIC,
            HANDWRITTEN_REGULAR,
            HANDWRITTEN_BOLD,
        ];
        for bytes in bundled {
            let font = FontRef::from_index(bytes, 0).unwrap();
            assert!(!outline_drawn(font, ch, &mut scale), "fixture");
        }
        if !installed_face_draws(ch) {
            return;
        }
        let policy = FontPolicy::Standard;
        let mut real_glyph = false;
        for (bold, italic) in STYLES {
            let minus = measure_plain_text("\u{207B}", "", FALLBACK_SIZE, bold, italic, policy);
            let draw = |runs: &[(&str, f32)]| draw_runs(runs, "", bold, italic, policy);
            assert!(
                draw(&[("\u{207B}X", 4.0)]) == draw(&[("\u{207B}", 4.0), ("X", 4.0 + minus.width)]),
                "the pen after U+207B (bold={bold} italic={italic}) must sit where \
                 measurement ends it"
            );
            let drawn = draw(&[("\u{207B}", 4.0)]);
            real_glyph |= has_ink(&drawn) && drawn != draw(&[(NOTDEF_TEXT, 4.0)]);
        }
        assert!(
            real_glyph,
            "an installed face draws U+207B, so the default font must draw it in \
             at least one style instead of the .notdef box"
        );
    }

    // Every legend symbol form must measure EXACTLY the same width at the
    // same font size — fixed fields, not glyph metrics, set the advance.
    #[test]
    fn legend_symbol_forms_share_exact_width() {
        use crate::config::{LegendEntryKind, symbol_segments};
        let mk = |kind: &LegendEntryKind| RichText {
            segments: symbol_segments(kind, Color::BLACK),
            color: Color::BLACK,
            font_size: 16.0,
            font: String::new(),
            align: TextAlign::Left,
        };
        let line = measure_rich_text_std(&mk(&LegendEntryKind::Line));
        let scatter = measure_rich_text_std(&mk(&LegendEntryKind::Scatter));
        let combo = measure_rich_text_std(&mk(&LegendEntryKind::LineScatter));
        assert!(
            (line.width - 32.0).abs() < 0.01,
            "field = 2.0 em × 16 px, got {}",
            line.width
        );
        assert!((line.width - scatter.width).abs() < 0.01);
        assert!((line.width - combo.width).abs() < 0.01);

        // The rule must actually ink (nearly) the whole field — a drawn bar,
        // not a short dash glyph centered in space.
        let mut canvas = Canvas::new(48, 32).unwrap();
        draw_rich_text(
            &mut canvas,
            &mk(&LegendEntryKind::Line),
            (4.0, 20.0),
            FontPolicy::Standard,
        );
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
            align: TextAlign::Left,
        };
        // Line 1 pre-tab cell ("WWW") is much wider than line 2's ("i").
        let rt = mk("WWW\tX\ni\tX");
        let m = measure_rich_text_std(&rt);
        let wide = measure_rich_text_std(&mk("WWW"));
        let x_w = measure_rich_text_std(&mk("X"));
        assert!(
            (m.width - (wide.width + x_w.width)).abs() < 0.5,
            "doc width {} must be widest cell0 {} + cell1 {}",
            m.width,
            wide.width,
            x_w.width
        );

        // Drawing: the leftmost ink of each line's second cell must agree.
        let mut canvas = Canvas::new(120, 60).unwrap();
        draw_rich_text(&mut canvas, &rt, (4.0, 18.0), FontPolicy::Standard);
        let rgba = canvas.into_rgba();
        let leftmost_ink = |y0: u32, y1: u32| -> Option<u32> {
            (0..120u32).find(|&x| (y0..y1).any(|y| rgba[((y * 120 + x) * 4 + 3) as usize] > 0))
        };
        // Second-cell x ranges: scan only right of the widest first cell.
        let cut = (4.0 + wide.width - 0.5) as u32;
        let second_ink = |y0: u32, y1: u32| -> Option<u32> {
            (cut..120u32).find(|&x| (y0..y1).any(|y| rgba[((y * 120 + x) * 4 + 3) as usize] > 0))
        };
        let line1_x = second_ink(4, 22).expect("line 1 second cell ink");
        let line2_x = second_ink(24, 45).expect("line 2 second cell ink");
        assert!(
            line1_x.abs_diff(line2_x) <= 1,
            "second-column ink must align: line1 at {line1_x}, line2 at {line2_x}"
        );
        let _ = leftmost_ink;
    }

    fn aligned(s: &str, align: TextAlign) -> RichText {
        RichText {
            align,
            ..RichText::plain(s, Color::BLACK, 16.0, "")
        }
    }

    fn draw_to_rgba(rt: &RichText, size: (u32, u32), origin: (f32, f32)) -> Vec<u8> {
        let mut canvas = Canvas::new(size.0, size.1).unwrap();
        draw_rich_text(&mut canvas, rt, origin, FontPolicy::Standard);
        canvas.into_rgba()
    }

    /// Leftmost inked column at or right of `x0` within `rows`.
    fn leftmost_ink(rgba: &[u8], width: u32, rows: std::ops::Range<u32>, x0: u32) -> Option<u32> {
        (x0..width).find(|&x| {
            rows.clone()
                .any(|y| rgba[((y * width + x) * 4 + 3) as usize] > 0)
        })
    }

    // `Left` is the layout every text had before alignment existed. This is
    // that layout, verbatim — each line's cells at `x + col_start` — and the
    // aligned draw must reproduce it pixel for pixel, tab columns included.
    #[test]
    fn left_alignment_reproduces_the_unaligned_layout_exactly() {
        fn unaligned_draw(canvas: &mut Canvas, rt: &RichText, origin: (f32, f32)) {
            let doc = layout_doc(rt, FontPolicy::Standard);
            let (x, mut baseline) = origin;
            for (i, cells) in doc.line_cells.iter().enumerate() {
                if i > 0 {
                    baseline += line_advance(&doc.line_metrics[i - 1], &doc.line_metrics[i], rt);
                }
                for (k, cell) in cells.iter().enumerate() {
                    let cell_origin = (x + doc.col_starts[k], baseline);
                    draw_line_segments(canvas, cell, rt, cell_origin, FontPolicy::Standard);
                }
            }
        }

        for text in [
            "WWW\tX\ni\tlonger cell\nmid",
            "short\na much longer line\n\nx",
        ] {
            let rt = aligned(text, TextAlign::Left);
            for origin in [(10.25, 30.5), (-0.0, 18.0)] {
                let mut expected = Canvas::new(300, 120).unwrap();
                unaligned_draw(&mut expected, &rt, origin);
                let expected = expected.into_rgba();
                assert!(expected.chunks_exact(4).any(|p| p[3] > 0));
                assert!(
                    draw_to_rgba(&rt, (300, 120), origin) == expected,
                    "{text:?} at {origin:?}"
                );
            }
        }
    }

    // Center/Right move each line by half/all of its slack (block width minus
    // line width). The widest line has none and stays put, and the measured
    // block envelope is the same for every alignment.
    #[test]
    fn center_and_right_offset_each_line_by_its_slack() {
        let text = "WWWWWW\nii";
        let left = measure_rich_text_std(&aligned(text, TextAlign::Left));
        for align in [TextAlign::Center, TextAlign::Right] {
            assert_eq!(measure_rich_text_std(&aligned(text, align)), left);
        }
        let slack = left.width - measure_rich_text_std(&aligned("ii", TextAlign::Left)).width;
        assert!(slack > 40.0, "fixture needs a visible slack, got {slack}");

        let (w, h, origin) = (200u32, 60u32, (20.0, 20.0));
        let rgba = |align| draw_to_rgba(&aligned(text, align), (w, h), origin);
        let (l, c, r) = (
            rgba(TextAlign::Left),
            rgba(TextAlign::Center),
            rgba(TextAlign::Right),
        );
        // Rows down to the first baseline hold only the widest line's ink.
        let first_line = |rgba: &[u8]| rgba[..(21 * w * 4) as usize].to_vec();
        assert!(first_line(&c) == first_line(&l) && first_line(&r) == first_line(&l));

        let second_line_x = |rgba: &[u8]| leftmost_ink(rgba, w, 24..h, 0).unwrap() as f32;
        let base = second_line_x(&l);
        for (rgba, expected) in [(&c, slack * 0.5), (&r, slack)] {
            let moved = second_line_x(rgba) - base;
            assert!(
                (moved - expected).abs() <= 1.0,
                "second line moved {moved} px, expected {expected}"
            );
        }
    }

    // A line's tab-column cells move with it: under Right alignment the short
    // line's first cell and its second cell shift by the same slack, so the
    // cell layout inside the line is preserved.
    #[test]
    fn tab_cells_move_with_their_aligned_line() {
        let text = "WWW\tXXX\ni\tX";
        let x_w = measure_rich_text_std(&aligned("X", TextAlign::Left)).width;
        let slack = 2.0 * x_w;
        let wide = measure_rich_text_std(&aligned("WWW", TextAlign::Left)).width;

        let (w, h, origin) = (200u32, 60u32, (10.0, 20.0));
        let l = draw_to_rgba(&aligned(text, TextAlign::Left), (w, h), origin);
        let r = draw_to_rgba(&aligned(text, TextAlign::Right), (w, h), origin);
        let rows = 24..h;
        let cut = (origin.0 + wide - 0.5) as u32;
        let first_cell = |rgba: &[u8]| leftmost_ink(rgba, w, rows.clone(), 0).unwrap() as f32;
        let second_cell =
            |rgba: &[u8], from: u32| leftmost_ink(rgba, w, rows.clone(), from).unwrap() as f32;
        let first_moved = first_cell(&r) - first_cell(&l);
        let second_moved = second_cell(&r, cut + slack as u32) - second_cell(&l, cut);
        for moved in [first_moved, second_moved] {
            assert!(
                (moved - slack).abs() <= 1.0,
                "cell moved {moved} px, expected the line slack {slack}"
            );
        }
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
            align: TextAlign::Left,
        };
        let one = measure_rich_text_std(&mk("abc"));
        let two = measure_rich_text_std(&mk("abc\nde"));
        assert!(
            two.height() > one.height() * 1.7,
            "second line must add a full line"
        );
        assert!(
            (two.width - one.width).abs() < 0.01,
            "width is the widest line (abc)"
        );

        let mut canvas = Canvas::new(60, 60).unwrap();
        draw_rich_text(
            &mut canvas,
            &mk("ab\nab"),
            (5.0, 18.0),
            FontPolicy::Standard,
        );
        let rgba = canvas.into_rgba();
        let row_has_ink = |y: u32| (0..60u32).any(|x| rgba[((y * 60 + x) * 4 + 3) as usize] > 0);
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
            align: TextAlign::Left,
        };

        let mut canvas = Canvas::new(60, 40).unwrap();
        draw_rich_text(&mut canvas, &rt, (5.0, 30.0), FontPolicy::Standard);
        let rgba = canvas.into_rgba();
        let red_ink = rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 200 && p[0] > 150 && p[1] < 50 && p[2] < 50)
            .count();
        let black_ink = rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 200 && p[0] < 50 && p[1] < 50 && p[2] < 50)
            .count();
        assert!(
            red_ink > 5,
            "override segment must ink red, got {red_ink} px"
        );
        assert!(
            black_ink > 5,
            "non-override segment keeps the default, got {black_ink} px"
        );
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
                align: TextAlign::Left,
            }
        };
        let plain = measure_rich_text_std(&mk(false));
        let overridden = measure_rich_text_std(&mk(true));
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
            align: TextAlign::Left,
        };
        let sup = RichText {
            segments: rich_segments_from_text("10³"), // '³' maps to a superscript segment
            color: Color::BLACK,
            font_size: 18.0,
            font: String::new(),
            align: TextAlign::Left,
        };
        let m_plain = measure_rich_text_std(&plain);
        let m_sup = measure_rich_text_std(&sup);
        assert!(
            m_sup.ascent > m_plain.ascent,
            "superscript must raise the envelope"
        );

        let mut canvas = Canvas::new(80, 40).unwrap();
        draw_rich_text(&mut canvas, &sup, (5.0, 30.0), FontPolicy::Standard);
        let rgba = canvas.into_rgba();
        assert!(rgba.chunks_exact(4).any(|p| p[3] > 0));
    }
}
