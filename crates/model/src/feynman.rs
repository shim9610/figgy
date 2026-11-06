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
use crate::data_config::{MAX_CATEGORY_BAR_LABEL_CHARS, MAX_VALUE_LABEL_OFFSET, SeriesConfig};
use crate::layout::{NudgeReject, NudgeResult, RectF};
use crate::sink::{PathSegment, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use crate::text::{MeasureText, RichText};
use std::f32::consts::PI;

/// The most characters of an id: a text-length limit, not a count.
pub const MAX_FEYNMAN_ID_CHARS: usize = MAX_CATEGORY_BAR_LABEL_CHARS;

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
    #[cfg_attr(feature = "serde", serde(default))]
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
    /// The angle between two lines of a fan, degrees, `0..=80`.
    pub fan_step: f32,
    /// How much a picked line or marker widens, `1..=4`.
    pub picked_width_scale: f32,
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
            fan_step: 30.0,
            picked_width_scale: 2.0,
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
            || !finite_in(self.fan_step, 0.0, 80.0)
        {
            return Err("invalid Feynman marker, arrow, loop or fan size");
        }
        if !finite_in(self.picked_width_scale, 1.0, 4.0) {
            return Err("picked_width_scale must be in 1..=4");
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

fn finite_in(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

fn color_ok(color: Color) -> bool {
    [color.r, color.g, color.b, color.a]
        .iter()
        .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
}

fn id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_FEYNMAN_ID_CHARS
        && !id.chars().any(char::is_control)
}

fn rich_ok(text: Option<&RichText>) -> bool {
    text.is_none_or(|t| t.validate_math().is_ok() && t.within_limits())
}

fn offset_ok(offset: [f32; 2]) -> bool {
    offset
        .iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_VALUE_LABEL_OFFSET)
}

/// Whether every id of `ids` is unique. Sorts a transient list of borrowed
/// ids (rule 3: dropped on return).
fn ids_unique<'a>(ids: impl Iterator<Item = &'a str>) -> bool {
    let mut sorted: Vec<&str> = ids.collect();
    sorted.sort_unstable();
    sorted.windows(2).all(|w| w[0] != w[1])
}

/// The vertex ids of a series sorted with their rows, for lookups by id: a
/// transient index (rule 3: dropped with the borrow).
struct VertexIndex<'a>(Vec<(&'a str, usize)>);

impl<'a> VertexIndex<'a> {
    fn new(series: &'a FeynmanSeries) -> Self {
        let mut ids: Vec<(&str, usize)> = series
            .vertices
            .iter()
            .enumerate()
            .map(|(i, v)| (v.id.as_str(), i))
            .collect();
        ids.sort_unstable();
        Self(ids)
    }

    fn row(&self, id: &str) -> Option<usize> {
        self.0
            .binary_search_by(|(k, _)| (*k).cmp(id))
            .ok()
            .map(|i| self.0[i].1)
    }
}

