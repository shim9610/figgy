//! `RichText::from_tex`: a convenience parser from a TeX subset to the math
//! segments of [`super::math`]. The TeX source is not stored; the segments
//! are the only SSOT, and hosts may build them directly instead.
//!
//! D2: the input is text with math spans, `$…$` or `\(…\)` (`\$` is a
//! dollar sign) in text style and `$$…$$` in display style; each span
//! becomes one `Math` node of that style, which a leading `\displaystyle`
//! or `\textstyle` sets too. The supported commands
//! are listed in [`RichText::from_tex`]. A letter is written as an italic
//! math character (`x` with `italic: true`), not as its Mathematical Italic
//! code point, so a consumer that ignores math nodes still draws an italic
//! x.
//!
//! The parser is a recursive descent over byte offsets that writes straight
//! into the output segment list: no token list and no syntax tree. A node
//! is written before its children; where TeX names the children first
//! (`x^2`, `\sqrt[n]{x}`, `\overset{a}{b}`) the written run is rotated in
//! place. Node ids are renumbered 0, 1, 2… in order of appearance at the
//! end, and the result is checked by [`MathCursor`].

use std::ops::Range;

use super::math::{
    AccentKind, FracStyle, Limits, MAX_MATH_DEPTH, MathClass, MathCursor, MathKind, MathNode,
    MathSlot, MathStyle, OverUnder,
};
use super::{CharSet, RichSegment, RichText, TextAlign};
use crate::color::Color;

/// Why a TeX string cannot be converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TexErrorKind {
    /// A command the subset does not know.
    UnknownCommand,
    /// A `}` without a `{`, or a `{` never closed.
    UnbalancedBrace,
    /// A command, `^` or `_` without its argument.
    MissingArgument,
    /// The input ends inside a math span or a group.
    UnexpectedEnd,
    /// Two superscripts or two subscripts on one base.
    DoubleScript,
    /// A `\right` without a `\left`, a `\left` without a `\right`, or a
    /// `\middle` outside them.
    UnpairedLeftRight,
    /// A construct the subset excludes: environments (`matrix`, `cases`,
    /// `array`, `align`), `\\`, `\tag`, macro definitions, `&`, and a
    /// `\displaystyle` or `\textstyle` anywhere but at the start of a math
    /// span.
    Unsupported,
    /// Math nodes nested deeper than [`MAX_MATH_DEPTH`].
    TooDeep,
    /// A `\left`, `\right`, `\middle` or `\big` argument that is not a
    /// delimiter.
    BadDelimiter,
}

impl TexErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TexErrorKind::UnknownCommand => "unknown command",
            TexErrorKind::UnbalancedBrace => "unbalanced brace",
            TexErrorKind::MissingArgument => "missing argument",
            TexErrorKind::UnexpectedEnd => "unexpected end of input",
            TexErrorKind::DoubleScript => "double superscript or subscript",
            TexErrorKind::UnpairedLeftRight => "unpaired \\left, \\right or \\middle",
            TexErrorKind::Unsupported => {
                "unsupported construct (an environment, `\\\\`, `&`, `\\tag`, a macro definition, \
                 or a style command other than a leading `\\displaystyle` or `\\textstyle` of a \
                 math span)"
            }
            TexErrorKind::TooDeep => "math nested deeper than 16",
            TexErrorKind::BadDelimiter => "not a delimiter",
        }
    }
}

/// A TeX string that cannot be converted: what is wrong, at which bytes of
/// the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TexError {
    pub at: Range<usize>,
    pub kind: TexErrorKind,
}

impl std::fmt::Display for TexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TeX bytes {}..{}: {}",
            self.at.start,
            self.at.end,
            self.kind.as_str()
        )
    }
}

impl std::error::Error for TexError {}

impl RichText {
    /// Convert `tex`, text with `$…$` or `\(…\)` math spans in text style and
    /// `$$…$$` spans in display style, into a rich text of `color`,
    /// `font_size` and `font`. A `\displaystyle` or `\textstyle` at the
    /// start of a span sets its style.
    ///
    /// Inside math: letters (italic), digits and operators; `^`, `_` and
    /// primes; groups; `\frac`, `\dfrac`, `\tfrac`, `\binom` (and `\dbinom`,
    /// `\tbinom`); `\sqrt` and `\sqrt[n]`; `\sum`, `\prod`, `\coprod`,
    /// `\int`, `\iint`, `\iiint`, `\oint`, `\bigcup`, `\bigcap`,
    /// `\bigoplus`, `\bigotimes`, `\bigvee`, `\bigwedge` with `\limits` and
    /// `\nolimits`; `\lim`, `\max`, `\min`, `\sup`, `\inf`, `\det`, `\gcd`
    /// and the function names (`\sin`, `\cos`, `\log`, `\ln`, `\exp`, …),
    /// `\operatorname{…}`; `\left … \middle … \right` and `\big` … `\Bigg`
    /// with their `l`, `r` and `m` forms; the accents `\hat`, `\bar`,
    /// `\vec`, `\dot`, `\ddot`, `\tilde`, `\check`, `\breve`, `\acute`,
    /// `\grave`, `\widehat`, `\widetilde`, `\overrightarrow`,
    /// `\overleftarrow`; `\overline`, `\underline`, `\overbrace`,
    /// `\underbrace`, `\overset`, `\underset`, `\stackrel`, `\xrightarrow`,
    /// `\xleftarrow` (and `\xRightarrow`, `\xLeftarrow`,
    /// `\xleftrightarrow`); Greek letters; `\text`, `\mathrm`, `\mathbf`,
    /// `\mathit`, `\mathcal`, `\mathbb`; the spaces `\,`, `\:`, `\;`, `\!`,
    /// `\ `, `\quad`, `\qquad`; and the common operator, relation, arrow and
    /// symbol macros.
    pub fn from_tex(
        tex: &str,
        color: Color,
        font_size: f32,
        font: impl Into<String>,
    ) -> Result<RichText, TexError> {
        Ok(RichText {
            segments: rich_segments_from_tex(tex)?,
            color,
            font_size,
            font: font.into(),
            align: TextAlign::Left,
        })
    }
}

/// The segments of [`RichText::from_tex`].
pub fn rich_segments_from_tex(tex: &str) -> Result<Vec<RichSegment>, TexError> {
    let mut p = Parser {
        src: tex,
        pos: 0,
        out: Vec::new(),
        next_id: 0,
        depth: 0,
    };
    p.text_mode()?;
    renumber(&mut p.out);
    let mut cursor = MathCursor::new();
    for (at, seg) in p.out.iter().enumerate() {
        if let Err(e) = cursor.step(at, seg) {
            let kind = match e.kind {
                super::MathErrorKind::TooDeep => TexErrorKind::TooDeep,
                _ => TexErrorKind::Unsupported,
            };
            return Err(TexError {
                at: 0..tex.len(),
                kind,
            });
        }
    }
    Ok(p.out)
}

/// Renumber node ids 0, 1, 2… per cell in order of appearance, with the
/// parents following: a fixed stack of (old, new) ids of the open nodes.
fn renumber(segments: &mut [RichSegment]) {
    let mut open = [(0u32, 0u32); MAX_MATH_DEPTH];
    let mut depth = 0;
    let mut next = 0u32;
    for seg in segments.iter_mut() {
        if matches!(seg.text, '\n' | '\t') {
            depth = 0;
            next = 0;
            continue;
        }
        match &mut seg.math_parent {
            None => depth = 0,
            Some(parent) => {
                if let Some(i) = open[..depth].iter().rposition(|o| o.0 == parent.id) {
                    depth = i + 1;
                    parent.id = open[i].1;
                }
            }
        }
        if let Some(node) = &mut seg.math_node {
            if depth < MAX_MATH_DEPTH {
                open[depth] = (node.id, next);
                depth += 1;
            }
            node.id = next;
            next += 1;
        }
    }
}

