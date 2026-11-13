//! Themes (#265): presets that write a chart's look into the document once.
//!
//! A [`Theme`] is a value, not a name stored in the document: applying one
//! writes exactly the fields it owns and keeps everything the user set as
//! content or layout. It owns:
//! - the chrome ([`ChromeTheme`]): the chart and plot fills, the draw style,
//!   the axis frame, the text look, the grid, the legend box and the
//!   selection highlight;
//! - one part per series family ([`SeriesTheme`]): colours through a
//!   [`ColorCycle`], stroke widths, marker sizes and each family's materials.
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
use crate::config::{AxisOptions, Config, DataSelectionsConfig, DrawStyle, GridOptions};
use crate::data_config::{
    BoxPoints, DataRenderType, FieldSeries, SeriesConfig, Violin, ViolinBody, ViolinHalf,
};
use crate::layer::LayerRef;
use crate::legend::Legend;
use crate::line::LineStylePreset;
use crate::material::Material;
use crate::preset::{AxisPreset, ColorCycle};
use crate::radial::RadialChart;
use crate::text::RichText;

/// A chart look: the chrome and one part per series family. Every built-in
/// theme defines every part; a host edits any field before applying.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
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
    /// The colourbar's outline.
    pub colorbar_border: Color,
    pub highlight: HighlightTheme,
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
    /// Series `i` takes `cycle.color(i)` ([`ColorCycle::apply_to_series`]).
    pub cycle: ColorCycle,
    pub lines: LineTheme,
    pub bars: BarTheme,
    pub boxes: BoxTheme,
    pub fields: FieldTheme,
    pub vectors: VectorTheme,
    pub radar: RadarTheme,
    pub range_bars: RangeBarTheme,
    pub ternary: TernaryTheme,
    pub surfaces: SurfaceTheme,
    pub energy_levels: EnergyLevelTheme,
    pub feynman: FeynmanTheme,
    pub radial: RadialTheme,
}

/// Lines, markers, stems and error bars of x-y and ternary series, and the
/// polylines of streamline and parallel series.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineTheme {
    pub line_width: f32,
    pub marker_size: f32,
    pub stem_width: f32,
    pub error_bar_width: f32,
    pub error_cap_width: f32,
}

/// Histogram bars and category bars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarTheme {
    /// A histogram bar's border.
    pub border_color: Color,
    pub border_width: f32,
    pub material: Material,
    pub gloss: f32,
    pub outline: bool,
    pub outline_width: f32,
    pub outline_color: Color,
}

/// Box plots, violins and the raw points beside them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxTheme {
    pub material: Material,
    pub ink: Color,
    pub outline_width: f32,
    pub median_width: f32,
    pub whisker_width: f32,
    pub mean_color: Color,
    pub outlier_color: Color,
    pub point_size: f32,
}

/// Heatmaps, contours, hexbins and meshes: the colourbar's map, the colour
/// of cells it cannot place, and the contour lines and labels.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldTheme {
    pub colormap: ColorMap,
    pub nan_color: Color,
    pub contour_width: f32,
    pub contour_label_color: Color,
}

/// Vector arrows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VectorTheme {
    pub width_px: f32,
    pub head_length_px: f32,
    pub head_width_px: f32,
}

/// Radar outlines; `None` keeps the chart's own value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadarTheme {
    pub line_width_px: Option<f32>,
    pub marker_size_px: Option<f32>,
    pub fill_opacity: Option<f32>,
}

/// Range bars: the bars take [`BarTheme`]'s material; these are the
/// whiskers and the connector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeBarTheme {
    pub whisker_width: f32,
    pub connector_color: Color,
    pub connector_width: f32,
}

/// Ternary region fills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TernaryTheme {
    pub region_opacity: f32,
}

/// 3D surfaces and their lighting response.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceTheme {
    pub material: Material,
    pub gloss: f32,
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

/// The radial (pie, donut, sunburst) chart. Its top-level slices take the
/// cycle colours; the chart's own font family stays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadialTheme {
    pub background: Color,
    pub label_color: Color,
    pub font_size: f32,
    pub material: Material,
    pub gloss: f32,
    pub shadow: bool,
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
    fn includes(self, index: usize) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Only(indices) => indices.contains(&index),
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
}

