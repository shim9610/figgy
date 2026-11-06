//! Feynman diagrams: [`DataRenderType::Feynman`] (#234).
//!
//! A Feynman series is DataLess: every value it draws is SSOT configuration,
//! held here and serialized with the document, and none comes through a
//! `ColumnSource`. The host states the topology, which is the data: the
//! vertices, the propagators that join them, each propagator's particle and
//! each external leg's role, and every label string. A vertex's position is
//! optional. A vertex without one is placed by [`FeynmanSeries::auto_layout`],
//! the way an axis without a range is fitted by `auto_fit_*`: the layout is
//! a display parameter, written into the SSOT when it is committed, and the
//! host reads the SSOT back. Until then the series is drawn at the layout's
//! positions, computed at raster time and not stored.
//!
//! The renderer computes appearance only (rule 1): the placement of the
//! unplaced vertices, pixel positions, the stroke geometry (a wave's or a
//! coil's phase, fitted so that whole periods span each propagator), the
//! fan of parallel propagators, the arrowheads along the drawn path and the
//! label places. No physics is computed: charge, momentum conservation and
//! the allowed couplings are the host's.
//!
//! The series is drawn through the CPU sink, as the energy-level series
//! (#203) is, and the vector export writes the same emit as vector parts.
//! It has no count limit: every list is drawn to its end (rule 10).
//!
//! [`DataRenderType::Feynman`]: crate::data_config::DataRenderType::Feynman

use crate::annotation::{AnnotationAxis, AnnotationSpace};
use crate::color::Color;
use crate::data_config::{MAX_VALUE_LABEL_OFFSET, SeriesConfig};
use crate::layout::{NudgeReject, NudgeResult, RectF};
use crate::sink::{PathSegment, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use crate::text::{MeasureText, RichText};
use std::f32::consts::PI;

/// A Feynman diagram: its vertices, its propagators and their look. See the
/// [module documentation](self).
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FeynmanSeries {
    /// The vertices, in the host's order. External legs end at `Incoming`
    /// and `Outgoing` vertices.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub vertices: Vec<FeynmanVertex>,
    /// The propagators, in the host's order: the drawing order.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub propagators: Vec<Propagator>,
    /// The look of the lines, markers and arrows.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "FeynmanStyle::is_default")
    )]
    pub style: FeynmanStyle,
    /// How the automatic layout places the unplaced vertices.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "FeynmanLayout::is_default")
    )]
    pub layout: FeynmanLayout,
}

/// One vertex: an interaction point, or the free end of an external leg.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FeynmanVertex {
    /// Unique within the series; propagators name vertices by it.
    pub id: String,
    /// An interaction point or an external leg's end. It guides the layout
    /// and the default marker, nothing else.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "VertexRole::is_internal")
    )]
    pub role: VertexRole,
    /// The position in data units of the series' x and y axes; `None`
    /// places the vertex by the automatic layout.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub at: Option<[f64; 2]>,
    /// The marker; `None` draws the role's default: a dot at an internal
    /// vertex, nothing at a leg's end.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub marker: Option<VertexMarker>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub label: Option<RichText>,
    /// The label's drag offset from its drawn place, logical pixels.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "is_zero_offset")
    )]
    pub label_offset: [f32; 2],
}

impl FeynmanVertex {
    /// An unplaced vertex of `role` with no marker override and no label.
    pub fn new(id: impl Into<String>, role: VertexRole) -> Self {
        Self {
            id: id.into(),
            role,
            at: None,
            marker: None,
            label: None,
            label_offset: [0.0; 2],
        }
    }

    /// The marker drawn: the override, or the role's default.
    pub fn drawn_marker(&self) -> VertexMarker {
        self.marker.unwrap_or(match self.role {
            VertexRole::Internal => VertexMarker::Dot,
            VertexRole::Incoming | VertexRole::Outgoing => VertexMarker::Hidden,
        })
    }
}

/// What a vertex is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VertexRole {
    /// An interaction point.
    #[default]
    Internal,
    /// The start of an incoming external leg: on the early side of the
    /// time direction.
    Incoming,
    /// The end of an outgoing external leg: on the late side.
    Outgoing,
}

impl VertexRole {
    #[cfg(feature = "serde")]
    fn is_internal(&self) -> bool {
        *self == Self::Internal
    }
}

/// How a vertex is marked.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VertexMarker {
    /// No marker.
    Hidden,
    /// A filled dot of the style's `dot_radius_px`.
    Dot,
    /// A disc of `radius_px` (an effective vertex), filled with `fill` or
    /// the line colour, outlined in the line colour.
    Blob { radius_px: f32, fill: Option<Color> },
    /// A cross (an insertion).
    Cross,
    /// A circled cross (a counterterm).
    CircledCross,
}

/// One propagator: a line from vertex `from` to vertex `to`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Propagator {
    /// Unique within the series.
    pub id: String,
    /// The vertex the line starts at. A fermion's arrow points from `from`
    /// to `to`.
    pub from: String,
    /// The vertex the line ends at; `from` itself draws a loop (a tadpole).
    pub to: String,
    /// The particle, which picks the stroke.
    pub particle: ParticleLine,
    /// The bend in degrees, in screen space: the angle between the chord
    /// and the line at `from`, positive to the left of the direction of
    /// travel. `None` bends automatically: straight for a single line, a
    /// symmetric fan for the lines that join the same two vertices.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub bend: Option<f32>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub label: Option<RichText>,
    /// The label's drag offset from its drawn place, logical pixels.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "is_zero_offset")
    )]
    pub label_offset: [f32; 2],
    /// A momentum arrow drawn beside the line.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub momentum: Option<MomentumArrow>,
    /// The line colour; `None` draws the style's.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub color: Option<Color>,
    /// The line width in logical pixels; `None` draws the style's.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub width_px: Option<f32>,
}

impl Propagator {
    /// A propagator of `particle` from `from` to `to`, with no bend, label,
    /// momentum or override.
    pub fn new(
        id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        particle: ParticleLine,
    ) -> Self {
        Self {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            particle,
            bend: None,
            label: None,
            label_offset: [0.0; 2],
            momentum: None,
            color: None,
            width_px: None,
        }
    }
}

/// The particle a propagator carries: its stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ParticleLine {
    /// A plain line.
    #[default]
    Plain,
    /// A solid line with an arrow along it.
    Fermion,
    /// A solid line with an arrow against it.
    AntiFermion,
    /// A wavy line.
    Photon,
    /// A wavy line with an arrow (a W boson).
    ChargedBoson,
    /// A coil.
    Gluon,
    /// A dashed line.
    Scalar,
    /// A dashed line with an arrow.
    ChargedScalar,
    /// A dotted line with an arrow.
    Ghost,
    /// A solid line with two arrows pointing apart.
    Majorana,
    /// A double wavy line.
    Graviton,
    /// A double line (a heavy quark).
    DoubleLine,
}

impl ParticleLine {
    /// Whether the line carries a single arrow, and whether it runs
    /// against the propagator's direction.
    fn arrow(self) -> Option<bool> {
        match self {
            Self::Fermion | Self::ChargedBoson | Self::ChargedScalar | Self::Ghost => Some(false),
            Self::AntiFermion => Some(true),
            _ => None,
        }
    }

    fn dash(self) -> &'static [f32] {
        match self {
            Self::Scalar | Self::ChargedScalar => &[6.0, 4.0],
            Self::Ghost => &[1.5, 3.0],
            _ => &[],
        }
    }
}

/// A momentum arrow beside a propagator.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MomentumArrow {
    pub label: RichText,
    /// Which side of the line, facing along it from `from` to `to`.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_left"))]
    pub side: MomentumSide,
    /// Point the arrow from `to` to `from`.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub reversed: bool,
    /// The label's drag offset from its drawn place, logical pixels.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "is_zero_offset")
    )]
    pub label_offset: [f32; 2],
}

/// A side of a propagator, facing along it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MomentumSide {
    #[default]
    Left,
    Right,
}

/// The look of a Feynman series, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FeynmanStyle {
    /// The line colour, opaque: the series colour and its legend swatch.
    pub color: Color,
    /// The line width, `0.5..=24`.
    pub width: f32,
    /// A photon's and a boson's wave: amplitude `0..=64`, wavelength
    /// `2..=256`.
    pub wave: Wiggle,
    /// A gluon's coil: loop size `0..=64`, wavelength `2..=256`.
    pub coil: Wiggle,
    /// The arrowheads: length and width, `0..=64`.
    pub head_length: f32,
    pub head_width: f32,
    /// A dot's radius, `0..=64`.
    pub dot_radius: f32,
    /// A tadpole loop's radius, `2..=512`.
    pub loop_radius: f32,
    /// The angle between two lines of a fan, degrees, `0..=120`: two lines
    /// between the same vertices bend half of it each way, a bubble.
    pub fan_step: f32,
    /// How the picked part is drawn.
    pub picked: FeynmanPicked,
}

/// How the picked vertex, propagator or label of a Feynman series is drawn:
/// a line or marker in `color`, `width_extra_px` wider; a label boxed in
/// `color` at that width.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FeynmanPicked {
    /// The colour, channels in `0..=1`.
    pub color: Color,
    /// The extra width, logical pixels, `0..=16`.
    pub width_extra_px: f32,
}

impl Default for FeynmanPicked {
    /// The selection highlight colour of the document's default picked
    /// data, two pixels wider.
    fn default() -> Self {
        Self {
            color: Color::from_rgb8(255, 215, 0),
            width_extra_px: 2.0,
        }
    }
}

/// The amplitude and wavelength of a wave or a coil, logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Wiggle {
    pub amplitude: f32,
    pub wavelength: f32,
}

impl Default for FeynmanStyle {
    fn default() -> Self {
        Self {
            color: Color::from_rgb8(30, 30, 30),
            width: 1.5,
            wave: Wiggle {
                amplitude: 4.0,
                wavelength: 12.0,
            },
            coil: Wiggle {
                amplitude: 5.0,
                wavelength: 9.0,
            },
            head_length: 9.0,
            head_width: 7.0,
            dot_radius: 3.0,
            loop_radius: 18.0,
            fan_step: 90.0,
            picked: FeynmanPicked::default(),
        }
    }
}