/// The alphabet letters and digits are written in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Alphabet {
    /// Italic letters, upright digits and capital Greek (TeX's default).
    Normal,
    Roman,
    Bold,
    Italic,
    Calligraphic,
    Blackboard,
}

/// What a row stops at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Until {
    Dollar,
    /// The `$$` closing a display span.
    DoubleDollar,
    ParenClose,
    Brace,
    Bracket,
    Right,
}

/// What the latest atom of a row is, for the scripts and `\limits` after
/// it.
#[derive(Clone, Copy)]
struct Last {
    /// Where the atom's segments start.
    start: usize,
    /// The node scripts attach to and its kind, once there is one.
    node: Option<usize>,
    /// The content start of each script slot written so far.
    slot_start: [Option<usize>; 3],
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    out: Vec<RichSegment>,
    next_id: u32,
    /// Nodes open around the current position.
    depth: usize,
}

const ZWSP: char = '\u{200B}';

fn err<T>(at: Range<usize>, kind: TexErrorKind) -> Result<T, TexError> {
    Err(TexError { at, kind })
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    /// Text outside math: characters as `RichText::plain` writes them, math
    /// spans as `Math` nodes.
    fn text_mode(&mut self) -> Result<(), TexError> {
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                '\\' if self.src[self.pos..].starts_with("\\$") => {
                    self.pos += 2;
                    self.out.push(RichSegment::plain('$'));
                }
                '\\' if self.src[self.pos..].starts_with("\\(") => {
                    self.pos += 2;
                    self.math_span(Until::ParenClose, start)?;
                }
                '$' => {
                    self.pos += 1;
                    if self.peek() == Some('$') {
                        self.pos += 1;
                        self.math_span(Until::DoubleDollar, start)?;
                    } else {
                        self.math_span(Until::Dollar, start)?;
                    }
                }
                _ => {
                    self.bump();
                    self.out.push(super::rich_segment_from_char(c));
                }
            }
        }
        Ok(())
    }

    /// A math span: one `Math` node in display style for `$$…$$`, in text
    /// style for `$…$` and `\(…\)`. A `\displaystyle` or `\textstyle` at the
    /// start of the span sets the node's style; anywhere else it is
    /// unsupported.
    fn math_span(&mut self, until: Until, start: usize) -> Result<(), TexError> {
        let mut style = if until == Until::DoubleDollar {
            MathStyle::Display
        } else {
            MathStyle::Text
        };
        let index = self.out.len();
        let id = self.node(ZWSP, MathKind::Math(style), None, start)?;
        loop {
            self.skip_spaces();
            let rest = &self.src[self.pos..];
            let command = ["\\displaystyle", "\\textstyle"].into_iter().find(|c| {
                rest.starts_with(c)
                    && !rest[c.len()..].starts_with(|n: char| n.is_ascii_alphabetic())
            });
            let Some(command) = command else { break };
            self.pos += command.len();
            style = if command == "\\displaystyle" {
                MathStyle::Display
            } else {
                MathStyle::Text
            };
        }
        if let Some(node) = &mut self.out[index].math_node {
            node.kind = MathKind::Math(style);
        }
        self.row(MathSlot { id, slot: 0 }, until, Alphabet::Normal)?;
        self.depth -= 1;
        Ok(())
    }

    /// Write a node segment and open it; returns its id.
    fn node(
        &mut self,
        text: char,
        kind: MathKind,
        parent: Option<MathSlot>,
        at: usize,
    ) -> Result<u32, TexError> {
        if self.depth == MAX_MATH_DEPTH {
            return err(at..self.pos, TexErrorKind::TooDeep);
        }
        self.depth += 1;
        let id = self.next_id;
        self.next_id += 1;
        self.out.push(RichSegment {
            math_node: Some(MathNode { id, kind }),
            math_parent: parent,
            ..RichSegment::plain(text)
        });
        Ok(id)
    }

    /// Write a leaf.
    fn leaf(&mut self, seg: RichSegment, parent: MathSlot) {
        self.out.push(RichSegment {
            math_parent: Some(parent),
            ..seg
        });
    }

    /// A math character in `alphabet`.
    fn math_char(&mut self, c: char, parent: MathSlot, alphabet: Alphabet) {
        let letter = c.is_ascii_alphabetic();
        let greek = ('α'..='ω').contains(&c) || matches!(c, 'ϵ' | 'ϑ' | 'ϕ' | 'ϖ' | 'ϱ' | 'ϰ');
        let (c, bold, italic) = match alphabet {
            Alphabet::Normal => (c, false, letter || greek),
            Alphabet::Roman => (c, false, false),
            Alphabet::Bold => (c, true, false),
            Alphabet::Italic => (c, false, letter || greek || ('Α'..='Ω').contains(&c)),
            Alphabet::Calligraphic => (calligraphic(c), false, false),
            Alphabet::Blackboard => (blackboard(c), false, false),
        };
        self.leaf(
            RichSegment {
                char_set: CharSet::Math,
                bold,
                italic,
                ..RichSegment::plain(c)
            },
            parent,
        );
    }

    /// A math row: atoms, scripts and `\limits` until `until`.
    fn row(&mut self, parent: MathSlot, until: Until, alphabet: Alphabet) -> Result<(), TexError> {
        let mut last: Option<Last> = None;
        loop {
            self.skip_spaces();
            let start = self.pos;
            let Some(c) = self.peek() else {
                return match until {
                    Until::Right => err(start..start, TexErrorKind::UnpairedLeftRight),
                    Until::Brace => err(start..start, TexErrorKind::UnbalancedBrace),
                    _ => err(start..start, TexErrorKind::UnexpectedEnd),
                };
            };
            match c {
                '$' => {
                    self.bump();
                    return match until {
                        Until::Dollar => Ok(()),
                        Until::DoubleDollar if self.peek() == Some('$') => {
                            self.bump();
                            Ok(())
                        }
                        Until::DoubleDollar => err(start..self.pos, TexErrorKind::Unsupported),
                        Until::Right => err(start..self.pos, TexErrorKind::UnpairedLeftRight),
                        Until::Brace | Until::Bracket => {
                            err(start..self.pos, TexErrorKind::UnbalancedBrace)
                        }
                        Until::ParenClose => err(start..self.pos, TexErrorKind::Unsupported),
                    };
                }
                '}' => {
                    self.bump();
                    if until == Until::Brace {
                        return Ok(());
                    }
                    return err(start..self.pos, TexErrorKind::UnbalancedBrace);
                }
                ']' if until == Until::Bracket => {
                    self.bump();
                    return Ok(());
                }
                '^' | '_' => {
                    self.bump();
                    let Some(l) = last.as_mut() else {
                        // A script with no base gets an empty one.
                        let empty = self.out.len();
                        let mut l = Last {
                            start: empty,
                            node: None,
                            slot_start: [None; 3],
                        };
                        self.script(&mut l, c == '^', parent, start, alphabet)?;
                        last = Some(l);
                        continue;
                    };
                    self.script(l, c == '^', parent, start, alphabet)?;
                }
                '\'' => {
                    let mut primes = 0;
                    while self.peek() == Some('\'') {
                        self.bump();
                        primes += 1;
                    }
                    let prime = match primes {
                        1 => '′',
                        2 => '″',
                        _ => '‴',
                    };
                    let mut l = last.unwrap_or(Last {
                        start: self.out.len(),
                        node: None,
                        slot_start: [None; 3],
                    });
                    let (id, slot) = self.script_slot(&mut l, true, parent, start)?;
                    self.math_char(prime, MathSlot { id, slot }, Alphabet::Roman);
                    self.depth -= 1;
                    last = Some(l);
                }
                '&' | '#' => return err(start..start + 1, TexErrorKind::Unsupported),
                '\\' => {
                    let name = self.command_name();
                    match name {
                        "\\)" if until == Until::ParenClose => return Ok(()),
                        "\\right" => {
                            if until == Until::Right {
                                return Ok(());
                            }
                            return err(start..self.pos, TexErrorKind::UnpairedLeftRight);
                        }
                        "\\limits" | "\\nolimits" => {
                            let limits = if name == "\\limits" {
                                Limits::Limits
                            } else {
                                Limits::NoLimits
                            };
                            let node = last.and_then(|l| l.node);
                            let Some(seg) = node.and_then(|i| self.out.get_mut(i)) else {
                                return err(start..self.pos, TexErrorKind::Unsupported);
                            };
                            match &mut seg.math_node {
                                Some(MathNode {
                                    kind: MathKind::BigOp(l) | MathKind::OpName(l),
                                    ..
                                }) => *l = limits,
                                _ => return err(start..self.pos, TexErrorKind::Unsupported),
                            }
                        }
                        "\\middle" => {
                            let delim = self.delimiter()?;
                            if until != Until::Right {
                                return err(start..self.pos, TexErrorKind::UnpairedLeftRight);
                            }
                            self.node(delim, MathKind::Middle, Some(parent), start)?;
                            self.depth -= 1;
                            last = None;
                        }
                        _ => {
                            self.pos = start;
                            last = Some(self.atom(parent, alphabet)?);
                        }
                    }
                }
                _ => {
                    last = Some(self.atom(parent, alphabet)?);
                }
            }
        }
    }

    /// The command at the position: `\` and its letters, or `\` and one
    /// other character.
    fn command_name(&mut self) -> &'a str {
        let src = self.src;
        let start = self.pos;
        self.bump();
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                    self.bump();
                }
            }
            Some(_) => {
                self.bump();
            }
            None => {}
        }
        &src[start..self.pos]
    }

    /// One atom: a character, a group or a command with its arguments.
    fn atom(&mut self, parent: MathSlot, alphabet: Alphabet) -> Result<Last, TexError> {
        self.skip_spaces();
        let start = self.pos;
        let first = self.out.len();
        let plain = Last {
            start: first,
            node: None,
            slot_start: [None; 3],
        };
        let Some(c) = self.peek() else {
            return err(start..start, TexErrorKind::MissingArgument);
        };
        match c {
            '{' => {
                self.bump();
                self.row(parent, Until::Brace, alphabet)?;
                return Ok(plain);
            }
            '}' | '^' | '_' | '$' => return err(start..start + 1, TexErrorKind::MissingArgument),
            '\\' => {}
            _ => {
                self.bump();
                let c = match c {
                    '-' => '−',
                    '*' => '∗',
                    other => other,
                };
                self.math_char(c, parent, alphabet);
                return Ok(plain);
            }
        }
        let name = self.command_name();
        let here = start..self.pos;
        if let Some(symbol) = symbol(name) {
            self.math_char(symbol, parent, Alphabet::Roman);
            return Ok(plain);
        }
        if let Some((letter, upright)) = greek(name) {
            let alphabet = match alphabet {
                Alphabet::Normal if upright => Alphabet::Roman,
                other => other,
            };
            self.math_char(letter, parent, alphabet);
            return Ok(plain);
        }
        if let Some(em) = space(name) {
            self.leaf(
                RichSegment {
                    field_em: em,
                    ..RichSegment::plain(' ')
                },
                parent,
            );
            return Ok(plain);
        }
        if let Some(op) = big_operator(name) {
            self.node(op, MathKind::BigOp(Limits::Auto), Some(parent), start)?;
            self.depth -= 1;
            return Ok(Last {
                node: Some(first),
                ..plain
            });
        }
        if let Some((op, limits)) = operator_name(name) {
            let id = self.node(ZWSP, MathKind::OpName(limits), Some(parent), start)?;
            for c in op.chars() {
                self.math_char(c, MathSlot { id, slot: 0 }, Alphabet::Roman);
            }
            self.depth -= 1;
            return Ok(Last {
                node: Some(first),
                ..plain
            });
        }
        if let Some((mark, kind)) = accent(name) {
            let id = self.node(mark, MathKind::Accent(kind), Some(parent), start)?;
            self.argument(MathSlot { id, slot: 0 }, alphabet, here)?;
            self.depth -= 1;
            return Ok(plain);
        }
        if let Some((level, class)) = sized(name) {
            let delim = self.delimiter()?;
            self.node(delim, MathKind::Sized { level, class }, Some(parent), start)?;
            self.depth -= 1;
            return Ok(plain);
        }
        if let Some(alphabet) = font(name) {
            self.argument(parent, alphabet, here)?;
            return Ok(plain);
        }
        match name {
            "\\frac" | "\\dfrac" | "\\tfrac" | "\\binom" | "\\dbinom" | "\\tbinom" => {
                let style = match name.as_bytes()[1] {
                    b'd' => FracStyle::Display,
                    b't' => FracStyle::Text,
                    _ => FracStyle::Auto,
                };
                let kind = if name.ends_with("binom") {
                    MathKind::Binom(style)
                } else {
                    MathKind::Frac(style)
                };
                let id = self.node(ZWSP, kind, Some(parent), start)?;
                self.argument(MathSlot { id, slot: 0 }, alphabet, here.clone())?;
                self.argument(MathSlot { id, slot: 1 }, alphabet, here)?;
                self.depth -= 1;
            }
            "\\sqrt" => {
                let id = self.node('√', MathKind::Sqrt, Some(parent), start)?;
                self.skip_spaces();
                let index = (self.peek() == Some('[')).then(|| {
                    self.bump();
                    self.out.len()
                });
                if let Some(index_start) = index {
                    self.row(MathSlot { id, slot: 1 }, Until::Bracket, alphabet)?;
                    let body_start = self.out.len();
                    self.argument(MathSlot { id, slot: 0 }, alphabet, here)?;
                    // The radicand (slot 0) goes before the index (slot 1).
                    self.out[index_start..].rotate_left(body_start - index_start);
                } else {
                    self.argument(MathSlot { id, slot: 0 }, alphabet, here)?;
                }
                self.depth -= 1;
            }
            "\\operatorname" | "\\operatornamewithlimits" => {
                let limits = if name == "\\operatorname" && self.peek() != Some('*') {
                    Limits::NoLimits
                } else {
                    if self.peek() == Some('*') {
                        self.bump();
                    }
                    Limits::Auto
                };
                let id = self.node(ZWSP, MathKind::OpName(limits), Some(parent), start)?;
                self.argument(MathSlot { id, slot: 0 }, Alphabet::Roman, here)?;
                self.depth -= 1;
                return Ok(Last {
                    node: Some(first),
                    ..plain
                });
            }
            "\\left" => {
                let open = self.delimiter()?;
                let id = self.node(
                    ZWSP,
                    MathKind::Delim { open, close: '.' },
                    Some(parent),
                    start,
                )?;
                self.row(MathSlot { id, slot: 0 }, Until::Right, alphabet)?;
                let close = self.delimiter()?;
                if let Some(MathNode {
                    kind: MathKind::Delim { close: c, .. },
                    ..
                }) = &mut self.out[first].math_node
                {
                    *c = close;
                }
                self.depth -= 1;
            }
            "\\overline" | "\\underline" => {
                let over = name == "\\overline";
                let kind = if over {
                    MathKind::Over(OverUnder::Line)
                } else {
                    MathKind::Under(OverUnder::Line)
                };
                let id = self.node(if over { '‾' } else { '_' }, kind, Some(parent), start)?;
                self.argument(MathSlot { id, slot: 0 }, alphabet, here)?;
                self.depth -= 1;
            }
            "\\overbrace" | "\\underbrace" => {
                let over = name == "\\overbrace";
                let kind = if over {
                    MathKind::Over(OverUnder::Brace)
                } else {
                    MathKind::Under(OverUnder::Brace)
                };
                let id = self.node(if over { '⏞' } else { '⏟' }, kind, Some(parent), start)?;
                self.argument(MathSlot { id, slot: 0 }, alphabet, here)?;
                self.depth -= 1;
                return Ok(Last {
                    node: Some(first),
                    ..plain
                });
            }
            "\\overset" | "\\underset" | "\\stackrel" => {
                let kind = if name == "\\underset" {
                    MathKind::Under(OverUnder::Set)
                } else {
                    MathKind::Over(OverUnder::Set)
                };
                let id = self.node(ZWSP, kind, Some(parent), start)?;
                let label_start = self.out.len();
                self.argument(MathSlot { id, slot: 1 }, alphabet, here.clone())?;
                let body_start = self.out.len();
                self.argument(MathSlot { id, slot: 0 }, alphabet, here)?;
                // The body (slot 0) goes before the label (slot 1).
                self.out[label_start..].rotate_left(body_start - label_start);
                self.depth -= 1;
            }
            "\\xrightarrow" | "\\xleftarrow" | "\\xRightarrow" | "\\xLeftarrow"
            | "\\xleftrightarrow" | "\\xmapsto" => {
                let arrow = match name {
                    "\\xrightarrow" => '→',
                    "\\xleftarrow" => '←',
                    "\\xRightarrow" => '⇒',
                    "\\xLeftarrow" => '⇐',
                    "\\xleftrightarrow" => '↔',
                    _ => '↦',
                };
                self.skip_spaces();
                if self.peek() == Some('[') {
                    // A label below: the arrow and its label above become
                    // the body of an `Under(Set)`.
                    self.bump();
                    let under =
                        self.node(ZWSP, MathKind::Under(OverUnder::Set), Some(parent), start)?;
                    let below_start = self.out.len();
                    self.row(MathSlot { id: under, slot: 1 }, Until::Bracket, alphabet)?;
                    let arrow_start = self.out.len();
                    let id = self.node(
                        arrow,
                        MathKind::Over(OverUnder::Arrow),
                        Some(MathSlot { id: under, slot: 0 }),
                        start,
                    )?;
                    self.argument(MathSlot { id, slot: 1 }, alphabet, here)?;
                    self.depth -= 2;
                    self.out[below_start..].rotate_left(arrow_start - below_start);
                } else {
                    let id =
                        self.node(arrow, MathKind::Over(OverUnder::Arrow), Some(parent), start)?;
                    self.argument(MathSlot { id, slot: 1 }, alphabet, here)?;
                    self.depth -= 1;
                }
            }
            "\\text" | "\\textrm" | "\\mbox" | "\\textit" | "\\textbf" => {
                self.text_argument(parent, name, here)?;
            }
            "\\begin" | "\\end" | "\\\\" | "\\tag" | "\\newcommand" | "\\renewcommand"
            | "\\def" | "\\displaystyle" | "\\textstyle" | "\\scriptstyle" | "\\cr" => {
                return err(here, TexErrorKind::Unsupported);
            }
            "\\" => return err(here, TexErrorKind::MissingArgument),
            _ => return err(here, TexErrorKind::UnknownCommand),
        }
        Ok(plain)
    }

    /// One argument of a command: a group or a single atom.
    fn argument(
        &mut self,
        parent: MathSlot,
        alphabet: Alphabet,
        command: Range<usize>,
    ) -> Result<(), TexError> {
        self.skip_spaces();
        match self.peek() {
            None | Some('}') | Some('$') => err(command, TexErrorKind::MissingArgument),
            _ => self.atom(parent, alphabet).map(|_| ()),
        }
    }

    /// The text of `\text{…}`: upright text characters, spaces kept.
    fn text_argument(
        &mut self,
        parent: MathSlot,
        name: &str,
        command: Range<usize>,
    ) -> Result<(), TexError> {
        self.skip_spaces();
        if self.peek() != Some('{') {
            return err(command, TexErrorKind::MissingArgument);
        }
        self.bump();
        let mut open = 1;
        loop {
            let at = self.pos;
            let Some(c) = self.bump() else {
                return err(at..at, TexErrorKind::UnbalancedBrace);
            };
            match c {
                '{' => open += 1,
                '}' => {
                    open -= 1;
                    if open == 0 {
                        return Ok(());
                    }
                }
                '$' => return err(at..at + 1, TexErrorKind::Unsupported),
                '\\' => {
                    let Some(escaped) = self.bump() else {
                        return err(at..self.pos, TexErrorKind::UnexpectedEnd);
                    };
                    if !"{}$%&#_ \\".contains(escaped) {
                        return err(at..self.pos, TexErrorKind::Unsupported);
                    }
                    self.leaf(RichSegment::plain(escaped), parent);
                }
                '\n' | '\t' => self.leaf(RichSegment::plain(' '), parent),
                c => self.leaf(
                    RichSegment {
                        bold: name == "\\textbf",
                        italic: name == "\\textit",
                        ..RichSegment::plain(c)
                    },
                    parent,
                ),
            }
        }
    }

    /// A delimiter argument of `\left`, `\right`, `\middle` or `\big`.
    fn delimiter(&mut self) -> Result<char, TexError> {
        self.skip_spaces();
        let start = self.pos;
        let Some(c) = self.peek() else {
            return err(start..start, TexErrorKind::MissingArgument);
        };
        let d = if c == '\\' {
            let name = self.command_name();
            match name {
                "\\{" | "\\lbrace" => '{',
                "\\}" | "\\rbrace" => '}',
                "\\|" | "\\Vert" => '‖',
                "\\vert" => '|',
                "\\langle" => '⟨',
                "\\rangle" => '⟩',
                "\\lfloor" => '⌊',
                "\\rfloor" => '⌋',
                "\\lceil" => '⌈',
                "\\rceil" => '⌉',
                "\\backslash" => '\\',
                "\\uparrow" => '↑',
                "\\downarrow" => '↓',
                "\\Uparrow" => '⇑',
                "\\Downarrow" => '⇓',
                "\\lbrack" => '[',
                "\\rbrack" => ']',
                _ => return err(start..self.pos, TexErrorKind::BadDelimiter),
            }
        } else {
            self.bump();
            c
        };
        if !super::math::is_delimiter(d) {
            return err(start..self.pos, TexErrorKind::BadDelimiter);
        }
        Ok(d)
    }

    /// The slot a `^` (`sup`) or `_` script fills on the latest atom `last`,
    /// opening a `Scripts` node around a plain atom; returns the node id and
    /// slot with the node open (`depth` raised by one).
    fn script_slot(
        &mut self,
        last: &mut Last,
        sup: bool,
        parent: MathSlot,
        at: usize,
    ) -> Result<(u32, u8), TexError> {
        let node = last
            .node
            .and_then(|i| self.out[i].math_node.map(|n| (i, n)));
        let slot = match node.map(|(_, n)| n.kind) {
            Some(MathKind::BigOp(_)) => Some(if sup { 1 } else { 0 }),
            Some(MathKind::OpName(_)) => Some(if sup { 2 } else { 1 }),
            Some(MathKind::Over(OverUnder::Brace)) if sup => Some(1),
            Some(MathKind::Under(OverUnder::Brace)) if !sup => Some(1),
            Some(MathKind::Scripts) => Some(if sup { 1 } else { 2 }),
            _ => None,
        };
        let (id, slot) = match (node, slot) {
            (Some((_, n)), Some(slot)) => {
                // The node is open again while its script is written.
                if self.depth == MAX_MATH_DEPTH {
                    return err(at..self.pos, TexErrorKind::TooDeep);
                }
                self.depth += 1;
                (n.id, slot)
            }
            _ => {
                // Wrap the atom in a `Scripts` node: re-parent its top-level
                // segments to slot 0 and move the node before them.
                let start = last.start;
                let id = self.node(ZWSP, MathKind::Scripts, Some(parent), at)?;
                for seg in &mut self.out[start..] {
                    if seg.math_parent == Some(parent) && seg.math_node.map(|n| n.id) != Some(id) {
                        seg.math_parent = Some(MathSlot { id, slot: 0 });
                    }
                }
                self.out[start..].rotate_right(1);
                last.node = Some(start);
                last.slot_start = [None; 3];
                (id, if sup { 1 } else { 2 })
            }
        };
        if last.slot_start[slot as usize].is_some() {
            self.depth -= 1;
            return err(at..self.pos, TexErrorKind::DoubleScript);
        }
        last.slot_start[slot as usize] = Some(self.out.len());
        Ok((id, slot))
    }

    /// A `^` or `_` script on `last`.
    fn script(
        &mut self,
        last: &mut Last,
        sup: bool,
        parent: MathSlot,
        at: usize,
        alphabet: Alphabet,
    ) -> Result<(), TexError> {
        let (id, slot) = self.script_slot(last, sup, parent, at)?;
        let start = self.out.len();
        self.argument(MathSlot { id, slot }, alphabet, at..self.pos)?;
        self.depth -= 1;
        // A slot written after a higher one moves before it.
        let later = last
            .slot_start
            .iter()
            .enumerate()
            .filter(|&(s, p)| s > slot as usize && p.is_some_and(|p| p < start))
            .filter_map(|(_, p)| *p)
            .min();
        if let Some(higher) = later {
            let len = self.out.len() - start;
            self.out[higher..].rotate_left(start - higher);
            for (s, p) in last.slot_start.iter_mut().enumerate() {
                if let Some(p) = p {
                    if s == slot as usize {
                        *p = higher;
                    } else if *p >= higher {
                        *p += len;
                    }
                }
            }
        }
        Ok(())
    }
}

