//! The math layout of #45: a formula of a cell, a `Math` node and its
//! subtree, laid out from the math face's MATH table ([`MathFace`]) and
//! placed into the cell's [`GlyphSink`] beside the text around it.
//!
//! The layout is one walk over the flat segment list. A row of atoms is
//! placed left to right with TeX's inter-atom spacing; a node measures its
//! slots first (the measuring sink, through the same code), places its own
//! pieces from those boxes, and then places its slots at their positions.
//! Measuring skips the second pass, so measuring costs one walk and drawing
//! one walk per nesting level; nothing is stored. Every box is the ink
//! extent of what it places, so a formula's measured envelope holds its
//! pixels. Baseline shifts are whole pixels, so every glyph of a formula
//! snaps to the pixel grid as the text around it does.
//!
//! The rules are MathML Core's with TeX's where it is silent: scripts by
//! `SuperscriptShiftUp`, `SubscriptShiftDown`, the baseline drops, the gap
//! minima and the italic correction; fractions and stacks by their shift
//! and gap constants with the bar on the math axis; radicals by
//! `RadicalVerticalGap`, the rule thickness and the degree kerns; big
//! operators by `DisplayOperatorMinHeight` and the limit constants;
//! delimiters stretched to `2·max(ascent − axis, descent + axis)`; accents
//! by their attachment points and `AccentBaseHeight`; over and under
//! constructions by the overbar, underbar and stretch-stack constants. A
//! formula starts in its root's style, text by default (D8) or display
//! (`$$…$$`), and script styles take the face's
//! `ssty` alternates (D5). Math glyphs come from the math face whatever the
//! font policy (D6); text inside math (`CharSet::Text`) keeps the policy's
//! chain.

use swash::FontRef;

use super::math_font::{MathFace, Stretched};
use super::{FontPolicy, GlyphSink, MeasureSink, glyph_extents_id, glyph_font};
use crate::color::Color;
use crate::layout::RectF;
use crate::text::math::subtree_end;
use crate::text::{
    AccentKind, CharSet, FracStyle, Limits, MathClass, MathKind, MathStyle, OverUnder, RichSegment,
    RichText, greek_char, math_alphanumeric, math_class,
};

/// TeX's atom classes, which decide the space between neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Atom {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

/// TeX's inter-atom spacing in mu (1/18 em of the style's size), rows the
/// left atom, columns the right one, in the order of [`Atom`]: 3 thin, 4
/// medium, 5 thick; negative entries apply in display and text style only.
const SPACING: [[i8; 8]; 8] = [
    [0, 3, -4, -5, 0, 0, 0, -3],
    [3, 3, 0, -5, 0, 0, 0, -3],
    [-4, -4, 0, 0, -4, 0, 0, -4],
    [-5, -5, 0, 0, -5, 0, 0, -5],
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 3, -4, -5, 0, 0, 0, -3],
    [-3, -3, 0, -3, -3, -3, -3, -3],
    [-3, 3, -4, -5, -3, 0, -3, -3],
];

/// The width of TeX's null delimiter (`\left.`, the space beside a
/// fraction), in em.
const NULL_DELIMITER_EM: f32 = 0.12;
/// The extra width of an extensible arrow past its label (`\xrightarrow`),
/// in em.
const ARROW_PADDING_EM: f32 = 0.6;
/// The height of `\big`, `\Big`, `\bigg`, `\Bigg`, in em.
const SIZED_EM: [f32; 4] = [0.85, 1.15, 1.45, 1.75];

/// A math style: display (0), text (1), script (2), scriptscript (3), and
/// whether it is cramped (superscripts lower).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Style {
    level: u8,
    cramped: bool,
}

impl Style {
    const DISPLAY: Style = Style {
        level: 0,
        cramped: false,
    };
    const TEXT: Style = Style {
        level: 1,
        cramped: false,
    };

    fn display(self) -> bool {
        self.level == 0
    }

    /// Script and scriptscript: no medium or thick spaces, smaller glyphs.
    fn small(self) -> bool {
        self.level >= 2
    }

    fn cramp(self) -> Style {
        Style {
            cramped: true,
            ..self
        }
    }

    /// The style of a superscript or an upper limit.
    fn sup(self) -> Style {
        Style {
            level: if self.level < 2 { 2 } else { 3 },
            ..self
        }
    }

    /// The style of a subscript or a lower limit.
    fn sub(self) -> Style {
        self.sup().cramp()
    }

    /// The style of a fraction's numerator.
    fn num(self) -> Style {
        Style {
            level: (self.level + 1).min(3),
            ..self
        }
    }

    /// The style of a fraction's denominator.
    fn den(self) -> Style {
        self.num().cramp()
    }
}

/// A laid-out box, relative to its pen (left, baseline; y down): its advance,
/// its ink extent above and below the baseline and either side of the pen,
/// and what an enclosing node reads of a single glyph.
#[derive(Clone, Copy, Debug)]
struct MBox {
    width: f32,
    ascent: f32,
    descent: f32,
    left: f32,
    right: f32,
    /// Whether the box has ink; the four extents mean nothing otherwise.
    ink: bool,
    /// The italic correction of its last glyph, in pixels.
    italic: f32,
    /// The top accent attachment of a single math-face glyph, from the pen.
    accent: Option<f32>,
    /// The math-face glyph a single-glyph box is.
    glyph: Option<u16>,
}

impl MBox {
    fn empty(width: f32) -> MBox {
        MBox {
            width,
            ascent: 0.0,
            descent: 0.0,
            left: 0.0,
            right: 0.0,
            ink: false,
            italic: 0.0,
            accent: None,
            glyph: None,
        }
    }

    /// Union `other`, placed with its pen at (`dx`, `dy`) from this pen.
    fn add(&mut self, other: &MBox, dx: f32, dy: f32) {
        if !other.ink {
            return;
        }
        self.ink_rect(
            dx + other.left,
            dy - other.ascent,
            dx + other.right,
            dy + other.descent,
        );
    }

    /// Union the ink rectangle `x0..x1` × `y0..y1` (y down from the
    /// baseline).
    fn ink_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        if self.ink {
            self.left = self.left.min(x0);
            self.right = self.right.max(x1);
            self.ascent = self.ascent.max(-y0);
            self.descent = self.descent.max(y1);
        } else {
            self.ink = true;
            self.left = x0;
            self.right = x1;
            self.ascent = -y0;
            self.descent = y1;
        }
    }
}

/// A placed glyph or assembly: the shape's advance and ink relative to its
/// pen. A vertical assembly's pen is at its bottom; a horizontal one's at
/// its left end, on the glyph baseline.
#[derive(Clone, Copy)]
struct Shape {
    stretched: Stretched,
    horizontal: bool,
    size: f32,
    ink: MBox,
}

/// One cell's formulas: the text, the cell's segments, the math face and the
/// font policy of `\text{}`.
pub(super) struct Formula<'a> {
    rt: &'a RichText,
    segs: &'a [RichSegment],
    face: MathFace,
    policy: FontPolicy,
}

/// The extents [`place_formula`] reports to the cell.
pub(super) struct FormulaExtents {
    pub(super) width: f32,
    pub(super) ascent: f32,
    pub(super) descent: f32,
}

