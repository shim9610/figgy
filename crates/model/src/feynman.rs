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
//! The renderer computes appearance only: the placement of the
//! unplaced vertices, pixel positions, the stroke geometry (a wave's or a
//! coil's phase, fitted so that whole periods span each propagator), the
//! fan of parallel propagators, the arrowheads along the drawn path and the
//! label places. No physics is computed: charge, momentum conservation and
//! the allowed couplings are the host's.
//!
//! The series is drawn through the CPU sink, as the energy-level series
//! (#203) is, and the vector export writes the same emit as vector parts.
//! It has no count limit: every list is drawn to its end.
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
#[derive(Clone, Debug, PartialEq)]
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
        serde(
            default = "crate::defaults::feynman_style",
            skip_serializing_if = "FeynmanStyle::is_default"
        )
    )]
    pub style: FeynmanStyle,
    /// How the automatic layout places the unplaced vertices.
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::feynman_layout",
            skip_serializing_if = "FeynmanLayout::is_default"
        )
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
        serde(
            default = "crate::defaults::vertex_role",
            skip_serializing_if = "VertexRole::is_internal"
        )
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VertexRole {
    /// An interaction point.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ParticleLine {
    /// A plain line.
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
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::momentum_side",
            skip_serializing_if = "is_left"
        )
    )]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MomentumSide {
    Left,
    Right,
}

/// The look of a Feynman series, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default = "crate::defaults::feynman_style"))]
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
#[cfg_attr(feature = "serde", serde(default = "crate::defaults::feynman_picked"))]
pub struct FeynmanPicked {
    /// The colour, channels in `0..=1`.
    pub color: Color,
    /// The extra width, logical pixels, `0..=16`.
    pub width_extra_px: f32,
}

/// The amplitude and wavelength of a wave or a coil, logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Wiggle {
    pub amplitude: f32,
    pub wavelength: f32,
}

impl FeynmanStyle {
    #[cfg(feature = "serde")]
    fn is_default(&self) -> bool {
        *self == crate::defaults::feynman_style()
    }

    fn validate(&self) -> Result<(), crate::RuleKind> {
        if !color_ok(self.color) || !finite_in(self.width, 0.5, 24.0) {
            return Err(crate::RuleKind::FeynmanLineStyle);
        }
        for wiggle in [self.wave, self.coil] {
            if !finite_in(wiggle.amplitude, 0.0, 64.0) || !finite_in(wiggle.wavelength, 2.0, 256.0)
            {
                return Err(crate::RuleKind::FeynmanWave);
            }
        }
        if !finite_in(self.head_length, 0.0, 64.0)
            || !finite_in(self.head_width, 0.0, 64.0)
            || !finite_in(self.dot_radius, 0.0, 64.0)
            || !finite_in(self.loop_radius, 2.0, 512.0)
            || !finite_in(self.fan_step, 0.0, 120.0)
        {
            return Err(crate::RuleKind::FeynmanSizes);
        }
        if !color_ok(self.picked.color) || !finite_in(self.picked.width_extra_px, 0.0, 16.0) {
            return Err(crate::RuleKind::FeynmanPickedStyle);
        }
        Ok(())
    }
}

/// The direction time runs in: where the layout puts the legs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimeDirection {
    /// Incoming legs on the left, outgoing on the right.
    LeftToRight,
    /// Incoming legs at the bottom, outgoing at the top.
    BottomToTop,
}

/// How the automatic layout places the unplaced vertices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FeynmanLayout {
    #[cfg_attr(feature = "serde", serde(default = "crate::defaults::time_direction"))]
    pub direction: TimeDirection,
}

impl FeynmanLayout {
    #[cfg(feature = "serde")]
    fn is_default(&self) -> bool {
        *self == crate::defaults::feynman_layout()
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
// at most four transient buffers, dropped when the operation returns: the
// topology arena (`Topology`), the position buffer (`Placed::px`, or the
// list `positions` returns) and the layout's conjugate-gradient work
// buffer (`lay_out`), each taken once at its exact size by `exact`, and the
// emit's path buffer, which grows (doubling, before it is written) to the
// longest visible path. No loop over vertices, propagators, solver
// iterations or path samples allocates. Rows are `usize`, so a series has
// no count limit (rule 10): the only failure is an allocation that fails.

/// The error of an operation whose working memory cannot be allocated.
pub(crate) const ALLOCATION_FAILED: crate::RuleKind = crate::RuleKind::FeynmanAllocation;
/// The error of a layout whose solver stops converging.
pub(crate) const LAYOUT_DID_NOT_CONVERGE: crate::RuleKind =
    crate::RuleKind::FeynmanLayoutDidNotConverge;

/// The widest bend of a line, degrees: a propagator's `bend` and the outer
/// lines of a fan stay within it, so every arc has a finite radius.
const MAX_BEND: f32 = 170.0;
/// An id that names no vertex.
const NONE: usize = usize::MAX;
/// The conjugate-gradient tolerance: the relative residual it solves to.
const CG_TOLERANCE_SQUARED: f64 = 1e-24;
/// Solver iterations without a new best residual after which the layout
/// reports that it did not converge: a numerical setting, not a count of
/// data (rule 10).
const CG_STALL: u32 = 64;

/// `len` copies of `fill` in one allocation of exactly that size.
fn exact<T: Copy>(len: usize, fill: T) -> Result<Vec<T>, crate::RuleKind> {
    let mut v: Vec<T> = Vec::new();
    v.try_reserve_exact(len).map_err(|_| ALLOCATION_FAILED)?;
    v.resize(len, fill);
    Ok(v)
}

/// The vertex row `order` (rows sorted by id) names for `id`, or [`NONE`].
fn lookup(series: &FeynmanSeries, order: &[usize], id: &str) -> usize {
    order
        .binary_search_by(|&r| series.vertices[r].id.as_str().cmp(id))
        .map_or(NONE, |k| order[k])
}

/// The topology of a series, worked out once per operation in
/// O(V log V + E log D) (D the largest degree): one exact-size allocation
/// of `usize` words, `[order | offsets | incident | ends | flags]`.
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
    words: Vec<usize>,
    v: usize,
    e: usize,
}

/// The regions of a topology arena: `(order, offsets, incident, ends, flags)`.
type Regions<'a> = (
    &'a mut [usize],
    &'a mut [usize],
    &'a mut [usize],
    &'a mut [usize],
    &'a mut [usize],
);

