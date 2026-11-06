//! Energy-level (Grotrian) diagrams: [`DataRenderType::EnergyLevels`] (#203).
//!
//! An energy-level series is DataLess: every value it draws is SSOT
//! configuration, held here and serialized with the document, and none comes
//! through a `ColumnSource`. Its records follow the NIST Atomic Spectra
//! Database exports field for field, so a host that parses a levels file and
//! a lines file fills them without losing a column; the parsing is the
//! host's (the renderer's `nist_grotrian` example shows one way).
//!
//! - [`EnergyLevel`]: one row of a levels file. Its `column` names a category
//!   of the chart's categorical `bottom_x` (a term column such as `3P*`); the
//!   level is drawn as a short horizontal stroke across that column's slot at
//!   its `energy` on `left_y`. A level whose column the axis does not list is
//!   not drawn in that panel, so an inset lists only the columns it shows.
//! - [`IonizationLimit`]: the `Limit` row of a levels file.
//! - [`Transition`]: one row of a lines file the host chose to draw, as an
//!   arrow between its two levels. Only listed transitions are drawn.
//!
//! The host computes every value the reader reads as data (rule 1): the
//! g-weighted centroid of a fine-structure multiplet is a level row of role
//! [`LevelRole::Centroid`], written by the host; the energy unit is whatever
//! the host stored, named by the axis title. The renderer computes only
//! appearance: pixel positions, mapped colours and arrowheads, at raster
//! time, never stored.
//!
//! The series is drawn through the CPU sink, as annotations and chrome are,
//! and the GPU only composites the raster (approved on #203). The SVG and
//! PDF export write the same emit as vector parts (#234). It has no count
//! limit: every list is drawn to its end (rule 10).
//!
//! [`DataRenderType::EnergyLevels`]: crate::data_config::DataRenderType::EnergyLevels

use crate::annotation::{Annotation, AnnotationAxis, AnnotationKind, AnnotationLineStyle};
use crate::color::Color;
use crate::data_config::{MAX_CATEGORY_BAR_LABEL_CHARS, MAX_VALUE_LABEL_OFFSET, SeriesConfig};
use crate::layout::{NudgeReject, NudgeResult};
use crate::text::RichText;

/// The most characters of one text field of a level, limit or transition
/// (configuration, term, J, reference, intensity, …): a text-length limit,
/// not a count.
pub const MAX_ENERGY_LEVEL_FIELD_CHARS: usize = MAX_CATEGORY_BAR_LABEL_CHARS;

/// An energy-level diagram: its levels, ionization limits and transitions,
/// and their look. See the [module documentation](self).
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EnergyLevelSeries {
    /// The levels, in the host's order (a levels file's row order). A
    /// series that shows another one's levels (`source`) keeps this empty.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub levels: Vec<EnergyLevel>,
    /// The ionization limits.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub limits: Vec<IonizationLimit>,
    /// The transitions the host chose to draw.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub transitions: Vec<Transition>,
    /// The look of the strokes, arrows and labels.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "EnergyLevelStyle::is_default")
    )]
    pub style: EnergyLevelStyle,
    /// Which levels are drawn: the main view or a fine-structure view.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "LevelView::is_main")
    )]
    pub show: LevelView,
    /// Draw the levels, limits and transitions of another energy-level
    /// series of the document (its `series_id`), so an inset shows them
    /// without a second copy. A series with a source holds no records of
    /// its own.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub source: Option<String>,
}

/// One level: a row of a NIST levels file.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EnergyLevel {
    /// Unique within the series; transitions and multiplets name levels by
    /// it.
    pub id: String,
    /// The category id of `bottom_x` the level is drawn in.
    pub column: String,
    /// NIST `Configuration`, e.g. `1s.2p`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub configuration: String,
    /// NIST `Term`, e.g. `3P*`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub term: String,
    /// NIST `J` as written: `2`, `3/2`, `0,1,2` for an unresolved
    /// multiplet, or `---`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub j: String,
    /// NIST `g`, the statistical weight.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub g: Option<f64>,
    /// NIST `Prefix`: `[` for a level derived from a theoretical value,
    /// `(` for an interpolated one.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub prefix: String,
    /// The level energy, in the unit the host chose (`left_y`'s).
    pub energy: f64,
    /// NIST `Suffix`, the counterpart of `prefix`, or a `?` / `+x` mark.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub suffix: String,
    /// NIST `Uncertainty`, in the energy unit.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub uncertainty: Option<f64>,
    /// NIST `Reference`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub reference: String,
    /// What the row is in a fine-structure multiplet.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "LevelRole::is_level")
    )]
    pub role: LevelRole,
    /// The drawn label, beside the stroke; none draws no label. The host
    /// writes it (from `configuration`, a term symbol in TeX, …).
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

/// What a level row is in a fine-structure multiplet.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LevelRole {
    /// A level drawn in the main view: a single J, or an unresolved
    /// multiplet as NIST lists it.
    #[default]
    Level,
    /// The host's centroid of a resolved multiplet, drawn in the main view
    /// in place of its components.
    Centroid,
    /// One J component of a resolved multiplet, drawn in a
    /// [`LevelView::Components`] view in place of its centroid; `multiplet`
    /// is the id of its centroid row.
    Component { multiplet: String },
}

impl LevelRole {
    #[cfg(feature = "serde")]
    fn is_level(&self) -> bool {
        *self == LevelRole::Level
    }

    /// Whether a row of this role is drawn in `view`.
    pub fn drawn_in(&self, view: LevelView) -> bool {
        match view {
            LevelView::Main => !matches!(self, LevelRole::Component { .. }),
            LevelView::Components => *self != LevelRole::Centroid,
        }
    }
}

/// Which levels a series draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LevelView {
    /// Levels and multiplet centroids; fine-structure components are not
    /// drawn.
    #[default]
    Main,
    /// Levels and fine-structure components, the resolved view an inset
    /// magnifies; centroids are not drawn. The inset's own categories and
    /// `left_y` range choose the window.
    Components,
}

impl LevelView {
    #[cfg(feature = "serde")]
    fn is_main(&self) -> bool {
        *self == LevelView::Main
    }
}

/// An ionization limit: the `Limit` row of a NIST levels file. Drawn as a
/// line across the plot at `energy`; the continuum above it is drawn by the
/// annotations [`EnergyLevelSeries::continuum_annotations`] writes.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IonizationLimit {
    /// Unique among the series' limits.
    pub id: String,
    /// The limit energy, in the levels' unit.
    pub energy: f64,
    /// The ion state NIST names in the configuration column, e.g.
    /// `He II (1s 2S<1/2>)`, drawn at the line's end; none draws no label.
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
    /// NIST `Uncertainty`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub uncertainty: Option<f64>,
    /// NIST `Reference`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub reference: String,
}

/// One transition the host chose to draw: a row of a NIST lines file. Its
/// lower and upper energies, configurations, terms, J and g are those of the
/// two levels it names, so they are not stored twice.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Transition {
    /// Unique among the series' transitions.
    pub id: String,
    /// The id of the lower level (NIST `i`).
    pub lower: String,
    /// The id of the upper level (NIST `k`).
    pub upper: String,
    /// The drawn label beside the arrow (a wavelength the host wrote); none
    /// draws no label.
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
    /// A host value mapped to the arrow's colour through `Config.colorbar`
    /// when [`TransitionArrowStyle::color_by_value`] is set (an `Aki` or an
    /// intensity the host chose).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub value: Option<f64>,
    /// The arrow's horizontal shift inside the slot, as a fraction of the
    /// slot width in `-0.5..=0.5`, so arrows leaving one level do not
    /// overlap.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_zero"))]
    pub x_offset: f32,
    /// NIST `obs_wl` (observed wavelength) in the host's unit.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub observed_wavelength: Option<f64>,
    /// NIST `unc_obs_wl`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub observed_uncertainty: Option<f64>,
    /// NIST `ritz_wl` (Ritz wavelength).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub ritz_wavelength: Option<f64>,
    /// NIST `unc_ritz_wl`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub ritz_uncertainty: Option<f64>,
    /// NIST `intens` as written, flags included (`1*`, `50 bl`).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub intensity: String,
    /// NIST `Aki`, the transition probability, in s⁻¹.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub aki: Option<f64>,
    /// NIST `Acc`, the accuracy class of `aki` (`AAA`, `B+`, …).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub accuracy: String,
    /// NIST `Type`: empty for an electric dipole line, `M1`, `E2`, …
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub line_type: String,
    /// NIST `tp_ref`, the transition probability reference.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub tp_reference: String,
    /// NIST `line_ref`, the line reference.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub line_reference: String,
}