impl FeynmanStyle {
    #[cfg(feature = "serde")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    fn validate(&self) -> Result<(), &'static str> {
        if !color_ok(self.color) || !finite_in(self.width, 0.5, 24.0) {
            return Err("invalid Feynman line style");
        }
        for wiggle in [self.wave, self.coil] {
            if !finite_in(wiggle.amplitude, 0.0, 64.0) || !finite_in(wiggle.wavelength, 2.0, 256.0)
            {
                return Err("invalid Feynman wave or coil");
            }
        }
        if !finite_in(self.head_length, 0.0, 64.0)
            || !finite_in(self.head_width, 0.0, 64.0)
            || !finite_in(self.dot_radius, 0.0, 64.0)
            || !finite_in(self.loop_radius, 2.0, 512.0)
            || !finite_in(self.fan_step, 0.0, 120.0)
        {
            return Err("invalid Feynman marker, arrow, loop or fan size");
        }
        if !color_ok(self.picked.color) || !finite_in(self.picked.width_extra_px, 0.0, 16.0) {
            return Err("invalid Feynman picked style: a colour and an extra width in 0..=16");
        }
        Ok(())
    }
}

/// The direction time runs in: where the layout puts the legs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimeDirection {
    /// Incoming legs on the left, outgoing on the right.
    #[default]
    LeftToRight,
    /// Incoming legs at the bottom, outgoing at the top.
    BottomToTop,
}

/// How the automatic layout places the unplaced vertices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FeynmanLayout {
    #[cfg_attr(feature = "serde", serde(default))]
    pub direction: TimeDirection,
}

impl FeynmanLayout {
    #[cfg(feature = "serde")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The list a Feynman label belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum FeynmanLabelPart {
    Vertex,
    Propagator,
    Momentum,
}

#[cfg(feature = "serde")]
fn is_zero_offset(offset: &[f32; 2]) -> bool {
    *offset == [0.0; 2]
}

#[cfg(feature = "serde")]
fn is_false(v: &bool) -> bool {
    !*v
}

#[cfg(feature = "serde")]
fn is_left(side: &MomentumSide) -> bool {
    *side == MomentumSide::Left
}

fn finite_in(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

fn color_ok(color: Color) -> bool {
    [color.r, color.g, color.b, color.a]
        .iter()
        .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
}

fn rich_ok(text: Option<&RichText>) -> bool {
    text.is_none_or(|t| t.validate_math().is_ok() && t.within_limits())
}

fn offset_ok(offset: [f32; 2]) -> bool {
    offset
        .iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_VALUE_LABEL_OFFSET)
}

// Working memory (#234, rule 3).
//
// Every operation on a series (validate, layout, emit, pick, drag) works in
// at most four transient buffers, each taken from the allocator once at its
// exact size by `exact` and dropped when the operation returns: the
// topology arena (`Topology`), the position buffer (`Placed::px`, or the
// list `positions` returns), the layout's conjugate-gradient work buffer
// (`lay_out`) and the emit's path buffer. No loop over vertices,
// propagators, solver iterations or path samples allocates. Rows are 32-bit
// indices: the width wasm32's `usize` has, so a series whose rows do not
// fit cannot be allocated on any target, and the conversion failing is
// reported as the allocation failing.

/// The error of an operation whose working memory cannot be allocated.
pub const ALLOCATION_FAILED: &str = "the Feynman series' working memory could not be allocated";
/// The error of a layout whose solver stops converging.
pub const LAYOUT_DID_NOT_CONVERGE: &str = "the Feynman layout did not converge";

/// The widest bend of a line, degrees: a propagator's `bend` and the outer
/// lines of a fan stay within it, so every arc has a finite radius.
const MAX_BEND: f32 = 170.0;
/// The reach of a vertex's grab, logical pixels: within it the vertex wins
/// the pick over the lines that meet it, so a leg end without a marker can
/// be picked and dragged.
const VERTEX_GRAB: f32 = 6.0;
/// An id that names no vertex.
const NONE: u32 = u32::MAX;
/// The conjugate-gradient tolerance: the relative residual it solves to.
const CG_TOLERANCE_SQUARED: f64 = 1e-24;
/// Solver iterations without a new best residual after which the layout
/// reports that it did not converge: a numerical setting, not a count of
/// data (rule 10).
const CG_STALL: u32 = 64;

/// `len` copies of `fill` in one allocation of exactly that size.
fn exact<T: Copy>(len: usize, fill: T) -> Result<Vec<T>, &'static str> {
    let mut v: Vec<T> = Vec::new();
    v.try_reserve_exact(len).map_err(|_| ALLOCATION_FAILED)?;
    v.resize(len, fill);
    Ok(v)
}

/// A row as a 32-bit index ([`NONE`] excluded).
fn row32(n: usize) -> Result<u32, &'static str> {
    u32::try_from(n)
        .ok()
        .filter(|&r| r != NONE)
        .ok_or(ALLOCATION_FAILED)
}

/// The vertex row `order` (rows sorted by id) names for `id`, or [`NONE`].
fn lookup(series: &FeynmanSeries, order: &[u32], id: &str) -> u32 {
    order
        .binary_search_by(|&r| series.vertices[r as usize].id.as_str().cmp(id))
        .map_or(NONE, |k| order[k])
}

/// The topology of a series, worked out once per operation in
/// O(V log V + E log D) (D the largest degree): one exact-size allocation
/// of 32-bit words, `[order | offsets | incident | ends | flags]`.
///
/// - `order` (V): the vertex rows sorted by id, the id index. Once the
///   ends are resolved the layout reuses it as its stack and its sort
///   scratch.
/// - `offsets` (V + 1) and `incident` (4E, `(other end, propagator)`
///   pairs): the CSR adjacency. Each vertex's pairs are sorted by
///   `(other end, bent, propagator)`, so the unbent lines between two
///   vertices (a fan) and the loops at a vertex are runs found by binary
///   search.
/// - `ends` (2E): each propagator's vertex rows, [`NONE`] for an id that
///   names no vertex.
/// - `flags` (V): the layout's state of each vertex.
pub(crate) struct Topology {
    words: Vec<u32>,
    v: usize,
    e: usize,
}

/// The regions of a topology arena: `(order, offsets, incident, ends, flags)`.
type Regions<'a> = (
    &'a mut [u32],
    &'a mut [u32],
    &'a mut [u32],
    &'a mut [u32],
    &'a mut [u32],
);

const FIXED: u32 = 1;
const ANCHORED: u32 = 2;

impl Topology {
    fn new(series: &FeynmanSeries) -> Result<Self, &'static str> {
        let (v, e) = (series.vertices.len(), series.propagators.len());
        row32(v)?;
        row32(e)?;
        let len = v
            .checked_mul(3)
            .and_then(|n| n.checked_add(1))
            .and_then(|n| n.checked_add(e.checked_mul(6)?))
            .ok_or(ALLOCATION_FAILED)?;
        let mut topology = Self {
            words: exact(len, 0u32)?,
            v,
            e,
        };
        let (order, offsets, incident, ends, _) = topology.parts_mut();
        for (i, slot) in order.iter_mut().enumerate() {
            *slot = i as u32;
        }
        order.sort_unstable_by(|&a, &b| {
            series.vertices[a as usize]
                .id
                .cmp(&series.vertices[b as usize].id)
        });
        for (p, propagator) in series.propagators.iter().enumerate() {
            ends[2 * p] = lookup(series, order, &propagator.from);
            ends[2 * p + 1] = lookup(series, order, &propagator.to);
        }
        // Degrees into offsets[0..v], an inclusive prefix sum (each entry
        // the end of its vertex's run), then a fill that decrements each
        // entry to the start of its run: no fill cursor is allocated.
        for p in 0..e {
            let (a, b) = (ends[2 * p], ends[2 * p + 1]);
            if a == NONE || b == NONE {
                continue;
            }
            offsets[a as usize] += 1;
            if a != b {
                offsets[b as usize] += 1;
            }
        }
        for i in 1..v {
            offsets[i] += offsets[i - 1];
        }
        offsets[v] = if v > 0 { offsets[v - 1] } else { 0 };
        for p in (0..e).rev() {
            let (a, b) = (ends[2 * p], ends[2 * p + 1]);
            if a == NONE || b == NONE {
                continue;
            }
            offsets[a as usize] -= 1;
            let k = offsets[a as usize] as usize;
            incident[2 * k] = b;
            incident[2 * k + 1] = p as u32;
            if a != b {
                offsets[b as usize] -= 1;
                let k = offsets[b as usize] as usize;
                incident[2 * k] = a;
                incident[2 * k + 1] = p as u32;
            }
        }
        for i in 0..v {
            let run = &mut incident[2 * offsets[i] as usize..2 * offsets[i + 1] as usize];
            pairs_mut(run).sort_unstable_by_key(|&[other, p]| pair_key(series, i as u32, other, p));
        }
        Ok(topology)
    }

    fn parts_mut(&mut self) -> Regions<'_> {
        let (order, rest) = self.words.split_at_mut(self.v);
        let (offsets, rest) = rest.split_at_mut(self.v + 1);
        let (incident, rest) = rest.split_at_mut(4 * self.e);
        let (ends, flags) = rest.split_at_mut(2 * self.e);
        (order, offsets, incident, ends, flags)
    }

    fn offsets(&self) -> &[u32] {
        &self.words[self.v..2 * self.v + 1]
    }

    fn incident(&self) -> &[u32] {
        let start = 2 * self.v + 1;
        &self.words[start..start + 4 * self.e]
    }

    fn ends(&self) -> &[u32] {
        let start = 2 * self.v + 1 + 4 * self.e;
        &self.words[start..start + 2 * self.e]
    }

    /// The `(other end, propagator)` pairs of vertex `i`.
    fn around(&self, i: usize) -> &[[u32; 2]] {
        around(self.offsets(), self.incident(), i)
    }

    /// Propagator `p`'s vertex rows, `None` when an id names no vertex.
    fn ends_of(&self, p: usize) -> Option<(usize, usize)> {
        let ends = self.ends();
        let (a, b) = (*ends.get(2 * p)?, *ends.get(2 * p + 1)?);
        (a != NONE && b != NONE).then_some((a as usize, b as usize))
    }

    /// The run of vertex `i`'s pairs whose other end is `other` and whose
    /// bent flag is `bent`, sorted by propagator.
    fn run(&self, series: &FeynmanSeries, i: usize, other: u32, bent: bool) -> &[[u32; 2]] {
        let pairs = self.around(i);
        let key = |&[o, p]: &[u32; 2]| pair_key(series, i as u32, o, p);
        let start = pairs.partition_point(|pair| key(pair) < (other, bent, 0));
        let end = pairs.partition_point(|pair| key(pair) <= (other, bent, u32::MAX));
        &pairs[start..end]
    }

    /// The rank of the unbent line `p` in its fan, the fan's size, and
    /// whether `p` runs from the fan's lower row.
    fn fan(&self, series: &FeynmanSeries, p: usize) -> Option<(usize, usize, bool)> {
        let (a, b) = self.ends_of(p)?;
        if a == b {
            return None;
        }
        let (low, high) = (a.min(b), a.max(b));
        let run = self.run(series, low, high as u32, false);
        let rank = run.binary_search_by_key(&(p as u32), |pair| pair[1]).ok()?;
        Some((rank, run.len(), a == low))
    }

    /// The rank of the loop `p` among the loops at its vertex, and their
    /// number.
    fn loop_rank(&self, series: &FeynmanSeries, p: usize) -> Option<(usize, usize)> {
        let (a, _) = self.ends_of(p)?;
        let run = self.run(series, a, a as u32, false);
        let rank = run.binary_search_by_key(&(p as u32), |pair| pair[1]).ok()?;
        Some((rank, run.len()))
    }
}

