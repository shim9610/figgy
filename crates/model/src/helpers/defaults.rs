//! `Default` on the document types, for hosts and examples (#266).

//!
//! Each impl returns the type's documented default from its one crate-private
//! source in `crate::defaults`. The model and the renderer build without the
//! `helpers` feature, so their own code cannot reach for `Default`.

// The deprecated document readers keep their `Default` until they go.
#![allow(deprecated)]

impl Default for crate::annotation::AnnotationLineStyle {
    fn default() -> Self {
        crate::defaults::annotation_line_style()
    }
}

impl Default for crate::annotation::ArrowStyle {
    fn default() -> Self {
        crate::defaults::arrow_style()
    }
}

impl Default for crate::annotation::BoxStyle {
    fn default() -> Self {
        crate::defaults::box_style()
    }
}

impl Default for crate::annotation::NoteAlign {
    fn default() -> Self {
        crate::defaults::note_align()
    }
}

impl Default for crate::annotation::ShapeStyle {
    fn default() -> Self {
        crate::defaults::shape_style()
    }
}

impl Default for crate::boxplot::BoxPlotChart {
    fn default() -> Self {
        crate::defaults::box_plot_chart()
    }
}

impl Default for crate::boxplot::BoxPlotDirection {
    fn default() -> Self {
        crate::defaults::box_plot_direction()
    }
}

impl Default for crate::categorical::CategoricalChart {
    fn default() -> Self {
        crate::defaults::categorical_chart()
    }
}

impl Default for crate::colormap::ColorMap {
    fn default() -> Self {
        crate::defaults::color_map()
    }
}

impl Default for crate::config::AxisBreakStyle {
    fn default() -> Self {
        crate::defaults::axis_break_style()
    }
}

impl Default for crate::config::AxisBreaks {
    fn default() -> Self {
        crate::defaults::axis_breaks()
    }
}

impl Default for crate::config::BreakMark {
    fn default() -> Self {
        crate::defaults::break_mark()
    }
}

impl Default for crate::config::ConstellationOptions {
    fn default() -> Self {
        crate::defaults::constellation_options()
    }
}

impl Default for crate::config::DataSelectionsConfig {
    fn default() -> Self {
        crate::defaults::data_selections_config()
    }
}

impl Default for crate::config::DrawStyle {
    fn default() -> Self {
        crate::defaults::draw_style()
    }
}

impl Default for crate::config::LogTickPolicy {
    fn default() -> Self {
        crate::defaults::log_tick_policy()
    }
}

impl Default for crate::config::MilkywayOptions {
    fn default() -> Self {
        crate::defaults::milkyway_options()
    }
}

impl Default for crate::config::PieceSizing {
    fn default() -> Self {
        crate::defaults::piece_sizing()
    }
}

impl Default for crate::config::SketchOptions {
    fn default() -> Self {
        crate::defaults::sketch_options()
    }
}

impl Default for crate::data_config::BoxOutlierShape {
    fn default() -> Self {
        crate::defaults::box_outlier_shape()
    }
}

impl Default for crate::data_config::BoxPlotLabels {
    fn default() -> Self {
        crate::defaults::box_plot_labels()
    }
}

impl Default for crate::data_config::BoxPlotSeriesLabels {
    fn default() -> Self {
        crate::defaults::box_plot_series_labels()
    }
}

impl Default for crate::data_config::BoxPlotStyle {
    fn default() -> Self {
        crate::defaults::box_plot_style()
    }
}

impl Default for crate::data_config::BoxPointsStyle {
    fn default() -> Self {
        crate::defaults::box_points_style()
    }
}

impl Default for crate::data_config::ViolinStyle {
    fn default() -> Self {
        crate::defaults::violin_style()
    }
}

impl Default for crate::data_config::BarGroups {
    fn default() -> Self {
        crate::defaults::bar_groups()
    }
}

impl Default for crate::data_config::CategoryBarDirection {
    fn default() -> Self {
        crate::defaults::category_bar_direction()
    }
}