/// The look of an energy-level series.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EnergyLevelStyle {
    /// The level strokes.
    #[cfg_attr(feature = "serde", serde(default))]
    pub level: LevelStrokeStyle,
    /// The ionization limit lines.
    #[cfg_attr(feature = "serde", serde(default))]
    pub limit: AnnotationLineStyle,
    /// The transition arrows.
    #[cfg_attr(feature = "serde", serde(default))]
    pub transition: TransitionArrowStyle,
    /// Where a level's label sits against its stroke.
    #[cfg_attr(feature = "serde", serde(default))]
    pub label_side: LevelLabelSide,
    /// How much a picked stroke or arrow widens: its width times this, in
    /// `1..=4`.
    #[cfg_attr(feature = "serde", serde(default = "default_picked_scale"))]
    pub picked_width_scale: f32,
}

impl Default for EnergyLevelStyle {
    fn default() -> Self {
        Self {
            level: LevelStrokeStyle::default(),
            limit: AnnotationLineStyle {
                color: Color::from_rgb8(60, 60, 140),
                width: 1.5,
                style: crate::line::LineStylePreset::Dash,
            },
            transition: TransitionArrowStyle::default(),
            label_side: LevelLabelSide::default(),
            picked_width_scale: default_picked_scale(),
        }
    }
}

impl EnergyLevelStyle {
    #[cfg(feature = "serde")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn default_picked_scale() -> f32 {
    2.0
}

/// The look of the level strokes.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LevelStrokeStyle {
    /// The stroke colour, opaque: the series colour, its legend swatch and
    /// the colour a palette cycle sets.
    pub color: Color,
    /// The stroke width in logical pixels, `0.5..=24`.
    pub width: f32,
    /// The share of the column's slot a stroke spans, `0.05..=1`.
    pub slot_fraction: f32,
}

impl Default for LevelStrokeStyle {
    fn default() -> Self {
        Self {
            color: Color::from_rgb8(40, 40, 40),
            width: 2.0,
            slot_fraction: 0.7,
        }
    }
}

/// The look of the transition arrows.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TransitionArrowStyle {
    /// The arrow colour when it is not mapped.
    pub color: Color,
    /// The shaft width in logical pixels, `0.5..=24`.
    pub width: f32,
    /// The head length in logical pixels, `0..=64`; zero draws no head.
    pub head_length: f32,
    /// The head width in logical pixels, `0..=64`.
    pub head_width: f32,
    /// Which end carries the head.
    pub direction: TransitionDirection,
    /// Map each transition's `value` to its colour through
    /// `Config.colorbar`; a transition without a value keeps `color`.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub color_by_value: bool,
}

impl Default for TransitionArrowStyle {
    fn default() -> Self {
        Self {
            color: Color::from_rgb8(200, 120, 0),
            width: 1.5,
            head_length: 9.0,
            head_width: 7.0,
            direction: TransitionDirection::Emission,
            color_by_value: false,
        }
    }
}

/// Which end of a transition arrow carries the head.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TransitionDirection {
    /// From the upper level down to the lower one.
    #[default]
    Emission,
    /// From the lower level up to the upper one.
    Absorption,
    /// No head: a line between the levels.
    None,
}

/// Where a level's label sits against its stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LevelLabelSide {
    /// Past the stroke's right end, centred on it.
    #[default]
    Right,
    /// Before the stroke's left end, centred on it.
    Left,
    /// Above the stroke's middle.
    Above,
}

/// Which list of an energy-level series a value label belongs to: the
/// `part` of a [`PickedDataRef::ValueLabel`](crate::config::PickedDataRef::ValueLabel)
/// on such a series, whose `category_index` is then the row in that list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum LabelPart {
    Level,
    Transition,
    Limit,
}

#[cfg(feature = "serde")]
fn is_zero_offset(offset: &[f32; 2]) -> bool {
    *offset == [0.0; 2]
}

#[cfg(feature = "serde")]
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

#[cfg(feature = "serde")]
fn is_false(v: &bool) -> bool {
    !*v
}

fn finite_in(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

fn text_ok(text: &str) -> bool {
    text.chars().count() <= MAX_ENERGY_LEVEL_FIELD_CHARS && !text.chars().any(char::is_control)
}

fn rich_ok(text: &Option<RichText>) -> bool {
    text.as_ref()
        .is_none_or(|t| t.validate_math().is_ok() && t.within_limits())
}

fn offset_ok(offset: [f32; 2]) -> bool {
    offset
        .iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_VALUE_LABEL_OFFSET)
}

fn color_ok(color: Color) -> bool {
    [color.r, color.g, color.b, color.a]
        .iter()
        .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
}

fn non_negative(v: Option<f64>) -> bool {
    v.is_none_or(|v| v.is_finite() && v >= 0.0)
}

/// Whether every id of `ids` is non-empty and unique. Sorts a transient
/// list of borrowed ids (rule 3: dropped on return).
fn ids_unique<'a>(ids: impl Iterator<Item = &'a str>) -> bool {
    let mut sorted: Vec<&str> = ids.collect();
    sorted.sort_unstable();
    sorted.iter().all(|id| !id.is_empty()) && sorted.windows(2).all(|w| w[0] != w[1])
}

