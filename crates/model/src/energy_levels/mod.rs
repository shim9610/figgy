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

mod placement;

pub use placement::*;

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
    /// The unit of the transitions' `wavelength`, as the host writes it:
    /// `nm`, `Å`, `nm (air)`. The renderer draws it after a formatted label
    /// ([`WavelengthLabels::unit_on_labels`]) and in a unit note
    /// ([`AnnotationKind::SeriesUnit`]); it converts nothing. `None` (the
    /// default, omitted) draws no unit anywhere. One line of at most
    /// [`MAX_ENERGY_LEVEL_FIELD_CHARS`] characters. A series with a
    /// `source` keeps it `None`: the unit belongs to the records.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub wavelength_unit: Option<String>,
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
    /// The drawn label beside the arrow, written by the host. It wins over
    /// a label formatted from `wavelength`
    /// ([`EnergyLevelStyle::wavelength_labels`]); with neither, no label is
    /// drawn. [`Transition::drawn_label`] decides which one is drawn.
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
    /// The line's wavelength in the series' `wavelength_unit`: the NIST
    /// column the host chose (`obs_wl`, `ritz_wl`, in air or vacuum). The
    /// renderer formats it for a drawn label
    /// ([`EnergyLevelStyle::wavelength_labels`]) and never converts or
    /// chooses it (rule 1).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub wavelength: Option<f64>,
    /// The uncertainty of `wavelength`, in the same unit (NIST `unc_obs_wl`
    /// or `unc_ritz_wl`).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub wavelength_uncertainty: Option<f64>,
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
    /// Labels formatted from each transition's `wavelength` (#258): set,
    /// every transition with a wavelength and no host `label` gets one.
    /// `None` (the default, omitted) draws only the host's labels.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub wavelength_labels: Option<WavelengthLabels>,
}

/// How a transition's `wavelength` is written as its drawn label (#258).
/// The renderer formats the host's value for display and nothing else: it
/// converts no unit and chooses between no values (rule 1).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WavelengthLabels {
    /// The number's form: fixed decimals (at most
    /// [`MAX_WAVELENGTH_DECIMALS`]) or the automatic form, and whether
    /// trailing zeros are trimmed.
    #[cfg_attr(feature = "serde", serde(default))]
    pub format: crate::format::PlainNumberFormat,
    /// Append ` {unit}` (the series' `wavelength_unit`) to each label. No
    /// unit is appended when the series has none.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub unit_on_labels: bool,
    /// The look of the drawn labels: the font, size and colour of this
    /// text, the style a host label carries. Its segments are not drawn;
    /// the label's characters are the formatted wavelength.
    pub text: RichText,
}

impl Default for WavelengthLabels {
    fn default() -> Self {
        Self {
            format: crate::format::PlainNumberFormat::AUTO,
            unit_on_labels: false,
            text: RichText::plain("", Color::from_rgb8(40, 40, 40), 11.0, ""),
        }
    }
}

/// The most fixed decimals a wavelength label prints: past it an `f64`
/// carries no more digits. A text-length limit, not a count.
pub const MAX_WAVELENGTH_DECIMALS: u8 = 17;

impl WavelengthLabels {
    fn validate(&self) -> Result<(), crate::RuleKind> {
        let style = RichText::plain(
            "0",
            self.text.color,
            self.text.font_size,
            self.text.font.clone(),
        );
        if self
            .format
            .decimals
            .is_some_and(|d| d > MAX_WAVELENGTH_DECIMALS)
            || !style.within_limits()
        {
            return Err(crate::RuleKind::WavelengthLabels);
        }
        Ok(())
    }

    /// The label of a transition at `wavelength`: the value in
    /// [`Self::format`], then ` {unit}` when [`Self::unit_on_labels`] is set
    /// and `unit` is a unit, in the font, size and colour of
    /// [`Self::text`].
    pub fn label(&self, wavelength: f64, unit: Option<&str>) -> RichText {
        let mut text = self.format.format(wavelength);
        if self.unit_on_labels
            && let Some(unit) = unit.filter(|u| !u.is_empty())
        {
            text.push(' ');
            text.push_str(unit);
        }
        RichText::plain(
            &text,
            self.text.color,
            self.text.font_size,
            self.text.font.clone(),
        )
    }
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
            wavelength_labels: None,
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
        if let Some(unit) = &self.wavelength_unit
            && !text_ok(unit)
        {
            return Err(crate::RuleKind::WavelengthUnit);
        }
        if self.source.is_some() {
            if !self.levels.is_empty()
                || !self.limits.is_empty()
                || !self.transitions.is_empty()
                || !self.bands.is_empty()
                || self.wavelength_unit.is_some()
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
    /// The label drawn beside this transition's arrow (#258), the one
    /// owner of the choice: the host's `label` when it has one; else, when
    /// `labels` is set and the transition has a `wavelength`, that value
    /// written by [`WavelengthLabels::label`] with `unit`; else none. The
    /// placement, the drag, the pick, the picked highlight and the vector
    /// export all read the label through it.
    pub fn drawn_label(
        &self,
        labels: Option<&WavelengthLabels>,
        unit: Option<&str>,
    ) -> Option<std::borrow::Cow<'_, RichText>> {
        if let Some(label) = &self.label {
            return Some(std::borrow::Cow::Borrowed(label));
        }
        let (labels, wavelength) = (labels?, self.wavelength?);
        Some(std::borrow::Cow::Owned(labels.label(wavelength, unit)))
    }