/// A space command's width in em (`None` for the interword space `\ `).
fn space(name: &str) -> Option<Option<f32>> {
    Some(match name {
        "\\," | "\\thinspace" => Some(3.0 / 18.0),
        "\\:" | "\\>" | "\\medspace" => Some(4.0 / 18.0),
        "\\;" | "\\thickspace" => Some(5.0 / 18.0),
        "\\!" | "\\negthinspace" => Some(-3.0 / 18.0),
        "\\quad" => Some(1.0),
        "\\qquad" => Some(2.0),
        "\\ " | "\\~" => None,
        _ => return None,
    })
}

/// A Greek letter command: the letter and whether TeX sets it upright
/// (the capitals).
fn greek(name: &str) -> Option<(char, bool)> {
    let lower = match name {
        "\\alpha" => 'α',
        "\\beta" => 'β',
        "\\gamma" => 'γ',
        "\\delta" => 'δ',
        "\\epsilon" => 'ϵ',
        "\\varepsilon" => 'ε',
        "\\zeta" => 'ζ',
        "\\eta" => 'η',
        "\\theta" => 'θ',
        "\\vartheta" => 'ϑ',
        "\\iota" => 'ι',
        "\\kappa" => 'κ',
        "\\varkappa" => 'ϰ',
        "\\lambda" => 'λ',
        "\\mu" => 'μ',
        "\\nu" => 'ν',
        "\\xi" => 'ξ',
        "\\omicron" => 'ο',
        "\\pi" => 'π',
        "\\varpi" => 'ϖ',
        "\\rho" => 'ρ',
        "\\varrho" => 'ϱ',
        "\\sigma" => 'σ',
        "\\varsigma" => 'ς',
        "\\tau" => 'τ',
        "\\upsilon" => 'υ',
        "\\phi" => 'ϕ',
        "\\varphi" => 'φ',
        "\\chi" => 'χ',
        "\\psi" => 'ψ',
        "\\omega" => 'ω',
        _ => {
            let upper = match name {
                "\\Gamma" => 'Γ',
                "\\Delta" => 'Δ',
                "\\Theta" => 'Θ',
                "\\Lambda" => 'Λ',
                "\\Xi" => 'Ξ',
                "\\Pi" => 'Π',
                "\\Sigma" => 'Σ',
                "\\Upsilon" => 'Υ',
                "\\Phi" => 'Φ',
                "\\Psi" => 'Ψ',
                "\\Omega" => 'Ω',
                _ => return None,
            };
            return Some((upper, true));
        }
    };
    Some((lower, false))
}

