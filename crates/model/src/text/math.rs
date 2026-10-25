//! Math layout nodes inside a [`RichText`]: the node and slot types, the
//! fixed-stack walker that validates a segment tree ([`MathCursor`]), and
//! the two Unicode tables the math layout reads ([`math_class`],
//! [`math_alphanumeric`]).
//!
//! A formula stays a flat list of segments. A segment that governs a layout
//! carries a [`MathNode`] (its id and what it lays out); every segment inside
//! a node names its direct parent and the slot it fills with a [`MathSlot`].
//! A node comes first, and its descendants follow it immediately and
//! contiguously, slot by slot in slot order, so the tree is walked with a
//! stack of open nodes whose size is the depth limit ([`MAX_MATH_DEPTH`]);
//! nothing is allocated and nothing is looked up by id.
//!
//! Ids strictly increase within a cell (the text between line breaks and
//! tabs), with gaps allowed, and are unique only within the cell, so two
//! legend labels that each start at id 0 stay valid when they are joined and
//! nothing is renumbered.

use super::{RichSegment, RichText};

/// The deepest nesting of math nodes a cell may hold: the size of every
/// walker's fixed stack.
pub const MAX_MATH_DEPTH: usize = 16;

/// A segment that governs a layout: its group id and what it lays out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MathNode {
    /// The id its slot children name. Strictly increasing within a cell.
    pub id: u32,
    pub kind: MathKind,
}

/// The node and slot a segment belongs to: its direct parent's id and the
/// slot of that parent it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MathSlot {
    pub id: u32,
    pub slot: u8,
}

/// What a [`MathNode`] lays out, and the slots its children fill.
///
/// A node's own `text` is its symbol where it has one (`∑`, `√`, an accent
/// mark, a brace, an arrow, a delimiter) and U+200B otherwise; a consumer
/// that ignores math nodes draws it flat with the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MathKind {
    /// Math mode, like `$…$`: slot 0 is the body. A root only: it has no
    /// parent and contains no other `Math`.
    Math,
    /// A fraction: slot 0 the numerator, slot 1 the denominator.
    Frac(FracStyle),
    /// A binomial coefficient: slots as [`MathKind::Frac`], no bar, in
    /// parentheses.
    Binom(FracStyle),
    /// A radical (the node's text is `√`): slot 0 the radicand, slot 1 the
    /// index.
    Sqrt,
    /// Scripts: slot 0 the base, slot 1 the superscript, slot 2 the
    /// subscript. Also a text-mode root (no `Math` ancestor), where it lays
    /// out exactly as the `superscript` / `subscript` flags do.
    Scripts,
    /// A big operator (the node's text is the symbol: `∑`, `∫`, `∏`, …):
    /// slot 0 the lower limit, slot 1 the upper limit.
    BigOp(Limits),
    /// An operator name (`lim`, `sin`, `\operatorname{…}`): slot 0 the
    /// name, slot 1 the lower limit, slot 2 the upper limit.
    OpName(Limits),
    /// Stretchy delimiters around slot 0, the body; `.` is no delimiter.
    Delim { open: char, close: char },
    /// A stretchy delimiter (the node's text) in the body of a
    /// [`MathKind::Delim`], sized with it. No slots.
    Middle,
    /// A delimiter (the node's text) at a fixed size: `level` 1 to 4 is
    /// `\big`, `\Big`, `\bigg`, `\Bigg`; `class` is how it spaces
    /// ([`MathClass::Normal`], [`MathClass::Opening`],
    /// [`MathClass::Closing`] or [`MathClass::Relation`]). No slots.
    Sized { level: u8, class: MathClass },
    /// An accent (the node's text is the mark) over slot 0, the body.
    Accent(AccentKind),
    /// A line, brace, arrow or set label over slot 0, the body; slot 1 is
    /// the label.
    Over(OverUnder),
    /// A line, brace, arrow or set label under slot 0, the body; slot 1 is
    /// the label.
    Under(OverUnder),
}

/// The style of a fraction: chosen from the surrounding style, or forced
/// to the display (`\dfrac`) or text (`\tfrac`) form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FracStyle {
    Auto,
    Display,
    Text,
}

/// Where a big operator's limits go: chosen from the style and the
/// operator (`Auto`), above and below (`\limits`) or beside it as scripts
/// (`\nolimits`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Limits {
    Auto,
    Limits,
    NoLimits,
}

/// An accent at its own size (`\hat`) or stretched over the body
/// (`\widehat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AccentKind {
    Fixed,
    Wide,
}

/// What an [`MathKind::Over`] or [`MathKind::Under`] node draws: a rule, the
/// node's brace or arrow stretched over the body, or only the label (set
/// over or under the body, as `\overset` and `\underset` do).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OverUnder {
    Line,
    Brace,
    Arrow,
    Set,
}

/// The Unicode math class of a character (Unicode Technical Report #25,
/// `MathClass.txt`). The math layout spaces atoms by the TeX class each of
/// these maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MathClass {
    /// `N`: digits and symbols with one form.
    Normal,
    /// `A`: letters.
    Alphabetic,
    /// `B`: binary operators.
    Binary,
    /// `C`: closing delimiters.
    Closing,
    /// `D`: diacritics.
    Diacritic,
    /// `F`: unpaired delimiters (`|`, `‖`).
    Fence,
    /// `G`: pieces of large operators.
    GlyphPart,
    /// `L`: n-ary and large operators.
    Large,
    /// `O`: opening delimiters.
    Opening,
    /// `P`: punctuation.
    Punctuation,
    /// `R`: relations, arrows included.
    Relation,
    /// `S`: spaces.
    Space,
    /// `U`: unary operators.
    Unary,
    /// `V`: operators that are unary or binary by context (`+`, `−`).
    Vary,
    /// `X`: anything else, and characters the table does not list.
    Special,
}

impl MathKind {
    /// How many slots the kind has; a child names a slot below it.
    pub fn slot_count(self) -> u8 {
        match self {
            MathKind::Middle | MathKind::Sized { .. } => 0,
            MathKind::Math | MathKind::Delim { .. } | MathKind::Accent(_) => 1,
            MathKind::Frac(_)
            | MathKind::Binom(_)
            | MathKind::Sqrt
            | MathKind::BigOp(_)
            | MathKind::Over(_)
            | MathKind::Under(_) => 2,
            MathKind::Scripts | MathKind::OpName(_) => 3,
        }
    }
}

/// Whether `c` is a delimiter a [`MathKind::Delim`], [`MathKind::Middle`]
/// or [`MathKind::Sized`] node may stretch: parentheses, brackets, braces,
/// bars, angle brackets, floors, ceilings, slashes, white brackets, and `.`
/// for no delimiter.
pub fn is_delimiter(c: char) -> bool {
    matches!(
        c,
        '.' | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '|'
            | '‖'
            | '⟨'
            | '⟩'
            | '⌊'
            | '⌋'
            | '⌈'
            | '⌉'
            | '/'
            | '\\'
            | '⟦'
            | '⟧'
            | '↑'
            | '↓'
            | '⇑'
            | '⇓'
    )
}

