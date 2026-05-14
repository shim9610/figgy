use crate::color::Color;

/// A rich text value composed of styled segments.
#[derive(Debug, Clone, PartialEq)]
pub struct RichText {
    pub segments: Vec<RichSegment>,
    pub color: Color,
    pub font_size: f32,
    /// Font family name (e.g. "Noto Sans"). Used to resolve a `Typeface` in the skia backend.
    pub font: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RichSegment {
    pub text: char,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub greek: bool,
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

pub(crate) fn rich_segments_from_text(text: &str) -> Vec<RichSegment> {
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
        text,
        bold: false,
        italic: false,
        underline: false,
        superscript,
        subscript,
        greek: false,
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
}