impl EnergyLevelSeries {
    /// The rules a series is held to on its own; the chart's rules (a
    /// categorical `bottom_x`, the `source` reference) are
    /// [`Config::validate_energy_levels`](crate::config::Config::validate_energy_levels)'s.
    ///
    /// Ids are non-empty and unique per list; energies are finite; an
    /// uncertainty or a statistical weight is finite and not negative;
    /// every transition names two different levels of the series; every
    /// component names a centroid row; text fields are one line of at most
    /// [`MAX_ENERGY_LEVEL_FIELD_CHARS`] characters and labels are valid rich
    /// text; offsets and styles are in range. A series with a `source`
    /// holds no records. No list has a count limit (rule 10).
    pub fn validate(&self) -> Result<(), &'static str> {
        self.style.validate()?;
        if self.source.is_some() {
            if !self.levels.is_empty() || !self.limits.is_empty() || !self.transitions.is_empty() {
                return Err("an energy-level series with a source holds no records of its own");
            }
            return Ok(());
        }
        if !ids_unique(self.levels.iter().map(|l| l.id.as_str())) {
            return Err("energy level ids must be non-empty and unique");
        }
        if !ids_unique(self.limits.iter().map(|l| l.id.as_str())) {
            return Err("ionization limit ids must be non-empty and unique");
        }
        if !ids_unique(self.transitions.iter().map(|t| t.id.as_str())) {
            return Err("transition ids must be non-empty and unique");
        }
        for level in &self.levels {
            level.validate()?;
            if let LevelRole::Component { multiplet } = &level.role {
                let centroid = self.level(multiplet);
                if !centroid.is_some_and(|c| c.role == LevelRole::Centroid) {
                    return Err("a fine-structure component must name a centroid level");
                }
            }
        }
        for limit in &self.limits {
            limit.validate()?;
        }
        for transition in &self.transitions {
            transition.validate()?;
            if transition.lower == transition.upper {
                return Err("a transition joins two different levels");
            }
            if self.level(&transition.lower).is_none() || self.level(&transition.upper).is_none() {
                return Err("a transition must name two levels of its series");
            }
        }
        Ok(())
    }

    /// The level with id `id`. A linear scan over the SSOT: no index is
    /// kept (rule 3).
    pub fn level(&self, id: &str) -> Option<&EnergyLevel> {
        self.levels.iter().find(|level| level.id == id)
    }

    /// The number of rows of `part`: the value label index space of that
    /// list.
    pub fn part_len(&self, part: LabelPart) -> usize {
        match part {
            LabelPart::Level => self.levels.len(),
            LabelPart::Transition => self.transitions.len(),
            LabelPart::Limit => self.limits.len(),
        }
    }

    /// The drag offset of the label of row `index` of `part`, or `None`
    /// past the end.
    pub fn label_offset(&self, part: LabelPart, index: usize) -> Option<[f32; 2]> {
        match part {
            LabelPart::Level => self.levels.get(index).map(|l| l.label_offset),
            LabelPart::Transition => self.transitions.get(index).map(|t| t.label_offset),
            LabelPart::Limit => self.limits.get(index).map(|l| l.label_offset),
        }
    }

    /// Move the label of row `index` of `part` by `(dx, dy)` logical pixels
    /// (x right, y down). Rejected for a row past the end, a non-finite
    /// delta or an offset past [`MAX_VALUE_LABEL_OFFSET`]; nothing changes
    /// then.
    pub fn drag_label(&mut self, part: LabelPart, index: usize, dx: f32, dy: f32) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let offset = match part {
            LabelPart::Level => self.levels.get_mut(index).map(|l| &mut l.label_offset),
            LabelPart::Transition => self.transitions.get_mut(index).map(|t| &mut t.label_offset),
            LabelPart::Limit => self.limits.get_mut(index).map(|l| &mut l.label_offset),
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

    /// The lowest and highest energy the series draws in `view`: its drawn
    /// levels and its limits. `None` when it draws none. A display
    /// parameter for an axis fit (rule 1), read from the SSOT.
    pub fn energy_extent(&self, view: LevelView) -> Option<(f64, f64)> {
        let drawn = self
            .levels
            .iter()
            .filter(|level| level.role.drawn_in(view))
            .map(|level| level.energy);
        let limits = self.limits.iter().map(|limit| limit.energy);
        drawn
            .chain(limits)
            .filter(|e| e.is_finite())
            .fold(None, |acc, e| match acc {
                None => Some((e, e)),
                Some((lo, hi)) => Some((f64::min(lo, e), f64::max(hi, e))),
            })
    }

    /// The continuum annotations of the series' ionization limits, for the
    /// document's annotation layer (the maintainer's choice on #203: the
    /// continuum is drawn by annotations; the limit lines and their labels
    /// are the series' own): per limit, a [`AnnotationKind::Span`] on Y from
    /// the limit up to `top`, filled with `fill`. Its id is
    /// `{series_id}/limit/{limit id}/continuum`, so
    /// [`Self::write_continuum`] replaces it on a later call. The list is
    /// the helper's output (rule 3).
    pub fn continuum_annotations(&self, series_id: &str, top: f64, fill: Color) -> Vec<Annotation> {
        self.limits
            .iter()
            .map(|limit| Annotation {
                id: format!("{series_id}/limit/{}/continuum", limit.id),
                visible: true,
                kind: AnnotationKind::Span {
                    axis: AnnotationAxis::Y,
                    from: limit.energy,
                    to: top,
                    fill,
                },
            })
            .collect()
    }

    /// Write [`Self::continuum_annotations`] into `layer`: an item whose id
    /// it writes is replaced in place, a new one is appended. Rejected,
    /// with `layer` unchanged, when the layer would then fail its own
    /// validation (its item count included).
    pub fn write_continuum(
        &self,
        layer: &mut crate::layer::AnnotationLayer,
        series_id: &str,
        top: f64,
        fill: Color,
    ) -> Result<(), &'static str> {
        let mut next = layer.clone();
        for item in self.continuum_annotations(series_id, top, fill) {
            match next.items.iter_mut().find(|old| old.id == item.id) {
                Some(old) => *old = item,
                None => next.items.push(item),
            }
        }
        next.validate()?;
        *layer = next;
        Ok(())
    }
}

impl EnergyLevel {
    fn validate(&self) -> Result<(), &'static str> {
        if !self.energy.is_finite() {
            return Err("an energy level's energy must be finite");
        }
        if !non_negative(self.uncertainty) || !non_negative(self.g) {
            return Err("an energy level's uncertainty and g must be finite and not negative");
        }
        let texts = [
            &self.column,
            &self.configuration,
            &self.term,
            &self.j,
            &self.prefix,
            &self.suffix,
            &self.reference,
        ];
        if self.column.is_empty() || !texts.iter().all(|t| text_ok(t)) {
            return Err(
                "an energy level's column must be set and its text fields one line of at most 160 characters",
            );
        }
        if let LevelRole::Component { multiplet } = &self.role {
            if !text_ok(multiplet) {
                return Err("a component's multiplet id is one line of at most 160 characters");
            }
        }
        if !rich_ok(&self.label) || !offset_ok(self.label_offset) {
            return Err("invalid energy level label");
        }
        Ok(())
    }
}

impl IonizationLimit {
    fn validate(&self) -> Result<(), &'static str> {
        if !self.energy.is_finite() || !non_negative(self.uncertainty) {
            return Err(
                "an ionization limit's energy must be finite and its uncertainty not negative",
            );
        }
        if !text_ok(&self.reference) || !rich_ok(&self.label) || !offset_ok(self.label_offset) {
            return Err("invalid ionization limit label or reference");
        }
        Ok(())
    }
}

impl Transition {
    fn validate(&self) -> Result<(), &'static str> {
        let optional = [
            self.value,
            self.observed_wavelength,
            self.ritz_wavelength,
            self.aki,
        ];
        if optional.iter().any(|v| v.is_some_and(|v| !v.is_finite()))
            || !non_negative(self.observed_uncertainty)
            || !non_negative(self.ritz_uncertainty)
        {
            return Err("a transition's values must be finite and its uncertainties not negative");
        }
        if !finite_in(self.x_offset, -0.5, 0.5) {
            return Err("a transition's x_offset must be in -0.5..=0.5 of the slot");
        }
        let texts = [
            &self.lower,
            &self.upper,
            &self.intensity,
            &self.accuracy,
            &self.line_type,
            &self.tp_reference,
            &self.line_reference,
        ];
        if !texts.iter().all(|t| text_ok(t)) {
            return Err("a transition's text fields are one line of at most 160 characters");
        }
        if !rich_ok(&self.label) || !offset_ok(self.label_offset) {
            return Err("invalid transition label");
        }
        Ok(())
    }
}

impl EnergyLevelStyle {
    fn validate(&self) -> Result<(), &'static str> {
        let level = &self.level;
        if !color_ok(level.color)
            || !finite_in(level.width, 0.5, 24.0)
            || !finite_in(level.slot_fraction, 0.05, 1.0)
        {
            return Err("invalid energy level stroke style");
        }
        let limit = &self.limit;
        if !color_ok(limit.color) || !finite_in(limit.width, 0.0, 64.0) {
            return Err("invalid ionization limit line style");
        }
        let arrow = &self.transition;
        if !color_ok(arrow.color)
            || !finite_in(arrow.width, 0.5, 24.0)
            || !finite_in(arrow.head_length, 0.0, 64.0)
            || !finite_in(arrow.head_width, 0.0, 64.0)
        {
            return Err("invalid transition arrow style");
        }
        if !finite_in(self.picked_width_scale, 1.0, 4.0) {
            return Err("picked_width_scale must be in 1..=4");
        }
        Ok(())
    }
}

impl SeriesConfig {
    /// The energy-level fields of this series, or `None` for every other
    /// kind.
    pub fn energy_levels(&self) -> Option<&EnergyLevelSeries> {
        match &self.render_type {
            crate::data_config::DataRenderType::EnergyLevels(levels) => Some(levels),
            _ => None,
        }
    }

    /// The energy-level fields of this series, mutably.
    pub fn energy_levels_mut(&mut self) -> Option<&mut EnergyLevelSeries> {
        match &mut self.render_type {
            crate::data_config::DataRenderType::EnergyLevels(levels) => Some(levels),
            _ => None,
        }
    }
}

impl crate::config::Config {
    /// The chart rules of energy-level series (#203): each is valid on its
    /// own ([`EnergyLevelSeries::validate`]), reads no column (`x_column` and
    /// `y_column` empty), names a source other than itself, and is drawn on
    /// a rectangular chart whose `bottom_x` is categorical, not on polar,
    /// ternary, parallel or 3D axes. A source is resolved against the
    /// document's series when the series is drawn, since an inset's source
    /// is a series of another panel.
    pub fn validate_energy_levels(&self, series: &[SeriesConfig]) -> Result<(), &'static str> {
        for item in series {
            let Some(levels) = item.energy_levels() else {
                continue;
            };
            levels.validate()?;
            if !item.x_column.is_empty() || !item.y_column.is_empty() || item.z_column.is_some() {
                return Err(
                    "an energy-level series reads no columns: x_column and y_column stay empty",
                );
            }
            if levels.source.as_deref() == Some(item.series_id.as_str()) {
                return Err("an energy-level series cannot be its own source");
            }
            if self.polar().is_some()
                || self.ternary().is_some()
                || self.parallel().is_some()
                || self.view_3d().is_some()
            {
                return Err("an energy-level series is drawn on a rectangular chart");
            }
            if self.bottom_x.categories.is_none() {
                return Err(
                    "an energy-level series needs a categorical bottom_x: its term columns",
                );
            }
        }
        Ok(())
    }
}

