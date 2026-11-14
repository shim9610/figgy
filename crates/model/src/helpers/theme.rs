//! Themes (#265, #291, #295): looks written into the document once.
//!
//! A theme family ([`ThemeFamily`]: `gallery`, `cool-slate`, `midnight`,
//! `warm-paper`) is the way to theme a chart by name: it has a member
//! [`Theme`] for every [`ChartKind`] and a member [`RadialTheme`], and
//! [`Theme::apply_named`] gives each chart in scope its own kind's member,
//! so one name themes any chart. Any other look is a custom theme: a
//! [`Theme`] value, made for one kind (a family's member to start from,
//! [`ThemeFamily::theme`], with any field changed), applied with
//! [`Theme::apply`] or [`Theme::apply_to_its_kind`]; a chart of another
//! kind refuses it ([`ThemeError::WrongKind`]).
//!
//! A theme is a value, not a name stored in the document: applying one
//! writes exactly the fields it owns and keeps everything the user set as
//! content or layout. A theme owns:
//! - the chrome ([`ChromeTheme`]): the chart and plot fills, the draw style,
//!   the axis frame, the text look, the grid, the legend box and the
//!   selection highlight;
//! - one part per series family ([`SeriesTheme`]): colours through a
//!   palette, stroke widths, marker sizes and each family's materials.
//!
//! It keeps axis ranges, scales, tick spacing, label formats, categories,
//! axis breaks, the chart area, every offset, all text content, the
//! annotation layer, the overlay and subplot structure, series ids, columns
//! and labels, and the selection itself (only its highlight look changes).
//! Overlays take the theme with their chart; subplots take it as
//! [`SubplotScope`] chooses.
//!
//! Font sizes change what the margins must hold. The renderer's
//! `Chart::apply_theme` and `Chart::apply_custom_theme` refit the layout
//! after applying; a host that applies a theme to a bare [`Config`] calls
//! `layout::auto_layout` itself.

use crate::annotation::AnnotationKind;
use crate::color::Color;
use crate::colormap::ColorMap;
use crate::config::{
    AxisOptions, Config, DataSelectionsConfig, DrawStyle, GridOptions, TickVisibility,
};
use crate::data_config::{
    BoxPlotOverride, BoxPlotTarget, BoxPoints, CategoryBarDirection, CategoryBarOverride,
    CategoryBarTarget, DataRenderType, FieldSeries, RangeColor, SeriesConfig, Violin, ViolinBody,
    ViolinHalf,
};
use crate::feynman::{ParticleLine, Wiggle};
use crate::layer::LayerRef;
use crate::legend::Legend;
use crate::line::LineStylePreset;
use crate::material::Material;
use crate::preset::{AxisPreset, ColorCycle};

mod catalog;
mod family;
mod kind;
use crate::radial::{RadialChart, RadialOutline, RadialSlice};
use crate::text::RichText;
pub use family::ThemeFamily;
pub use kind::ChartKind;

/// A chart look made for one kind of chart: the chrome and one part per
/// series family. Every built-in theme defines every part; a host edits any
/// field before applying.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Theme {
    /// The kind of chart the theme is made for; [`Theme::apply`] refuses
    /// a chart of another kind.
    pub kind: ChartKind,
    pub chrome: ChromeTheme,
    pub series: SeriesTheme,
}

/// The look of everything around the data.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ChromeTheme {
    /// [`Config::background`]: the chart area's fill, or none.
    pub background: Option<Color>,
    /// [`Config::plot_background`]: the data area's fill, or none.
    pub plot_background: Option<Color>,
    pub draw_style: DrawStyle,
    /// The axis frame: which lines, ticks and tick labels are drawn. A
    /// family with a frame of its own replaces it
    /// ([`EnergyLevelTheme::axis_preset`], [`FeynmanTheme::axis_preset`]).
    pub axis_preset: AxisPreset,
    /// Whether the frame's ticks are drawn. `false` hides the ticks of
    /// every side the frame gives some and keeps their length, so the tick
    /// labels sit where they would beside drawn ticks.
    pub ticks: bool,
    /// The chart title and the axis titles.
    pub ink: Color,
    /// Axis lines and ticks.
    pub frame_ink: Color,
    /// Tick labels and the legend text.
    pub label_ink: Color,
    /// The legend text, where it differs from the tick labels; `None` is
    /// [`Self::label_ink`].
    pub legend_ink: Option<Color>,
    pub axis_width: f32,
    pub major_tick_length: f32,
    pub minor_tick_length: f32,
    /// The font family of every chrome text; empty is the default face.
    pub font: String,
    pub title_size: f32,
    pub axis_title_size: f32,
    pub tick_label_size: f32,
    pub legend_size: f32,
    pub grid: GridTheme,
    pub legend_fill: Color,
    pub legend_border: Color,
    /// The space between the legend box and its text, logical pixels.
    pub legend_padding: f32,
    pub colorbar: ColorbarTheme,
    pub highlight: HighlightTheme,
}

/// The colourbar's look; its map is the field part's
/// ([`FieldTheme::colormap`]). Its side, alignment, range and offsets stay.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ColorbarTheme {
    pub border: Color,
    pub border_width: f32,
    /// The bar's thickness, logical pixels.
    pub thickness: f32,
    /// The bar's length as a share of the plot's side.
    pub length: f32,
    /// The gap between the plot and the bar, logical pixels.
    pub gap: f32,
    pub ticks: TickVisibility,
    /// The size of the bar's tick labels, in the chrome's label ink.
    pub label_size: f32,
    /// The size of the bar's title, in the chrome's ink.
    pub title_size: f32,
}

/// One kind of grid line: whether it is drawn and how.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct GridLine {
    pub show: bool,
    pub color: Color,
    pub width: f32,
    pub style: LineStylePreset,
}

/// The four kinds of grid line.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct GridTheme {
    pub major_x: GridLine,
    pub major_y: GridLine,
    pub minor_x: GridLine,
    pub minor_y: GridLine,
}

/// The selection highlight's look ([`DataSelectionsConfig`]); the selected
/// refs and their visibility stay.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct HighlightTheme {
    pub color: Color,
    pub outline_width_px: f32,
    pub point_radius_extra_px: f32,
    pub contour_width_extra_px: f32,
}

/// One part per series family.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct SeriesTheme {
    /// Series `i` takes `palette[i % palette.len()]`, on every part a
    /// palette colours ([`ColorCycle::apply_to_series`] names them). Empty
    /// keeps every series colour.
    pub palette: Vec<Color>,
    pub lines: LineTheme,
    pub bars: BarTheme,
    pub boxes: BoxTheme,
    pub fields: FieldTheme,
    pub vectors: VectorTheme,
    pub radar: RadarTheme,
    pub range_bars: RangeBarTheme,
    pub ternary: TernaryTheme,
    pub parallel: ParallelTheme,
    pub surfaces: SurfaceTheme,
    pub energy_levels: EnergyLevelTheme,
    pub feynman: FeynmanTheme,
}

/// Lines, markers, stems and error bars of x-y and ternary series, and the
/// polylines of streamline and parallel series.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct LineTheme {
    pub line_width: f32,
    /// The opacity of a filled line's fill, in the line's colour; `None`
    /// keeps each fill's own colour.
    pub fill_opacity: Option<f32>,
    pub marker_size: f32,
    /// The opacity of every marker, in its colour; `None` keeps each
    /// marker's own alpha. A marker coloured through the colourbar takes
    /// its alpha from the map.
    pub marker_opacity: Option<f32>,
    pub stem_width: f32,
    /// The opacity of every stem, in its colour; `None` keeps each stem's
    /// own alpha.
    pub stem_opacity: Option<f32>,
    pub error_bar_width: f32,
    pub error_cap_width: f32,
    /// The length of an error bar's cap, logical pixels.
    pub error_cap_size: f32,
}

/// Histogram bars, category bars and the bars of range series.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct BarTheme {
    /// A histogram bar's border.
    pub border_color: Color,
    pub border_width: f32,
    /// The space between neighbouring histogram bars, logical pixels.
    pub gap: f32,
    pub material: Material,
    /// A category or range bar's corner radius, logical pixels; only the
    /// exposed end of a stack is rounded.
    pub corner_radius: f32,
    /// The strength of a category or range bar's material texture, `0..=1`.
    pub texture_strength: f32,
    pub gloss: f32,
    pub outline: bool,
    pub outline_width: f32,
    pub outline_color: Color,
    /// Whether the bars of a chart with one category-bar series take one
    /// palette colour per category ([`Theme::apply`] writes them as the
    /// series' per-bar colours), in reading order: left to right, and top
    /// to bottom for horizontal bars. `false` keeps the per-bar colours.
    pub color_by_category: bool,
}

