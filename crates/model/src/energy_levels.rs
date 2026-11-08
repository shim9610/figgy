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
    /// The runs of levels the host shows as bands (#205): a Rydberg series
    /// near its limit, a dense manifold. No count limit.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub bands: Vec<LevelBand>,
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
    /// The level bands (#205).
    #[cfg_attr(feature = "serde", serde(default))]
    pub band: LevelBandStyle,
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
            band: LevelBandStyle::default(),
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
    /// A level band's label (#205).
    Band,
}

/// A run of levels of one column the host shows as one band (#205): a
/// light fill from its first level to its last, one label beside it, and
/// the levels inside drawn thin without their own labels. The host chooses
/// the run, as it chooses the transitions it draws.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LevelBand {
    pub id: String,
    /// The column the band lies in; its levels' column.
    pub column: String,
    /// The ids of the band's lowest and highest levels, in either order.
    pub first: String,
    pub last: String,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub label: Option<RichText>,
    /// The label's drag offset, logical pixels.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "is_zero_offset")
    )]
    pub label_offset: [f32; 2],
}

/// The look of the level bands (#205).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct LevelBandStyle {
    /// The band's fill.
    pub fill: Color,
    /// The width of the levels inside a band, logical pixels, in `0..=16`.
    pub inner_width: f32,
}

impl Default for LevelBandStyle {
    fn default() -> Self {
        Self {
            fill: Color::from_rgba8(34, 52, 88, 40),
            inner_width: 0.8,
        }
    }
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
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        self.style.validate()?;
        if self.source.is_some() {
            if !self.levels.is_empty()
                || !self.limits.is_empty()
                || !self.transitions.is_empty()
                || !self.bands.is_empty()
            {
                return Err(crate::RuleKind::EnergyLevelsSourceWithRecords);
            }
            return Ok(());
        }
        if !ids_unique(self.levels.iter().map(|l| l.id.as_str())) {
            return Err(crate::RuleKind::EnergyLevelIds);
        }
        if !ids_unique(self.limits.iter().map(|l| l.id.as_str())) {
            return Err(crate::RuleKind::IonizationLimitIds);
        }
        if !ids_unique(self.transitions.iter().map(|t| t.id.as_str())) {
            return Err(crate::RuleKind::TransitionIds);
        }
        if !ids_unique(self.bands.iter().map(|b| b.id.as_str())) {
            return Err(crate::RuleKind::LevelBandIds);
        }
        for band in &self.bands {
            let in_column = |id: &str| self.level(id).is_some_and(|l| l.column == band.column);
            if !in_column(&band.first) || !in_column(&band.last) {
                return Err(crate::RuleKind::LevelBandEnds);
            }
            if !offset_ok(band.label_offset) {
                return Err(crate::RuleKind::LevelBandLabelOffset);
            }
        }
        for level in &self.levels {
            level.validate()?;
            if let LevelRole::Component { multiplet } = &level.role {
                let centroid = self.level(multiplet);
                if !centroid.is_some_and(|c| c.role == LevelRole::Centroid) {
                    return Err(crate::RuleKind::FineStructureCentroid);
                }
            }
        }
        for limit in &self.limits {
            limit.validate()?;
        }
        for transition in &self.transitions {
            transition.validate()?;
            if transition.lower == transition.upper {
                return Err(crate::RuleKind::TransitionSameLevel);
            }
            if self.level(&transition.lower).is_none() || self.level(&transition.upper).is_none() {
                return Err(crate::RuleKind::TransitionLevels);
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
            LabelPart::Band => self.bands.len(),
        }
    }