/// Lay out the `Math` node at `start` of the cell `segs` (its subtree ends
/// at `end`) with its pen at `pen` (left, baseline), placing it into
/// `sink` when it paints. The box starts at the formula's leftmost ink, so
/// nothing is drawn left of the pen or past the reported width.
pub(super) fn place_formula(
    sink: &mut dyn GlyphSink,
    rt: &RichText,
    segs: &[RichSegment],
    (start, end): (usize, usize),
    pen: (f32, f32),
    policy: FontPolicy,
) -> FormulaExtents {
    let f = Formula {
        rt,
        segs,
        face: MathFace::for_family(&rt.font),
        policy,
    };
    let base = super::segment_base_size(&segs[start], rt);
    let [body, _, _] = f.slots(start, end);
    // The root's style is the formula's starting style (D8: text).
    let style = match segs[start].math_node.map(|n| n.kind) {
        Some(MathKind::Math(MathStyle::Display)) => Style::DISPLAY,
        _ => Style::TEXT,
    };
    let m = f.row(body, style, base, &mut MeasureSink, (0.0, 0.0), None);
    if !m.ink {
        return FormulaExtents {
            width: m.width,
            ascent: 0.0,
            descent: 0.0,
        };
    }
    let shift = (-m.left).max(0.0);
    if sink.paints() {
        f.row(body, style, base, sink, (pen.0 + shift, pen.1), None);
    }
    FormulaExtents {
        width: m.width.max(m.right) + shift,
        ascent: m.ascent,
        descent: m.descent,
    }
}

/// The face and glyph of a `CharSet::Math` character outside math nodes:
/// the math face's glyph of its mathematical alphanumeric (or of the
/// character itself), `None` when the math face lacks both.
pub(super) fn math_text_glyph(
    rt: &RichText,
    seg: &RichSegment,
    ch: char,
) -> Option<(FontRef<'static>, u16)> {
    let face = MathFace::for_family(&rt.font);
    let mapped = face.glyph(math_alphanumeric(ch, seg.bold, seg.italic));
    let glyph = if mapped != 0 { mapped } else { face.glyph(ch) };
    (glyph != 0).then_some((face.font, glyph))
}

/// Whether `c` is an integral sign, whose limits go beside it by default.
fn is_integral(c: char) -> bool {
    matches!(c, '∫'..='∳' | '⨋'..='⨜')
}

/// The TeX class of a math character.
fn char_atom(c: char) -> Atom {
    match c {
        '/' | '.' => Atom::Ord,
        ':' => Atom::Rel,
        '!' | '?' => Atom::Close,
        ',' | ';' => Atom::Punct,
        _ => match math_class(c) {
            MathClass::Binary | MathClass::Vary => Atom::Bin,
            MathClass::Relation => Atom::Rel,
            MathClass::Opening => Atom::Open,
            MathClass::Closing => Atom::Close,
            MathClass::Large => Atom::Op,
            _ => Atom::Ord,
        },
    }
}

