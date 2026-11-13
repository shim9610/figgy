//! The chart kinds a theme is made for (#291).

use crate::config::{AxisScale, Config};
use crate::coordinates::Coordinates;
use crate::data_config::{DataRenderType, SeriesConfig, XySeries};
use crate::layer::LayerRef;

/// The kind of one chart, which decides the themes it takes. A theme is
/// made for one kind: a dashboard look is for line charts, a parchment
/// card for Feynman diagrams. The radial chart is a model of its own and
/// has its own themes ([`super::RadialTheme`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChartKind {
    /// Lines, areas and steps: x-y series that draw a line.
    Line,
    /// Markers alone: x-y series that draw no line, including value maps.
    Scatter,
    /// Measurements with error bars.
    ErrorBars,
    Histogram,
    /// Category bars, grouped or stacked.
    CategoryBars,
    /// Box plots, violins and rainclouds.
    Distribution,
    /// Range bars: waterfalls, candlesticks and Gantt charts.
    RangeBars,
    /// Stems and lollipops.
    Stem,
    /// Heatmaps, contours, hexbins and RGB images.
    Field,
    /// Vector arrows and streamlines.
    VectorField,
    /// Any chart on a 3D view: surfaces, 3D meshes, 3D arrows and lines.
    Surface3d,
    /// A field over a triangle mesh.
    TriMesh,
    /// A chart on polar coordinates, including polar heatmaps and roses.
    Polar,
    Radar,
    /// Parallel coordinates.
    Parallel,
    Ternary,
    /// A chart with a probability axis.
    Probability,
    /// Energy-level (Grotrian) diagrams.
    EnergyLevels,
    /// Feynman diagrams.
    Feynman,
}