impl FeynmanSeries {
    /// The rules a series is held to on its own; the chart's rules are
    /// [`Config::validate_feynman`](crate::config::Config::validate_feynman)'s.
    ///
    /// Ids are non-empty, one line of at most [`MAX_FEYNMAN_ID_CHARS`]
    /// characters and unique per list; every propagator names vertices of
    /// the series; positions are finite; bends lie in `-170..=170` degrees;
    /// labels are valid rich text; offsets and the style are in range. No
    /// list has a count limit (rule 10).
    pub fn validate(&self) -> Result<(), &'static str> {
        self.style.validate()?;
        if !self.vertices.iter().all(|v| id_ok(&v.id))
            || !ids_unique(self.vertices.iter().map(|v| v.id.as_str()))
        {
            return Err("Feynman vertex ids must be non-empty, one short line and unique");
        }
        if !self.propagators.iter().all(|p| id_ok(&p.id))
            || !ids_unique(self.propagators.iter().map(|p| p.id.as_str()))
        {
            return Err("Feynman propagator ids must be non-empty, one short line and unique");
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
        let index = VertexIndex::new(self);
        for propagator in &self.propagators {
            if index.row(&propagator.from).is_none() || index.row(&propagator.to).is_none() {
                return Err("a propagator must name two vertices of its series");
            }
            if propagator
                .bend
                .is_some_and(|b| !finite_in(b, -170.0, 170.0))
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
    /// call, which the host reads back.
    pub fn auto_layout(&mut self) -> bool {
        if !self.has_unplaced() {
            return false;
        }
        let positions = self.positions();
        for (vertex, p) in self.vertices.iter_mut().zip(positions) {
            if vertex.at.is_none() {
                vertex.at = Some(p);
            }
        }
        true
    }

    /// The position of every vertex, in row order: its `at`, or the
    /// automatic layout's place for it. Deterministic: identical SSOT gives
    /// bit-identical positions. The list is the caller's (rule 3: a
    /// transient of the raster, the pick and the layout commit).
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
    pub fn positions(&self) -> Vec<[f64; 2]> {
        layout_positions(self)
    }

    /// The bounding box of every vertex position, `[[x0, y0], [x1, y1]]`,
    /// or `None` without vertices: what an axis fit reads (a display
    /// parameter, rule 1).
    pub fn extent(&self) -> Option<[[f64; 2]; 2]> {
        bounding_box(&self.positions())
    }

    /// Move vertex `index` by `(dx, dy)` screen pixels in `space` (the
    /// panel's anchor space, logical pixels): its position is converted
    /// back through the axes and written into `at`. The first drag of a
    /// diagram with unplaced vertices commits the whole layout first, so
    /// no other vertex moves. Rejected for a row past the end, a non-finite
    /// delta or a place the axes cannot convert (outside a logarithmic
    /// domain, in a break gap); nothing changes then.
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
        let positions = self.positions();
        let p = positions[index];
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
        for (vertex, p) in self.vertices.iter_mut().zip(positions) {
            if vertex.at.is_none() {
                vertex.at = Some(p);
            }
        }
        self.vertices[index].at = Some([vx, vy]);
        NudgeResult::Moved
    }

    /// Bend propagator `index` by dragging its middle `(dx, dy)` screen
    /// pixels in `space`: the drag across the chord sets `bend` to the arc
    /// through both ends and the moved middle. Rejected for a row past the
    /// end, a loop, a non-finite delta, a line whose ends are not placed on
    /// screen or a bend past 170 degrees; nothing changes then.
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
        let positions = self.positions();
        let placement = FeynmanPlacement {
            space,
            positions: &positions,
            picked: None,
        };
        let Some(geometry) = placement.propagator_geometry(self, index) else {
            return reject;
        };
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
        if !finite_in(bend, -170.0, 170.0) {
            return reject;
        }
        self.propagators[index].bend = Some(bend);
        NudgeResult::Moved
    }
}

/// The bounding box of `points`, `None` for none.
fn bounding_box(points: &[[f64; 2]]) -> Option<[[f64; 2]; 2]> {
    points.iter().fold(None, |acc, p| {
        Some(match acc {
            None => [*p, *p],
            Some([lo, hi]) => [
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            ],
        })
    })
}