/// The picked row of an energy-level series the emit widens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickedRow {
    Level(usize),
    Transition(usize),
}

/// Where an energy-level series is placed: the panel's anchor space (a
/// categorical `bottom_x`, the energy on `left_y`), its categories, the view
/// the series draws, the colour bar its mapped transitions read and the row
/// drawn picked. Everything here is a display parameter (rule 1).
pub struct LevelPlacement<'a> {
    pub space: &'a dyn crate::annotation::AnnotationSpace,
    pub categories: &'a [crate::config::Category],
    pub view: LevelView,
    pub colorbar: Option<&'a crate::config::ColorBarOptions>,
    pub picked: Option<PickedRow>,
}

/// The gap between a stroke's end and its label, logical pixels.
const LABEL_GAP: f32 = 4.0;

impl LevelPlacement<'_> {
    fn column_x(&self, column: &str) -> Option<f64> {
        self.categories
            .iter()
            .position(|c| c.id == column)
            .map(|i| i as f64)
    }

    fn px(&self, axis: AnnotationAxis, value: f64) -> Option<f32> {
        self.space.value_to_px(axis, value)
    }

    /// The stroke of level row `index` of `records`, left end then right
    /// end, or `None` when the view does not draw it, its column is not on
    /// the axis or it falls outside the axes' ranges.
    pub fn level_stroke(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        index: usize,
    ) -> Option<([f32; 2], [f32; 2])> {
        let level = records.levels.get(index)?;
        if !level.role.drawn_in(self.view) {
            return None;
        }
        let x = self.column_x(&level.column)?;
        let half = f64::from(style.level.slot_fraction) / 2.0;
        let y = self.px(AnnotationAxis::Y, level.energy)?;
        let x0 = self.px(AnnotationAxis::X, x - half)?;
        let x1 = self.px(AnnotationAxis::X, x + half)?;
        Some(([x0, y], [x1, y]))
    }

    /// The arrow of transition row `index`: its tail and its tip (the head
    /// end under `style.transition.direction`; for `None`, lower then
    /// upper), or `None` when either level is not drawn.
    pub fn transition_arrow(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        index: usize,
    ) -> Option<([f32; 2], [f32; 2])> {
        let transition = records.transitions.get(index)?;
        let end = |id: &str| -> Option<[f32; 2]> {
            let row = records.levels.iter().position(|l| l.id == id)?;
            let (a, b) = self.level_stroke(records, style, row)?;
            let level = &records.levels[row];
            let x = self.column_x(&level.column)? + f64::from(transition.x_offset);
            let px = self.px(AnnotationAxis::X, x).unwrap_or((a[0] + b[0]) / 2.0);
            Some([px, a[1]])
        };
        let lower = end(&transition.lower)?;
        let upper = end(&transition.upper)?;
        Some(match style.transition.direction {
            TransitionDirection::Emission => (upper, lower),
            TransitionDirection::Absorption | TransitionDirection::None => (lower, upper),
        })
    }

    /// The line of limit row `index` across the plot, or `None` outside
    /// the energy range.
    pub fn limit_line(
        &self,
        records: &EnergyLevelSeries,
        index: usize,
    ) -> Option<([f32; 2], [f32; 2])> {
        let limit = records.limits.get(index)?;
        let y = self.px(AnnotationAxis::Y, limit.energy)?;
        let plot = self.space.plot_rect();
        Some(([plot.x, y], [plot.x + plot.width, y]))
    }

    fn transition_color(&self, style: &EnergyLevelStyle, transition: &Transition) -> Color {
        let arrow = &style.transition;
        if !arrow.color_by_value {
            return arrow.color;
        }
        match (self.colorbar, transition.value) {
            (Some(bar), Some(value)) => match bar.normalized_z(value) {
                Some(t) => bar.colormap.sample(t),
                None => bar.nan_color,
            },
            _ => arrow.color,
        }
    }

    /// The text block of a label at `anchor` (a stroke end or a midpoint),
    /// placed on `side` and moved by `offset`.
    fn label_block<'t>(
        text: &'t RichText,
        measure: &dyn crate::text::MeasureText,
        anchor: [f32; 2],
        left: f32,
        side: LevelLabelSide,
        offset: [f32; 2],
    ) -> crate::sink::PlacedText<'t> {
        let extents = measure.measure_rich(text);
        let centred = anchor[1] + (extents.ascent - extents.descent) / 2.0;
        let origin = match side {
            LevelLabelSide::Right => (anchor[0] + LABEL_GAP, centred),
            LevelLabelSide::Left => (left - LABEL_GAP - extents.width, centred),
            LevelLabelSide::Above => (
                (left + anchor[0]) / 2.0 - extents.width / 2.0,
                anchor[1] - LABEL_GAP - extents.descent,
            ),
        };
        crate::sink::PlacedText {
            text: crate::sink::TextRef::Rich(text),
            origin: (origin.0 + offset[0], origin.1 + offset[1]),
            extents,
            rotation: None,
        }
    }

    /// The placed label of row `index` of `part`, or `None` when the row
    /// has no label or is not drawn.
    pub fn label<'t>(
        &self,
        records: &'t EnergyLevelSeries,
        style: &EnergyLevelStyle,
        part: LabelPart,
        index: usize,
        measure: &dyn crate::text::MeasureText,
    ) -> Option<crate::sink::PlacedText<'t>> {
        match part {
            LabelPart::Level => {
                let level = records.levels.get(index)?;
                let (a, b) = self.level_stroke(records, style, index)?;
                Some(Self::label_block(
                    level.label.as_ref()?,
                    measure,
                    b,
                    a[0],
                    style.label_side,
                    level.label_offset,
                ))
            }
            LabelPart::Transition => {
                let transition = records.transitions.get(index)?;
                let (tail, tip) = self.transition_arrow(records, style, index)?;
                let mid = [(tail[0] + tip[0]) / 2.0, (tail[1] + tip[1]) / 2.0];
                Some(Self::label_block(
                    transition.label.as_ref()?,
                    measure,
                    mid,
                    mid[0],
                    LevelLabelSide::Right,
                    transition.label_offset,
                ))
            }
            LabelPart::Limit => {
                let limit = records.limits.get(index)?;
                let (_, end) = self.limit_line(records, index)?;
                let text = limit.label.as_ref()?;
                let extents = measure.measure_rich(text);
                let origin = (
                    end[0] - LABEL_GAP - extents.width + limit.label_offset[0],
                    end[1] - LABEL_GAP - extents.descent + limit.label_offset[1],
                );
                Some(crate::sink::PlacedText {
                    text: crate::sink::TextRef::Rich(text),
                    origin,
                    extents,
                    rotation: None,
                })
            }
        }
    }

    /// Emit every drawn part of `records` in the look of `style` into
    /// `sink`, in the [`crate::sink::Slot::Above`] slot: the limit lines,
    /// the level strokes, the transition arrows over them, then every
    /// label. The picked row is drawn `style.picked_width_scale` times as
    /// wide. Nothing is stored; the caller's sink paints, bounds or hit
    /// tests the parts.
    pub fn emit(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
        sink: &mut dyn crate::sink::Sink,
    ) {
        use crate::sink::{Slot, StrokeStyle};
        let slot = Slot::Above;
        for index in 0..records.limits.len() {
            if let Some((a, b)) = self.limit_line(records, index) {
                let line = &style.limit;
                sink.stroke(
                    slot,
                    a,
                    b,
                    &StrokeStyle {
                        width: line.width,
                        color: line.color,
                        dash: line.style.pattern(),
                        tag: "energy_limit",
                    },
                );
            }
        }
        let widen = |picked: bool, width: f32| {
            if picked {
                width * style.picked_width_scale
            } else {
                width
            }
        };
        for index in 0..records.levels.len() {
            if let Some((a, b)) = self.level_stroke(records, style, index) {
                let picked = self.picked == Some(PickedRow::Level(index));
                sink.stroke(
                    slot,
                    a,
                    b,
                    &StrokeStyle {
                        width: widen(picked, style.level.width),
                        color: style.level.color,
                        dash: &[],
                        tag: "energy_level",
                    },
                );
            }
        }
        let arrow = &style.transition;
        for (index, transition) in records.transitions.iter().enumerate() {
            let Some((tail, tip)) = self.transition_arrow(records, style, index) else {
                continue;
            };
            let picked = self.picked == Some(PickedRow::Transition(index));
            let color = self.transition_color(style, transition);
            let width = widen(picked, arrow.width);
            let (dx, dy) = (tip[0] - tail[0], tip[1] - tail[1]);
            let length = dx.hypot(dy);
            let head = if arrow.direction == TransitionDirection::None {
                0.0
            } else {
                arrow.head_length.min(length)
            };
            let (ux, uy) = if length > 0.0 {
                (dx / length, dy / length)
            } else {
                (0.0, 0.0)
            };
            let base = [tip[0] - ux * head, tip[1] - uy * head];
            sink.stroke(
                slot,
                tail,
                if head > 0.0 { base } else { tip },
                &StrokeStyle {
                    width,
                    color,
                    dash: &[],
                    tag: "energy_transition",
                },
            );
            if head > 0.0 {
                let half = widen(picked, arrow.head_width) / 2.0;
                let (nx, ny) = (-uy * half, ux * half);
                sink.triangle(
                    slot,
                    [
                        tip,
                        [base[0] + nx, base[1] + ny],
                        [base[0] - nx, base[1] - ny],
                    ],
                    color,
                );
            }
        }
        for part in [LabelPart::Level, LabelPart::Transition, LabelPart::Limit] {
            for index in 0..records.part_len(part) {
                if let Some(text) = self.label(records, style, part, index, measure) {
                    sink.text(slot, &text);
                }
            }
        }
    }
}