/// Why a segment tree is malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathErrorKind {
    /// A node id equal to an earlier node's in the cell.
    DuplicateId,
    /// A node id below an earlier node's in the cell.
    IdOutOfOrder,
    /// A parent id that is not an open node: unknown, already closed, or
    /// not the innermost node the segment follows.
    UnknownParent,
    /// A slot the parent's kind does not have.
    SlotOutOfRange,
    /// A slot below the one the parent's previous child filled.
    SlotOutOfOrder,
    /// Nodes nested deeper than [`MAX_MATH_DEPTH`].
    TooDeep,
    /// A node other than `Scripts` without a `Math` ancestor.
    MissingMathAncestor,
    /// A `Math` node with a parent.
    NestedMath,
    /// A `Middle` node whose parent is not a `Delim`.
    MiddleOutsideDelim,
    /// A line break or tab inside a node, or carrying a node.
    BreakInMath,
    /// A `superscript` or `subscript` flag on a segment inside a node: the
    /// node's slots place scripts, so a flag would be a second source.
    StyleFlagInMath,
    /// A `Delim`, `Middle` or `Sized` character that is not a delimiter, or
    /// a `Sized` level or class out of range.
    BadDelimiter,
}

impl MathErrorKind {
    /// The reason, for `FiggyError::InvalidConfig` and parse errors.
    pub fn as_str(self) -> &'static str {
        match self {
            MathErrorKind::DuplicateId => "a math node id repeats an earlier id in its cell",
            MathErrorKind::IdOutOfOrder => {
                "math node ids must increase within a cell, in order of appearance"
            }
            MathErrorKind::UnknownParent => {
                "a math segment's parent is not an open node it follows"
            }
            MathErrorKind::SlotOutOfRange => "a math segment names a slot its parent does not have",
            MathErrorKind::SlotOutOfOrder => "a math node's children must fill its slots in order",
            MathErrorKind::TooDeep => "math nodes are nested deeper than 16",
            MathErrorKind::MissingMathAncestor => {
                "a math node other than Scripts must be inside a Math node"
            }
            MathErrorKind::NestedMath => "a Math node must be a root",
            MathErrorKind::MiddleOutsideDelim => "a Middle node must be a child of a Delim node",
            MathErrorKind::BreakInMath => "a line break or tab cannot be inside a math node",
            MathErrorKind::StyleFlagInMath => {
                "a segment inside a math node cannot set superscript or subscript"
            }
            MathErrorKind::BadDelimiter => {
                "a math delimiter must be a delimiter character, at a Sized level 1..4 and class"
            }
        }
    }
}

/// A malformed segment tree: what is wrong, at which segment of the
/// [`RichText`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathError {
    /// Index of the offending segment in `RichText.segments`.
    pub at: usize,
    pub kind: MathErrorKind,
}

impl MathError {
    /// The reason, without the position.
    pub fn as_str(&self) -> &'static str {
        self.kind.as_str()
    }
}

impl std::fmt::Display for MathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "segment {}: {}", self.at, self.kind.as_str())
    }
}

impl std::error::Error for MathError {}

/// One open node of a [`MathCursor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathFrame {
    pub id: u32,
    pub kind: MathKind,
    /// The slot its latest child filled.
    pub slot: u8,
    /// Whether the node is a `Math` node or inside one.
    pub in_math: bool,
}

const EMPTY_FRAME: MathFrame = MathFrame {
    id: 0,
    kind: MathKind::Math,
    slot: 0,
    in_math: false,
};

/// Where [`MathCursor::step`] placed a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathStep {
    /// How many nodes enclose the segment (0 for plain text).
    pub depth: usize,
    /// Whether the segment is inside a `Math` node (for a node: whether it
    /// is a `Math` node or inside one).
    pub in_math: bool,
}

/// The walker of a segment tree: a fixed stack of open nodes, no
/// allocation. It validates the ordering rule segment by segment — pop to
/// the named parent, check the slot's range and order, the `Math`-ancestor
/// rule, the id and the depth — and tells each segment where it sits.
/// Line breaks and tabs close every node.
#[derive(Debug, Clone)]
pub struct MathCursor {
    frames: [MathFrame; MAX_MATH_DEPTH],
    depth: usize,
    last_id: Option<u32>,
}

impl Default for MathCursor {
    fn default() -> Self {
        Self::new()
    }
}

impl MathCursor {
    pub const fn new() -> Self {
        Self {
            frames: [EMPTY_FRAME; MAX_MATH_DEPTH],
            depth: 0,
            last_id: None,
        }
    }

    /// The open nodes, outermost first.
    pub fn open(&self) -> &[MathFrame] {
        &self.frames[..self.depth]
    }

    /// Place segment `at`, `seg`, the next segment of the text.
    pub fn step(&mut self, at: usize, seg: &RichSegment) -> Result<MathStep, MathError> {
        let err = |kind| Err(MathError { at, kind });
        if matches!(seg.text, '\n' | '\t') {
            if seg.math_node.is_some() || seg.math_parent.is_some() {
                return err(MathErrorKind::BreakInMath);
            }
            // A break ends the cell: every node closes, ids start over.
            self.depth = 0;
            self.last_id = None;
            return Ok(MathStep {
                depth: 0,
                in_math: false,
            });
        }
        // Pop to the parent: the nodes above it are complete.
        match seg.math_parent {
            None => self.depth = 0,
            Some(parent) => {
                let Some(index) = self.open().iter().rposition(|f| f.id == parent.id) else {
                    return err(MathErrorKind::UnknownParent);
                };
                self.depth = index + 1;
                let frame = &mut self.frames[index];
                if parent.slot >= frame.kind.slot_count() {
                    return err(MathErrorKind::SlotOutOfRange);
                }
                if parent.slot < frame.slot {
                    return err(MathErrorKind::SlotOutOfOrder);
                }
                frame.slot = parent.slot;
                if seg.superscript || seg.subscript {
                    return err(MathErrorKind::StyleFlagInMath);
                }
            }
        }
        let parent = self.open().last().copied();
        let in_math = parent.is_some_and(|f| f.in_math);
        let Some(node) = seg.math_node else {
            return Ok(MathStep {
                depth: self.depth,
                in_math,
            });
        };
        if let Some(last) = self.last_id
            && node.id <= last
        {
            let repeated = node.id == last || self.open().iter().any(|f| f.id == node.id);
            return err(if repeated {
                MathErrorKind::DuplicateId
            } else {
                MathErrorKind::IdOutOfOrder
            });
        }
        match node.kind {
            MathKind::Math if parent.is_some() => return err(MathErrorKind::NestedMath),
            MathKind::Math | MathKind::Scripts => {}
            _ if !in_math => return err(MathErrorKind::MissingMathAncestor),
            _ => {}
        }
        let delimiter_ok = match node.kind {
            MathKind::Delim { open, close } => is_delimiter(open) && is_delimiter(close),
            MathKind::Middle => {
                if !parent.is_some_and(|f| matches!(f.kind, MathKind::Delim { .. })) {
                    return err(MathErrorKind::MiddleOutsideDelim);
                }
                is_delimiter(seg.text)
            }
            MathKind::Sized { level, class } => {
                (1..=4).contains(&level)
                    && matches!(
                        class,
                        MathClass::Normal
                            | MathClass::Opening
                            | MathClass::Closing
                            | MathClass::Relation
                    )
                    && is_delimiter(seg.text)
            }
            _ => true,
        };
        if !delimiter_ok {
            return err(MathErrorKind::BadDelimiter);
        }
        if self.depth == MAX_MATH_DEPTH {
            return err(MathErrorKind::TooDeep);
        }
        self.frames[self.depth] = MathFrame {
            id: node.id,
            kind: node.kind,
            slot: 0,
            in_math: in_math || node.kind == MathKind::Math,
        };
        self.depth += 1;
        self.last_id = Some(node.id);
        Ok(MathStep {
            depth: self.depth,
            in_math: in_math || node.kind == MathKind::Math,
        })
    }
}

