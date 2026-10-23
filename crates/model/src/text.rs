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
    /// bundled Liberation Sans. Empty string = bundled font. A character the
    /// resolved face cannot draw falls back per character to registered
    /// fonts, the bundled face, then (native only) installed system fonts.
    pub font: String,
    /// Where each line of a multi-line text sits inside the block. Additive
    /// schema: documents written before it existed parse as `Left` (the only
    /// layout they could have had), and `Left` is not serialized, so their
    /// JSON round-trips unchanged.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "TextAlign::is_left")
    )]
    pub align: TextAlign,
}

/// Horizontal alignment of the lines of a [`RichText`] inside its block.
///
/// The block is as wide as its widest line, and placement code positions that
/// block; alignment only decides where each shorter line spends its slack
/// (the block width minus its own width): none of it before the line, half,
/// or all. The block's measured envelope is therefore the same for every
/// alignment, so selection and hit boxes need no change, and single-line text
/// (no slack) is unaffected. Tab-column cells move with their line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TextAlign {
    /// Every line starts at the block's left edge — the layout all text had
    /// before alignment existed.
    #[default]
    Left,
    /// Each line is centred on the block.
    Center,
    /// Every line ends at the block's right edge.
    Right,
}

impl TextAlign {
    /// `true` for the default, `Left`. Used to keep it out of serialized JSON.
    #[cfg(feature = "serde")]
    fn is_left(&self) -> bool {
        *self == TextAlign::Left
    }

    /// Horizontal offset of a line `line_width` wide inside a block
    /// `block_width` wide. `Left` is exactly `0.0` whatever the widths, so
    /// left-aligned layout never depends on the slack arithmetic.
    pub fn line_offset(self, block_width: f32, line_width: f32) -> f32 {
        match self {
            TextAlign::Left => 0.0,
            TextAlign::Center => (block_width - line_width) * 0.5,
            TextAlign::Right => block_width - line_width,
        }
    }
}

/// The character set a [`RichSegment`]'s `text` is drawn from.
///
/// One value per segment, so meaningless combinations (Greek and math at
/// once) cannot be written. Serialized as the `greek` flag the documents
/// have always carried, plus a `math` flag written only for [`CharSet::Math`]
/// (see [`RichSegment`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CharSet {
    /// The character as given, from the text font: the layout of every
    /// segment written with `"greek": false`.
    #[default]
    Text,
    /// Latin `a`–`z` / `A`–`Z` drawn as the Greek letter of the Adobe Symbol
    /// mapping ([`greek_char`]); other characters pass through. The layout of
    /// a segment written with `"greek": true`.
    Greek,
    /// A math character, given as itself (`∑`, `≤`, `∂`, `∇`, …). For now it
    /// is drawn exactly like [`CharSet::Text`]: the character as given, from
    /// the text font. The math font and the spacing by Unicode math class
    /// are later decisions of #45.
    Math,
}

impl CharSet {
    /// The character drawn for `c` in this set: [`greek_char`] for
    /// [`CharSet::Greek`], `c` itself for [`CharSet::Text`] and
    /// [`CharSet::Math`].
    pub fn glyph_char(self, c: char) -> char {
        match self {
            CharSet::Greek => greek_char(c),
            CharSet::Text | CharSet::Math => c,
        }
    }
}