/// Box plots, violins and the raw points beside them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct BoxTheme {
    pub material: Material,
    pub gloss: f32,
    /// The outlines, medians, whiskers and raw points.
    pub ink: Color,
    /// Whether a box is outlined.
    pub outline: bool,
    pub outline_width: f32,
    pub median_width: f32,
    pub whisker_width: f32,
    /// Whether a whisker ends in a cap.
    pub caps: bool,
    /// Whether a violin half is outlined, in [`Self::ink`] at
    /// [`Self::outline_width`]. A half with a fill of its own takes the
    /// series' palette colour, and the high half of a violin with two
    /// such halves the colour after it.
    pub violin_outline: bool,
    pub mean_color: Color,
    pub outlier_color: Color,
    pub point_size: f32,
    /// The opacity of the raw points, in [`Self::ink`]; `None` keeps each
    /// series' own alpha.
    pub point_opacity: Option<f32>,
    /// Whether the boxes of a chart with one box plot series take one
    /// palette colour per category ([`Theme::apply`] writes them as the
    /// series' per-box colours), in reading order: left to right, and top
    /// to bottom for horizontal boxes. A violin half without a fill of its
    /// own follows its box. `false` keeps the per-box colours.
    pub color_by_category: bool,
}

/// A text's colour and size.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct TextLook {
    pub color: Color,
    pub size: f32,
}

/// Heatmaps, contours, hexbins and meshes: the colourbar's map, the colour
/// of cells it cannot place, the fill's opacity, and the contour lines and
/// labels. Which levels are drawn, whether they are labelled and how far
/// apart the labels sit stay.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FieldTheme {
    pub colormap: ColorMap,
    pub nan_color: Color,
    /// The opacity of a field or mesh fill; `None` keeps each fill's own.
    pub fill_opacity: Option<f32>,
    /// The contour lines' colour; `None` is the series' palette colour.
    pub contour_color: Option<Color>,
    pub contour_width: f32,
    /// Each contour level in the colour map's colour at that level, its
    /// red, green and blue multiplied by this factor (`1.0` is the map's
    /// own colour, less is darker) so that lines over a pale stretch of the
    /// map stay visible. The level's place on the map is the chart's
    /// colourbar's, so only [`Theme::apply`] writes it, to a chart with a
    /// colourbar; `None` keeps each series' own per-level colours.
    pub contour_shade: Option<f32>,
    pub contour_label_color: Color,
    pub contour_label_size: f32,
    /// The space around a contour label, logical pixels: the gap it leaves
    /// in its line.
    pub contour_label_padding: f32,
}

/// Vector arrows.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct VectorTheme {
    pub width_px: f32,
    pub head_length_px: f32,
    pub head_width_px: f32,
}

/// Radar outlines; `None` keeps each series' own value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct RadarTheme {
    pub line_width_px: Option<f32>,
    pub marker_size_px: Option<f32>,
    pub fill_opacity: Option<f32>,
}

/// Range bars: the bars take [`BarTheme`]'s material and corners; these
/// are the colours that carry a bar's meaning, the whiskers, the connector
/// and the unfinished part of a bar with progress. The theme never chooses
/// how a series is coloured ([`crate::data_config::RangeColor`]): it
/// recolours the colouring the series has.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct RangeBarTheme {
    /// The rising bars of a series coloured by direction; `None` keeps the
    /// series' own.
    pub up: Option<Color>,
    /// The falling bars of a series coloured by direction; `None` keeps the
    /// series' own.
    pub down: Option<Color>,
    /// The total bars of a series coloured by direction with a total flag;
    /// `None` keeps the series' own.
    pub total: Option<Color>,
    /// The state colours of a series coloured by state: state `k` takes
    /// `states[k % states.len()]`. Empty keeps every state colour.
    pub states: Vec<Color>,
    pub whisker_width: f32,
    pub connector_color: Color,
    pub connector_width: f32,
    /// The opacity of a bar's unfinished part; `None` keeps the series'
    /// own.
    pub remaining_opacity: Option<f32>,
}

/// Ternary region fills and their outlines.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct TernaryTheme {
    /// The fill colours of the series that fill a region: series `i` takes
    /// `regions[i % regions.len()]`. Empty keeps every region colour.
    pub regions: Vec<Color>,
    pub region_opacity: f32,
    /// The outline of a filled region, the series' line: its colour and
    /// width. `None` leaves the line to the palette and the line part, as
    /// any other line.
    pub region_outline: Option<(Color, f32)>,
}

/// Parallel coordinates: the axes take the chrome's frame and tick labels,
/// the rows the line part.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ParallelTheme {
    /// The size of the axis names, the labels of the category axis.
    pub name_size: f32,
}

/// 3D surfaces and their lighting response.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct SurfaceTheme {
    pub material: Material,
    pub gloss: f32,
    /// Whether a surface draws its wireframe. A [`ChartKind::Surface3d`]
    /// theme decides it for every filled field and mesh, which that chart
    /// draws as surfaces; a theme of another kind leaves it as each series
    /// has it.
    pub wireframe: bool,
    /// A wireframe's colour and width, when the surface has one.
    pub wireframe_color: Color,
    pub wireframe_width: f32,
}

/// Energy-level (Grotrian) diagrams. The levels take the cycle colour.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct EnergyLevelTheme {
    /// The frame a chart of these series takes.
    pub axis_preset: AxisPreset,
    pub level_width: f32,
    pub limit_color: Color,
    pub limit_width: f32,
    pub transition_color: Color,
    pub transition_width: f32,
    pub band_fill: Color,
    /// The level labels; `None`, here and below, keeps each text's own
    /// colour and size.
    pub level_labels: Option<TextLook>,
    /// The transitions' labels: the host's, and the ones the renderer
    /// writes from the wavelengths.
    pub transition_labels: Option<TextLook>,
    /// The ionization limits' labels.
    pub limit_labels: Option<TextLook>,
    /// The level bands' labels.
    pub band_labels: Option<TextLook>,
    /// The headers over groups of term columns: the category groups of an
    /// [`ChartKind::EnergyLevels`] chart's column axis.
    pub group_headers: Option<TextLook>,
    /// The term columns' rich labels on that axis. A plain label is a tick
    /// label and takes the chrome's.
    pub column_labels: Option<TextLook>,
    /// The fill of the continuum above each ionization limit: the spans
    /// `EnergyLevelSeries::write_continuum` wrote into the chart's
    /// annotation layer, which [`Theme::apply`] recolours. `None` keeps
    /// their fill.
    pub continuum_fill: Option<Color>,
}

/// Feynman diagrams. The lines take the cycle colour unless their particle
/// has a colour of its own here.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FeynmanTheme {
    /// The frame a chart of these series takes.
    pub axis_preset: AxisPreset,
    pub width: f32,
    /// A photon's and a boson's wave.
    pub wave: Wiggle,
    /// A gluon's coil.
    pub coil: Wiggle,
    /// The arrowheads' length and width, logical pixels.
    pub head_length: f32,
    pub head_width: f32,
    pub dot_radius: f32,
    /// The colour of photon and graviton lines; `None`, here and below,
    /// keeps each line's own colour.
    pub photon: Option<Color>,
    pub gluon: Option<Color>,
    /// The colour of charged boson lines (a W).
    pub boson: Option<Color>,
    /// The colour of scalar and charged scalar lines.
    pub scalar: Option<Color>,
    /// The vertices' labels; `None` keeps each label's own colour and
    /// size.
    pub vertex_labels: Option<TextLook>,
    /// The propagators' labels and their momentum arrows' labels; `None`
    /// keeps each label's own colour and size.
    pub line_labels: Option<TextLook>,
}

