//! Chart families: radial, radar, polar, ternary, parallel, 3D and the bounded documents.

#![allow(unused_imports)]
// The deprecated document readers keep their defaults until they go.
#![allow(deprecated)]
// Some sources are read only by `Default` (`crate::helpers`).
#![cfg_attr(not(feature = "helpers"), allow(dead_code))]

use super::*;
use crate::boxplot::*;
use crate::categorical::*;
use crate::format::*;
use crate::layout::*;
use crate::line::*;
use crate::parallel::*;
use crate::polar::*;
use crate::radar::*;
use crate::radial::*;
use crate::ternary::*;
use crate::text::*;
use crate::view3d::*;
use crate::*;

/// The documented default of [`AngleUnit`]: the value a document that
/// omits it takes.
pub(crate) const fn angle_unit() -> AngleUnit {
    AngleUnit::Degrees
}

/// The documented default of [`BoxPlotChart`]: the value a document that
/// omits it takes.
pub(crate) fn box_plot_chart() -> BoxPlotChart {
    BoxPlotChart {
        decorations: decoration_layout(),
        title: String::new(),
        chart_title: None,
        active_layer: layer_ref(),
        annotations: annotation_layer(),
        categories: vec![],
        series: vec![],
        direction: BoxPlotDirection::Vertical,
        style: box_plot_style(),
        overrides: vec![],
        hovered: None,
        selected: None,
        labels: BoxPlotLabels::None,
        label_decimals: 1,
        value_suffix: String::new(),
        value_title: String::new(),
        category_title: String::new(),
        value_range: None,
        grid: true,
        legend: true,
        group_width: 0.65,
        box_gap: 8.0,
        font_family: "sans-serif".into(),
        font_size: 14.0,
        text_color: Color::from_rgb8(32, 46, 60),
        background: Color::WHITE,
    }
}

/// The documented default of [`BoxPlotDirection`]: the value a document that
/// omits it takes.
pub(crate) const fn box_plot_direction() -> BoxPlotDirection {
    BoxPlotDirection::Vertical
}

/// The documented default of [`CategoricalChart`]: the value a document that
/// omits it takes.
pub(crate) fn categorical_chart() -> CategoricalChart {
    CategoricalChart {
        decorations: decoration_layout(),
        title: String::new(),
        chart_title: None,
        active_layer: layer_ref(),
        annotations: annotation_layer(),
        categories: vec![],
        series: vec![],
        mode: CategoryBarMode::Grouped,
        direction: CategoryBarDirection::Vertical,
        style: category_bar_style(),
        overrides: vec![],
        hovered: None,
        selected: None,
        labels: CategoryBarLabels::Auto,
        label_format: CategoryBarLabelFormat::Value,
        label_decimals: 1,
        value_suffix: String::new(),
        value_title: String::new(),
        category_title: String::new(),
        grid: true,
        legend: true,
        group_width: 0.7,
        bar_gap: 3.0,
        font_family: "sans-serif".into(),
        font_size: 14.0,
        text_color: Color::from_rgb8(35, 45, 65),
        background: Color::WHITE,
    }
}

/// The documented default of [`GridShape`]: the value a document that
/// omits it takes.
pub(crate) const fn grid_shape() -> GridShape {
    GridShape::Circle
}

/// The documented default of [`ParallelOptions`]: the value a document that
/// omits it takes.
pub(crate) fn parallel_options() -> ParallelOptions {
    ParallelOptions { axes: Vec::new() }
}

/// The documented default of [`PolarInterpolation`]: the value a document that
/// omits it takes.
pub(crate) const fn polar_interpolation() -> PolarInterpolation {
    PolarInterpolation::Chord
}

/// The documented default of [`PolarOptions`]: the value a document that
/// omits it takes.
pub(crate) fn polar_options() -> PolarOptions {
    PolarOptions {
        zero_angle_degrees: 0.0,
        clockwise: false,
        angle_unit: AngleUnit::Degrees,
        sector: None,
        inner_radius_fraction: 0.0,
        interpolation: PolarInterpolation::Chord,
        radial_label_angle_degrees: 90.0,
        grid_shape: None,
        hide_first_radius_label: false,
    }
}

/// The documented default of [`Projection3d`]: the value a document that
/// omits it takes.
pub(crate) const fn projection3d() -> Projection3d {
    Projection3d::Orthographic
}

/// The documented default of [`RadarChart`]: the value a document that
/// omits it takes.
pub(crate) fn radar_chart() -> RadarChart {
    RadarChart {
        decorations: decoration_layout(),
        title: String::new(),
        chart_title: None,
        active_layer: layer_ref(),
        annotations: annotation_layer(),
        categories: vec![],
        series: vec![],
        style: radar_style(),
        hovered: None,
        selected: None,
        labels: RadarLabels::None,
        label_decimals: 1,
        value_suffix: String::new(),
        value_range: None,
        ring_spacing: None,
        grid: true,
        legend: true,
        font_family: "sans-serif".into(),
        font_size: 14.0,
        text_color: Color::from_rgb8(32, 46, 60),
        background: Color::WHITE,
    }
}