impl Default for crate::data_config::CategoryBarLabelFormat {
    fn default() -> Self {
        crate::defaults::category_bar_label_format()
    }
}

impl Default for crate::data_config::CategoryBarLabels {
    fn default() -> Self {
        crate::defaults::category_bar_labels()
    }
}

impl Default for crate::data_config::CategoryBarMode {
    fn default() -> Self {
        crate::defaults::category_bar_mode()
    }
}

impl Default for crate::data_config::CategoryBarSeriesLabels {
    fn default() -> Self {
        crate::defaults::category_bar_series_labels()
    }
}

impl Default for crate::data_config::CategoryBarStyle {
    fn default() -> Self {
        crate::defaults::category_bar_style()
    }
}

impl Default for crate::data_config::DataBarBinStyleConfig {
    fn default() -> Self {
        crate::defaults::data_bar_bin_style_config()
    }
}

impl Default for crate::data_config::LineOffsets {
    fn default() -> Self {
        crate::defaults::line_offsets()
    }
}

impl Default for crate::data_config::RadarLabels {
    fn default() -> Self {
        crate::defaults::radar_labels()
    }
}

impl Default for crate::data_config::RadarMarker {
    fn default() -> Self {
        crate::defaults::radar_marker()
    }
}

impl Default for crate::data_config::RadarSeriesLabels {
    fn default() -> Self {
        crate::defaults::radar_series_labels()
    }
}

impl Default for crate::data_config::RadarSeriesStyle {
    fn default() -> Self {
        crate::defaults::radar_series_style()
    }
}

impl Default for crate::data_config::RangeColor {
    fn default() -> Self {
        crate::defaults::range_color()
    }
}

impl Default for crate::data_config::StreamlineArrows {
    fn default() -> Self {
        crate::defaults::streamline_arrows()
    }
}

impl Default for crate::data_config::VectorPivot {
    fn default() -> Self {
        crate::defaults::vector_pivot()
    }
}

impl Default for crate::data_config::VectorScale {
    fn default() -> Self {
        crate::defaults::vector_scale()
    }
}

impl Default for crate::data_config::DataErrorBarPointStyleConfig {
    fn default() -> Self {
        crate::defaults::data_error_bar_point_style_config()
    }
}

impl Default for crate::data_config::DataLineStyleConfig {
    fn default() -> Self {
        crate::defaults::data_line_style_config()
    }
}

impl Default for crate::data_config::DataScatterPointStyleConfig {
    fn default() -> Self {
        crate::defaults::data_scatter_point_style_config()
    }
}

impl Default for crate::data_config::LineFill {
    fn default() -> Self {
        crate::defaults::line_fill()
    }
}

impl Default for crate::data_config::LineInterpolation {
    fn default() -> Self {
        crate::defaults::line_interpolation()
    }
}

impl Default for crate::data_config::SizeScale {
    fn default() -> Self {
        crate::defaults::size_scale()
    }
}

impl Default for crate::decoration::DecorationLayout {
    fn default() -> Self {
        crate::defaults::decoration_layout()
    }
}

impl Default for crate::energy_levels::EnergyLevelSeries {
    fn default() -> Self {
        crate::defaults::energy_level_series()
    }
}

impl Default for crate::energy_levels::EnergyLevelStyle {
    fn default() -> Self {
        crate::defaults::energy_level_style()
    }
}

impl Default for crate::energy_levels::LevelBandStyle {
    fn default() -> Self {
        crate::defaults::level_band_style()
    }
}

impl Default for crate::energy_levels::LevelLabelSide {
    fn default() -> Self {
        crate::defaults::level_label_side()
    }
}

impl Default for crate::energy_levels::LevelRole {
    fn default() -> Self {
        crate::defaults::level_role()
    }
}

impl Default for crate::energy_levels::LevelStrokeStyle {
    fn default() -> Self {
        crate::defaults::level_stroke_style()
    }
}

impl Default for crate::energy_levels::LevelView {
    fn default() -> Self {
        crate::defaults::level_view()
    }
}