impl ChartKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 19] = [
        Self::Line,
        Self::Scatter,
        Self::ErrorBars,
        Self::Histogram,
        Self::CategoryBars,
        Self::Distribution,
        Self::RangeBars,
        Self::Stem,
        Self::Field,
        Self::VectorField,
        Self::Surface3d,
        Self::TriMesh,
        Self::Polar,
        Self::Radar,
        Self::Parallel,
        Self::Ternary,
        Self::Probability,
        Self::EnergyLevels,
        Self::Feynman,
    ];

    /// The kind's name in the web API and in error messages.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Scatter => "scatter",
            Self::ErrorBars => "error-bars",
            Self::Histogram => "histogram",
            Self::CategoryBars => "category-bars",
            Self::Distribution => "distribution",
            Self::RangeBars => "range-bars",
            Self::Stem => "stem",
            Self::Field => "field",
            Self::VectorField => "vector-field",
            Self::Surface3d => "surface-3d",
            Self::TriMesh => "tri-mesh",
            Self::Polar => "polar",
            Self::Radar => "radar",
            Self::Parallel => "parallel",
            Self::Ternary => "ternary",
            Self::Probability => "probability",
            Self::EnergyLevels => "energy-levels",
            Self::Feynman => "feynman",
        }
    }

    /// The kind of the chart `layer` names: the base chart or a subplot,
    /// read from its coordinates, its axes and the series drawn on it and
    /// on its overlays. `None` for a chart that draws no series yet: any
    /// theme applies to it. An overlay or the annotation layer reads as its
    /// chart.
    ///
    /// The coordinates decide first: a 3D view is [`Self::Surface3d`], a
    /// ternary chart [`Self::Ternary`], a parallel chart
    /// [`Self::Parallel`], and a polar chart [`Self::Radar`] when it draws a
    /// radar series, [`Self::Polar`] otherwise. On a rectangular chart a
    /// mix of families takes the most specific one, in this order:
    /// Feynman, energy levels, a probability axis, vectors, a triangle
    /// mesh, a field, range bars, boxes, category bars, a histogram, stems,
    /// error bars; then [`Self::Scatter`] when no x-y series draws a line,
    /// and [`Self::Line`] otherwise. So a candlestick with a moving average
    /// is range bars, and wind arrows over a speed field are a vector field.
    pub fn of(config: &Config, series: &[SeriesConfig], layer: LayerRef) -> Option<Self> {
        let (chart, subplot) = match layer {
            LayerRef::Subplot(i) | LayerRef::SubplotOverlay(i, _) => match config.subplots.get(i) {
                Some(subplot) => (&subplot.config, Some(i)),
                None => (config, None),
            },
            LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => (config, None),
        };
        let on_chart =
            |item: &&SeriesConfig| match (subplot, config.layer_of_series(&item.series_id)) {
                (Some(i), LayerRef::Subplot(j) | LayerRef::SubplotOverlay(j, _)) => i == j,
                (Some(_), _) => false,
                (None, LayerRef::Subplot(_) | LayerRef::SubplotOverlay(..)) => false,
                (None, _) => true,
            };
        if !series.iter().any(|item| on_chart(&item)) {
            return None;
        }
        let has = |test: fn(&DataRenderType) -> bool| {
            series
                .iter()
                .filter(on_chart)
                .any(|item| test(&item.render_type))
        };
        match &chart.coordinates {
            Coordinates::View3d(_) => return Some(Self::Surface3d),
            Coordinates::Ternary(_) => return Some(Self::Ternary),
            Coordinates::Parallel(_) => return Some(Self::Parallel),
            Coordinates::Polar(_) => {
                return Some(if has(|rt| rt.radar().is_some()) {
                    Self::Radar
                } else {
                    Self::Polar
                });
            }
            Coordinates::Cartesian => {}
        }
        let probability = chart_axes(chart).any(|scale| matches!(scale, AxisScale::Probability(_)));
        let kind = if has(|rt| matches!(rt, DataRenderType::Feynman(_))) {
            Self::Feynman
        } else if has(|rt| matches!(rt, DataRenderType::EnergyLevels(_))) {
            Self::EnergyLevels
        } else if probability {
            Self::Probability
        } else if has(|rt| {
            matches!(
                rt,
                DataRenderType::Vector { .. } | DataRenderType::Streamlines { .. }
            )
        }) {
            Self::VectorField
        } else if has(|rt| matches!(rt, DataRenderType::TriMesh(_))) {
            Self::TriMesh
        } else if has(|rt| {
            matches!(
                rt,
                DataRenderType::Field(_) | DataRenderType::RgbImage { .. }
            )
        }) {
            Self::Field
        } else if has(|rt| matches!(rt, DataRenderType::RangeBars { .. })) {
            Self::RangeBars
        } else if has(|rt| matches!(rt, DataRenderType::BoxPlot { .. })) {
            Self::Distribution
        } else if has(|rt| matches!(rt, DataRenderType::CategoryBars { .. })) {
            Self::CategoryBars
        } else if has(|rt| matches!(rt, DataRenderType::Histogram { .. })) {
            Self::Histogram
        } else if has(|rt| matches!(rt, DataRenderType::Xy(XySeries { stem: Some(_), .. }))) {
            Self::Stem
        } else if has(|rt| {
            matches!(
                rt,
                DataRenderType::Xy(XySeries {
                    errors: Some(_),
                    ..
                })
            )
        }) {
            Self::ErrorBars
        } else if has(|rt| {
            matches!(
                rt,
                DataRenderType::Xy(XySeries { line: Some(_), .. })
                    | DataRenderType::MonotonicLine(_)
            )
        }) {
            Self::Line
        } else if has(|rt| matches!(rt, DataRenderType::Xy(_))) {
            Self::Scatter
        } else {
            // Radar, ternary and parallel series off their own coordinates
            // are invalid documents; they read as the line charts they sit on.
            Self::Line
        };
        Some(kind)
    }
}

impl std::fmt::Display for ChartKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The scales of a chart's own axes and of its overlays' axes.
fn chart_axes(chart: &Config) -> impl Iterator<Item = &AxisScale> {
    [&chart.bottom_x, &chart.top_x, &chart.left_y, &chart.right_y]
        .into_iter()
        .chain(
            chart
                .overlays
                .iter()
                .flat_map(|overlay| [&overlay.x_axis, &overlay.y_axis]),
        )
        .map(|axis| &axis.scale)
}