/// A radial (pie, donut, sunburst) chart's look (#291). The radial chart is
/// a model of its own, so its themes are too: a family's radial member
/// ([`ThemeFamily::radial`], applied by name with
/// [`crate::radial::RadialChart::apply_theme`]) or a custom value, which
/// [`RadialTheme::apply`] writes. Its top-level slices take
/// the palette; values, labels, the slice structure, the kind (pie or
/// donut, and the hole), the light, the hover response and the font family
/// stay, and so does a slice's own style.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct RadialTheme {
    /// Slice `i` of the top level takes `palette[i % palette.len()]`;
    /// empty keeps every slice colour.
    pub palette: Vec<Color>,
    /// The colours below the top level: child `i` of a slice (a sunburst's
    /// nested rings, the detail chart of a split slice) takes its parent's
    /// colour moved towards the background, keeping `(1 - child_tint)`
    /// to the power `i + 1` of it, so each ring is paler than the one
    /// inside it and siblings step apart. `None` keeps every child colour.
    pub child_tint: Option<f32>,
    pub background: Color,
    /// The labels and the chart title.
    pub label_color: Color,
    pub font_size: f32,
    /// The chart title's size.
    pub title_size: f32,
    pub material: Material,
    pub gloss: f32,
    pub roughness: f32,
    pub texture_strength: f32,
    pub texture_scale: f32,
    pub shadow: bool,
    /// The camera's tilt, degrees; zero is a flat top view.
    pub tilt_degrees: f32,
    /// The extrusion height as a share of the radius.
    pub depth: f32,
    pub bevel: f32,
    /// The gap between slices, degrees.
    pub gap_degrees: f32,
    pub inner_corner: f32,
    pub outer_corner: f32,
    /// The rim and separator lines, their width and colour.
    pub outline: RadialOutline,
}

/// The defaults of a theme and its parts are the documented default look
/// of a line chart (the look a default document draws with): the values a theme object a host
/// saved before a field was added reads for that field (the `serde`
/// feature), as the document format's defaults do.
impl Default for Theme {
    fn default() -> Self {
        Self::documented(ChartKind::Line)
    }
}

macro_rules! part_defaults {
    ($($part:ty => |$theme:ident| $value:expr;)*) => {$(
        impl Default for $part {
            fn default() -> Self {
                let $theme = Theme::default();
                $value
            }
        }
    )*};
}

part_defaults! {
    ChromeTheme => |theme| theme.chrome;
    ColorbarTheme => |theme| theme.chrome.colorbar;
    GridTheme => |theme| theme.chrome.grid;
    GridLine => |theme| theme.chrome.grid.major_x;
    HighlightTheme => |theme| theme.chrome.highlight;
    SeriesTheme => |theme| theme.series;
    LineTheme => |theme| theme.series.lines;
    BarTheme => |theme| theme.series.bars;
    BoxTheme => |theme| theme.series.boxes;
    FieldTheme => |theme| theme.series.fields;
    VectorTheme => |theme| theme.series.vectors;
    RadarTheme => |theme| theme.series.radar;
    RangeBarTheme => |theme| theme.series.range_bars;
    TernaryTheme => |theme| theme.series.ternary;
    ParallelTheme => |theme| theme.series.parallel;
    SurfaceTheme => |theme| theme.series.surfaces;
    EnergyLevelTheme => |theme| theme.series.energy_levels;
    FeynmanTheme => |theme| theme.series.feynman;
    TextLook => |theme| TextLook {
        color: theme.chrome.label_ink,
        size: theme.chrome.tick_label_size,
    };
}

/// The documented default radial look.
impl Default for RadialTheme {
    fn default() -> Self {
        Self::documented()
    }
}

/// Why a theme was not applied. Nothing is changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeError {
    /// A chart in scope is of another kind than a custom theme is made
    /// for ([`Theme::apply`]); a family fits every kind.
    WrongKind {
        /// The theme's [`Theme::kind`].
        theme: ChartKind,
        chart: ChartKind,
        /// The chart: [`LayerRef::Base`] or a [`LayerRef::Subplot`].
        layer: LayerRef,
    },
    /// The theme's draw style cannot draw a series in scope: the renderer
    /// would drop it (`renderer::draw_style_supports`). The model cannot
    /// tell; [`Theme::apply_named`] takes the answer as an argument, and
    /// the renderer's `Chart` methods and the web wrappers check it before
    /// applying.
    UnsupportedDrawStyle { series_id: String },
    /// The name is not a family's.
    UnknownName {
        name: String,
        /// The names that work: [`ThemeFamily::NAMES`].
        names: &'static [&'static str],
    },
    /// A subplot index past the chart's subplots, in a
    /// [`SubplotScope::Only`] or given for a subplot's theme.
    SubplotIndex(SubplotIndexError),
}

/// A subplot index past the chart's subplots (`RuleKind::SubplotIndex`):
/// refused by the subplot setters and getters and, inside
/// [`ThemeError::SubplotIndex`], by a theme scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubplotIndexError {
    /// The index asked for.
    pub index: usize,
    /// How many subplots the chart has.
    pub count: usize,
}

impl SubplotIndexError {
    /// `Ok(())` when `index` is one of `count` subplots, the error otherwise.
    pub fn check(index: usize, count: usize) -> Result<(), Self> {
        if index < count {
            Ok(())
        } else {
            Err(Self { index, count })
        }
    }
}

impl std::fmt::Display for SubplotIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { index, count } = *self;
        write!(f, "{}: no subplot {index}, ", crate::RuleKind::SubplotIndex)?;
        match count {
            0 => write!(f, "the chart has no subplots"),
            1 => write!(f, "the chart has one subplot (index 0)"),
            _ => write!(
                f,
                "the chart has {count} subplots (indices 0 to {})",
                count - 1
            ),
        }
    }
}

impl std::error::Error for SubplotIndexError {}

impl From<SubplotIndexError> for ThemeError {
    fn from(error: SubplotIndexError) -> Self {
        Self::SubplotIndex(error)
    }
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongKind {
                theme,
                chart,
                layer,
            } => write!(
                f,
                "a {theme} theme does not apply to the {chart} chart at {layer:?}: apply it \
                 to the charts of its kind (apply_custom_theme skips the others), leave that \
                 chart out of the subplot scope, or use a family ({})",
                ThemeFamily::NAMES.join(", ")
            ),
            Self::UnsupportedDrawStyle { series_id } => write!(
                f,
                "the theme's draw style cannot draw series {series_id:?}: use a theme in the \
                 Precise draw style, such as a family, or remove that series"
            ),
            Self::UnknownName { name, names } => write!(
                f,
                "no theme family {name:?}: use one of {}",
                names.join(", ")
            ),
            Self::SubplotIndex(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ThemeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SubplotIndex(error) => Some(error),
            Self::WrongKind { .. }
            | Self::UnsupportedDrawStyle { .. }
            | Self::UnknownName { .. } => None,
        }
    }
}

/// Which subplots take a theme. The chart's own layers and its overlays
/// always do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubplotScope<'a> {
    None,
    All,
    /// The subplots at these indices into [`Config::subplots`].
    Only(&'a [usize]),
}

impl SubplotScope<'_> {
    /// Whether the subplot at `index` is in scope.
    pub fn includes(self, index: usize) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Only(indices) => indices.contains(&index),
        }
    }
}

fn grid_line(show: bool, color: Color, width: f32, style: LineStylePreset) -> GridLine {
    GridLine {
        show,
        color,
        width,
        style,
    }
}

/// A series' markers: their size, and their opacity when the theme sets
/// one.
fn style_markers(scatter: &mut crate::data_config::DataScatterStyleConfig, lines: &LineTheme) {
    scatter.point_size = lines.marker_size;
    if let Some(opacity) = lines.marker_opacity {
        scatter.point_color.a = opacity;
    }
}

/// The charts one application of a theme writes: the base chart or not,
/// and the subplots in scope. [`Theme::apply`] writes the base chart and
/// the subplots its [`SubplotScope`] names; a theme family writes one chart
/// at a time, each with its own member ([`Theme::apply_named`]).
#[derive(Clone, Copy, Debug)]
struct Charts<'a> {
    base: bool,
    subplots: SubplotScope<'a>,
}

impl<'a> Charts<'a> {
    /// The base chart and the subplots `subplots` names.
    fn scope(subplots: SubplotScope<'a>) -> Self {
        Self {
            base: true,
            subplots,
        }
    }