/// The `(other end, propagator)` pairs of vertex `i` in a CSR adjacency.
fn around<'a>(offsets: &[u32], incident: &'a [u32], i: usize) -> &'a [[u32; 2]] {
    let words = &incident[2 * offsets[i] as usize..2 * offsets[i + 1] as usize];
    let (pairs, _) = words.as_chunks::<2>();
    pairs
}

fn pairs_mut(words: &mut [u32]) -> &mut [[u32; 2]] {
    let (pairs, _) = words.as_chunks_mut::<2>();
    pairs
}

/// The sort key of the pair `(other, p)` at vertex `i`: the other end, then
/// unbent lines before bent ones (a loop counts as unbent), then the row.
fn pair_key(series: &FeynmanSeries, i: u32, other: u32, p: u32) -> (u32, bool, u32) {
    let bent = other != i && series.propagators[p as usize].bend.is_some();
    (other, bent, p)
}

/// The box of `points`, `None` for none.
fn bounding_box(points: impl Iterator<Item = [f64; 2]>) -> Option<[[f64; 2]; 2]> {
    points.fold(None, |acc, p| {
        Some(match acc {
            None => [p, p],
            Some([lo, hi]) => [
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            ],
        })
    })
}

/// Fill `positions` (one per vertex) with every vertex's `at` and the
/// automatic layout's place for every vertex without one
/// ([`FeynmanSeries::positions`]). Works in `topology`'s `order` and
/// `flags` and, when some vertex is solved for, in one conjugate-gradient
/// work buffer of `3 V` entries, dropped on return.
fn lay_out(
    series: &FeynmanSeries,
    topology: &mut Topology,
    positions: &mut [[f64; 2]],
) -> Result<(), &'static str> {
    let n = series.vertices.len();
    let (order, offsets, incident, _, flags) = topology.parts_mut();
    let (offsets, incident) = (&*offsets, &*incident);
    let nbrs = |i: usize| around(offsets, incident, i);
    for ((vertex, p), flag) in series
        .vertices
        .iter()
        .zip(positions.iter_mut())
        .zip(&mut *flags)
    {
        *p = vertex.at.unwrap_or([0.0; 2]);
        *flag = if vertex.at.is_some() { FIXED } else { 0 };
    }
    // The frame: the placed vertices' box, or the unit square.
    let frame = match bounding_box(series.vertices.iter().filter_map(|v| v.at)) {
        Some([lo, hi]) if hi[0] > lo[0] && hi[1] > lo[1] => [lo, hi],
        Some([lo, hi]) if hi[0] > lo[0] || hi[1] > lo[1] => {
            let size = (hi[0] - lo[0]).max(hi[1] - lo[1]);
            let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
            [
                [c[0] - size / 2.0, c[1] - size / 2.0],
                [c[0] + size / 2.0, c[1] + size / 2.0],
            ]
        }
        Some([lo, _]) => [lo, [lo[0] + 1.0, lo[1] + 1.0]],
        None => [[0.0, 0.0], [1.0, 1.0]],
    };
    let at = |time: f64, across: f64| -> [f64; 2] {
        let (x, y) = match series.layout.direction {
            TimeDirection::LeftToRight => (time, across),
            TimeDirection::BottomToTop => (across, time),
        };
        [
            frame[0][0] + x * (frame[1][0] - frame[0][0]),
            frame[0][1] + y * (frame[1][1] - frame[0][1]),
        ]
    };
    // The legs on the frame's early and late edges, in row order.
    for (role, time) in [(VertexRole::Incoming, 0.0), (VertexRole::Outgoing, 1.0)] {
        let is_leg = |v: &FeynmanVertex| v.role == role && v.at.is_none();
        let count = series.vertices.iter().filter(|v| is_leg(v)).count();
        let mut k = 0usize;
        for (i, vertex) in series.vertices.iter().enumerate() {
            if is_leg(vertex) {
                // Top to bottom on screen: the first leg is at the top.
                let across = 1.0 - (k as f64 + 1.0) / (count as f64 + 1.0);
                positions[i] = at(time, across);
                flags[i] = FIXED;
                k += 1;
            }
        }
    }
    // The vertices a fixed one reaches, with `order` as the stack: each
    // vertex is pushed once, when it is marked.
    let mut top = 0usize;
    for i in 0..n {
        if flags[i] & FIXED != 0 {
            flags[i] |= ANCHORED;
            order[top] = i as u32;
            top += 1;
        }
    }
    while top > 0 {
        top -= 1;
        let i = order[top] as usize;
        for &[j, _] in nbrs(i) {
            let j = j as usize;
            if flags[j] & ANCHORED == 0 {
                flags[j] |= ANCHORED;
                order[top] = j as u32;
                top += 1;
            }
        }
    }
    // The barycentric embedding of the anchored free vertices.
    if flags.contains(&ANCHORED) {
        solve_barycentric(&nbrs, flags, positions)?;
    }
    // The bubbles: each connected group on a circle about the frame's
    // centre, in row order, collected in `order`.
    let centre = at(0.5, 0.5);
    let radius = 0.25 * (frame[1][0] - frame[0][0]).min(frame[1][1] - frame[0][1]);
    let mut group = 0usize;
    for start in 0..n {
        if flags[start] & ANCHORED != 0 {
            continue;
        }
        flags[start] |= ANCHORED;
        order[0] = start as u32;
        let (mut m, mut k) = (1usize, 0usize);
        while k < m {
            for &[j, _] in nbrs(order[k] as usize) {
                let j = j as usize;
                if flags[j] & ANCHORED == 0 {
                    flags[j] |= ANCHORED;
                    order[m] = j as u32;
                    m += 1;
                }
            }
            k += 1;
        }
        let members = &mut order[..m];
        members.sort_unstable();
        let shift = 0.1 * group as f64 * (frame[1][0] - frame[0][0]);
        for (k, &i) in members.iter().enumerate() {
            let angle = std::f64::consts::TAU * k as f64 / m as f64;
            positions[i as usize] = [
                centre[0] + shift + radius * angle.cos(),
                centre[1] + radius * angle.sin(),
            ];
        }
        group += 1;
    }
    spread_coincident(positions, flags, order, series.layout.direction, frame);
    Ok(())
}

