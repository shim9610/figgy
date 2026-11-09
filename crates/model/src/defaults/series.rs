//! Series styles: lines, markers, bars, box plots, fields, vectors, ranges.

#![allow(unused_imports)]
// Some sources are read only by `Default` (`crate::helpers`).
#![cfg_attr(not(feature = "helpers"), allow(dead_code))]

use super::*;
use crate::data_config::*;
use crate::format::*;
use crate::layout::*;
use crate::line::*;
use crate::text::*;
use crate::*;

/// The documented default of [`BarGroups`]: the value a document that
/// omits it takes.
pub(crate) fn bar_groups() -> BarGroups {
    BarGroups {
        mode: CategoryBarMode::Grouped,
        direction: CategoryBarDirection::Vertical,
        group_width: 0.7,
        bar_gap: 3.0,
    }
}

/// The documented default of [`BoxOutlierShape`]: the value a document that
/// omits it takes.
pub(crate) const fn box_outlier_shape() -> BoxOutlierShape {
    BoxOutlierShape::Circle
}

/// The documented default of [`BoxPlotLabels`]: the value a document that
/// omits it takes.
pub(crate) const fn box_plot_labels() -> BoxPlotLabels {
    BoxPlotLabels::None
}

/// The documented default of [`BoxPlotSeriesLabels`]: the value a document that
/// omits it takes.
pub(crate) fn box_plot_series_labels() -> BoxPlotSeriesLabels {
    BoxPlotSeriesLabels {
        placement: BoxPlotLabels::None,
        decimals: 1,
        value_suffix: String::new(),
        text: Vec::new(),
        offsets: Vec::new(),
    }
}

/// The documented default of [`BoxPlotStyle`]: the value a document that
/// omits it takes.
pub(crate) fn box_plot_style() -> BoxPlotStyle {
    let ink = Color::from_rgb8(32, 46, 60);
    BoxPlotStyle {
        material: Material::Matte,
        corner_radius: 0.0,
        outline: true,
        outline_width: 1.2,
        outline_color: ink,
        median_width: 1.2,
        median_color: ink,
        whisker_width: 1.2,
        whisker_color: ink,
        caps: true,
        cap_ratio: 0.5,
        notched: false,
        notch_depth: 0.22,
        show_mean: false,
        mean_size: 7.0,
        mean_color: Color::from_rgb8(180, 45, 45),
        show_outliers: true,
        outlier_size: 7.0,
        outlier_color: ink,
        outlier_shape: BoxOutlierShape::Circle,
        outlier_filled: false,
        texture_strength: 0.2,
        texture_scale: 1.0,
        gloss: 0.45,
        emphasis_brightness: 0.08,
    }
}

/// The documented default of [`BoxPointsStyle`]: the value a document that
/// omits it takes.
pub(crate) fn box_points_style() -> BoxPointsStyle {
    BoxPointsStyle {
        color: Color::from_rgb8(32, 46, 60),
        size: 4.0,
        shape: BoxOutlierShape::Circle,
        filled: true,
        jitter: 0.5,
        offset: 0.0,
    }
}

/// The documented default of [`CategoryBarDirection`]: the value a document that
/// omits it takes.
pub(crate) const fn category_bar_direction() -> CategoryBarDirection {
    CategoryBarDirection::Vertical
}

/// The documented default of [`CategoryBarLabelFormat`]: the value a document that
/// omits it takes.
pub(crate) const fn category_bar_label_format() -> CategoryBarLabelFormat {
    CategoryBarLabelFormat::Value
}

/// The documented default of [`CategoryBarLabels`]: the value a document that
/// omits it takes.
pub(crate) const fn category_bar_labels() -> CategoryBarLabels {
    CategoryBarLabels::Auto
}

/// The documented default of [`CategoryBarMode`]: the value a document that
/// omits it takes.
pub(crate) const fn category_bar_mode() -> CategoryBarMode {
    CategoryBarMode::Grouped
}

/// The documented default of [`CategoryBarSeriesLabels`]: the value a document that
/// omits it takes.
pub(crate) fn category_bar_series_labels() -> CategoryBarSeriesLabels {
    CategoryBarSeriesLabels {
        placement: CategoryBarLabels::Auto,
        decimals: 1,
        value_suffix: String::new(),
        text: Vec::new(),
        offsets: Vec::new(),
    }
}

/// The documented default of [`CategoryBarStyle`]: the value a document that
/// omits it takes.
pub(crate) fn category_bar_style() -> CategoryBarStyle {
    CategoryBarStyle {
        material: Material::Flat,
        corner_radius: 5.0,
        outline: false,
        outline_width: 1.0,
        outline_color: Color::from_rgb8(40, 55, 75),
        texture_strength: 0.2,
        texture_scale: 1.0,
        gloss: 0.45,
        emphasis_brightness: 0.08,
    }
}

/// The documented default of [`DataBarBinStyleConfig`]: the value a document that
/// omits it takes.
pub(crate) fn data_bar_bin_style_config() -> DataBarBinStyleConfig {
    DataBarBinStyleConfig {
        fill_color: None,
        border_color: None,
        border_width: None,
        gap_px: None,
        width_ratio: None,
    }
}