const FIXED: usize = 1;
const ANCHORED: usize = 2;

impl Topology {
    fn new(series: &FeynmanSeries) -> Result<Self, crate::RuleKind> {
        let (v, e) = (series.vertices.len(), series.propagators.len());
        let len = v
            .checked_mul(3)
            .and_then(|n| n.checked_add(1))
            .and_then(|n| n.checked_add(e.checked_mul(6)?))
            .ok_or(ALLOCATION_FAILED)?;
        let mut topology = Self {
            words: exact(len, 0usize)?,
            v,
            e,
        };
        let (order, offsets, incident, ends, _) = topology.parts_mut();
        for (i, slot) in order.iter_mut().enumerate() {
            *slot = i;
        }
        order.sort_unstable_by(|&a, &b| series.vertices[a].id.cmp(&series.vertices[b].id));
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
            offsets[a] += 1;
            if a != b {
                offsets[b] += 1;
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
            offsets[a] -= 1;
            let k = offsets[a];
            incident[2 * k] = b;
            incident[2 * k + 1] = p;
            if a != b {
                offsets[b] -= 1;
                let k = offsets[b];
                incident[2 * k] = a;
                incident[2 * k + 1] = p;
            }
        }
        for i in 0..v {
            let run = &mut incident[2 * offsets[i]..2 * offsets[i + 1]];
            pairs_mut(run).sort_unstable_by_key(|&[other, p]| pair_key(series, i, other, p));
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

    fn offsets(&self) -> &[usize] {
        &self.words[self.v..2 * self.v + 1]
    }

    fn incident(&self) -> &[usize] {
        let start = 2 * self.v + 1;
        &self.words[start..start + 4 * self.e]
    }

    fn ends(&self) -> &[usize] {
        let start = 2 * self.v + 1 + 4 * self.e;
        &self.words[start..start + 2 * self.e]
    }

    /// The `(other end, propagator)` pairs of vertex `i`.
    fn around(&self, i: usize) -> &[[usize; 2]] {
        around(self.offsets(), self.incident(), i)
    }

    /// Propagator `p`'s vertex rows, `None` when an id names no vertex.
    fn ends_of(&self, p: usize) -> Option<(usize, usize)> {
        let ends = self.ends();
        let (a, b) = (*ends.get(2 * p)?, *ends.get(2 * p + 1)?);
        (a != NONE && b != NONE).then_some((a, b))
    }

    /// The run of vertex `i`'s pairs whose other end is `other` and whose
    /// bent flag is `bent`, sorted by propagator.
    fn run(&self, series: &FeynmanSeries, i: usize, other: usize, bent: bool) -> &[[usize; 2]] {
        let pairs = self.around(i);
        let key = |&[o, p]: &[usize; 2]| pair_key(series, i, o, p);
        let start = pairs.partition_point(|pair| key(pair) < (other, bent, 0));
        let end = pairs.partition_point(|pair| key(pair) <= (other, bent, usize::MAX));
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
        let run = self.run(series, low, high, false);
        let rank = run.binary_search_by_key(&(p), |pair| pair[1]).ok()?;
        Some((rank, run.len(), a == low))
    }

    /// The rank of the loop `p` among the loops at its vertex, and their
    /// number.
    fn loop_rank(&self, series: &FeynmanSeries, p: usize) -> Option<(usize, usize)> {
        let (a, _) = self.ends_of(p)?;
        let run = self.run(series, a, a, false);
        let rank = run.binary_search_by_key(&(p), |pair| pair[1]).ok()?;
        Some((rank, run.len()))
    }
}

/// The `(other end, propagator)` pairs of vertex `i` in a CSR adjacency.
fn around<'a>(offsets: &[usize], incident: &'a [usize], i: usize) -> &'a [[usize; 2]] {
    let words = &incident[2 * offsets[i]..2 * offsets[i + 1]];
    let (pairs, _) = words.as_chunks::<2>();
    pairs
}

fn pairs_mut(words: &mut [usize]) -> &mut [[usize; 2]] {
    let (pairs, _) = words.as_chunks_mut::<2>();
    pairs
}

