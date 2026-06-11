use crate::color::Color;

/// A rich text value composed of styled segments.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RichText {
    pub segments: Vec<RichSegment>,
    pub color: Color,
    pub font_size: f32,
    /// Font family name (e.g. "Noto Sans"). Resolved by the renderer's text
    /// stack: runtime-registered fonts → system fonts (native only) →
    /// bundled Liberation Sans. Empty string = bundled font.
    pub font: String,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RichSegment {
    pub text: char,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub greek: bool,
    /// Per-segment ink color. `None` inherits `RichText.color`.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub color: Option<Color>,
    /// Per-segment base font size. `None` inherits `RichText.font_size`;
    /// the sub/superscript ratio applies on top of the effective size.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub font_size: Option<f32>,
    /// Fixed advance in em (× effective font size). The glyph ink is drawn
    /// horizontally centered inside the field; the advance itself ignores
    /// glyph metrics entirely. This is what gives legend marks an exact,
    /// font-size-relative width no glyph combination could guarantee.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub field_em: Option<f32>,
    /// Render as a horizontal rule (drawn bar) spanning the full advance
    /// instead of a glyph — the line part of legend marks. `text` stays as
    /// a fallback character for consumers that ignore the flag.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "std::ops::Not::not"))]
    pub rule: bool,
}

impl RichSegment {
    /// One plain character: all style flags off, no per-segment overrides —
    /// color and font size inherit from the parent `RichText`.
    pub fn plain(text: char) -> Self {
        Self {
            text,
            bold: false,
            italic: false,
            underline: false,
            superscript: false,
            subscript: false,
            greek: false,
            color: None,
            font_size: None,
            field_em: None,
            rule: false,
        }
    }

    /// A drawn horizontal rule filling a fixed `width_em × font_size`
    /// advance — the line stroke of legend marks. `'—'` remains as the
    /// fallback text form.
    pub fn rule(width_em: f32, color: Option<Color>) -> Self {
        Self {
            field_em: Some(width_em),
            rule: true,
            color,
            ..Self::plain('—')
        }
    }

    /// A glyph centered inside a fixed `width_em × font_size` advance.
    pub fn fielded(text: char, width_em: f32, color: Option<Color>) -> Self {
        Self {
            field_em: Some(width_em),
            color,
            ..Self::plain(text)
        }
    }
}

impl RichText {
    pub fn plain(text: &str, color: Color, font_size: f32, font: impl Into<String>) -> Self {
        Self {
            segments: rich_segments_from_text(text),
            color,
            font_size,
            font: font.into(),
        }
    }
}

/// Measured pixel extents of a laid-out `RichText`.
///
/// Mirrors the renderer's skia `TextMetrics` shape so geometry formulas can be
/// shared verbatim between drawing (renderer) and bounds policies (model).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextExtents {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

