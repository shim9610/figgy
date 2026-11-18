//! The math face: the bundled New Computer Modern Math and the reader of its
//! OpenType MATH table (constants, italic corrections, top accent
//! attachments, glyph variants and assemblies) and of the
//! GSUB `ssty` script alternates.
//!
//! The face's bytes are parsed once into a [`FontRef`] in a `OnceLock`, as
//! the bundled text faces are, so the glyph caches key it stably. A
//! [`MathFace`] is a cheap view built from the face's table slices whenever
//! a cell holds math; nothing is cached and nothing allocates. Values are in
//! font units; [`MathFace::px`] scales one to a size in pixels.

use std::sync::OnceLock;

use read_fonts::tables::gsub::{AlternateSubst, Gsub, SingleSubst, SubstitutionSubtables};
use read_fonts::tables::head::Head;
use read_fonts::tables::math::{
    GlyphAssembly, GlyphPartRecord, Math, MathConstants, MathValueRecord, PartFlags, StretchAxis,
};
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontData, FontRead};
use swash::FontRef;

// Bundled math face (GUST Font License, fonts/LICENSE-NewCMMath.txt):
// New Computer Modern Math, shipped verbatim.
static NEW_CM_MATH: &[u8] = include_bytes!("../../fonts/NewCMMath-Regular.otf");

/// The bundled math face, parsed once.
pub(in crate::text_render) fn bundled_math_font() -> FontRef<'static> {
    static FACE: OnceLock<FontRef<'static>> = OnceLock::new();
    *FACE.get_or_init(|| super::bundled_face(NEW_CM_MATH))
}

/// A face with a MATH table, read through `read-fonts`.
#[derive(Clone)]
pub(in crate::text_render) struct MathFace {
    pub(in crate::text_render) font: FontRef<'static>,
    upem: f32,
    constants: MathConstants<'static>,
    math: Math<'static>,
    gsub: Option<Gsub<'static>>,
}

/// A glyph stretched to a size: one glyph (the base or a size variant), or
/// an assembly of parts with its repeat count and connector overlap.
#[derive(Clone)]
pub(in crate::text_render) enum Stretched {
    Glyph(u16),
    Assembly(Assembly),
}

/// A glyph assembly laid out to a size, in font units.
#[derive(Clone)]
pub(in crate::text_render) struct Assembly {
    pub(in crate::text_render) assembly: GlyphAssembly<'static>,
    /// How many times each extender part repeats.
    pub(in crate::text_render) repeats: u16,
    /// Overlap of every connection between consecutive parts.
    pub(in crate::text_render) overlap: f32,
    /// Length along the stretch direction.
    pub(in crate::text_render) length: f32,
}

fn is_extender(part: &GlyphPartRecord) -> bool {
    part.part_flags().contains(PartFlags::EXTENDER_FLAG)
}

impl Assembly {
    /// Every part in drawing order (bottom to top, or left to right), the
    /// extenders repeated, with its start offset along the stretch
    /// direction, in font units. Walks the part array; nothing is stored.
    pub(in crate::text_render) fn for_each_part(&self, mut f: impl FnMut(GlyphPartRecord, f32)) {
        let mut offset = 0.0f32;
        let mut first = true;
        for part in self.assembly.part_records() {
            let count = if is_extender(part) { self.repeats } else { 1 };
            for _ in 0..count {
                if !first {
                    offset -= self.overlap;
                }
                first = false;
                f(*part, offset);
                offset += f32::from(part.full_advance().to_u16());
            }
        }
    }
}

impl MathFace {
    /// The math face of a text whose family is `family`: the family's face
    /// when it has a MATH table with constants, the bundled face otherwise.
    /// `None` only if the bundled face has no MATH table (a corrupted
    /// bundle; a test checks it has one): math then lays out flat.
    pub(in crate::text_render) fn for_family(family: &str) -> Option<MathFace> {
        if !family.is_empty()
            && let Some(face) = MathFace::of(super::resolve_font(family, false, false))
        {
            return Some(face);
        }
        MathFace::of(bundled_math_font())
    }