/// The documented default of [`RadarGridShape`]: the value a document that
/// omits it takes.
pub(crate) const fn radar_grid_shape() -> RadarGridShape {
    RadarGridShape::Polygon
}

/// The documented default of [`RadarStyle`]: the value a document that
/// omits it takes.
pub(crate) fn radar_style() -> RadarStyle {
    RadarStyle {
        start_angle_degrees: -90.0,
        clockwise: true,
        grid_shape: RadarGridShape::Polygon,
        spoke_color: Color::from_rgb8(200, 206, 214),
        ring_color: Color::from_rgb8(224, 229, 235),
        line_width_px: RadarSeriesStyle::DEFAULT_LINE_WIDTH_PX,
        fill_opacity: RadarSeriesStyle::DEFAULT_FILL_OPACITY,
        marker: RadarSeriesStyle::DEFAULT_MARKER,
        marker_size_px: RadarSeriesStyle::DEFAULT_MARKER_SIZE_PX,
        emphasis_brightness: 0.08,
    }
}

/// The documented default of [`RadialChart`]: the value a document that
/// omits it takes.
pub(crate) fn radial_chart() -> RadialChart {
    RadialChart {
        decorations: decoration_layout(),
        chart_title: None,
        active_layer: layer_ref(),
        annotations: annotation_layer(),
        slices: Vec::new(),
        kind: RadialKind::Pie,
        start_angle_degrees: -90.0,
        split: None,
        style: radial_style(),
        labels: RadialLabels::Outside,
        label_format: RadialLabelFormat::NamePercent,
        label_decimals: 1,
        value_suffix: String::new(),
        interaction: radial_interaction(),
        font_family: "sans-serif".into(),
        font_size: 16.0,
        label_color: Color::new(0.12, 0.16, 0.22, 1.0),
        background: Color::WHITE,
    }
}

/// The documented default of [`RadialInteraction`]: the value a document that
/// omits it takes.
pub(crate) fn radial_interaction() -> RadialInteraction {
    RadialInteraction {
        hovered: None,
        selected: None,
        hover_progress: 0.0,
    }
}

/// The documented default of [`RadialLabelFormat`]: the value a document that
/// omits it takes.
pub(crate) const fn radial_label_format() -> RadialLabelFormat {
    RadialLabelFormat::NamePercent
}

/// The documented default of [`RadialOutline`]: the value a document that
/// omits it takes.
pub(crate) fn radial_outline() -> RadialOutline {
    RadialOutline {
        rim: false,
        separators: false,
        emphasis: false,
        width: 1.0,
        color: Color::new(0.08, 0.12, 0.2, 1.0),
    }
}

/// The documented default of [`RadialStyle`]: the value a document that
/// omits it takes.
pub(crate) fn radial_style() -> RadialStyle {
    RadialStyle {
        material: Material::Flat,
        tilt_degrees: 0.0,
        depth: 0.0,
        bevel: 0.015,
        gap_degrees: 0.0,
        roughness: 0.35,
        texture_strength: 0.2,
        texture_scale: 1.0,
        light: [-0.5, -0.6, 1.0],
        shadow: false,
        inner_corner: 0.0,
        outer_corner: 0.0,
        outline: radial_outline(),
        hover_lift: 0.08,
        hover_brightness: 0.04,
        gloss: 0.55,
        texture_angle_degrees: 0.0,
    }
}

/// The documented default of [`SurfaceStyle`]: the value a document that
/// omits it takes.
pub(crate) fn surface_style() -> SurfaceStyle {
    SurfaceStyle {
        color_matrix: None,
        wireframe: None,
        material: Material::Flat,
        gloss: 0.45,
        texture_strength: 0.2,
        texture_scale: 1.0,
        emphasis_brightness: 0.12,
    }
}

/// The documented default of [`TernaryOptions`]: the value a document that
/// omits it takes.
pub(crate) fn ternary_options() -> TernaryOptions {
    TernaryOptions {}
}

/// The documented default of [`TernaryRegionStyle`]: the value a document that
/// omits it takes.
pub(crate) fn ternary_region_style() -> TernaryRegionStyle {
    TernaryRegionStyle {
        color: Color::new(0.3, 0.5, 0.8, 1.0),
        opacity: 0.35,
    }
}

/// The documented default of [`View3d`]: the value a document that
/// omits it takes.
pub(crate) fn view3d() -> View3d {
    View3d {
        azimuth_degrees: -60.0,
        elevation_degrees: 30.0,
        distance: REFERENCE_DISTANCE,
        projection: Projection3d::Orthographic,
        box_aspect: None,
    }
}

/// [`RadialChart::label_decimals`] where a document omits it.
#[cfg(feature = "serde")]
pub(crate) fn radial_label_decimals() -> usize {
    1
}

/// [`ParallelAxis::scale`] where a document omits it.
#[cfg(feature = "serde")]
pub(crate) fn parallel_axis_scale() -> crate::config::AxisScale {
    crate::config::AxisScale::Linear
}