impl Default for crate::energy_levels::TransitionArrowStyle {
    fn default() -> Self {
        crate::defaults::transition_arrow_style()
    }
}

impl Default for crate::energy_levels::TransitionDirection {
    fn default() -> Self {
        crate::defaults::transition_direction()
    }
}

impl Default for crate::energy_levels::WavelengthLabels {
    fn default() -> Self {
        crate::defaults::wavelength_labels()
    }
}

impl Default for crate::feynman::FeynmanLayout {
    fn default() -> Self {
        crate::defaults::feynman_layout()
    }
}

impl Default for crate::feynman::FeynmanPicked {
    fn default() -> Self {
        crate::defaults::feynman_picked()
    }
}

impl Default for crate::feynman::FeynmanSeries {
    fn default() -> Self {
        crate::defaults::feynman_series()
    }
}

impl Default for crate::feynman::FeynmanStyle {
    fn default() -> Self {
        crate::defaults::feynman_style()
    }
}

impl Default for crate::feynman::MomentumSide {
    fn default() -> Self {
        crate::defaults::momentum_side()
    }
}

impl Default for crate::feynman::ParticleLine {
    fn default() -> Self {
        crate::defaults::particle_line()
    }
}

impl Default for crate::feynman::TimeDirection {
    fn default() -> Self {
        crate::defaults::time_direction()
    }
}

impl Default for crate::feynman::VertexRole {
    fn default() -> Self {
        crate::defaults::vertex_role()
    }
}

impl Default for crate::format::FractionalSecondDigits {
    fn default() -> Self {
        crate::defaults::fractional_second_digits()
    }
}

impl Default for crate::format::PlainNumberFormat {
    fn default() -> Self {
        crate::defaults::plain_number_format()
    }
}

impl Default for crate::format::TimestampLabelFormat {
    fn default() -> Self {
        crate::defaults::timestamp_label_format()
    }
}

impl Default for crate::format::TimestampLabelMode {
    fn default() -> Self {
        crate::defaults::timestamp_label_mode()
    }
}

impl Default for crate::format::TimestampTickPolicy {
    fn default() -> Self {
        crate::defaults::timestamp_tick_policy()
    }
}

impl Default for crate::format::TimestampUnit {
    fn default() -> Self {
        crate::defaults::timestamp_unit()
    }
}

impl Default for crate::format::TimestampZone {
    fn default() -> Self {
        crate::defaults::timestamp_zone()
    }
}

impl Default for crate::layer::AnnotationLayer {
    fn default() -> Self {
        crate::defaults::annotation_layer()
    }
}

impl Default for crate::layer::InsetMarker {
    fn default() -> Self {
        crate::defaults::inset_marker()
    }
}

impl Default for crate::layer::LayerRef {
    fn default() -> Self {
        crate::defaults::layer_ref()
    }
}

impl Default for crate::layer::OverlayLayer {
    fn default() -> Self {
        crate::defaults::overlay_layer()
    }
}

impl Default for crate::layer::Subplot {
    fn default() -> Self {
        crate::defaults::subplot()
    }
}

impl Default for crate::line::LineStylePreset {
    fn default() -> Self {
        crate::defaults::line_style_preset()
    }
}

impl Default for crate::material::Material {
    fn default() -> Self {
        crate::defaults::material()
    }
}

impl Default for crate::parallel::ParallelOptions {
    fn default() -> Self {
        crate::defaults::parallel_options()
    }
}

impl Default for crate::polar::AngleUnit {
    fn default() -> Self {
        crate::defaults::angle_unit()
    }
}

impl Default for crate::polar::GridShape {
    fn default() -> Self {
        crate::defaults::grid_shape()
    }
}

impl Default for crate::polar::PolarInterpolation {
    fn default() -> Self {
        crate::defaults::polar_interpolation()
    }
}

impl Default for crate::polar::PolarOptions {
    fn default() -> Self {
        crate::defaults::polar_options()
    }
}

impl Default for crate::radar::RadarChart {
    fn default() -> Self {
        crate::defaults::radar_chart()
    }
}

