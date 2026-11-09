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