impl RichText {
    /// Check the math segment tree: the ordering rule, ids, slots, depth and
    /// the kind rules of [`MathErrorKind`]. A text without math nodes is
    /// always valid.
    pub fn validate_math(&self) -> Result<(), MathError> {
        validate_math_segments(&self.segments)
    }
}

/// [`RichText::validate_math`] over a segment list.
pub fn validate_math_segments(segments: &[RichSegment]) -> Result<(), MathError> {
    if segments
        .iter()
        .all(|s| s.math_node.is_none() && s.math_parent.is_none())
    {
        return Ok(());
    }
    let mut cursor = MathCursor::new();
    for (at, seg) in segments.iter().enumerate() {
        cursor.step(at, seg)?;
    }
    Ok(())
}

/// The end (exclusive) of the subtree that starts at segment `start` of a
/// valid tree: the node and every descendant, or `start + 1` for a leaf.
/// Walks a fixed stack of the subtree's open ids.
pub fn subtree_end(segments: &[RichSegment], start: usize) -> usize {
    let Some(root) = segments.get(start).and_then(|s| s.math_node) else {
        return start + 1;
    };
    let mut open = [0u32; MAX_MATH_DEPTH];
    open[0] = root.id;
    let mut depth = 1;
    for (i, seg) in segments.iter().enumerate().skip(start + 1) {
        let Some(parent) = seg.math_parent else {
            return i;
        };
        let Some(index) = open[..depth].iter().rposition(|&id| id == parent.id) else {
            return i;
        };
        depth = index + 1;
        if let Some(node) = seg.math_node
            && depth < MAX_MATH_DEPTH
        {
            open[depth] = node.id;
            depth += 1;
        }
    }
    segments.len()
}