/// What a pick of an energy-level series hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelHit {
    Level(usize),
    Transition(usize),
    Label(LabelPart, usize),
}

/// The distance from `p` to the segment `a`–`b`.
fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
}

/// The distance from `p` to `rect`, zero inside it.
fn rect_distance(p: [f32; 2], rect: crate::layout::RectF) -> f32 {
    let dx = (rect.x - p[0]).max(p[0] - (rect.x + rect.width)).max(0.0);
    let dy = (rect.y - p[1]).max(p[1] - (rect.y + rect.height)).max(0.0);
    dx.hypot(dy)
}

impl LevelPlacement<'_> {
    /// The drawn part of `records` nearest `cursor`, within `max_distance`
    /// of its edge, and that distance (zero on the part). Walks the SSOT
    /// through the same placement the emit draws with, so the pick is the
    /// drawn geometry; nothing is registered per row (rule 3). On a tie a
    /// part drawn later wins: labels over arrows over strokes.
    pub fn pick(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
        cursor: [f32; 2],
        max_distance: f32,
    ) -> Option<(LevelHit, f32)> {
        let mut best: Option<(LevelHit, f32)> = None;
        let mut offer = |hit: LevelHit, distance: f32| {
            if distance <= max_distance && best.is_none_or(|(_, d)| distance <= d) {
                best = Some((hit, distance));
            }
        };
        for index in 0..records.levels.len() {
            if let Some((a, b)) = self.level_stroke(records, style, index) {
                let edge = segment_distance(cursor, a, b) - style.level.width / 2.0;
                offer(LevelHit::Level(index), edge.max(0.0));
            }
        }
        for index in 0..records.transitions.len() {
            if let Some((tail, tip)) = self.transition_arrow(records, style, index) {
                let half = style.transition.width.max(style.transition.head_width) / 2.0;
                let edge = segment_distance(cursor, tail, tip) - half;
                offer(LevelHit::Transition(index), edge.max(0.0));
            }
        }
        for part in [LabelPart::Level, LabelPart::Transition, LabelPart::Limit] {
            for index in 0..records.part_len(part) {
                if let Some(text) = self.label(records, style, part, index, measure) {
                    offer(
                        LevelHit::Label(part, index),
                        rect_distance(cursor, text.rect()),
                    );
                }
            }
        }
        best
    }
}

impl crate::config::Config {
    /// The subplot nested in this chart's data area whose chart area holds
    /// the point `(x, y)` (in this config's pixels), the last such one when
    /// several do: an inset (#203). A pointer there belongs to that
    /// subplot, whatever the active layer. `None` when no inset holds it.
    pub fn nested_subplot_at(&self, x: f32, y: f32) -> Option<usize> {
        let plot = self.data_area().ok()?.0;
        let inside = |r: &crate::layout::Rect, x: f32, y: f32| {
            x >= r.x as f32
                && y >= r.y as f32
                && x < (r.x + r.width) as f32
                && y < (r.y + r.height) as f32
        };
        self.subplots
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, subplot)| {
                let area = subplot.config.chart_area.0;
                let nested = area.x >= plot.x
                    && area.y >= plot.y
                    && area.x + area.width <= plot.x + plot.width
                    && area.y + area.height <= plot.y + plot.height;
                (nested && inside(&area, x, y)).then_some(index)
            })
    }
}

impl crate::config::Config {
    /// The annotations that mark what inset `subplot` magnifies (#203): a
    /// [`AnnotationKind::Rect`] around the window `x` × `y` in this chart's
    /// data coordinates, and two headless [`AnnotationKind::Arrow`]s from
    /// the window's corners to the inset's plot (its data area), as ratio
    /// anchors of
    /// this chart's plot (so they stay on the inset as it is drawn now; call
    /// again after the inset moves). Ids are `inset/{subplot}/window`,
    /// `…/link0` and `…/link1`. `None` for a subplot that is not an inset
    /// of this chart. The list is the helper's output (rule 3).
    pub fn inset_marker(
        &self,
        subplot: usize,
        x: (f64, f64),
        y: (f64, f64),
        line: AnnotationLineStyle,
    ) -> Option<Vec<Annotation>> {
        use crate::annotation::{Anchor, ArrowStyle, ShapeStyle};
        let plot = self.data_area().ok()?.0;
        let child = &self.subplots.get(subplot)?.config;
        let area = child.chart_area.0;
        let nested = area.x >= plot.x
            && area.y >= plot.y
            && area.x + area.width <= plot.x + plot.width
            && area.y + area.height <= plot.y + plot.height;
        let inset = child.data_area().ok()?.0;
        let nested = nested
            && inset.x >= plot.x
            && inset.y >= plot.y
            && inset.x + inset.width <= plot.x + plot.width
            && inset.y + inset.height <= plot.y + plot.height;
        if !nested || plot.width == 0 || plot.height == 0 {
            return None;
        }
        let ratio = |px: u32, py: u32| Anchor::Ratio {
            x: (px - plot.x) as f32 / plot.width as f32,
            y: (py - plot.y) as f32 / plot.height as f32,
        };
        let id = |part: &str| format!("inset/{subplot}/{part}");
        let link = |n: usize, from: Anchor, to: Anchor| Annotation {
            id: id(&format!("link{n}")),
            visible: true,
            kind: AnnotationKind::Arrow {
                from,
                to,
                style: ArrowStyle {
                    line: line.clone(),
                    head_length: 0.0,
                    head_width: 0.0,
                },
                text: None,
            },
        };
        Some(vec![
            Annotation {
                id: id("window"),
                visible: true,
                kind: AnnotationKind::Rect {
                    from: Anchor::Data { x: x.0, y: y.0 },
                    to: Anchor::Data { x: x.1, y: y.1 },
                    style: ShapeStyle {
                        fill: None,
                        border: line.clone(),
                        below: false,
                    },
                },
            },
            link(0, Anchor::Data { x: x.1, y: y.1 }, ratio(inset.x, inset.y)),
            link(
                1,
                Anchor::Data { x: x.1, y: y.0 },
                ratio(inset.x, inset.y + inset.height),
            ),
        ])
    }
}

