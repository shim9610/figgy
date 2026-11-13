//! Themes (#265, #291): looks written into the document once, each made for
//! one kind of chart.
//!
//! A [`Theme`] is a value, not a name stored in the document: applying one
//! writes exactly the fields it owns and keeps everything the user set as
//! content or layout. It is made for one [`ChartKind`]: applying it to a
//! chart of another kind is refused ([`ThemeError::Kind`]) and changes
//! nothing. [`Theme::named`] looks one up in a kind's catalogue
//! ([`Theme::names`]). The radial chart has its own ([`RadialTheme`]). A
//! theme owns:
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
//! `Chart::apply_theme` refits the layout after applying; a host that applies
//! a theme to a bare [`Config`] calls `layout::auto_layout` itself.

use crate::color::Color;
use crate::colormap::ColorMap;
use crate::config::{
    AxisOptions, Config, DataSelectionsConfig, DrawStyle, GridOptions, TickVisibility,
};
use crate::data_config::{
    BoxPoints, DataRenderType, FieldSeries, RangeColor, SeriesConfig, Violin, ViolinBody,
    ViolinHalf,
};
use crate::layer::LayerRef;
use crate::legend::Legend;
use crate::line::LineStylePreset;
use crate::material::Material;
use crate::preset::{AxisPreset, ColorCycle};

mod catalog;
mod kind;
use crate::radial::RadialChart;
use crate::text::RichText;
pub use kind::ChartKind;

/// A chart look made for one kind of chart: the chrome and one part per
/// series family. Every built-in theme defines every part; a host edits any
/// field before applying.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// The kind of chart the theme is made for; [`Theme::apply`] refuses
    /// a chart of another kind.
    pub kind: ChartKind,
    pub chrome: ChromeTheme,
    pub series: SeriesTheme,
}

/// The look of everything around the data.
#[derive(Clone, Debug, PartialEq)]
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
pub struct GridLine {
    pub show: bool,
    pub color: Color,
    pub width: f32,
    pub style: LineStylePreset,
}

/// The four kinds of grid line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridTheme {
    pub major_x: GridLine,
    pub major_y: GridLine,
    pub minor_x: GridLine,
    pub minor_y: GridLine,
}

/// The selection highlight's look ([`DataSelectionsConfig`]); the selected
/// refs and their visibility stay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighlightTheme {
    pub color: Color,
    pub outline_width_px: f32,
    pub point_radius_extra_px: f32,
    pub contour_width_extra_px: f32,
}

/// One part per series family.
#[derive(Clone, Debug, PartialEq)]
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
}

/// Box plots, violins and the raw points beside them.
#[derive(Clone, Copy, Debug, PartialEq)]
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
}

/// Heatmaps, contours, hexbins and meshes: the colourbar's map, the colour
/// of cells it cannot place, the fill's opacity, and the contour lines and
/// labels. Which levels are drawn, whether they are labelled and how far
/// apart the labels sit stay.
#[derive(Clone, Debug, PartialEq)]
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
pub struct VectorTheme {
    pub width_px: f32,
    pub head_length_px: f32,
    pub head_width_px: f32,
}

/// Radar outlines; `None` keeps each series' own value.
#[derive(Clone, Copy, Debug, PartialEq)]
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
pub struct ParallelTheme {
    /// The size of the axis names, the labels of the category axis.
    pub name_size: f32,
}

/// 3D surfaces and their lighting response.
#[derive(Clone, Copy, Debug, PartialEq)]
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
pub struct EnergyLevelTheme {
    /// The frame a chart of these series takes.
    pub axis_preset: AxisPreset,
    pub level_width: f32,
    pub limit_color: Color,
    pub limit_width: f32,
    pub transition_color: Color,
    pub transition_width: f32,
    pub band_fill: Color,
}

/// Feynman diagrams. The lines take the cycle colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeynmanTheme {
    /// The frame a chart of these series takes.
    pub axis_preset: AxisPreset,
    pub width: f32,
}

/// A radial (pie, donut, sunburst) chart's look (#291). The radial chart is
/// a model of its own, so its themes are too: [`RadialTheme::named`] looks
/// one up and [`RadialTheme::apply`] writes it. Its top-level slices take
/// the palette; values, labels, the slice structure and the font family
/// stay.
#[derive(Clone, Debug, PartialEq)]
pub struct RadialTheme {
    /// Slice `i` of the top level takes `palette[i % palette.len()]`;
    /// empty keeps every slice colour.
    pub palette: Vec<Color>,
    pub background: Color,
    pub label_color: Color,
    pub font_size: f32,
    /// The chart title's size.
    pub title_size: f32,
    pub material: Material,
    pub gloss: f32,
    pub shadow: bool,
}

/// Why a theme was not applied. Nothing is changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeError {
    /// A chart in scope is of another kind than the theme is made for.
    Kind {
        theme: ChartKind,
        chart: ChartKind,
        /// The chart: [`LayerRef::Base`] or a [`LayerRef::Subplot`].
        layer: LayerRef,
    },
    /// The theme's draw style cannot draw a series in scope: the renderer
    /// would drop it (`renderer::draw_style_supports`). The model cannot
    /// tell; the renderer's `Chart::apply_theme` and the web wrappers
    /// check it before applying.
    DrawStyle { series_id: String },
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kind {
                theme,
                chart,
                layer,
            } => write!(
                f,
                "a {theme} theme does not apply to a {chart} chart ({layer:?})"
            ),
            Self::DrawStyle { series_id } => {
                write!(f, "the theme's draw style cannot draw series {series_id:?}")
            }
        }
    }
}

impl std::error::Error for ThemeError {}

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