/// One styled character of a [`RichText`].
///
/// Serialized form: the fields in declaration order, with `char_set` written
/// as `"greek"` (`true` only for [`CharSet::Greek`]) followed by
/// `"math": true` only for [`CharSet::Math`], so documents written before
/// `CharSet` existed parse and re-serialize byte for byte. A document that
/// sets both `"greek": true` and `"math": true` is rejected.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize),
    serde(try_from = "segment_serde::RichSegmentIn")
)]
pub struct RichSegment {
    pub text: char,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub superscript: bool,
    pub subscript: bool,
    /// The character set `text` is drawn from. Replaces the former
    /// `greek: bool` (`true` is [`CharSet::Greek`], `false` is
    /// [`CharSet::Text`]).
    pub char_set: CharSet,
    /// Per-segment ink color. `None` inherits `RichText.color`.
    pub color: Option<Color>,
    /// Per-segment base font size. `None` inherits `RichText.font_size`;
    /// the sub/superscript ratio applies on top of the effective size.
    pub font_size: Option<f32>,
    /// Fixed advance in em (× effective font size). The glyph ink is drawn
    /// horizontally centered inside the field; the advance itself ignores
    /// glyph metrics entirely. This is what gives legend marks an exact,
    /// font-size-relative width no glyph combination could guarantee.
    pub field_em: Option<f32>,
    /// Render as a horizontal rule (drawn bar) spanning the full advance
    /// instead of a glyph — the line part of legend marks. `text` stays as
    /// a fallback character for consumers that ignore the flag.
    pub rule: bool,
    /// Optional dash pattern for a rule segment, in em units. Values are
    /// multiplied by the effective segment font size at draw time.
    pub rule_dash: Option<Vec<f32>>,
    /// Optional band painted under a rule segment, from the rule down to the
    /// baseline: the legend mark of a filled line series. `None` draws the
    /// rule alone. Ignored on glyph segments.
    pub rule_band: Option<Color>,
}

/// The wire form of [`RichSegment`]. `char_set` is two keys on the wire, so
/// the segment serializes through a borrowed view and deserializes through
/// an owned mirror that is moved, field by field, into the segment. A field
/// added to `RichSegment` is added to both, with its `skip_serializing_if`.
#[cfg(feature = "serde")]
mod segment_serde {
    use super::{CharSet, RichSegment};
    use crate::color::Color;