/// The automatic layout ([`FeynmanSeries::positions`]).
fn layout_positions(series: &FeynmanSeries) -> Vec<[f64; 2]> {
    let n = series.vertices.len();
    let mut positions: Vec<[f64; 2]> = vec![[0.0; 2]; n];
    let mut fixed = vec![false; n];
    for (i, vertex) in series.vertices.iter().enumerate() {
        if let Some(at) = vertex.at {
            positions[i] = at;
            fixed[i] = true;
        }
    }
    // The frame: the placed vertices' box, or the unit square.
    let placed: Vec<[f64; 2]> = series.vertices.iter().filter_map(|v| v.at).collect();
    let frame = match bounding_box(&placed) {
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
        let t = [
            frame[0][0] + time * (frame[1][0] - frame[0][0]),
            frame[0][1] + across * (frame[1][1] - frame[0][1]),
        ];
        match series.layout.direction {
            TimeDirection::LeftToRight => t,
            TimeDirection::BottomToTop => [
                frame[0][0] + across * (frame[1][0] - frame[0][0]),
                frame[0][1] + time * (frame[1][1] - frame[0][1]),
            ],
        }
    };
    // The legs on the frame's early and late edges, in row order.
    for (role, time) in [(VertexRole::Incoming, 0.0), (VertexRole::Outgoing, 1.0)] {
        let legs = || {
            series
                .vertices
                .iter()
                .enumerate()
                .filter(|(i, v)| v.role == role && !fixed[*i])
        };
        let count = legs().count();
        let rows: Vec<usize> = legs().map(|(i, _)| i).collect();
        for (k, i) in rows.into_iter().enumerate() {
            // Top to bottom on screen: the first leg is at the top.
            let across = 1.0 - (k as f64 + 1.0) / (count as f64 + 1.0);
            positions[i] = at(time, across);
            fixed[i] = true;
        }
    }
    // The neighbours of each vertex (a propagator per occurrence), as
    // offsets into one list: the CSR adjacency, transient.
    let index = VertexIndex::new(series);
    let ends: Vec<(usize, usize)> = series
        .propagators
        .iter()
        .filter_map(|p| Some((index.row(&p.from)?, index.row(&p.to)?)))
        .filter(|(a, b)| a != b)
        .collect();
    let mut offsets = vec![0usize; n + 1];
    for &(a, b) in &ends {
        offsets[a + 1] += 1;
        offsets[b + 1] += 1;
    }
    for i in 0..n {
        offsets[i + 1] += offsets[i];
    }
    let mut fill = offsets.clone();
    let mut neighbours = vec![0usize; offsets[n]];
    for &(a, b) in &ends {
        neighbours[fill[a]] = b;
        fill[a] += 1;
        neighbours[fill[b]] = a;
        fill[b] += 1;
    }
    let around = |i: usize| &neighbours[offsets[i]..offsets[i + 1]];
    // The free vertices a fixed one reaches; the rest are bubbles.
    let mut anchored = fixed.clone();
    let mut queue: Vec<usize> = (0..n).filter(|&i| fixed[i]).collect();
    while let Some(i) = queue.pop() {
        for &j in around(i) {
            if !anchored[j] {
                anchored[j] = true;
                queue.push(j);
            }
        }
    }
    // The barycentric embedding of the anchored free vertices: solve
    // L_ff x = L_fp p for each coordinate by conjugate gradients.
    let free: Vec<usize> = (0..n).filter(|&i| !fixed[i] && anchored[i]).collect();
    if !free.is_empty() {
        let mut slot = vec![usize::MAX; n];
        for (k, &i) in free.iter().enumerate() {
            slot[i] = k;
        }
        let multiply = |x: &[f64], out: &mut [f64]| {
            for (k, &i) in free.iter().enumerate() {
                let mut v = around(i).len() as f64 * x[k];
                for &j in around(i) {
                    if slot[j] != usize::MAX {
                        v -= x[slot[j]];
                    }
                }
                out[k] = v;
            }
        };
        for axis in 0..2 {
            let rhs: Vec<f64> = free
                .iter()
                .map(|&i| {
                    around(i)
                        .iter()
                        .filter(|&&j| slot[j] == usize::MAX)
                        .map(|&j| positions[j][axis])
                        .sum()
                })
                .collect();
            let solution = conjugate_gradient(&multiply, &rhs, free.len());
            for (k, &i) in free.iter().enumerate() {
                positions[i][axis] = solution[k];
            }
        }
    }
    // The bubbles: each connected group on a circle about the frame's
    // centre, in row order.
    let mut placed_bubble = anchored;
    let centre = at(0.5, 0.5);
    let radius = 0.25 * (frame[1][0] - frame[0][0]).min(frame[1][1] - frame[0][1]);
    let mut group = 0usize;
    for start in 0..n {
        if placed_bubble[start] {
            continue;
        }
        let mut members = vec![start];
        placed_bubble[start] = true;
        let mut k = 0;
        while k < members.len() {
            for &j in around(members[k]) {
                if !placed_bubble[j] {
                    placed_bubble[j] = true;
                    members.push(j);
                }
            }
            k += 1;
        }
        members.sort_unstable();
        let shift = 0.1 * group as f64 * (frame[1][0] - frame[0][0]);
        for (k, &i) in members.iter().enumerate() {
            let angle = std::f64::consts::TAU * k as f64 / members.len() as f64;
            positions[i] = [
                centre[0] + shift + radius * angle.cos(),
                centre[1] + radius * angle.sin(),
            ];
        }
        group += 1;
    }
    spread_coincident(&mut positions, &fixed, series.layout.direction, frame);
    positions
}