impl Formula<'_> {
    /// The glyph size of style `style` over base size `base`.
    fn size(&self, style: Style, base: f32) -> f32 {
        let c = self.face.constants();
        base * match style.level {
            0 | 1 => 1.0,
            2 => c.script_percent_scale_down() as f32 / 100.0,
            _ => c.script_script_percent_scale_down() as f32 / 100.0,
        }
    }

    /// The `[start, end)` range of each slot of the node at `node` (subtree
    /// end `end`); an empty slot is an empty range.
    fn slots(&self, node: usize, end: usize) -> [(usize, usize); 3] {
        let mut ranges = [(end, end); 3];
        let mut j = node + 1;
        while j < end {
            let e = subtree_end(self.segs, j).min(end);
            let slot = self.segs[j].math_parent.map_or(0, |p| p.slot as usize);
            if let Some(range) = ranges.get_mut(slot) {
                if range.0 == end {
                    range.0 = j;
                }
                range.1 = e;
            }
            j = e;
        }
        ranges
    }

    /// The class of the atom at `j` (subtree end `end`), `None` for a space.
    fn atom_class(&self, j: usize, end: usize) -> Option<Atom> {
        let seg = &self.segs[j];
        let Some(node) = seg.math_node else {
            if seg.text.is_whitespace() {
                return None;
            }
            return Some(match seg.char_set {
                CharSet::Text | CharSet::Greek => Atom::Ord,
                CharSet::Math => char_atom(seg.text),
            });
        };
        Some(match node.kind {
            MathKind::Frac(_) | MathKind::Binom(_) | MathKind::Delim { .. } => Atom::Inner,
            MathKind::BigOp(_) | MathKind::OpName(_) => Atom::Op,
            MathKind::Sized { class, .. } => match class {
                MathClass::Opening => Atom::Open,
                MathClass::Closing => Atom::Close,
                MathClass::Relation => Atom::Rel,
                _ => Atom::Ord,
            },
            MathKind::Scripts => self.single_class(j, end, 0).unwrap_or(Atom::Ord),
            MathKind::Over(OverUnder::Set) | MathKind::Under(OverUnder::Set) => {
                self.single_class(j, end, 0).unwrap_or(Atom::Ord)
            }
            MathKind::Over(OverUnder::Arrow) | MathKind::Under(OverUnder::Arrow) => {
                let [body, _, _] = self.slots(j, end);
                if body.0 == body.1 {
                    Atom::Rel
                } else {
                    Atom::Ord
                }
            }
            _ => Atom::Ord,
        })
    }

    /// The class of slot `slot` of the node at `j` when it is one atom.
    fn single_class(&self, j: usize, end: usize, slot: usize) -> Option<Atom> {
        let range = self.slots(j, end)[slot];
        if range.0 == range.1 {
            return None;
        }
        let e = subtree_end(self.segs, range.0).min(range.1);
        if e != range.1 {
            return None;
        }
        self.atom_class(range.0, e)
    }

    /// The class of the first atom from `j` that is not a space.
    fn next_class(&self, mut j: usize, end: usize) -> Option<Atom> {
        while j < end {
            let e = subtree_end(self.segs, j).min(end);
            if let Some(class) = self.atom_class(j, e) {
                return Some(class);
            }
            j = e;
        }
        None
    }

    /// The space between a `left` and a `right` atom in `style`.
    fn space(&self, left: Atom, right: Atom, style: Style, base: f32) -> f32 {
        let mu = SPACING[left as usize][right as usize];
        if mu == 0 || (mu < 0 && style.small()) {
            return 0.0;
        }
        f32::from(mu.abs()) * self.size(style, base) / 18.0
    }

    /// Lay out the atoms of `range` left to right from `pen`. `middle` is the
    /// size `Middle` delimiters stretch to (the enclosing `Delim`'s).
    fn row(
        &self,
        (start, end): (usize, usize),
        style: Style,
        base: f32,
        sink: &mut dyn GlyphSink,
        pen: (f32, f32),
        middle: Option<f32>,
    ) -> MBox {
        let mut out = MBox::empty(0.0);
        let mut x = 0.0f32;
        let mut prev: Option<Atom> = None;
        let mut atoms = 0;
        let mut last = MBox::empty(0.0);
        let mut j = start;
        while j < end {
            let e = subtree_end(self.segs, j).min(end);
            let class = self.atom_class(j, e);
            if let Some(mut class) = class {
                // Bin demotion: a binary operator needs an operand on both
                // sides.
                if class == Atom::Bin
                    && (matches!(
                        prev,
                        None | Some(Atom::Bin | Atom::Op | Atom::Rel | Atom::Open | Atom::Punct)
                    ) || matches!(
                        self.next_class(e, end),
                        None | Some(Atom::Rel | Atom::Close | Atom::Punct)
                    ))
                {
                    class = Atom::Ord;
                }
                if let Some(p) = prev {
                    x += self.space(p, class, style, base);
                }
                prev = Some(class);
            }
            let b = self.atom(j, e, style, base, sink, (pen.0 + x, pen.1), middle);
            out.add(&b, x, 0.0);
            x += b.width;
            atoms += 1;
            last = b;
            j = e;
        }
        out.width = x;
        out.italic = last.italic;
        if atoms == 1 {
            out.accent = last.accent;
            out.glyph = last.glyph;
        }
        out
    }

    /// Lay out one atom: a leaf segment or a node and its subtree.
    #[allow(clippy::too_many_arguments)]
    fn atom(
        &self,
        j: usize,
        end: usize,
        style: Style,
        base: f32,
        sink: &mut dyn GlyphSink,
        pen: (f32, f32),
        middle: Option<f32>,
    ) -> MBox {
        let seg = &self.segs[j];
        let base = seg.font_size.unwrap_or(base);
        let Some(node) = seg.math_node else {
            return self.leaf(seg, style, base, sink, pen);
        };
        let slots = self.slots(j, end);
        let color = seg.color.unwrap_or(self.rt.color);
        let at = Place {
            style,
            base,
            pen,
            color,
        };
        match node.kind {
            MathKind::Math(_) => self.row(slots[0], style, base, sink, pen, None),
            MathKind::Frac(fs) => self.fraction(slots, fs, false, at, sink),
            MathKind::Binom(fs) => self.fraction(slots, fs, true, at, sink),
            MathKind::Sqrt => self.radical(seg.text, slots, at, sink),
            MathKind::Scripts => {
                let b = self.row(slots[0], style, base, &mut MeasureSink, (0.0, 0.0), None);
                if sink.paints() {
                    self.row(slots[0], style, base, sink, pen, None);
                }
                self.scripts(b, slots[1], slots[2], false, at, sink)
            }
            MathKind::BigOp(limits) => self.big_op(seg.text, slots, limits, at, sink),
            MathKind::OpName(limits) => self.op_name(slots, limits, at, sink),
            MathKind::Delim { open, close } => self.delimited(open, close, slots[0], at, sink),
            MathKind::Middle => self.fence(seg.text, middle.unwrap_or(0.0), at, sink),
            MathKind::Sized { level, .. } => {
                let target = SIZED_EM[(level.clamp(1, 4) - 1) as usize] * self.size(style, base);
                self.fence(seg.text, target, at, sink)
            }
            MathKind::Accent(kind) => self.accent(seg.text, kind, slots[0], at, sink),
            MathKind::Over(kind) => self.over_under(seg.text, kind, true, slots, at, sink),
            MathKind::Under(kind) => self.over_under(seg.text, kind, false, slots, at, sink),
        }
    }

    /// A leaf: a glyph, or a space of its field width.
    fn leaf(
        &self,
        seg: &RichSegment,
        style: Style,
        base: f32,
        sink: &mut dyn GlyphSink,
        pen: (f32, f32),
    ) -> MBox {
        let size = self.size(style, base);
        if seg.text.is_whitespace() {
            let width = match seg.field_em {
                Some(f) => f * size,
                None => {
                    let face = self.face.font;
                    glyph_extents_id(face, size, self.face.glyph(' ')).advance
                }
            };
            return MBox::empty(width);
        }
        let (font, glyph, math) = self.leaf_glyph(seg, style);
        let g = glyph_extents_id(font, size, glyph);
        let (width, inset) = match seg.field_em {
            Some(f) => (f * size, (f * size - g.advance) * 0.5),
            None => (g.advance, 0.0),
        };
        if sink.paints() {
            let color = seg.color.unwrap_or(self.rt.color);
            sink.glyph(font, glyph, size, (pen.0 + inset, pen.1), &color);
        }
        let mut b = MBox::empty(width);
        if g.ink_left != g.ink_right || g.ink_top != g.ink_bottom {
            b.ink_rect(
                inset + g.ink_left,
                g.ink_top,
                inset + g.ink_right,
                g.ink_bottom,
            );
        }
        if math {
            b.italic = self.face.px(self.face.italic_correction(glyph), size);
            b.accent = self
                .face
                .top_accent(glyph)
                .map(|a| inset + self.face.px(a, size));
            b.glyph = Some(glyph);
        }
        b
    }

    /// The face and glyph a leaf draws, and whether it is a math-face glyph:
    /// math and Greek characters from the math face (their mathematical
    /// alphanumeric, its script alternate in script styles), text from the
    /// policy's chain; a character the math face lacks falls back to the
    /// standard chain.
    fn leaf_glyph(&self, seg: &RichSegment, style: Style) -> (FontRef<'static>, u16, bool) {
        let ch = match seg.char_set {
            CharSet::Text => {
                let font = glyph_font(self.policy, &self.rt.font, seg.bold, seg.italic, seg.text);
                return (font, font.charmap().map(seg.text), false);
            }
            CharSet::Greek => greek_char(seg.text),
            CharSet::Math => seg.text,
        };
        let mapped = self.face.glyph(math_alphanumeric(ch, seg.bold, seg.italic));
        let glyph = if mapped != 0 {
            mapped
        } else {
            self.face.glyph(ch)
        };
        if glyph == 0 {
            let font = glyph_font(
                FontPolicy::Standard,
                &self.rt.font,
                seg.bold,
                seg.italic,
                ch,
            );
            return (font, font.charmap().map(ch), false);
        }
        let level = style.level.saturating_sub(1);
        (
            self.face.font,
            self.face.script_alternate(glyph, level),
            true,
        )
    }

    /// The shape of `stretched` at `size`.
    fn shape(&self, stretched: Stretched, horizontal: bool, size: f32) -> Shape {
        let font = self.face.font;
        let mut ink = MBox::empty(0.0);
        match stretched {
            Stretched::Glyph(glyph) => {
                let g = glyph_extents_id(font, size, glyph);
                ink.width = g.advance;
                if g.ink_left != g.ink_right || g.ink_top != g.ink_bottom {
                    ink.ink_rect(g.ink_left, g.ink_top, g.ink_right, g.ink_bottom);
                }
                ink.italic = self.face.px(self.face.italic_correction(glyph), size);
                ink.accent = self.face.top_accent(glyph).map(|a| self.face.px(a, size));
                ink.glyph = Some(glyph);
            }
            Stretched::Assembly(a) => {
                a.for_each_part(|part, offset| {
                    let g = glyph_extents_id(font, size, part.glyph_id.0);
                    let o = self.face.px(offset, size);
                    let (dx, dy) = if horizontal { (o, 0.0) } else { (0.0, -o) };
                    ink.width = ink.width.max(if horizontal { 0.0 } else { g.advance });
                    if g.ink_left != g.ink_right || g.ink_top != g.ink_bottom {
                        ink.ink_rect(
                            dx + g.ink_left,
                            dy + g.ink_top,
                            dx + g.ink_right,
                            dy + g.ink_bottom,
                        );
                    }
                });
                if horizontal {
                    ink.width = self.face.px(a.length, size);
                }
                ink.italic = self
                    .face
                    .px(a.assembly.italics_correction.value as f32, size);
            }
        }
        Shape {
            stretched,
            horizontal,
            size,
            ink,
        }
    }

    /// Place `shape` with its pen at `pen`.
    fn draw_shape(&self, sink: &mut dyn GlyphSink, shape: &Shape, pen: (f32, f32), color: &Color) {
        if !sink.paints() {
            return;
        }
        let font = self.face.font;
        match shape.stretched {
            Stretched::Glyph(glyph) => sink.glyph(font, glyph, shape.size, pen, color),
            Stretched::Assembly(a) => a.for_each_part(|part, offset| {
                let o = self.face.px(offset, shape.size);
                let at = if shape.horizontal {
                    (pen.0 + o, pen.1)
                } else {
                    (pen.0, pen.1 - o)
                };
                sink.glyph(font, part.glyph_id.0, shape.size, at, color);
            }),
        }
    }

    /// The glyph of `c` stretched to `target` pixels at `size`.
    fn stretch(&self, c: char, target: f32, horizontal: bool, size: f32) -> Option<Shape> {
        let glyph = self.face.glyph(c);
        if glyph == 0 {
            return None;
        }
        let stretched = self
            .face
            .stretch(glyph, self.face.units(target, size), horizontal);
        Some(self.shape(stretched, horizontal, size))
    }

    /// A fraction or binomial: numerator over denominator, a bar on the
    /// axis (none for a binomial, which sits in parentheses).
    fn fraction(
        &self,
        [num_r, den_r, _]: [(usize, usize); 3],
        fs: FracStyle,
        binom: bool,
        at: Place,
        sink: &mut dyn GlyphSink,
    ) -> MBox {
        let style = match fs {
            FracStyle::Auto => at.style,
            FracStyle::Display => Style {
                level: 0,
                ..at.style
            },
            FracStyle::Text => Style {
                level: 1,
                ..at.style
            },
        };
        let size = self.size(style, at.base);
        let c = *self.face.constants();
        let v = |m| self.face.value(m, size);
        let display = style.display();
        let num = self.row(
            num_r,
            style.num(),
            at.base,
            &mut MeasureSink,
            (0.0, 0.0),
            None,
        );
        let den = self.row(
            den_r,
            style.den(),
            at.base,
            &mut MeasureSink,
            (0.0, 0.0),
            None,
        );
        let axis = v(c.axis_height());
        let thickness = v(c.fraction_rule_thickness()).max(1.0);
        let (u, d) = if binom {
            let (mut u, mut d, gap) = if display {
                (
                    v(c.stack_top_display_style_shift_up()),
                    v(c.stack_bottom_display_style_shift_down()),
                    v(c.stack_display_style_gap_min()),
                )
            } else {
                (
                    v(c.stack_top_shift_up()),
                    v(c.stack_bottom_shift_down()),
                    v(c.stack_gap_min()),
                )
            };
            let actual = (u - num.descent) - (den.ascent - d);
            if actual < gap {
                u += (gap - actual) * 0.5;
                d += (gap - actual) * 0.5;
            }
            (u, d)
        } else {
            let (u, d, gap_num, gap_den) = if display {
                (
                    v(c.fraction_numerator_display_style_shift_up()),
                    v(c.fraction_denominator_display_style_shift_down()),
                    v(c.fraction_num_display_style_gap_min()),
                    v(c.fraction_denom_display_style_gap_min()),
                )
            } else {
                (
                    v(c.fraction_numerator_shift_up()),
                    v(c.fraction_denominator_shift_down()),
                    v(c.fraction_numerator_gap_min()),
                    v(c.fraction_denominator_gap_min()),
                )
            };
            let half = thickness * 0.5;
            (
                u.max(axis + half + gap_num + num.descent),
                d.max(den.ascent - axis + half + gap_den),
            )
        };
        let (u, d) = (u.round(), d.round());
        let inner = num.width.max(den.width);
        let mut out = MBox::empty(0.0);
        // The binomial's parentheses cover the stack, centred on the axis.
        let (open, close) = if binom {
            let target = 2.0 * (u + num.ascent - axis).max(d + den.descent + axis);
            (
                self.stretch('(', target, false, size),
                self.stretch(')', target, false, size),
            )
        } else {
            (None, None)
        };
        let pad = NULL_DELIMITER_EM * size;
        let left = open.as_ref().map_or(pad, |s| s.ink.width);
        let num_x = left + (inner - num.width) * 0.5;
        let den_x = left + (inner - den.width) * 0.5;
        out.add(&num, num_x, -u);
        out.add(&den, den_x, d);
        if sink.paints() {
            self.row(
                num_r,
                style.num(),
                at.base,
                sink,
                at.offset(num_x, -u),
                None,
            );
            self.row(den_r, style.den(), at.base, sink, at.offset(den_x, d), None);
        }
        if !binom {
            let bar = RectF {
                x: left,
                y: -axis - thickness * 0.5,
                width: inner,
                height: thickness,
            };
            out.ink_rect(bar.x, bar.y, bar.x + bar.width, bar.y + bar.height);
            sink.rect(at.rect(bar), &at.color);
        }
        let mut right = left + inner;
        for (shape, x) in [(open, 0.0), (close, right)] {
            if let Some(shape) = shape {
                let dy = self.axis_shift(&shape, size);
                out.add(&shape.ink, x, dy);
                self.draw_shape(sink, &shape, at.offset(x, dy), &at.color);
                if x > 0.0 {
                    right += shape.ink.width;
                }
            }
        }
        out.width = if binom { right } else { right + pad };
        out
    }

    /// The baseline shift (y down) that centres `shape` on the math axis.
    fn axis_shift(&self, shape: &Shape, size: f32) -> f32 {
        let axis = self.face.value(self.face.constants().axis_height(), size);
        let (top, bottom) = self.vertical_extent(shape);
        ((top - bottom) * 0.5 - axis).round()
    }

    /// A shape's ink above and below its pen, a vertical assembly's from its
    /// bottom.
    fn vertical_extent(&self, shape: &Shape) -> (f32, f32) {
        if shape.ink.ink {
            (shape.ink.ascent, shape.ink.descent)
        } else {
            (0.0, 0.0)
        }
    }

    /// A radical over slot 0 with the index of slot 1.
    fn radical(
        &self,
        symbol: char,
        [body_r, index_r, _]: [(usize, usize); 3],
        at: Place,
        sink: &mut dyn GlyphSink,
    ) -> MBox {
        let size = self.size(at.style, at.base);
        let c = *self.face.constants();
        let v = |m| self.face.value(m, size);
        let body_style = at.style.cramp();
        let body = self.row(
            body_r,
            body_style,
            at.base,
            &mut MeasureSink,
            (0.0, 0.0),
            None,
        );
        let gap = if at.style.display() {
            v(c.radical_display_style_vertical_gap())
        } else {
            v(c.radical_vertical_gap())
        };
        let thickness = v(c.radical_rule_thickness()).max(1.0);
        let (body_top, body_bottom) = if body.ink {
            (body.ascent, body.descent)
        } else {
            (0.0, 0.0)
        };
        let target = body_top + body_bottom + gap + thickness;
        let symbol = if symbol == '\u{200B}' { '√' } else { symbol };
        let Some(shape) = self.stretch(symbol, target, false, size) else {
            return self.row(body_r, body_style, at.base, sink, at.pen, None);
        };
        let (glyph_top, glyph_bottom) = self.vertical_extent(&shape);
        // The excess of the glyph over the target widens the gap evenly.
        let gap = gap + (glyph_top + glyph_bottom - target).max(0.0) * 0.5;
        let rule_top = body_top + gap + thickness;
        // The glyph's ink top meets the rule's top.
        let dy = (glyph_top - rule_top).round();
        let mut out = MBox::empty(0.0);
        let mut glyph_x = 0.0;
        if index_r.0 != index_r.1 {
            let index_style = Style {
                level: 3,
                cramped: true,
            };
            let index = self.row(
                index_r,
                index_style,
                at.base,
                &mut MeasureSink,
                (0.0, 0.0),
                None,
            );
            let before = v(c.radical_kern_before_degree());
            let after = v(c.radical_kern_after_degree());
            let bottom = dy + glyph_bottom;
            let raise = c.radical_degree_bottom_raise_percent() as f32 / 100.0;
            let index_y = (bottom - raise * (rule_top + bottom)).round();
            glyph_x = (before + index.width + after).max(0.0);
            out.add(&index, before, index_y);
            if sink.paints() {
                self.row(
                    index_r,
                    index_style,
                    at.base,
                    sink,
                    at.offset(before, index_y),
                    None,
                );
            }
        }
        out.add(&shape.ink, glyph_x, dy);
        self.draw_shape(sink, &shape, at.offset(glyph_x, dy), &at.color);
        let body_x = glyph_x + shape.ink.width;
        out.add(&body, body_x, 0.0);
        if sink.paints() {
            self.row(
                body_r,
                body_style,
                at.base,
                sink,
                at.offset(body_x, 0.0),
                None,
            );
        }
        let rule = RectF {
            x: body_x,
            y: -rule_top,
            width: body.width,
            height: thickness,
        };
        out.ink_rect(rule.x, rule.y, rule.x + rule.width, rule.y + rule.height);
        sink.rect(at.rect(rule), &at.color);
        out.width = body_x + body.width;
        out
    }

    /// Scripts beside a base box `base` already placed at `at.pen`:
    /// superscript `sup_r`, subscript `sub_r`. A large operator's subscript
    /// tucks under it by its italic correction.
    fn scripts(
        &self,
        base: MBox,
        sup_r: (usize, usize),
        sub_r: (usize, usize),
        large_op: bool,
        at: Place,
        sink: &mut dyn GlyphSink,
    ) -> MBox {
        let mut out = MBox::empty(base.width);
        out.add(&base, 0.0, 0.0);
        let has_sup = sup_r.0 != sup_r.1;
        let has_sub = sub_r.0 != sub_r.1;
        if !has_sup && !has_sub {
            out.italic = base.italic;
            out.accent = base.accent;
            out.glyph = base.glyph;
            return out;
        }
        let size = self.size(at.style, at.base);
        let c = *self.face.constants();
        let v = |m| self.face.value(m, size);
        let measure = |r, style| self.row(r, style, at.base, &mut MeasureSink, (0.0, 0.0), None);
        let sup = has_sup.then(|| measure(sup_r, at.style.sup()));
        let sub = has_sub.then(|| measure(sub_r, at.style.sub()));
        // A single glyph keeps the constant shifts (TeX); a composite base
        // pushes its scripts out by its own height and depth.
        let composite = base.glyph.is_none() && base.ink;
        let mut u = 0.0f32;
        let mut d = 0.0f32;
        if let Some(s) = &sup {
            u = if at.style.cramped {
                v(c.superscript_shift_up_cramped())
            } else {
                v(c.superscript_shift_up())
            };
            if composite {
                u = u.max(base.ascent - v(c.superscript_baseline_drop_max()));
            }
            u = u.max(v(c.superscript_bottom_min()) + s.descent);
        }
        if let Some(s) = &sub {
            d = v(c.subscript_shift_down());
            if composite {
                d = d.max(base.descent + v(c.subscript_baseline_drop_min()));
            }
            d = d.max(s.ascent - v(c.subscript_top_max()));
        }
        if let (Some(p), Some(b)) = (&sup, &sub) {
            let gap = (u - p.descent) - (b.ascent - d);
            let min_gap = v(c.sub_superscript_gap_min());
            if gap < min_gap {
                d += min_gap - gap;
            }
            let lift = v(c.superscript_bottom_max_with_subscript()) - (u - p.descent);
            if lift > 0.0 {
                u += lift;
                d -= lift;
            }
        }
        let (u, d) = (u.round(), d.round());
        let sup_x = base.width + base.italic;
        let sub_x = base.width - if large_op { base.italic } else { 0.0 };
        let mut width = base.width;
        if let Some(s) = &sup {
            out.add(s, sup_x, -u);
            width = width.max(sup_x + s.width);
            if sink.paints() {
                self.row(
                    sup_r,
                    at.style.sup(),
                    at.base,
                    sink,
                    at.offset(sup_x, -u),
                    None,
                );
            }
        }
        if let Some(s) = &sub {
            out.add(s, sub_x, d);
            width = width.max(sub_x + s.width);
            if sink.paints() {
                self.row(
                    sub_r,
                    at.style.sub(),
                    at.base,
                    sink,
                    at.offset(sub_x, d),
                    None,
                );
            }
        }
        out.width = width + v(c.space_after_script());
        out
    }

    /// Limits above and below an operator box `op`, centred, the upper one
    /// shifted right and the lower one left by half its italic correction.
    /// `draw_op` places the operator at its x offset.
    fn limits(
        &self,
        op: MBox,
        lower_r: (usize, usize),
        upper_r: (usize, usize),
        at: Place,
        sink: &mut dyn GlyphSink,
        draw_op: &mut dyn FnMut(&mut dyn GlyphSink, f32),
    ) -> MBox {
        let size = self.size(at.style, at.base);
        let c = *self.face.constants();
        let v = |m| self.face.value(m, size);
        let measure = |r, style| self.row(r, style, at.base, &mut MeasureSink, (0.0, 0.0), None);
        let upper = (upper_r.0 != upper_r.1).then(|| measure(upper_r, at.style.sup()));
        let lower = (lower_r.0 != lower_r.1).then(|| measure(lower_r, at.style.sub()));
        let width = op
            .width
            .max(upper.map_or(0.0, |b| b.width))
            .max(lower.map_or(0.0, |b| b.width));
        let op_x = (width - op.width) * 0.5;
        let mut out = MBox::empty(width);
        out.add(&op, op_x, 0.0);
        draw_op(sink, op_x);
        if let Some(b) = &upper {
            let rise =
                (v(c.upper_limit_gap_min()) + b.descent).max(v(c.upper_limit_baseline_rise_min()));
            let y = -(op.ascent + rise).round();
            let x = (width - b.width) * 0.5 + op.italic * 0.5;
            out.add(b, x, y);
            if sink.paints() {
                self.row(
                    upper_r,
                    at.style.sup(),
                    at.base,
                    sink,
                    at.offset(x, y),
                    None,
                );
            }
        }
        if let Some(b) = &lower {
            let drop =
                (v(c.lower_limit_gap_min()) + b.ascent).max(v(c.lower_limit_baseline_drop_min()));
            let y = (op.descent + drop).round();
            let x = (width - b.width) * 0.5 - op.italic * 0.5;
            out.add(b, x, y);
            if sink.paints() {
                self.row(
                    lower_r,
                    at.style.sub(),
                    at.base,
                    sink,
                    at.offset(x, y),
                    None,
                );
            }
        }
        out
    }

    /// A big operator (its glyph centred on the axis, the display variant in
    /// display style) with limits or scripts.
    fn big_op(
        &self,
        symbol: char,
        [lower_r, upper_r, _]: [(usize, usize); 3],
        limits: Limits,
        at: Place,
        sink: &mut dyn GlyphSink,
    ) -> MBox {
        let size = self.size(at.style, at.base);
        let glyph = self.face.glyph(symbol);
        if glyph == 0 {
            let leaf = RichSegment {
                char_set: CharSet::Math,
                color: Some(at.color),
                ..RichSegment::plain(symbol)
            };
            let b = self.leaf(&leaf, at.style, at.base, sink, at.pen);
            return self.scripts(b, upper_r, lower_r, true, at, sink);
        }
        let stretched = if at.style.display() {
            let min = self.face.constants().display_operator_min_height() as f32;
            self.face.stretch(glyph, min, false)
        } else {
            Stretched::Glyph(glyph)
        };
        let shape = self.shape(stretched, false, size);
        let dy = self.axis_shift(&shape, size);
        let mut op = MBox::empty(shape.ink.width);
        op.add(&shape.ink, 0.0, dy);
        op.italic = shape.ink.italic;
        let above_below = match limits {
            Limits::Limits => true,
            Limits::NoLimits => false,
            Limits::Auto => at.style.display() && !is_integral(symbol),
        };
        if above_below {
            let mut draw = |sink: &mut dyn GlyphSink, x: f32| {
                self.draw_shape(sink, &shape, at.offset(x, dy), &at.color)
            };
            return self.limits(op, lower_r, upper_r, at, sink, &mut draw);
        }
        self.draw_shape(sink, &shape, at.offset(0.0, dy), &at.color);
        self.scripts(op, upper_r, lower_r, true, at, sink)
    }

    /// An operator name (slot 0) with limits (slots 1 and 2), placed as a
    /// big operator's are: above and below in display style for `Auto`.
    fn op_name(
        &self,
        [name_r, lower_r, upper_r]: [(usize, usize); 3],
        limits: Limits,
        at: Place,
        sink: &mut dyn GlyphSink,
    ) -> MBox {
        let name = self.row(
            name_r,
            at.style,
            at.base,
            &mut MeasureSink,
            (0.0, 0.0),
            None,
        );
        let above_below = match limits {
            Limits::Limits => true,
            Limits::NoLimits => false,
            Limits::Auto => at.style.display(),
        };
        let mut draw = |sink: &mut dyn GlyphSink, x: f32| {
            if sink.paints() {
                self.row(name_r, at.style, at.base, sink, at.offset(x, 0.0), None);
            }
        };
        let mut op = name;
        op.glyph = None;
        op.italic = 0.0;
        if above_below {
            return self.limits(op, lower_r, upper_r, at, sink, &mut draw);
        }
        draw(sink, 0.0);
        self.scripts(op, upper_r, lower_r, false, at, sink)
    }

    /// Stretchy delimiters around slot 0; `Middle` nodes in it stretch to
    /// the same size.
    fn delimited(
        &self,
        open: char,
        close: char,
        body_r: (usize, usize),
        at: Place,
        sink: &mut dyn GlyphSink,
    ) -> MBox {
        let size = self.size(at.style, at.base);
        let axis = self.face.value(self.face.constants().axis_height(), size);
        let measure = |middle| {
            self.row(
                body_r,
                at.style,
                at.base,
                &mut MeasureSink,
                (0.0, 0.0),
                middle,
            )
        };
        let natural = measure(None);
        let target = if natural.ink {
            2.0 * (natural.ascent - axis).max(natural.descent + axis)
        } else {
            0.0
        };
        let has_middle = (body_r.0..body_r.1).any(|i| {
            self.segs[i]
                .math_node
                .is_some_and(|n| n.kind == MathKind::Middle)
        });
        let body = if has_middle {
            measure(Some(target))
        } else {
            natural
        };
        let mut out = MBox::empty(0.0);
        let mut x = 0.0;
        let open_box = self.fence(open, target, at, sink);
        out.add(&open_box, 0.0, 0.0);
        x += open_box.width;
        out.add(&body, x, 0.0);
        if sink.paints() {
            let middle = has_middle.then_some(target);
            self.row(body_r, at.style, at.base, sink, at.offset(x, 0.0), middle);
        }
        x += body.width;
        let close_box = self.fence(close, target, at.shifted(x), sink);
        out.add(&close_box, x, 0.0);
        out.width = x + close_box.width;
        out
    }

    /// A delimiter `c` stretched to `target` pixels, centred on the axis
    /// (`.` is the null delimiter: an empty space).
    fn fence(&self, c: char, target: f32, at: Place, sink: &mut dyn GlyphSink) -> MBox {
        let size = self.size(at.style, at.base);
        if c == '.' {
            return MBox::empty(NULL_DELIMITER_EM * size);
        }
        let Some(shape) = self.stretch(c, target, false, size) else {
            let leaf = RichSegment {
                char_set: CharSet::Math,
                color: Some(at.color),
                ..RichSegment::plain(c)
            };
            return self.leaf(&leaf, at.style, at.base, sink, at.pen);
        };
        let dy = self.axis_shift(&shape, size);
        let mut out = MBox::empty(shape.ink.width);
        out.add(&shape.ink, 0.0, dy);
        self.draw_shape(sink, &shape, at.offset(0.0, dy), &at.color);
        out
    }

    /// An accent over slot 0: its attachment point over the body's, raised
    /// by the body's height past `AccentBaseHeight`; a wide accent
    /// stretches over the body.
    fn accent(
        &self,
        mark: char,
        kind: AccentKind,
        body_r: (usize, usize),
        at: Place,
        sink: &mut dyn GlyphSink,
    ) -> MBox {
        let size = self.size(at.style, at.base);
        let body_style = at.style.cramp();
        let body = self.row(
            body_r,
            body_style,
            at.base,
            &mut MeasureSink,
            (0.0, 0.0),
            None,
        );
        if sink.paints() {
            self.row(body_r, body_style, at.base, sink, at.pen, None);
        }
        let mut out = body;
        let Some(shape) = (match kind {
            AccentKind::Fixed => {
                let glyph = self.face.glyph(mark);
                (glyph != 0).then(|| self.shape(Stretched::Glyph(glyph), true, size))
            }
            AccentKind::Wide => self.stretch(mark, body.width, true, size),
        }) else {
            return out;
        };
        let base_attach = match (body.glyph, body.accent) {
            (Some(_), Some(a)) => a,
            _ => body.width * 0.5,
        };
        let mark_attach = match (kind, shape.ink.accent) {
            (AccentKind::Fixed, Some(a)) => a,
            _ => shape.ink.width * 0.5,
        };
        let x = base_attach - mark_attach;
        let base_height = self
            .face
            .value(self.face.constants().accent_base_height(), size);
        let raise = if body.ink {
            (body.ascent - base_height).max(0.0).round()
        } else {
            0.0
        };
        out.add(&shape.ink, x, -raise);
        self.draw_shape(sink, &shape, at.offset(x, -raise), &at.color);
        out.width = body.width;
        out.glyph = None;
        out.accent = None;
        out
    }

    /// A rule, brace, arrow or label over (`over`) or under slot 0, with the
    /// label of slot 1 past a brace or an arrow.
    fn over_under(
        &self,
        symbol: char,
        kind: OverUnder,
        over: bool,
        [body_r, label_r, _]: [(usize, usize); 3],
        at: Place,
        sink: &mut dyn GlyphSink,
    ) -> MBox {
        let size = self.size(at.style, at.base);
        let c = *self.face.constants();
        let v = |m| self.face.value(m, size);
        let body_style = if over && kind == OverUnder::Line {
            at.style.cramp()
        } else {
            at.style
        };
        let label_style = if over { at.style.sup() } else { at.style.sub() };
        let body = self.row(
            body_r,
            body_style,
            at.base,
            &mut MeasureSink,
            (0.0, 0.0),
            None,
        );
        let has_label = label_r.0 != label_r.1;
        let label = self.row(
            label_r,
            label_style,
            at.base,
            &mut MeasureSink,
            (0.0, 0.0),
            None,
        );
        let (body_top, body_bottom) = if body.ink {
            (body.ascent, body.descent)
        } else {
            (0.0, 0.0)
        };
        // The stretched piece: a rule, or the node's glyph across the body
        // (an arrow also across its label).
        let empty_body = body_r.0 == body_r.1;
        let mut piece_w = body.width;
        if kind == OverUnder::Arrow {
            piece_w = piece_w.max(label.width + ARROW_PADDING_EM * size);
        }
        let shape = match kind {
            OverUnder::Brace | OverUnder::Arrow => self.stretch(symbol, piece_w, true, size),
            _ => None,
        };
        let width = body
            .width
            .max(if has_label { label.width } else { 0.0 })
            .max(shape.as_ref().map_or(0.0, |s| s.ink.width));
        let body_x = (width - body.width) * 0.5;
        let mut out = MBox::empty(width);
        out.add(&body, body_x, 0.0);
        if sink.paints() {
            self.row(
                body_r,
                body_style,
                at.base,
                sink,
                at.offset(body_x, 0.0),
                None,
            );
        }
        // `edge` tracks the outer ink edge (distance from the baseline,
        // positive away from it) the next piece stacks past.
        let mut edge = if over { body_top } else { body_bottom };
        match kind {
            OverUnder::Line => {
                let (gap, t) = if over {
                    (v(c.overbar_vertical_gap()), v(c.overbar_rule_thickness()))
                } else {
                    (v(c.underbar_vertical_gap()), v(c.underbar_rule_thickness()))
                };
                let t = t.max(1.0);
                let y = if over { -(edge + gap + t) } else { edge + gap };
                let rule = RectF {
                    x: body_x,
                    y,
                    width: body.width,
                    height: t,
                };
                out.ink_rect(rule.x, rule.y, rule.x + rule.width, rule.y + rule.height);
                sink.rect(at.rect(rule), &at.color);
                edge += gap + t;
            }
            OverUnder::Brace | OverUnder::Arrow => {
                if let Some(shape) = &shape {
                    let (top, bottom) = self.vertical_extent(shape);
                    let x = (width - shape.ink.width) * 0.5;
                    // An arrow without a body stays where the font puts it
                    // (on the axis); otherwise the piece stacks past the body.
                    let dy = if empty_body && kind == OverUnder::Arrow {
                        0.0
                    } else {
                        let gap = if kind == OverUnder::Brace {
                            v(c.stretch_stack_gap_below_min())
                        } else {
                            v(c.overbar_vertical_gap())
                        };
                        if over {
                            -(edge + gap + bottom).round()
                        } else {
                            (edge + gap + top).round()
                        }
                    };
                    out.add(&shape.ink, x, dy);
                    self.draw_shape(sink, shape, at.offset(x, dy), &at.color);
                    edge = if over { top - dy } else { bottom + dy };
                }
            }
            OverUnder::Set => {}
        }
        if has_label {
            let gap = match kind {
                OverUnder::Brace => v(c.stretch_stack_gap_above_min()),
                _ if over => v(c.overbar_vertical_gap()),
                _ => v(c.underbar_vertical_gap()),
            };
            let x = (width - label.width) * 0.5;
            let y = if over {
                -(edge + gap + label.descent).round()
            } else {
                (edge + gap + label.ascent).round()
            };
            out.add(&label, x, y);
            if sink.paints() {
                self.row(label_r, label_style, at.base, sink, at.offset(x, y), None);
            }
        }
        out
    }
}