/// The records an energy-level series draws: its own, or those of its
/// `source` in `series` (the document's series list). `None` when the
/// source does not name an energy-level series of the list.
pub fn drawn_records<'a>(
    item: &'a EnergyLevelSeries,
    series: &'a [SeriesConfig],
) -> Option<&'a EnergyLevelSeries> {
    match &item.source {
        None => Some(item),
        Some(id) => series
            .iter()
            .find(|s| s.series_id == *id)
            .and_then(SeriesConfig::energy_levels)
            .filter(|source| source.source.is_none()),
    }
}

impl EnergyLevelSeries {
    /// Whether level row `index` is drawn in `view`.
    pub fn level_drawn(&self, index: usize, view: LevelView) -> bool {
        self.levels
            .get(index)
            .is_some_and(|level| level.role.drawn_in(view))
    }

    /// Whether transition row `index` is drawn in `view`: both its levels
    /// are.
    pub fn transition_drawn(&self, index: usize, view: LevelView) -> bool {
        self.transitions.get(index).is_some_and(|t| {
            let drawn = |id: &str| self.level(id).is_some_and(|l| l.role.drawn_in(view));
            drawn(&t.lower) && drawn(&t.upper)
        })
    }
}

/// The drawn row nearest `index` (at or after it first), among `len` rows.
fn nearest_drawn(len: usize, drawn: &dyn Fn(usize) -> bool, index: usize) -> Option<usize> {
    let start = index.min(len.checked_sub(1)?);
    (start..len)
        .find(|&i| drawn(i))
        .or_else(|| (0..start).rev().find(|&i| drawn(i)))
}

/// `delta` drawn rows on from the drawn row nearest `index`, stopping at
/// the first or last drawn row; nothing wraps.
fn step_drawn(
    len: usize,
    drawn: &dyn Fn(usize) -> bool,
    index: usize,
    delta: i32,
) -> Option<usize> {
    let mut at = nearest_drawn(len, drawn, index)?;
    for _ in 0..delta.unsigned_abs() {
        let next = if delta > 0 {
            (at + 1..len).find(|&i| drawn(i))
        } else {
            (0..at).rev().find(|&i| drawn(i))
        };
        match next {
            Some(n) => at = n,
            None => break,
        }
    }
    Some(at)
}