/// Solve `A x = b` for the symmetric positive definite `A` that `multiply`
/// applies, from zero: conjugate gradients to a relative residual of
/// 1e-12. In exact arithmetic it ends within `n` steps; the bound below is
/// that guarantee with a margin for rounding, a numerical setting rather
/// than a cap on the data.
fn conjugate_gradient(multiply: &dyn Fn(&[f64], &mut [f64]), b: &[f64], n: usize) -> Vec<f64> {
    let mut x = vec![0.0; n];
    let mut r = b.to_vec();
    let mut p = r.clone();
    let mut ap = vec![0.0; n];
    let norm = |v: &[f64]| v.iter().map(|a| a * a).sum::<f64>();
    let mut rr = norm(&r);
    let target = rr * 1e-24;
    for _ in 0..2 * n + 16 {
        if rr <= target || rr == 0.0 {
            break;
        }
        multiply(&p, &mut ap);
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap <= 0.0 || !pap.is_finite() {
            break;
        }
        let alpha = rr / pap;
        for k in 0..n {
            x[k] += alpha * p[k];
            r[k] -= alpha * ap[k];
        }
        let next = norm(&r);
        let beta = next / rr;
        for k in 0..n {
            p[k] = r[k] + beta * p[k];
        }
        rr = next;
    }
    x
}