impl TextExtents {
    pub fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// Text measurement contract.
///
/// The model crate computes element bounds (selection boxes, hit areas) from
/// text extents but cannot measure glyphs itself — the renderer implements
/// this with its skia text stack and injects it; tests use fixed stubs.
pub trait MeasureText {
    fn measure_rich(&self, rt: &RichText) -> TextExtents;
}

/// One plain `RichSegment` per char, mapping Unicode super/subscript chars to
/// styled segments. Public because the renderer's `Chart` text builders
/// (`with_title`, …) construct plain titles through this.
pub fn rich_segments_from_text(text: &str) -> Vec<RichSegment> {
    text.chars().map(rich_segment_from_char).collect()
}

fn rich_segment_from_char(c: char) -> RichSegment {
    let (text, superscript, subscript) = match c {
        '⁰' => ('0', true, false),
        '¹' => ('1', true, false),
        '²' => ('2', true, false),
        '³' => ('3', true, false),
        '⁴' => ('4', true, false),
        '⁵' => ('5', true, false),
        '⁶' => ('6', true, false),
        '⁷' => ('7', true, false),
        '⁸' => ('8', true, false),
        '⁹' => ('9', true, false),
        '⁺' => ('+', true, false),
        '⁻' => ('-', true, false),
        '⁽' => ('(', true, false),
        '⁾' => (')', true, false),
        'ⁿ' => ('n', true, false),
        'ᵗ' => ('t', true, false),
        '₀' => ('0', false, true),
        '₁' => ('1', false, true),
        '₂' => ('2', false, true),
        '₃' => ('3', false, true),
        '₄' => ('4', false, true),
        '₅' => ('5', false, true),
        '₆' => ('6', false, true),
        '₇' => ('7', false, true),
        '₈' => ('8', false, true),
        '₉' => ('9', false, true),
        '₊' => ('+', false, true),
        '₋' => ('-', false, true),
        '₍' => ('(', false, true),
        '₎' => (')', false, true),
        other => (other, false, false),
    };
    RichSegment {
        superscript,
        subscript,
        ..RichSegment::plain(text)
    }
}

/// Map Latin a-z/A-Z to the matching Greek letter following the Adobe Symbol
/// encoding. Other input (digits, whitespace, already-Greek, etc.) passes through unchanged.
pub fn greek_char(c: char) -> char {
    match c {
        'a' => 'α', 'b' => 'β', 'c' => 'χ', 'd' => 'δ', 'e' => 'ε',
        'f' => 'φ', 'g' => 'γ', 'h' => 'η', 'i' => 'ι', 'j' => 'ϕ',
        'k' => 'κ', 'l' => 'λ', 'm' => 'μ', 'n' => 'ν', 'o' => 'ο',
        'p' => 'π', 'q' => 'θ', 'r' => 'ρ', 's' => 'σ', 't' => 'τ',
        'u' => 'υ', 'v' => 'ϖ', 'w' => 'ω', 'x' => 'ξ', 'y' => 'ψ',
        'z' => 'ζ',
        'A' => 'Α', 'B' => 'Β', 'C' => 'Χ', 'D' => 'Δ', 'E' => 'Ε',
        'F' => 'Φ', 'G' => 'Γ', 'H' => 'Η', 'I' => 'Ι', 'J' => 'ϑ',
        'K' => 'Κ', 'L' => 'Λ', 'M' => 'Μ', 'N' => 'Ν', 'O' => 'Ο',
        'P' => 'Π', 'Q' => 'Θ', 'R' => 'Ρ', 'S' => 'Σ', 'T' => 'Τ',
        'U' => 'Υ', 'V' => 'ς', 'W' => 'Ω', 'X' => 'Ξ', 'Y' => 'Ψ',
        'Z' => 'Ζ',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercase_full_table() {
        let pairs: &[(char, char)] = &[
            ('a', 'α'), ('b', 'β'), ('c', 'χ'), ('d', 'δ'), ('e', 'ε'),
            ('f', 'φ'), ('g', 'γ'), ('h', 'η'), ('i', 'ι'), ('j', 'ϕ'),
            ('k', 'κ'), ('l', 'λ'), ('m', 'μ'), ('n', 'ν'), ('o', 'ο'),
            ('p', 'π'), ('q', 'θ'), ('r', 'ρ'), ('s', 'σ'), ('t', 'τ'),
            ('u', 'υ'), ('v', 'ϖ'), ('w', 'ω'), ('x', 'ξ'), ('y', 'ψ'),
            ('z', 'ζ'),
        ];
        for &(l, g) in pairs {
            assert_eq!(greek_char(l), g, "lower '{}' -> '{}'", l, g);
        }
    }

    #[test]
    fn uppercase_full_table() {
        let pairs: &[(char, char)] = &[
            ('A', 'Α'), ('B', 'Β'), ('C', 'Χ'), ('D', 'Δ'), ('E', 'Ε'),
            ('F', 'Φ'), ('G', 'Γ'), ('H', 'Η'), ('I', 'Ι'), ('J', 'ϑ'),
            ('K', 'Κ'), ('L', 'Λ'), ('M', 'Μ'), ('N', 'Ν'), ('O', 'Ο'),
            ('P', 'Π'), ('Q', 'Θ'), ('R', 'Ρ'), ('S', 'Σ'), ('T', 'Τ'),
            ('U', 'Υ'), ('V', 'ς'), ('W', 'Ω'), ('X', 'Ξ'), ('Y', 'Ψ'),
            ('Z', 'Ζ'),
        ];
        for &(l, g) in pairs {
            assert_eq!(greek_char(l), g, "upper '{}' -> '{}'", l, g);
        }
    }

    #[test]
    fn non_latin_passthrough() {
        for c in ['0', '9', ' ', '+', '=', '.', '-', '_', 'α', 'Ω', '漢'] {
            assert_eq!(greek_char(c), c);
        }
    }

    // Per-segment overrides are additive schema: documents serialized before
    // they existed must still parse (→ None), and None must not serialize.
    #[cfg(feature = "serde")]
    #[test]
    fn segment_overrides_stay_backward_compatible_and_compact() {
        let old = r#"{"text":"a","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"greek":false}"#;
        let seg: RichSegment = serde_json::from_str(old).expect("pre-override document parses");
        assert_eq!(seg, RichSegment::plain('a'));

        let json = serde_json::to_string(&seg).unwrap();
        assert!(!json.contains("color"), "None color must be skipped: {json}");
        assert!(!json.contains("font_size"), "None font_size must be skipped: {json}");
    }
}