    /// The one chart `layer` names: the base chart or a subplot.
    fn one(layer: &'a LayerRef) -> Self {
        match layer {
            LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => Self {
                base: false,
                subplots: SubplotScope::Only(std::slice::from_ref(index)),
            },
            LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => {
                Self::scope(SubplotScope::None)
            }
        }
    }

    /// Whether the chart a series in `layer` is drawn on is written.
    fn holds(self, layer: LayerRef) -> bool {
        match layer {
            LayerRef::Subplot(i) | LayerRef::SubplotOverlay(i, _) => self.subplots.includes(i),
            LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => self.base,
        }
    }
}

/// Every index `subplots` names is a subplot of `config`
/// ([`ThemeError::SubplotIndex`] for the first that is not).
fn check_scope(config: &Config, subplots: SubplotScope) -> Result<(), ThemeError> {
    let count = config.subplots.len();
    match subplots {
        SubplotScope::Only(indices) => match indices.iter().find(|&&index| index >= count) {
            Some(&index) => Err(SubplotIndexError { index, count }.into()),
            None => Ok(()),
        },
        SubplotScope::None | SubplotScope::All => Ok(()),
    }
}

/// The charts `charts` writes: the base chart, then the subplots in scope.
fn charts_in_scope<'a>(config: &Config, charts: Charts<'a>) -> impl Iterator<Item = LayerRef> + 'a {
    chart_layers(config.subplots.len(), charts)
}

/// The charts `charts` writes of a document with `subplots` subplots,
/// without borrowing the document.
fn chart_layers(subplots: usize, charts: Charts) -> impl Iterator<Item = LayerRef> {
    (charts.base.then_some(LayerRef::Base)).into_iter().chain(
        (0..subplots)
            .filter(move |&index| charts.subplots.includes(index))
            .map(LayerRef::Subplot),
    )
}

/// Whether series `id` is drawn on the chart `layer` names (the base chart
/// or a subplot) or on one of its overlays.
fn drawn_on(config: &Config, layer: LayerRef, id: &str) -> bool {
    match (layer, config.layer_of_series(id)) {
        (LayerRef::Subplot(i), LayerRef::Subplot(j) | LayerRef::SubplotOverlay(j, _)) => i == j,
        (LayerRef::Subplot(_), _) => false,
        (_, LayerRef::Subplot(_) | LayerRef::SubplotOverlay(..)) => false,
        _ => true,
    }
}

/// [`EnergyLevelTheme::continuum_fill`]: the continuum spans of the
/// energy-level series in scope, found by the ids
/// `EnergyLevelSeries::write_continuum` gives them, filled with `fill`.
fn fill_continuum(config: &mut Config, series: &[SeriesConfig], charts: Charts, fill: Color) {
    for item in series {
        let DataRenderType::EnergyLevels(levels) = &item.render_type else {
            continue;
        };
        let layer = config.layer_of_series(&item.series_id);
        if !charts.holds(layer) {
            continue;
        }
        let chart = match layer {
            LayerRef::Subplot(i) | LayerRef::SubplotOverlay(i, _) => &mut config.subplots[i].config,
            _ => &mut *config,
        };
        for annotation in &mut chart.annotations.items {
            if let AnnotationKind::Span { fill: old, .. } = &mut annotation.kind
                && levels.is_continuum_id(&item.series_id, &annotation.id)
            {
                *old = fill;
            }
        }
    }
}

impl Theme {
    /// The documented default look of `kind`.
    pub(crate) fn documented(kind: ChartKind) -> Self {
        let config = crate::defaults::config();
        let grid = &config.grid;
        let legend = &config.legend;
        let highlight = crate::defaults::data_selections_config();
        let boxes = crate::defaults::box_plot_style();
        let bars = crate::defaults::category_bar_style();
        let levels = crate::defaults::energy_level_style();
        let feynman = crate::defaults::feynman_style();
        let surface = crate::defaults::surface_style();
        let radar = crate::defaults::radar_series_style();
        let bar = crate::defaults::color_bar_options();
        let vector = crate::data_config::VectorStyle::default();
        let errors = crate::data_config::DataErrorBarStyleConfig::default();
        let contour_labels = crate::data_config::ContourLabelConfig::default();
        Self {
            kind,
            chrome: ChromeTheme {
                background: None,
                plot_background: None,
                draw_style: DrawStyle::Precise,
                axis_preset: AxisPreset::BoxedInward,
                ticks: true,
                ink: config.chart_title.text.color,
                frame_ink: config.bottom_x.line_color,
                label_ink: config.bottom_x.label_style.color,
                legend_ink: None,
                axis_width: config.bottom_x.line_width,
                major_tick_length: config.bottom_x.major_tick_length,
                minor_tick_length: config.bottom_x.minor_tick_length,
                font: String::new(),
                title_size: config.chart_title.text.font_size,
                axis_title_size: config.bottom_x.title_option.text.font_size,
                tick_label_size: config.bottom_x.label_style.font_size,
                legend_size: legend.content.font_size,
                grid: GridTheme {
                    major_x: grid_line(
                        grid.show_major_x,
                        grid.major_x_color,
                        grid.major_x_width,
                        grid.major_x_style,
                    ),
                    major_y: grid_line(
                        grid.show_major_y,
                        grid.major_y_color,
                        grid.major_y_width,
                        grid.major_y_style,
                    ),
                    minor_x: grid_line(
                        grid.show_minor_x,
                        grid.minor_x_color,
                        grid.minor_x_width,
                        grid.minor_x_style,
                    ),
                    minor_y: grid_line(
                        grid.show_minor_y,
                        grid.minor_y_color,
                        grid.minor_y_width,
                        grid.minor_y_style,
                    ),
                },
                legend_fill: legend.bg_color,
                legend_border: legend.border_color,
                legend_padding: legend.padding,
                colorbar: ColorbarTheme {
                    border: bar.border_color,
                    border_width: bar.border_width,
                    thickness: bar.thickness_px,
                    length: bar.length_frac,
                    gap: bar.gap_px,
                    ticks: bar.axis.tick.clone(),
                    label_size: bar.axis.label_style.font_size,
                    title_size: bar.axis.title_option.text.font_size,
                },
                highlight: HighlightTheme {
                    color: highlight.highlight_color,
                    outline_width_px: highlight.outline_width_px,
                    point_radius_extra_px: highlight.point_radius_extra_px,
                    contour_width_extra_px: highlight.contour_width_extra_px,
                },
            },
            series: SeriesTheme {
                palette: ColorCycle::Balanced.colors().to_vec(),
                lines: LineTheme {
                    line_width: 1.0,
                    fill_opacity: None,
                    marker_size: 6.0,
                    marker_opacity: None,
                    stem_width: 1.0,
                    stem_opacity: None,
                    error_bar_width: 1.0,
                    error_cap_width: 1.0,
                    error_cap_size: errors.error_bar_cap_size,
                },
                bars: BarTheme {
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    gap: crate::data_config::DataBarStyleConfig::default().gap_px,
                    material: bars.material,
                    corner_radius: bars.corner_radius,
                    texture_strength: bars.texture_strength,
                    gloss: bars.gloss,
                    outline: bars.outline,
                    outline_width: bars.outline_width,
                    outline_color: bars.outline_color,
                    color_by_category: false,
                },
                boxes: BoxTheme {
                    material: boxes.material,
                    gloss: boxes.gloss,
                    ink: boxes.outline_color,
                    outline: boxes.outline,
                    outline_width: boxes.outline_width,
                    median_width: boxes.median_width,
                    whisker_width: boxes.whisker_width,
                    caps: boxes.caps,
                    violin_outline: crate::defaults::violin_style().outline,
                    mean_color: boxes.mean_color,
                    outlier_color: boxes.outlier_color,
                    point_size: crate::defaults::box_points_style().size,
                    point_opacity: None,
                    color_by_category: false,
                },
                fields: FieldTheme {
                    colormap: crate::defaults::color_bar_options().colormap,
                    nan_color: crate::defaults::color_bar_options().nan_color,
                    fill_opacity: None,
                    contour_color: None,
                    contour_width: 1.0,
                    contour_shade: None,
                    contour_label_color: contour_labels.color,
                    contour_label_size: contour_labels.font_size,
                    contour_label_padding: contour_labels.bg_padding_px,
                },
                vectors: VectorTheme {
                    width_px: vector.width_px,
                    head_length_px: vector.head_length_px,
                    head_width_px: vector.head_width_px,
                },
                radar: RadarTheme {
                    line_width_px: radar.line_width_px,
                    marker_size_px: radar.marker_size_px,
                    fill_opacity: radar.fill_opacity,
                },
                range_bars: RangeBarTheme {
                    up: None,
                    down: None,
                    total: None,
                    states: Vec::new(),
                    whisker_width: 1.0,
                    connector_color: Color::BLACK,
                    connector_width: 1.0,
                    remaining_opacity: None,
                },
                ternary: TernaryTheme {
                    regions: Vec::new(),
                    region_opacity: crate::defaults::ternary_region_style().opacity,
                    region_outline: None,
                },
                parallel: ParallelTheme {
                    name_size: config.bottom_x.label_style.font_size,
                },
                surfaces: SurfaceTheme {
                    material: surface.material,
                    gloss: surface.gloss,
                    wireframe: false,
                    wireframe_color: Color::BLACK,
                    wireframe_width: 1.0,
                },
                energy_levels: EnergyLevelTheme {
                    axis_preset: AxisPreset::Grotrian,
                    level_width: levels.level.width,
                    limit_color: levels.limit.color,
                    limit_width: levels.limit.width,
                    transition_color: levels.transition.color,
                    transition_width: levels.transition.width,
                    band_fill: levels.band.fill,
                    level_labels: None,
                    transition_labels: None,
                    limit_labels: None,
                    band_labels: None,
                    group_headers: None,
                    column_labels: None,
                    continuum_fill: None,
                },
                feynman: FeynmanTheme {
                    axis_preset: AxisPreset::Hidden,
                    width: feynman.width,
                    wave: feynman.wave,
                    coil: feynman.coil,
                    head_length: feynman.head_length,
                    head_width: feynman.head_width,
                    dot_radius: feynman.dot_radius,
                    photon: None,
                    gluon: None,
                    boson: None,
                    scalar: None,
                    vertex_labels: None,
                    line_labels: None,
                },
            },
        }
    }