/// Solve `L_ff x = L_fp p` for the free vertices (flag exactly
/// [`ANCHORED`]) by conjugate gradients, both axes in one pass over the
/// adjacency, into `positions`. One work buffer of `3 V` entries holds the
/// residual, the direction and the product (`r | p | Ap`); `positions` is
/// the iterate. Ends at a relative residual of 1e-12 on both axes; an
/// iteration count of [`CG_STALL`] without a new best residual, or a
/// non-finite value, is [`LAYOUT_DID_NOT_CONVERGE`].
fn solve_barycentric<'a>(
    nbrs: &dyn Fn(usize) -> &'a [[u32; 2]],
    flags: &[u32],
    positions: &mut [[f64; 2]],
) -> Result<(), &'static str> {
    let n = flags.len();
    let free = |i: usize| flags[i] == ANCHORED;
    let mut work = exact(3 * n, [0.0f64; 2])?;
    let (r, rest) = work.split_at_mut(n);
    let (p, ap) = rest.split_at_mut(n);
    let mut rr = [0.0f64; 2];
    for i in 0..n {
        if !free(i) {
            continue;
        }
        positions[i] = [0.0; 2];
        let mut b = [0.0f64; 2];
        for &[j, _] in nbrs(i) {
            let j = j as usize;
            if j != i && !free(j) {
                b[0] += positions[j][0];
                b[1] += positions[j][1];
            }
        }
        r[i] = b;
        p[i] = b;
        rr[0] += b[0] * b[0];
        rr[1] += b[1] * b[1];
    }
    let target = [rr[0] * CG_TOLERANCE_SQUARED, rr[1] * CG_TOLERANCE_SQUARED];
    let mut best = rr;
    let mut stalled = [0u32; 2];
    loop {
        let active = [rr[0] > target[0], rr[1] > target[1]];
        if !active[0] && !active[1] {
            return Ok(());
        }
        let mut pap = [0.0f64; 2];
        for i in 0..n {
            if !free(i) {
                continue;
            }
            let mut degree = 0.0f64;
            let mut sum = [0.0f64; 2];
            for &[j, _] in nbrs(i) {
                let j = j as usize;
                if j == i {
                    continue;
                }
                degree += 1.0;
                if free(j) {
                    sum[0] += p[j][0];
                    sum[1] += p[j][1];
                }
            }
            ap[i] = [degree * p[i][0] - sum[0], degree * p[i][1] - sum[1]];
            pap[0] += p[i][0] * ap[i][0];
            pap[1] += p[i][1] * ap[i][1];
        }
        let mut alpha = [0.0f64; 2];
        for axis in 0..2 {
            if active[axis] {
                if !(pap[axis] > 0.0 && pap[axis].is_finite()) {
                    return Err(LAYOUT_DID_NOT_CONVERGE);
                }
                alpha[axis] = rr[axis] / pap[axis];
            }
        }
        let mut next = rr;
        for (axis, on) in active.iter().enumerate() {
            if *on {
                next[axis] = 0.0;
            }
        }
        for i in 0..n {
            if !free(i) {
                continue;
            }
            for axis in 0..2 {
                if active[axis] {
                    positions[i][axis] += alpha[axis] * p[i][axis];
                    r[i][axis] -= alpha[axis] * ap[i][axis];
                    next[axis] += r[i][axis] * r[i][axis];
                }
            }
        }
        let mut beta = [0.0f64; 2];
        for axis in 0..2 {
            if !active[axis] {
                continue;
            }
            if !next[axis].is_finite() {
                return Err(LAYOUT_DID_NOT_CONVERGE);
            }
            beta[axis] = next[axis] / rr[axis];
            rr[axis] = next[axis];
            if next[axis] < best[axis] {
                best[axis] = next[axis];
                stalled[axis] = 0;
            } else {
                stalled[axis] += 1;
                if stalled[axis] >= CG_STALL {
                    return Err(LAYOUT_DID_NOT_CONVERGE);
                }
            }
        }
        for i in 0..n {
            if !free(i) {
                continue;
            }
            for axis in 0..2 {
                if active[axis] {
                    p[i][axis] = r[i][axis] + beta[axis] * p[i][axis];
                }
            }
        }
    }
}

/// Spread the unpinned vertices that share a point across the time
/// direction, in row order, a tenth of the frame apart. Sorts the rows in
/// `order` (rows tie-broken by row, so each run is in row order).
fn spread_coincident(
    positions: &mut [[f64; 2]],
    flags: &[u32],
    order: &mut [u32],
    direction: TimeDirection,
    frame: [[f64; 2]; 2],
) {
    let size = [frame[1][0] - frame[0][0], frame[1][1] - frame[0][1]];
    let tolerance = 1e-9 * size[0].max(size[1]);
    for (i, slot) in order.iter_mut().enumerate() {
        *slot = i as u32;
    }
    order.sort_unstable_by(|&a, &b| {
        let (pa, pb) = (positions[a as usize], positions[b as usize]);
        pa[0]
            .total_cmp(&pb[0])
            .then(pa[1].total_cmp(&pb[1]))
            .then(a.cmp(&b))
    });
    let mut k = 0;
    while k < order.len() {
        let first = positions[order[k] as usize];
        let mut end = k + 1;
        while end < order.len()
            && (positions[order[end] as usize][0] - first[0]).abs() <= tolerance
            && (positions[order[end] as usize][1] - first[1]).abs() <= tolerance
        {
            end += 1;
        }
        let mut moved = 0;
        for &i in &order[k..end] {
            let i = i as usize;
            if flags[i] & FIXED != 0 {
                continue;
            }
            if moved > 0 {
                let step = 0.1 * moved as f64;
                match direction {
                    TimeDirection::LeftToRight => positions[i][1] += step * size[1],
                    TimeDirection::BottomToTop => positions[i][0] += step * size[0],
                }
            }
            moved += 1;
        }
        k = end;
    }
}