    /// The drag offset of the label of row `index` of `part`, or `None`
    /// past the end.
    pub fn label_offset(&self, part: LabelPart, index: usize) -> Option<[f32; 2]> {
        match part {
            LabelPart::Level => self.levels.get(index).map(|l| l.label_offset),
            LabelPart::Transition => self.transitions.get(index).map(|t| t.label_offset),
            LabelPart::Limit => self.limits.get(index).map(|l| l.label_offset),
            LabelPart::Band => self.bands.get(index).map(|b| b.label_offset),
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
            LabelPart::Band => self.bands.get_mut(index).map(|b| &mut b.label_offset),
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
    ) -> Result<(), crate::RuleKind> {
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
    fn validate(&self) -> Result<(), crate::RuleKind> {
        if !self.energy.is_finite() {
            return Err(crate::RuleKind::EnergyLevelEnergy);
        }
        if !non_negative(self.uncertainty) || !non_negative(self.g) {
            return Err(crate::RuleKind::EnergyLevelUncertainty);
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
            return Err(crate::RuleKind::EnergyLevelText);
        }
        if let LevelRole::Component { multiplet } = &self.role {
            if !text_ok(multiplet) {
                return Err(crate::RuleKind::ComponentMultipletId);
            }
        }
        if !rich_ok(&self.label) || !offset_ok(self.label_offset) {
            return Err(crate::RuleKind::EnergyLevelLabel);
        }
        Ok(())
    }
}

impl IonizationLimit {
    fn validate(&self) -> Result<(), crate::RuleKind> {
        if !self.energy.is_finite() || !non_negative(self.uncertainty) {
            return Err(crate::RuleKind::IonizationLimitEnergy);
        }
        if !text_ok(&self.reference) || !rich_ok(&self.label) || !offset_ok(self.label_offset) {
            return Err(crate::RuleKind::IonizationLimitLabel);
        }
        Ok(())
    }
}

impl Transition {
    fn validate(&self) -> Result<(), crate::RuleKind> {
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
            return Err(crate::RuleKind::TransitionValues);
        }
        if !finite_in(self.x_offset, -0.5, 0.5) {
            return Err(crate::RuleKind::TransitionXOffset);
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
            return Err(crate::RuleKind::TransitionText);
        }
        if !rich_ok(&self.label) || !offset_ok(self.label_offset) {
            return Err(crate::RuleKind::TransitionLabel);
        }
        Ok(())
    }
}

impl EnergyLevelStyle {
    fn validate(&self) -> Result<(), crate::RuleKind> {
        let level = &self.level;
        if !color_ok(level.color)
            || !finite_in(level.width, 0.5, 24.0)
            || !finite_in(level.slot_fraction, 0.05, 1.0)
        {
            return Err(crate::RuleKind::EnergyLevelStroke);
        }
        let limit = &self.limit;
        if !color_ok(limit.color) || !finite_in(limit.width, 0.0, 64.0) {
            return Err(crate::RuleKind::IonizationLimitLine);
        }
        let arrow = &self.transition;
        if !color_ok(arrow.color)
            || !finite_in(arrow.width, 0.5, 24.0)
            || !finite_in(arrow.head_length, 0.0, 64.0)
            || !finite_in(arrow.head_width, 0.0, 64.0)
        {
            return Err(crate::RuleKind::TransitionArrow);
        }
        if !finite_in(self.picked_width_scale, 1.0, 4.0) {
            return Err(crate::RuleKind::EnergyLevelsPickedWidth);
        }
        if !color_ok(self.band.fill) || !finite_in(self.band.inner_width, 0.0, 16.0) {
            return Err(crate::RuleKind::LevelBandStyle);
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
    pub fn validate_energy_levels(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
        for item in series {
            let Some(levels) = item.energy_levels() else {
                continue;
            };
            levels.validate()?;
            if !item.x_column.is_empty() || !item.y_column.is_empty() || item.z_column.is_some() {
                return Err(crate::RuleKind::EnergyLevelsColumns);
            }
            if levels.source.as_deref() == Some(item.series_id.as_str()) {
                return Err(crate::RuleKind::EnergyLevelsSelfSource);
            }
            if self.polar().is_some()
                || self.ternary().is_some()
                || self.parallel().is_some()
                || self.view_3d().is_some()
            {
                return Err(crate::RuleKind::EnergyLevelsChart);
            }
            if self.bottom_x.categories.is_none() {
                return Err(crate::RuleKind::EnergyLevelsCategoricalX);
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
/// The space kept between two stacked level labels, logical pixels.
const LABEL_STACK_GAP: f32 = 1.0;
/// How far a moved label stands off its stroke's end so its leader line
/// shows, logical pixels.
const LEADER_RUN: f32 = 10.0;
/// A run of at least this many labels in one lane whose strokes lie closer
/// than [`COLLAPSE_SPACING`] of a label's height apart collapses (#205):
/// its first and last labels stay, joined by a bracket, and the labels
/// between are not drawn. A Rydberg series near its limit is such a run.
const COLLAPSE_COUNT: usize = 4;
const COLLAPSE_SPACING: f32 = 0.5;
/// How far a stacked label may stand from its own stroke, in label
/// heights (#205). A stack that would move one of its labels farther is a
/// crowd, not a cluster: it collapses to its first and last labels joined
/// by a bracket, as a dense run does, and the rest is stacked again.
const MAX_LABEL_SHIFT: f32 = 1.5;
/// The widest share of a level's slot that the arrows meeting one of its
/// ends spread over, and the most room between two of them, pixels.
const ARROW_SPREAD: f32 = 0.6;
const ARROW_STEP: f32 = 12.0;
/// Where along its arrow a transition label is tried, from the middle out
/// in steps of a twentieth.
const LABEL_STOPS: [f32; 17] = [
    0.5, 0.45, 0.55, 0.4, 0.6, 0.35, 0.65, 0.3, 0.7, 0.25, 0.75, 0.2, 0.8, 0.15, 0.85, 0.1, 0.9,
];

/// Where the emit and the pick place a series' parts this draw (#205):
/// each arrow's ends spread across the levels they meet, each level label
/// stacked clear of the others in its column, and each transition label at
/// the first free spot along its arrow. Display parameters only (rule 1),
/// computed from the SSOT and the panel on every raster and pick and
/// dropped after it: the three lists are the per-raster placement scratch
/// #205 names (rule 3).
pub struct LevelLayout {
    arrows: Vec<Option<([f32; 2], [f32; 2])>>,
    /// Per label item (the levels, then the bands): its shift from its own
    /// place, `[run, dy]`; `run` is the leader's horizontal run (zero when
    /// the label did not move).
    level_shift: Vec<[f32; 2]>,
    /// Per label item: the side it is drawn on, the style's or the other
    /// one for a column whose labels would leave the plot on that side.
    level_side: Vec<LevelLabelSide>,
    /// Per level: whether it lies in a band (#205), drawn thin and without
    /// its label.
    inner: Vec<bool>,
    /// Per label item: whether a collapsed run hides it (#205).
    hidden: Vec<bool>,
    /// The collapsed runs (#205): the items of each run's first and last
    /// labels, which a bracket joins.
    brackets: Vec<(usize, usize)>,
    transition_spot: Vec<Option<([f32; 2], LevelLabelSide)>>,
}

/// The room a transition label keeps from the parts around it, pixels.
const LABEL_CLEARANCE: f32 = 2.0;

/// Whether the segment `a`–`b` passes within [`LABEL_CLEARANCE`] of `rect`
/// (Liang–Barsky clipping against the grown rectangle).
fn crosses(a: [f32; 2], b: [f32; 2], rect: crate::layout::RectF) -> bool {
    let c = LABEL_CLEARANCE;
    let (x0, y0) = (rect.x - c, rect.y - c);
    let (x1, y1) = (rect.x + rect.width + c, rect.y + rect.height + c);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [
        (-dx, a[0] - x0),
        (dx, x1 - a[0]),
        (-dy, a[1] - y0),
        (dy, y1 - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// Whether two rectangles come closer than [`LABEL_CLEARANCE`].
fn overlaps(a: crate::layout::RectF, b: crate::layout::RectF) -> bool {
    let c = LABEL_CLEARANCE;
    a.x < b.x + b.width + c
        && b.x < a.x + a.width + c
        && a.y < b.y + b.height + c
        && b.y < a.y + a.height + c
}

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

    /// The level rows transition row `index` joins, lower then upper, with
    /// both drawn.
    fn transition_rows(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        index: usize,
    ) -> Option<(usize, usize)> {
        let transition = records.transitions.get(index)?;
        let row = |id: &str| {
            let row = records.levels.iter().position(|l| l.id == id)?;
            self.level_stroke(records, style, row).map(|_| row)
        };
        Some((row(&transition.lower)?, row(&transition.upper)?))
    }

    /// The arrows of every transition row, their ends spread across the
    /// levels they meet: the arrows that meet one end of a level are
    /// ordered by where their other ends lie, so they do not cross there,
    /// and spaced over [`ARROW_SPREAD`] of the slot, at most [`ARROW_STEP`]
    /// apart. A host's `x_offset` adds to the spread.
    fn arrows(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
    ) -> Vec<Option<([f32; 2], [f32; 2])>> {
        let rows: Vec<Option<(usize, usize)>> = (0..records.transitions.len())
            .map(|index| self.transition_rows(records, style, index))
            .collect();
        let centre = |row: usize| -> Option<([f32; 2], f32)> {
            let (a, b) = self.level_stroke(records, style, row)?;
            Some(([(a[0] + b[0]) / 2.0, a[1]], (b[0] - a[0]).abs()))
        };
        // Each end's place among the arrows meeting that level: a sort key
        // per (transition, end), then the rank within the level.
        let mut ends: Vec<(usize, usize, bool, f32, f32)> = Vec::new();
        for (index, pair) in rows.iter().enumerate() {
            let Some((lower, upper)) = *pair else {
                continue;
            };
            for (row, other, is_lower) in [(lower, upper, true), (upper, lower, false)] {
                if let Some((c, _)) = centre(other) {
                    ends.push((row, index, is_lower, c[0], c[1]));
                }
            }
        }
        ends.sort_by(|a, b| {
            (a.0, a.3, a.4, a.1)
                .partial_cmp(&(b.0, b.3, b.4, b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut x_of = vec![[f32::NAN; 2]; records.transitions.len()];
        let mut start = 0;
        while start < ends.len() {
            let row = ends[start].0;
            let end = ends[start..]
                .iter()
                .position(|e| e.0 != row)
                .map_or(ends.len(), |n| start + n);
            let count = end - start;
            if let Some((c, width)) = centre(row) {
                let step = if count > 1 {
                    (width * ARROW_SPREAD / (count - 1) as f32).min(ARROW_STEP)
                } else {
                    0.0
                };
                for (k, e) in ends[start..end].iter().enumerate() {
                    let x = c[0] + (k as f32 - (count - 1) as f32 / 2.0) * step;
                    x_of[e.1][usize::from(!e.2)] = x;
                }
            }
            start = end;
        }
        rows.iter()
            .enumerate()
            .map(|(index, pair)| {
                let (lower, upper) = (*pair)?;
                let transition = &records.transitions[index];
                let shift = {
                    let x = self.column_x(&records.levels[lower].column)?;
                    let a = self.px(AnnotationAxis::X, x)?;
                    let b = self.px(AnnotationAxis::X, x + f64::from(transition.x_offset))?;
                    b - a
                };
                let point = |row: usize, slot: usize| -> Option<[f32; 2]> {
                    let (c, _) = centre(row)?;
                    let x = x_of[index][slot];
                    Some([if x.is_finite() { x } else { c[0] } + shift, c[1]])
                };
                let lower = point(lower, 0)?;
                let upper = point(upper, 1)?;
                Some(match style.transition.direction {
                    TransitionDirection::Emission => (upper, lower),
                    TransitionDirection::Absorption | TransitionDirection::None => (lower, upper),
                })
            })
            .collect()
    }

    /// The fill of band `index` (#205): its column's slot, from its lowest
    /// to its highest level, half a stroke past each; `None` when either
    /// level is not drawn.
    fn band_rect(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        index: usize,
    ) -> Option<crate::layout::RectF> {
        let band = records.bands.get(index)?;
        let stroke = |id: &str| {
            let row = records.levels.iter().position(|l| l.id == id)?;
            self.level_stroke(records, style, row)
        };
        let (a0, b0) = stroke(&band.first)?;
        let (a1, _) = stroke(&band.last)?;
        let half = style.level.width / 2.0;
        let (top, bottom) = (a0[1].min(a1[1]) - half, a0[1].max(a1[1]) + half);
        Some(crate::layout::RectF {
            x: a0[0].min(b0[0]),
            y: top,
            width: (b0[0] - a0[0]).abs(),
            height: bottom - top,
        })
    }

    /// Which levels lie in a band of their column (#205).
    fn inner_levels(&self, records: &EnergyLevelSeries, style: &EnergyLevelStyle) -> Vec<bool> {
        let mut inner = vec![false; records.levels.len()];
        for (index, band) in records.bands.iter().enumerate() {
            let Some(rect) = self.band_rect(records, style, index) else {
                continue;
            };
            for (row, level) in records.levels.iter().enumerate() {
                if level.column != band.column {
                    continue;
                }
                if let Some((a, _)) = self.level_stroke(records, style, row)
                    && a[1] >= rect.y - 0.01
                    && a[1] <= rect.y + rect.height + 0.01
                {
                    inner[row] = true;
                }
            }
        }
        inner
    }

    /// The column label item `item` (a level, then the bands) lies in.
    fn item_column<'r>(records: &'r EnergyLevelSeries, item: usize) -> Option<&'r str> {
        let levels = records.levels.len();
        if item < levels {
            Some(records.levels[item].column.as_str())
        } else {
            records.bands.get(item - levels).map(|b| b.column.as_str())
        }
    }

    /// The ends a label item stands beside: a level's stroke, or the
    /// middle of a band's fill across its slot (its top edge for a label
    /// above it).
    fn item_ends(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        item: usize,
        side: LevelLabelSide,
    ) -> Option<([f32; 2], [f32; 2])> {
        let levels = records.levels.len();
        if item < levels {
            return self.level_stroke(records, style, item);
        }
        let rect = self.band_rect(records, style, item - levels)?;
        let y = match side {
            LevelLabelSide::Above => rect.y,
            _ => rect.y + rect.height / 2.0,
        };
        Some(([rect.x, y], [rect.x + rect.width, y]))
    }

    /// The layout of `records` in this placement: see [`LevelLayout`].
    pub fn layout(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        measure: &dyn crate::text::MeasureText,
    ) -> LevelLayout {
        let arrows = self.arrows(records, style);
        let plot = self.space.plot_rect();
        let inner = self.inner_levels(records, style);
        // Label items: the levels, then the bands.
        let items = records.levels.len() + records.bands.len();
        // A column whose labels would leave the plot on the style's side
        // draws them on the other side.
        let mut level_side = vec![style.label_side; items];
        let flip = match style.label_side {
            LevelLabelSide::Right => Some(LevelLabelSide::Left),
            LevelLabelSide::Left => Some(LevelLabelSide::Right),
            LevelLabelSide::Above => None,
        };
        let outside = |rect: crate::layout::RectF| {
            rect.x < plot.x - 0.5 || rect.x + rect.width > plot.x + plot.width + 0.5
        };
        if let Some(other) = flip {
            let mut flipped: Vec<&str> = Vec::new();
            for index in 0..items {
                let Some(column) = Self::item_column(records, index) else {
                    continue;
                };
                if flipped.contains(&column) {
                    continue;
                }
                let leaves = self
                    .label_on(
                        records,
                        style,
                        &inner,
                        index,
                        measure,
                        [0.0; 2],
                        style.label_side,
                    )
                    .is_some_and(|t| outside(t.rect()));
                if leaves {
                    flipped.push(column);
                }
            }
            for (index, side) in level_side.iter_mut().enumerate() {
                if Self::item_column(records, index).is_some_and(|c| flipped.contains(&c)) {
                    *side = other;
                }
            }
        }
        // Level labels: per lane, in order of their strokes, stacked so no
        // two overlap. A lane is the gap a label is drawn in: right of its
        // column, left of it (the lane right of the column before), or above
        // it. A run of overlapping labels becomes one group centred on the
        // mean of their own places, and a group that then meets the one
        // above it merges with it.
        let lane = |index: usize| -> (u8, i64) {
            let column = Self::item_column(records, index)
                .and_then(|c| self.column_x(c))
                .map_or(i64::MIN, |x| x as i64);
            match level_side[index] {
                LevelLabelSide::Right => (0, column),
                LevelLabelSide::Left => (0, column - 1),
                LevelLabelSide::Above => (1, column),
            }
        };
        let mut level_shift = vec![[0.0f32; 2]; items];
        let mut hidden = vec![false; items];
        let mut brackets: Vec<(usize, usize)> = Vec::new();
        // Per item, its label on each side (right, left, above),
        // unshifted, measured the first time it is asked for: a shift
        // only translates it.
        let unshifted: Vec<[std::cell::OnceCell<Option<crate::layout::RectF>>; 3]> =
            (0..items).map(|_| Default::default()).collect();
        // Per item, an item of the stacked group or collapsed run it moves
        // with (#248): the group's first label, or the first end of the
        // bracket that hid it.
        let mut root: Vec<usize> = (0..items).collect();
        let mut placed: Vec<(usize, f32, f32)> = Vec::new();
        for index in 0..items {
            let Some(text) = self.label_at(
                records,
                style,
                &inner,
                index,
                measure,
                [0.0; 2],
                level_side[index],
            ) else {
                continue;
            };
            let rect = text.rect();
            let slot = match level_side[index] {
                LevelLabelSide::Right => 0,
                LevelLabelSide::Left => 1,
                LevelLabelSide::Above => 2,
            };
            unshifted[index][slot].get_or_init(|| Some(rect));
            placed.push((index, rect.y + rect.height / 2.0, rect.height));
        }
        placed.sort_by(|a, b| {
            lane(a.0)
                .cmp(&lane(b.0))
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut start = 0;
        while start < placed.len() {
            let key = lane(placed[start].0);
            let end = placed[start..]
                .iter()
                .position(|p| lane(p.0) != key)
                .map_or(placed.len(), |n| start + n);
            // Collapse the dense runs first: consecutive labels whose strokes
            // lie closer than half a label apart, at least four of them.
            let mut kept: Vec<(usize, f32, f32)> = Vec::new();
            let lane_run = &placed[start..end];
            let mut k = 0;
            while k < lane_run.len() {
                let mut j = k;
                while j + 1 < lane_run.len()
                    && (lane_run[j + 1].1 - lane_run[j].1).abs()
                        < COLLAPSE_SPACING * lane_run[j].2.max(lane_run[j + 1].2)
                {
                    j += 1;
                }
                if j + 1 - k >= COLLAPSE_COUNT {
                    for m in &lane_run[k + 1..j] {
                        hidden[m.0] = true;
                        root[m.0] = lane_run[k].0;
                    }
                    kept.push(lane_run[k]);
                    kept.push(lane_run[j]);
                    brackets.push((lane_run[k].0, lane_run[j].0));
                } else {
                    kept.extend_from_slice(&lane_run[k..=j]);
                }
                k = j + 1;
            }
            // Groups as (first, last, top), in screen order.
            let stack = |run: &[(usize, f32, f32)]| {
                let mut groups: Vec<(usize, usize, f32)> = Vec::new();
                for (k, &(_, centre, height)) in run.iter().enumerate() {
                    groups.push((k, k, centre - height / 2.0));
                    while groups.len() > 1 {
                        let (b0, b1, _) = groups[groups.len() - 1];
                        let (a0, a1, a_top) = groups[groups.len() - 2];
                        let a_height: f32 =
                            run[a0..=a1].iter().map(|p| p.2 + LABEL_STACK_GAP).sum();
                        if a_top + a_height <= groups[groups.len() - 1].2 {
                            break;
                        }
                        let members = &run[a0..=b1];
                        let total: f32 = members.iter().map(|p| p.2 + LABEL_STACK_GAP).sum();
                        let mean = members.iter().map(|p| p.1).sum::<f32>() / members.len() as f32;
                        let top = (mean - total / 2.0)
                            .min(plot.y + plot.height - total)
                            .max(plot.y);
                        groups.pop();
                        groups.pop();
                        groups.push((a0, b1.max(b0), top));
                    }
                }
                groups
            };
            // The farthest a group moves one of its labels, in its tallest
            // label's heights.
            let reach = |members: &[(usize, f32, f32)], top: f32| {
                let tallest = members.iter().map(|p| p.2).fold(0.0f32, f32::max).max(1.0);
                let mut y = top;
                let mut far = 0.0f32;
                for &(_, centre, height) in members {
                    far = far.max((y + height / 2.0 - centre).abs());
                    y += height + LABEL_STACK_GAP;
                }
                far / tallest
            };
            // A crowd collapses to its ends; the rest is stacked again,
            // until every stack keeps its labels near their strokes. Each
            // pass hides at least one label, so the walk ends.
            let mut groups = stack(&kept);
            loop {
                let mut collapsed = false;
                let mut next: Vec<(usize, f32, f32)> = Vec::with_capacity(kept.len());
                for &(first, last, top) in &groups {
                    let members = &kept[first..=last];
                    if members.len() <= 2 || reach(members, top) <= MAX_LABEL_SHIFT {
                        next.extend_from_slice(members);
                        continue;
                    }
                    // The crowd's dense runs (strokes closer than a label
                    // apart, three or more) collapse first, so a label
                    // with room of its own keeps it; a crowd without one
                    // collapses whole.
                    let close = |a: &(usize, f32, f32), b: &(usize, f32, f32)| {
                        (b.1 - a.1).abs() < a.2.max(b.2)
                    };
                    let mut runs = 0;
                    let mut k = 0;
                    while k < members.len() {
                        let mut j = k;
                        while j + 1 < members.len() && close(&members[j], &members[j + 1]) {
                            j += 1;
                        }
                        if j >= k + 2 {
                            for m in &members[k + 1..j] {
                                hidden[m.0] = true;
                                root[m.0] = members[k].0;
                            }
                            next.push(members[k]);
                            next.push(members[j]);
                            brackets.push((members[k].0, members[j].0));
                            runs += 1;
                        } else {
                            next.extend_from_slice(&members[k..=j]);
                        }
                        k = j + 1;
                    }
                    if runs == 0 {
                        // Drop what this group pushed and collapse it whole.
                        next.truncate(next.len() - members.len());
                        for m in &members[1..members.len() - 1] {
                            hidden[m.0] = true;
                            root[m.0] = members[0].0;
                        }
                        next.push(members[0]);
                        next.push(members[members.len() - 1]);
                        brackets.push((members[0].0, members[members.len() - 1].0));
                    }
                    collapsed = true;
                }
                if !collapsed {
                    break;
                }
                kept = next;
                groups = stack(&kept);
            }
            let run = &kept[..];
            for (first, last, top) in groups {
                let members = &run[first..=last];
                let moved = last > first;
                let mut y = top;
                for &(index, centre, height) in members {
                    // A group can hold labels of the two columns either
                    // side of its gap; each column's part is a unit.
                    let column = Self::item_column(records, index);
                    root[index] = members
                        .iter()
                        .find(|m| Self::item_column(records, m.0) == column)
                        .map_or(index, |m| m.0);
                    let new_centre = y + height / 2.0;
                    level_shift[index] =
                        [if moved { LEADER_RUN } else { 0.0 }, new_centre - centre];
                    y += height + LABEL_STACK_GAP;
                }
            }
            start = end;
        }
        // A bracket whose end a later collapse hid is part of the
        // larger bracket now.
        brackets.retain(|&(a, b)| !hidden[a] && !hidden[b]);
        brackets.sort_unstable();
        brackets.dedup();
        // Resolve each item to the root of its unit: a hidden end
        // points at the end that hid it, which belongs to a group.
        for item in 0..items {
            let mut r = root[item];
            let mut steps = 0;
            while root[r] != r && steps < items {
                r = root[r];
                steps += 1;
            }
            root[item] = r;
        }
        // The level strokes and band fills, sorted by their tops: the
        // obstacles a level label keeps clear of (#248) and, with the level
        // labels added below, a transition label.
        let mut obstacles: Vec<crate::layout::RectF> = Vec::new();
        for index in 0..records.levels.len() {
            if let Some((a, b)) = self.level_stroke(records, style, index) {
                let half = style.level.width / 2.0 + 1.0;
                obstacles.push(crate::layout::RectF {
                    x: a[0].min(b[0]),
                    y: a[1] - half,
                    width: (b[0] - a[0]).abs(),
                    height: half * 2.0,
                });
            }
        }
        for index in 0..records.bands.len() {
            if let Some(rect) = self.band_rect(records, style, index) {
                obstacles.push(rect);
            }
        }
        obstacles.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
        let rect_of = |item: usize, shift: [f32; 2], side: LevelLabelSide| {
            let (slot, run) = match side {
                LevelLabelSide::Right => (0, shift[0]),
                LevelLabelSide::Left => (1, -shift[0]),
                LevelLabelSide::Above => (2, 0.0),
            };
            unshifted[item][slot]
                .get_or_init(|| {
                    self.label_on(records, style, &inner, item, measure, [0.0; 2], side)
                        .map(|t| t.rect())
                })
                .map(|r| crate::layout::RectF {
                    x: r.x + run,
                    y: r.y + shift[1],
                    ..r
                })
        };
        // Labels that lie on a stroke, fill or limit line or meet another
        // label move (#248): a stacked group or collapsed run as one unit, or one of
        // its labels alone, to its column's other side, above its stroke,
        // or up or down, wherever it overlaps least within the plot. A
        // label beside its own column never reaches that column's strokes
        // or fill, so they count only above it. A unit with a label the
        // host moved keeps its place.
        if flip.is_some() {
            let tallest = obstacles.iter().map(|o| o.height).fold(0.0f32, f32::max);
            // The strokes, fills and limit lines a rect overlaps.
            let hits = |rect: crate::layout::RectF| {
                let from = obstacles.partition_point(|o| o.y < rect.y - tallest - LABEL_CLEARANCE);
                let to =
                    obstacles.partition_point(|o| o.y <= rect.y + rect.height + LABEL_CLEARANCE);
                let limits = (0..records.limits.len())
                    .filter_map(|index| self.limit_line(records, index))
                    .filter(|(a, _)| a[1] > rect.y && a[1] < rect.y + rect.height)
                    .count();
                obstacles[from..to.max(from)]
                    .iter()
                    .filter(|o| overlaps(**o, rect))
                    .count()
                    + limits
            };
            let inside = |rect: crate::layout::RectF| {
                !outside(rect)
                    && rect.y >= plot.y - 0.5
                    && rect.y + rect.height <= plot.y + plot.height + 0.5
            };
            let host_moved = |item: usize| {
                let levels = records.levels.len();
                let offset = if item < levels {
                    records.levels[item].label_offset
                } else {
                    records
                        .bands
                        .get(item - levels)
                        .map_or([0.0; 2], |b| b.label_offset)
                };
                offset != [0.0; 2]
            };
            // Per item: its drawn rect; and the drawn items sorted by their
            // tops, so a label meets only the ones whose rows reach it.
            let mut rects: Vec<Option<crate::layout::RectF>> = (0..items)
                .map(|item| {
                    (!hidden[item])
                        .then(|| rect_of(item, level_shift[item], level_side[item]))
                        .flatten()
                })
                .collect();
            let mut by_top: Vec<usize> = (0..items).filter(|&i| rects[i].is_some()).collect();
            let top = |rects: &[Option<crate::layout::RectF>], i: usize| {
                rects[i].map_or(f32::NEG_INFINITY, |r| r.y)
            };
            by_top.sort_by(|&a, &b| {
                top(&rects, a)
                    .partial_cmp(&top(&rects, b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let label_height = rects
                .iter()
                .flatten()
                .map(|r| r.height)
                .fold(0.0f32, f32::max);
            // Whether two labels share ink: stacked labels stand a gap
            // apart and never meet.
            let meets = |a: crate::layout::RectF, b: crate::layout::RectF| {
                a.x < b.x + b.width
                    && b.x < a.x + a.width
                    && a.y < b.y + b.height
                    && b.y < a.y + a.height
            };
            // The overlaps of `members`, the labels `moving` names, when
            // drawn at `place`: the strokes and fills they lie on and the other
            // labels they meet; `None` when one would leave the plot.
            let cost_of = |members: &[usize],
                           moving: &dyn Fn(usize) -> bool,
                           rects: &[Option<crate::layout::RectF>],
                           by_top: &[usize],
                           place: &dyn Fn(usize) -> Option<crate::layout::RectF>|
             -> Option<usize> {
                let mut cost = 0;
                for &item in members.iter().filter(|&&i| !hidden[i]) {
                    let Some(rect) = place(item) else {
                        continue;
                    };
                    if !inside(rect) {
                        return None;
                    }
                    cost += hits(rect);
                    let from = by_top.partition_point(|&o| top(rects, o) < rect.y - label_height);
                    let to = by_top.partition_point(|&o| top(rects, o) < rect.y + rect.height);
                    cost += by_top[from..to.max(from)]
                        .iter()
                        .filter(|&&other| !moving(other))
                        .filter_map(|&other| rects[other])
                        .filter(|other| meets(*other, rect))
                        .count();
                }
                Some(cost)
            };
            // A label that ends a bracket stays with its run.
            let bracket_end = |item: usize| brackets.iter().any(|&(a, b)| a == item || b == item);
            // The vertical moves a lone label may take, in quarters of a
            // label height; a stack of several moves by at most
            // `STACK_NUDGE` heights, so its leaders stay short.
            const STACK_NUDGE: f32 = 1.0;
            const NUDGES: [f32; 17] = [
                0.0, -0.25, 0.25, -0.5, 0.5, -0.75, 0.75, -1.0, 1.0, -1.25, 1.25, -1.5, 1.5, -2.0,
                2.0, -2.5, 2.5,
            ];
            // Sweeps over the units: each takes its first place of least
            // overlap (a unit, or one label of a stack taken out of it
            // alone) as soon as one overlaps less, until a sweep moves
            // nothing. A move lowers the overlaps of what moved, and two
            // labels that meet count for both, so the strokes and fills
            // under every label plus the pairs of labels of different
            // units that meet fall with each move: the sweeps end.
            let mut by_unit: Vec<usize> = (0..items).collect();
            loop {
                by_unit.sort_by_key(|&i| root[i]);
                let mut swept = false;
                let mut first = 0;
                while first < items {
                    let key = root[by_unit[first]];
                    let mut past = first + 1;
                    while past < items && root[by_unit[past]] == key {
                        past += 1;
                    }
                    let chunk = &by_unit[first..past];
                    first = past;
                    // The chunk's labels still in its unit: an earlier move
                    // of this sweep may have taken one out.
                    let unit = root[chunk[0]];
                    let mut taken = false;
                    let lone_count = chunk.iter().filter(|&&i| root[i] == unit).count();
                    for single in std::iter::once(None).chain(chunk.iter().copied().map(Some)) {
                        if taken {
                            break;
                        }
                        let unit = match single {
                            Some(one) => {
                                if root[one] != unit
                                    || lone_count < 2
                                    || hidden[one]
                                    || bracket_end(one)
                                    || rects[one].is_none()
                                {
                                    continue;
                                }
                                unit
                            }
                            None => unit,
                        };
                        let moving = |i: usize| match single {
                            Some(one) => i == one,
                            None => root[i] == unit,
                        };
                        let in_unit = |i: &&usize| root[**i] == unit;
                        if chunk.iter().filter(in_unit).any(|&i| host_moved(i))
                            || chunk
                                .iter()
                                .filter(in_unit)
                                .any(|&i| level_side[i] == LevelLabelSide::Above)
                        {
                            break;
                        }
                        let one;
                        let moved_items: &[usize] = match single {
                            Some(item) => {
                                one = [item];
                                &one[..]
                            }
                            None => chunk,
                        };
                        let current = |item: usize| if moving(item) { rects[item] } else { None };
                        let Some(now) = cost_of(moved_items, &moving, &rects, &by_top, &current)
                        else {
                            continue;
                        };
                        if now == 0 {
                            // A unit clear of everything has no label that
                            // is not: its stacked labels never meet.
                            if single.is_none() {
                                break;
                            }
                            continue;
                        }
                        let height = moved_items
                            .iter()
                            .filter(|&&i| moving(i))
                            .filter_map(|&i| rects[i])
                            .map(|r| r.height)
                            .fold(0.0f32, f32::max)
                            .max(1.0);
                        let side = level_side[moved_items
                            .iter()
                            .copied()
                            .find(|&i| moving(i))
                            .unwrap_or(unit)];
                        let other = match side {
                            LevelLabelSide::Right => LevelLabelSide::Left,
                            _ => LevelLabelSide::Right,
                        };
                        let place_at = |item: usize, to: LevelLabelSide, dy: f32| {
                            let shift = level_shift[item];
                            let moved = dy != 0.0 || shift[0] != 0.0;
                            let shift = match to {
                                LevelLabelSide::Above => [0.0; 2],
                                _ => [if moved { LEADER_RUN } else { 0.0 }, shift[1] + dy],
                            };
                            (shift, rect_of(item, shift, to))
                        };
                        // Nearest first: the other side at the same height,
                        // a lone label above its stroke, then each nudge on
                        // either side; a later place must overlap less.
                        let lone = lone_count == 1 || single.is_some();
                        let candidates = std::iter::once((other, 0.0))
                            .chain(lone.then_some((LevelLabelSide::Above, 0.0)))
                            .chain(
                                NUDGES[1..]
                                    .iter()
                                    .filter(|n| lone || n.abs() <= STACK_NUDGE)
                                    .flat_map(|&n| [(side, n * height), (other, n * height)]),
                            );
                        let mut best: Option<(usize, LevelLabelSide, f32)> = None;
                        for (to, dy) in candidates {
                            let place = |item: usize| {
                                if moving(item) {
                                    place_at(item, to, dy).1
                                } else {
                                    None
                                }
                            };
                            if let Some(cost) =
                                cost_of(moved_items, &moving, &rects, &by_top, &place)
                                && cost < best.map_or(now, |(c, _, _)| c)
                            {
                                best = Some((cost, to, dy));
                                if cost == 0 {
                                    break;
                                }
                            }
                        }
                        let Some((_, to, dy)) = best else {
                            continue;
                        };
                        // A label taken out of its stack is a unit of its
                        // own; when it was the stack's root, the rest takes
                        // its next label.
                        if let Some(one) = single {
                            if root[one] == one
                                && let Some(&next) =
                                    chunk.iter().find(|&&i| i != one && root[i] == one)
                            {
                                for &item in chunk {
                                    if item != one && root[item] == one {
                                        root[item] = next;
                                    }
                                }
                            }
                            root[one] = one;
                        }
                        for &item in moved_items {
                            if single.is_none() && root[item] != unit {
                                continue;
                            }
                            let held = level_shift[item];
                            let moved = dy != 0.0 || held[0] != 0.0;
                            let shift = match to {
                                LevelLabelSide::Above => [0.0; 2],
                                _ => [if moved { LEADER_RUN } else { 0.0 }, held[1] + dy],
                            };
                            level_shift[item] = shift;
                            level_side[item] = to;
                            rects[item] =
                                (!hidden[item]).then(|| rect_of(item, shift, to)).flatten();
                        }
                        by_top.sort_by(|&a, &b| {
                            top(&rects, a)
                                .partial_cmp(&top(&rects, b))
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        swept = true;
                        taken = single.is_none();
                    }
                }
                if !swept {
                    break;
                }
            }
        }
        // Transition labels: the first spot along each arrow, right of it
        // then left, clear of the level labels, the strokes, the other
        // arrows and the labels placed before it; when none is clear, the
        // middle.
        for index in (0..items).filter(|i| !hidden[*i]) {
            if let Some(rect) = rect_of(index, level_shift[index], level_side[index]) {
                obstacles.push(rect);
            }
        }
        // The strokes and level labels sorted by their tops, so a spot
        // tests only the ones whose rows can reach it; the transition labels
        // placed so far are a list of their own.
        obstacles.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
        let tallest = obstacles.iter().map(|o| o.height).fold(0.0f32, f32::max);
        let mut placed_labels: Vec<crate::layout::RectF> = Vec::new();
        let near = |rect: crate::layout::RectF| {
            let from = obstacles.partition_point(|o| o.y < rect.y - tallest - LABEL_CLEARANCE);
            let to = obstacles.partition_point(|o| o.y <= rect.y + rect.height + LABEL_CLEARANCE);
            &obstacles[from..to.max(from)]
        };
        let mut transition_spot = vec![None; records.transitions.len()];
        for (index, transition) in records.transitions.iter().enumerate() {
            let (Some(text), Some((tail, tip))) = (transition.label.as_ref(), arrows[index]) else {
                continue;
            };
            let at = |t: f32| {
                [
                    tail[0] + (tip[0] - tail[0]) * t,
                    tail[1] + (tip[1] - tail[1]) * t,
                ]
            };
            let block = |t: f32, side: LevelLabelSide| {
                let p = at(t);
                Self::label_block(text, measure, p, p[0], side, transition.label_offset).rect()
            };
            // The cost of a spot: a label or stroke it overlaps weighs more
            // than every arrow it crosses together, so a crowded bundle
            // still keeps its labels apart. The first spot of least cost
            // wins; a label the host moved stays at the middle.
            let host_placed = transition.label_offset != [0.0; 2];
            let cost = |t: f32, side: LevelLabelSide| -> usize {
                let rect = block(t, side);
                let hits = near(rect).iter().filter(|o| overlaps(**o, rect)).count()
                    + placed_labels.iter().filter(|o| overlaps(**o, rect)).count();
                let reach = |a: [f32; 2], b: [f32; 2]| {
                    a[0].max(b[0]) >= rect.x - LABEL_CLEARANCE
                        && a[0].min(b[0]) <= rect.x + rect.width + LABEL_CLEARANCE
                        && a[1].max(b[1]) >= rect.y - LABEL_CLEARANCE
                        && a[1].min(b[1]) <= rect.y + rect.height + LABEL_CLEARANCE
                };
                let crossings = arrows
                    .iter()
                    .enumerate()
                    .filter(|(other, arrow)| {
                        *other != index
                            && arrow.is_some_and(|(a, b)| reach(a, b) && crosses(a, b, rect))
                    })
                    .count();
                hits * (arrows.len() + 1) + crossings
            };
            let spot = if host_placed {
                (at(0.5), LevelLabelSide::Right)
            } else {
                let mut best = (usize::MAX, 0.5, LevelLabelSide::Right);
                'search: for &t in &LABEL_STOPS {
                    for side in [LevelLabelSide::Right, LevelLabelSide::Left] {
                        let c = cost(t, side);
                        if c < best.0 {
                            best = (c, t, side);
                            if c == 0 {
                                break 'search;
                            }
                        }
                    }
                }
                (at(best.1), best.2)
            };
            let rect = {
                let p = spot.0;
                Self::label_block(text, measure, p, p[0], spot.1, transition.label_offset).rect()
            };
            placed_labels.push(rect);
            transition_spot[index] = Some(spot);
        }
        LevelLayout {
            arrows,
            level_shift,
            level_side,
            inner,
            hidden,
            brackets,
            transition_spot,
        }
    }

    /// The arrow of transition row `index` in `layout`: its tail and its
    /// tip (the head end under `style.transition.direction`; for `None`,
    /// lower then upper), or `None` when either level is not drawn.
    pub fn transition_arrow(
        &self,
        layout: &LevelLayout,
        index: usize,
    ) -> Option<([f32; 2], [f32; 2])> {
        layout.arrows.get(index).copied().flatten()
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

    /// The label of level row `index` moved by `shift` (`[run, dy]`, see
    /// [`LevelLayout`]) away from its own place: further out by the run, on
    /// its side, and down by `dy`.
    #[allow(clippy::too_many_arguments)]
    fn label_on<'t>(
        &self,
        records: &'t EnergyLevelSeries,
        style: &EnergyLevelStyle,
        inner: &[bool],
        index: usize,
        measure: &dyn crate::text::MeasureText,
        shift: [f32; 2],
        side: LevelLabelSide,
    ) -> Option<crate::sink::PlacedText<'t>> {
        let levels = records.levels.len();
        let (text, offset) = if index < levels {
            if inner.get(index).copied().unwrap_or(false) {
                return None;
            }
            let level = &records.levels[index];
            (level.label.as_ref()?, level.label_offset)
        } else {
            let band = records.bands.get(index - levels)?;
            (band.label.as_ref()?, band.label_offset)
        };
        let (a, b) = self.item_ends(records, style, index, side)?;
        // A level outside the plot (beyond the energy range) is clipped
        // away, and so is its label: it is not pulled into the plot.
        let plot = self.space.plot_rect();
        if a[1] < plot.y - 0.5 || a[1] > plot.y + plot.height + 0.5 {
            return None;
        }
        let run = match side {
            LevelLabelSide::Right => shift[0],
            LevelLabelSide::Left => -shift[0],
            LevelLabelSide::Above => 0.0,
        };
        Some(Self::label_block(
            text,
            measure,
            b,
            a[0],
            side,
            [offset[0] + run, offset[1] + shift[1]],
        ))
    }

    /// The label of item `index` (a level, then the bands) on the side
    /// `layout` gives it.
    #[allow(clippy::too_many_arguments)]
    fn label_at<'t>(
        &self,
        records: &'t EnergyLevelSeries,
        style: &EnergyLevelStyle,
        inner: &[bool],
        index: usize,
        measure: &dyn crate::text::MeasureText,
        shift: [f32; 2],
        side: LevelLabelSide,
    ) -> Option<crate::sink::PlacedText<'t>> {
        self.label_on(records, style, inner, index, measure, shift, side)
    }

    /// The leader line from level row `index`'s stroke to its label when
    /// the layout moved the label, or `None`.
    fn leader(
        &self,
        records: &EnergyLevelSeries,
        style: &EnergyLevelStyle,
        layout: &LevelLayout,
        index: usize,
        measure: &dyn crate::text::MeasureText,
    ) -> Option<([f32; 2], [f32; 2])> {
        let shift = *layout.level_shift.get(index)?;
        if shift[0] == 0.0 && shift[1].abs() < 0.5 {
            return None;
        }
        let side = *layout.level_side.get(index)?;
        let (a, b) = self.item_ends(records, style, index, side)?;
        let rect = self
            .label_at(records, style, &layout.inner, index, measure, shift, side)?
            .rect();
        let mid = rect.y + rect.height / 2.0;
        Some(match side {
            LevelLabelSide::Right => ([b[0] + 1.0, b[1]], [rect.x - 1.0, mid]),
            LevelLabelSide::Left => ([a[0] - 1.0, a[1]], [rect.x + rect.width + 1.0, mid]),
            LevelLabelSide::Above => (
                [(a[0] + b[0]) / 2.0, a[1] - 1.0],
                [rect.x + rect.width / 2.0, rect.y + rect.height + 1.0],
            ),
        })
    }

    /// The placed label of row `index` of `part` in `layout`, or `None`
    /// when the row has no label or is not drawn.
    pub fn label<'t>(
        &self,
        records: &'t EnergyLevelSeries,
        style: &EnergyLevelStyle,
        layout: &LevelLayout,
        part: LabelPart,
        index: usize,
        measure: &dyn crate::text::MeasureText,
    ) -> Option<crate::sink::PlacedText<'t>> {
        match part {
            LabelPart::Level | LabelPart::Band => {
                let item = match part {
                    LabelPart::Band => records.levels.len() + index,
                    _ => {
                        if index >= records.levels.len() {
                            return None;
                        }
                        index
                    }
                };
                if layout.hidden.get(item).copied().unwrap_or(false) {
                    return None;
                }
                let shift = layout.level_shift.get(item).copied().unwrap_or([0.0; 2]);
                let side = layout
                    .level_side
                    .get(item)
                    .copied()
                    .unwrap_or(style.label_side);
                self.label_at(records, style, &layout.inner, item, measure, shift, side)
            }
            LabelPart::Transition => {
                let transition = records.transitions.get(index)?;
                let (spot, side) = layout.transition_spot.get(index).copied().flatten()?;
                Some(Self::label_block(
                    transition.label.as_ref()?,
                    measure,
                    spot,
                    spot[0],
                    side,
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
        let layout = self.layout(records, style, measure);
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
        // The bands under their levels (#205).
        for index in 0..records.bands.len() {
            if let Some(rect) = self.band_rect(records, style, index) {
                sink.rect(slot, rect, Some(style.band.fill), None);
            }
        }
        for index in 0..records.levels.len() {
            if let Some((a, b)) = self.level_stroke(records, style, index) {
                let picked = self.picked == Some(PickedRow::Level(index));
                let width = if layout.inner[index] {
                    style.band.inner_width
                } else {
                    style.level.width
                };
                sink.stroke(
                    slot,
                    a,
                    b,
                    &StrokeStyle {
                        width: widen(picked, width),
                        color: style.level.color,
                        dash: &[],
                        tag: "energy_level",
                    },
                );
            }
        }
        let arrow = &style.transition;
        for (index, transition) in records.transitions.iter().enumerate() {
            let Some((tail, tip)) = self.transition_arrow(&layout, index) else {
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
        for index in 0..records.levels.len() + records.bands.len() {
            if let Some((a, b)) = self.leader(records, style, &layout, index, measure) {
                sink.stroke(
                    slot,
                    a,
                    b,
                    &StrokeStyle {
                        width: 0.6,
                        color: style.level.color,
                        dash: &[],
                        tag: "energy_leader",
                    },
                );
            }
        }
        // A collapsed run's bracket: from its first stroke to its last,
        // just past the strokes' ends on the labels' side, with a tick to
        // each end.
        for &(lo, hi) in &layout.brackets {
            let side = layout.level_side[lo];
            let ends = |item: usize| self.item_ends(records, style, item, side);
            let (Some((a0, b0)), Some((a1, _))) = (ends(lo), ends(hi)) else {
                continue;
            };
            let (x, tick) = match side {
                LevelLabelSide::Left => (a0[0] - LABEL_GAP / 2.0, LABEL_GAP / 2.0),
                _ => (b0[0] + LABEL_GAP / 2.0, -LABEL_GAP / 2.0),
            };
            let line = StrokeStyle {
                width: 0.8,
                color: style.level.color,
                dash: &[],
                tag: "energy_bracket",
            };
            sink.stroke(slot, [x, a0[1]], [x, a1[1]], &line);
            sink.stroke(slot, [x, a0[1]], [x + tick, a0[1]], &line);
            sink.stroke(slot, [x, a1[1]], [x + tick, a1[1]], &line);
        }
        for part in [
            LabelPart::Level,
            LabelPart::Band,
            LabelPart::Transition,
            LabelPart::Limit,
        ] {
            for index in 0..records.part_len(part) {
                if let Some(text) = self.label(records, style, &layout, part, index, measure) {
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
        let layout = self.layout(records, style, measure);
        let mut best: Option<(LevelHit, f32)> = None;
        let mut offer = |hit: LevelHit, distance: f32| {
            if distance <= max_distance && best.is_none_or(|(_, d)| distance <= d) {
                best = Some((hit, distance));
            }
        };
        // A band answers for its first level anywhere on its fill; the
        // levels inside it, offered after, win on their strokes.
        for (index, band) in records.bands.iter().enumerate() {
            if let (Some(rect), Some(row)) = (
                self.band_rect(records, style, index),
                records.levels.iter().position(|l| l.id == band.first),
            ) {
                offer(LevelHit::Level(row), rect_distance(cursor, rect));
            }
        }
        for index in 0..records.levels.len() {
            if let Some((a, b)) = self.level_stroke(records, style, index) {
                let width = if layout.inner[index] {
                    style.band.inner_width
                } else {
                    style.level.width
                };
                let edge = segment_distance(cursor, a, b) - width / 2.0;
                offer(LevelHit::Level(index), edge.max(0.0));
            }
        }
        for index in 0..records.transitions.len() {
            if let Some((tail, tip)) = self.transition_arrow(&layout, index) {
                let half = style.transition.width.max(style.transition.head_width) / 2.0;
                let edge = segment_distance(cursor, tail, tip) - half;
                offer(LevelHit::Transition(index), edge.max(0.0));
            }
        }
        for part in [
            LabelPart::Level,
            LabelPart::Band,
            LabelPart::Transition,
            LabelPart::Limit,
        ] {
            for index in 0..records.part_len(part) {
                if let Some(text) = self.label(records, style, &layout, part, index, measure) {
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
    /// The subplot nested in this chart's data area, or in an inset
    /// nested there, whose chart area holds the point `(x, y)` (in this
    /// config's pixels): an inset (#203). When several hold it, the one
    /// drawn on top answers, the last of [`crate::layout::inset_paint_order`]
    /// (#205). A pointer there belongs to that subplot, whatever the active
    /// layer. `None` when no inset holds it.
    pub fn nested_subplot_at(&self, x: f32, y: f32) -> Option<usize> {
        use crate::layout::{Rect, inset_paint_order, inset_parent};
        let empty = Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        // Panel 0 is this chart, panel `i + 1` subplot `i`.
        let rects = |index: usize| {
            let config = match index {
                0 => self,
                _ => &self.subplots[index - 1].config,
            };
            (
                config.chart_area.0,
                config.data_area().map_or(empty, |d| d.0),
            )
        };
        let count = self.subplots.len() + 1;
        let root = |mut index: usize| {
            while let Some(parent) = inset_parent(count, rects, index) {
                index = parent;
            }
            index
        };
        inset_paint_order(count, rects)
            .filter(|&index| {
                let area = rects(index).0;
                root(index) == 0
                    && x >= area.x as f32
                    && y >= area.y as f32
                    && x < (area.x + area.width) as f32
                    && y < (area.y + area.height) as f32
            })
            .last()
            .map(|index| index - 1)
    }
}

impl crate::config::Config {
    /// The subplot whose frame (`chart_area`) is drawn on top at `(x, y)`
    /// (in this config's pixels), any subplot at any place (#205): panels of
    /// their own in list order, then insets in
    /// [`crate::layout::inset_paint_order`], the order they are painted in.
    /// A pointer there belongs to that subplot on the base layer, inside the
    /// parent chart or outside it. `None` when no frame holds it.
    pub fn subplot_at(&self, x: f32, y: f32) -> Option<usize> {
        use crate::layout::{Rect, inset_paint_order, inset_parent};
        let empty = Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        let rects = |index: usize| {
            let config = match index {
                0 => self,
                _ => &self.subplots[index - 1].config,
            };
            (
                config.chart_area.0,
                config.data_area().map_or(empty, |d| d.0),
            )
        };
        let holds = |index: usize| {
            let area = rects(index).0;
            x >= area.x as f32
                && y >= area.y as f32
                && x < (area.x + area.width) as f32
                && y < (area.y + area.height) as f32
        };
        let count = self.subplots.len() + 1;
        if count == 1 {
            return None;
        }
        inset_paint_order(count, rects)
            .filter(|&index| index != 0 && holds(index))
            .last()
            .or_else(|| {
                (1..count)
                    .filter(|&index| inset_parent(count, rects, index).is_none() && holds(index))
                    .last()
            })
            .map(|index| index - 1)
    }
}

impl crate::config::Config {
    /// The subplots that are insets now, nested in another panel's data
    /// area, in the order they are painted ([`crate::layout::inset_paint_order`],
    /// #205): bottom to top. A subplot moved out of every data area is a
    /// panel of its own and is not listed; it still takes the pointer on the
    /// base layer ([`Self::subplot_at`]).
    pub fn insets(&self) -> impl Iterator<Item = usize> + '_ {
        use crate::layout::{Rect, inset_paint_order};
        let empty = Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        let rects = move |index: usize| {
            let config = match index {
                0 => self,
                _ => &self.subplots[index - 1].config,
            };
            (
                config.chart_area.0,
                config.data_area().map_or(empty, |d| d.0),
            )
        };
        inset_paint_order(self.subplots.len() + 1, rects)
            .filter(|&index| index != 0)
            .map(|index| index - 1)
    }
}

/// Where an inset's marker is drawn on its parent (#205), in the parent's
/// pixels: the window clipped to the parent's plot and, when the marker
/// draws them and the inset's plot lies clear of the window, the two
/// connectors from the window's facing corners to the inset's plot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InsetMarkerParts {
    pub window: crate::layout::RectF,
    pub links: Option<[[[f32; 2]; 2]; 2]>,
}

impl crate::config::Config {
    /// The window subplot `subplot` magnifies, derived from it (#205):
    /// `[[x0, x1], [y0, y1]]` in this chart's data coordinates. `y` is the
    /// inset's `left_y` range. On a categorical `bottom_x`, `x` covers the
    /// slots of this chart's categories that the inset's categories name
    /// by id (the column whose records an energy-level `Components` view
    /// shows), cut to the inset's own window; on a numeric one it is the
    /// inset's `bottom_x` range. `None` when the inset names none of this
    /// chart's categories or its x axis is of the other kind.
    pub fn inset_window(&self, subplot: usize) -> Option<[[f64; 2]; 2]> {
        let inset = self.subplot_config(subplot)?;
        let (y0, y1) = inset.left_y.data_range();
        let x = match (&self.bottom_x.categories, &inset.bottom_x.categories) {
            (Some(parent), Some(own)) => {
                let (lo, hi) = inset.bottom_x.data_range();
                let slot = |k: usize| parent.iter().position(|c| c.id == own[k].id);
                // The parent slot of the inset's first category, and whether
                // the inset's categories follow it slot by slot.
                let first = slot(0)?;
                let consecutive = (0..own.len()).all(|k| slot(k) == Some(first + k));
                if consecutive {
                    [first as f64 + lo, first as f64 + hi]
                } else {
                    let (a, b) = (0..own.len())
                        .filter_map(slot)
                        .fold((usize::MAX, 0), |(a, b), p| (a.min(p), b.max(p)));
                    [a as f64 - 0.5, b as f64 + 0.5]
                }
            }
            (None, None) => {
                let (a, b) = inset.bottom_x.data_range();
                [a, b]
            }
            _ => return None,
        };
        Some([x, [y0, y1]])
    }

    /// Where subplot `subplot`'s marker is drawn on this chart (#205), from
    /// its [`crate::layer::InsetMarker`]: its explicit window or the derived
    /// one ([`Self::inset_window`]) in this chart's pixels, at least ten
    /// line widths across and clipped to the plot. `None` for a subplot without a marker, a window that has no
    /// position or lies outside the plot.
    pub fn inset_marker_parts(&self, subplot: usize) -> Option<InsetMarkerParts> {
        use crate::layout::{RectF, axis_fraction};
        let marker = self.subplots.get(subplot)?.marker.as_ref()?;
        let [[x0, x1], [y0, y1]] = marker.window.or_else(|| self.inset_window(subplot))?;
        let plot = RectF::from_rect(&self.data_area().ok()?.0);
        let px = |v: f64| plot.x + axis_fraction(v, &self.bottom_x) * plot.width;
        let py = |v: f64| plot.y + (1.0 - axis_fraction(v, &self.left_y)) * plot.height;
        let (a, b) = (px(x0), px(x1));
        let (c, d) = (py(y0), py(y1));
        if ![a, b, c, d].iter().all(|v| v.is_finite()) {
            return None;
        }
        // A window finer than the parent can show (a fine-structure inset
        // spans a ten-thousandth of the axis) is drawn at least ten line
        // widths across, about its centre: how it looks, not where. The
        // width scales with the export, so 2x is 1x doubled.
        let least = 10.0 * marker.line.width.max(0.0);
        let grow = |p: f32, q: f32| {
            let half = ((q - p).abs().max(least)) / 2.0;
            let centre = (p + q) / 2.0;
            (centre - half, centre + half)
        };
        let ((a, b), (c, d)) = (grow(a, b), grow(c, d));
        // Clipped to the plot: a window the view has moved off draws
        // nothing, one across the plot's edge is cut there.
        let left = a.min(b).max(plot.x);
        let right = a.max(b).min(plot.x + plot.width);
        let top = c.min(d).max(plot.y);
        let bottom = c.max(d).min(plot.y + plot.height);
        if left >= right || top >= bottom {
            return None;
        }
        let window = RectF {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        };
        let inset = RectF::from_rect(&self.subplot_config(subplot)?.data_area().ok()?.0);
        let links = marker.connectors.then(|| {
            let (w0, w1) = ((left, top), (right, bottom));
            let (i0, i1) = (
                (inset.x, inset.y),
                (inset.x + inset.width, inset.y + inset.height),
            );
            // The window's edge that faces the inset, joined to the
            // inset's facing edge, corner to corner.
            if i0.0 >= w1.0 {
                Some([[[w1.0, w0.1], [i0.0, i0.1]], [[w1.0, w1.1], [i0.0, i1.1]]])
            } else if i1.0 <= w0.0 {
                Some([[[w0.0, w0.1], [i1.0, i0.1]], [[w0.0, w1.1], [i1.0, i1.1]]])
            } else if i1.1 <= w0.1 {
                Some([[[w0.0, w0.1], [i0.0, i1.1]], [[w1.0, w0.1], [i1.0, i1.1]]])
            } else if i0.1 >= w1.1 {
                Some([[[w0.0, w1.1], [i0.0, i0.1]], [[w1.0, w1.1], [i1.0, i0.1]]])
            } else {
                None
            }
        });
        Some(InsetMarkerParts {
            window,
            links: links.flatten(),
        })
    }

    /// Stroke every subplot's marker into `sink` above the data (#205):
    /// the parts [`Self::inset_marker_parts`] places, tagged
    /// `"inset_marker"`. The annotation layer draws them, under the insets.
    pub fn emit_inset_markers(&self, sink: &mut dyn crate::sink::Sink) {
        use crate::sink::{Slot, StrokeStyle};
        for (index, subplot) in self.subplots.iter().enumerate() {
            let (Some(marker), Some(parts)) =
                (subplot.marker.as_ref(), self.inset_marker_parts(index))
            else {
                continue;
            };
            if marker.line.width <= 0.0 || marker.line.color.a <= 0.0 {
                continue;
            }
            let style = StrokeStyle {
                width: marker.line.width,
                color: marker.line.color,
                dash: marker.line.style.pattern(),
                tag: "inset_marker",
            };
            let w = parts.window;
            let corners = [
                [w.x, w.y],
                [w.x + w.width, w.y],
                [w.x + w.width, w.y + w.height],
                [w.x, w.y + w.height],
            ];
            for k in 0..4 {
                sink.stroke(Slot::Above, corners[k], corners[(k + 1) % 4], &style);
            }
            for [from, to] in parts.links.into_iter().flatten() {
                sink.stroke(Slot::Above, from, to, &style);
            }
        }
    }
}

impl crate::config::Config {
    /// The annotations that mark what inset `subplot` magnifies (#203): a
    /// [`AnnotationKind::Rect`] around the window `x` × `y` in this chart's
    /// data coordinates, and two headless [`AnnotationKind::Arrow`]s from
    /// the window's corners to the inset's plot (its data area), as ratio
    /// anchors of
    /// this chart's plot (so they stay on the inset as it is drawn now; call
    /// again after the inset moves). Superseded by
    /// [`crate::layer::Subplot::marker`] (#205), which is placed on every
    /// draw and follows the inset. Ids are `inset/{subplot}/window`,
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
mod band_tests {
    use super::*;

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

    fn band(first: &str, last: &str) -> LevelBand {
        LevelBand {
            id: "run".into(),
            column: "p".into(),
            first: first.into(),
            last: last.into(),
            label: None,
            label_offset: [0.0; 2],
        }
    }

    fn series() -> EnergyLevelSeries {
        EnergyLevelSeries {
            levels: vec![
                level("5p", "p", 1.0),
                level("6p", "p", 1.5),
                level("7p", "p", 1.7),
                level("5s", "s", 1.2),
            ],
            bands: vec![band("6p", "7p")],
            ..EnergyLevelSeries::default()
        }
    }

    #[test]
    fn a_band_names_two_levels_of_its_column() {
        assert!(series().validate().is_ok());
        let mut other = series();
        other.bands[0].last = "5s".into();
        assert!(other.validate().is_err(), "a level of another column");
        let mut missing = series();
        missing.bands[0].first = "9p".into();
        assert!(missing.validate().is_err(), "no such level");
        let mut twice = series();
        twice.bands.push(band("5p", "6p"));
        assert!(twice.validate().is_err(), "ids must be unique");
        let mut viewer = EnergyLevelSeries {
            source: Some("he".into()),
            ..EnergyLevelSeries::default()
        };
        viewer.bands.push(band("6p", "7p"));
        assert!(viewer.validate().is_err(), "a view holds no records");
    }

    #[test]
    fn a_band_label_drags_into_its_offset() {
        let mut s = series();
        assert_eq!(s.part_len(LabelPart::Band), 1);
        assert_eq!(
            s.drag_label(LabelPart::Band, 0, 3.0, -2.0),
            NudgeResult::Moved
        );
        assert_eq!(s.label_offset(LabelPart::Band, 0), Some([3.0, -2.0]));
        assert!(matches!(
            s.drag_label(LabelPart::Band, 1, 1.0, 0.0),
            NudgeResult::Rejected(_)
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn bands_round_trip_and_are_omitted_when_empty() {
        let s = series();
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["bands"][0]["first"], "6p");
        assert!(json["bands"][0].get("label").is_none());
        let back: EnergyLevelSeries = serde_json::from_value(json).unwrap();
        assert_eq!(back, s);
        let none = serde_json::to_value(EnergyLevelSeries::default()).unwrap();
        assert!(none.get("bands").is_none());
        assert_eq!(
            serde_json::to_value(LabelPart::Band).unwrap(),
            serde_json::json!("band")
        );
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

#[cfg(test)]
mod inset_marker_tests {
    use crate::config::{Category, Config};
    use crate::layer::{InsetMarker, Subplot};
    use crate::layout::{ChartArea, Rect};

    /// A 600 × 400 parent with categories a, b, c over 0..100, and an inset
    /// at its top right naming `b` over 40..50.
    fn document() -> Config {
        let mut config = crate::default::default_config();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 600,
            height: 400,
        });
        config.bottom_x.categories = Some(vec![
            Category::new("a", "A"),
            Category::new("b", "B"),
            Category::new("c", "C"),
        ]);
        config.left_y.min = 0.0;
        config.left_y.max = 100.0;
        let plot = config.data_area().unwrap().0;
        let mut inset = crate::default::default_config();
        inset.chart_area = ChartArea(Rect {
            x: plot.x + plot.width - 200,
            y: plot.y + 10,
            width: 190,
            height: 150,
        });
        inset.bottom_x.categories = Some(vec![Category::new("b", "B")]);
        inset.left_y.min = 40.0;
        inset.left_y.max = 50.0;
        config.subplots.push(Subplot {
            config: inset,
            marker: Some(InsetMarker::default()),
            ..Subplot::default()
        });
        config
    }

    // #205: the window is derived from the inset (its categories' slots and
    // its y range), placed in the parent's pixels, and joined to the inset
    // from the facing edge.
    #[test]
    fn the_window_is_derived_from_the_inset_and_follows_it() {
        let mut config = document();
        assert_eq!(config.inset_window(0), Some([[0.5, 1.5], [40.0, 50.0]]));
        let plot = crate::layout::RectF::from_rect(&config.data_area().unwrap().0);
        let parts = config.inset_marker_parts(0).expect("a marker");
        let w = parts.window;
        assert!((w.x - (plot.x + plot.width / 3.0)).abs() < 0.5, "{w:?}");
        assert!((w.width - plot.width / 3.0).abs() < 0.5, "{w:?}");
        assert!((w.y - (plot.y + plot.height * 0.5)).abs() < 0.5, "{w:?}");
        // The inset is right of the window: right corners to left corners.
        let inset = config.subplot_config(0).unwrap().data_area().unwrap().0;
        let [top, bottom] = parts.links.expect("connectors");
        assert_eq!(
            top,
            [[w.x + w.width, w.y], [inset.x as f32, inset.y as f32]]
        );
        assert_eq!(bottom[1], [inset.x as f32, (inset.y + inset.height) as f32]);
        // Moved left of the window, the inset is joined from the left edge.
        config.subplots[0].config.chart_area.0.x = plot.x as u32 + 4;
        config.subplots[0].config.chart_area.0.width = 160;
        let inset = config.subplot_config(0).unwrap().data_area().unwrap().0;
        let [top, _] = config.inset_marker_parts(0).unwrap().links.unwrap();
        assert_eq!(
            top,
            [[w.x, w.y], [(inset.x + inset.width) as f32, inset.y as f32]]
        );
        // No connectors when asked for none.
        config.subplots[0].marker.as_mut().unwrap().connectors = false;
        assert_eq!(config.inset_marker_parts(0).unwrap().links, None);
    }

    // #205: a view that moves the window off the plot draws nothing; one
    // across the edge is cut there. An explicit window wins, and a window
    // finer than ten line widths is drawn that wide.
    #[test]
    fn the_window_is_clipped_to_the_plot_and_kept_visible() {
        let mut config = document();
        let plot = crate::layout::RectF::from_rect(&config.data_area().unwrap().0);
        config.bottom_x.category_window = Some([1.6, 2.5]);
        assert_eq!(config.inset_marker_parts(0), None);
        config.bottom_x.category_window = Some([1.0, 2.5]);
        let w = config.inset_marker_parts(0).unwrap().window;
        assert_eq!(w.x, plot.x);
        config.bottom_x.category_window = None;
        config.subplots[0].marker.as_mut().unwrap().window = Some([[1.8, 2.2], [10.0, 10.001]]);
        let w = config.inset_marker_parts(0).unwrap().window;
        assert!((w.height - 10.0).abs() < 1e-3, "{w:?}");
        // No marker, no parts; an inset naming no parent category derives
        // no window.
        config.subplots[0].marker.as_mut().unwrap().window = None;
        config.subplots[0].config.bottom_x.categories = Some(vec![Category::new("z", "Z")]);
        assert_eq!(config.inset_window(0), None);
        config.subplots[0].marker = None;
        assert_eq!(config.inset_marker_parts(0), None);
    }

    #[test]
    fn the_marker_scales_and_serializes_only_when_set() {
        let mut config = document();
        config.scale_in_place(2.0);
        assert_eq!(config.subplots[0].marker.as_ref().unwrap().line.width, 2.0);
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string(&Subplot::default()).unwrap();
            assert!(!json.contains("marker"), "{json}");
            let json = serde_json::to_string(&config.subplots[0]).unwrap();
            let back: Subplot = serde_json::from_str(&json).unwrap();
            assert_eq!(back.marker, config.subplots[0].marker);
        }
    }
}