    /// Apply the theme to a chart and its series: the chrome of the chart,
    /// its overlays and the subplots `subplots` names, and every series
    /// drawn on one of those. The legend symbols follow the new series
    /// look. The frame is the chrome's [`AxisPreset`] unless a chart draws
    /// a family with a frame of its own. A theme with a
    /// [`FieldTheme::contour_shade`] then colours every contour level from
    /// its chart's colourbar.
    ///
    /// Refused, changing nothing, when the base chart or a subplot in
    /// scope is of another kind than [`Self::kind`] ([`ChartKind::of`]); a
    /// chart with no series yet takes any theme.
    pub fn apply(
        &self,
        config: &mut Config,
        series: &mut [SeriesConfig],
        subplots: SubplotScope,
    ) -> Result<(), ThemeError> {
        let charts = Charts::scope(subplots);
        self.check_kind(config, series, charts)?;
        self.style_charts(config, series, charts);
        self.finish_charts(config, series, charts);
        Ok(())
    }

    /// The first half of an application: the series drawn on `charts`,
    /// each as the series of its index in `series`, then their colours per
    /// category. Series are styled before the legends, which show their
    /// marks ([`Self::finish_charts`]).
    fn style_charts(&self, config: &Config, series: &mut [SeriesConfig], charts: Charts) {
        for (index, item) in series.iter_mut().enumerate() {
            if charts.holds(config.layer_of_series(&item.series_id)) {
                self.style_series(item, index);
            }
        }
        self.color_categories(config, series, charts);
    }

    /// The second half of an application: the chrome of `charts`, then the
    /// contour levels and continuum fills that read it.
    fn finish_charts(&self, config: &mut Config, series: &mut [SeriesConfig], charts: Charts) {
        self.write_chrome(config, series, charts);
        if let Some(shade) = self.series.fields.contour_shade {
            self.shade_contours(config, series, charts, shade);
        }
        if let Some(fill) = self.series.energy_levels.continuum_fill {
            fill_continuum(config, series, charts, fill);
        }
    }