/// The upright symbol of an operator, relation, arrow or symbol command.
fn symbol(name: &str) -> Option<char> {
    Some(match name {
        "\\cdot" => '⋅',
        "\\times" => '×',
        "\\div" => '÷',
        "\\pm" => '±',
        "\\mp" => '∓',
        "\\ast" => '∗',
        "\\star" => '⋆',
        "\\circ" => '∘',
        "\\bullet" => '∙',
        "\\oplus" => '⊕',
        "\\ominus" => '⊖',
        "\\otimes" => '⊗',
        "\\odot" => '⊙',
        "\\cup" => '∪',
        "\\cap" => '∩',
        "\\setminus" => '∖',
        "\\wedge" | "\\land" => '∧',
        "\\vee" | "\\lor" => '∨',
        "\\le" | "\\leq" => '≤',
        "\\ge" | "\\geq" => '≥',
        "\\ne" | "\\neq" => '≠',
        "\\approx" => '≈',
        "\\equiv" => '≡',
        "\\sim" => '∼',
        "\\simeq" => '≃',
        "\\cong" => '≅',
        "\\propto" => '∝',
        "\\in" => '∈',
        "\\notin" => '∉',
        "\\ni" => '∋',
        "\\subset" => '⊂',
        "\\supset" => '⊃',
        "\\subseteq" => '⊆',
        "\\supseteq" => '⊇',
        "\\ll" => '≪',
        "\\gg" => '≫',
        "\\perp" => '⊥',
        "\\parallel" => '∥',
        "\\mid" => '∣',
        "\\models" => '⊨',
        "\\vdash" => '⊢',
        "\\to" | "\\rightarrow" => '→',
        "\\gets" | "\\leftarrow" => '←',
        "\\leftrightarrow" => '↔',
        "\\Rightarrow" => '⇒',
        "\\Leftarrow" => '⇐',
        "\\Leftrightarrow" => '⇔',
        "\\implies" => '⟹',
        "\\iff" => '⟺',
        "\\mapsto" => '↦',
        "\\uparrow" => '↑',
        "\\downarrow" => '↓',
        "\\longrightarrow" => '⟶',
        "\\longleftarrow" => '⟵',
        "\\hookrightarrow" => '↪',
        "\\infty" => '∞',
        "\\partial" => '∂',
        "\\nabla" => '∇',
        "\\hbar" => 'ℏ',
        "\\ell" => 'ℓ',
        "\\degree" => '°',
        "\\forall" => '∀',
        "\\exists" => '∃',
        "\\neg" | "\\lnot" => '¬',
        "\\emptyset" | "\\varnothing" => '∅',
        "\\Re" => 'ℜ',
        "\\Im" => 'ℑ',
        "\\aleph" => 'ℵ',
        "\\angle" => '∠',
        "\\triangle" => '△',
        "\\prime" => '′',
        "\\cdots" => '⋯',
        "\\ldots" | "\\dots" => '…',
        "\\vdots" => '⋮',
        "\\ddots" => '⋱',
        "\\langle" => '⟨',
        "\\rangle" => '⟩',
        "\\lbrace" | "\\{" => '{',
        "\\rbrace" | "\\}" => '}',
        "\\|" | "\\Vert" => '‖',
        "\\vert" => '|',
        "\\backslash" => '\\',
        "\\lfloor" => '⌊',
        "\\rfloor" => '⌋',
        "\\lceil" => '⌈',
        "\\rceil" => '⌉',
        "\\#" => '#',
        "\\%" => '%',
        "\\&" => '&',
        "\\_" => '_',
        "\\$" => '$',
        _ => return None,
    })
}