    /// The MATH table view of `font`, or `None` when it has none.
    fn of(font: FontRef<'static>) -> Option<MathFace> {
        let table = |tag: &[u8; 4]| font.table(swash::tag_from_bytes(tag)).map(FontData::new);
        let head = Head::read(table(b"head")?).ok()?;
        let math = Math::read(table(b"MATH")?).ok()?;
        let constants = math.math_constants().ok()?;
        Some(MathFace {
            font,
            upem: f32::from(head.units_per_em()),
            constants,
            math,
            gsub: table(b"GSUB").and_then(|data| Gsub::read(data).ok()),
        })
    }

    /// `value` font units at a font size of `size` pixels.
    pub(in crate::text_render) fn px(&self, value: f32, size: f32) -> f32 {
        value * size / self.upem
    }

    /// `px` pixels at a font size of `size` pixels, in font units.
    pub(in crate::text_render) fn units(&self, px: f32, size: f32) -> f32 {
        px * self.upem / size.max(1e-3)
    }

    /// The MATH constants.
    pub(in crate::text_render) fn constants(&self) -> &MathConstants<'static> {
        &self.constants
    }

    /// A MATH constant in pixels at `size`.
    pub(in crate::text_render) fn value(&self, v: &MathValueRecord, size: f32) -> f32 {
        self.px(f32::from(v.value().to_i16()), size)
    }

    /// The glyph `c` maps to; 0 when the face lacks it.
    pub(in crate::text_render) fn glyph(&self, c: char) -> u16 {
        self.font.charmap().map(c)
    }

    /// The italic correction of `glyph`, in font units (0 when none).
    pub(in crate::text_render) fn italic_correction(&self, glyph: u16) -> f32 {
        self.math
            .math_glyph_info()
            .ok()
            .and_then(|info| info.math_italics_correction_info()?.ok())
            .and_then(|info| info.correction(GlyphId::new(u32::from(glyph))))
            .map_or(0.0, |v| v as f32)
    }

    /// The top accent attachment of `glyph`, in font units from its origin.
    pub(in crate::text_render) fn top_accent(&self, glyph: u16) -> Option<f32> {
        self.math
            .math_glyph_info()
            .ok()
            .and_then(|info| info.math_top_accent_attachment()?.ok())
            .and_then(|info| info.attachment(GlyphId::new(u32::from(glyph))))
            .map(|v| v as f32)
    }

    /// The `ssty` alternate of `glyph` for script level `level` (1 for
    /// script, 2 for scriptscript), or `glyph` when the face has none (D5).
    /// An alternate substitution takes the level's alternate (or the last
    /// one); a single substitution applies at any level. The 24-bit glyph
    /// formats cannot name a `u16` glyph's substitute and are skipped.
    pub(in crate::text_render) fn script_alternate(&self, glyph: u16, level: u8) -> u16 {
        if level == 0 {
            return glyph;
        }
        let Some(gsub) = &self.gsub else {
            return glyph;
        };
        let (Ok(features), Ok(lookups)) = (gsub.feature_list(), gsub.lookup_list()) else {
            return glyph;
        };
        let ssty = Tag::new(b"ssty");
        let Some(feature) = features
            .feature_records()
            .iter()
            .find(|record| record.feature_tag() == ssty)
            .and_then(|record| record.feature(features.offset_data()).ok())
        else {
            return glyph;
        };
        let id = GlyphId::new(u32::from(glyph));
        let pick = usize::from(level) - 1;
        for index in feature.lookup_list_indices() {
            let Ok(lookup) = lookups.lookups().get(usize::from(index.get())) else {
                continue;
            };
            match lookup.subtables() {
                Ok(SubstitutionSubtables::Alternate(subtables)) => {
                    for subtable in subtables.iter().flatten() {
                        let AlternateSubst::Format1(alt) = subtable else {
                            continue;
                        };
                        let Some(set) = alt
                            .coverage()
                            .ok()
                            .and_then(|c| c.get(id))
                            .and_then(|i| alt.alternate_sets().get(i as usize).ok())
                        else {
                            continue;
                        };
                        let alternates = set.alternate_glyph_ids();
                        if let Some(g) =
                            alternates.get(pick.min(alternates.len().saturating_sub(1)))
                        {
                            return g.get().to_u16();
                        }
                    }
                }
                Ok(SubstitutionSubtables::Single(subtables)) => {
                    for subtable in subtables.iter().flatten() {
                        let hit = match subtable {
                            SingleSubst::Format1(single) => {
                                single.coverage().ok().and_then(|c| c.get(id)).map(|_| {
                                    (i32::from(glyph) + i32::from(single.delta_glyph_id())) as u16
                                })
                            }
                            SingleSubst::Format2(single) => single
                                .coverage()
                                .ok()
                                .and_then(|c| c.get(id))
                                .and_then(|i| single.substitute_glyph_ids().get(i as usize))
                                .map(|g| g.get().to_u16()),
                            SingleSubst::Format3(_) | SingleSubst::Format4(_) => None,
                        };
                        if let Some(g) = hit {
                            return g;
                        }
                    }
                }
                _ => {}
            }
        }
        glyph
    }

    /// `glyph` stretched along one direction to at least `target` font
    /// units: the first size variant that reaches it, else the assembly
    /// (MathML Core: the fewest extender repeats that reach it, the overlap
    /// spread evenly), else the largest variant. A glyph with no
    /// construction stays as it is.
    pub(in crate::text_render) fn stretch(
        &self,
        glyph: u16,
        target: f32,
        horizontal: bool,
    ) -> Stretched {
        let axis = if horizontal {
            StretchAxis::Horizontal
        } else {
            StretchAxis::Vertical
        };
        let Some(construction) =
            self.math.math_variants().ok().and_then(|variants| {
                variants.glyph_construction(GlyphId::new(u32::from(glyph)), axis)
            })
        else {
            return Stretched::Glyph(glyph);
        };
        let mut largest = glyph;
        for variant in construction.math_glyph_variant_records() {
            largest = variant.variant_glyph().to_u16();
            if f32::from(variant.advance_measurement().to_u16()) >= target {
                return Stretched::Glyph(largest);
            }
        }
        match construction
            .glyph_assembly()
            .and_then(Result::ok)
            .and_then(|assembly| self.assemble(assembly, target))
        {
            Some(assembly) => Stretched::Assembly(assembly),
            None => Stretched::Glyph(largest),
        }
    }

    /// The assembly of `assembly` reaching `target` font units.
    fn assemble(&self, assembly: GlyphAssembly<'static>, target: f32) -> Option<Assembly> {
        let min_overlap = self
            .math
            .math_variants()
            .map_or(0, |v| v.min_connector_overlap().to_u16());
        let min_overlap = f32::from(min_overlap);
        // First pass: the fixed and the repeated parts.
        let (mut fixed, mut fixed_len, mut ext, mut ext_len) = (0u32, 0.0f32, 0u32, 0.0f32);
        for part in assembly.part_records() {
            let advance = f32::from(part.full_advance().to_u16());
            if is_extender(part) {
                ext += 1;
                ext_len += advance;
            } else {
                fixed += 1;
                fixed_len += advance;
            }
        }
        if fixed + ext == 0 {
            return None;
        }
        let length = |repeats: u32, overlap: f32| {
            let count = fixed + repeats * ext;
            fixed_len + repeats as f32 * ext_len - count.saturating_sub(1) as f32 * overlap
        };
        // The fewest repeats reaching the target with the least overlap.
        let mut repeats = 0u32;
        if ext > 0 {
            while length(repeats, min_overlap) < target && repeats < 512 {
                repeats += 1;
            }
            // An extender must appear at least once between fixed parts.
            repeats = repeats.max(1);
        }
        let count = fixed + repeats * ext;
        // Second pass: the largest overlap every connection allows.
        let mut max_overlap = f32::INFINITY;
        let mut previous: Option<GlyphPartRecord> = None;
        let probe = Assembly {
            assembly,
            repeats: repeats as u16,
            overlap: 0.0,
            length: 0.0,
        };
        probe.for_each_part(|part, _| {
            if let Some(prev) = previous {
                let allowed = prev
                    .end_connector_length()
                    .to_u16()
                    .min(part.start_connector_length().to_u16());
                max_overlap = max_overlap.min(f32::from(allowed));
            }
            previous = Some(part);
        });
        let overlap = if count > 1 {
            let spread = (length(repeats, 0.0) - target) / (count - 1) as f32;
            spread.min(max_overlap).max(min_overlap.min(max_overlap))
        } else {
            0.0
        };
        Some(Assembly {
            assembly: probe.assembly,
            repeats: repeats as u16,
            overlap,
            length: length(repeats, overlap),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face() -> MathFace {
        MathFace::for_family("").unwrap()
    }

    #[test]
    fn the_bundled_face_reads_its_math_table() {
        let f = face();
        assert_eq!(f.upem, 1000.0);
        assert_eq!(f.constants().axis_height().value().to_i16(), 250);
        assert_eq!(f.constants().script_percent_scale_down(), 70);
        assert_eq!(f.px(250.0, 20.0), 5.0);
        // Italic f has an italic correction; upright digits do not.
        let italic_f = f.glyph('\u{1D453}');
        assert_ne!(italic_f, 0);
        assert!(f.italic_correction(italic_f) > 0.0);
        assert_eq!(f.italic_correction(f.glyph('1')), 0.0);
        // A family without a MATH table falls back to the bundled face.
        assert_eq!(
            MathFace::for_family("Liberation Sans").unwrap().font.key,
            bundled_math_font().key
        );
    }

    #[test]
    fn script_alternates_follow_the_level() {
        let f = face();
        let prime = f.glyph('′');
        let script = f.script_alternate(prime, 1);
        let scriptscript = f.script_alternate(prime, 2);
        assert_ne!(script, prime, "the prime has an ssty alternate (D5)");
        assert_ne!(scriptscript, script);
        assert_eq!(f.script_alternate(prime, 0), prime);
    }

    #[test]
    fn stretching_picks_a_variant_then_an_assembly() {
        let f = face();
        let paren = f.glyph('(');
        // The base glyph covers its own size.
        assert!(matches!(f.stretch(paren, 900.0, false), Stretched::Glyph(g) if g == paren));
        // A larger target picks a larger variant.
        assert!(matches!(f.stretch(paren, 1400.0, false), Stretched::Glyph(g) if g != paren));
        // Past the largest variant (2991 units), the assembly reaches the
        // target exactly or by the least excess its connectors allow.
        match f.stretch(paren, 6000.0, false) {
            Stretched::Assembly(a) => {
                assert!(a.length >= 6000.0 - 0.5, "{}", a.length);
                assert!(a.overlap >= 20.0);
                let mut parts = 0;
                let mut last_end = 0.0f32;
                a.for_each_part(|part, offset| {
                    parts += 1;
                    last_end = offset + f32::from(part.full_advance().to_u16());
                });
                assert!(parts >= 3);
                assert!((last_end - a.length).abs() < 0.01);
            }
            Stretched::Glyph(_) => panic!("expected an assembly"),
        }
        // A horizontal brace stretches too.
        let brace = f.glyph('⏞');
        assert!(matches!(
            f.stretch(brace, 9000.0, true),
            Stretched::Assembly(_)
        ));
        // A glyph with no construction stays.
        let x = f.glyph('x');
        assert!(matches!(f.stretch(x, 5000.0, false), Stretched::Glyph(g) if g == x));
    }
}