fn grid_line(show: bool, color: Color, width: f32, style: LineStylePreset) -> GridLine {
    GridLine {
        show,
        color,
        width,
        style,
    }
}

impl Theme {
    /// The documented default look: applying it to a default document
    /// changes nothing.
    pub fn light() -> Self {
        let config = crate::defaults::config();
        let grid = &config.grid;
        let legend = &config.legend;
        let highlight = crate::defaults::data_selections_config();
        let boxes = crate::defaults::box_plot_style();
        let bars = crate::defaults::category_bar_style();
        let levels = crate::defaults::energy_level_style();
        let surface = crate::defaults::surface_style();
        let radial = crate::defaults::radial_chart();
        let radar = crate::defaults::radar_series_style();
        let vector = crate::data_config::VectorStyle::default();
        Self {
            chrome: ChromeTheme {
                background: None,
                plot_background: None,
                draw_style: DrawStyle::Precise,
                axis_preset: AxisPreset::BoxedInward,
                ink: config.chart_title.text.color,
                frame_ink: config.bottom_x.line_color,
                label_ink: config.bottom_x.label_style.color,
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
                colorbar_border: crate::defaults::color_bar_options().border_color,
                highlight: HighlightTheme {
                    color: highlight.highlight_color,
                    outline_width_px: highlight.outline_width_px,
                    point_radius_extra_px: highlight.point_radius_extra_px,
                    contour_width_extra_px: highlight.contour_width_extra_px,
                },
            },
            series: SeriesTheme {
                cycle: ColorCycle::Balanced,
                lines: LineTheme {
                    line_width: 1.0,
                    marker_size: 6.0,
                    stem_width: 1.0,
                    error_bar_width: 1.0,
                    error_cap_width: 1.0,
                },
                bars: BarTheme {
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    material: bars.material,
                    gloss: bars.gloss,
                    outline: bars.outline,
                    outline_width: bars.outline_width,
                    outline_color: bars.outline_color,
                },
                boxes: BoxTheme {
                    material: boxes.material,
                    ink: boxes.outline_color,
                    outline_width: boxes.outline_width,
                    median_width: boxes.median_width,
                    whisker_width: boxes.whisker_width,
                    mean_color: boxes.mean_color,
                    outlier_color: boxes.outlier_color,
                    point_size: crate::defaults::box_points_style().size,
                },
                fields: FieldTheme {
                    colormap: crate::defaults::color_bar_options().colormap,
                    nan_color: crate::defaults::color_bar_options().nan_color,
                    contour_width: 1.0,
                    contour_label_color: crate::defaults::contour_label_color(),
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
                    whisker_width: 1.0,
                    connector_color: Color::BLACK,
                    connector_width: 1.0,
                },
                ternary: TernaryTheme {
                    region_opacity: crate::defaults::ternary_region_style().opacity,
                },
                surfaces: SurfaceTheme {
                    material: surface.material,
                    gloss: surface.gloss,
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
                radial: RadialTheme {
                    background: radial.background,
                    label_color: radial.label_color,
                    font_size: radial.font_size,
                    material: radial.style.material,
                    gloss: radial.style.gloss,
                    shadow: radial.style.shadow,
                },
            },
        }
    }

    /// Light ink on a dark chart area, for screens.
    pub fn dark() -> Self {
        let mut theme = Self::light();
        let bg = rgb(28, 30, 36);
        let ink = rgb(225, 228, 235);
        let soft = rgb(190, 195, 205);
        let chrome = &mut theme.chrome;
        chrome.background = Some(bg);
        chrome.ink = ink;
        chrome.frame_ink = soft;
        chrome.label_ink = soft;
        chrome.grid.major_x.color = rgb(66, 70, 80);
        chrome.grid.major_y.color = rgb(66, 70, 80);
        chrome.grid.minor_x.color = rgb(48, 51, 59);
        chrome.grid.minor_y.color = rgb(48, 51, 59);
        chrome.legend_fill = rgba(28, 30, 36, 0.85);
        chrome.legend_border = rgb(90, 95, 106);
        chrome.colorbar_border = rgb(140, 145, 155);
        chrome.highlight.color = rgb(255, 214, 10);
        let series = &mut theme.series;
        series.cycle = ColorCycle::ColorblindSafe;
        series.lines.line_width = 1.5;
        series.bars.border_color = bg;
        series.bars.outline_color = ink;
        series.boxes.ink = ink;
        series.boxes.mean_color = ink;
        series.boxes.outlier_color = soft;
        series.fields.colormap = ColorMap::Magma;
        series.fields.contour_label_color = ink;
        series.range_bars.connector_color = soft;
        series.surfaces.wireframe_color = ink;
        series.energy_levels.limit_color = rgb(140, 160, 255);
        series.energy_levels.transition_color = rgb(255, 170, 60);
        series.energy_levels.band_fill = rgba(140, 160, 255, 0.18);
        series.radial.background = bg;
        series.radial.label_color = ink;
        theme
    }

    /// Publication figures: an open frame with outward ticks, no grid, a
    /// white chart area, grayscale-safe ink and a perceptually uniform map.
    pub fn paper() -> Self {
        let mut theme = Self::light();
        let chrome = &mut theme.chrome;
        chrome.background = Some(Color::WHITE);
        chrome.axis_preset = AxisPreset::OpenOutward;
        chrome.grid.major_x.show = false;
        chrome.grid.major_y.show = false;
        chrome.grid.minor_x.show = false;
        chrome.grid.minor_y.show = false;
        chrome.legend_fill = rgba(255, 255, 255, 0.0);
        chrome.legend_border = rgba(255, 255, 255, 0.0);
        chrome.colorbar_border = Color::BLACK;
        chrome.highlight.color = rgb(213, 94, 0);
        let series = &mut theme.series;
        series.cycle = ColorCycle::Monochrome;
        series.lines.line_width = 1.5;
        series.lines.marker_size = 5.0;
        series.bars.material = Material::Flat;
        series.bars.outline = true;
        series.bars.outline_color = Color::BLACK;
        series.boxes.material = Material::Flat;
        series.boxes.ink = Color::BLACK;
        series.boxes.mean_color = Color::BLACK;
        series.boxes.outlier_color = Color::BLACK;
        series.fields.colormap = ColorMap::Viridis;
        series.fields.contour_label_color = Color::BLACK;
        series.surfaces.material = Material::Matte;
        series.energy_levels.limit_color = Color::BLACK;
        series.energy_levels.transition_color = rgb(80, 80, 80);
        series.energy_levels.band_fill = rgba(0, 0, 0, 0.12);
        series.radial.background = Color::WHITE;
        series.radial.label_color = Color::BLACK;
        theme
    }

    /// A web dashboard: no axis lines or ticks, horizontal grid lines only,
    /// large text and thick lines on a soft chart area.
    pub fn dashboard() -> Self {
        let mut theme = Self::light();
        let bg = rgb(250, 251, 253);
        let ink = rgb(36, 41, 51);
        let soft = rgb(96, 104, 118);
        let chrome = &mut theme.chrome;
        chrome.background = Some(bg);
        chrome.axis_preset = AxisPreset::Minimal;
        chrome.ink = ink;
        chrome.frame_ink = soft;
        chrome.label_ink = soft;
        chrome.title_size = 34.0;
        chrome.axis_title_size = 23.0;
        chrome.tick_label_size = 21.0;
        chrome.legend_size = 22.0;
        chrome.grid.major_x.show = false;
        chrome.grid.major_y = grid_line(true, rgb(226, 230, 236), 1.0, LineStylePreset::Solid);
        chrome.grid.minor_x.show = false;
        chrome.grid.minor_y.show = false;
        // A legend inside the plot sits on the data: a translucent card with
        // a grid-coloured edge, so a trace under it stays in view.
        chrome.legend_fill = rgba(250, 251, 253, 0.85);
        chrome.legend_border = rgb(226, 230, 236);
        chrome.colorbar_border = rgba(36, 41, 51, 0.25);
        let series = &mut theme.series;
        series.cycle = ColorCycle::Balanced;
        series.lines.line_width = 2.5;
        series.lines.marker_size = 8.0;
        series.lines.stem_width = 2.0;
        series.bars.border_color = bg;
        series.bars.material = Material::Flat;
        series.bars.outline = false;
        series.boxes.ink = ink;
        series.fields.colormap = ColorMap::Viridis;
        series.fields.contour_width = 1.5;
        series.vectors.width_px = 2.0;
        series.radar.line_width_px = Some(2.5);
        series.range_bars.connector_color = soft;
        series.radial.background = bg;
        series.radial.label_color = ink;
        series.radial.font_size = 18.0;
        theme
    }

    /// Hand-drawn strokes and the handwritten face on a cream chart area.
    pub fn sketch() -> Self {
        let mut theme = Self::light();
        let bg = rgb(253, 250, 241);
        let ink = rgb(45, 42, 38);
        let chrome = &mut theme.chrome;
        chrome.background = Some(bg);
        chrome.draw_style = DrawStyle::Sketch(crate::defaults::sketch_options());
        chrome.axis_preset = AxisPreset::OpenOutward;
        chrome.ink = ink;
        chrome.frame_ink = ink;
        chrome.label_ink = ink;
        chrome.axis_width = 1.5;
        chrome.grid.major_x.show = false;
        chrome.grid.major_y.color = rgb(222, 214, 196);
        chrome.legend_fill = bg;
        chrome.legend_border = ink;
        chrome.colorbar_border = ink;
        let series = &mut theme.series;
        series.cycle = ColorCycle::Vivid;
        series.lines.line_width = 2.0;
        series.lines.marker_size = 7.0;
        series.bars.border_color = ink;
        series.bars.material = Material::Paper;
        series.bars.outline = true;
        series.bars.outline_color = ink;
        series.boxes.material = Material::Paper;
        series.boxes.ink = ink;
        series.fields.contour_label_color = ink;
        series.surfaces.material = Material::Paper;
        series.radial.background = bg;
        series.radial.label_color = ink;
        series.radial.material = Material::Paper;
        theme
    }

    /// The built-in theme named `name`: `light`, `dark`, `paper`,
    /// `dashboard` or `sketch`.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "light" => Some(Self::light()),
            "dark" => Some(Self::dark()),
            "paper" => Some(Self::paper()),
            "dashboard" => Some(Self::dashboard()),
            "sketch" => Some(Self::sketch()),
            _ => None,
        }
    }

    /// Apply the theme to a chart and its series: the chrome of the chart,
    /// its overlays and the subplots `subplots` names, and every series
    /// drawn on one of those. The legend symbols follow the new series
    /// look. The frame is the chrome's [`AxisPreset`] unless a chart draws
    /// a family with a frame of its own.
    pub fn apply(&self, config: &mut Config, series: &mut [SeriesConfig], subplots: SubplotScope) {
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

    /// Apply the chrome alone, to a chart whose series are styled elsewhere
    /// ([`Self::style_series`]): the frame is the chrome's [`AxisPreset`].
    pub fn apply_chrome(&self, config: &mut Config, subplots: SubplotScope) {
        self.apply(config, &mut [], subplots);
    }

    /// Style one series as the `index`-th of its chart: the cycle colour,
    /// then its family's widths, sizes and materials. Ids, columns, labels
    /// and the series' structure stay.
    pub fn style_series(&self, series: &mut SeriesConfig, index: usize) {
        self.series.cycle.apply_to_series(series, index);
        let theme = &self.series;
        let lines = &theme.lines;
        series.for_each_line(|line| line.line_width = lines.line_width);
        match &mut series.render_type {
            DataRenderType::Xy(xy) => {
                if let Some(scatter) = &mut xy.scatter {
                    scatter.point_size = lines.marker_size;
                }
                if let Some(stem) = &mut xy.stem {
                    stem.width = lines.stem_width;
                }
                if let Some(errors) = &mut xy.errors {
                    errors.style.error_bar_width = lines.error_bar_width;
                    errors.style.cap_width = lines.error_cap_width;
                }
            }
            DataRenderType::Ternary {
                scatter, region, ..
            } => {
                if let Some(scatter) = scatter {
                    scatter.point_size = lines.marker_size;
                }
                if let Some(region) = region {
                    region.opacity = theme.ternary.region_opacity;
                }
            }
            DataRenderType::Histogram { bar } => {
                bar.border_color = theme.bars.border_color;
                bar.border_width = theme.bars.border_width;
            }
            DataRenderType::CategoryBars { style, .. }
            | DataRenderType::RangeBars { style, .. } => {
                style.material = theme.bars.material;
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
                style.outline_width = boxes.outline_width;
                style.outline_color = boxes.ink;
                style.median_width = boxes.median_width;
                style.median_color = boxes.ink;
                style.whisker_width = boxes.whisker_width;
                style.whisker_color = boxes.ink;
                style.mean_color = boxes.mean_color;
                style.outlier_color = boxes.outlier_color;
                if let Some(Violin { body, .. }) = violin {
                    let half = |half: &mut ViolinHalf| {
                        half.style.outline_width = boxes.outline_width;
                        half.style.outline_color = boxes.ink;
                    };
                    match body {
                        ViolinBody::Symmetric(one) => half(one),
                        ViolinBody::Split { low, high } => {
                            low.iter_mut().chain(high.iter_mut()).for_each(half);
                        }
                    }
                }
                if let Some(BoxPoints { style, .. }) = points {
                    style.color = boxes.ink;
                    style.size = boxes.point_size;
                }
            }
            DataRenderType::Field(FieldSeries {
                contour, surface, ..
            }) => {
                self.style_field(contour.as_mut(), surface.as_mut());
            }
            DataRenderType::TriMesh(mesh) => {
                self.style_field(mesh.contour.as_mut(), mesh.surface.as_mut());
            }
            DataRenderType::Vector { style, .. } => {
                style.width_px = theme.vectors.width_px;
                style.head_length_px = theme.vectors.head_length_px;
                style.head_width_px = theme.vectors.head_width_px;
            }
            DataRenderType::Radar { style, .. } => {
                style.line_width_px = theme.radar.line_width_px;
                style.marker_size_px = theme.radar.marker_size_px;
                style.fill_opacity = theme.radar.fill_opacity;
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
            connector,
            ..
        } = &mut series.render_type
        {
            if let Some(whiskers) = whiskers {
                whiskers.width = theme.range_bars.whisker_width;
            }
            if let Some(connector) = connector {
                connector.color = theme.range_bars.connector_color;
                connector.width = theme.range_bars.connector_width;
            }
        }
    }

    fn style_field(
        &self,
        contour: Option<&mut crate::data_config::ContourConfig>,
        surface: Option<&mut crate::view3d::SurfaceStyle>,
    ) {
        let fields = &self.series.fields;
        if let Some(contour) = contour
            && let Some(labels) = &mut contour.labels
        {
            labels.color = fields.contour_label_color;
        }
        if let Some(surface) = surface {
            let surfaces = &self.series.surfaces;
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
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            self.style_axis(axis);
        }
        self.style_text(&mut config.chart_title.text, chrome.ink, chrome.title_size);
        self.style_grid(&mut config.grid);
        let colormap = match &mut config.colorbar {
            Some(bar) => {
                bar.colormap = self.series.fields.colormap.clone();
                bar.nan_color = self.series.fields.nan_color;
                bar.border_color = chrome.colorbar_border;
                self.style_axis_look(&mut bar.axis);
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
        self.style_text(&mut legend.content, chrome.label_ink, chrome.legend_size);
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

    /// Apply the theme to a radial chart: its background, label look,
    /// material and title, and the cycle colours on its top-level slices.
    /// Values, labels, the slice structure and the font family stay.
    pub fn apply_radial(&self, chart: &mut RadialChart) {
        let radial = &self.series.radial;
        chart.background = radial.background;
        chart.label_color = radial.label_color;
        chart.font_size = radial.font_size;
        chart.style.material = radial.material;
        chart.style.gloss = radial.gloss;
        chart.style.shadow = radial.shadow;
        for (index, slice) in chart.slices.iter_mut().enumerate() {
            slice.color = self.series.cycle.color(index);
        }
        if let Some(title) = &mut chart.chart_title {
            self.style_text(&mut title.text, radial.label_color, self.chrome.title_size);
        }
    }
}

#[cfg(test)]
mod tests;