impl FeynmanSeries {
    /// The rules a series is held to on its own; the chart's rules are
    /// [`Config::validate_feynman`](crate::config::Config::validate_feynman)'s.
    ///
    /// Ids are unique per list; every propagator names vertices of the
    /// series; positions are finite; bends lie in `-170..=170` degrees;
    /// labels are valid rich text within the per-string limit; offsets and
    /// the style are in range. No list has a count limit (rule 10). Works
    /// in one transient list of `V + E` rows.
    pub fn validate(&self) -> Result<(), &'static str> {
        self.style.validate()?;
        let (v, e) = (self.vertices.len(), self.propagators.len());
        row32(v)?;
        row32(e)?;
        let mut rows = exact(v.checked_add(e).ok_or(ALLOCATION_FAILED)?, 0u32)?;
        let (order, propagator_order) = rows.split_at_mut(v);
        for (i, slot) in order.iter_mut().enumerate() {
            *slot = i as u32;
        }
        order.sort_unstable_by(|&a, &b| {
            self.vertices[a as usize]
                .id
                .cmp(&self.vertices[b as usize].id)
        });
        if order
            .windows(2)
            .any(|w| self.vertices[w[0] as usize].id == self.vertices[w[1] as usize].id)
        {
            return Err("Feynman vertex ids must be unique");
        }
        for (i, slot) in propagator_order.iter_mut().enumerate() {
            *slot = i as u32;
        }
        propagator_order.sort_unstable_by(|&a, &b| {
            self.propagators[a as usize]
                .id
                .cmp(&self.propagators[b as usize].id)
        });
        if propagator_order
            .windows(2)
            .any(|w| self.propagators[w[0] as usize].id == self.propagators[w[1] as usize].id)
        {
            return Err("Feynman propagator ids must be unique");
        }
        for vertex in &self.vertices {
            if vertex
                .at
                .is_some_and(|p| !p[0].is_finite() || !p[1].is_finite())
            {
                return Err("a Feynman vertex position must be finite");
            }
            if let Some(VertexMarker::Blob { radius_px, fill }) = vertex.marker
                && (!finite_in(radius_px, 0.0, 256.0) || fill.is_some_and(|c| !color_ok(c)))
            {
                return Err("invalid Feynman blob marker");
            }
            if !rich_ok(vertex.label.as_ref()) || !offset_ok(vertex.label_offset) {
                return Err("invalid Feynman vertex label");
            }
        }
        for propagator in &self.propagators {
            if lookup(self, order, &propagator.from) == NONE
                || lookup(self, order, &propagator.to) == NONE
            {
                return Err("a propagator must name two vertices of its series");
            }
            if propagator
                .bend
                .is_some_and(|b| !finite_in(b, -MAX_BEND, MAX_BEND))
            {
                return Err("a propagator's bend must be in -170..=170 degrees");
            }
            if propagator.color.is_some_and(|c| !color_ok(c))
                || propagator
                    .width_px
                    .is_some_and(|w| !finite_in(w, 0.5, 24.0))
            {
                return Err("invalid propagator colour or width");
            }
            if !rich_ok(propagator.label.as_ref()) || !offset_ok(propagator.label_offset) {
                return Err("invalid propagator label");
            }
            if let Some(momentum) = &propagator.momentum
                && (!rich_ok(Some(&momentum.label)) || !offset_ok(momentum.label_offset))
            {
                return Err("invalid momentum label");
            }
        }
        Ok(())
    }

    /// The row of the vertex with id `id`. A linear scan over the SSOT: no
    /// index is kept (rule 3).
    pub fn vertex_row(&self, id: &str) -> Option<usize> {
        self.vertices.iter().position(|v| v.id == id)
    }

    /// The number of rows of `part`: the label index space of that list.
    pub fn part_len(&self, part: FeynmanLabelPart) -> usize {
        match part {
            FeynmanLabelPart::Vertex => self.vertices.len(),
            FeynmanLabelPart::Propagator | FeynmanLabelPart::Momentum => self.propagators.len(),
        }
    }
    /// The drag offset of the label of row `index` of `part`, or `None`
    /// past the end or for a propagator without a momentum arrow.
    pub fn label_offset(&self, part: FeynmanLabelPart, index: usize) -> Option<[f32; 2]> {
        match part {
            FeynmanLabelPart::Vertex => self.vertices.get(index).map(|v| v.label_offset),
            FeynmanLabelPart::Propagator => self.propagators.get(index).map(|p| p.label_offset),
            FeynmanLabelPart::Momentum => self
                .propagators
                .get(index)
                .and_then(|p| p.momentum.as_ref())
                .map(|m| m.label_offset),
        }
    }

    /// Move the label of row `index` of `part` by `(dx, dy)` logical pixels
    /// (x right, y down). Rejected for a row past the end, a missing
    /// momentum arrow, a non-finite delta or an offset past
    /// [`MAX_VALUE_LABEL_OFFSET`]; nothing changes then.
    pub fn drag_label(
        &mut self,
        part: FeynmanLabelPart,
        index: usize,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let offset = match part {
            FeynmanLabelPart::Vertex => self.vertices.get_mut(index).map(|v| &mut v.label_offset),
            FeynmanLabelPart::Propagator => {
                self.propagators.get_mut(index).map(|p| &mut p.label_offset)
            }
            FeynmanLabelPart::Momentum => self
                .propagators
                .get_mut(index)
                .and_then(|p| p.momentum.as_mut())
                .map(|m| &mut m.label_offset),
        };
        let Some(offset) = offset else {
            return reject;
        };
        let moved = [offset[0] + dx, offset[1] + dy];
        if !dx.is_finite() || !dy.is_finite() || !offset_ok(moved) {
            return reject;
        }
        *offset = moved;
        NudgeResult::Moved
    }

    /// Whether some vertex has no position: the layout places it.
    pub fn has_unplaced(&self) -> bool {
        self.vertices.iter().any(|v| v.at.is_none())
    }

    /// Commit the automatic layout: write the position of every vertex
    /// without one into its `at` ([`Self::positions`]). Placed vertices are
    /// untouched. Returns whether anything was written. The auto-fit
    /// pattern: a display parameter written into the SSOT on an explicit
    /// call, which the host reads back. Allocates nothing when every
    /// vertex is placed.
    pub fn auto_layout(&mut self) -> Result<bool, &'static str> {
        if !self.has_unplaced() {
            return Ok(false);
        }
        let positions = self.positions()?;
        for (vertex, p) in self.vertices.iter_mut().zip(positions) {
            if vertex.at.is_none() {
                vertex.at = Some(p);
            }
        }
        Ok(true)
    }

    /// The position of every vertex, in row order: its `at`, or the
    /// automatic layout's place for it. Deterministic: identical SSOT gives
    /// bit-identical positions. The list is the caller's; it is the one
    /// allocation when every vertex is placed, and the layout adds its
    /// topology and solver buffers, dropped on return.
    ///
    /// The layout pins every placed vertex. It spaces the unplaced incoming
    /// legs evenly on the early edge of the frame and the outgoing legs on
    /// the late edge, in row order, and places every other unplaced vertex
    /// at the mean of its neighbours (the barycentric, or Tutte, embedding,
    /// solved by conjugate gradients), so a planar diagram with its legs on
    /// the boundary is drawn without crossings. A group of vertices that no
    /// placed vertex or leg reaches (a vacuum bubble) is placed on a
    /// circle. Vertices that land on one point are spread across the time
    /// direction. The frame is the bounding box of the placed vertices, or
    /// the unit square when fewer than two span one.
    pub fn positions(&self) -> Result<Vec<[f64; 2]>, &'static str> {
        let mut positions = exact(self.vertices.len(), [0.0f64; 2])?;
        self.fill_positions(None, &mut positions)?;
        Ok(positions)
    }

    /// Fill `positions` with [`Self::positions`]: the `at` of each vertex,
    /// or the layout, in `topology` or in one built for it.
    fn fill_positions(
        &self,
        topology: Option<&mut Topology>,
        positions: &mut [[f64; 2]],
    ) -> Result<(), &'static str> {
        if !self.has_unplaced() {
            for (p, vertex) in positions.iter_mut().zip(&self.vertices) {
                *p = vertex.at.unwrap_or_default();
            }
            return Ok(());
        }
        match topology {
            Some(topology) => lay_out(self, topology, positions),
            None => lay_out(self, &mut Topology::new(self)?, positions),
        }
    }

    /// The bounding box of every vertex position, `[[x0, y0], [x1, y1]]`,
    /// or `None` without vertices: what an axis fit reads (a display
    /// parameter, rule 1). Allocates nothing when every vertex is placed.
    pub fn extent(&self) -> Result<Option<[[f64; 2]; 2]>, &'static str> {
        if self.has_unplaced() {
            Ok(bounding_box(self.positions()?.into_iter()))
        } else {
            Ok(bounding_box(self.vertices.iter().filter_map(|v| v.at)))
        }
    }

    /// Move vertex `index` by `(dx, dy)` screen pixels in `space` (the
    /// panel's anchor space, logical pixels): its position is converted
    /// back through the axes and written into `at`. The first drag of a
    /// diagram with unplaced vertices commits the whole layout first, so
    /// no other vertex moves. Rejected for a row past the end, a non-finite
    /// delta, a layout that fails or a place the axes cannot convert
    /// (outside a logarithmic domain, in a break gap); nothing changes
    /// then. Allocates nothing when every vertex is placed.
    pub fn drag_vertex(
        &mut self,
        index: usize,
        dx: f32,
        dy: f32,
        space: &dyn AnnotationSpace,
    ) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        if !dx.is_finite() || !dy.is_finite() || index >= self.vertices.len() {
            return reject;
        }
        let layout = if self.has_unplaced() {
            match self.positions() {
                Ok(positions) => Some(positions),
                Err(_) => return reject,
            }
        } else {
            None
        };
        let p = match &layout {
            Some(positions) => positions[index],
            None => self.vertices[index].at.unwrap_or_default(),
        };
        let (Some(x), Some(y)) = (
            space.value_to_px(AnnotationAxis::X, p[0]),
            space.value_to_px(AnnotationAxis::Y, p[1]),
        ) else {
            return reject;
        };
        let (Some(vx), Some(vy)) = (
            space.px_to_value(AnnotationAxis::X, x + dx),
            space.px_to_value(AnnotationAxis::Y, y + dy),
        ) else {
            return reject;
        };
        if !vx.is_finite() || !vy.is_finite() {
            return reject;
        }
        if let Some(positions) = layout {
            for (vertex, p) in self.vertices.iter_mut().zip(positions) {
                if vertex.at.is_none() {
                    vertex.at = Some(p);
                }
            }
        }
        self.vertices[index].at = Some([vx, vy]);
        NudgeResult::Moved
    }

    /// Bend propagator `index` by dragging its middle `(dx, dy)` screen
    /// pixels in `space`: the drag across the chord sets `bend` to the arc
    /// through both ends and the moved middle. Rejected for a row past the
    /// end, a loop, a non-finite delta, a line whose ends are not placed on
    /// screen, a layout that fails or a bend past 170 degrees; nothing
    /// changes then.
    pub fn drag_bend(
        &mut self,
        index: usize,
        dx: f32,
        dy: f32,
        space: &dyn AnnotationSpace,
    ) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        if !dx.is_finite() || !dy.is_finite() {
            return reject;
        }
        let Ok(placed) = self.placed(space, None) else {
            return reject;
        };
        let Some(geometry) = placed.geometry(index) else {
            return reject;
        };
        drop(placed);
        let (a, b) = geometry.ends;
        if geometry.is_loop {
            return reject;
        }
        let chord = sub(b, a);
        let c = len(chord);
        if c <= 0.0 {
            return reject;
        }
        let normal = left_normal(scale(chord, 1.0 / c));
        let middle = geometry.base.point(0.5);
        let sagitta = dot(sub(add(middle, [dx, dy]), lerp(a, b, 0.5)), normal);
        let bend = (2.0 * (2.0 * sagitta / c).atan()).to_degrees();
        if !finite_in(bend, -MAX_BEND, MAX_BEND) {
            return reject;
        }
        self.propagators[index].bend = Some(bend);
        NudgeResult::Moved
    }

    /// Place the series in `space` with `picked` drawn picked: the
    /// topology and every vertex's screen point, the two buffers an emit,
    /// a pick and a bend drag work in (plus the layout's, while it runs).
    pub(crate) fn placed<'s>(
        &'s self,
        space: &dyn AnnotationSpace,
        picked: Option<FeynmanHit>,
    ) -> Result<Placed<'s>, &'static str> {
        let mut topology = Topology::new(self)?;
        let mut px = exact(self.vertices.len(), [f64::NAN; 2])?;
        self.fill_positions(Some(&mut topology), &mut px)?;
        for p in &mut px {
            *p = match (
                space.value_to_px(AnnotationAxis::X, p[0]),
                space.value_to_px(AnnotationAxis::Y, p[1]),
            ) {
                (Some(x), Some(y)) => [f64::from(x), f64::from(y)],
                _ => [f64::NAN; 2],
            };
        }
        Ok(Placed {
            series: self,
            topology,
            px,
            picked,
        })
    }

    /// Emit every drawn part into `sink` in the [`Slot::Above`] slot, in
    /// `space` (the panel's anchor space, logical pixels): the propagators
    /// in row order, the vertex markers over them, then every label. The
    /// part `picked` is drawn in `style.picked`: its colour, that much
    /// wider, a picked label boxed. Nothing is stored; the sink paints, bounds or
    /// hit tests the parts. Works in the placement's two buffers and one
    /// path buffer reused by every line.
    pub fn emit(
        &self,
        space: &dyn AnnotationSpace,
        picked: Option<FeynmanHit>,
        measure: &dyn MeasureText,
        sink: &mut dyn Sink,
    ) -> Result<(), &'static str> {
        let placed = self.placed(space, picked)?;
        let mut path: Vec<PathSegment> = Vec::new();
        for index in 0..self.propagators.len() {
            placed.emit_propagator(index, &mut path, sink)?;
        }
        for index in 0..self.vertices.len() {
            placed.emit_vertex(index, &mut path, sink)?;
        }
        for part in LABEL_PARTS {
            for index in 0..self.part_len(part) {
                let Some(text) = placed.label(part, index, measure) else {
                    continue;
                };
                if picked == Some(FeynmanHit::Label(part, index)) {
                    placed.emit_label_box(&text, sink);
                }
                sink.text(Slot::Above, &text);
            }
        }
        Ok(())
    }

    /// The drawn part nearest `cursor` (logical pixels in `space`), within
    /// `max_distance` of its ink, and that distance (zero on the part).
    /// Walks the SSOT through the same placement the emit draws with, so
    /// the pick is the drawn geometry; nothing is registered per row (rule
    /// 3). Within six pixels of a vertex the vertex wins over its
    /// lines; on a tie a part drawn later wins: labels over markers over
    /// lines. The series is drawn clipped to the plot, so a cursor outside
    /// `space`'s plot rectangle picks nothing.
    pub fn pick(
        &self,
        space: &dyn AnnotationSpace,
        measure: &dyn MeasureText,
        cursor: [f32; 2],
        max_distance: f32,
    ) -> Result<Option<(FeynmanHit, f32)>, &'static str> {
        let plot = space.plot_rect();
        if !(plot.x..=plot.x + plot.width).contains(&cursor[0])
            || !(plot.y..=plot.y + plot.height).contains(&cursor[1])
        {
            return Ok(None);
        }
        let placed = self.placed(space, None)?;
        let mut best: Option<(FeynmanHit, f32)> = None;
        let mut offer = |hit: FeynmanHit, distance: f32| {
            if distance <= max_distance && best.is_none_or(|(_, d)| distance <= d) {
                best = Some((hit, distance));
            }
        };
        for index in 0..self.propagators.len() {
            if let Some(geometry) = placed.geometry(index) {
                let edge = geometry.base.distance(cursor) - placed.envelope(index);
                offer(FeynmanHit::Propagator(index), edge.max(0.0));
            }
        }
        for index in 0..self.vertices.len() {
            if let Some(p) = placed.px(index) {
                let reach = placed.marker_radius(index).max(VERTEX_GRAB);
                offer(
                    FeynmanHit::Vertex(index),
                    (len(sub(cursor, p)) - reach).max(0.0),
                );
            }
        }
        for part in LABEL_PARTS {
            for index in 0..self.part_len(part) {
                if let Some(text) = placed.label(part, index, measure) {
                    offer(
                        FeynmanHit::Label(part, index),
                        rect_distance(cursor, text.rect()),
                    );
                }
            }
        }
        Ok(best)
    }
}