/// The documented default of [`DataErrorBarPointStyleConfig`]: the value a document that
/// omits it takes.
pub(crate) fn data_error_bar_point_style_config() -> DataErrorBarPointStyleConfig {
    DataErrorBarPointStyleConfig {
        error_bar_color: None,
        error_bar_width: None,
        error_bar_cap_size: None,
        cap_width: None,
    }
}

/// The documented default of [`DataLineStyleConfig`]: the value a document that
/// omits it takes.
pub(crate) fn data_line_style_config() -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: LineStylePreset::Solid,
        line_color: Color::BLACK,
        line_width: 1.0,
        fill: None,
        fill_color: None,
        interpolation: crate::data_config::LineInterpolation::Linear,
    }
}

/// The documented default of [`DataScatterPointStyleConfig`]: the value a document that
/// omits it takes.
pub(crate) fn data_scatter_point_style_config() -> DataScatterPointStyleConfig {
    DataScatterPointStyleConfig {
        point_color: None,
        point_shape: None,
        point_size: None,
    }
}

/// The documented default of [`LineFill`]: the value a document that
/// omits it takes.
pub(crate) fn line_fill() -> LineFill {
    LineFill::ToBaseline(0.0)
}

/// The documented default of [`LineInterpolation`]: the value a document that
/// omits it takes.
pub(crate) const fn line_interpolation() -> LineInterpolation {
    LineInterpolation::Linear
}

/// The documented default of [`LineOffsets`]: the value a document that
/// omits it takes.
pub(crate) fn line_offsets() -> LineOffsets {
    LineOffsets {
        series: Vec::new(),
        step_y: 0.0,
        step_x: 0.0,
    }
}

/// The documented default of [`RadarLabels`]: the value a document that
/// omits it takes.
pub(crate) const fn radar_labels() -> RadarLabels {
    RadarLabels::None
}

/// The documented default of [`RadarMarker`]: the value a document that
/// omits it takes.
pub(crate) const fn radar_marker() -> RadarMarker {
    RadarMarker::Circle
}

/// The documented default of [`RadarSeriesLabels`]: the value a document that
/// omits it takes.
pub(crate) fn radar_series_labels() -> RadarSeriesLabels {
    RadarSeriesLabels {
        placement: RadarLabels::None,
        decimals: 1,
        value_suffix: String::new(),
        text: Vec::new(),
        offsets: Vec::new(),
    }
}

/// The documented default of [`RadarSeriesStyle`]: the value a document that
/// omits it takes.
pub(crate) fn radar_series_style() -> RadarSeriesStyle {
    RadarSeriesStyle {
        line_width_px: None,
        fill: None,
        marker: None,
        marker_size_px: None,
        fill_opacity: None,
    }
}

/// The documented default of [`RangeColor`]: the value a document that
/// omits it takes.
pub(crate) const fn range_color() -> RangeColor {
    RangeColor::Single
}

/// The documented default of [`SizeScale`]: the value a document that
/// omits it takes.
pub(crate) const fn size_scale() -> SizeScale {
    SizeScale::Area
}

/// The documented default of [`StreamlineArrows`]: the value a document that
/// omits it takes.
pub(crate) fn streamline_arrows() -> StreamlineArrows {
    StreamlineArrows {
        spacing_px: 80.0,
        head_length_px: 8.0,
        head_width_px: 7.0,
    }
}

/// The documented default of [`VectorPivot`]: the value a document that
/// omits it takes.
pub(crate) const fn vector_pivot() -> VectorPivot {
    VectorPivot::Tail
}

/// The documented default of [`VectorScale`]: the value a document that
/// omits it takes.
pub(crate) fn vector_scale() -> VectorScale {
    VectorScale::Pixels { px_per_unit: 1.0 }
}

/// The documented default of [`ViolinStyle`]: the value a document that
/// omits it takes.
pub(crate) fn violin_style() -> ViolinStyle {
    ViolinStyle {
        fill: None,
        outline: true,
        outline_width: 1.2,
        outline_color: Color::from_rgb8(32, 46, 60),
    }
}

/// [`Violin::box_width`] where a document omits it, and
/// [`Violin::DEFAULT_BOX_WIDTH`]: the box takes a quarter of the slot.
pub(crate) const fn violin_box_width() -> f32 {
    0.25
}

/// The histogram bar width ratio where a document omits it.
#[cfg(feature = "serde")]
pub(crate) fn histogram_bar_width_ratio() -> f32 {
    1.0
}

/// [`ContourLabelConfig::color`] where a document omits it.
#[cfg(feature = "serde")]
pub(crate) fn contour_label_color() -> Color {
    Color::BLACK
}

/// An image series' value range where a document omits it.
#[cfg(feature = "serde")]
pub(crate) fn image_range() -> [f64; 2] {
    [0.0, 1.0]
}

/// An image series' shading where a document omits it.
#[cfg(feature = "serde")]
pub(crate) fn image_shading() -> Shading {
    Shading::Flat
}

/// An image series' opacity where a document omits it.
#[cfg(feature = "serde")]
pub(crate) fn image_opacity() -> f32 {
    1.0
}