    /// [`BoxTheme::color_by_category`] and [`BarTheme::color_by_category`]:
    /// on every chart in scope that draws exactly one box plot series, or
    /// exactly one category-bar series, that series' per-category colours
    /// from the palette, in reading order.
    fn color_categories(&self, config: &Config, series: &mut [SeriesConfig], charts: Charts) {
        let palette = &self.series.palette;
        if palette.is_empty() {
            return;
        }
        let boxes = |rt: &DataRenderType| matches!(rt, DataRenderType::BoxPlot { .. });
        let bars = |rt: &DataRenderType| matches!(rt, DataRenderType::CategoryBars { .. });
        let families: [(bool, &dyn Fn(&DataRenderType) -> bool); 2] = [
            (self.series.boxes.color_by_category, &boxes),
            (self.series.bars.color_by_category, &bars),
        ];
        for layer in charts_in_scope(config, charts) {
            let chart = match layer {
                LayerRef::Subplot(i) => &config.subplots[i].config,
                _ => config,
            };
            let horizontal =
                config.bar_groups_of(layer).direction == CategoryBarDirection::Horizontal;
            let axis = if horizontal {
                &chart.left_y
            } else {
                &chart.bottom_x
            };
            let Some(categories) = &axis.categories else {
                continue;
            };
            // The first category of a vertical axis is the bottom row.
            let reversed = horizontal != axis.inverted;
            for (enabled, family) in families {
                let mut drawn = series.iter_mut().filter(|item| {
                    family(&item.render_type) && drawn_on(config, layer, &item.series_id)
                });
                let (Some(item), None, true) = (drawn.next(), drawn.next(), enabled) else {
                    continue;
                };
                for (index, category) in categories.iter().enumerate() {
                    let place = if reversed {
                        categories.len() - 1 - index
                    } else {
                        index
                    };
                    let color = Some(palette[place % palette.len()]);
                    match &mut item.render_type {
                        DataRenderType::BoxPlot { overrides, .. } => {
                            let overrides = overrides.get_or_insert_with(Vec::new);
                            match overrides
                                .iter_mut()
                                .find(|o| o.target.category_id == category.id)
                            {
                                Some(o) => o.color = color,
                                None => overrides.push(BoxPlotOverride {
                                    color,
                                    ..BoxPlotOverride::new(BoxPlotTarget::new(
                                        category.id.as_str(),
                                        item.series_id.as_str(),
                                    ))
                                }),
                            }
                        }
                        DataRenderType::CategoryBars { overrides, .. } => {
                            let overrides = overrides.get_or_insert_with(Vec::new);
                            match overrides
                                .iter_mut()
                                .find(|o| o.target.category_id == category.id)
                            {
                                Some(o) => o.color = color,
                                None => overrides.push(CategoryBarOverride {
                                    color,
                                    ..CategoryBarOverride::new(CategoryBarTarget::new(
                                        category.id.as_str(),
                                        item.series_id.as_str(),
                                    ))
                                }),
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// [`FieldTheme::contour_shade`]: every contour level of the series in
    /// scope in its colourbar's colour at that level, shaded by `shade`.
    /// The colourbar already carries the theme's map.
    fn shade_contours(
        &self,
        config: &Config,
        series: &mut [SeriesConfig],
        charts: Charts,
        shade: f32,
    ) {
        for item in series {
            let layer = config.layer_of_series(&item.series_id);
            if !charts.holds(layer) {
                continue;
            }
            let chart = match layer {
                LayerRef::Subplot(i) | LayerRef::SubplotOverlay(i, _) => &config.subplots[i].config,
                _ => config,
            };
            let contour = match &mut item.render_type {
                DataRenderType::Field(FieldSeries { contour, .. }) => contour.as_mut(),
                DataRenderType::TriMesh(mesh) => mesh.contour.as_mut(),
                _ => None,
            };
            if let (Some(bar), Some(contour)) = (&chart.colorbar, contour) {
                contour.per_level_color = Some(
                    contour
                        .levels
                        .iter()
                        .map(|level| {
                            let color = bar.color_for_z(*level);
                            Color::new(color.r * shade, color.g * shade, color.b * shade, color.a)
                        })
                        .collect(),
                );
            }
        }
    }

    /// Apply the chrome alone, to a chart whose series are styled elsewhere
    /// ([`Self::style_series`]). The legend symbols follow `series`. Refused
    /// as [`Self::apply`] is.
    pub fn apply_chrome(
        &self,
        config: &mut Config,
        series: &[SeriesConfig],
        subplots: SubplotScope,
    ) -> Result<(), ThemeError> {
        let charts = Charts::scope(subplots);
        self.check_kind(config, series, charts)?;
        self.write_chrome(config, series, charts);
        Ok(())
    }

    /// Whether every chart in scope takes a theme of [`Self::kind`].
    fn check_kind(
        &self,
        config: &Config,
        series: &[SeriesConfig],
        charts: Charts,
    ) -> Result<(), ThemeError> {
        check_scope(config, charts.subplots)?;
        for layer in charts_in_scope(config, charts) {
            if let Some(chart) = ChartKind::of(config, series, layer)
                && chart != self.kind
            {
                return Err(ThemeError::WrongKind {
                    theme: self.kind,
                    chart,
                    layer,
                });
            }
        }
        Ok(())
    }

    /// The chrome of the charts `charts` names and their overlays.
    fn write_chrome(&self, config: &mut Config, series: &[SeriesConfig], charts: Charts) {
        if charts.base {
            let preset = self.frame_for(config, series, LayerRef::Base);
            self.apply_chart_chrome(config, preset, series);
        }
        for index in 0..config.subplots.len() {
            if !charts.subplots.includes(index) {
                continue;
            }
            let preset = self.frame_for(config, series, LayerRef::Subplot(index));
            let subplot = &mut config.subplots[index].config;
            self.apply_chart_chrome(subplot, preset, series);
        }
    }

    /// Style one series as the `index`-th of its chart: the palette colour,
    /// then its family's widths, sizes and materials. Ids, columns, labels
    /// and the series' structure stay, and so do a contour's per-level
    /// colours, which need the chart's colourbar ([`Self::apply`]).
    pub fn style_series(&self, series: &mut SeriesConfig, index: usize) {
        let palette = &self.series.palette;
        if !palette.is_empty() {
            crate::preset::recolor_series(series, palette[index % palette.len()]);
        }
        let theme = &self.series;
        let lines = &theme.lines;
        series.for_each_line(|line| {
            line.line_width = lines.line_width;
            if let (Some(opacity), Some(_)) = (lines.fill_opacity, &line.fill) {
                line.fill_color = Some(Color {
                    a: line.line_color.a * opacity,
                    ..line.line_color
                });
            }
        });
        match &mut series.render_type {
            DataRenderType::Xy(xy) => {
                if let Some(scatter) = &mut xy.scatter {
                    style_markers(scatter, lines);
                }
                if let Some(stem) = &mut xy.stem {
                    stem.width = lines.stem_width;
                    if let Some(opacity) = lines.stem_opacity {
                        stem.color.a = opacity;
                    }
                }
                if let Some(errors) = &mut xy.errors {
                    errors.style.error_bar_width = lines.error_bar_width;
                    errors.style.cap_width = lines.error_cap_width;
                    errors.style.error_bar_cap_size = lines.error_cap_size;
                }
            }
            DataRenderType::Ternary {
                scatter,
                line,
                region,
                ..
            } => {
                if let Some(scatter) = scatter {
                    style_markers(scatter, lines);
                }
                if let Some(region) = region {
                    let ternary = &theme.ternary;
                    if !ternary.regions.is_empty() {
                        region.color = ternary.regions[index % ternary.regions.len()];
                    }
                    region.opacity = ternary.region_opacity;
                    if let (Some((color, width)), Some(line)) = (ternary.region_outline, line) {
                        line.line_color = color;
                        line.line_width = width;
                    }
                }
            }
            DataRenderType::Histogram { bar } => {
                bar.border_color = theme.bars.border_color;
                bar.border_width = theme.bars.border_width;
                bar.gap_px = theme.bars.gap;
            }
            DataRenderType::CategoryBars { style, .. }
            | DataRenderType::RangeBars { style, .. } => {
                style.material = theme.bars.material;
                style.corner_radius = theme.bars.corner_radius;
                style.texture_strength = theme.bars.texture_strength;
                style.gloss = theme.bars.gloss;
                style.outline = theme.bars.outline;
                style.outline_width = theme.bars.outline_width;
                style.outline_color = theme.bars.outline_color;
            }
            DataRenderType::BoxPlot {
                style,
                violin,
                points,
                ..
            } => {
                let boxes = &theme.boxes;
                style.material = boxes.material;
                style.gloss = boxes.gloss;
                style.outline = boxes.outline;
                style.outline_width = boxes.outline_width;
                style.outline_color = boxes.ink;
                style.median_width = boxes.median_width;
                style.median_color = boxes.ink;
                style.whisker_width = boxes.whisker_width;
                style.whisker_color = boxes.ink;
                style.caps = boxes.caps;
                style.mean_color = boxes.mean_color;
                style.outlier_color = boxes.outlier_color;
                if let Some(Violin { body, .. }) = violin {
                    let half = |half: &mut ViolinHalf| {
                        half.style.outline = boxes.violin_outline;
                        half.style.outline_width = boxes.outline_width;
                        half.style.outline_color = boxes.ink;
                    };
                    // A half with a fill of its own is recoloured, as the
                    // series is; of two such halves, two conditions side
                    // by side, the high one takes the next colour.
                    let own = |half: &mut ViolinHalf, index: usize| {
                        if let (Some(fill), false) = (&mut half.style.fill, palette.is_empty()) {
                            *fill = palette[index % palette.len()];
                        }
                    };
                    match body {
                        ViolinBody::Symmetric(one) => {
                            half(one);
                            own(one, index);
                        }
                        ViolinBody::Split { low, high } => {
                            let next = index + usize::from(low.is_some());
                            if let Some(low) = low {
                                half(low);
                                own(low, index);
                            }
                            if let Some(high) = high {
                                half(high);
                                own(high, next);
                            }
                        }
                    }
                }
                if let Some(BoxPoints { style, .. }) = points {
                    style.color = boxes.ink;
                    if let Some(opacity) = boxes.point_opacity {
                        style.color.a = opacity;
                    }
                    style.size = boxes.point_size;
                }
            }
            DataRenderType::Field(FieldSeries {
                fill,
                contour,
                surface,
                ..
            }) => {
                self.style_field(fill.as_mut(), contour.as_mut(), surface);
            }
            DataRenderType::TriMesh(mesh) => {
                self.style_field(mesh.fill.as_mut(), mesh.contour.as_mut(), &mut mesh.surface);
            }
            DataRenderType::Vector { style, .. } => {
                style.width_px = theme.vectors.width_px;
                style.head_length_px = theme.vectors.head_length_px;
                style.head_width_px = theme.vectors.head_width_px;
            }
            DataRenderType::Radar { style, .. } => {
                let radar = &theme.radar;
                if radar.line_width_px.is_some() {
                    style.line_width_px = radar.line_width_px;
                }
                if radar.marker_size_px.is_some() {
                    style.marker_size_px = radar.marker_size_px;
                }
                if radar.fill_opacity.is_some() {
                    style.fill_opacity = radar.fill_opacity;
                }
            }
            DataRenderType::EnergyLevels(levels) => {
                let energy = &theme.energy_levels;
                let style = &mut levels.style;
                style.level.width = energy.level_width;
                style.limit.color = energy.limit_color;
                style.limit.width = energy.limit_width;
                style.transition.color = energy.transition_color;
                style.transition.width = energy.transition_width;
                style.band.fill = energy.band_fill;
                if let (Some(look), Some(labels)) =
                    (energy.transition_labels, &mut style.wavelength_labels)
                {
                    self.style_text(&mut labels.text, look.color, look.size);
                }
                let texts = (levels.levels.iter_mut())
                    .filter_map(|level| level.label.as_mut())
                    .map(|text| (text, energy.level_labels))
                    .chain(
                        (levels.transitions.iter_mut())
                            .filter_map(|line| line.label.as_mut())
                            .map(|text| (text, energy.transition_labels)),
                    )
                    .chain(
                        (levels.limits.iter_mut())
                            .filter_map(|limit| limit.label.as_mut())
                            .map(|text| (text, energy.limit_labels)),
                    )
                    .chain(
                        (levels.bands.iter_mut())
                            .filter_map(|band| band.label.as_mut())
                            .map(|text| (text, energy.band_labels)),
                    );
                for (text, look) in texts {
                    if let Some(look) = look {
                        self.style_text(text, look.color, look.size);
                    }
                }
            }
            DataRenderType::Feynman(diagram) => {
                let feynman = &theme.feynman;
                let style = &mut diagram.style;
                style.width = feynman.width;
                style.wave = feynman.wave;
                style.coil = feynman.coil;
                style.head_length = feynman.head_length;
                style.head_width = feynman.head_width;
                style.dot_radius = feynman.dot_radius;
                if let Some(look) = feynman.vertex_labels {
                    for label in (diagram.vertices.iter_mut()).filter_map(|v| v.label.as_mut()) {
                        self.style_text(label, look.color, look.size);
                    }
                }
                for line in &mut diagram.propagators {
                    let own = match line.particle {
                        ParticleLine::Photon | ParticleLine::Graviton => feynman.photon,
                        ParticleLine::Gluon => feynman.gluon,
                        ParticleLine::ChargedBoson => feynman.boson,
                        ParticleLine::Scalar | ParticleLine::ChargedScalar => feynman.scalar,
                        ParticleLine::Plain
                        | ParticleLine::Fermion
                        | ParticleLine::AntiFermion
                        | ParticleLine::Ghost
                        | ParticleLine::Majorana
                        | ParticleLine::DoubleLine => None,
                    };
                    if own.is_some() {
                        line.color = own;
                    }
                    if let Some(look) = feynman.line_labels {
                        let momentum = line.momentum.as_mut().map(|arrow| &mut arrow.label);
                        for label in line.label.as_mut().into_iter().chain(momentum) {
                            self.style_text(label, look.color, look.size);
                        }
                    }
                }
            }
            // The line width is set above; a streamline's arrowheads keep
            // their size, which follows its spacing.
            DataRenderType::Streamlines { .. }
            | DataRenderType::MonotonicLine(_)
            | DataRenderType::Parallel { .. }
            | DataRenderType::RgbImage { .. } => {}
        }
        if let DataRenderType::RangeBars {
            whiskers,
            color_by,
            progress,
            connector,
            ..
        } = &mut series.render_type
        {
            let range = &theme.range_bars;
            match color_by {
                RangeColor::Single => {}
                RangeColor::Direction { up, down, total } => {
                    if let Some(color) = range.up {
                        *up = color;
                    }
                    if let Some(color) = range.down {
                        *down = color;
                    }
                    if let (Some(color), Some(total)) = (range.total, total) {
                        total.color = color;
                    }
                }
                RangeColor::State { styles, .. } => {
                    if !range.states.is_empty() {
                        for (k, style) in styles.iter_mut().enumerate() {
                            style.color = range.states[k % range.states.len()];
                        }
                    }
                }
            }
            if let (Some(opacity), Some(progress)) = (range.remaining_opacity, progress) {
                progress.remaining_opacity = opacity;
            }
            if let Some(whiskers) = whiskers {
                whiskers.width = range.whisker_width;
            }
            if let Some(connector) = connector {
                connector.color = range.connector_color;
                connector.width = range.connector_width;
            }
        }
    }

    /// A field's or mesh's fill, contour lines and labels, and surface.
    fn style_field(
        &self,
        fill: Option<&mut crate::data_config::FieldFillConfig>,
        contour: Option<&mut crate::data_config::ContourConfig>,
        surface: &mut Option<crate::view3d::SurfaceStyle>,
    ) {
        let fields = &self.series.fields;
        let surfaces = &self.series.surfaces;
        // A 3D chart draws every filled field and mesh as a surface, so its
        // theme decides the wireframe; a series without a surface look
        // gets one unless the theme's is the documented default it draws
        // with anyway.
        if self.kind == ChartKind::Surface3d && fill.is_some() {
            let documented = crate::defaults::surface_style();
            let default_look = surfaces.material == documented.material
                && surfaces.gloss == documented.gloss
                && !surfaces.wireframe;
            if surface.is_some() || !default_look {
                let surface = surface.get_or_insert(documented);
                match (surfaces.wireframe, &surface.wireframe) {
                    (true, None) => surface.wireframe = Some(Default::default()),
                    (false, Some(_)) => surface.wireframe = None,
                    _ => {}
                }
            }
        }
        if let (Some(opacity), Some(fill)) = (fields.fill_opacity, fill) {
            fill.opacity = opacity;
        }
        if let Some(contour) = contour {
            if let Some(color) = fields.contour_color {
                contour.line.line_color = color;
            }
            contour.line.line_width = fields.contour_width;
            if let Some(labels) = &mut contour.labels {
                labels.color = fields.contour_label_color;
                labels.font_size = fields.contour_label_size;
                labels.bg_padding_px = fields.contour_label_padding;
            }
        }
        if let Some(surface) = surface {
            surface.material = surfaces.material;
            surface.gloss = surfaces.gloss;
            if let Some(wireframe) = &mut surface.wireframe {
                wireframe.color = surfaces.wireframe_color;
                wireframe.width = surfaces.wireframe_width;
            }
        }
    }

    /// The frame of the chart `layer` names: a family's own frame when the
    /// chart draws one, the chrome's otherwise.
    fn frame_for(&self, config: &Config, series: &[SeriesConfig], layer: LayerRef) -> AxisPreset {
        let on_layer = |item: &&SeriesConfig| drawn_on(config, layer, &item.series_id);
        for item in series.iter().filter(on_layer) {
            match item.render_type {
                DataRenderType::Feynman(_) => return self.series.feynman.axis_preset,
                DataRenderType::EnergyLevels(_) => return self.series.energy_levels.axis_preset,
                _ => {}
            }
        }
        self.chrome.axis_preset
    }

    /// The chrome of one chart: its own axes, titles, grid, legend,
    /// colourbar and highlight, and its overlays' axes and legends.
    fn apply_chart_chrome(&self, config: &mut Config, preset: AxisPreset, series: &[SeriesConfig]) {
        let chrome = &self.chrome;
        config.background = chrome.background;
        config.plot_background = chrome.plot_background;
        config.draw_style = chrome.draw_style;
        // The theme keeps the axis breaks, which the Grotrian frame would
        // size.
        let breaks = config.left_y.breaks.take();
        config.apply_axis_preset(preset);
        config.left_y.breaks = breaks;
        if !chrome.ticks {
            let overlays = (config.overlays.iter_mut())
                .flat_map(|overlay| [&mut overlay.x_axis, &mut overlay.y_axis]);
            for axis in [
                &mut config.top_x,
                &mut config.bottom_x,
                &mut config.left_y,
                &mut config.right_y,
            ]
            .into_iter()
            .chain(overlays)
            {
                axis.tick = TickVisibility::None;
            }
        }
        // A ternary chart's third component and a 3D chart's height are read
        // on `right_y`: a data axis, framed as the labelled sides are.
        if config.ternary().is_some() || config.view_3d().is_some() {
            let right = &mut config.right_y;
            right.line_visible = config.left_y.line_visible;
            right.tick = config.left_y.tick.clone();
            right.label_style.label_visible = config.left_y.label_style.label_visible;
        }
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            self.style_axis(axis);
        }
        // A parallel chart reads its values on the parallel axes, drawn in
        // `left_y`'s style; `bottom_x` only names them. It is framed as
        // the unlabelled sides are, so an open frame draws no line under
        // the names.
        if config.parallel().is_some() {
            let names = &mut config.bottom_x;
            names.line_visible = config.top_x.line_visible;
            names.tick = config.top_x.tick.clone();
            names.label_style.font_size = self.series.parallel.name_size;
        }
        // An energy-level chart names its term columns on `bottom_x`.
        if self.kind == ChartKind::EnergyLevels {
            let energy = &self.series.energy_levels;
            let columns = config.bottom_x.categories.iter_mut().flatten();
            if let Some(look) = energy.column_labels {
                for label in columns.filter_map(|column| column.rich_label.as_mut()) {
                    self.style_text(label, look.color, look.size);
                }
            }
            if let (Some(look), Some(groups)) =
                (energy.group_headers, &mut config.bottom_x.category_groups)
            {
                for group in &mut groups.groups {
                    self.style_text(&mut group.label, look.color, look.size);
                }
            }
        }
        self.style_text(&mut config.chart_title.text, chrome.ink, chrome.title_size);
        self.style_grid(&mut config.grid);
        let colormap = match &mut config.colorbar {
            Some(bar) => {
                bar.colormap = self.series.fields.colormap.clone();
                bar.nan_color = self.series.fields.nan_color;
                let look = &chrome.colorbar;
                bar.border_color = look.border;
                bar.border_width = look.border_width;
                bar.thickness_px = look.thickness;
                bar.length_frac = look.length;
                bar.gap_px = look.gap;
                self.style_axis_look(&mut bar.axis);
                bar.axis.tick = look.ticks.clone();
                bar.axis.label_style.font_size = look.label_size;
                bar.axis.title_option.text.font_size = look.title_size;
                Some(bar.colormap.clone())
            }
            None => None,
        };
        self.style_legend(&mut config.legend, series, colormap.as_ref());
        for overlay in &mut config.overlays {
            self.style_axis(&mut overlay.y_axis);
            self.style_axis(&mut overlay.x_axis);
            if let Some(legend) = &mut overlay.legend {
                self.style_legend(legend, series, colormap.as_ref());
            }
        }
        self.style_highlight(&mut config.picked_data);
    }

    /// An axis's look and its frame-independent parts.
    fn style_axis(&self, axis: &mut AxisOptions) {
        self.style_axis_look(axis);
        axis.major_tick_length = self.chrome.major_tick_length;
        axis.minor_tick_length = self.chrome.minor_tick_length;
    }

    /// Colours, widths and text of an axis; which parts are drawn stays.
    fn style_axis_look(&self, axis: &mut AxisOptions) {
        let chrome = &self.chrome;
        axis.line_color = chrome.frame_ink;
        axis.line_width = chrome.axis_width;
        axis.label_style.color = chrome.label_ink;
        axis.label_style.font_size = chrome.tick_label_size;
        axis.label_style.label_font.clone_from(&chrome.font);
        self.style_text(
            &mut axis.title_option.text,
            chrome.ink,
            chrome.axis_title_size,
        );
    }

    /// The colour, size and face of a text; its segments stay.
    fn style_text(&self, text: &mut RichText, color: Color, size: f32) {
        text.color = color;
        text.font_size = size;
        text.font.clone_from(&self.chrome.font);
    }

    fn style_grid(&self, grid: &mut GridOptions) {
        let GridTheme {
            major_x,
            major_y,
            minor_x,
            minor_y,
        } = &self.chrome.grid;
        (
            grid.show_major_x,
            grid.major_x_color,
            grid.major_x_width,
            grid.major_x_style,
        ) = (major_x.show, major_x.color, major_x.width, major_x.style);
        (
            grid.show_major_y,
            grid.major_y_color,
            grid.major_y_width,
            grid.major_y_style,
        ) = (major_y.show, major_y.color, major_y.width, major_y.style);
        (
            grid.show_minor_x,
            grid.minor_x_color,
            grid.minor_x_width,
            grid.minor_x_style,
        ) = (minor_x.show, minor_x.color, minor_x.width, minor_x.style);
        (
            grid.show_minor_y,
            grid.minor_y_color,
            grid.minor_y_width,
            grid.minor_y_style,
        ) = (minor_y.show, minor_y.color, minor_y.width, minor_y.style);
    }

    /// The legend box and text look, and each entry's symbol redrawn from
    /// its series; the entries' text, the corner and the offsets stay.
    fn style_legend(
        &self,
        legend: &mut Legend,
        series: &[SeriesConfig],
        colormap: Option<&ColorMap>,
    ) {
        let chrome = &self.chrome;
        legend.bg_color = chrome.legend_fill;
        legend.border_color = chrome.legend_border;
        legend.padding = chrome.legend_padding;
        let ink = chrome.legend_ink.unwrap_or(chrome.label_ink);
        self.style_text(&mut legend.content, ink, chrome.legend_size);
        crate::legend::update_legend_symbols_preserving_text(&mut legend.content, series, colormap);
    }

    /// The highlight look of the chart's selection. A chart without one
    /// gets an empty selection carrying the look, unless the look is the
    /// documented default a first selection takes anyway.
    fn style_highlight(&self, picked: &mut Option<DataSelectionsConfig>) {
        let look = &self.chrome.highlight;
        let documented = crate::defaults::data_selections_config();
        let default_look = HighlightTheme {
            color: documented.highlight_color,
            outline_width_px: documented.outline_width_px,
            point_radius_extra_px: documented.point_radius_extra_px,
            contour_width_extra_px: documented.contour_width_extra_px,
        };
        let selection = match picked {
            Some(selection) => selection,
            None if *look == default_look => return,
            None => picked.insert(documented),
        };
        selection.highlight_color = look.color;
        selection.outline_width_px = look.outline_width_px;
        selection.point_radius_extra_px = look.point_radius_extra_px;
        selection.contour_width_extra_px = look.contour_width_extra_px;
    }
}

impl RadialTheme {
    /// The documented default look.
    pub(crate) fn documented() -> Self {
        let radial = crate::defaults::radial_chart();
        let style = &radial.style;
        Self {
            palette: Vec::new(),
            child_tint: None,
            background: radial.background,
            label_color: radial.label_color,
            font_size: radial.font_size,
            // A plain radial title is half as large again as the labels.
            title_size: radial.font_size * 1.5,
            material: style.material,
            gloss: style.gloss,
            roughness: style.roughness,
            texture_strength: style.texture_strength,
            texture_scale: style.texture_scale,
            shadow: style.shadow,
            tilt_degrees: style.tilt_degrees,
            depth: style.depth,
            bevel: style.bevel,
            gap_degrees: style.gap_degrees,
            inner_corner: style.inner_corner,
            outer_corner: style.outer_corner,
            outline: style.outline,
        }
    }

    /// Apply the theme to a radial chart: its background, label look,
    /// material, view, slice shape, outline and title, the palette on its
    /// top-level slices and [`Self::child_tint`] on the slices below them.
    /// Values, labels, the slice structure, the kind and the font family
    /// stay. A sunburst stays flat: its rings share one centre only without
    /// a tilt or a depth (`RuleKind::SunburstTilt`), so it takes the rest of
    /// a tilted theme and keeps its top view.
    pub fn apply(&self, chart: &mut RadialChart) {
        let flat = chart.is_sunburst();
        chart.background = self.background;
        chart.label_color = self.label_color;
        chart.font_size = self.font_size;
        let style = &mut chart.style;
        style.material = self.material;
        style.gloss = self.gloss;
        style.roughness = self.roughness;
        style.texture_strength = self.texture_strength;
        style.texture_scale = self.texture_scale;
        style.shadow = self.shadow;
        style.tilt_degrees = if flat { 0.0 } else { self.tilt_degrees };
        style.depth = if flat { 0.0 } else { self.depth };
        style.bevel = self.bevel;
        style.gap_degrees = self.gap_degrees;
        style.inner_corner = self.inner_corner;
        style.outer_corner = self.outer_corner;
        style.outline = self.outline;
        if !self.palette.is_empty() {
            for (index, slice) in chart.slices.iter_mut().enumerate() {
                slice.color = self.palette[index % self.palette.len()];
            }
        }
        if let Some(tint) = self.child_tint {
            for slice in &mut chart.slices {
                self.tint_children(&mut slice.children, slice.color, tint);
            }
            if let Some(split) = &mut chart.split
                && let Some(parent) = chart.slices.get(split.slice_index)
            {
                self.tint_children(&mut split.children, parent.color, tint);
            }
        }
        if let Some(title) = &mut chart.chart_title {
            title.text.color = self.label_color;
            title.text.font_size = self.title_size;
        }
    }

    /// [`Self::child_tint`]: each of `children` in `parent`'s colour moved
    /// towards the background, and its own children after it.
    fn tint_children(&self, children: &mut [RadialSlice], parent: Color, tint: f32) {
        let mut keep = 1.0;
        for child in children {
            keep *= 1.0 - tint;
            let mix = |from: f32, to: f32| to + (from - to) * keep;
            child.color = Color::new(
                mix(parent.r, self.background.r),
                mix(parent.g, self.background.g),
                mix(parent.b, self.background.b),
                parent.a,
            );
            self.tint_children(&mut child.children, child.color, tint);
        }
    }
}

#[cfg(test)]
mod tests;