    #[derive(serde::Serialize)]
    #[serde(rename = "RichSegment")]
    pub(super) struct RichSegmentOut<'a> {
        text: char,
        bold: bool,
        italic: bool,
        underline: bool,
        superscript: bool,
        subscript: bool,
        greek: bool,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        math: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<&'a Color>,
        #[serde(skip_serializing_if = "Option::is_none")]
        font_size: Option<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        field_em: Option<f32>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        rule: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        rule_dash: Option<&'a [f32]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rule_band: Option<&'a Color>,
    }

    impl<'a> From<&'a RichSegment> for RichSegmentOut<'a> {
        fn from(seg: &'a RichSegment) -> Self {
            Self {
                text: seg.text,
                bold: seg.bold,
                italic: seg.italic,
                underline: seg.underline,
                superscript: seg.superscript,
                subscript: seg.subscript,
                greek: seg.char_set == CharSet::Greek,
                math: seg.char_set == CharSet::Math,
                color: seg.color.as_ref(),
                font_size: seg.font_size,
                field_em: seg.field_em,
                rule: seg.rule,
                rule_dash: seg.rule_dash.as_deref(),
                rule_band: seg.rule_band.as_ref(),
            }
        }
    }

    #[derive(serde::Deserialize)]
    #[serde(rename = "RichSegment")]
    pub(super) struct RichSegmentIn {
        text: char,
        bold: bool,
        italic: bool,
        underline: bool,
        superscript: bool,
        subscript: bool,
        greek: bool,
        /// Additive: absent in every document written before `CharSet`.
        #[serde(default)]
        math: bool,
        #[serde(default)]
        color: Option<Color>,
        #[serde(default)]
        font_size: Option<f32>,
        #[serde(default)]
        field_em: Option<f32>,
        #[serde(default)]
        rule: bool,
        #[serde(default)]
        rule_dash: Option<Vec<f32>>,
        #[serde(default)]
        rule_band: Option<Color>,
    }

    /// The error for a segment that names two character sets.
    pub(super) const GREEK_AND_MATH: &str =
        "a RichSegment cannot set both \"greek\": true and \"math\": true";

    impl TryFrom<RichSegmentIn> for RichSegment {
        type Error = &'static str;

        fn try_from(wire: RichSegmentIn) -> Result<Self, Self::Error> {
            let char_set = match (wire.greek, wire.math) {
                (false, false) => CharSet::Text,
                (true, false) => CharSet::Greek,
                (false, true) => CharSet::Math,
                (true, true) => return Err(GREEK_AND_MATH),
            };
            Ok(RichSegment {
                text: wire.text,
                bold: wire.bold,
                italic: wire.italic,
                underline: wire.underline,
                superscript: wire.superscript,
                subscript: wire.subscript,
                char_set,
                color: wire.color,
                font_size: wire.font_size,
                field_em: wire.field_em,
                rule: wire.rule,
                rule_dash: wire.rule_dash,
                rule_band: wire.rule_band,
            })
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for RichSegment {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&segment_serde::RichSegmentOut::from(self), serializer)
    }
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
            char_set: CharSet::Text,
            color: None,
            font_size: None,
            field_em: None,
            rule: false,
            rule_dash: None,
            rule_band: None,
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

    /// A drawn horizontal rule with an optional dash pattern. Dash entries are
    /// expressed in em units so the pattern follows the legend font size.
    pub fn dashed_rule(width_em: f32, color: Option<Color>, dash_em: Vec<f32>) -> Self {
        Self {
            rule_dash: (!dash_em.is_empty()).then_some(dash_em),
            ..Self::rule(width_em, color)
        }
    }

    /// The same rule with a band under it in `band`: the mark of a line series
    /// that fills the area under or beside its line.
    pub fn with_rule_band(self, band: Option<Color>) -> Self {
        Self {
            rule_band: band,
            ..self
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
            align: TextAlign::Left,
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

    /// Place the parts of decoration `element` a renderer draws under `cfg`
    /// into `sink`, in `cfg`'s chart frame, and return `true`. Tick labels
    /// ([`Element::AxisLabel`], [`Element::ColorBarLabel`]) are one
    /// [`PlacedText`] per drawn label; axes ([`Element::Axis`],
    /// [`Element::ColorBarAxis`]) are their line and ticks as strokes.
    ///
    /// Tick generation, formatting and pruning belong to the renderer, so the
    /// model cannot know which labels and ticks are drawn. A measure that
    /// lays them out lets selection, hit testing and drag containment follow
    /// the drawn parts: the renderer's measure runs the same `emit` function
    /// its raster draws with. The default lays out nothing and returns
    /// `false`; tick labels and axes then have no bounds, since nothing tells
    /// the model where they are drawn. The elements the model places itself
    /// (titles, the legend, the colourbar strip and title, the data area)
    /// never reach this method: [`crate::select::emit_element`] places them.
    ///
    /// [`Element::AxisLabel`]: crate::layout::Element::AxisLabel
    /// [`Element::ColorBarLabel`]: crate::layout::Element::ColorBarLabel
    /// [`Element::Axis`]: crate::layout::Element::Axis
    /// [`Element::ColorBarAxis`]: crate::layout::Element::ColorBarAxis
    /// [`PlacedText`]: crate::sink::PlacedText
    fn emit(
        &self,
        _cfg: &crate::config::Config,
        _element: &crate::layout::Element,
        _sink: &mut dyn crate::sink::Sink,
    ) -> bool {
        false
    }
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
        'a' => 'α',
        'b' => 'β',
        'c' => 'χ',
        'd' => 'δ',
        'e' => 'ε',
        'f' => 'φ',
        'g' => 'γ',
        'h' => 'η',
        'i' => 'ι',
        'j' => 'ϕ',
        'k' => 'κ',
        'l' => 'λ',
        'm' => 'μ',
        'n' => 'ν',
        'o' => 'ο',
        'p' => 'π',
        'q' => 'θ',
        'r' => 'ρ',
        's' => 'σ',
        't' => 'τ',
        'u' => 'υ',
        'v' => 'ϖ',
        'w' => 'ω',
        'x' => 'ξ',
        'y' => 'ψ',
        'z' => 'ζ',
        'A' => 'Α',
        'B' => 'Β',
        'C' => 'Χ',
        'D' => 'Δ',
        'E' => 'Ε',
        'F' => 'Φ',
        'G' => 'Γ',
        'H' => 'Η',
        'I' => 'Ι',
        'J' => 'ϑ',
        'K' => 'Κ',
        'L' => 'Λ',
        'M' => 'Μ',
        'N' => 'Ν',
        'O' => 'Ο',
        'P' => 'Π',
        'Q' => 'Θ',
        'R' => 'Ρ',
        'S' => 'Σ',
        'T' => 'Τ',
        'U' => 'Υ',
        'V' => 'ς',
        'W' => 'Ω',
        'X' => 'Ξ',
        'Y' => 'Ψ',
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
            ('a', 'α'),
            ('b', 'β'),
            ('c', 'χ'),
            ('d', 'δ'),
            ('e', 'ε'),
            ('f', 'φ'),
            ('g', 'γ'),
            ('h', 'η'),
            ('i', 'ι'),
            ('j', 'ϕ'),
            ('k', 'κ'),
            ('l', 'λ'),
            ('m', 'μ'),
            ('n', 'ν'),
            ('o', 'ο'),
            ('p', 'π'),
            ('q', 'θ'),
            ('r', 'ρ'),
            ('s', 'σ'),
            ('t', 'τ'),
            ('u', 'υ'),
            ('v', 'ϖ'),
            ('w', 'ω'),
            ('x', 'ξ'),
            ('y', 'ψ'),
            ('z', 'ζ'),
        ];
        for &(l, g) in pairs {
            assert_eq!(greek_char(l), g, "lower '{}' -> '{}'", l, g);
        }
    }

    #[test]
    fn uppercase_full_table() {
        let pairs: &[(char, char)] = &[
            ('A', 'Α'),
            ('B', 'Β'),
            ('C', 'Χ'),
            ('D', 'Δ'),
            ('E', 'Ε'),
            ('F', 'Φ'),
            ('G', 'Γ'),
            ('H', 'Η'),
            ('I', 'Ι'),
            ('J', 'ϑ'),
            ('K', 'Κ'),
            ('L', 'Λ'),
            ('M', 'Μ'),
            ('N', 'Ν'),
            ('O', 'Ο'),
            ('P', 'Π'),
            ('Q', 'Θ'),
            ('R', 'Ρ'),
            ('S', 'Σ'),
            ('T', 'Τ'),
            ('U', 'Υ'),
            ('V', 'ς'),
            ('W', 'Ω'),
            ('X', 'Ξ'),
            ('Y', 'Ψ'),
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
        assert!(
            !json.contains("color"),
            "None color must be skipped: {json}"
        );
        assert!(
            !json.contains("font_size"),
            "None font_size must be skipped: {json}"
        );
    }

    #[test]
    fn char_set_draws_greek_only_for_greek() {
        assert_eq!(CharSet::default(), CharSet::Text);
        assert_eq!(CharSet::Text.glyph_char('a'), 'a');
        assert_eq!(CharSet::Greek.glyph_char('a'), 'α');
        assert_eq!(CharSet::Greek.glyph_char('1'), '1');
        for c in ['a', '∑', '≤', '∂', '∇'] {
            assert_eq!(CharSet::Math.glyph_char(c), c, "math draws {c} as given");
        }
        assert_eq!(RichSegment::plain('a').char_set, CharSet::Text);
    }

    // `CharSet` keeps the `greek` flag documents have always carried and adds
    // `math` only for `Math`: `Text` and `Greek` serialize exactly as the
    // former `greek: false` / `greek: true`, and a segment naming both sets
    // is an explicit error.
    #[cfg(feature = "serde")]
    #[test]
    fn char_set_serializes_as_the_greek_flag_plus_math_only_for_math() {
        const HEAD: &str = r#"{"text":"a","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"#;
        let cases = [
            (CharSet::Text, r#""greek":false}"#),
            (CharSet::Greek, r#""greek":true}"#),
            (CharSet::Math, r#""greek":false,"math":true}"#),
        ];
        for (char_set, tail) in cases {
            let seg = RichSegment {
                char_set,
                ..RichSegment::plain('a')
            };
            let json = serde_json::to_string(&seg).unwrap();
            assert_eq!(json, format!("{HEAD}{tail}"), "{char_set:?}");
            let back: RichSegment = serde_json::from_str(&json).unwrap();
            assert_eq!(back, seg, "{char_set:?} round-trips");
        }

        // `math` sits right after `greek`, before the optional overrides.
        let seg = RichSegment {
            char_set: CharSet::Math,
            color: Some(Color::BLACK),
            ..RichSegment::plain('∑')
        };
        let json = serde_json::to_string(&seg).unwrap();
        assert!(
            json.contains(r#""greek":false,"math":true,"color":"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<RichSegment>(&json).unwrap(), seg);

        // An explicit `"math": false` reads as `Text` and is not written back.
        let explicit = format!(r#"{HEAD}"greek":false,"math":false}}"#);
        let seg: RichSegment = serde_json::from_str(&explicit).unwrap();
        assert_eq!(seg.char_set, CharSet::Text);
        assert_eq!(
            serde_json::to_string(&seg).unwrap(),
            format!("{HEAD}\"greek\":false}}")
        );

        let both = format!(r#"{HEAD}"greek":true,"math":true}}"#);
        let err = serde_json::from_str::<RichSegment>(&both)
            .expect_err("greek and math at once is rejected")
            .to_string();
        assert!(err.contains("both"), "{err}");

        // `greek` stays required, as it always was.
        let missing = r#"{"text":"a","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false}"#;
        assert!(serde_json::from_str::<RichSegment>(missing).is_err());
    }

    // A document holding every optional segment field re-serializes byte for
    // byte through the wire structs.
    #[cfg(feature = "serde")]
    #[test]
    fn segment_with_every_field_round_trips_byte_for_byte() {
        let seg = RichSegment {
            bold: true,
            italic: true,
            underline: true,
            superscript: true,
            subscript: true,
            char_set: CharSet::Greek,
            color: Some(Color::new(0.25, 0.5, 0.75, 1.0)),
            font_size: Some(13.5),
            field_em: Some(2.0),
            rule: true,
            rule_dash: Some(vec![0.5, 0.25]),
            rule_band: Some(Color::BLACK),
            ..RichSegment::plain('q')
        };
        let json = serde_json::to_string(&seg).unwrap();
        let back: RichSegment = serde_json::from_str(&json).unwrap();
        assert_eq!(back, seg);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // `align` is additive schema too: a document written before it existed
    // parses as `Left`, `Left` does not serialize (so existing JSON stays
    // byte-identical), and the other alignments round-trip by name.
    #[cfg(feature = "serde")]
    #[test]
    fn align_defaults_to_left_and_round_trips() {
        let old = r#"{"segments":[],"color":{"r":0.0,"g":0.0,"b":0.0,"a":1.0},"font_size":12.0,"font":""}"#;
        let rt: RichText = serde_json::from_str(old).expect("pre-align document parses");
        assert_eq!(rt.align, TextAlign::Left);
        assert_eq!(rt, RichText::plain("", Color::BLACK, 12.0, ""));
        assert_eq!(
            serde_json::to_string(&rt).unwrap(),
            old,
            "a left-aligned document must serialize exactly as before"
        );

        for align in [TextAlign::Center, TextAlign::Right] {
            let rt = RichText {
                align,
                ..RichText::plain("a\nbc", Color::BLACK, 12.0, "")
            };
            let json = serde_json::to_string(&rt).unwrap();
            assert!(json.contains(&format!(r#""align":"{align:?}""#)), "{json}");
            let back: RichText = serde_json::from_str(&json).unwrap();
            assert_eq!(back, rt);
        }
    }

    #[test]
    fn align_spends_none_half_or_all_of_the_slack() {
        assert_eq!(TextAlign::default(), TextAlign::Left);
        assert_eq!(TextAlign::Left.line_offset(100.0, 40.0), 0.0);
        assert_eq!(TextAlign::Center.line_offset(100.0, 40.0), 30.0);
        assert_eq!(TextAlign::Right.line_offset(100.0, 40.0), 60.0);
        // The widest line has no slack and never moves.
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            assert_eq!(align.line_offset(100.0, 100.0), 0.0);
        }
        // Left never depends on the widths, even unusable ones.
        assert_eq!(TextAlign::Left.line_offset(f32::NAN, 1.0), 0.0);
    }
}