/// The label lists in drawing order.
const LABEL_PARTS: [FeynmanLabelPart; 3] = [
    FeynmanLabelPart::Propagator,
    FeynmanLabelPart::Momentum,
    FeynmanLabelPart::Vertex,
];

// Screen geometry: points are `[x, y]` logical pixels, y down.

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn scale(a: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] * s, a[1] * s]
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn len(a: [f32; 2]) -> f32 {
    a[0].hypot(a[1])
}

fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn unit(a: [f32; 2]) -> [f32; 2] {
    let l = len(a);
    if l > 0.0 {
        scale(a, 1.0 / l)
    } else {
        [1.0, 0.0]
    }
}

/// The left normal of the direction `d` on screen (y down): `(1, 0)` turns
/// to `(0, -1)`, up.
fn left_normal(d: [f32; 2]) -> [f32; 2] {
    [d[1], -d[0]]
}

/// The path a propagator's stroke follows: a straight line or a circular
/// arc, parametrized by `t` in `0..=1` at a constant speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Base {
    Line {
        a: [f32; 2],
        b: [f32; 2],
    },
    Arc {
        centre: [f32; 2],
        radius: f32,
        start: f32,
        sweep: f32,
    },
}

impl Base {
    /// The point at `t`.
    pub(crate) fn point(&self, t: f32) -> [f32; 2] {
        match *self {
            Base::Line { a, b } => lerp(a, b, t),
            Base::Arc {
                centre,
                radius,
                start,
                sweep,
            } => {
                let angle = start + sweep * t;
                [
                    centre[0] + radius * angle.cos(),
                    centre[1] + radius * angle.sin(),
                ]
            }
        }
    }

    /// The unit direction of travel at `t`.
    pub(crate) fn tangent(&self, t: f32) -> [f32; 2] {
        match *self {
            Base::Line { a, b } => unit(sub(b, a)),
            Base::Arc { start, sweep, .. } => {
                let angle = start + sweep * t;
                let s = sweep.signum();
                [-angle.sin() * s, angle.cos() * s]
            }
        }
    }

    /// The length of the path.
    pub(crate) fn length(&self) -> f32 {
        match *self {
            Base::Line { a, b } => len(sub(b, a)),
            Base::Arc { radius, sweep, .. } => radius * sweep.abs(),
        }
    }

    /// The distance from `p` to the path, sampled a few pixels apart.
    fn distance(&self, p: [f32; 2]) -> f32 {
        match *self {
            Base::Line { a, b } => segment_distance(p, a, b),
            Base::Arc { .. } => {
                let steps = ((self.length() / 3.0).ceil() as usize).max(8);
                let mut best = f32::INFINITY;
                let mut prev = self.point(0.0);
                for k in 1..=steps {
                    let next = self.point(k as f32 / steps as f32);
                    best = best.min(segment_distance(p, prev, next));
                    prev = next;
                }
                best
            }
        }
    }
}

/// The distance from `p` to the segment `a`–`b`.
fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = sub(b, a);
    let l2 = dot(d, d);
    let t = if l2 > 0.0 {
        (dot(sub(p, a), d) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    len(sub(p, lerp(a, b, t)))
}

/// The distance from `p` to `rect`, zero inside it.
fn rect_distance(p: [f32; 2], rect: RectF) -> f32 {
    let dx = (rect.x - p[0]).max(p[0] - (rect.x + rect.width)).max(0.0);
    let dy = (rect.y - p[1]).max(p[1] - (rect.y + rect.height)).max(0.0);
    dx.hypot(dy)
}

/// A placed propagator: its base path, its ends, whether it is a loop and
/// the side its label sits on (`1` left of travel, `-1` right).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    pub(crate) base: Base,
    pub(crate) ends: ([f32; 2], [f32; 2]),
    pub(crate) is_loop: bool,
    pub(crate) label_side: f32,
}

/// What a pick of a Feynman series hit, by row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeynmanHit {
    Vertex(usize),
    Propagator(usize),
    Label(FeynmanLabelPart, usize),
}

/// A series placed in a panel ([`FeynmanSeries::placed`]): the topology,
/// every vertex's screen point (logical pixels, NaN where the axes do not
/// place it) and the part drawn picked. Everything here is a display
/// parameter (rule 1), dropped with the operation.
pub(crate) struct Placed<'s> {
    series: &'s FeynmanSeries,
    topology: Topology,
    px: Vec<[f64; 2]>,
    picked: Option<FeynmanHit>,
}

/// The gap between a part and its label, logical pixels.
const LABEL_GAP: f32 = 6.0;
/// The gap between a line and its momentum arrow, logical pixels.
const MOMENTUM_GAP: f32 = 6.0;
/// The share of a line its momentum arrow spans, centred.
const MOMENTUM_SPAN: f32 = 0.4;
/// Samples per half period of a wave and per loop of a coil.
const WAVE_SAMPLES: usize = 6;
const COIL_SAMPLES: usize = 12;
/// The segments of a marker circle.
const CIRCLE_SEGMENTS: usize = 24;
/// The gap between a picked label and its box, logical pixels.
const LABEL_BOX_GAP: f32 = 2.0;

