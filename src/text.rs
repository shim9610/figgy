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