/// The selection `step_selection` moves to from `reference` on the
/// energy-level series `series[position]` (#203): a level (`HistogramBin`)
/// steps by `dx` over the level rows its view draws and a transition
/// (`Point`) over the transitions it draws; `dy` moves to another
/// energy-level series of the list, in series order, to its drawn row
/// nearest the same index. Rows a view does not draw are skipped; a value
/// label does not step. `None` when nothing is drawn to move to.
pub(crate) fn stepped_selection(
    series: &[SeriesConfig],
    position: usize,
    reference: &crate::config::PickedDataRef,
    dx: i32,
    dy: i32,
) -> Option<crate::config::PickedDataRef> {
    use crate::config::PickedDataRef;
    let transitions = match reference {
        PickedDataRef::HistogramBin { .. } => false,
        PickedDataRef::Point { .. } => true,
        _ => return None,
    };
    let index = match reference {
        PickedDataRef::HistogramBin { bin_index, .. } => *bin_index,
        PickedDataRef::Point { point_index, .. } => *point_index,
        _ => return None,
    };
    let members = || series.iter().filter(|s| s.energy_levels().is_some());
    let at = members().position(|s| s.series_id == series[position].series_id)?;
    let target = at
        .saturating_add_signed(dy as isize)
        .min(members().count().saturating_sub(1));
    let next = members().nth(target)?;
    let own = next.energy_levels()?;
    let records = drawn_records(own, series)?;
    let view = own.show;
    let len = if transitions {
        records.transitions.len()
    } else {
        records.levels.len()
    };
    let drawn = |i| {
        if transitions {
            records.transition_drawn(i, view)
        } else {
            records.level_drawn(i, view)
        }
    };
    let row = step_drawn(len, &drawn, index, dx)?;
    Some(if transitions {
        PickedDataRef::Point {
            source_id: next.source_id.clone(),
            series_id: next.series_id.clone(),
            point_index: row,
        }
    } else {
        PickedDataRef::HistogramBin {
            source_id: next.source_id.clone(),
            series_id: next.series_id.clone(),
            bin_index: row,
            half: None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Category, Config, DataSelectionsConfig, PickedDataRef};
    use crate::data_config::DataRenderType;

    fn level(id: &str, column: &str, energy: f64) -> EnergyLevel {
        EnergyLevel {
            id: id.into(),
            column: column.into(),
            configuration: String::new(),
            term: String::new(),
            j: String::new(),
            g: None,
            prefix: String::new(),
            energy,
            suffix: String::new(),
            uncertainty: None,
            reference: String::new(),
            role: LevelRole::Level,
            label: None,
            label_offset: [0.0; 2],
        }
    }

    fn transition(id: &str, lower: &str, upper: &str) -> Transition {
        Transition {
            id: id.into(),
            lower: lower.into(),
            upper: upper.into(),
            label: None,
            label_offset: [0.0; 2],
            value: None,
            x_offset: 0.0,
            observed_wavelength: None,
            observed_uncertainty: None,
            ritz_wavelength: None,
            ritz_uncertainty: None,
            intensity: String::new(),
            aki: None,
            accuracy: String::new(),
            line_type: String::new(),
            tp_reference: String::new(),
            line_reference: String::new(),
        }
    }

    /// He I as the NIST exports list it: the ground state, 2s ³S, the
    /// resolved 2p ³P° multiplet with its centroid, 2p ¹P°, the limit and
    /// two lines.
    fn helium() -> EnergyLevelSeries {
        let component = |id: &str, j: &str, g: f64, energy: f64| EnergyLevel {
            j: j.into(),
            g: Some(g),
            role: LevelRole::Component {
                multiplet: "2p3P".into(),
            },
            ..level(id, "3P*", energy)
        };
        EnergyLevelSeries {
            levels: vec![
                EnergyLevel {
                    configuration: "1s2".into(),
                    term: "1S".into(),
                    j: "0".into(),
                    g: Some(1.0),
                    prefix: "[".into(),
                    suffix: "]".into(),
                    uncertainty: Some(0.000_000_15),
                    reference: "L22715".into(),
                    label: Some(
                        RichText::from_tex("$1s^2$", Color::from_rgb8(0, 0, 0), 12.0, "").unwrap(),
                    ),
                    ..level("g", "1S", 0.0)
                },
                level("2s3S", "3S", 19.819_614_525),
                EnergyLevel {
                    role: LevelRole::Centroid,
                    ..level("2p3P", "3P*", 20.964_1)
                },
                component("2p3P2", "2", 5.0, 20.964_086_889_08),
                component("2p3P1", "1", 3.0, 20.964_096_364_64),
                component("2p3P0", "0", 1.0, 20.964_218_850_51),
                level("2p1P", "1P*", 21.218_022_711_2),
            ],
            limits: vec![IonizationLimit {
                id: "He II".into(),
                energy: 24.587_389_011,
                label: None,
                label_offset: [0.0; 2],
                uncertainty: Some(0.000_000_025),
                reference: "L17714".into(),
            }],
            transitions: vec![
                Transition {
                    observed_wavelength: Some(1083.0),
                    aki: Some(1.0216e7),
                    accuracy: "AAA".into(),
                    intensity: "1*".into(),
                    ..transition("1083", "2s3S", "2p3P")
                },
                transition("j2", "2s3S", "2p3P2"),
                transition("j1", "2s3S", "2p3P1"),
            ],
            ..EnergyLevelSeries::default()
        }
    }

    fn series(id: &str, levels: EnergyLevelSeries) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: String::new(),
            y_column: String::new(),
            z_column: None,
            render_type: DataRenderType::EnergyLevels(levels),
        }
    }

    fn chart() -> Config {
        let mut config = crate::default::default_config();
        config.bottom_x.categories = Some(
            ["1S", "1P*", "3S", "3P*"]
                .into_iter()
                .map(|c| Category::new(c, c))
                .collect(),
        );
        config
    }

    #[test]
    fn a_helium_series_validates_on_a_categorical_chart() {
        let config = chart();
        assert_eq!(config.validate_series(&[series("he", helium())]), Ok(()));
        let mut numeric = config.clone();
        numeric.bottom_x.categories = None;
        let err = numeric
            .validate_series(&[series("he", helium())])
            .unwrap_err();
        assert_eq!(err.field, "energy_levels");
        assert_eq!(err.series_id.as_deref(), Some("he"));
    }

    #[test]
    fn broken_records_are_refused() {
        let check = |edit: &dyn Fn(&mut EnergyLevelSeries)| {
            let mut levels = helium();
            edit(&mut levels);
            levels.validate()
        };
        assert!(
            check(&|s| s.levels[1].id = "g".into()).is_err(),
            "duplicate id"
        );
        assert!(check(&|s| s.levels[1].id.clear()).is_err(), "empty id");
        assert!(check(&|s| s.levels[1].energy = f64::NAN).is_err());
        assert!(check(&|s| s.levels[1].g = Some(-1.0)).is_err());
        assert!(check(&|s| s.levels[1].column.clear()).is_err());
        assert!(check(&|s| s.levels[1].term = "a\nb".into()).is_err());
        assert!(check(&|s| s.transitions[0].upper = "nowhere".into()).is_err());
        assert!(check(&|s| s.transitions[0].upper = "2s3S".into()).is_err());
        assert!(check(&|s| s.transitions[0].x_offset = 0.6).is_err());
        assert!(check(&|s| s.limits[0].energy = f64::INFINITY).is_err());
        // A component must name a centroid row, not a plain level.
        assert!(
            check(&|s| s.levels[3].role = LevelRole::Component {
                multiplet: "2s3S".into()
            })
            .is_err()
        );
        assert!(check(&|s| s.style.level.width = 0.0).is_err());
        assert!(check(&|s| s.style.picked_width_scale = 5.0).is_err());
        // A view of another series holds nothing of its own.
        assert!(check(&|s| s.source = Some("main".into())).is_err());
    }

    #[test]
    fn the_chart_rules_refuse_columns_and_self_sources() {
        let config = chart();
        let mut with_column = series("he", helium());
        with_column.x_column = "x".into();
        assert!(config.validate_series(&[with_column]).is_err());
        let view = series(
            "inset",
            EnergyLevelSeries {
                source: Some("inset".into()),
                show: LevelView::Components,
                ..EnergyLevelSeries::default()
            },
        );
        assert!(config.validate_series(&[view]).is_err());
    }

    #[test]
    fn no_list_has_a_count_limit() {
        // Past every bound the code has had: the 1024 annotation items and
        // the 64 radial slices (rule 10).
        let mut levels = EnergyLevelSeries::default();
        for i in 0..5_000 {
            levels.levels.push(level(&format!("l{i}"), "1S", i as f64));
        }
        for i in 0..2_000 {
            levels.transitions.push(transition(
                &format!("t{i}"),
                &format!("l{i}"),
                &format!("l{}", i + 1),
            ));
        }
        assert_eq!(chart().validate_series(&[series("big", levels)]), Ok(()));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn the_series_round_trips_and_omits_its_defaults() {
        let item = series("he", helium());
        let json = serde_json::to_value(&item).unwrap();
        let body = &json["render_type"]["EnergyLevels"];
        assert!(body.get("style").is_none(), "a default style is omitted");
        assert!(body.get("show").is_none() && body.get("source").is_none());
        assert!(body["levels"][1].get("configuration").is_none());
        assert!(body["levels"][1].get("label_offset").is_none());
        assert_eq!(body["levels"][3]["role"]["Component"]["multiplet"], "2p3P");
        assert_eq!(body["transitions"][0]["intensity"], "1*");
        let back: SeriesConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, item);
        // A picked label names its list; other labels omit the field.
        let picked = PickedDataRef::ValueLabel {
            source_id: None,
            series_id: "he".into(),
            category_index: 2,
            part: Some(LabelPart::Transition),
        };
        let json = serde_json::to_value(&picked).unwrap();
        assert_eq!(json["part"], "transition");
        assert_eq!(
            serde_json::from_value::<PickedDataRef>(json).unwrap(),
            picked
        );
        let plain = PickedDataRef::ValueLabel {
            source_id: None,
            series_id: "bars".into(),
            category_index: 0,
            part: None,
        };
        assert!(serde_json::to_value(&plain).unwrap().get("part").is_none());
    }

    fn selected(config: &mut Config, reference: PickedDataRef) {
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![reference],
            ..DataSelectionsConfig::default()
        });
    }

    fn first(config: &Config) -> PickedDataRef {
        config.picked_data.as_ref().unwrap().refs[0].clone()
    }

    #[test]
    fn step_selection_walks_the_drawn_levels_and_transitions() {
        let list = [
            series("he", helium()),
            series(
                "inset",
                EnergyLevelSeries {
                    source: Some("he".into()),
                    show: LevelView::Components,
                    ..EnergyLevelSeries::default()
                },
            ),
        ];
        let no_columns = |_: &str| None;
        let mut config = chart();
        let bin = |series: &str, i| PickedDataRef::HistogramBin {
            source_id: None,
            series_id: series.into(),
            bin_index: i,
            half: None,
        };
        // The main view skips the components (rows 3..=5).
        selected(&mut config, bin("he", 2));
        assert_eq!(config.step_selection(&list, 1, 0, no_columns), Ok(true));
        assert_eq!(first(&config), bin("he", 6));
        assert_eq!(config.step_selection(&list, 1, 0, no_columns), Ok(false));
        assert_eq!(config.step_selection(&list, -5, 0, no_columns), Ok(true));
        assert_eq!(first(&config), bin("he", 0));
        // `dy` moves to the inset, which draws only the components.
        selected(&mut config, bin("he", 2));
        assert_eq!(config.step_selection(&list, 0, 1, no_columns), Ok(true));
        assert_eq!(first(&config), bin("inset", 3), "the centroid is skipped");
        assert_eq!(config.step_selection(&list, -9, 0, no_columns), Ok(true));
        assert_eq!(first(&config), bin("inset", 0));
        // Transitions: the main view draws only the one to the centroid;
        // the inset the two to components.
        let point = |series: &str, i| PickedDataRef::Point {
            source_id: None,
            series_id: series.into(),
            point_index: i,
        };
        selected(&mut config, point("he", 0));
        assert_eq!(config.step_selection(&list, 1, 0, no_columns), Ok(false));
        assert_eq!(config.step_selection(&list, 0, 1, no_columns), Ok(true));
        assert_eq!(first(&config), point("inset", 1));
        assert_eq!(config.step_selection(&list, 1, 0, no_columns), Ok(true));
        assert_eq!(first(&config), point("inset", 2));
        // A value label does not step.
        let label = PickedDataRef::ValueLabel {
            source_id: None,
            series_id: "he".into(),
            category_index: 0,
            part: Some(LabelPart::Level),
        };
        selected(&mut config, label.clone());
        assert_eq!(config.step_selection(&list, 1, 0, no_columns), Ok(false));
        assert_eq!(first(&config), label);
    }

    #[test]
    fn labels_drag_per_part_and_reject_bad_moves() {
        let mut levels = helium();
        assert_eq!(
            levels.drag_label(LabelPart::Level, 0, 4.0, -2.0),
            NudgeResult::Moved
        );
        assert_eq!(levels.label_offset(LabelPart::Level, 0), Some([4.0, -2.0]));
        assert_eq!(
            levels.drag_label(LabelPart::Transition, 2, 1.0, 1.0),
            NudgeResult::Moved
        );
        assert_eq!(
            levels.drag_label(LabelPart::Limit, 0, 0.0, 3.0),
            NudgeResult::Moved
        );
        assert_eq!(levels.label_offset(LabelPart::Limit, 0), Some([0.0, 3.0]));
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        assert_eq!(levels.drag_label(LabelPart::Limit, 1, 1.0, 0.0), reject);
        assert_eq!(
            levels.drag_label(LabelPart::Level, 0, f32::NAN, 0.0),
            reject
        );
        assert_eq!(levels.drag_label(LabelPart::Level, 0, 5000.0, 0.0), reject);
        assert_eq!(levels.label_offset(LabelPart::Level, 0), Some([4.0, -2.0]));
        assert_eq!(levels.part_len(LabelPart::Transition), 3);
    }

    #[test]
    fn the_energy_extent_follows_the_view() {
        let levels = helium();
        assert_eq!(
            levels.energy_extent(LevelView::Main),
            Some((0.0, 24.587_389_011))
        );
        let mut components = levels.clone();
        components.levels.retain(|l| l.role != LevelRole::Level);
        let (lo, hi) = components.energy_extent(LevelView::Components).unwrap();
        assert_eq!(lo, 20.964_086_889_08, "the centroid is not drawn there");
        assert_eq!(hi, 24.587_389_011, "the limits are drawn in every view");
        assert_eq!(
            EnergyLevelSeries::default().energy_extent(LevelView::Main),
            None
        );
    }

    #[test]
    fn the_continuum_is_written_and_rewritten_in_place() {
        let levels = helium();
        let mut config = chart();
        let fill = Color::from_rgba8(80, 80, 220, 40);
        levels
            .write_continuum(&mut config.annotations, "he", 26.0, fill)
            .unwrap();
        assert_eq!(config.annotations.items.len(), 1);
        assert_eq!(config.annotations.items[0].id, "he/limit/He II/continuum");
        assert!(matches!(
            config.annotations.items[0].kind,
            AnnotationKind::Span { from, to, .. } if from == 24.587_389_011 && to == 26.0
        ));
        // A second call replaces the same item.
        levels
            .write_continuum(&mut config.annotations, "he", 30.0, fill)
            .unwrap();
        assert_eq!(config.annotations.items.len(), 1);
        assert!(matches!(
            config.annotations.items[0].kind,
            AnnotationKind::Span { to, .. } if to == 30.0
        ));
        // A full layer refuses a new item and stays as it was.
        let mut full = config.annotations.clone();
        let filler = full.items[0].clone();
        while full.items.len() < crate::annotation::MAX_ANNOTATIONS {
            let mut item = filler.clone();
            item.id = format!("f{}", full.items.len());
            full.items.push(item);
        }
        let before = full.clone();
        let mut other = levels.clone();
        other.limits[0].id = "other".into();
        assert!(other.write_continuum(&mut full, "he", 30.0, fill).is_err());
        assert_eq!(full, before);
    }
}