impl Placed<'_> {
    /// The screen point of vertex `index`, or `None` where the axes do not
    /// place it.
    pub(crate) fn px(&self, index: usize) -> Option<[f32; 2]> {
        let p = self.px.get(index)?;
        (p[0].is_finite() && p[1].is_finite()).then(|| [p[0] as f32, p[1] as f32])
    }

    /// Vertex `index`'s outward direction: away from its neighbours, up
    /// when they cancel.
    fn outward(&self, index: usize) -> [f32; 2] {
        let Some(p) = self.px(index) else {
            return [0.0, -1.0];
        };
        let mut sum = [0.0f32; 2];
        for &[j, _] in self.topology.around(index) {
            let j = j as usize;
            if j == index {
                continue;
            }
            if let Some(q) = self.px(j) {
                sum = add(sum, unit(sub(q, p)));
            }
        }
        if len(sum) < 1e-3 {
            [0.0, -1.0]
        } else {
            unit(scale(sum, -1.0))
        }
    }

    /// Propagator `index`'s bend, degrees: its own, or its place in its
    /// fan. The fan's step narrows so that its outer lines stay within
    /// [`MAX_BEND`], so any number of parallel lines draws, each apart.
    fn bend(&self, index: usize) -> f32 {
        let series = self.series;
        if let Some(bend) = series.propagators[index].bend {
            return bend.clamp(-MAX_BEND, MAX_BEND);
        }
        let Some((rank, count, from_low)) = self.topology.fan(series, index) else {
            return 0.0;
        };
        let step = if count > 1 {
            series
                .style
                .fan_step
                .min(2.0 * MAX_BEND / (count - 1) as f32)
        } else {
            series.style.fan_step
        };
        let canonical = (count as f32 - 1.0) / 2.0 - rank as f32;
        // The fan is laid out from the lower row; a line running the other
        // way bends the other way to land on the same arc.
        canonical * step * if from_low { 1.0 } else { -1.0 }
    }

    /// The geometry of propagator `index`, or `None` when an end is not
    /// placed on screen.
    pub(crate) fn geometry(&self, index: usize) -> Option<Geometry> {
        let (from, to) = self.topology.ends_of(index)?;
        let a = self.px(from)?;
        if from == to {
            let (rank, count) = self.topology.loop_rank(self.series, index)?;
            let out = self.outward(from);
            let base_angle = out[1].atan2(out[0]);
            let turn = if count > 1 {
                (rank as f32 - (count as f32 - 1.0) / 2.0) * (PI / count as f32)
            } else {
                0.0
            };
            let angle = base_angle + turn;
            let radius = self.series.style.loop_radius;
            let centre = add(a, [radius * angle.cos(), radius * angle.sin()]);
            let start = (a[1] - centre[1]).atan2(a[0] - centre[0]);
            return Some(Geometry {
                base: Base::Arc {
                    centre,
                    radius,
                    start,
                    sweep: 2.0 * PI,
                },
                ends: (a, a),
                is_loop: true,
                label_side: -1.0,
            });
        }
        let b = self.px(to)?;
        let chord = sub(b, a);
        let c = len(chord);
        let bend = self.bend(index).to_radians();
        if c <= 0.0 || bend.abs() < 1e-3 {
            return Some(Geometry {
                base: Base::Line { a, b },
                ends: (a, b),
                is_loop: false,
                label_side: 1.0,
            });
        }
        let normal = left_normal(scale(chord, 1.0 / c));
        let middle = lerp(a, b, 0.5);
        let radius = (c / 2.0) / bend.sin().abs();
        let offset = (c / 2.0) / bend.tan();
        // The centre lies across the chord from the bulge.
        let centre = sub(middle, scale(normal, offset));
        let start = (a[1] - centre[1]).atan2(a[0] - centre[0]);
        let bulge = add(middle, scale(normal, (c / 2.0) * (bend / 2.0).tan()));
        let mid_angle = (bulge[1] - centre[1]).atan2(bulge[0] - centre[0]);
        let mut half = mid_angle - start;
        while half > PI {
            half -= 2.0 * PI;
        }
        while half < -PI {
            half += 2.0 * PI;
        }
        Some(Geometry {
            base: Base::Arc {
                centre,
                radius,
                start,
                sweep: 2.0 * half,
            },
            ends: (a, b),
            is_loop: false,
            label_side: if bend >= 0.0 { 1.0 } else { -1.0 },
        })
    }

    /// How far propagator `index`'s ink reaches from its base path: half
    /// its width, plus its wave, coil or double line.
    fn envelope(&self, index: usize) -> f32 {
        let series = self.series;
        let propagator = &series.propagators[index];
        let width = propagator.width_px.unwrap_or(series.style.width);
        let reach = match propagator.particle {
            ParticleLine::Photon | ParticleLine::ChargedBoson => series.style.wave.amplitude,
            ParticleLine::Graviton => series.style.wave.amplitude + width,
            ParticleLine::Gluon => 2.0 * series.style.coil.amplitude,
            ParticleLine::DoubleLine => width,
            _ => 0.0,
        };
        reach + width / 2.0
    }

    /// The extra width and the colour of part `hit`: the picked style's
    /// when it is the picked part, else none and `color`.
    fn picked_look(&self, hit: FeynmanHit, color: Color) -> (f32, Color) {
        if self.picked == Some(hit) {
            let picked = self.series.style.picked;
            (picked.width_extra_px, picked.color)
        } else {
            (0.0, color)
        }
    }

    /// Emit propagator `index`'s strokes and arrows, its paths built in
    /// `path`.
    fn emit_propagator(
        &self,
        index: usize,
        path: &mut Vec<PathSegment>,
        sink: &mut dyn Sink,
    ) -> Result<(), &'static str> {
        let Some(geometry) = self.geometry(index) else {
            return Ok(());
        };
        let series = self.series;
        let propagator = &series.propagators[index];
        let style = &series.style;
        let (extra, color) = self.picked_look(
            FeynmanHit::Propagator(index),
            propagator.color.unwrap_or(style.color),
        );
        let width = propagator.width_px.unwrap_or(style.width) + extra;
        let base = geometry.base;
        let closed = geometry.is_loop;
        let stroke = StrokeStyle {
            width,
            color,
            dash: propagator.particle.dash(),
            tag: "feynman_propagator",
        };
        match propagator.particle {
            ParticleLine::Photon | ParticleLine::ChargedBoson => {
                wave_path(&base, style.wave, 0.0, closed, path)?;
                sink.path(Slot::Above, path, &stroke);
            }
            ParticleLine::Graviton => {
                for shift in [-width, width] {
                    wave_path(&base, style.wave, shift, closed, path)?;
                    sink.path(Slot::Above, path, &stroke);
                }
            }
            ParticleLine::Gluon => {
                coil_path(&base, style.coil, closed, path)?;
                sink.path(Slot::Above, path, &stroke);
            }
            ParticleLine::DoubleLine => {
                for shift in [-width, width] {
                    offset_path(&base, shift, 0.0, 1.0, closed, path)?;
                    sink.path(Slot::Above, path, &stroke);
                }
            }
            _ => match base {
                Base::Line { a, b } => sink.stroke(Slot::Above, a, b, &stroke),
                Base::Arc { .. } => {
                    offset_path(&base, 0.0, 0.0, 1.0, closed, path)?;
                    sink.path(Slot::Above, path, &stroke);
                }
            },
        }
        let head = |sink: &mut dyn Sink, t: f32, reversed: bool| {
            let mut dir = base.tangent(t);
            if reversed {
                dir = scale(dir, -1.0);
            }
            let length = style.head_length + extra;
            let half = (style.head_width + extra) / 2.0;
            let centre = base.point(t);
            let tip = add(centre, scale(dir, length / 2.0));
            let back = sub(centre, scale(dir, length / 2.0));
            let n = scale(left_normal(dir), half);
            sink.triangle(Slot::Above, [tip, add(back, n), sub(back, n)], color);
        };
        if let Some(reversed) = propagator.particle.arrow() {
            head(sink, 0.5, reversed);
        }
        if propagator.particle == ParticleLine::Majorana {
            head(sink, 0.3, true);
            head(sink, 0.7, false);
        }
        if let Some(momentum) = &propagator.momentum {
            let side = match momentum.side {
                MomentumSide::Left => 1.0,
                MomentumSide::Right => -1.0,
            };
            let shift = side * (self.envelope(index) + MOMENTUM_GAP);
            let (t0, t1) = (0.5 - MOMENTUM_SPAN / 2.0, 0.5 + MOMENTUM_SPAN / 2.0);
            offset_path(&base, shift, t0, t1, false, path)?;
            let thin = StrokeStyle {
                width: style.width.min(1.5),
                color,
                dash: &[],
                tag: "feynman_momentum",
            };
            sink.path(Slot::Above, path, &thin);
            let (end, before) = if momentum.reversed {
                (t0, t1)
            } else {
                (t1, t0)
            };
            let tip = offset_point(&base, shift, end);
            let dir = unit(sub(
                tip,
                offset_point(&base, shift, lerp_f(end, before, 0.05)),
            ));
            let length = style.head_length * 0.8;
            let back = sub(tip, scale(dir, length));
            let n = scale(left_normal(dir), style.head_width * 0.4);
            sink.triangle(Slot::Above, [tip, add(back, n), sub(back, n)], color);
        }
        Ok(())
    }

    /// Emit vertex `index`'s marker; a circled cross builds its circle in
    /// `path`.
    fn emit_vertex(
        &self,
        index: usize,
        path: &mut Vec<PathSegment>,
        sink: &mut dyn Sink,
    ) -> Result<(), &'static str> {
        let Some(p) = self.px(index) else {
            return Ok(());
        };
        let style = &self.series.style;
        let picked = self.picked == Some(FeynmanHit::Vertex(index));
        let (extra, color) = self.picked_look(FeynmanHit::Vertex(index), style.color);
        let line = StrokeStyle {
            width: style.width + extra,
            color,
            dash: &[],
            tag: "feynman_vertex",
        };
        let marker = self.series.vertices[index].drawn_marker();
        match marker {
            // A picked leg end shows where it is.
            VertexMarker::Hidden if picked => {
                sink.polygon(
                    Slot::Above,
                    &circle(p, style.dot_radius + extra / 2.0),
                    Some(color),
                    None,
                );
            }
            VertexMarker::Hidden => {}
            VertexMarker::Dot => {
                sink.polygon(
                    Slot::Above,
                    &circle(p, style.dot_radius + extra / 2.0),
                    Some(color),
                    None,
                );
            }
            VertexMarker::Blob { radius_px, fill } => {
                sink.polygon(
                    Slot::Above,
                    &circle(p, radius_px),
                    Some(fill.unwrap_or(style.color)),
                    Some(&line),
                );
            }
            VertexMarker::Cross | VertexMarker::CircledCross => {
                let r = (style.dot_radius * 2.0).max(4.0);
                let d = r * std::f32::consts::FRAC_1_SQRT_2;
                sink.stroke(
                    Slot::Above,
                    [p[0] - d, p[1] - d],
                    [p[0] + d, p[1] + d],
                    &line,
                );
                sink.stroke(
                    Slot::Above,
                    [p[0] - d, p[1] + d],
                    [p[0] + d, p[1] - d],
                    &line,
                );
                if marker == VertexMarker::CircledCross {
                    let base = Base::Arc {
                        centre: p,
                        radius: r,
                        start: 0.0,
                        sweep: 2.0 * PI,
                    };
                    offset_path(&base, 0.0, 0.0, 1.0, true, path)?;
                    sink.path(Slot::Above, path, &line);
                }
            }
        }
        Ok(())
    }

    /// The box drawn around a picked label: its rect, a little apart, in
    /// the picked colour, one pixel plus the picked extra width across.
    fn emit_label_box(&self, text: &PlacedText<'_>, sink: &mut dyn Sink) {
        let style = &self.series.style;
        let r = text.rect();
        let (x0, y0) = (r.x - LABEL_BOX_GAP, r.y - LABEL_BOX_GAP);
        let (x1, y1) = (
            r.x + r.width + LABEL_BOX_GAP,
            r.y + r.height + LABEL_BOX_GAP,
        );
        let line = StrokeStyle {
            width: 1.0 + style.picked.width_extra_px,
            color: style.picked.color,
            dash: &[],
            tag: "feynman_label",
        };
        sink.polygon(
            Slot::Above,
            &[[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
            None,
            Some(&line),
        );
    }

    /// The radius a vertex's marker covers, for its label and its pick.
    fn marker_radius(&self, index: usize) -> f32 {
        let style = &self.series.style;
        match self.series.vertices[index].drawn_marker() {
            VertexMarker::Hidden => 0.0,
            VertexMarker::Dot => style.dot_radius,
            VertexMarker::Blob { radius_px, .. } => radius_px + style.width / 2.0,
            VertexMarker::Cross | VertexMarker::CircledCross => {
                (style.dot_radius * 2.0).max(4.0) + style.width / 2.0
            }
        }
    }

    /// The placed label of row `index` of `part`, or `None` when the row
    /// has no label or is not drawn.
    pub(crate) fn label(
        &self,
        part: FeynmanLabelPart,
        index: usize,
        measure: &dyn MeasureText,
    ) -> Option<PlacedText<'_>> {
        let series = self.series;
        let (text, anchor, direction, offset) = match part {
            FeynmanLabelPart::Vertex => {
                let vertex = series.vertices.get(index)?;
                let text = vertex.label.as_ref()?;
                let p = self.px(index)?;
                let out = self.outward(index);
                let reach = self.marker_radius(index) + LABEL_GAP;
                (text, add(p, scale(out, reach)), out, vertex.label_offset)
            }
            FeynmanLabelPart::Propagator => {
                let propagator = series.propagators.get(index)?;
                let text = propagator.label.as_ref()?;
                let geometry = self.geometry(index)?;
                let side = geometry.label_side;
                let n = scale(left_normal(geometry.base.tangent(0.5)), side);
                let reach = self.label_reach(index, side);
                (
                    text,
                    add(geometry.base.point(0.5), scale(n, reach)),
                    n,
                    propagator.label_offset,
                )
            }
            FeynmanLabelPart::Momentum => {
                let propagator = series.propagators.get(index)?;
                let momentum = propagator.momentum.as_ref()?;
                let geometry = self.geometry(index)?;
                let side = match momentum.side {
                    MomentumSide::Left => 1.0,
                    MomentumSide::Right => -1.0,
                };
                let shift = side
                    * (self.envelope(index)
                        + MOMENTUM_GAP
                        + series.style.head_width * 0.4
                        + LABEL_GAP);
                let n = scale(left_normal(geometry.base.tangent(0.5)), side);
                (
                    &momentum.label,
                    offset_point(&geometry.base, shift, 0.5),
                    n,
                    momentum.label_offset,
                )
            }
        };
        let extents = measure.measure_rich(text);
        // The block's centre sits half its size beyond the anchor, along
        // `direction`, so its near side touches the anchor.
        let centre = [
            anchor[0] + direction[0] * extents.width / 2.0,
            anchor[1] + direction[1] * extents.height() / 2.0,
        ];
        Some(PlacedText {
            text: TextRef::Rich(text),
            origin: (
                centre[0] - extents.width / 2.0 + offset[0],
                centre[1] + (extents.ascent - extents.descent) / 2.0 + offset[1],
            ),
            extents,
            rotation: None,
        })
    }

    /// How far a propagator's label sits from its base path on `side`: past
    /// its ink there, and past its momentum arrow when that is on the same
    /// side.
    fn label_reach(&self, index: usize, side: f32) -> f32 {
        let series = self.series;
        let propagator = &series.propagators[index];
        let mut reach = self.envelope(index);
        // A coil lies on the left of travel.
        if propagator.particle == ParticleLine::Gluon && side < 0.0 {
            reach = propagator.width_px.unwrap_or(series.style.width) / 2.0;
        }
        if let Some(momentum) = &propagator.momentum {
            let same = matches!(
                (momentum.side, side > 0.0),
                (MomentumSide::Left, true) | (MomentumSide::Right, false)
            );
            if same {
                reach += MOMENTUM_GAP + series.style.head_width;
            }
        }
        reach + LABEL_GAP
    }
}