/// Where a node is laid out: its style, base size, pen and colour.
#[derive(Clone, Copy)]
struct Place {
    style: Style,
    base: f32,
    pen: (f32, f32),
    color: Color,
}

impl Place {
    /// The pen moved by (`dx`, `dy`).
    fn offset(&self, dx: f32, dy: f32) -> (f32, f32) {
        (self.pen.0 + dx, self.pen.1 + dy)
    }

    /// The same place, its pen moved right by `dx`.
    fn shifted(&self, dx: f32) -> Place {
        Place {
            pen: self.offset(dx, 0.0),
            ..*self
        }
    }

    /// A rectangle relative to the pen, in the sink's frame.
    fn rect(&self, r: RectF) -> RectF {
        RectF {
            x: self.pen.0 + r.x,
            y: self.pen.1 + r.y,
            ..r
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{CanvasSink, draw_rich_text, measure_rich_text, place_cell};
    use super::*;
    use crate::raster::Canvas;
    use crate::text::{MathNode, MathSlot};

    const Z: char = '\u{200B}';

    fn node(text: char, id: u32, kind: MathKind, parent: Option<(u32, u8)>) -> RichSegment {
        RichSegment {
            math_node: Some(MathNode { id, kind }),
            math_parent: parent.map(|(id, slot)| MathSlot { id, slot }),
            ..RichSegment::plain(text)
        }
    }

    /// An italic math letter, digit or operator in slot `slot` of `id`.
    fn leaf(text: char, id: u32, slot: u8) -> RichSegment {
        RichSegment {
            char_set: CharSet::Math,
            italic: text.is_ascii_alphabetic(),
            math_parent: Some(MathSlot { id, slot }),
            ..RichSegment::plain(text)
        }
    }

    fn text(segments: Vec<RichSegment>, size: f32) -> RichText {
        RichText {
            segments,
            ..RichText::plain("", Color::BLACK, size, "")
        }
    }

    /// `$\frac{a+b}{c}$`.
    fn fraction() -> Vec<RichSegment> {
        vec![
            node(Z, 0, MathKind::Math(MathStyle::Text), None),
            node(Z, 1, MathKind::Frac(FracStyle::Auto), Some((0, 0))),
            leaf('a', 1, 0),
            leaf('+', 1, 0),
            leaf('b', 1, 0),
            leaf('c', 1, 1),
        ]
    }

    /// `$\sqrt[3]{x_i^2}$`.
    fn radical() -> Vec<RichSegment> {
        vec![
            node(Z, 0, MathKind::Math(MathStyle::Text), None),
            node('√', 1, MathKind::Sqrt, Some((0, 0))),
            node(Z, 2, MathKind::Scripts, Some((1, 0))),
            leaf('x', 2, 0),
            leaf('2', 2, 1),
            leaf('i', 2, 2),
            leaf('3', 1, 1),
        ]
    }

    /// `$\overrightarrow{ABCDEFGH}$`.
    fn arrow() -> Vec<RichSegment> {
        let mut s = vec![
            node(Z, 0, MathKind::Math(MathStyle::Text), None),
            node(
                '\u{20D7}',
                1,
                MathKind::Accent(AccentKind::Wide),
                Some((0, 0)),
            ),
        ];
        s.extend("ABCDEFGH".chars().map(|c| leaf(c, 1, 0)));
        s
    }

    /// Draw `rt` with a margin; the image, its width and height, and the
    /// first line's origin.
    fn draw(rt: &RichText, policy: FontPolicy) -> (Vec<u8>, u32, u32, (f32, f32)) {
        let m = measure_rich_text(rt, policy);
        let (w, h) = (m.width.ceil() as u32 + 16, m.height().ceil() as u32 + 18);
        let mut canvas = Canvas::new(w, h).unwrap();
        // A whole-pixel baseline: the glyph raster snaps baselines to the
        // pixel grid, which would add half a pixel to every bound below.
        let origin = (8.0, (8.0 + m.ascent).ceil());
        draw_rich_text(&mut canvas, rt, origin, policy);
        (canvas.into_rgba(), w, h, origin)
    }

    /// The bounding box of the pixels with ink, as (x0, y0, x1, y1).
    fn ink_box(rgba: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
        let mut b: Option<(u32, u32, u32, u32)> = None;
        for y in 0..h {
            for x in 0..w {
                if rgba[((y * w + x) * 4 + 3) as usize] > 0 {
                    b = Some(match b {
                        None => (x, y, x, y),
                        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    });
                }
            }
        }
        b
    }

    // A formula's pixels lie inside its measured envelope (within the one
    // pixel of anti-aliasing), and the measuring and the drawing pass of
    // `place_cell` report the same envelope.
    #[test]
    fn formulas_draw_inside_their_measured_envelope() {
        for segments in [fraction(), radical(), arrow()] {
            for size in [9.0, 16.0, 40.0] {
                let rt = text(segments.clone(), size);
                for policy in [FontPolicy::Standard, FontPolicy::Handwritten] {
                    let m = measure_rich_text(&rt, policy);
                    let (rgba, w, h, (ox, oy)) = draw(&rt, policy);
                    let (x0, y0, x1, y1) = ink_box(&rgba, w, h).expect("a formula has ink");
                    assert!(x0 as f32 >= ox - 1.0 && (x1 + 1) as f32 <= ox + m.width + 1.0);
                    assert!(
                        y0 as f32 >= oy - m.ascent - 1.0,
                        "{size}: top {y0} vs {m:?}"
                    );
                    assert!(
                        (y1 + 1) as f32 <= oy + m.descent + 1.0,
                        "{size}: {y1} vs {m:?}"
                    );
                    // The envelope is tight: the ink reaches its top.
                    assert!((y0 as f32 - (oy - m.ascent)).abs() <= 1.0, "{size}: {m:?}");
                    let mut canvas = Canvas::new(w, h).unwrap();
                    let drawn = place_cell(
                        &mut CanvasSink(&mut canvas),
                        &rt.segments,
                        &rt,
                        (ox, oy),
                        policy,
                    );
                    assert_eq!(drawn, m, "measure and draw agree");
                }
            }
        }
    }

    // Math glyphs come from the math face whatever the font policy (D6):
    // a formula without text draws the same pixels in sketch mode.
    #[test]
    fn math_ignores_the_handwritten_policy() {
        let rt = text(radical(), 18.0);
        assert_eq!(
            draw(&rt, FontPolicy::Standard).0,
            draw(&rt, FontPolicy::Handwritten).0
        );
    }

    // A fraction is taller than its letters, its bar between the two rows.
    #[test]
    fn a_fraction_stacks_numerator_over_denominator() {
        let rt = text(fraction(), 20.0);
        let flat = text(
            fraction()
                .into_iter()
                .map(|s| RichSegment {
                    math_node: None,
                    math_parent: None,
                    ..s
                })
                .collect(),
            20.0,
        );
        let stacked = measure_rich_text(&rt, FontPolicy::Standard);
        let inline = measure_rich_text(&flat, FontPolicy::Standard);
        assert!(
            stacked.height() > inline.height() * 1.4,
            "{stacked:?} vs {inline:?}"
        );
        assert!(stacked.width < inline.width, "{stacked:?} vs {inline:?}");
    }

    // A text-mode `Scripts` node places its slots exactly as the
    // superscript and subscript flags do: the same pixels and extents.
    #[test]
    fn text_mode_scripts_are_the_flags() {
        let flags = text(
            vec![
                RichSegment::plain('1'),
                RichSegment::plain('0'),
                RichSegment {
                    superscript: true,
                    ..RichSegment::plain('−')
                },
                RichSegment {
                    superscript: true,
                    ..RichSegment::plain('3')
                },
                RichSegment {
                    subscript: true,
                    ..RichSegment::plain('k')
                },
            ],
            14.0,
        );
        let child = |c: char, slot: u8| RichSegment {
            math_parent: Some(MathSlot { id: 0, slot }),
            ..RichSegment::plain(c)
        };
        let node = text(
            vec![
                node(Z, 0, MathKind::Scripts, None),
                child('1', 0),
                child('0', 0),
                child('−', 1),
                child('3', 1),
                child('k', 2),
            ],
            14.0,
        );
        for policy in [FontPolicy::Standard, FontPolicy::Handwritten] {
            assert_eq!(
                measure_rich_text(&node, policy),
                measure_rich_text(&flags, policy)
            );
            assert_eq!(draw(&node, policy).0, draw(&flags, policy).0);
        }
    }

    // A malformed tree draws flat, as a consumer ignoring math nodes would,
    // in measure and draw alike.
    #[test]
    fn a_malformed_cell_draws_flat() {
        let mut broken = fraction();
        broken[5].math_parent = Some(MathSlot { id: 9, slot: 0 });
        let flat: Vec<RichSegment> = broken
            .iter()
            .map(|s| RichSegment {
                math_node: None,
                math_parent: None,
                ..s.clone()
            })
            .collect();
        let (a, b) = (text(broken, 16.0), text(flat, 16.0));
        assert_eq!(
            measure_rich_text(&a, FontPolicy::Standard),
            measure_rich_text(&b, FontPolicy::Standard)
        );
        assert_eq!(
            draw(&a, FontPolicy::Standard).0,
            draw(&b, FontPolicy::Standard).0
        );
    }

    // A stretched assembly's parts join: the wide arrow's shaft has ink in
    // every column it spans.
    #[test]
    fn a_horizontal_assembly_joins_without_gaps() {
        let rt = text(arrow(), 60.0);
        let (rgba, w, h, _) = draw(&rt, FontPolicy::Standard);
        let inked = |x: u32| (0..h / 4).any(|y| rgba[((y * w + x) * 4 + 3) as usize] > 40);
        let first = (0..w).find(|&x| inked(x)).unwrap();
        let last = (0..w).rev().find(|&x| inked(x)).unwrap();
        assert!((first..=last).all(inked), "a gap in {first}..{last}");
        assert!(last - first > 200, "the arrow spans the body");
    }

    /// A sink that records the glyph ids it is handed.
    struct Glyphs(Vec<u16>);

    impl GlyphSink for Glyphs {
        fn paints(&self) -> bool {
            true
        }
        fn glyph(&mut self, _: FontRef<'static>, glyph: u16, _: f32, _: (f32, f32), _: &Color) {
            self.0.push(glyph);
        }
        fn rect(&mut self, _: RectF, _: &Color) {}
    }

    fn tex(src: &str) -> RichText {
        RichText::from_tex(src, Color::BLACK, 20.0, "").expect("valid TeX")
    }

    fn glyphs(rt: &RichText) -> Vec<u16> {
        let mut sink = Glyphs(Vec::new());
        place_cell(
            &mut sink,
            &rt.segments,
            rt,
            (0.0, 0.0),
            FontPolicy::Standard,
        );
        sink.0
    }

    // A display root takes the operator's display variant and its limits
    // above and below; a text root keeps the base glyph and side limits.
    #[test]
    fn the_root_style_chooses_the_operator_variant_and_its_limits() {
        let face = MathFace::for_family("");
        let sum = face.glyph('∑');
        let (text, display) = (tex(r"$\sum_{i=1}^{n} i$"), tex(r"$$\sum_{i=1}^{n} i$$"));
        assert!(glyphs(&text).contains(&sum));
        let drawn = glyphs(&display);
        assert!(
            !drawn.contains(&sum),
            "the display variant replaces the base glyph"
        );
        let m = |rt: &RichText| measure_rich_text(rt, FontPolicy::Standard);
        let (t, d) = (m(&text), m(&display));
        assert!(d.height() > t.height() * 1.5, "{d:?} vs {t:?}");
        assert!(d.width < t.width, "limits stack: {d:?} vs {t:?}");
        // `\limits` in text style stacks the limits around the text glyph.
        assert!(glyphs(&tex(r"$\sum\limits_{i=1}^{n} i$")).contains(&sum));
    }

    // A display root lays a fraction out with the display shifts and gaps
    // and text-style parts; `\dfrac` inside a text root is the same
    // fraction.
    #[test]
    fn the_root_style_sets_the_fraction_gaps_and_dfrac_still_forces_them() {
        let m = |src: &str| measure_rich_text(&tex(src), FontPolicy::Standard);
        let (text, display) = (m(r"$\frac{a}{b}$"), m(r"$$\frac{a}{b}$$"));
        assert!(
            display.height() > text.height() * 1.3,
            "{display:?} vs {text:?}"
        );
        assert_eq!(m(r"$\dfrac{a}{b}$"), display);
        assert_eq!(m(r"$$\tfrac{a}{b}$$"), text);
        // The style starts at the root: a display formula's scripts are
        // still script style.
        assert_eq!(m("$$x^2$$"), m("$x^2$"));
    }

    // Script styles draw the `ssty` alternates (D5): a superscript prime is
    // the face's script prime, not the base glyph scaled.
    #[test]
    fn script_styles_take_the_script_alternates() {
        let face = MathFace::for_family("");
        let rt = text(
            vec![node(Z, 0, MathKind::Math(MathStyle::Text), None)],
            16.0,
        );
        let f = Formula {
            rt: &rt,
            segs: &rt.segments,
            face,
            policy: FontPolicy::Standard,
        };
        let prime = leaf('′', 0, 0);
        let script = Style::TEXT.sup();
        let (_, base_glyph, _) = f.leaf_glyph(&prime, Style::TEXT);
        let (_, script_glyph, _) = f.leaf_glyph(&prime, script);
        let (_, tiny_glyph, _) = f.leaf_glyph(&prime, script.sup());
        assert_eq!(base_glyph, face.glyph('′'));
        assert_eq!(script_glyph, face.script_alternate(base_glyph, 1));
        assert_eq!(tiny_glyph, face.script_alternate(base_glyph, 2));
        assert_ne!(script_glyph, base_glyph);
    }
}