    fn validate(&self) -> Result<(), crate::RuleKind> {
        let optional = [self.value, self.wavelength, self.aki];
        if optional.iter().any(|v| v.is_some_and(|v| !v.is_finite()))
            || !non_negative(self.wavelength_uncertainty)
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
        if let Some(labels) = &self.wavelength_labels {
            labels.validate()?;
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
        let px = |v: f64| plot.x + axis_fraction(v, &self.bottom_x, plot.width) * plot.width;
        let py =
            |v: f64| plot.y + (1.0 - axis_fraction(v, &self.left_y, plot.height)) * plot.height;
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

/// The wavelength unit the energy-level series `series_id` of `series`
/// draws (#258): the `wavelength_unit` of the records it draws (its own, or
/// its source's). `None` when no energy-level series of the list has that
/// id, its source holds no records, or the unit is `None` or empty. A unit
/// note ([`AnnotationKind::SeriesUnit`]) reads it on every draw.
pub fn wavelength_unit<'a>(series: &'a [SeriesConfig], series_id: &str) -> Option<&'a str> {
    let own = series
        .iter()
        .find(|s| s.series_id == series_id)?
        .energy_levels()?;
    drawn_records(own, series)?
        .wavelength_unit
        .as_deref()
        .filter(|unit| !unit.is_empty())
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
            wavelength: None,
            wavelength_uncertainty: None,
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
                    wavelength: Some(1083.0),
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
mod wavelength_label_tests {
    use super::*;
    use crate::annotation::{AnnotationSpace, AxisMap, AxisSpace, PixelSpace};
    use crate::config::Category;
    use crate::format::PlainNumberFormat;
    use crate::layout::{BreakMap, RectF};
    use crate::sink::{DrawnPart, Parts};
    use crate::text::{MeasureText, TextExtents};

    /// Eight pixels a character, 10 up and 3 down.
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

    fn chars(text: &RichText) -> String {
        text.segments.iter().map(|s| s.text).collect()
    }

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

    fn line(id: &str, lower: &str, upper: &str, wavelength: Option<f64>) -> Transition {
        Transition {
            id: id.into(),
            lower: lower.into(),
            upper: upper.into(),
            label: None,
            label_offset: [0.0; 2],
            value: None,
            x_offset: 0.0,
            wavelength,
            wavelength_uncertainty: None,
            intensity: String::new(),
            aki: None,
            accuracy: String::new(),
            line_type: String::new(),
            tp_reference: String::new(),
            line_reference: String::new(),
        }
    }

    fn labels(decimals: Option<u8>, unit_on_labels: bool) -> WavelengthLabels {
        WavelengthLabels {
            format: PlainNumberFormat {
                decimals,
                trim_zeros: false,
            },
            unit_on_labels,
            text: RichText::plain("", Color::from_rgb8(150, 90, 0), 11.0, "serif"),
        }
    }

    /// Two levels of two columns and two lines: one with a wavelength, one
    /// without.
    fn records() -> EnergyLevelSeries {
        let mut records = EnergyLevelSeries {
            levels: vec![level("low", "a", 2.0), level("high", "b", 20.0)],
            transitions: vec![
                line("t", "low", "high", Some(1083.034)),
                line("bare", "low", "high", None),
            ],
            wavelength_unit: Some("nm".into()),
            ..EnergyLevelSeries::default()
        };
        records.transitions[1].x_offset = 0.4;
        records.style.wavelength_labels = Some(labels(Some(1), true));
        records
    }

    fn space() -> PixelSpace {
        PixelSpace {
            panel: RectF {
                x: 0.,
                y: 0.,
                width: 600.,
                height: 400.,
            },
            plot: RectF {
                x: 50.,
                y: 20.,
                width: 500.,
                height: 360.,
            },
            x: AxisSpace {
                map: AxisMap::Index { count: 2 },
                inverted: false,
                breaks: BreakMap::IDENTITY,
            },
            y: AxisSpace {
                map: AxisMap::Linear { min: 0., max: 25. },
                inverted: false,
                breaks: BreakMap::IDENTITY,
            },
        }
    }

    #[test]
    fn a_wavelength_is_written_in_the_format_and_a_host_label_wins() {
        let t = line("t", "a", "b", Some(1083.034));
        let text = |labels: &WavelengthLabels, unit| {
            chars(&t.drawn_label(Some(labels), unit).expect("a label"))
        };
        assert_eq!(text(&labels(Some(1), false), Some("nm")), "1083.0");
        assert_eq!(text(&labels(Some(3), false), None), "1083.034");
        assert_eq!(text(&labels(Some(0), false), None), "1083");
        assert_eq!(text(&labels(None, false), None), "1083.034");
        let trimmed = WavelengthLabels {
            format: PlainNumberFormat {
                decimals: Some(4),
                trim_zeros: true,
            },
            ..labels(None, false)
        };
        assert_eq!(text(&trimmed, None), "1083.034");
        // The unit is appended only when asked for and when there is one.
        assert_eq!(text(&labels(Some(1), true), Some("nm")), "1083.0 nm");
        assert_eq!(
            text(&labels(Some(1), true), Some("nm (air)")),
            "1083.0 nm (air)"
        );
        assert_eq!(text(&labels(Some(1), true), None), "1083.0");
        assert_eq!(text(&labels(Some(1), true), Some("")), "1083.0");
        // The labels' font, size and colour.
        let drawn = t.drawn_label(Some(&labels(Some(1), false)), None).unwrap();
        assert_eq!(
            (drawn.font_size, drawn.color, drawn.font.as_str()),
            (11.0, Color::from_rgb8(150, 90, 0), "serif")
        );
        assert!(matches!(drawn, std::borrow::Cow::Owned(_)));
        // A host label wins, borrowed as it is.
        let host = Transition {
            label: Some(RichText::plain("He 1083", Color::BLACK, 12.0, "")),
            ..t.clone()
        };
        let drawn = host.drawn_label(Some(&labels(Some(1), true)), Some("nm"));
        assert!(matches!(&drawn, Some(std::borrow::Cow::Borrowed(_))));
        assert_eq!(chars(&drawn.unwrap()), "He 1083");
        // No wavelength or no labels: no label.
        assert_eq!(
            line("u", "a", "b", None).drawn_label(Some(&labels(Some(1), true)), Some("nm")),
            None
        );
        assert_eq!(t.drawn_label(None, Some("nm")), None);
    }

    #[test]
    fn a_formatted_label_is_placed_drawn_picked_and_dragged_as_a_host_label() {
        let space = space();
        let categories = [Category::new("a", "a"), Category::new("b", "b")];
        let placement = LevelPlacement {
            space: &space,
            categories: &categories,
            view: LevelView::Main,
            colorbar: None,
            picked: None,
        };
        let mut records = records();
        let style = records.style.clone();
        let placed = |records: &EnergyLevelSeries| {
            let layout = placement.layout(records, &style, &Fixed);
            let label = placement.label(records, &style, &layout, LabelPart::Transition, 0, &Fixed);
            label.map(|l| {
                let crate::sink::TextRef::Rich(text) = l.text else {
                    panic!("a rich label")
                };
                (chars(text), l.rect())
            })
        };
        let (text, rect) = placed(&records).expect("the line with a wavelength is labelled");
        assert_eq!(text, "1083.0 nm");
        let layout = placement.layout(&records, &style, &Fixed);
        assert!(
            placement
                .label(&records, &style, &layout, LabelPart::Transition, 1, &Fixed)
                .is_none(),
            "a line without a wavelength has no label"
        );
        // The emit draws it where the placement puts it.
        let mut texts = Vec::new();
        placement.emit(
            &records,
            &style,
            &Fixed,
            &mut Parts(&mut |part| {
                if let DrawnPart::Text(r) = part {
                    texts.push(r);
                }
            }),
        );
        assert_eq!(texts, [rect]);
        // The pick answers it as the transition's value label.
        let centre = [rect.x + rect.width / 2.0, rect.y + rect.height / 2.0];
        assert_eq!(
            placement.pick(&records, &style, &Fixed, centre, 4.0),
            Some((LevelHit::Label(LabelPart::Transition, 0), 0.0))
        );
        // A host-placed label keeps its place, and a drag moves it by the
        // delta, as a host label.
        records.transitions[0].label_offset = [0.0, 1.0];
        let (_, held) = placed(&records).unwrap();
        assert_eq!(
            records.drag_label(LabelPart::Transition, 0, 5.0, 7.0),
            NudgeResult::Moved
        );
        let (_, moved) = placed(&records).unwrap();
        assert!((moved.x - held.x - 5.0).abs() < 1e-3 && (moved.y - held.y - 7.0).abs() < 1e-3);
        // A host label wins over the formatted one.
        records.transitions[0].label = Some(RichText::plain("He", Color::BLACK, 12.0, ""));
        assert_eq!(placed(&records).unwrap().0, "He");
        // No unit: the number alone, and nothing else changes.
        let mut bare = self::records();
        bare.wavelength_unit = None;
        assert_eq!(placed(&bare).unwrap().0, "1083.0");
        // No labels style: the host's labels only.
        let mut off = self::records();
        off.style.wavelength_labels = None;
        let layout = placement.layout(&off, &off.style, &Fixed);
        assert!(
            placement
                .label(&off, &off.style, &layout, LabelPart::Transition, 0, &Fixed)
                .is_none()
        );
        let _ = space.plot_rect();
    }

    #[test]
    fn the_unit_and_the_labels_validate() {
        assert_eq!(records().validate(), Ok(()));
        let check = |edit: &dyn Fn(&mut EnergyLevelSeries)| {
            let mut r = records();
            edit(&mut r);
            r.validate()
        };
        assert_eq!(
            check(&|r| r.wavelength_unit = Some("n\nm".into())),
            Err(crate::RuleKind::WavelengthUnit)
        );
        assert_eq!(
            check(&|r| r.wavelength_unit = Some("m".repeat(161))),
            Err(crate::RuleKind::WavelengthUnit)
        );
        assert_eq!(
            check(&|r| r.style.wavelength_labels.as_mut().unwrap().format.decimals = Some(18)),
            Err(crate::RuleKind::WavelengthLabels)
        );
        assert_eq!(
            check(&|r| r.style.wavelength_labels.as_mut().unwrap().text.font_size = 2.0),
            Err(crate::RuleKind::WavelengthLabels)
        );
        assert_eq!(
            check(&|r| r.transitions[0].wavelength_uncertainty = Some(-1.0)),
            Err(crate::RuleKind::TransitionValues)
        );
        assert_eq!(
            check(&|r| r.transitions[0].wavelength = Some(f64::NAN)),
            Err(crate::RuleKind::TransitionValues)
        );
        // A view of another series holds no unit: the unit is the records'.
        let view = EnergyLevelSeries {
            source: Some("he".into()),
            wavelength_unit: Some("nm".into()),
            ..EnergyLevelSeries::default()
        };
        assert_eq!(
            view.validate(),
            Err(crate::RuleKind::EnergyLevelsSourceWithRecords)
        );
    }

    #[test]
    fn the_unit_is_read_from_the_records_a_series_draws() {
        let spec = |id: &str, levels: EnergyLevelSeries| SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: String::new(),
            y_column: String::new(),
            z_column: None,
            render_type: crate::data_config::DataRenderType::EnergyLevels(levels),
        };
        let view = EnergyLevelSeries {
            source: Some("he".into()),
            ..EnergyLevelSeries::default()
        };
        let list = [spec("he", records()), spec("inset", view)];
        assert_eq!(wavelength_unit(&list, "he"), Some("nm"));
        assert_eq!(wavelength_unit(&list, "inset"), Some("nm"), "its source's");
        assert_eq!(wavelength_unit(&list, "absent"), None);
        assert_eq!(wavelength_unit(&list[1..], "inset"), None, "no source");
        let mut none = records();
        none.wavelength_unit = None;
        assert_eq!(wavelength_unit(&[spec("he", none)], "he"), None);
        let mut empty = records();
        empty.wavelength_unit = Some(String::new());
        assert_eq!(wavelength_unit(&[spec("he", empty)], "he"), None);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn the_new_fields_round_trip_and_are_omitted_when_unset() {
        let full = records();
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(json["wavelength_unit"], "nm");
        assert_eq!(json["transitions"][0]["wavelength"], 1083.034);
        assert!(json["transitions"][1].get("wavelength").is_none());
        assert!(
            json["transitions"][0]
                .get("wavelength_uncertainty")
                .is_none()
        );
        let labels = &json["style"]["wavelength_labels"];
        assert_eq!(labels["format"]["decimals"], 1);
        assert_eq!(labels["unit_on_labels"], true);
        let back: EnergyLevelSeries = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, full);
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
        // Unset, nothing is written, and a document without them parses.
        let mut plain = full.clone();
        plain.wavelength_unit = None;
        plain.style.wavelength_labels = None;
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("wavelength_unit").is_none());
        assert!(json.get("style").is_none(), "a default style is omitted");
        let back: EnergyLevelSeries = serde_json::from_value(json).unwrap();
        assert_eq!(back, plain);
        let mut no_unit_flag = labels_json();
        no_unit_flag
            .as_object_mut()
            .unwrap()
            .remove("unit_on_labels");
        let parsed: WavelengthLabels = serde_json::from_value(no_unit_flag).unwrap();
        assert!(!parsed.unit_on_labels);
    }

    #[cfg(feature = "serde")]
    fn labels_json() -> serde_json::Value {
        serde_json::to_value(labels(Some(2), true)).unwrap()
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