impl Default for crate::radar::RadarGridShape {
    fn default() -> Self {
        crate::defaults::radar_grid_shape()
    }
}

impl Default for crate::radar::RadarStyle {
    fn default() -> Self {
        crate::defaults::radar_style()
    }
}

impl Default for crate::radial::RadialChart {
    fn default() -> Self {
        crate::defaults::radial_chart()
    }
}

impl Default for crate::radial::RadialInteraction {
    fn default() -> Self {
        crate::defaults::radial_interaction()
    }
}

impl Default for crate::radial::RadialLabelFormat {
    fn default() -> Self {
        crate::defaults::radial_label_format()
    }
}

impl Default for crate::radial::RadialOutline {
    fn default() -> Self {
        crate::defaults::radial_outline()
    }
}

impl Default for crate::radial::RadialStyle {
    fn default() -> Self {
        crate::defaults::radial_style()
    }
}

impl Default for crate::ternary::TernaryOptions {
    fn default() -> Self {
        crate::defaults::ternary_options()
    }
}

impl Default for crate::ternary::TernaryRegionStyle {
    fn default() -> Self {
        crate::defaults::ternary_region_style()
    }
}

impl Default for crate::text::TextAlign {
    fn default() -> Self {
        crate::defaults::text_align()
    }
}

impl Default for crate::text::math::MathStyle {
    fn default() -> Self {
        crate::defaults::math_style()
    }
}

impl Default for crate::view3d::Projection3d {
    fn default() -> Self {
        crate::defaults::projection3d()
    }
}

impl Default for crate::view3d::SurfaceStyle {
    fn default() -> Self {
        crate::defaults::surface_style()
    }
}

impl Default for crate::view3d::View3d {
    fn default() -> Self {
        crate::defaults::view3d()
    }
}

// The chart frame: the document and the options `default` builds (#265).
// `AxisOptions` and `LabelStyle` have none: their default differs by axis
// (`default_axis_options_x`, `_y`, `_colorbar`).

impl Default for crate::config::Config {
    fn default() -> Self {
        crate::defaults::config()
    }
}

impl Default for crate::config::ChartTitleOptions {
    fn default() -> Self {
        crate::defaults::chart_title_options()
    }
}

impl Default for crate::config::AxisTitleOptions {
    fn default() -> Self {
        crate::defaults::axis_title_options()
    }
}

impl Default for crate::config::GridOptions {
    fn default() -> Self {
        crate::defaults::grid_options()
    }
}

impl Default for crate::config::ColorBarOptions {
    fn default() -> Self {
        crate::defaults::color_bar_options()
    }
}

impl Default for crate::legend::Legend {
    fn default() -> Self {
        crate::defaults::legend()
    }
}

impl Default for crate::layout::ChartArea {
    fn default() -> Self {
        crate::defaults::chart_area()
    }
}

// Series styles a document always spells out (#265). They have no
// `defaults/` source because no document omits them: these values exist for
// hosts only. Strokes take the ink of the documented line default (black,
// 1 px); a bar fills with the first colour of the balanced palette. A theme
// recolours them.

/// Ink for the stroke-only host defaults below.
const INK: crate::color::Color = crate::color::Color::BLACK;

impl Default for crate::data_config::Stem {
    fn default() -> Self {
        Self {
            baseline: 0.0,
            direction: crate::data_config::CategoryBarDirection::Vertical,
            color: INK,
            width: 1.0,
            line_style: crate::line::LineStylePreset::Solid,
        }
    }
}

impl Default for crate::data_config::DataScatterStyleConfig {
    fn default() -> Self {
        Self {
            point_color: INK,
            point_shape: crate::data_config::ScatterShape::CircleFilled,
            point_size: 6.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        }
    }
}

impl Default for crate::data_config::DataErrorBarStyleConfig {
    fn default() -> Self {
        Self {
            error_bar_color: INK,
            error_bar_width: 1.0,
            error_bar_cap_size: 6.0,
            cap_width: 1.0,
            error_bar_style_table: None,
            error_bar_style_index_column: None,
            error_bar_style_overrides: None,
        }
    }
}