/// A big operator command's symbol.
fn big_operator(name: &str) -> Option<char> {
    Some(match name {
        "\\sum" => '∑',
        "\\prod" => '∏',
        "\\coprod" => '∐',
        "\\int" => '∫',
        "\\iint" => '∬',
        "\\iiint" => '∭',
        "\\oint" => '∮',
        "\\bigcup" => '⋃',
        "\\bigcap" => '⋂',
        "\\bigoplus" => '⨁',
        "\\bigotimes" => '⨂',
        "\\bigodot" => '⨀',
        "\\bigvee" => '⋁',
        "\\bigwedge" => '⋀',
        "\\bigsqcup" => '⨆',
        _ => return None,
    })
}

/// An operator name command's name and where its limits go.
fn operator_name(name: &str) -> Option<(&'static str, Limits)> {
    let with_limits = [
        "lim", "limsup", "liminf", "max", "min", "sup", "inf", "det", "gcd", "Pr", "argmax",
        "argmin",
    ];
    let without = [
        "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh",
        "tanh", "coth", "log", "ln", "lg", "exp", "arg", "deg", "dim", "hom", "ker",
    ];
    let bare = name.strip_prefix('\\')?;
    if let Some(n) = with_limits.iter().find(|n| **n == bare) {
        let shown = match *n {
            "limsup" => "lim sup",
            "liminf" => "lim inf",
            "argmax" => "arg max",
            "argmin" => "arg min",
            other => other,
        };
        return Some((shown, Limits::Auto));
    }
    without
        .iter()
        .find(|n| **n == bare)
        .map(|n| (*n, Limits::NoLimits))
}