#[cfg(test)]
mod category_group_tests {
    use crate::config::{Category, CategoryGroup, CategoryGroups, Config};
    use crate::layout::{Element, NudgeResult};
    use crate::select::HitMap;
    use crate::sink::{DrawnPart, Parts};
    use crate::text::{MeasureText, RichText, TextExtents};

    /// The text blocks `emit` places.
    fn texts(emit: impl FnOnce(&mut Parts<'_>) -> bool) -> Vec<crate::layout::RectF> {
        let mut out = Vec::new();
        let mut each = |part: DrawnPart| {
            if let DrawnPart::Text(rect) = part {
                out.push(rect);
            }
        };
        emit(&mut Parts(&mut each));
        out
    }

    struct Fixed;
    impl MeasureText for Fixed {
        fn measure_rich(&self, rt: &RichText) -> TextExtents {
            TextExtents {
                width: rt.segments.len() as f32 * 8.0,
                ascent: 10.0,
                descent: 3.0,
            }
        }
    }

    fn group(label: &str, first: &str, last: &str) -> CategoryGroup {
        CategoryGroup {
            label: RichText::plain(label, crate::color::Color::BLACK, 12.0, ""),
            first: first.into(),
            last: last.into(),
            bracket: true,
        }
    }

    fn chart() -> Config {
        let mut config = crate::default::default_config();
        config.bottom_x.categories = Some(
            ["a", "b", "c", "d"]
                .into_iter()
                .map(|c| Category::new(c, c))
                .collect(),
        );
        config.bottom_x.out_margin = 90.0;
        config.bottom_x.category_groups = Some(CategoryGroups {
            groups: vec![group("Left", "a", "b"), group("Right", "c", "d")],
            offset: [0.0; 2],
        });
        config
    }

    #[test]
    fn groups_validate_against_their_axis() {
        assert_eq!(chart().validate_chart(), Ok(()));
        let check = |edit: &dyn Fn(&mut Config)| {
            let mut config = chart();
            edit(&mut config);
            config.validate_chart().is_err()
        };
        assert!(check(&|c| c
            .bottom_x
            .category_groups
            .as_mut()
            .unwrap()
            .groups[0]
            .last = "z".into()));
        assert!(check(&|c| {
            let g = &mut c.bottom_x.category_groups.as_mut().unwrap().groups[0];
            (g.first, g.last) = ("b".into(), "a".into());
        }));
        assert!(
            check(&|c| {
                c.bottom_x.category_groups.as_mut().unwrap().groups[1].first = "b".into()
            }),
            "overlap"
        );
        assert!(check(&|c| c
            .bottom_x
            .category_groups
            .as_mut()
            .unwrap()
            .offset[0] = f32::NAN));
        // Only on a categorical x axis.
        assert!(check(&|c| c.bottom_x.categories = None));
        assert!(check(&|c| {
            c.left_y.categories = c.bottom_x.categories.clone();
            c.left_y.category_groups = c.bottom_x.category_groups.clone();
        }));
        // No count limit on the groups (rule 10).
        let mut many = chart();
        let ids: Vec<String> = (0..300).map(|i| format!("k{i}")).collect();
        many.bottom_x.categories = Some(ids.iter().map(|i| Category::new(i, i)).collect());
        many.bottom_x.category_groups.as_mut().unwrap().groups =
            ids.iter().map(|i| group("g", i, i)).collect();
        assert_eq!(many.validate_chart(), Ok(()));
    }

    #[test]
    fn a_rich_category_label_is_validated_and_drawn_in_place_of_the_plain_one() {
        let mut config = chart();
        config.bottom_x.categories.as_mut().unwrap()[0].rich_label =
            Some(RichText::from_tex("$^3P^o$", crate::color::Color::BLACK, 12.0, "").unwrap());
        assert_eq!(config.validate_chart(), Ok(()));
        let mut texts = Vec::new();
        let plot = crate::layout::RectF {
            x: 50.0,
            y: 20.0,
            width: 400.0,
            height: 300.0,
        };
        crate::chrome::each_axis_tick_label(
            &config.bottom_x,
            crate::layout::Side::Bottom,
            &plot,
            &Fixed,
            &[],
            |text, _| {
                texts.push(matches!(text, crate::chrome::TickText::Power(_)));
            },
        );
        assert_eq!(texts, [true, false, false, false]);
    }

    #[test]
    fn headers_are_hit_and_dragged_into_the_axis_offset() {
        let mut config = chart();
        let map = HitMap::for_active_layer(&config);
        let id = map.find("category_groups_bottom").expect("registered");
        let element = map.get(id).unwrap();
        let headers = texts(|sink| element.emit(&config, &Fixed, sink));
        assert_eq!(headers.len(), 2, "one block per header");
        let left = headers[0];
        let (cx, cy) = (left.x + left.width / 2.0, left.y + left.height / 2.0);
        // Under the tick labels of the bottom axis.
        let plot_bottom = config.data_area().unwrap().0;
        assert!(cy > (plot_bottom.y + plot_bottom.height) as f32 + 10.0);
        assert_eq!(
            map.hit_test(&config, &Fixed, cx, cy),
            Some(id),
            "hit at {cx},{cy}"
        );
        let moved =
            element
                .as_draggable()
                .unwrap()
                .drag_from(&mut config, &Fixed, [cx, cy], 5.0, 7.0);
        assert_eq!(moved, NudgeResult::Moved);
        assert_eq!(
            config.bottom_x.category_groups.as_ref().unwrap().offset,
            [5.0, 7.0]
        );
        let after = texts(|sink| element.emit(&config, &Fixed, sink));
        assert_eq!((after[0].x - left.x, after[0].y - left.y), (5.0, 7.0));
        // An axis without groups places nothing and is never hit.
        let top = map.find("category_groups_top").unwrap();
        assert!(texts(|sink| map.get(top).unwrap().emit(&config, &Fixed, sink)).is_empty());
        let _ = Element::CategoryGroupLabel(crate::layout::Side::Top);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn groups_and_rich_labels_round_trip_and_stay_absent_by_default() {
        let plain = crate::default::default_config();
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json["bottom_x"].get("category_groups").is_none());
        let config = chart();
        let json = serde_json::to_value(&config).unwrap();
        assert!(
            json["bottom_x"]["categories"][0]
                .get("rich_label")
                .is_none()
        );
        assert_eq!(
            json["bottom_x"]["category_groups"]["groups"][1]["first"],
            "c"
        );
        assert!(json["bottom_x"]["category_groups"].get("offset").is_none());
        let back: Config = serde_json::from_value(json).unwrap();
        assert_eq!(back, config);
    }
}