impl Default for crate::data_config::DataBarStyleConfig {
    fn default() -> Self {
        Self {
            fill_color: crate::preset::ColorCycle::Balanced.color(0),
            border_color: INK,
            border_width: 1.0,
            baseline: 0.0,
            gap_px: 0.0,
            width_ratio: 1.0,
            orientation: crate::data_config::BarOrientation::Vertical,
            bar_style_overrides: None,
        }
    }
}

impl Default for crate::data_config::VectorStyle {
    fn default() -> Self {
        Self {
            color: INK,
            width_px: 1.5,
            scale: crate::defaults::vector_scale(),
            pivot: crate::defaults::vector_pivot(),
            head_length_px: 9.0,
            head_width_px: 8.0,
            min_length_px: 0.0,
            color_map: None,
        }
    }
}

impl Default for crate::data_config::FieldFillConfig {
    fn default() -> Self {
        Self {
            mode: crate::data_config::FillMode::Continuous,
            shading: crate::data_config::Shading::Flat,
            opacity: 1.0,
        }
    }
}

impl Default for crate::data_config::ContourLabelConfig {
    fn default() -> Self {
        Self {
            visible: true,
            font_size: 12.0,
            color: crate::defaults::contour_label_color(),
            format: crate::format::LabelFormat::Decimal,
            significant_digits: 2,
            spacing_px: 120.0,
            anchors: Vec::new(),
            bg_color: None,
            bg_padding_px: 2.0,
        }
    }
}

impl Default for crate::data_config::RangeConnector {
    fn default() -> Self {
        Self {
            color: INK,
            width: 1.0,
        }
    }
}

impl Default for crate::view3d::SurfaceWireframe {
    fn default() -> Self {
        Self {
            color: INK,
            width: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::data_config::{
        ContourConfig, ContourLabelConfig, DataBarStyleConfig, DataErrorBarStyleConfig,
        DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ErrorBars, ErrorRef,
        FieldFillConfig, FieldSeries, GridLayout, MatrixOrientation, MatrixRef, SeriesConfig, Stem,
        VectorStyle, XySeries,
    };

    fn series(render_type: DataRenderType) -> SeriesConfig {
        SeriesConfig {
            series_id: "s".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type,
        }
    }

    // Each host style default is a valid part of a series as it stands
    // (#265): markers with error bars and stems, a line, histogram bars, a
    // filled field with labelled contours, and the vector and wireframe
    // looks.
    #[test]
    fn host_style_defaults_build_valid_series() {
        let config = crate::Config::default();
        let xy = series(DataRenderType::Xy(XySeries {
            scatter: Some(DataScatterStyleConfig::default()),
            line: Some(DataLineStyleConfig::default()),
            errors: Some(ErrorBars {
                x: None,
                y: Some(ErrorRef::Symmetric { column: "e".into() }),
                style: DataErrorBarStyleConfig::default(),
            }),
            stem: Some(Stem::default()),
        }));
        let histogram = series(DataRenderType::Histogram {
            bar: DataBarStyleConfig::default(),
        });
        let field = series(DataRenderType::Field(FieldSeries {
            matrix: MatrixRef {
                columns: vec!["z0".into(), "z1".into()],
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            fill: Some(FieldFillConfig::default()),
            contour: Some(ContourConfig {
                levels: vec![0.5],
                line: DataLineStyleConfig::default(),
                per_level_color: None,
                labels: Some(ContourLabelConfig::default()),
            }),
            surface: None,
        }));
        for item in [&xy, &histogram, &field] {
            config
                .validate_series(std::slice::from_ref(item))
                .unwrap_or_else(|error| panic!("{:?}: {error:?}", item.render_type));
        }
        Stem::default().validate().unwrap();
        ContourLabelConfig::default().validate().unwrap();
        VectorStyle::default().validate().unwrap();
        let wireframe = crate::view3d::SurfaceWireframe::default();
        assert!(wireframe.width > 0.0 && wireframe.width.is_finite());
    }
}