/// Spread the unpinned vertices that share a point across the time
/// direction, in row order, a tenth of the frame apart.
fn spread_coincident(
    positions: &mut [[f64; 2]],
    fixed_legs: &[bool],
    direction: TimeDirection,
    frame: [[f64; 2]; 2],
) {
    let size = [frame[1][0] - frame[0][0], frame[1][1] - frame[0][1]];
    let tolerance = 1e-9 * size[0].max(size[1]);
    let mut order: Vec<usize> = (0..positions.len()).collect();
    order.sort_by(|&a, &b| {
        positions[a][0]
            .total_cmp(&positions[b][0])
            .then(positions[a][1].total_cmp(&positions[b][1]))
            .then(a.cmp(&b))
    });
    let mut k = 0;
    while k < order.len() {
        let mut end = k + 1;
        while end < order.len()
            && (positions[order[end]][0] - positions[order[k]][0]).abs() <= tolerance
            && (positions[order[end]][1] - positions[order[k]][1]).abs() <= tolerance
        {
            end += 1;
        }
        let mut run: Vec<usize> = order[k..end].to_vec();
        run.sort_unstable();
        let mut moved = 0;
        for &i in &run {
            if fixed_legs[i] {
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
pub enum Base {
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
    pub fn point(&self, t: f32) -> [f32; 2] {
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
    pub fn tangent(&self, t: f32) -> [f32; 2] {
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
    pub fn length(&self) -> f32 {
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
pub struct PropagatorGeometry {
    pub base: Base,
    pub ends: ([f32; 2], [f32; 2]),
    pub is_loop: bool,
    pub label_side: f32,
}

/// What a pick of a Feynman series hit, by row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeynmanHit {
    Vertex(usize),
    Propagator(usize),
    Label(FeynmanLabelPart, usize),
}

/// Where a Feynman series is placed: the panel's anchor space (logical
/// pixels), every vertex's position in data units ([`FeynmanSeries::positions`])
/// and the part drawn picked. Everything here is a display parameter
/// (rule 1).
pub struct FeynmanPlacement<'a> {
    pub space: &'a dyn AnnotationSpace,
    pub positions: &'a [[f64; 2]],
    pub picked: Option<FeynmanHit>,
}

/// The gap between a part and its label, logical pixels.
const LABEL_GAP: f32 = 4.0;
/// The gap between a line and its momentum arrow, logical pixels.
const MOMENTUM_GAP: f32 = 6.0;
/// The share of a line its momentum arrow spans, centred.
const MOMENTUM_SPAN: f32 = 0.4;
/// Samples per half period of a wave and per loop of a coil.
const WAVE_SAMPLES: usize = 6;
const COIL_SAMPLES: usize = 12;
/// The segments of a marker circle.
const CIRCLE_SEGMENTS: usize = 24;

impl FeynmanPlacement<'_> {
    /// The screen point of vertex `index`, or `None` where the axes do not
    /// place it.
    pub fn vertex_px(&self, index: usize) -> Option<[f32; 2]> {
        let p = self.positions.get(index)?;
        Some([
            self.space.value_to_px(AnnotationAxis::X, p[0])?,
            self.space.value_to_px(AnnotationAxis::Y, p[1])?,
        ])
    }

    /// The bend of propagator `index` in degrees: its own, or its place in
    /// the fan of the lines that join the same two vertices (by row, an
    /// order independent of their direction).
    fn bend(&self, series: &FeynmanSeries, index: usize) -> f32 {
        let propagator = &series.propagators[index];
        if let Some(bend) = propagator.bend {
            return bend;
        }
        fn key(p: &Propagator) -> (&str, &str) {
            if p.from <= p.to {
                (p.from.as_str(), p.to.as_str())
            } else {
                (p.to.as_str(), p.from.as_str())
            }
        }
        let own = key(propagator);
        let fan = |p: &&Propagator| key(p) == own && p.bend.is_none();
        let count = series.propagators.iter().filter(fan).count();
        let rank = series.propagators[..index].iter().filter(fan).count();
        let step = series.style.fan_step;
        let canonical = (count as f32 - 1.0) / 2.0 - rank as f32;
        // The fan is laid out for the canonical direction; a line running
        // the other way bends the other way to land on the same arc.
        let sign = if propagator.from <= propagator.to {
            1.0
        } else {
            -1.0
        };
        canonical * step * sign
    }

    /// The tadpoles at vertex `vertex` before propagator `index`, and their
    /// count: loops at one vertex turn about it.
    fn loop_rank(&self, series: &FeynmanSeries, index: usize) -> (usize, usize) {
        let own = &series.propagators[index].from;
        let is_loop = |p: &&Propagator| p.from == p.to && p.from == *own;
        (
            series.propagators[..index].iter().filter(is_loop).count(),
            series.propagators.iter().filter(is_loop).count(),
        )
    }

    /// The direction away from the neighbours of vertex `row` on screen,
    /// or up when they balance.
    fn outward(&self, series: &FeynmanSeries, row: usize) -> [f32; 2] {
        let Some(p) = self.vertex_px(row) else {
            return [0.0, -1.0];
        };
        let id = &series.vertices[row].id;
        let mut sum = [0.0f32; 2];
        let mut count = 0;
        for propagator in &series.propagators {
            let other = if propagator.from == *id && propagator.to != *id {
                &propagator.to
            } else if propagator.to == *id && propagator.from != *id {
                &propagator.from
            } else {
                continue;
            };
            if let Some(q) = series.vertex_row(other).and_then(|r| self.vertex_px(r)) {
                sum = add(sum, unit(sub(q, p)));
                count += 1;
            }
        }
        if count == 0 || len(sum) < 1e-3 {
            [0.0, -1.0]
        } else {
            unit(scale(sum, -1.0))
        }
    }

    /// The geometry of propagator `index`, or `None` when an end is not
    /// placed on screen.
    pub fn propagator_geometry(
        &self,
        series: &FeynmanSeries,
        index: usize,
    ) -> Option<PropagatorGeometry> {
        let propagator = series.propagators.get(index)?;
        let from = series.vertex_row(&propagator.from)?;
        let to = series.vertex_row(&propagator.to)?;
        let a = self.vertex_px(from)?;
        if from == to {
            let (rank, count) = self.loop_rank(series, index);
            let out = self.outward(series, from);
            let base_angle = out[1].atan2(out[0]);
            let turn = if count > 1 {
                (rank as f32 - (count as f32 - 1.0) / 2.0) * (PI / count as f32)
            } else {
                0.0
            };
            let angle = base_angle + turn;
            let radius = series.style.loop_radius;
            let centre = add(a, [radius * angle.cos(), radius * angle.sin()]);
            let start = (a[1] - centre[1]).atan2(a[0] - centre[0]);
            return Some(PropagatorGeometry {
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
        let b = self.vertex_px(to)?;
        let chord = sub(b, a);
        let c = len(chord);
        let bend = self.bend(series, index).to_radians();
        if c <= 0.0 || bend.abs() < 1e-3 {
            return Some(PropagatorGeometry {
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
        Some(PropagatorGeometry {
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
    fn envelope(&self, series: &FeynmanSeries, index: usize) -> f32 {
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

    fn picked_scale(&self, series: &FeynmanSeries, hit: FeynmanHit) -> f32 {
        if self.picked == Some(hit) {
            series.style.picked_width_scale
        } else {
            1.0
        }
    }

    /// Emit propagator `index`'s strokes and arrows.
    fn emit_propagator(&self, series: &FeynmanSeries, index: usize, sink: &mut dyn Sink) {
        let Some(geometry) = self.propagator_geometry(series, index) else {
            return;
        };
        let propagator = &series.propagators[index];
        let style = &series.style;
        let widen = self.picked_scale(series, FeynmanHit::Propagator(index));
        let width = propagator.width_px.unwrap_or(style.width) * widen;
        let color = propagator.color.unwrap_or(style.color);
        let base = geometry.base;
        let stroke = StrokeStyle {
            width,
            color,
            dash: propagator.particle.dash(),
            tag: "feynman_propagator",
        };
        let mut path = Vec::new();
        match propagator.particle {
            ParticleLine::Photon | ParticleLine::ChargedBoson => {
                wave_path(&base, style.wave, 0.0, geometry.is_loop, &mut path);
                sink.path(Slot::Above, &path, &stroke);
            }
            ParticleLine::Graviton => {
                for shift in [-width, width] {
                    path.clear();
                    wave_path(&base, style.wave, shift, geometry.is_loop, &mut path);
                    sink.path(Slot::Above, &path, &stroke);
                }
            }
            ParticleLine::Gluon => {
                coil_path(&base, style.coil, geometry.is_loop, &mut path);
                sink.path(Slot::Above, &path, &stroke);
            }
            ParticleLine::DoubleLine => {
                for shift in [-width, width] {
                    path.clear();
                    offset_path(&base, shift, 0.0, 1.0, geometry.is_loop, &mut path);
                    sink.path(Slot::Above, &path, &stroke);
                }
            }
            _ => match base {
                Base::Line { a, b } => sink.stroke(Slot::Above, a, b, &stroke),
                Base::Arc { .. } => {
                    offset_path(&base, 0.0, 0.0, 1.0, geometry.is_loop, &mut path);
                    sink.path(Slot::Above, &path, &stroke);
                }
            },
        }
        let head = |sink: &mut dyn Sink, t: f32, reversed: bool| {
            let mut dir = base.tangent(t);
            if reversed {
                dir = scale(dir, -1.0);
            }
            let length = style.head_length * widen;
            let half = style.head_width * widen / 2.0;
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
            let shift = side * (self.envelope(series, index) + MOMENTUM_GAP);
            let (t0, t1) = (0.5 - MOMENTUM_SPAN / 2.0, 0.5 + MOMENTUM_SPAN / 2.0);
            path.clear();
            offset_path(&base, shift, t0, t1, false, &mut path);
            let thin = StrokeStyle {
                width: style.width.min(1.5),
                color,
                dash: &[],
                tag: "feynman_momentum",
            };
            sink.path(Slot::Above, &path, &thin);
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
    }

    /// Emit vertex `index`'s marker.
    fn emit_vertex(&self, series: &FeynmanSeries, index: usize, sink: &mut dyn Sink) {
        let Some(p) = self.vertex_px(index) else {
            return;
        };
        let style = &series.style;
        let widen = self.picked_scale(series, FeynmanHit::Vertex(index));
        let line = StrokeStyle {
            width: style.width * widen,
            color: style.color,
            dash: &[],
            tag: "feynman_vertex",
        };
        let marker = series.vertices[index].drawn_marker();
        match marker {
            VertexMarker::Hidden => {
                // A picked leg end shows where it is.
                if self.picked == Some(FeynmanHit::Vertex(index)) {
                    sink.polygon(
                        Slot::Above,
                        &circle(p, style.dot_radius * widen),
                        Some(style.color),
                        None,
                    );
                }
            }
            VertexMarker::Dot => {
                sink.polygon(
                    Slot::Above,
                    &circle(p, style.dot_radius * widen),
                    Some(style.color),
                    None,
                );
            }
            VertexMarker::Blob { radius_px, fill } => {
                sink.polygon(
                    Slot::Above,
                    &circle(p, radius_px * widen),
                    Some(fill.unwrap_or(style.color)),
                    Some(&line),
                );
            }
            VertexMarker::Cross | VertexMarker::CircledCross => {
                let r = (style.dot_radius * 2.0).max(4.0) * widen;
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
                    let mut path = Vec::new();
                    let base = Base::Arc {
                        centre: p,
                        radius: r,
                        start: 0.0,
                        sweep: 2.0 * PI,
                    };
                    offset_path(&base, 0.0, 0.0, 1.0, true, &mut path);
                    sink.path(Slot::Above, &path, &line);
                }
            }
        }
    }

    /// The radius a vertex's marker covers, for its label and its pick.
    fn marker_radius(&self, series: &FeynmanSeries, index: usize) -> f32 {
        let style = &series.style;
        match series.vertices[index].drawn_marker() {
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
    pub fn label<'t>(
        &self,
        series: &'t FeynmanSeries,
        part: FeynmanLabelPart,
        index: usize,
        measure: &dyn MeasureText,
    ) -> Option<PlacedText<'t>> {
        let (text, anchor, direction, offset) = match part {
            FeynmanLabelPart::Vertex => {
                let vertex = series.vertices.get(index)?;
                let text = vertex.label.as_ref()?;
                let p = self.vertex_px(index)?;
                let out = self.outward(series, index);
                let reach = self.marker_radius(series, index) + LABEL_GAP;
                (text, add(p, scale(out, reach)), out, vertex.label_offset)
            }
            FeynmanLabelPart::Propagator => {
                let propagator = series.propagators.get(index)?;
                let text = propagator.label.as_ref()?;
                let geometry = self.propagator_geometry(series, index)?;
                let side = geometry.label_side;
                let n = scale(left_normal(geometry.base.tangent(0.5)), side);
                let reach = self.label_reach(series, index, side);
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
                let geometry = self.propagator_geometry(series, index)?;
                let side = match momentum.side {
                    MomentumSide::Left => 1.0,
                    MomentumSide::Right => -1.0,
                };
                let shift = side
                    * (self.envelope(series, index)
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
    fn label_reach(&self, series: &FeynmanSeries, index: usize, side: f32) -> f32 {
        let propagator = &series.propagators[index];
        let mut reach = self.envelope(series, index);
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

    /// Emit every drawn part of `series` into `sink`, in the
    /// [`Slot::Above`] slot: the propagators in row order, the vertex
    /// markers over them, then every label. The picked part is drawn
    /// `style.picked_width_scale` times as wide. Nothing is stored; the
    /// caller's sink paints, bounds or hit tests the parts.
    pub fn emit(&self, series: &FeynmanSeries, measure: &dyn MeasureText, sink: &mut dyn Sink) {
        for index in 0..series.propagators.len() {
            self.emit_propagator(series, index, sink);
        }
        for index in 0..series.vertices.len() {
            self.emit_vertex(series, index, sink);
        }
        for part in [
            FeynmanLabelPart::Propagator,
            FeynmanLabelPart::Momentum,
            FeynmanLabelPart::Vertex,
        ] {
            for index in 0..series.part_len(part) {
                if let Some(text) = self.label(series, part, index, measure) {
                    sink.text(Slot::Above, &text);
                }
            }
        }
    }

    /// The drawn part of `series` nearest `cursor`, within `max_distance`
    /// of its ink, and that distance (zero on the part). Walks the SSOT
    /// through the same placement the emit draws with, so the pick is the
    /// drawn geometry; nothing is registered per row (rule 3). On a tie a
    /// part drawn later wins: labels over markers over lines.
    pub fn pick(
        &self,
        series: &FeynmanSeries,
        measure: &dyn MeasureText,
        cursor: [f32; 2],
        max_distance: f32,
    ) -> Option<(FeynmanHit, f32)> {
        let mut best: Option<(FeynmanHit, f32)> = None;
        let mut offer = |hit: FeynmanHit, distance: f32| {
            if distance <= max_distance && best.is_none_or(|(_, d)| distance <= d) {
                best = Some((hit, distance));
            }
        };
        for index in 0..series.propagators.len() {
            if let Some(geometry) = self.propagator_geometry(series, index) {
                let edge = geometry.base.distance(cursor) - self.envelope(series, index);
                offer(FeynmanHit::Propagator(index), edge.max(0.0));
            }
        }
        for index in 0..series.vertices.len() {
            if let Some(p) = self.vertex_px(index) {
                let edge = len(sub(cursor, p)) - self.marker_radius(series, index);
                offer(FeynmanHit::Vertex(index), edge.max(0.0));
            }
        }
        for part in [
            FeynmanLabelPart::Propagator,
            FeynmanLabelPart::Momentum,
            FeynmanLabelPart::Vertex,
        ] {
            for index in 0..series.part_len(part) {
                if let Some(text) = self.label(series, part, index, measure) {
                    offer(
                        FeynmanHit::Label(part, index),
                        rect_distance(cursor, text.rect()),
                    );
                }
            }
        }
        best
    }
}

fn lerp_f(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The point of `base` at `t`, `shift` pixels to its left.
fn offset_point(base: &Base, shift: f32, t: f32) -> [f32; 2] {
    add(base.point(t), scale(left_normal(base.tangent(t)), shift))
}

/// The polygon of a circle of `radius` about `centre`.
fn circle(centre: [f32; 2], radius: f32) -> Vec<[f32; 2]> {
    (0..CIRCLE_SEGMENTS)
        .map(|k| {
            let angle = 2.0 * PI * k as f32 / CIRCLE_SEGMENTS as f32;
            [
                centre[0] + radius * angle.cos(),
                centre[1] + radius * angle.sin(),
            ]
        })
        .collect()
}

/// Append to `out` the smooth path through `f(t)` for `t` from `t0` to
/// `t1` in `samples` steps: cubic Hermite segments with the curve's own
/// derivatives (taken numerically), as Bézier segments. `closed` closes
/// the path.
fn sampled_path(
    f: &dyn Fn(f32) -> [f32; 2],
    t0: f32,
    t1: f32,
    samples: usize,
    closed: bool,
    out: &mut Vec<PathSegment>,
) {
    let samples = samples.max(1);
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
}

/// The base path from `t0` to `t1`, `shift` pixels to its left.
fn offset_path(
    base: &Base,
    shift: f32,
    t0: f32,
    t1: f32,
    closed: bool,
    out: &mut Vec<PathSegment>,
) {
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
    );
}

/// A wave along `base`: a sine of the wiggle's amplitude, its wavelength
/// fitted so that whole half periods span the path, `shift` pixels to the
/// left. A loop's wave closes on itself with whole periods.
fn wave_path(base: &Base, wave: Wiggle, shift: f32, closed: bool, out: &mut Vec<PathSegment>) {
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
    sampled_path(&f, 0.0, 1.0, halves * WAVE_SAMPLES, closed, out);
}

/// A coil along `base`: whole loops (a prolate cycloid) on the left of
/// travel, of the wiggle's size, its wavelength fitted to the path.
fn coil_path(base: &Base, coil: Wiggle, closed: bool, out: &mut Vec<PathSegment>) {
    let length = base.length().max(1e-3);
    let loops = ((length / coil.wavelength).round() as usize).max(1);
    let a = coil.amplitude;
    let f = |t: f32| {
        let phase = 2.0 * PI * loops as f32 * t;
        let along = t - a * phase.sin() / length;
        offset_point(base, a * (1.0 - phase.cos()), along)
    };
    sampled_path(&f, 0.0, 1.0, loops * COIL_SAMPLES, closed, out);
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