fn lerp_f(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The point of `base` at `t`, `shift` pixels to its left.
fn offset_point(base: &Base, shift: f32, t: f32) -> [f32; 2] {
    add(base.point(t), scale(left_normal(base.tangent(t)), shift))
}

/// The polygon of a circle of `radius` about `centre`, on the stack.
fn circle(centre: [f32; 2], radius: f32) -> [[f32; 2]; CIRCLE_SEGMENTS] {
    std::array::from_fn(|k| {
        let angle = 2.0 * PI * k as f32 / CIRCLE_SEGMENTS as f32;
        [
            centre[0] + radius * angle.cos(),
            centre[1] + radius * angle.sin(),
        ]
    })
}

/// Replace `out` with the smooth path through `f(t)` for `t` from `t0` to
/// `t1` in `samples` steps: cubic Hermite segments with the curve's own
/// derivatives (taken numerically), as Bézier segments. `closed` closes
/// the path. The segment count is known first, so `out` grows (doubling)
/// before it is written, never while it is.
fn sampled_path(
    f: &dyn Fn(f32) -> [f32; 2],
    t0: f32,
    t1: f32,
    samples: usize,
    closed: bool,
    out: &mut Vec<PathSegment>,
) -> Result<(), &'static str> {
    let samples = samples.max(1);
    out.clear();
    out.try_reserve(samples + 2)
        .map_err(|_| ALLOCATION_FAILED)?;
    let h = (t1 - t0) / samples as f32;
    let eps = (h * 1e-2).abs().max(1e-6);
    let derivative = |t: f32| scale(sub(f(t + eps), f(t - eps)), 1.0 / (2.0 * eps));
    let tuple = |p: [f32; 2]| (p[0], p[1]);
    out.push(PathSegment::Move(tuple(f(t0))));
    for k in 0..samples {
        let (ta, tb) = (t0 + h * k as f32, t0 + h * (k + 1) as f32);
        let (pa, pb) = (f(ta), f(tb));
        let (da, db) = (derivative(ta), derivative(tb));
        out.push(PathSegment::Cubic(
            tuple(add(pa, scale(da, h / 3.0))),
            tuple(sub(pb, scale(db, h / 3.0))),
            tuple(pb),
        ));
    }
    if closed {
        out.push(PathSegment::Close);
    }
    Ok(())
}

/// The base path from `t0` to `t1`, `shift` pixels to its left, into `out`.
fn offset_path(
    base: &Base,
    shift: f32,
    t0: f32,
    t1: f32,
    closed: bool,
    out: &mut Vec<PathSegment>,
) -> Result<(), &'static str> {
    let samples = match base {
        Base::Line { .. } => 1,
        Base::Arc { sweep, .. } => ((sweep.abs() * (t1 - t0) / (PI / 8.0)).ceil() as usize).max(1),
    };
    sampled_path(
        &|t| offset_point(base, shift, t),
        t0,
        t1,
        samples,
        closed,
        out,
    )
}

/// A wave along `base` into `out`: a sine of the wiggle's amplitude, its
/// wavelength fitted so that whole half periods span the path, `shift`
/// pixels to the left. A loop's wave closes on itself with whole periods.
fn wave_path(
    base: &Base,
    wave: Wiggle,
    shift: f32,
    closed: bool,
    out: &mut Vec<PathSegment>,
) -> Result<(), &'static str> {
    let length = base.length();
    let mut halves = ((length / (wave.wavelength / 2.0)).round() as usize).max(1);
    if closed && halves % 2 == 1 {
        halves += 1;
    }
    let amplitude = wave.amplitude;
    let f = |t: f32| {
        let a = amplitude * (PI * halves as f32 * t).sin() + shift;
        offset_point(base, a, t)
    };
    sampled_path(&f, 0.0, 1.0, halves * WAVE_SAMPLES, closed, out)
}

/// A coil along `base` into `out`: whole loops (a prolate cycloid) on the
/// left of travel, of the wiggle's size, its wavelength fitted to the path.
fn coil_path(
    base: &Base,
    coil: Wiggle,
    closed: bool,
    out: &mut Vec<PathSegment>,
) -> Result<(), &'static str> {
    let length = base.length().max(1e-3);
    let loops = ((length / coil.wavelength).round() as usize).max(1);
    let a = coil.amplitude;
    let f = |t: f32| {
        let phase = 2.0 * PI * loops as f32 * t;
        let along = t - a * phase.sin() / length;
        offset_point(base, a * (1.0 - phase.cos()), along)
    };
    sampled_path(&f, 0.0, 1.0, loops * COIL_SAMPLES, closed, out)
}

impl SeriesConfig {
    /// The Feynman fields of this series, or `None` for every other kind.
    pub fn feynman(&self) -> Option<&FeynmanSeries> {
        match &self.render_type {
            crate::data_config::DataRenderType::Feynman(diagram) => Some(diagram),
            _ => None,
        }
    }

    /// The Feynman fields of this series, mutably.
    pub fn feynman_mut(&mut self) -> Option<&mut FeynmanSeries> {
        match &mut self.render_type {
            crate::data_config::DataRenderType::Feynman(diagram) => Some(diagram),
            _ => None,
        }
    }

    /// Whether the series is DataLess: drawn from its SSOT on the CPU, with
    /// no column (an energy-level or Feynman series).
    pub fn is_dataless(&self) -> bool {
        self.energy_levels().is_some() || self.feynman().is_some()
    }
}

impl crate::config::Config {
    /// The chart rules of Feynman series (#234): each is valid on its own
    /// ([`FeynmanSeries::validate`]), reads no column (`x_column` and
    /// `y_column` empty) and is drawn on a rectangular chart with numeric
    /// x and y axes, not on polar, ternary, parallel or 3D axes.
    pub fn validate_feynman(&self, series: &[SeriesConfig]) -> Result<(), &'static str> {
        for item in series {
            let Some(diagram) = item.feynman() else {
                continue;
            };
            diagram.validate()?;
            if !item.x_column.is_empty() || !item.y_column.is_empty() || item.z_column.is_some() {
                return Err("a Feynman series reads no columns: x_column and y_column stay empty");
            }
            if self.polar().is_some()
                || self.ternary().is_some()
                || self.parallel().is_some()
                || self.view_3d().is_some()
            {
                return Err("a Feynman series is drawn on a rectangular chart");
            }
            if self.bottom_x.categories.is_some() {
                return Err("a Feynman series is placed on a numeric bottom_x");
            }
        }
        Ok(())
    }
}

/// The selection `step_selection` moves to from `reference` on the Feynman
/// series `series[position]` (#234): a vertex steps by `dx` over the
/// vertices and a propagator over the propagators; `dy` moves to another
/// Feynman series of the list, in series order, to its row nearest the
/// same index. A label does not step. `None` when there is nothing to move
/// to.
pub(crate) fn stepped_selection(
    series: &[SeriesConfig],
    position: usize,
    reference: &crate::config::PickedDataRef,
    dx: i32,
    dy: i32,
) -> Option<crate::config::PickedDataRef> {
    use crate::config::PickedDataRef;
    let (vertices, index) = match reference {
        PickedDataRef::FeynmanVertex { vertex_index, .. } => (true, *vertex_index),
        PickedDataRef::FeynmanPropagator {
            propagator_index, ..
        } => (false, *propagator_index),
        _ => return None,
    };
    let members = || series.iter().filter(|s| s.feynman().is_some());
    let at = members().position(|s| s.series_id == series[position].series_id)?;
    let target = at
        .saturating_add_signed(dy as isize)
        .min(members().count().saturating_sub(1));
    let next = members().nth(target)?;
    let diagram = next.feynman()?;
    let len = if vertices {
        diagram.vertices.len()
    } else {
        diagram.propagators.len()
    };
    let last = len.checked_sub(1)?;
    let row = index.min(last).saturating_add_signed(dx as isize).min(last);
    let source_id = next.source_id.clone();
    let series_id = next.series_id.clone();
    Some(if vertices {
        PickedDataRef::FeynmanVertex {
            source_id,
            series_id,
            vertex_index: row,
        }
    } else {
        PickedDataRef::FeynmanPropagator {
            source_id,
            series_id,
            propagator_index: row,
        }
    })
}

#[cfg(test)]
mod tests;