/// An accent command's mark and kind.
fn accent(name: &str) -> Option<(char, AccentKind)> {
    Some(match name {
        "\\hat" => ('\u{0302}', AccentKind::Fixed),
        "\\check" => ('\u{030C}', AccentKind::Fixed),
        "\\tilde" => ('\u{0303}', AccentKind::Fixed),
        "\\acute" => ('\u{0301}', AccentKind::Fixed),
        "\\grave" => ('\u{0300}', AccentKind::Fixed),
        "\\dot" => ('\u{0307}', AccentKind::Fixed),
        "\\ddot" => ('\u{0308}', AccentKind::Fixed),
        "\\breve" => ('\u{0306}', AccentKind::Fixed),
        "\\bar" => ('\u{0304}', AccentKind::Fixed),
        "\\vec" => ('\u{20D7}', AccentKind::Fixed),
        "\\mathring" => ('\u{030A}', AccentKind::Fixed),
        "\\widehat" => ('\u{0302}', AccentKind::Wide),
        "\\widetilde" => ('\u{0303}', AccentKind::Wide),
        "\\widecheck" => ('\u{030C}', AccentKind::Wide),
        "\\overrightarrow" => ('\u{20D7}', AccentKind::Wide),
        "\\overleftarrow" => ('\u{20D6}', AccentKind::Wide),
        "\\overleftrightarrow" => ('\u{20E1}', AccentKind::Wide),
        _ => return None,
    })
}

/// A `\big`-family command's level and class.
fn sized(name: &str) -> Option<(u8, MathClass)> {
    let bare = name.strip_prefix('\\')?;
    let (stem, class) = match bare.as_bytes().last()? {
        b'l' => (&bare[..bare.len() - 1], MathClass::Opening),
        b'r' => (&bare[..bare.len() - 1], MathClass::Closing),
        b'm' => (&bare[..bare.len() - 1], MathClass::Relation),
        _ => (bare, MathClass::Normal),
    };
    let level = match stem {
        "big" => 1,
        "Big" => 2,
        "bigg" => 3,
        "Bigg" => 4,
        _ => return None,
    };
    Some((level, class))
}

/// A font command's alphabet.
fn font(name: &str) -> Option<Alphabet> {
    Some(match name {
        "\\mathrm" => Alphabet::Roman,
        "\\mathbf" => Alphabet::Bold,
        "\\mathit" => Alphabet::Italic,
        "\\mathcal" => Alphabet::Calligraphic,
        "\\mathbb" => Alphabet::Blackboard,
        _ => return None,
    })
}

/// The Mathematical Script capital of `c` (`\mathcal`), with the holes in
/// the Letterlike Symbols block.
fn calligraphic(c: char) -> char {
    match c {
        'B' => 'ℬ',
        'E' => 'ℰ',
        'F' => 'ℱ',
        'H' => 'ℋ',
        'I' => 'ℐ',
        'L' => 'ℒ',
        'M' => 'ℳ',
        'R' => 'ℛ',
        'A'..='Z' => char::from_u32(0x1D49C + (c as u32 - 'A' as u32)).unwrap_or(c),
        _ => c,
    }
}