impl Theme {
    /// The names in `kind`'s catalogue, `default` first.
    pub fn names(kind: ChartKind) -> &'static [&'static str] {
        catalog::names(kind)
    }

    /// The theme `name` of `kind`'s catalogue ([`Self::names`]), or `None`.
    /// Every kind has `default`, the documented default look: applying it
    /// to a default document changes nothing.
    pub fn named(kind: ChartKind, name: &str) -> Option<Self> {
        catalog::named(kind, name)
    }

    /// The documented default look of `kind`.
    pub(crate) fn documented(kind: ChartKind) -> Self {
        let config = crate::defaults::config();
        let grid = &config.grid;
        let legend = &config.legend;
        let highlight = crate::defaults::data_selections_config();
        let boxes = crate::defaults::box_plot_style();
        let bars = crate::defaults::category_bar_style();
        let levels = crate::defaults::energy_level_style();
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
                },
                feynman: FeynmanTheme {
                    axis_preset: AxisPreset::Hidden,
                    width: crate::defaults::feynman_style().width,
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
        self.check_kind(config, series, subplots)?;
        // Series are styled before the legends, which show their marks.
        for (index, item) in series.iter_mut().enumerate() {
            let themed = match config.layer_of_series(&item.series_id) {
                LayerRef::Subplot(i) | LayerRef::SubplotOverlay(i, _) => subplots.includes(i),
                _ => true,
            };
            if themed {
                self.style_series(item, index);
            }
        }
        self.write_chrome(config, series, subplots);
        if let Some(shade) = self.series.fields.contour_shade {
            self.shade_contours(config, series, subplots, shade);
        }
        Ok(())
    }

    /// [`FieldTheme::contour_shade`]: every contour level of the series in
    /// scope in its colourbar's colour at that level, shaded by `shade`.
    /// The colourbar already carries the theme's map.
    fn shade_contours(
        &self,
        config: &Config,
        series: &mut [SeriesConfig],
        subplots: SubplotScope,
        shade: f32,
    ) {
        for item in series {
            let chart = match config.layer_of_series(&item.series_id) {
                LayerRef::Subplot(i) | LayerRef::SubplotOverlay(i, _) => {
                    if !subplots.includes(i) {
                        continue;
                    }
                    &config.subplots[i].config
                }
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
        self.check_kind(config, series, subplots)?;
        self.write_chrome(config, series, subplots);
        Ok(())
    }

    /// Whether every chart in scope takes a theme of [`Self::kind`].
    fn check_kind(
        &self,
        config: &Config,
        series: &[SeriesConfig],
        subplots: SubplotScope,
    ) -> Result<(), ThemeError> {
        let charts = std::iter::once(LayerRef::Base).chain(
            (0..config.subplots.len())
                .filter(|&index| subplots.includes(index))
                .map(LayerRef::Subplot),
        );
        for layer in charts {
            if let Some(chart) = ChartKind::of(config, series, layer)
                && chart != self.kind
            {
                return Err(ThemeError::Kind {
                    theme: self.kind,
                    chart,
                    layer,
                });
            }
        }
        Ok(())
    }

    /// The chrome of the chart, its overlays and the subplots in scope.
    fn write_chrome(&self, config: &mut Config, series: &[SeriesConfig], subplots: SubplotScope) {
        let preset = self.frame_for(config, series, LayerRef::Base);
        self.apply_chart_chrome(config, preset, series);
        for index in 0..config.subplots.len() {
            if !subplots.includes(index) {
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
            }
            DataRenderType::Feynman(diagram) => {
                diagram.style.width = theme.feynman.width;
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
        let on_layer = |item: &&SeriesConfig| match (layer, config.layer_of_series(&item.series_id))
        {
            (LayerRef::Subplot(i), LayerRef::Subplot(j) | LayerRef::SubplotOverlay(j, _)) => i == j,
            (LayerRef::Subplot(_), _) => false,
            (_, LayerRef::Subplot(_) | LayerRef::SubplotOverlay(..)) => false,
            _ => true,
        };
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
        config.apply_axis_preset(preset);
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
    /// The names in the radial catalogue, `default` first.
    pub fn names() -> &'static [&'static str] {
        catalog::radial_names()
    }

    /// The radial theme `name` ([`Self::names`]), or `None`. `default` is
    /// the documented default look: applying it to a default radial chart
    /// changes nothing.
    pub fn named(name: &str) -> Option<Self> {
        catalog::radial_named(name)
    }

    /// The documented default look.
    pub(crate) fn documented() -> Self {
        let radial = crate::defaults::radial_chart();
        Self {
            palette: Vec::new(),
            background: radial.background,
            label_color: radial.label_color,
            font_size: radial.font_size,
            title_size: crate::defaults::config().chart_title.text.font_size,
            material: radial.style.material,
            gloss: radial.style.gloss,
            shadow: radial.style.shadow,
        }
    }

    /// Apply the theme to a radial chart: its background, label look,
    /// material and title, and the palette on its top-level slices.
    /// Values, labels, the slice structure and the font family stay.
    pub fn apply(&self, chart: &mut RadialChart) {
        chart.background = self.background;
        chart.label_color = self.label_color;
        chart.font_size = self.font_size;
        chart.style.material = self.material;
        chart.style.gloss = self.gloss;
        chart.style.shadow = self.shadow;
        if !self.palette.is_empty() {
            for (index, slice) in chart.slices.iter_mut().enumerate() {
                slice.color = self.palette[index % self.palette.len()];
            }
        }
        if let Some(title) = &mut chart.chart_title {
            title.text.color = self.label_color;
            title.text.font_size = self.title_size;
        }
    }
}

#[cfg(test)]
mod tests;
