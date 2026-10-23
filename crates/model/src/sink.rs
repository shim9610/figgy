//! The placement contract between an element and its consumers.
//!
//! Drawing has three stages: placement (what is drawn where, with which
//! arguments), coverage (which pixels a glyph, stroke or fill paints) and the
//! pixels. Every chart element has exactly one placement function, its
//! `emit`, which hands each placed part to a [`Sink`]. The renderer's canvas
//! sink turns the parts into pixels; the model's sinks in [`crate::select`]
//! turn the same parts into the element's bounds, its hit test and its
//! selection box. A second placement for the same element has nothing to
//! plug into, so drawn and interaction geometry cannot diverge.
//!
//! There is no intermediate shape list: an emit calls the sink part by part,
//! and a sink keeps only what its answer needs.

use crate::color::Color;
use crate::config::ColorBarOptions;
use crate::layout::{RectF, TitlePlacement};
use crate::text::{RichText, TextExtents};

/// The z position of a part relative to the data: the renderer's compositor
/// draws every `Below` part, then the data, then every `Above` part. An
/// element declares the slot of each part where it emits it; the geometry
/// sinks ignore it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Under the data: the space background, grid lines, polar rings and
    /// spokes, annotation spans.
    Below,
    /// Over the data: axis lines and ticks, tick labels, titles, the legend,
    /// the colourbar, selection boxes, annotation lines, arrows and text.
    Above,
}

/// The text of a placed block: a single-style string or a rich text.
#[derive(Clone, Copy, Debug)]
pub enum TextRef<'a> {
    Plain {
        text: &'a str,
        family: &'a str,
        size: f32,
        color: Color,
    },
    Rich(&'a RichText),
}

/// One placed block of text: its text, the baseline origin of its first
/// line, its measured extents and an optional rotation about a centre, in
/// degrees. The emit measures once; every sink reads the same extents.
#[derive(Clone, Copy, Debug)]
pub struct PlacedText<'a> {
    pub text: TextRef<'a>,
    pub origin: (f32, f32),
    pub extents: TextExtents,
    pub rotation: Option<(f32, (f32, f32))>,
}

impl PlacedText<'_> {
    /// The axis-aligned screen rectangle of the block, rotation applied.
    pub fn rect(&self) -> RectF {
        let (rotation_degrees, rotation_center) = self.rotation.unwrap_or((0.0, self.origin));
        TitlePlacement {
            origin: self.origin,
            rotation_center,
            rotation_degrees,
        }
        .rect(self.extents)
    }
}

/// How a segment is stroked. `tag` is the stable element tag the renderer's
/// sketch stroker seeds its wobble from (`"axis_left"`, `"tick_bottom_3"`).
#[derive(Clone, Copy, Debug)]
pub struct StrokeStyle<'a> {
    pub width: f32,
    pub color: Color,
    pub dash: &'a [f32],
    pub tag: &'a str,
}

/// A rectangle's border, stroked centred on its edge.
#[derive(Clone, Copy, Debug)]
pub struct Border<'a> {
    pub color: Color,
    pub width: f32,
    pub tag: &'a str,
}