/// The Mathematical Double-Struck character of `c` (`\mathbb`), with the
/// holes in the Letterlike Symbols block.
fn blackboard(c: char) -> char {
    match c {
        'C' => 'ℂ',
        'H' => 'ℍ',
        'N' => 'ℕ',
        'P' => 'ℙ',
        'Q' => 'ℚ',
        'R' => 'ℝ',
        'Z' => 'ℤ',
        'A'..='Z' => char::from_u32(0x1D538 + (c as u32 - 'A' as u32)).unwrap_or(c),
        'a'..='z' => char::from_u32(0x1D552 + (c as u32 - 'a' as u32)).unwrap_or(c),
        '0'..='9' => char::from_u32(0x1D7D8 + (c as u32 - '0' as u32)).unwrap_or(c),
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::MathErrorKind;

    fn parse(tex: &str) -> Vec<RichSegment> {
        rich_segments_from_tex(tex).unwrap_or_else(|e| panic!("{tex}: {e}"))
    }

    fn kind_of(tex: &str) -> TexErrorKind {
        rich_segments_from_tex(tex).expect_err(tex).kind
    }

    /// Each segment as (text, node kind and id, parent id and slot).
    fn shape(segs: &[RichSegment]) -> Vec<(char, Option<(u32, MathKind)>, Option<(u32, u8)>)> {
        segs.iter()
            .map(|s| {
                (
                    s.text,
                    s.math_node.map(|n| (n.id, n.kind)),
                    s.math_parent.map(|p| (p.id, p.slot)),
                )
            })
            .collect()
    }

    #[test]
    fn letters_are_italic_math_characters_not_math_italic_code_points() {
        let s = parse("$x$");
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].text, 'x');
        assert!(s[1].italic && !s[1].bold);
        assert_eq!(s[1].char_set, CharSet::Math);
        let s = parse(r"$\mathbf{x}\mathrm{d}2\Gamma\alpha$");
        let flags: Vec<_> = s[1..].iter().map(|s| (s.text, s.bold, s.italic)).collect();
        assert_eq!(
            flags,
            [
                ('x', true, false),
                ('d', false, false),
                ('2', false, false),
                ('Γ', false, false),
                ('α', false, true)
            ]
        );
        // A minus is the minus sign.
        assert_eq!(parse("$-$")[1].text, '−');
    }

    #[test]
    fn scripts_wrap_their_base_and_keep_slot_order() {
        const Z: char = ZWSP;
        let x2 = parse("$x^2$");
        assert_eq!(
            shape(&x2),
            [
                (Z, Some((0, MathKind::Math(MathStyle::Text))), None),
                (Z, Some((1, MathKind::Scripts)), Some((0, 0))),
                ('x', None, Some((1, 0))),
                ('2', None, Some((1, 1))),
            ]
        );
        // Both orders give one tree: base, superscript, subscript.
        assert_eq!(shape(&parse("$x_i^2$")), shape(&parse("$x^2_i$")));
        assert_eq!(
            shape(&parse("$x_i^2$"))[2..],
            [
                ('x', None, Some((1, 0))),
                ('2', None, Some((1, 1))),
                ('i', None, Some((1, 2)))
            ]
        );
        // A group base is wrapped whole; nested scripts nest.
        let s = parse("${(a+b)}^{n^2}$");
        assert_eq!(s[1].math_node.map(|n| n.kind), Some(MathKind::Scripts));
        assert!(
            s[2..7]
                .iter()
                .all(|s| s.math_parent == Some(MathSlot { id: 1, slot: 0 }))
        );
        assert_eq!(kind_of("$x^2^3$"), TexErrorKind::DoubleScript);
        assert_eq!(kind_of("$x_1_2$"), TexErrorKind::DoubleScript);
        // Primes are a superscript.
        let s = parse("$f''$");
        assert_eq!(s[3].text, '″');
        assert_eq!(s[3].math_parent, Some(MathSlot { id: 1, slot: 1 }));
    }

    #[test]
    fn operators_take_their_limits_as_slots() {
        let s = parse(r"$\sum_{i=1}^{n} x$");
        assert_eq!(
            s[1].math_node.map(|n| n.kind),
            Some(MathKind::BigOp(Limits::Auto))
        );
        assert_eq!(s[1].text, '∑');
        let slots: Vec<_> = s[2..7]
            .iter()
            .map(|s| (s.text, s.math_parent.unwrap().slot))
            .collect();
        assert_eq!(slots, [('i', 0), ('=', 0), ('1', 0), ('n', 1), ('x', 0)]);
        // The upper limit written first still goes after the lower one.
        assert_eq!(shape(&parse(r"$\int^b_a$")), shape(&parse(r"$\int_a^b$")));
        let s = parse(r"$\sum\limits_i \int\nolimits_0$");
        assert_eq!(
            s[1].math_node.map(|n| n.kind),
            Some(MathKind::BigOp(Limits::Limits))
        );
        assert_eq!(
            s[3].math_node.map(|n| n.kind),
            Some(MathKind::BigOp(Limits::NoLimits))
        );
        let s = parse(r"$\lim_{x\to 0} \sin x$");
        assert_eq!(
            s[1].math_node.map(|n| n.kind),
            Some(MathKind::OpName(Limits::Auto))
        );
        let name: String = s
            .iter()
            .filter(|s| s.math_parent == Some(MathSlot { id: 1, slot: 0 }))
            .map(|s| s.text)
            .collect();
        assert_eq!(name, "lim");
        assert!(
            s.iter()
                .any(|s| s.text == '→' && s.math_parent == Some(MathSlot { id: 1, slot: 1 }))
        );
        let sin = s
            .iter()
            .position(|s| {
                s.math_node
                    .is_some_and(|n| n.kind == MathKind::OpName(Limits::NoLimits))
            })
            .unwrap();
        assert_eq!(s[sin + 1].text, 's');
        assert!(!s[sin + 1].italic);
        assert_eq!(kind_of(r"$x\limits$"), TexErrorKind::Unsupported);
    }

    #[test]
    fn arguments_written_before_their_body_are_reordered() {
        // `\sqrt[3]{x}`: the radicand (slot 0) before the index (slot 1).
        let s = parse(r"$\sqrt[3]{x}$");
        assert_eq!(s[1].math_node.map(|n| n.kind), Some(MathKind::Sqrt));
        assert_eq!((s[2].text, s[2].math_parent.unwrap().slot), ('x', 0));
        assert_eq!((s[3].text, s[3].math_parent.unwrap().slot), ('3', 1));
        // `\overset{?}{=}`: the body before the label.
        let s = parse(r"$\overset{?}{=}$");
        assert_eq!(
            s[1].math_node.map(|n| n.kind),
            Some(MathKind::Over(OverUnder::Set))
        );
        assert_eq!((s[2].text, s[2].math_parent.unwrap().slot), ('=', 0));
        assert_eq!((s[3].text, s[3].math_parent.unwrap().slot), ('?', 1));
        // `\xleftarrow[g]{h}`: the arrow (with h over it) in the body, g under.
        let s = parse(r"$\xleftarrow[g]{h}$");
        assert_eq!(
            s[1].math_node.map(|n| n.kind),
            Some(MathKind::Under(OverUnder::Set))
        );
        assert_eq!(
            s[2].math_node.map(|n| n.kind),
            Some(MathKind::Over(OverUnder::Arrow))
        );
        assert_eq!(
            (s[3].text, s[3].math_parent.unwrap()),
            ('h', MathSlot { id: 2, slot: 1 })
        );
        assert_eq!(
            (s[4].text, s[4].math_parent.unwrap()),
            ('g', MathSlot { id: 1, slot: 1 })
        );
    }

    #[test]
    fn delimiters_pair_and_size() {
        let s = parse(r"$\left( x \middle| y \right]$");
        assert_eq!(
            s[1].math_node.map(|n| n.kind),
            Some(MathKind::Delim {
                open: '(',
                close: ']'
            })
        );
        assert_eq!(s[3].math_node.map(|n| n.kind), Some(MathKind::Middle));
        assert_eq!(s[3].text, '|');
        let s = parse(r"$\left. x \right\rangle$");
        assert_eq!(
            s[1].math_node.map(|n| n.kind),
            Some(MathKind::Delim {
                open: '.',
                close: '⟩'
            })
        );
        let s = parse(r"$\big( \Bigl[ \biggr\} \Biggm|$");
        let sized: Vec<_> = s[1..]
            .iter()
            .map(|s| (s.text, s.math_node.unwrap().kind))
            .collect();
        assert_eq!(
            sized,
            [
                (
                    '(',
                    MathKind::Sized {
                        level: 1,
                        class: MathClass::Normal
                    }
                ),
                (
                    '[',
                    MathKind::Sized {
                        level: 2,
                        class: MathClass::Opening
                    }
                ),
                (
                    '}',
                    MathKind::Sized {
                        level: 3,
                        class: MathClass::Closing
                    }
                ),
                (
                    '|',
                    MathKind::Sized {
                        level: 4,
                        class: MathClass::Relation
                    }
                ),
            ]
        );
        assert_eq!(kind_of(r"$\left( x$"), TexErrorKind::UnpairedLeftRight);
        assert_eq!(kind_of(r"$x \right)$"), TexErrorKind::UnpairedLeftRight);
        assert_eq!(kind_of(r"$x \middle|$"), TexErrorKind::UnpairedLeftRight);
        assert_eq!(kind_of(r"$\left x \right)$"), TexErrorKind::BadDelimiter);
        assert_eq!(kind_of(r"$\big\alpha$"), TexErrorKind::BadDelimiter);
    }

    #[test]
    fn text_spaces_and_spans() {
        let s = parse(r"speed $= 3\,\text{m/s}$ \$5");
        let plain: String = s[..6].iter().map(|s| s.text).collect();
        assert_eq!(plain, "speed ");
        let m = s.iter().position(|s| s.math_node.is_some()).unwrap();
        let space = s.iter().find(|s| s.field_em.is_some()).unwrap();
        assert_eq!(space.text, ' ');
        assert!((space.field_em.unwrap() - 3.0 / 18.0).abs() < 1e-6);
        let unit: String = s[m..]
            .iter()
            .filter(|s| {
                s.char_set == CharSet::Text && s.math_parent.is_some() && s.field_em.is_none()
            })
            .map(|s| s.text)
            .collect();
        assert_eq!(unit, "m/s");
        assert!(s.ends_with(&[
            RichSegment::plain(' '),
            RichSegment::plain('$'),
            RichSegment::plain('5')
        ]));
        // `\(…\)` spans too; two spans in a cell number on.
        let s = parse(r"\(a\) and \(\sqrt b\)");
        let ids: Vec<u32> = s.iter().filter_map(|s| s.math_node.map(|n| n.id)).collect();
        assert_eq!(ids, [0, 1, 2]);
        // `\!` is a negative space, valid inside math.
        let rt = RichText::from_tex(r"$a\!b$", Color::BLACK, 12.0, "").unwrap();
        assert!(rt.segments.iter().any(|s| s.field_em == Some(-3.0 / 18.0)));
        assert!(rt.within_limits());
        // Text outside math maps Unicode scripts as `RichText::plain` does.
        assert_eq!(parse("m²"), crate::text::rich_segments_from_text("m²"));
    }

    #[test]
    fn errors_name_their_bytes() {
        let e = rich_segments_from_tex(r"ab $x + \foo y$").unwrap_err();
        assert_eq!(
            (e.kind, e.at.clone()),
            (TexErrorKind::UnknownCommand, 8..12)
        );
        assert_eq!(&r"ab $x + \foo y$"[e.at.clone()], r"\foo");
        assert!(e.to_string().contains("8..12"), "{e}");
        assert_eq!(kind_of("$x}$"), TexErrorKind::UnbalancedBrace);
        assert_eq!(kind_of("${x$"), TexErrorKind::UnbalancedBrace);
        assert_eq!(kind_of(r"$\frac{a}$"), TexErrorKind::MissingArgument);
        assert_eq!(kind_of("$x^$"), TexErrorKind::MissingArgument);
        assert_eq!(kind_of("$x"), TexErrorKind::UnexpectedEnd);
        assert_eq!(kind_of(r"\(x"), TexErrorKind::UnexpectedEnd);
        for unsupported in [
            r"$\begin{matrix} a \end{matrix}$",
            r"$a \\ b$",
            r"$a & b$",
            r"$\tag{1}$",
            r"$\newcommand{\x}{y}$",
            r"$x \displaystyle y$",
            r"$\frac{\displaystyle a}{b}$",
            "$$x$ y$$",
        ] {
            assert_eq!(
                kind_of(unsupported),
                TexErrorKind::Unsupported,
                "{unsupported}"
            );
        }
        // Every kind has a reason of its own.
        let kinds = [
            TexErrorKind::UnknownCommand,
            TexErrorKind::UnbalancedBrace,
            TexErrorKind::MissingArgument,
            TexErrorKind::UnexpectedEnd,
            TexErrorKind::DoubleScript,
            TexErrorKind::UnpairedLeftRight,
            TexErrorKind::Unsupported,
            TexErrorKind::TooDeep,
            TexErrorKind::BadDelimiter,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                assert_ne!(a.as_str(), b.as_str());
            }
        }
    }

    // `$$…$$` and a leading `\displaystyle` start a formula in display
    // style; `\textstyle` returns to text; the style is the root's.
    #[test]
    fn display_spans_set_the_root_style() {
        let root = |tex: &str| parse(tex)[0].math_node.map(|n| n.kind);
        let display = Some(MathKind::Math(MathStyle::Display));
        let text = Some(MathKind::Math(MathStyle::Text));
        assert_eq!(root(r"$$\sum_i x_i$$"), display);
        assert_eq!(root(r"$\displaystyle \sum_i x_i$"), display);
        assert_eq!(root(r"\(\displaystyle x\)"), display);
        assert_eq!(root(r"$\displaystyle\textstyle x$"), text);
        assert_eq!(root(r"$$\textstyle x$$"), text);
        assert_eq!(root("$x$"), text);
        // The same body either way, after the root.
        assert_eq!(parse("$$x+1$$")[1..], parse("$x+1$")[1..]);
        // `\displaystylex` is not the command.
        assert_eq!(kind_of(r"$\displaystylex$"), TexErrorKind::UnknownCommand);
        assert!(TexErrorKind::Unsupported.as_str().contains("leading"));
        assert_eq!(kind_of("$$x"), TexErrorKind::UnexpectedEnd);
    }

    // 16 nested nodes (the Math root and 15 radicals) convert; a 17th is
    // too deep, at parse time and in validation.
    #[test]
    fn nesting_stops_at_the_depth_limit() {
        let nest = |n: usize| format!("${}x{}$", r"\sqrt{".repeat(n), "}".repeat(n));
        let s = parse(&nest(MAX_MATH_DEPTH - 1));
        assert_eq!(
            s.iter().filter(|s| s.math_node.is_some()).count(),
            MAX_MATH_DEPTH
        );
        let e = rich_segments_from_tex(&nest(MAX_MATH_DEPTH)).unwrap_err();
        assert_eq!(e.kind, TexErrorKind::TooDeep);
        // Hand-built, the same 17 levels fail validation.
        let mut segments = s.clone();
        let last = segments
            .iter()
            .rposition(|s| s.math_node.is_some())
            .unwrap();
        let id = segments[last].math_node.unwrap().id;
        segments.insert(
            last + 1,
            RichSegment {
                math_node: Some(MathNode {
                    id: id + 1,
                    kind: MathKind::Sqrt,
                }),
                math_parent: Some(MathSlot { id, slot: 0 }),
                ..RichSegment::plain('√')
            },
        );
        let rt = RichText {
            segments,
            ..RichText::plain("", Color::BLACK, 12.0, "")
        };
        assert_eq!(rt.validate_math().unwrap_err().kind, MathErrorKind::TooDeep);
    }

    /// A formula of every supported command family.
    pub(crate) const CORPUS: &[&str] = &[
        r"$\frac{a}{b} \dfrac{1}{1+\frac{1}{x}} \tfrac12 \binom{n}{k} \dbinom{a}{b}$",
        r"$\sqrt{x^2+y^2} \sqrt[3]{x}$",
        r"$\sum_{i=1}^{n} i \prod_k \coprod \int_0^\infty \iint \iiint \oint \bigcup_i \bigcap_j \bigoplus \bigotimes \bigvee \bigwedge$",
        r"$\lim_{x\to 0} \max_x \min \sup \inf \det \gcd \sin \cos \tan \log \ln \exp \operatorname{tr} \operatorname*{argmax}_x$",
        r"$\left( \frac{a}{b} \middle| c \right) \left[ x \right] \left\{ y \right\} \left\langle z \right\rangle \left\lfloor w \right\rceil \left. v \right|$",
        r"$\big( \Big[ \bigg\{ \Bigg| \bigl( \bigr) \bigm|$",
        r"$\hat{x} \bar{y} \vec{v} \dot{q} \ddot{r} \tilde{z} \check{c} \breve{b} \acute{a} \grave{g} \widehat{abc} \widetilde{xyz} \overrightarrow{AB} \overleftarrow{CD}$",
        r"$\overline{AB} \underline{cd} \overbrace{a+b}^{n} \underbrace{x\cdots x}_{k} \overset{?}{=} \underset{x}{\to} \stackrel{\text{def}}{=} \xrightarrow{f} \xleftarrow[g]{h}$",
        r"$\alpha\beta\gamma\delta\epsilon\varepsilon\zeta\eta\theta\vartheta\iota\kappa\lambda\mu\nu\xi\pi\varpi\rho\varrho\sigma\varsigma\tau\upsilon\phi\varphi\chi\psi\omega \Gamma\Delta\Theta\Lambda\Xi\Pi\Sigma\Upsilon\Phi\Psi\Omega$",
        r"$\text{speed} = 3\,\text{m/s} \mathrm{d}x \mathbf{A} \mathit{if} \mathcal{L} \mathbb{R}$",
        r"$a\,b\:c\;d\!e\ f\quad g\qquad h$",
        r"$\pm\mp\times\div\cdot\ast\star\circ\bullet\oplus\otimes\cup\cap\setminus\wedge\vee \le\ge\ne\approx\equiv\sim\simeq\cong\propto\in\notin\subset\supset\subseteq\supseteq\ll\gg\perp\parallel\mid$",
        r"$\to\gets\leftrightarrow\Rightarrow\Leftarrow\Leftrightarrow\implies\iff\mapsto\infty\partial\nabla\hbar\ell\forall\exists\neg\emptyset\cdots\ldots\vdots\ddots\{\}\|\#\%\&\_\$$",
        r"$x^{y^{z}} a_{n+1} f'(x) g''(x) e^{i\pi} {}_2F_1 x^2_i$",
    ];

    // `from_tex` output always validates, the self-check of the parser.
    #[test]
    fn every_conversion_validates() {
        for tex in CORPUS {
            let rt = RichText::from_tex(tex, Color::BLACK, 14.0, "")
                .unwrap_or_else(|e| panic!("{tex}: {e}"));
            assert_eq!(rt.validate_math(), Ok(()), "{tex}");
            assert!(rt.segments.iter().all(|s| s.math_node.is_some()
                || s.math_parent.is_some()
                || !matches!(s.text, '\u{200B}')));
        }
    }

    // Converted texts round-trip through JSON byte for byte.
    #[cfg(feature = "serde")]
    #[test]
    fn conversions_round_trip_through_json() {
        for tex in CORPUS {
            let rt = RichText::from_tex(tex, Color::BLACK, 14.0, "").unwrap();
            let json = serde_json::to_string(&rt).unwrap();
            let back: RichText = serde_json::from_str(&json).unwrap();
            assert_eq!(back, rt, "{tex}");
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
    }
}