/// The sort key of the pair `(other, p)` at vertex `i`: the other end, then
/// unbent lines before bent ones (a loop counts as unbent), then the row.
fn pair_key(series: &FeynmanSeries, i: usize, other: usize, p: usize) -> (usize, bool, usize) {
    let bent = other != i && series.propagators[p].bend.is_some();
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
) -> Result<(), crate::RuleKind> {
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
            order[top] = i;
            top += 1;
        }
    }
    while top > 0 {
        top -= 1;
        let i = order[top];
        for &[j, _] in nbrs(i) {
            let j = j;
            if flags[j] & ANCHORED == 0 {
                flags[j] |= ANCHORED;
                order[top] = j;
                top += 1;
            }
        }
    }
    // The barycentric embedding of the anchored free vertices.
    if flags.contains(&ANCHORED) {
        // Solved in the frame's units, so the residual neither overflows
        // nor underflows whatever the data's magnitude.
        let span = (frame[1][0] - frame[0][0]).max(frame[1][1] - frame[0][1]);
        let unit = if span.is_finite() && span > 0.0 {
            span
        } else {
            1.0
        };
        solve_barycentric(&nbrs, flags, positions, frame[0], unit)?;
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
        order[0] = start;
        let (mut m, mut k) = (1usize, 0usize);
        while k < m {
            for &[j, _] in nbrs(order[k]) {
                let j = j;
                if flags[j] & ANCHORED == 0 {
                    flags[j] |= ANCHORED;
                    order[m] = j;
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
            positions[i] = [
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
/// adjacency, into `positions`, in units of `unit` from `origin` (the
/// frame), so the residual stays finite for any finite data. One work
/// buffer of `3 V` entries holds the residual, the direction and the
/// product (`r | p | Ap`); `positions` is the iterate. Ends at a relative
/// residual of 1e-12 on both axes; [`CG_STALL`] iterations without a new
/// best residual, or a non-finite value, is [`LAYOUT_DID_NOT_CONVERGE`].
fn solve_barycentric<'a>(
    nbrs: &dyn Fn(usize) -> &'a [[usize; 2]],
    flags: &[usize],
    positions: &mut [[f64; 2]],
    origin: [f64; 2],
    unit: f64,
) -> Result<(), crate::RuleKind> {
    // The free vertices are solved in frame units and converted back on
    // success; a pinned position is read, never rewritten, so it stays
    // bit-exact.
    solve_in_frame(nbrs, flags, positions, origin, unit)?;
    for (i, p) in positions.iter_mut().enumerate() {
        if flags[i] == ANCHORED {
            *p = [origin[0] + p[0] * unit, origin[1] + p[1] * unit];
        }
    }
    Ok(())
}

/// [`solve_barycentric`] in frame units: the free vertices' positions are
/// the iterate, in those units; the pinned ones are read through them.
fn solve_in_frame<'a>(
    nbrs: &dyn Fn(usize) -> &'a [[usize; 2]],
    flags: &[usize],
    positions: &mut [[f64; 2]],
    origin: [f64; 2],
    unit: f64,
) -> Result<(), crate::RuleKind> {
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
            let j = j;
            if j != i && !free(j) {
                b[0] += (positions[j][0] - origin[0]) / unit;
                b[1] += (positions[j][1] - origin[1]) / unit;
            }
        }
        r[i] = b;
        p[i] = b;
        rr[0] += b[0] * b[0];
        rr[1] += b[1] * b[1];
    }
    if !(rr[0].is_finite() && rr[1].is_finite()) {
        return Err(LAYOUT_DID_NOT_CONVERGE);
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
                let j = j;
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
    flags: &[usize],
    order: &mut [usize],
    direction: TimeDirection,
    frame: [[f64; 2]; 2],
) {
    let size = [frame[1][0] - frame[0][0], frame[1][1] - frame[0][1]];
    let tolerance = 1e-9 * size[0].max(size[1]);
    for (i, slot) in order.iter_mut().enumerate() {
        *slot = i;
    }
    order.sort_unstable_by(|&a, &b| {
        let (pa, pb) = (positions[a], positions[b]);
        pa[0]
            .total_cmp(&pb[0])
            .then(pa[1].total_cmp(&pb[1]))
            .then(a.cmp(&b))
    });
    let mut k = 0;
    while k < order.len() {
        let first = positions[order[k]];
        let mut end = k + 1;
        while end < order.len()
            && (positions[order[end]][0] - first[0]).abs() <= tolerance
            && (positions[order[end]][1] - first[1]).abs() <= tolerance
        {
            end += 1;
        }
        let mut moved = 0;
        for &i in &order[k..end] {
            let i = i;
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
    /// the style are in range. No list has a count limit. Works
    /// in one transient list of `V + E` rows.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        self.style.validate()?;
        let (v, e) = (self.vertices.len(), self.propagators.len());
        let mut rows = exact(v.checked_add(e).ok_or(ALLOCATION_FAILED)?, 0usize)?;
        let (order, propagator_order) = rows.split_at_mut(v);
        for (i, slot) in order.iter_mut().enumerate() {
            *slot = i;
        }
        order.sort_unstable_by(|&a, &b| self.vertices[a].id.cmp(&self.vertices[b].id));
        if order
            .windows(2)
            .any(|w| self.vertices[w[0]].id == self.vertices[w[1]].id)
        {
            return Err(crate::RuleKind::FeynmanVertexIds);
        }
        for (i, slot) in propagator_order.iter_mut().enumerate() {
            *slot = i;
        }
        propagator_order
            .sort_unstable_by(|&a, &b| self.propagators[a].id.cmp(&self.propagators[b].id));
        if propagator_order
            .windows(2)
            .any(|w| self.propagators[w[0]].id == self.propagators[w[1]].id)
        {
            return Err(crate::RuleKind::FeynmanPropagatorIds);
        }
        for vertex in &self.vertices {
            if vertex
                .at
                .is_some_and(|p| !p[0].is_finite() || !p[1].is_finite())
            {
                return Err(crate::RuleKind::FeynmanVertexPosition);
            }
            if let Some(VertexMarker::Blob { radius_px, fill }) = vertex.marker
                && (!finite_in(radius_px, 0.0, 256.0) || fill.is_some_and(|c| !color_ok(c)))
            {
                return Err(crate::RuleKind::FeynmanBlob);
            }
            if !rich_ok(vertex.label.as_ref()) || !offset_ok(vertex.label_offset) {
                return Err(crate::RuleKind::FeynmanVertexLabel);
            }
        }
        for propagator in &self.propagators {
            if lookup(self, order, &propagator.from) == NONE
                || lookup(self, order, &propagator.to) == NONE
            {
                return Err(crate::RuleKind::FeynmanPropagatorVertices);
            }
            if propagator
                .bend
                .is_some_and(|b| !finite_in(b, -MAX_BEND, MAX_BEND))
            {
                return Err(crate::RuleKind::FeynmanBend);
            }
            if propagator.color.is_some_and(|c| !color_ok(c))
                || propagator
                    .width_px
                    .is_some_and(|w| !finite_in(w, 0.5, 24.0))
            {
                return Err(crate::RuleKind::FeynmanPropagatorStyle);
            }
            if !rich_ok(propagator.label.as_ref()) || !offset_ok(propagator.label_offset) {
                return Err(crate::RuleKind::FeynmanPropagatorLabel);
            }
            if let Some(momentum) = &propagator.momentum
                && (!rich_ok(Some(&momentum.label)) || !offset_ok(momentum.label_offset))
            {
                return Err(crate::RuleKind::FeynmanMomentumLabel);
            }
        }
        Ok(())
    }

    /// The row of the vertex with id `id`. A linear scan over the SSOT: no
    /// index is kept.
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
    pub fn auto_layout(&mut self) -> Result<bool, crate::RuleKind> {
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
    pub fn positions(&self) -> Result<Vec<[f64; 2]>, crate::RuleKind> {
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
    ) -> Result<(), crate::RuleKind> {
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
    /// parameter). Allocates nothing when every vertex is placed.
    pub fn extent(&self) -> Result<Option<[[f64; 2]; 2]>, crate::RuleKind> {
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
    /// delta or a place the axes cannot convert (outside a logarithmic
    /// domain, in a break gap); nothing changes then. A layout that fails or
    /// cannot be allocated is the error, and nothing changes either.
    /// Allocates nothing when every vertex is placed.
    pub fn drag_vertex(
        &mut self,
        index: usize,
        dx: f32,
        dy: f32,
        space: &dyn AnnotationSpace,
    ) -> Result<NudgeResult, crate::RuleKind> {
        let reject = Ok(NudgeResult::Rejected(NudgeReject::OutOfBounds));
        if !dx.is_finite() || !dy.is_finite() || index >= self.vertices.len() {
            return reject;
        }
        let layout = if self.has_unplaced() {
            Some(self.positions()?)
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
        Ok(NudgeResult::Moved)
    }

    /// Bend propagator `index` by dragging its middle `(dx, dy)` screen
    /// pixels in `space`: the drag across the chord sets `bend` to the arc
    /// through both ends and the moved middle. Rejected for a row past the
    /// end, a loop, a non-finite delta, a line whose ends are not placed on
    /// screen or a bend past 170 degrees; nothing changes then. A layout
    /// that fails or cannot be allocated is the error.
    pub fn drag_bend(
        &mut self,
        index: usize,
        dx: f32,
        dy: f32,
        space: &dyn AnnotationSpace,
    ) -> Result<NudgeResult, crate::RuleKind> {
        let reject = Ok(NudgeResult::Rejected(NudgeReject::OutOfBounds));
        if !dx.is_finite() || !dy.is_finite() {
            return reject;
        }
        let placed = self.placed(space, None)?;
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
        Ok(NudgeResult::Moved)
    }

    /// Place the series in `space` with `picked` drawn picked: the
    /// topology and every vertex's screen point, the two buffers an emit,
    /// a pick and a bend drag work in (plus the layout's, while it runs).
    pub(crate) fn placed<'s>(
        &'s self,
        space: &dyn AnnotationSpace,
        picked: Option<FeynmanHit>,
    ) -> Result<Placed<'s>, crate::RuleKind> {
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
            clip: space.plot_rect(),
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
    ) -> Result<(), crate::RuleKind> {
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
    /// 3). A vertex, a marker or a bare leg end, answers within
    /// `max_distance` of its marker, and there it wins over the lines that
    /// meet at it (within a third of a line's length, so a short line still
    /// picks in its middle); every other part competes by distance, a tie
    /// going to the part drawn later: labels over markers over lines. Lines
    /// are measured within the plot only, as they are drawn, so a cursor
    /// outside `space`'s plot rectangle picks nothing.
    pub fn pick(
        &self,
        space: &dyn AnnotationSpace,
        measure: &dyn MeasureText,
        cursor: [f32; 2],
        max_distance: f32,
    ) -> Result<Option<(FeynmanHit, f32)>, crate::RuleKind> {
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
            let Some(geometry) = placed.geometry(index) else {
                continue;
            };
            let envelope = placed.envelope(index);
            // A vertex the cursor is at wins over the lines that meet it:
            // past its marker, within the pick distance or a third of the
            // line between the markers, whichever is less.
            let at_end = placed.topology.ends_of(index).is_some_and(|(a, b)| {
                let bare = (geometry.base.length() as f32
                    - placed.marker_radius(a)
                    - placed.marker_radius(b))
                .max(0.0);
                [a, b].into_iter().any(|end| {
                    placed.px(end).is_some_and(|p| {
                        len(sub(cursor, p))
                            <= placed.marker_radius(end) + max_distance.min(bare / 3.0)
                    })
                })
            });
            if at_end {
                continue;
            }
            let reach = widened(placed.clip, max_distance + envelope + 1.0);
            let edge = geometry.base.distance(cursor, reach) - envelope;
            offer(FeynmanHit::Propagator(index), edge.max(0.0));
        }
        for index in 0..self.vertices.len() {
            if let Some(p) = placed.px(index) {
                let edge = len(sub(cursor, p)) - placed.marker_radius(index);
                offer(FeynmanHit::Vertex(index), edge.max(0.0));
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

/// The path a propagator's stroke follows, parametrized by `t` in `0..=1`
/// at a constant speed. Points are evaluated in `f64` and returned as
/// logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Base {
    /// A straight line from `a` to `b`.
    Line { a: [f32; 2], b: [f32; 2] },
    /// A circular arc from `a` to `b` that turns by `2 * half` radians,
    /// bulging to the left of the chord for a positive `half`. It is
    /// evaluated from the chord and the angle, never from a centre and a
    /// radius, so a nearly straight arc stays exact however small `half`.
    Bow { a: [f32; 2], b: [f32; 2], half: f32 },
    /// A whole circle about `centre`, starting at angle `start`: a loop.
    Ring {
        centre: [f32; 2],
        radius: f32,
        start: f32,
    },
}

/// `[x, y]` as `f64`.
fn wide(p: [f32; 2]) -> [f64; 2] {
    [f64::from(p[0]), f64::from(p[1])]
}

/// `[x, y]` as `f32`.
fn narrow(p: [f64; 2]) -> [f32; 2] {
    [p[0] as f32, p[1] as f32]
}

/// Below this half angle a bow is evaluated by its series: the exact
/// quotients lose their digits there.
const BOW_SERIES: f64 = 1e-3;

impl Base {
    /// The chord frame of a line or a bow: its start, its unit direction,
    /// its left normal and its length.
    fn chord(a: [f32; 2], b: [f32; 2]) -> ([f64; 2], [f64; 2], [f64; 2], f64) {
        let (a, b) = (wide(a), wide(b));
        let d = [b[0] - a[0], b[1] - a[1]];
        let c = d[0].hypot(d[1]);
        let u = if c > 0.0 {
            [d[0] / c, d[1] / c]
        } else {
            [1.0, 0.0]
        };
        (a, u, [u[1], -u[0]], c)
    }

    /// A bow's point at `t` in its chord frame, in units of the chord: `x`
    /// along it, `y` to its left.
    fn bow_xy(half: f64, t: f64) -> (f64, f64) {
        let s = 1.0 - 2.0 * t;
        if half.abs() < BOW_SERIES {
            let h2 = half * half;
            let ratio = s * (1.0 + h2 * (1.0 - s * s) / 6.0);
            (0.5 - 0.5 * ratio, half * (1.0 - s * s) / 4.0)
        } else {
            let ratio = (half * s).sin() / half.sin();
            let y = (half * (1.0 + s) / 2.0).sin() * (half * (1.0 - s) / 2.0).sin() / half.sin();
            (0.5 - 0.5 * ratio, y)
        }
    }

    /// The point at `t`.
    pub(crate) fn point(&self, t: f64) -> [f32; 2] {
        match *self {
            Base::Line { a, b } => {
                let (a, b) = (wide(a), wide(b));
                narrow([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t])
            }
            Base::Bow { a, b, half } => {
                let (a, u, n, c) = Self::chord(a, b);
                let (x, y) = Self::bow_xy(f64::from(half), t);
                narrow([
                    a[0] + c * (u[0] * x + n[0] * y),
                    a[1] + c * (u[1] * x + n[1] * y),
                ])
            }
            Base::Ring {
                centre,
                radius,
                start,
            } => {
                let angle = f64::from(start) + std::f64::consts::TAU * t;
                let (c, r) = (wide(centre), f64::from(radius));
                narrow([c[0] + r * angle.cos(), c[1] + r * angle.sin()])
            }
        }
    }

    /// The unit direction of travel at `t`.
    pub(crate) fn tangent(&self, t: f64) -> [f32; 2] {
        match *self {
            Base::Line { a, b } => unit(sub(b, a)),
            Base::Bow { a, b, half } => {
                let (_, u, n, _) = Self::chord(a, b);
                let theta = f64::from(half) * (1.0 - 2.0 * t);
                let (cos, sin) = (theta.cos(), theta.sin());
                narrow([u[0] * cos + n[0] * sin, u[1] * cos + n[1] * sin])
            }
            Base::Ring { start, .. } => {
                let angle = f64::from(start) + std::f64::consts::TAU * t;
                narrow([-angle.sin(), angle.cos()])
            }
        }
    }

    /// The length of the path, logical pixels; infinite or NaN when its
    /// ends are.
    pub(crate) fn length(&self) -> f64 {
        match *self {
            Base::Line { a, b } => Self::chord(a, b).3,
            Base::Bow { a, b, half } => {
                let c = Self::chord(a, b).3;
                let half = f64::from(half);
                if half.abs() < BOW_SERIES {
                    c * (1.0 + half * half / 6.0)
                } else {
                    c * half / half.sin()
                }
            }
            Base::Ring { radius, .. } => std::f64::consts::TAU * f64::from(radius),
        }
    }

    /// The parts of the path within `rect`, as parameter intervals in
    /// order (`Spans`). The work is bounded by the rectangle, never by the
    /// length: a line is clipped exactly, a bow is bisected to within half
    /// a pixel of the rectangle's edges, at most `SPAN_DEPTH` times, and a
    /// loop, whose radius the style bounds, is kept or dropped whole.
    pub(crate) fn spans_within(&self, rect: RectF) -> Spans {
        let mut spans = Spans::NONE;
        match *self {
            Base::Line { a, b } => {
                if let Some([t0, t1]) = clip_line(a, b, rect) {
                    spans.push(t0, t1);
                }
            }
            Base::Ring { centre, radius, .. } => {
                let r = radius.abs();
                let ring = RectF {
                    x: centre[0] - r,
                    y: centre[1] - r,
                    width: 2.0 * r,
                    height: 2.0 * r,
                };
                if overlaps(ring, rect) {
                    spans.push(0.0, 1.0);
                }
            }
            Base::Bow { half, .. } => {
                // Pieces that turn by at most a quarter turn, so each one
                // lies within its chord's box widened by its sagitta.
                let turn = 2.0 * f64::from(half).abs();
                let pieces = (turn / std::f64::consts::FRAC_PI_4).ceil().max(1.0) as usize;
                for k in 0..pieces {
                    let (t0, t1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
                    self.bisect(t0, t1, rect, &mut spans);
                }
            }
        }
        spans
    }

    /// Add the parts of the bow between `t0` and `t1` that lie within
    /// `rect` to `spans`, left to right, bisecting on a fixed stack.
    fn bisect(&self, t0: f64, t1: f64, rect: RectF, spans: &mut Spans) {
        let Base::Bow { half, .. } = *self else {
            return;
        };
        let mut stack = [(0.0f64, 0.0f64, 0u32); SPAN_DEPTH as usize + 2];
        let mut top = 1usize;
        stack[0] = (t0, t1, 0);
        while top > 0 {
            top -= 1;
            let (s0, s1, depth) = stack[top];
            let (p0, p1) = (self.point(s0), self.point(s1));
            let chord = len(sub(p1, p0));
            let turn = 2.0 * f64::from(half).abs() * (s1 - s0);
            let sagitta = (f64::from(chord) / 2.0 * (turn / 4.0).tan()) as f32;
            let reach = RectF {
                x: p0[0].min(p1[0]) - sagitta,
                y: p0[1].min(p1[1]) - sagitta,
                width: (p0[0] - p1[0]).abs() + 2.0 * sagitta,
                height: (p0[1] - p1[1]).abs() + 2.0 * sagitta,
            };
            if !(reach.width.is_finite() && reach.height.is_finite()) || !overlaps(reach, rect) {
                continue;
            }
            if inside(reach, rect) || chord <= 0.5 || depth >= SPAN_DEPTH {
                spans.push(s0, s1);
                continue;
            }
            let mid = 0.5 * (s0 + s1);
            // The right half under the left, so the left is taken first.
            stack[top] = (mid, s1, depth + 1);
            stack[top + 1] = (s0, mid, depth + 1);
            top += 2;
        }
    }

    /// The distance from `p` to the parts of the path within `rect`,
    /// sampled at most three pixels apart over those parts only.
    fn distance(&self, p: [f32; 2], rect: RectF) -> f32 {
        let spans = self.spans_within(rect);
        let mut best = f32::INFINITY;
        if let Base::Line { .. } = self {
            for [t0, t1] in spans.iter() {
                best = best.min(segment_distance(p, self.point(t0), self.point(t1)));
            }
            return best;
        }
        let length = self.length();
        for [t0, t1] in spans.iter() {
            let steps = ((length * (t1 - t0) / 3.0).ceil()).clamp(1.0, f64::from(u32::MAX)) as u64;
            let mut prev = self.point(t0);
            for k in 1..=steps {
                let next = self.point(t0 + (t1 - t0) * k as f64 / steps as f64);
                best = best.min(segment_distance(p, prev, next));
                prev = next;
            }
        }
        best
    }
}

/// The most times a bow is bisected towards a rectangle's edge: a numerical
/// setting (each step halves the parameter), not a count of data.
const SPAN_DEPTH: u32 = 60;
/// The most spans kept per path; more are merged into the last.
const SPAN_SLOTS: usize = 8;

/// The parameter intervals of a path that lie within a rectangle, in order,
/// on the stack. When more than `SPAN_SLOTS` would be kept, the last one is
/// widened instead: a superset that draws the same inside the rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spans {
    spans: [[f64; 2]; SPAN_SLOTS],
    len: usize,
    /// Whether a span was merged into the last because the slots were full.
    pub(crate) overflowed: bool,
}

impl Spans {
    const NONE: Self = Self {
        spans: [[0.0; 2]; SPAN_SLOTS],
        len: 0,
        overflowed: false,
    };

    fn push(&mut self, t0: f64, t1: f64) {
        // A circle meets a rectangle at most 8 times, so a path within one
        // has at most 5 visible spans and the slots never fill: widening
        // the last over hidden range would sample off-screen length. The
        // widening is recorded, and
        // `spans_never_overflow_for_lines_and_arcs_crossing_a_plot` checks
        // that it never happens.
        if self.len == SPAN_SLOTS && self.spans[SPAN_SLOTS - 1][1] < t0 {
            self.overflowed = true;
        }
        if let Some(last) = self.len.checked_sub(1)
            && (self.spans[last][1] >= t0 || self.len == SPAN_SLOTS)
        {
            self.spans[last][1] = self.spans[last][1].max(t1);
            return;
        }
        self.spans[self.len] = [t0, t1];
        self.len += 1;
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = [f64; 2]> + '_ {
        self.spans[..self.len].iter().copied()
    }

    /// Whether the spans cover the whole path.
    fn whole(&self) -> bool {
        self.len == 1 && self.spans[0] == [0.0, 1.0]
    }

    /// The spans within `[t0, t1]`.
    fn within(&self, t0: f64, t1: f64) -> Self {
        let mut out = Self::NONE;
        for [a, b] in self.iter() {
            let (a, b) = (a.max(t0), b.min(t1));
            if a < b {
                out.push(a, b);
            }
        }
        out
    }

    /// The spans with each start moved back to a whole period of a dash
    /// `period` pixels long on a path `length` pixels long, so the dashes
    /// keep their place along the path where it is clipped.
    fn dash_aligned(&self, length: f64, period: f64) -> Self {
        if !(period > 0.0 && length.is_finite() && length > 0.0) {
            return *self;
        }
        let mut out = Self::NONE;
        for [t0, t1] in self.iter() {
            let at = (t0 * length / period).floor() * period / length;
            out.push(at.max(0.0), t1);
        }
        out
    }
}

/// Whether two rectangles overlap (touching counts).
fn overlaps(a: RectF, b: RectF) -> bool {
    a.x <= b.x + b.width && b.x <= a.x + a.width && a.y <= b.y + b.height && b.y <= a.y + a.height
}

/// Whether `a` lies within `b`.
fn inside(a: RectF, b: RectF) -> bool {
    a.x >= b.x && a.y >= b.y && a.x + a.width <= b.x + b.width && a.y + a.height <= b.y + b.height
}

/// `rect` widened by `by` on every side.
fn widened(rect: RectF, by: f32) -> RectF {
    RectF {
        x: rect.x - by,
        y: rect.y - by,
        width: rect.width + 2.0 * by,
        height: rect.height + 2.0 * by,
    }
}

/// The part of the segment `a`–`b` within `rect` as a parameter interval
/// (Liang–Barsky), `None` when it misses.
fn clip_line(a: [f32; 2], b: [f32; 2], rect: RectF) -> Option<[f64; 2]> {
    let (a, b) = (wide(a), wide(b));
    let d = [b[0] - a[0], b[1] - a[1]];
    let (x0, y0) = (f64::from(rect.x), f64::from(rect.y));
    let (x1, y1) = (x0 + f64::from(rect.width), y0 + f64::from(rect.height));
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-d[0], a[0] - x0),
        (d[0], x1 - a[0]),
        (-d[1], a[1] - y0),
        (d[1], y1 - a[1]),
    ] {
        if !(p.is_finite() && q.is_finite()) {
            return None;
        }
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    (t0 <= t1).then_some([t0, t1])
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
    /// The plot rectangle the series is drawn clipped to: every path is
    /// sampled within it only (`Base::spans_within`).
    clip: RectF,
}

/// The gap between a part and its label, logical pixels.
const LABEL_GAP: f32 = 6.0;
/// The gap between a line and its momentum arrow, logical pixels.
const MOMENTUM_GAP: f32 = 6.0;
/// The share of a line its momentum arrow spans, centred.
const MOMENTUM_SPAN: f64 = 0.4;
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
        // In f64: a rank and a count are exact there to 2^53 lines, where
        // f32 would merge ranks past 2^24.
        let fan_step = f64::from(series.style.fan_step);
        let step = if count > 1 {
            fan_step.min(2.0 * f64::from(MAX_BEND) / (count - 1) as f64)
        } else {
            fan_step
        };
        let canonical = (count as f64 - 1.0) / 2.0 - rank as f64;
        // The fan is laid out from the lower row; a line running the other
        // way bends the other way to land on the same arc.
        (canonical * step * if from_low { 1.0 } else { -1.0 }) as f32
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
                base: Base::Ring {
                    centre,
                    radius,
                    start,
                },
                ends: (a, a),
                is_loop: true,
                label_side: -1.0,
            });
        }
        let b = self.px(to)?;
        let bend = self.bend(index).to_radians();
        // Only an exactly zero bend is straight: a bow of any other angle
        // is evaluated from its chord, exact however small the angle.
        let base = if bend == 0.0 || a == b {
            Base::Line { a, b }
        } else {
            Base::Bow { a, b, half: bend }
        };
        Some(Geometry {
            base,
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
    ) -> Result<(), crate::RuleKind> {
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
        let stroke = StrokeStyle {
            width,
            color,
            dash: propagator.particle.dash(),
            tag: "feynman_propagator",
        };
        // Only the parts within the plot (and the ink's reach past it) are
        // sampled: the work is bounded by the plot, never by the line's
        // length on screen.
        let spans = base.spans_within(widened(
            self.clip,
            2.0 * self.envelope(index) + width + LABEL_GAP,
        ));
        let closed = geometry.is_loop && spans.whole();
        let draw = |path: &[PathSegment], sink: &mut dyn Sink| {
            if !path.is_empty() {
                sink.path(Slot::Above, path, &stroke);
            }
        };
        match propagator.particle {
            ParticleLine::Photon | ParticleLine::ChargedBoson => {
                wave_path(&base, style.wave, 0.0, &spans, closed, path)?;
                draw(path, sink);
            }
            ParticleLine::Graviton => {
                for shift in [-width, width] {
                    wave_path(&base, style.wave, shift, &spans, closed, path)?;
                    draw(path, sink);
                }
            }
            ParticleLine::Gluon => {
                coil_path(&base, style.coil, &spans, closed, path)?;
                draw(path, sink);
            }
            ParticleLine::DoubleLine => {
                for shift in [-width, width] {
                    offset_path(&base, shift, &spans, closed, path)?;
                    draw(path, sink);
                }
            }
            particle => {
                // A clipped dash starts on a whole period, so it keeps its
                // place along the line.
                let period: f32 = particle.dash().iter().sum();
                let spans = spans.dash_aligned(base.length(), f64::from(period));
                match base {
                    Base::Line { .. } => {
                        for [t0, t1] in spans.iter() {
                            sink.stroke(Slot::Above, base.point(t0), base.point(t1), &stroke);
                        }
                    }
                    Base::Bow { .. } | Base::Ring { .. } => {
                        offset_path(&base, 0.0, &spans, closed, path)?;
                        draw(path, sink);
                    }
                }
            }
        }
        let head = |sink: &mut dyn Sink, t: f64, reversed: bool| {
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
            offset_path(&base, shift, &spans.within(t0, t1), false, path)?;
            let thin = StrokeStyle {
                width: style.width.min(1.5),
                color,
                dash: &[],
                tag: "feynman_momentum",
            };
            if !path.is_empty() {
                sink.path(Slot::Above, path, &thin);
            }
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
    ) -> Result<(), crate::RuleKind> {
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
                    let base = Base::Ring {
                        centre: p,
                        radius: r,
                        start: 0.0,
                    };
                    let mut whole = Spans::NONE;
                    whole.push(0.0, 1.0);
                    offset_path(&base, 0.0, &whole, true, path)?;
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

fn lerp_f(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// The point of `base` at `t`, `shift` pixels to its left.
fn offset_point(base: &Base, shift: f32, t: f64) -> [f32; 2] {
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

/// The cubic segments a span of `dt` takes at `density` segments per unit
/// of `t`: at least one.
fn span_segments(density: f64, dt: f64) -> usize {
    (density * dt).ceil().clamp(1.0, usize::MAX as f64) as usize
}

/// Replace `out` with one subpath per span of `spans`: the smooth path
/// through `f(t)`, `density` cubic segments per unit of `t`, as cubic
/// Hermite segments with the curve's own derivatives (taken numerically)
/// written as Bézier segments. `closed` closes the subpath of a span that
/// covers the whole path. The segment count is known first, so `out` grows
/// before it is written, never while it is; a count past what can be
/// allocated is [`ALLOCATION_FAILED`].
fn sampled_path(
    f: &dyn Fn(f64) -> [f32; 2],
    spans: &Spans,
    density: f64,
    closed: bool,
    out: &mut Vec<PathSegment>,
) -> Result<(), crate::RuleKind> {
    out.clear();
    let mut total = 0usize;
    for [t0, t1] in spans.iter() {
        total = total
            .checked_add(span_segments(density, t1 - t0))
            .and_then(|n| n.checked_add(2))
            .ok_or(ALLOCATION_FAILED)?;
    }
    out.try_reserve(total).map_err(|_| ALLOCATION_FAILED)?;
    let tuple = |p: [f32; 2]| (p[0], p[1]);
    for [t0, t1] in spans.iter() {
        let samples = span_segments(density, t1 - t0);
        let h = (t1 - t0) / samples as f64;
        let eps = (h * 1e-2).abs().max(1e-12);
        // The derivative times h / 3: a Bézier handle.
        let handle = |t: f64| {
            let d = sub(f(t + eps), f(t - eps));
            scale(d, (h / (6.0 * eps)) as f32)
        };
        out.push(PathSegment::Move(tuple(f(t0))));
        for k in 0..samples {
            let (ta, tb) = (t0 + h * k as f64, t0 + h * (k + 1) as f64);
            let (pa, pb) = (f(ta), f(tb));
            out.push(PathSegment::Cubic(
                tuple(add(pa, handle(ta))),
                tuple(sub(pb, handle(tb))),
                tuple(pb),
            ));
        }
        if closed && t0 == 0.0 && t1 == 1.0 {
            out.push(PathSegment::Close);
        }
    }
    Ok(())
}

/// The base path over `spans`, `shift` pixels to its left, into `out`: a
/// line in one segment per span, a bow or a loop in a cubic per sixteenth
/// of a turn.
fn offset_path(
    base: &Base,
    shift: f32,
    spans: &Spans,
    closed: bool,
    out: &mut Vec<PathSegment>,
) -> Result<(), crate::RuleKind> {
    let density = match *base {
        Base::Line { .. } => 0.0,
        Base::Bow { half, .. } => 2.0 * f64::from(half).abs() / (std::f64::consts::PI / 8.0),
        Base::Ring { .. } => 16.0,
    };
    sampled_path(
        &|t| offset_point(base, shift, t),
        spans,
        density,
        closed,
        out,
    )
}

/// A wave along `base` over `spans` into `out`: a sine of the wiggle's
/// amplitude, its wavelength fitted so that whole half periods span the
/// whole path, `shift` pixels to the left; the phase is the whole path's,
/// the samples only the spans'. A loop's wave closes on itself with whole
/// periods. A path of no finite length draws nothing.
fn wave_path(
    base: &Base,
    wave: Wiggle,
    shift: f32,
    spans: &Spans,
    closed: bool,
    out: &mut Vec<PathSegment>,
) -> Result<(), crate::RuleKind> {
    out.clear();
    let length = base.length();
    if !(length.is_finite() && length > 0.0) {
        return Ok(());
    }
    let mut halves = (length / (f64::from(wave.wavelength) / 2.0))
        .round()
        .max(1.0);
    if closed && halves % 2.0 == 1.0 {
        halves += 1.0;
    }
    let amplitude = f64::from(wave.amplitude);
    let f = |t: f64| {
        let a = amplitude * (std::f64::consts::PI * halves * t).sin() + f64::from(shift);
        offset_point(base, a as f32, t)
    };
    sampled_path(&f, spans, halves * WAVE_SAMPLES as f64, closed, out)
}

/// A coil along `base` over `spans` into `out`: whole loops (a prolate
/// cycloid) on the left of travel, of the wiggle's size, its wavelength
/// fitted to the whole path. A path of no finite length draws nothing.
fn coil_path(
    base: &Base,
    coil: Wiggle,
    spans: &Spans,
    closed: bool,
    out: &mut Vec<PathSegment>,
) -> Result<(), crate::RuleKind> {
    out.clear();
    let length = base.length();
    if !(length.is_finite() && length > 0.0) {
        return Ok(());
    }
    let loops = (length / f64::from(coil.wavelength)).round().max(1.0);
    let a = f64::from(coil.amplitude);
    let f = |t: f64| {
        let phase = std::f64::consts::TAU * loops * t;
        let along = t - a * phase.sin() / length;
        offset_point(base, (a * (1.0 - phase.cos())) as f32, along)
    };
    sampled_path(&f, spans, loops * COIL_SAMPLES as f64, closed, out)
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
    pub fn validate_feynman(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
        for item in series {
            let Some(diagram) = item.feynman() else {
                continue;
            };
            diagram.validate()?;
            if !item.x_column.is_empty() || !item.y_column.is_empty() || item.z_column.is_some() {
                return Err(crate::RuleKind::FeynmanColumns);
            }
            if self.polar().is_some()
                || self.ternary().is_some()
                || self.parallel().is_some()
                || self.view_3d().is_some()
            {
                return Err(crate::RuleKind::FeynmanChart);
            }
            if self.bottom_x.categories.is_some() {
                return Err(crate::RuleKind::FeynmanNumericX);
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
    let own = &series.get(position)?.series_id;
    let at = members().position(|s| s.series_id == *own)?;
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