/// A consumer of placed parts. The renderer's canvas sink paints them; the
/// model's sinks compute bounds, containment and selection boxes from them.
pub trait Sink {
    fn text(&mut self, slot: Slot, text: &PlacedText<'_>);
    fn stroke(&mut self, slot: Slot, a: [f32; 2], b: [f32; 2], style: &StrokeStyle<'_>);
    /// A rectangle with an optional fill and an optional border. Both `None`
    /// places a rectangle that paints nothing, such as the data area.
    fn rect(&mut self, slot: Slot, rect: RectF, fill: Option<Color>, border: Option<Border<'_>>);
    fn triangle(&mut self, slot: Slot, points: [[f32; 2]; 3], color: Color);
    /// The colourbar strip: `rect` is its placement, the colour bands inside
    /// it are coverage the canvas sink paints from `bar`.
    fn ramp(&mut self, slot: Slot, rect: RectF, bar: &ColorBarOptions);
}

/// The stored form of one emitted part, in the chart frame: what the
/// geometry sinks keep and what a per-part selection outlines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawnPart {
    /// A block of glyphs, rotation applied: one tick label, a title.
    Text(RectF),
    /// A stroked segment `width` across: an axis line, one tick, a reference
    /// line, an arrow's shaft.
    Segment {
        a: [f32; 2],
        b: [f32; 2],
        width: f32,
    },
    /// A rectangle: the legend box, the colourbar strip, the data area, a
    /// span, a text box.
    Rect(RectF),
    /// A filled triangle: an arrow head, tip first.
    Triangle([[f32; 2]; 3]),
}

impl DrawnPart {
    /// The axis-aligned extent of the part: a text block or rectangle as it
    /// is, a segment with half its width either side, a triangle's bounding
    /// box.
    pub fn extent(&self) -> RectF {
        match *self {
            DrawnPart::Text(r) | DrawnPart::Rect(r) => r,
            DrawnPart::Segment { a, b, width } => {
                let half = width * 0.5;
                RectF {
                    x: a[0].min(b[0]) - half,
                    y: a[1].min(b[1]) - half,
                    width: (a[0] - b[0]).abs() + half * 2.0,
                    height: (a[1] - b[1]).abs() + half * 2.0,
                }
            }
            DrawnPart::Triangle(t) => {
                let x0 = t[0][0].min(t[1][0]).min(t[2][0]);
                let y0 = t[0][1].min(t[1][1]).min(t[2][1]);
                let x1 = t[0][0].max(t[1][0]).max(t[2][0]);
                let y1 = t[0][1].max(t[1][1]).max(t[2][1]);
                RectF {
                    x: x0,
                    y: y0,
                    width: x1 - x0,
                    height: y1 - y0,
                }
            }
        }
    }

    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        let p = |p: [f32; 2]| [p[0] + dx, p[1] + dy];
        match *self {
            DrawnPart::Text(r) => DrawnPart::Text(r.translated(dx, dy)),
            DrawnPart::Rect(r) => DrawnPart::Rect(r.translated(dx, dy)),
            DrawnPart::Segment { a, b, width } => DrawnPart::Segment {
                a: p(a),
                b: p(b),
                width,
            },
            DrawnPart::Triangle(t) => DrawnPart::Triangle([p(t[0]), p(t[1]), p(t[2])]),
        }
    }

    /// The part at `scale` times its size about the origin.
    pub fn scaled(&self, scale: f32) -> Self {
        let p = |p: [f32; 2]| [p[0] * scale, p[1] * scale];
        match *self {
            DrawnPart::Text(r) => DrawnPart::Text(r.scaled(scale)),
            DrawnPart::Rect(r) => DrawnPart::Rect(r.scaled(scale)),
            DrawnPart::Segment { a, b, width } => DrawnPart::Segment {
                a: p(a),
                b: p(b),
                width: width * scale,
            },
            DrawnPart::Triangle(t) => DrawnPart::Triangle([p(t[0]), p(t[1]), p(t[2])]),
        }
    }
}

/// A sink that hands every part on as a [`DrawnPart`], the slot dropped:
/// the adapter the geometry sinks and tests are built on.
pub struct Parts<'a>(pub &'a mut dyn FnMut(DrawnPart));

impl Sink for Parts<'_> {
    fn text(&mut self, _slot: Slot, text: &PlacedText<'_>) {
        (self.0)(DrawnPart::Text(text.rect()));
    }

    fn stroke(&mut self, _slot: Slot, a: [f32; 2], b: [f32; 2], style: &StrokeStyle<'_>) {
        (self.0)(DrawnPart::Segment {
            a,
            b,
            width: style.width,
        });
    }

    fn rect(
        &mut self,
        _slot: Slot,
        rect: RectF,
        _fill: Option<Color>,
        _border: Option<Border<'_>>,
    ) {
        (self.0)(DrawnPart::Rect(rect));
    }

    fn triangle(&mut self, _slot: Slot, points: [[f32; 2]; 3], _color: Color) {
        (self.0)(DrawnPart::Triangle(points));
    }

    fn ramp(&mut self, _slot: Slot, rect: RectF, _bar: &ColorBarOptions) {
        (self.0)(DrawnPart::Rect(rect));
    }
}

/// The union of two rectangles.
pub fn union(u: RectF, r: RectF) -> RectF {
    let (x0, y0) = (u.x.min(r.x), u.y.min(r.y));
    let x1 = (u.x + u.width).max(r.x + r.width);
    let y1 = (u.y + u.height).max(r.y + r.height);
    RectF {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}