/// The Unicode math class of `c` (`MathClass.txt`, revision 15);
/// [`MathClass::Special`] for a character the table does not list.
pub fn math_class(c: char) -> MathClass {
    let c = c as u32;
    match MATH_CLASS_RANGES.binary_search_by(|&(lo, hi, _)| {
        if hi < c {
            std::cmp::Ordering::Less
        } else if lo > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(i) => MATH_CLASS_RANGES[i].2,
        Err(_) => MathClass::Special,
    }
}

/// The Mathematical Alphanumeric Symbols code point of `c` in the
/// bold, italic or bold-italic alphabet, including the reserved holes that
/// map to the Letterlike Symbols block (italic `h` is U+210E). Latin
/// letters, Greek letters (with the symbol variants `ϵ ϑ ϰ ϕ ϱ ϖ ∂ ∇`),
/// digits (bold only), and the dotless `ı ȷ` (italic only) map; everything
/// else, and upright non-bold text, is returned as given.
pub fn math_alphanumeric(c: char, bold: bool, italic: bool) -> char {
    let style = match (bold, italic) {
        (false, false) => return c,
        (true, false) => 0,
        (false, true) => 1,
        (true, true) => 2,
    };
    let at = |base: u32, offset: u32| char::from_u32(base + offset).unwrap_or(c);
    match c {
        'A'..='Z' => at([0x1D400, 0x1D434, 0x1D468][style], c as u32 - 'A' as u32),
        'a'..='z' => {
            if style == 1 && c == 'h' {
                return '\u{210E}';
            }
            at([0x1D41A, 0x1D44E, 0x1D482][style], c as u32 - 'a' as u32)
        }
        '0'..='9' if style == 0 => at(0x1D7CE, c as u32 - '0' as u32),
        'ı' if style == 1 => '\u{1D6A4}',
        'ȷ' if style == 1 => '\u{1D6A5}',
        _ => {
            // The Greek block: 58 code points per style, bold from U+1D6A8.
            let Some(offset) = greek_offset(c) else {
                return c;
            };
            at(0x1D6A8 + 58 * style as u32, offset)
        }
    }
}

/// Position of `c` inside one style of the mathematical Greek block.
fn greek_offset(c: char) -> Option<u32> {
    Some(match c {
        // Α..Ρ, then ϴ in the slot of the unassigned U+03A2, then Σ..Ω.
        'Α'..='Ρ' => c as u32 - 'Α' as u32,
        'ϴ' => 17,
        'Σ'..='Ω' => c as u32 - 'Α' as u32,
        '∇' => 25,
        // α..ω, final sigma in its own slot.
        'α'..='ω' => 26 + c as u32 - 'α' as u32,
        '∂' => 51,
        'ϵ' => 52,
        'ϑ' => 53,
        'ϰ' => 54,
        'ϕ' => 55,
        'ϱ' => 56,
        'ϖ' => 57,
        _ => return None,
    })
}

use MathClass::{
    Alphabetic as A, Binary as B, Closing as C, Diacritic as D, Fence as F, GlyphPart as G,
    Large as L, Normal as N, Opening as O, Punctuation as P, Relation as R, Space as S,
    Special as X, Unary as U, Vary as V,
};

/// `MathClass.txt` (revision 15) as sorted, merged, disjoint inclusive
/// ranges.
const MATH_CLASS_RANGES: &[(u32, u32, MathClass)] = &[
    (0x0020, 0x0020, S),
    (0x0021, 0x0021, N),
    (0x0023, 0x0026, N),
    (0x0028, 0x0028, O),
    (0x0029, 0x0029, C),
    (0x002A, 0x002A, N),
    (0x002B, 0x002B, V),
    (0x002C, 0x002C, P),
    (0x002D, 0x002D, N),
    (0x002E, 0x002E, P),
    (0x002F, 0x002F, B),
    (0x0030, 0x0039, N),
    (0x003A, 0x003B, P),
    (0x003C, 0x003E, R),
    (0x003F, 0x003F, P),
    (0x0040, 0x0040, N),
    (0x0041, 0x005A, A),
    (0x005B, 0x005B, O),
    (0x005C, 0x005C, B),
    (0x005D, 0x005D, C),
    (0x005E, 0x005F, N),
    (0x0060, 0x0060, D),
    (0x0061, 0x007A, A),
    (0x007B, 0x007B, O),
    (0x007C, 0x007C, F),
    (0x007D, 0x007D, C),
    (0x007E, 0x007E, N),
    (0x00A0, 0x00A0, S),
    (0x00A1, 0x00A1, P),
    (0x00A2, 0x00A7, N),
    (0x00A8, 0x00A8, D),
    (0x00AC, 0x00AC, U),
    (0x00AF, 0x00AF, D),
    (0x00B0, 0x00B0, N),
    (0x00B1, 0x00B1, V),
    (0x00B2, 0x00B6, N),
    (0x00B7, 0x00B7, B),
    (0x00B9, 0x00B9, N),
    (0x00BC, 0x00BE, N),
    (0x00BF, 0x00BF, P),
    (0x00D7, 0x00D7, B),
    (0x00F7, 0x00F7, B),
    (0x0131, 0x0131, A),
    (0x0237, 0x0237, A),
    (0x02C6, 0x02C7, D),
    (0x02D8, 0x02DA, D),
    (0x02DC, 0x02DC, D),
    (0x0300, 0x0308, D),
    (0x030A, 0x030A, D),
    (0x030C, 0x030C, D),
    (0x0311, 0x0311, D),
    (0x0323, 0x0323, D),
    (0x032C, 0x0333, D),
    (0x0338, 0x0338, D),
    (0x033A, 0x033A, D),
    (0x033F, 0x033F, D),
    (0x0346, 0x0346, D),
    (0x0391, 0x03A1, A),
    (0x03A3, 0x03A4, A),
    (0x03A6, 0x03A9, A),
    (0x03B1, 0x03C1, A),
    (0x03C3, 0x03C9, A),
    (0x03D0, 0x03D2, A),
    (0x03D5, 0x03D6, A),
    (0x03D8, 0x03D9, N),
    (0x03DA, 0x03DD, A),
    (0x03E0, 0x03E1, A),
    (0x03F0, 0x03F1, A),
    (0x03F4, 0x03F5, A),
    (0x03F6, 0x03F6, N),
    (0x0428, 0x0428, A),
    (0x0606, 0x0607, L),
    (0x0608, 0x0608, A),
    (0x2000, 0x2007, S),
    (0x2009, 0x200B, S),
    (0x2010, 0x2010, P),
    (0x2012, 0x2014, P),
    (0x2016, 0x2016, F),
    (0x2020, 0x2021, R),
    (0x2022, 0x2022, B),
    (0x2026, 0x2026, N),
    (0x2032, 0x2037, N),
    (0x203B, 0x203C, N),
    (0x2040, 0x2040, B),
    (0x2044, 0x2044, B),
    (0x204E, 0x204E, B),
    (0x204F, 0x2050, R),
    (0x2051, 0x2052, N),
    (0x2057, 0x2057, N),
    (0x205F, 0x205F, S),
    (0x2061, 0x2062, B),
    (0x2063, 0x2063, P),
    (0x2064, 0x2064, X),
    (0x207A, 0x207E, N),
    (0x208A, 0x208E, N),
    (0x20D0, 0x20D2, D),
    (0x20D3, 0x20D3, X),
    (0x20D4, 0x20DF, D),
    (0x20E1, 0x20E1, D),
    (0x20E4, 0x20EF, D),
    (0x2102, 0x2102, A),
    (0x2107, 0x2107, N),
    (0x210A, 0x210D, A),
    (0x210E, 0x210F, N),
    (0x2110, 0x2113, A),
    (0x2115, 0x2115, A),
    (0x2118, 0x211D, A),
    (0x2124, 0x2124, A),
    (0x2126, 0x2127, N),
    (0x2128, 0x2128, A),
    (0x2129, 0x2129, N),
    (0x212B, 0x212D, A),
    (0x212F, 0x2131, A),
    (0x2132, 0x2132, N),
    (0x2133, 0x2138, A),
    (0x213C, 0x213D, A),
    (0x213E, 0x213E, N),
    (0x213F, 0x213F, A),
    (0x2140, 0x2140, L),
    (0x2141, 0x2149, N),
    (0x214B, 0x214B, N),
    (0x2190, 0x21B3, R),
    (0x21B6, 0x21B7, R),
    (0x21BA, 0x21FF, R),
    (0x2200, 0x2201, U),
    (0x2202, 0x2202, N),
    (0x2203, 0x2204, U),
    (0x2205, 0x2205, N),
    (0x2206, 0x2207, U),
    (0x2208, 0x220D, R),
    (0x220E, 0x220E, N),
    (0x220F, 0x2211, L),
    (0x2212, 0x2213, V),
    (0x2214, 0x2219, B),
    (0x221A, 0x221C, L),
    (0x221D, 0x221D, R),
    (0x221E, 0x2222, N),
    (0x2223, 0x2226, R),
    (0x2227, 0x222A, B),
    (0x222B, 0x2233, L),
    (0x2234, 0x2237, R),
    (0x2238, 0x2238, B),
    (0x2239, 0x223D, R),
    (0x223E, 0x223E, B),
    (0x223F, 0x223F, N),
    (0x2240, 0x2240, B),
    (0x2241, 0x228B, R),
    (0x228C, 0x228E, B),
    (0x228F, 0x2292, R),
    (0x2293, 0x22A1, B),
    (0x22A2, 0x22A3, R),
    (0x22A4, 0x22A4, N),
    (0x22A5, 0x22B8, R),
    (0x22B9, 0x22BD, B),
    (0x22BE, 0x22BF, N),
    (0x22C0, 0x22C3, L),
    (0x22C4, 0x22C7, B),
    (0x22C8, 0x22C8, R),
    (0x22C9, 0x22CC, B),
    (0x22CD, 0x22CD, R),
    (0x22CE, 0x22CF, B),
    (0x22D0, 0x22D1, R),
    (0x22D2, 0x22D3, B),
    (0x22D4, 0x22FF, R),
    (0x2300, 0x2300, N),
    (0x2302, 0x2302, N),
    (0x2305, 0x2306, B),
    (0x2308, 0x2308, O),
    (0x2309, 0x2309, C),
    (0x230A, 0x230A, O),
    (0x230B, 0x230B, C),
    (0x2310, 0x2311, N),
    (0x2319, 0x2319, N),
    (0x231C, 0x231C, O),
    (0x231D, 0x231D, C),
    (0x231E, 0x231E, O),
    (0x231F, 0x231F, C),
    (0x2320, 0x2321, G),
    (0x2322, 0x2323, R),
    (0x2336, 0x2336, N),
    (0x233D, 0x233D, B),
    (0x233F, 0x233F, R),
    (0x237C, 0x237C, R),
    (0x2394, 0x2394, N),
    (0x239B, 0x23AF, G),
    (0x23B0, 0x23B1, R),
    (0x23B2, 0x23B3, G),
    (0x23B4, 0x23B6, N),
    (0x23B7, 0x23B7, G),
    (0x23D0, 0x23D0, G),
    (0x23DC, 0x23E7, N),
    (0x24C8, 0x24C8, N),
    (0x25A0, 0x25A1, N),
    (0x25AA, 0x25AB, N),
    (0x25AD, 0x25B1, N),
    (0x25B2, 0x25B9, B),
    (0x25BC, 0x25C5, B),
    (0x25C6, 0x25C9, N),
    (0x25CA, 0x25CB, B),
    (0x25CE, 0x25D3, N),
    (0x25D6, 0x25D7, N),
    (0x25E2, 0x25E5, N),
    (0x25E6, 0x25E6, B),
    (0x25E7, 0x25EA, N),
    (0x25EB, 0x25EC, B),
    (0x25EF, 0x25EF, N),
    (0x25F8, 0x25FF, B),
    (0x2605, 0x2606, B),
    (0x2609, 0x2609, N),
    (0x260C, 0x260C, N),
    (0x263D, 0x2644, N),
    (0x2646, 0x2649, N),
    (0x2660, 0x2667, N),
    (0x2669, 0x2669, N),
    (0x266D, 0x266F, N),
    (0x2680, 0x2689, N),
    (0x26AA, 0x26AC, N),
    (0x26B2, 0x26B2, N),
    (0x2713, 0x2713, N),
    (0x2717, 0x2717, N),
    (0x2720, 0x2720, N),
    (0x272A, 0x272A, N),
    (0x2736, 0x2736, N),
    (0x2772, 0x2772, O),
    (0x2773, 0x2773, C),
    (0x27C0, 0x27C1, N),
    (0x27C2, 0x27CB, R),
    (0x27CC, 0x27CC, L),
    (0x27CD, 0x27CD, R),
    (0x27CE, 0x27CF, B),
    (0x27D0, 0x27D0, N),
    (0x27D1, 0x27D1, B),
    (0x27D2, 0x27D4, R),
    (0x27D5, 0x27D9, L),
    (0x27DA, 0x27DF, R),
    (0x27E0, 0x27E5, B),
    (0x27E6, 0x27E6, O),
    (0x27E7, 0x27E7, C),
    (0x27E8, 0x27E8, O),
    (0x27E9, 0x27E9, C),
    (0x27EA, 0x27EA, O),
    (0x27EB, 0x27EB, C),
    (0x27EC, 0x27EC, O),
    (0x27ED, 0x27ED, C),
    (0x27EE, 0x27EE, O),
    (0x27EF, 0x27EF, C),
    (0x27F0, 0x27FF, R),
    (0x2900, 0x297F, R),
    (0x2980, 0x2980, F),
    (0x2981, 0x2981, N),
    (0x2982, 0x2982, F),
    (0x2983, 0x2983, O),
    (0x2984, 0x2984, C),
    (0x2985, 0x2985, O),
    (0x2986, 0x2986, C),
    (0x2987, 0x2987, O),
    (0x2988, 0x2988, C),
    (0x2989, 0x2989, O),
    (0x298A, 0x298A, C),
    (0x298B, 0x298B, O),
    (0x298C, 0x298C, C),
    (0x298D, 0x298D, O),
    (0x298E, 0x298E, C),
    (0x298F, 0x298F, O),
    (0x2990, 0x2990, C),
    (0x2991, 0x2991, O),
    (0x2992, 0x2992, C),
    (0x2993, 0x2993, O),
    (0x2994, 0x2994, C),
    (0x2995, 0x2995, O),
    (0x2996, 0x2996, C),
    (0x2997, 0x2997, O),
    (0x2998, 0x2998, C),
    (0x2999, 0x299A, F),
    (0x299B, 0x29B5, N),
    (0x29B6, 0x29B9, B),
    (0x29BA, 0x29BF, N),
    (0x29C0, 0x29C1, B),
    (0x29C2, 0x29C3, N),
    (0x29C4, 0x29C8, B),
    (0x29C9, 0x29CD, N),
    (0x29CE, 0x29D5, R),
    (0x29D6, 0x29D7, B),
    (0x29D8, 0x29D8, O),
    (0x29D9, 0x29D9, C),
    (0x29DA, 0x29DA, O),
    (0x29DB, 0x29DB, C),
    (0x29DC, 0x29DE, N),
    (0x29DF, 0x29DF, R),
    (0x29E0, 0x29E0, N),
    (0x29E1, 0x29E1, R),
    (0x29E2, 0x29E2, B),
    (0x29E3, 0x29E6, R),
    (0x29E7, 0x29EA, N),
    (0x29EB, 0x29EB, B),
    (0x29EC, 0x29F3, N),
    (0x29F4, 0x29F4, R),
    (0x29F5, 0x29F7, B),
    (0x29F8, 0x29F9, L),
    (0x29FA, 0x29FB, B),
    (0x29FC, 0x29FC, O),
    (0x29FD, 0x29FD, C),
    (0x29FE, 0x29FF, B),
    (0x2A00, 0x2A21, L),
    (0x2A22, 0x2A58, B),
    (0x2A59, 0x2A59, R),
    (0x2A5A, 0x2A65, B),
    (0x2A66, 0x2A70, R),
    (0x2A71, 0x2A72, B),
    (0x2A73, 0x2AE0, R),
    (0x2AE1, 0x2AE1, N),
    (0x2AE2, 0x2AF0, R),
    (0x2AF1, 0x2AF1, N),
    (0x2AF2, 0x2AF3, R),
    (0x2AF4, 0x2AF6, B),
    (0x2AF7, 0x2AFA, R),
    (0x2AFB, 0x2AFB, B),
    (0x2AFC, 0x2AFC, L),
    (0x2AFD, 0x2AFE, B),
    (0x2AFF, 0x2AFF, L),
    (0x2B00, 0x2B11, R),
    (0x2B12, 0x2B19, N),
    (0x2B1B, 0x2B2F, N),
    (0x2B30, 0x2B4C, R),
    (0x2B50, 0x2B54, N),
    (0x2B95, 0x2B95, R),
    (0x2BC2, 0x2BC8, N),
    (0x2BCA, 0x2BCB, N),
    (0x3008, 0x3009, X),
    (0x301A, 0x301B, X),
    (0x306E, 0x306E, N),
    (0xFB29, 0xFB29, X),
    (0xFE00, 0xFE00, D),
    (0xFE61, 0xFE66, X),
    (0xFE68, 0xFE68, X),
    (0xFF0B, 0xFF0B, X),
    (0xFF1C, 0xFF1E, X),
    (0xFF3C, 0xFF3C, X),
    (0xFF3E, 0xFF3E, X),
    (0xFF5C, 0xFF5C, X),
    (0xFF5E, 0xFF5E, X),
    (0xFFE2, 0xFFE2, X),
    (0xFFE9, 0xFFEC, X),
    (0x1D400, 0x1D454, A),
    (0x1D456, 0x1D49C, A),
    (0x1D49E, 0x1D49F, A),
    (0x1D4A2, 0x1D4A2, A),
    (0x1D4A5, 0x1D4A6, A),
    (0x1D4A9, 0x1D4AC, A),
    (0x1D4AE, 0x1D4B9, A),
    (0x1D4BB, 0x1D4BB, A),
    (0x1D4BD, 0x1D4C3, A),
    (0x1D4C5, 0x1D505, A),
    (0x1D507, 0x1D50A, A),
    (0x1D50D, 0x1D514, A),
    (0x1D516, 0x1D51C, A),
    (0x1D51E, 0x1D539, A),
    (0x1D53B, 0x1D53E, A),
    (0x1D540, 0x1D544, A),
    (0x1D546, 0x1D546, A),
    (0x1D54A, 0x1D550, A),
    (0x1D552, 0x1D6A5, A),
    (0x1D6A8, 0x1D7CB, A),
    (0x1D7CE, 0x1D7FF, N),
    (0x1EE00, 0x1EE03, A),
    (0x1EE05, 0x1EE1F, A),
    (0x1EE21, 0x1EE22, A),
    (0x1EE24, 0x1EE24, A),
    (0x1EE27, 0x1EE27, A),
    (0x1EE29, 0x1EE32, A),
    (0x1EE34, 0x1EE37, A),
    (0x1EE39, 0x1EE39, A),
    (0x1EE3B, 0x1EE3B, A),
    (0x1EE42, 0x1EE42, A),
    (0x1EE47, 0x1EE47, A),
    (0x1EE49, 0x1EE49, A),
    (0x1EE4B, 0x1EE4B, A),
    (0x1EE4D, 0x1EE4F, A),
    (0x1EE51, 0x1EE52, A),
    (0x1EE54, 0x1EE54, A),
    (0x1EE57, 0x1EE57, A),
    (0x1EE59, 0x1EE59, A),
    (0x1EE5B, 0x1EE5B, A),
    (0x1EE5D, 0x1EE5D, A),
    (0x1EE5F, 0x1EE5F, A),
    (0x1EE61, 0x1EE62, A),
    (0x1EE64, 0x1EE64, A),
    (0x1EE67, 0x1EE6A, A),
    (0x1EE6C, 0x1EE72, A),
    (0x1EE74, 0x1EE77, A),
    (0x1EE79, 0x1EE7C, A),
    (0x1EE7E, 0x1EE7E, A),
    (0x1EE80, 0x1EE89, A),
    (0x1EE8B, 0x1EE9B, A),
    (0x1EEA1, 0x1EEA3, A),
    (0x1EEA5, 0x1EEA9, A),
    (0x1EEAB, 0x1EEBB, A),
    (0x1EEF0, 0x1EEF1, L),
    (0x1F784, 0x1F784, N),
    (0x1F78C, 0x1F78D, N),
    (0x1F797, 0x1F799, N),
    (0x1F79D, 0x1F79F, N),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;

    fn node(text: char, id: u32, kind: MathKind, parent: Option<(u32, u8)>) -> RichSegment {
        RichSegment {
            math_node: Some(MathNode { id, kind }),
            math_parent: parent.map(|(id, slot)| MathSlot { id, slot }),
            ..RichSegment::plain(text)
        }
    }

    fn leaf(text: char, id: u32, slot: u8) -> RichSegment {
        RichSegment {
            math_parent: Some(MathSlot { id, slot }),
            ..RichSegment::plain(text)
        }
    }

    fn check(segments: Vec<RichSegment>) -> Result<(), MathError> {
        RichText {
            segments,
            ..RichText::plain("", Color::BLACK, 12.0, "")
        }
        .validate_math()
    }

    const Z: char = '\u{200B}';

    // `\frac{\sqrt{a^2}}{b}` from #45, inside `Math`.
    fn frac_example() -> Vec<RichSegment> {
        vec![
            node(Z, 0, MathKind::Math, None),
            node(Z, 1, MathKind::Frac(FracStyle::Auto), Some((0, 0))),
            node('√', 2, MathKind::Sqrt, Some((1, 0))),
            node(Z, 3, MathKind::Scripts, Some((2, 0))),
            leaf('a', 3, 0),
            leaf('2', 3, 1),
            leaf('b', 1, 1),
        ]
    }

    #[test]
    fn the_issue_examples_validate() {
        assert_eq!(check(frac_example()), Ok(()));
        // `\sum_{i=1}^{n} x_i`.
        let sum = vec![
            node(Z, 0, MathKind::Math, None),
            node('∑', 1, MathKind::BigOp(Limits::Auto), Some((0, 0))),
            leaf('i', 1, 0),
            leaf('=', 1, 0),
            leaf('1', 1, 0),
            leaf('n', 1, 1),
            node(Z, 2, MathKind::Scripts, Some((0, 0))),
            leaf('x', 2, 0),
            leaf('i', 2, 2),
        ];
        assert_eq!(check(sum), Ok(()));
        // A text-mode `Scripts` root needs no `Math`.
        let power = vec![
            RichSegment::plain('2'),
            node(Z, 0, MathKind::Scripts, None),
            leaf('1', 0, 0),
            leaf('0', 0, 0),
            leaf('3', 0, 1),
        ];
        assert_eq!(check(power), Ok(()));
    }

    fn error(segments: Vec<RichSegment>) -> (usize, MathErrorKind) {
        let e = check(segments).expect_err("malformed");
        (e.at, e.kind)
    }

    #[test]
    fn every_error_kind_is_reported_at_its_segment() {
        use MathErrorKind::*;
        let math = || node(Z, 0, MathKind::Math, None);
        // Ids.
        let mut s = frac_example();
        s[2].math_node.as_mut().unwrap().id = 1;
        assert_eq!(error(s), (2, DuplicateId));
        let mut s = frac_example();
        s[3].math_node.as_mut().unwrap().id = 2;
        assert_eq!(error(s), (3, DuplicateId));
        let s = vec![
            math(),
            node(Z, 5, MathKind::Sqrt, Some((0, 0))),
            leaf('x', 5, 0),
            node(Z, 3, MathKind::Sqrt, Some((0, 0))),
        ];
        assert_eq!(error(s), (3, IdOutOfOrder));
        // Parents: unknown, closed and out of order.
        assert_eq!(error(vec![math(), leaf('x', 7, 0)]), (1, UnknownParent));
        let closed = vec![
            math(),
            node(Z, 1, MathKind::Sqrt, Some((0, 0))),
            leaf('x', 1, 0),
            leaf('+', 0, 0),
            leaf('y', 1, 0),
        ];
        assert_eq!(error(closed), (4, UnknownParent));
        let detached = vec![
            math(),
            leaf('x', 0, 0),
            RichSegment::plain(' '),
            leaf('y', 0, 0),
        ];
        assert_eq!(error(detached), (3, UnknownParent));
        // Slots.
        assert_eq!(error(vec![math(), leaf('x', 0, 1)]), (1, SlotOutOfRange));
        let mut s = frac_example();
        s[6].math_parent = Some(MathSlot { id: 1, slot: 2 });
        assert_eq!(error(s), (6, SlotOutOfRange));
        let order = vec![
            math(),
            node(Z, 1, MathKind::Frac(FracStyle::Auto), Some((0, 0))),
            leaf('b', 1, 1),
            leaf('a', 1, 0),
        ];
        assert_eq!(error(order), (3, SlotOutOfOrder));
        // Depth: 16 nodes nest, a 17th does not.
        let mut deep = vec![math()];
        for id in 1..MAX_MATH_DEPTH as u32 {
            deep.push(node(Z, id, MathKind::Sqrt, Some((id - 1, 0))));
        }
        assert_eq!(check(deep.clone()), Ok(()));
        deep.push(node(Z, 16, MathKind::Sqrt, Some((15, 0))));
        assert_eq!(error(deep), (16, TooDeep));
        // Kind rules.
        assert_eq!(
            error(vec![node('√', 0, MathKind::Sqrt, None)]),
            (0, MissingMathAncestor)
        );
        let in_text_scripts = vec![
            node(Z, 0, MathKind::Scripts, None),
            node(Z, 1, MathKind::Frac(FracStyle::Auto), Some((0, 0))),
        ];
        assert_eq!(error(in_text_scripts), (1, MissingMathAncestor));
        assert_eq!(
            error(vec![math(), node(Z, 1, MathKind::Math, Some((0, 0)))]),
            (1, NestedMath)
        );
        assert_eq!(
            error(vec![math(), node('|', 1, MathKind::Middle, Some((0, 0)))]),
            (1, MiddleOutsideDelim)
        );
        let tab = RichSegment {
            math_parent: Some(MathSlot { id: 0, slot: 0 }),
            ..RichSegment::plain('\t')
        };
        assert_eq!(error(vec![math(), tab]), (1, BreakInMath));
        let flagged = RichSegment {
            superscript: true,
            ..leaf('2', 0, 0)
        };
        assert_eq!(error(vec![math(), flagged]), (1, StyleFlagInMath));
        let bad_delim = node(
            Z,
            1,
            MathKind::Delim {
                open: 'x',
                close: ')',
            },
            Some((0, 0)),
        );
        assert_eq!(error(vec![math(), bad_delim]), (1, BadDelimiter));
        let bad_level = node(
            '(',
            1,
            MathKind::Sized {
                level: 5,
                class: MathClass::Opening,
            },
            Some((0, 0)),
        );
        assert_eq!(error(vec![math(), bad_level]), (1, BadDelimiter));
        let bad_class = node(
            '(',
            1,
            MathKind::Sized {
                level: 1,
                class: MathClass::Binary,
            },
            Some((0, 0)),
        );
        assert_eq!(error(vec![math(), bad_class]), (1, BadDelimiter));
        // Every kind has a reason of its own.
        let kinds = [
            DuplicateId,
            IdOutOfOrder,
            UnknownParent,
            SlotOutOfRange,
            SlotOutOfOrder,
            TooDeep,
            MissingMathAncestor,
            NestedMath,
            MiddleOutsideDelim,
            BreakInMath,
            StyleFlagInMath,
            BadDelimiter,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                assert_ne!(a.as_str(), b.as_str());
            }
        }
    }

    // D1: ids increase within a cell and are unique only there, so two
    // legend labels joined by tabs or line breaks may both start at id 0.
    #[test]
    fn ids_are_scoped_to_their_cell() {
        let mut s = frac_example();
        s.push(RichSegment::plain('\t'));
        s.extend(frac_example());
        s.push(RichSegment::plain('\n'));
        s.extend(frac_example());
        assert_eq!(check(s), Ok(()));
        // Gaps are allowed.
        let gaps = vec![
            node(Z, 3, MathKind::Math, None),
            node(Z, 10, MathKind::Sqrt, Some((3, 0))),
            leaf('x', 10, 0),
        ];
        assert_eq!(check(gaps), Ok(()));
    }

    #[test]
    fn subtree_end_spans_the_node_and_its_descendants() {
        let s = frac_example();
        assert_eq!(subtree_end(&s, 0), 7);
        assert_eq!(subtree_end(&s, 1), 7);
        assert_eq!(subtree_end(&s, 2), 6);
        assert_eq!(subtree_end(&s, 3), 6);
        assert_eq!(subtree_end(&s, 4), 5);
        let mut t = frac_example();
        t.push(RichSegment::plain('!'));
        assert_eq!(subtree_end(&t, 0), 7);
    }

    #[test]
    fn the_class_table_is_sorted_and_classifies() {
        for w in MATH_CLASS_RANGES.windows(2) {
            assert!(w[0].0 <= w[0].1 && w[0].1 < w[1].0, "{:X?}", w);
        }
        assert_eq!(math_class('+'), MathClass::Vary);
        assert_eq!(math_class('−'), MathClass::Vary);
        assert_eq!(math_class('='), MathClass::Relation);
        assert_eq!(math_class('('), MathClass::Opening);
        assert_eq!(math_class(')'), MathClass::Closing);
        assert_eq!(math_class('|'), MathClass::Fence);
        assert_eq!(math_class(','), MathClass::Punctuation);
        assert_eq!(math_class('∑'), MathClass::Large);
        assert_eq!(math_class('x'), MathClass::Alphabetic);
        assert_eq!(math_class('1'), MathClass::Normal);
        assert_eq!(math_class('×'), MathClass::Binary);
        assert_eq!(math_class('→'), MathClass::Relation);
        assert_eq!(math_class(' '), MathClass::Space);
        assert_eq!(math_class('\u{10FFFF}'), MathClass::Special);
    }

    fn every_kind() -> [MathKind; 13] {
        [
            MathKind::Math,
            MathKind::Frac(FracStyle::Display),
            MathKind::Binom(FracStyle::Text),
            MathKind::Sqrt,
            MathKind::Scripts,
            MathKind::BigOp(Limits::Limits),
            MathKind::OpName(Limits::NoLimits),
            MathKind::Delim {
                open: '(',
                close: '.',
            },
            MathKind::Middle,
            MathKind::Sized {
                level: 4,
                class: MathClass::Opening,
            },
            MathKind::Accent(AccentKind::Wide),
            MathKind::Over(OverUnder::Brace),
            MathKind::Under(OverUnder::Set),
        ]
    }

    // The wire form of each kind is pinned: externally tagged, unit kinds as
    // their name, and every kind round-trips.
    #[cfg(feature = "serde")]
    #[test]
    fn every_kind_has_a_pinned_wire_form_and_round_trips() {
        let pinned = [
            r#""Math""#,
            r#"{"Frac":"Display"}"#,
            r#"{"Binom":"Text"}"#,
            r#""Sqrt""#,
            r#""Scripts""#,
            r#"{"BigOp":"Limits"}"#,
            r#"{"OpName":"NoLimits"}"#,
            r#"{"Delim":{"open":"(","close":"."}}"#,
            r#""Middle""#,
            r#"{"Sized":{"level":4,"class":"Opening"}}"#,
            r#"{"Accent":"Wide"}"#,
            r#"{"Over":"Brace"}"#,
            r#"{"Under":"Set"}"#,
        ];
        for (kind, json) in every_kind().into_iter().zip(pinned) {
            assert_eq!(serde_json::to_string(&kind).unwrap(), json);
            assert_eq!(serde_json::from_str::<MathKind>(json).unwrap(), kind);
        }
        for kind in [FracStyle::Auto, FracStyle::Display, FracStyle::Text] {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(serde_json::from_str::<FracStyle>(&json).unwrap(), kind);
        }
        for kind in [Limits::Auto, Limits::Limits, Limits::NoLimits] {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(serde_json::from_str::<Limits>(&json).unwrap(), kind);
        }
        for kind in [
            OverUnder::Line,
            OverUnder::Brace,
            OverUnder::Arrow,
            OverUnder::Set,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(serde_json::from_str::<OverUnder>(&json).unwrap(), kind);
        }
        assert_eq!(
            serde_json::to_string(&AccentKind::Fixed).unwrap(),
            r#""Fixed""#
        );
    }

    // The two fields are written last, only when set; a segment carrying
    // both reads back as written, and a text of every kind round-trips byte
    // for byte.
    #[cfg(feature = "serde")]
    #[test]
    fn math_fields_are_additive_on_the_wire() {
        let seg = node('√', 2, MathKind::Sqrt, Some((1, 0)));
        let json = serde_json::to_string(&seg).unwrap();
        assert_eq!(
            json,
            r#"{"text":"√","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"greek":false,"math_node":{"id":2,"kind":"Sqrt"},"math_parent":{"id":1,"slot":0}}"#
        );
        assert_eq!(serde_json::from_str::<RichSegment>(&json).unwrap(), seg);
        let plain = serde_json::to_string(&RichSegment::plain('a')).unwrap();
        assert!(!plain.contains("math"), "{plain}");

        // Every kind in one valid text.
        let mut segments = vec![node(Z, 0, MathKind::Math, None)];
        let mut id = 1;
        for kind in every_kind().into_iter().skip(1) {
            match kind {
                MathKind::Middle => {
                    let delim = MathKind::Delim {
                        open: '[',
                        close: ']',
                    };
                    segments.push(node(Z, id, delim, Some((0, 0))));
                    segments.push(node('|', id + 1, kind, Some((id, 0))));
                    id += 2;
                }
                MathKind::Sized { .. } => {
                    segments.push(node('(', id, kind, Some((0, 0))));
                    id += 1;
                }
                _ => {
                    segments.push(node(Z, id, kind, Some((0, 0))));
                    for slot in 0..kind.slot_count() {
                        segments.push(leaf('x', id, slot));
                    }
                    id += 1;
                }
            }
        }
        let text = RichText {
            segments,
            ..RichText::plain("", Color::BLACK, 12.0, "")
        };
        assert_eq!(text.validate_math(), Ok(()));
        let json = serde_json::to_string(&text).unwrap();
        let back: RichText = serde_json::from_str(&json).unwrap();
        assert_eq!(back, text);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // A malformed tree fails at parse, with the segment and the reason, in a
    // whole document as in a lone text.
    #[cfg(feature = "serde")]
    #[test]
    fn a_malformed_tree_fails_to_parse() {
        let mut s = frac_example();
        s[6].math_parent = Some(MathSlot { id: 9, slot: 0 });
        let text = RichText {
            segments: s,
            ..RichText::plain("", Color::BLACK, 12.0, "")
        };
        let json = serde_json::to_string(&text).unwrap();
        let err = serde_json::from_str::<RichText>(&json)
            .expect_err("malformed tree")
            .to_string();
        assert!(err.contains("segment 6"), "{err}");
        assert!(err.contains(MathErrorKind::UnknownParent.as_str()), "{err}");

        let mut config = crate::default::default_config();
        config.chart_title.text = text;
        let json = serde_json::to_string(&config).unwrap();
        assert!(serde_json::from_str::<crate::config::Config>(&json).is_err());
        config.chart_title.text.segments = frac_example();
        let json = serde_json::to_string(&config).unwrap();
        let back: crate::config::Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);
    }

    // The model's own validators (annotation texts, bounded titles, the
    // radial title) refuse a malformed tree with its reason.
    #[test]
    fn the_model_validators_refuse_a_malformed_tree() {
        let mut segments = frac_example();
        segments[4].math_parent = Some(MathSlot { id: 3, slot: 7 });
        let broken = RichText {
            segments,
            ..RichText::plain("", Color::BLACK, 12.0, "")
        };
        let reason = MathErrorKind::SlotOutOfRange.as_str();
        let layer = crate::layer::AnnotationLayer {
            items: vec![crate::annotation::Annotation::new(
                crate::annotation::AnnotationKind::Text {
                    anchor: crate::annotation::Anchor::Ratio { x: 0.5, y: 0.5 },
                    text: broken.clone(),
                    box_style: None,
                },
            )],
            ..Default::default()
        };
        assert_eq!(layer.validate(), Err(reason));
        let title = crate::bounded_title::BoundedTitleOptions {
            visible: true,
            text: broken.clone(),
        };
        assert_eq!(title.validate(), Err(reason));
        let mut radial = crate::radial::RadialChart::default();
        radial.chart_title = Some(crate::config::ChartTitleOptions {
            text: broken,
            ..radial.resolved_title()
        });
        assert_eq!(radial.validate(), Err(reason));
    }

    // `\!` is a space of negative width: allowed on a space inside `Math`
    // only.
    #[test]
    fn a_negative_field_is_limited_to_math_spaces() {
        let space = |field: f32, parent: Option<(u32, u8)>| RichSegment {
            field_em: Some(field),
            math_parent: parent.map(|(id, slot)| MathSlot { id, slot }),
            ..RichSegment::plain(' ')
        };
        let text = |segments| RichText {
            segments,
            ..RichText::plain("", Color::BLACK, 12.0, "")
        };
        let math = node(Z, 0, MathKind::Math, None);
        assert!(text(vec![math.clone(), space(-3. / 18., Some((0, 0)))]).within_limits());
        assert!(text(vec![math.clone(), space(3. / 18., Some((0, 0)))]).within_limits());
        assert!(!text(vec![space(-3. / 18., None)]).within_limits());
        let in_text_scripts = vec![
            node(Z, 0, MathKind::Scripts, None),
            space(-0.1, Some((0, 0))),
        ];
        assert!(!text(in_text_scripts).within_limits());
        let x = RichSegment {
            field_em: Some(-0.1),
            ..leaf('x', 0, 0)
        };
        assert!(!text(vec![math.clone(), x]).within_limits());
        assert!(!text(vec![math, space(-33., Some((0, 0)))]).within_limits());
    }

    #[test]
    fn math_alphanumerics_map_with_the_letterlike_holes() {
        assert_eq!(math_alphanumeric('x', false, false), 'x');
        assert_eq!(math_alphanumeric('x', false, true), '\u{1D465}');
        assert_eq!(math_alphanumeric('h', false, true), '\u{210E}');
        assert_eq!(math_alphanumeric('A', true, false), '\u{1D400}');
        assert_eq!(math_alphanumeric('A', true, true), '\u{1D468}');
        assert_eq!(math_alphanumeric('z', true, true), '\u{1D49B}');
        assert_eq!(math_alphanumeric('0', true, false), '\u{1D7CE}');
        assert_eq!(math_alphanumeric('0', false, true), '0');
        assert_eq!(math_alphanumeric('α', false, true), '\u{1D6FC}');
        assert_eq!(math_alphanumeric('Γ', false, true), '\u{1D6E4}');
        assert_eq!(math_alphanumeric('ω', true, false), '\u{1D6DA}');
        assert_eq!(math_alphanumeric('ϖ', true, true), '\u{1D755}');
        assert_eq!(math_alphanumeric('∂', false, true), '\u{1D715}');
        assert_eq!(math_alphanumeric('ϴ', true, false), '\u{1D6B9}');
        assert_eq!(math_alphanumeric('ı', false, true), '\u{1D6A4}');
        assert_eq!(math_alphanumeric('+', true, true), '+');
    }
}
