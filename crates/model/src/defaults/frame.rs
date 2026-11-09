//! Chart frame: the document, its axes, title, legend, grid and colour bar.

#![allow(unused_imports)]
// Some sources are read only by `Default` (`crate::helpers`).
#![cfg_attr(not(feature = "helpers"), allow(dead_code))]

use super::*;
use crate::config::*;
use crate::format::*;
use crate::layout::*;
use crate::line::*;
use crate::text::*;
use crate::*;

/// The documented default of [`AxisBreakStyle`]: the value a document that
/// omits it takes.
pub(crate) const fn axis_break_style() -> AxisBreakStyle {
    AxisBreakStyle::Continuous
}

/// The documented default of [`AxisBreaks`]: the value a document that
/// omits it takes.
pub(crate) fn axis_breaks() -> AxisBreaks {
    AxisBreaks {
        ranges: Vec::new(),
        style: axis_break_style(),
        gap_px: 8.0,
        mark: break_mark(),
        frame_each_panel: true,
        sizing: PieceSizing::ByValue,
    }
}

/// The documented default of [`AxisOptions`] (colorbar): the value a document that
/// omits it takes.
pub(crate) fn axis_options_colorbar() -> AxisOptions {
    let mut title_option = axis_title_options();
    // A z title is the exception, not the rule — most colourbars are labelled
    // by their tick values alone.
    title_option.visible = false;
    AxisOptions {
        scale: AxisScale::Linear,
        min: 0.0,
        max: 1.0,
        major_spacing: 0.2,
        minor_count: 4,
        inverted: false,
        label_style: label_style_y(),
        tick: TickVisibility::Outside,
        title_option,
        // Holds the tick labels only; no title band by default.
        out_margin: 60.0,
        line_offset: 0.0,
        line_visible: false,
        line_color: Color::BLACK,
        line_width: 1.0,
        line_style: LineStylePreset::Solid,
        major_tick_length: 5.0,
        minor_tick_length: 3.0,
        categories: None,
        category_groups: None,
        category_window: None,
        cross_at: None,
        log_ticks: LogTickPolicy::Decades,
        breaks: None,
    }
}

/// The documented default of [`AxisOptions`] (x): the value a document that
/// omits it takes.
pub(crate) fn axis_options_x() -> AxisOptions {
    AxisOptions {
        scale: AxisScale::Linear,
        min: 0.0,
        max: 1.0,
        major_spacing: 0.2,
        minor_count: 4,
        inverted: false,
        label_style: label_style_x(),
        tick: TickVisibility::Inside,
        title_option: axis_title_options(),
        out_margin: 80.0,
        line_offset: 0.0,
        line_visible: true,
        line_color: Color::BLACK,
        line_width: 1.0,
        line_style: LineStylePreset::Solid,
        major_tick_length: 5.0,
        minor_tick_length: 3.0,
        categories: None,
        category_groups: None,
        category_window: None,
        cross_at: None,
        log_ticks: LogTickPolicy::Decades,
        breaks: None,
    }
}

/// The documented default of [`AxisOptions`] (y): the value a document that
/// omits it takes.
pub(crate) fn axis_options_y() -> AxisOptions {
    AxisOptions {
        scale: AxisScale::Linear,
        min: 0.0,
        max: 1.0,
        major_spacing: 0.2,
        minor_count: 4,
        inverted: false,
        label_style: label_style_y(),
        tick: TickVisibility::Inside,
        title_option: axis_title_options(),
        out_margin: 110.0,
        line_offset: 0.0,
        line_visible: true,
        line_color: Color::BLACK,
        line_width: 1.0,
        line_style: LineStylePreset::Solid,
        major_tick_length: 5.0,
        minor_tick_length: 3.0,
        categories: None,
        category_groups: None,
        category_window: None,
        cross_at: None,
        log_ticks: LogTickPolicy::Decades,
        breaks: None,
    }
}

/// The documented default of [`AxisTitleOptions`]: the value a document that
/// omits it takes.
pub(crate) fn axis_title_options() -> AxisTitleOptions {
    AxisTitleOptions {
        text: RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 22.0,
            font: String::new(),
            align: TextAlign::Left,
        },
        visible: true,
        offset_x: 0.0,
        offset_y: 0.0,
    }
}

/// The documented default of [`BreakMark`]: the value a document that
/// omits it takes.
pub(crate) fn break_mark() -> BreakMark {
    BreakMark::Slash {
        size: 6.0,
        color: None,
        width: 1.0,
    }
}

/// The documented default of [`ChartArea`]: the value a document that
/// omits it takes.
pub(crate) fn chart_area() -> ChartArea {
    ChartArea(Rect {
        x: 0,
        y: 0,
        width: 1000,
        height: 800,
    })
}

/// The documented default of [`ChartTitleOptions`]: the value a document that
/// omits it takes.
pub(crate) fn chart_title_options() -> ChartTitleOptions {
    ChartTitleOptions {
        text: RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 28.0,
            font: String::new(),
            align: TextAlign::Left,
        },
        visible: true,
        offset_x: 0.0,
        offset_y: 0.0,
        // 28pt title text + a bit of breathing room above and below.
        top_margin: 32.0,
    }
}

/// The documented default of [`ColorBarOptions`]: the value a document that
/// omits it takes.
pub(crate) fn color_bar_options() -> ColorBarOptions {
    ColorBarOptions {
        visible: true,
        side: Side::Right,
        thickness_px: 18.0,
        // The axis band on the bar's side already separates it from the data
        // area; the right one holds the last bottom tick label.
        gap_px: 8.0,
        length_frac: 0.75,
        align: BarAlign::Center,
        offset_x: 0.0,
        offset_y: 0.0,
        colormap: ColorMap::Viridis,
        nan_color: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        border_color: Color::from_rgb8(80, 80, 80),
        border_width: 1.0,
        axis: axis_options_colorbar(),
    }
}

/// The documented default of [`Config`]: the value a document that
/// omits it takes.
pub(crate) fn config() -> Config {
    let mut top_x = axis_options_x();
    top_x.label_style.label_visible = false;
    top_x.title_option.visible = false;
    // Labels/title off → out_margin only needs to span the gap between the
    // axis line and the title band.
    top_x.out_margin = 8.0;

    let mut right_y = axis_options_y();
    right_y.label_style.label_visible = false;
    right_y.title_option.visible = false;
    // The last bottom tick label is centred on the data area's right edge, so
    // half of it lands in this margin: 32 holds a seven-character label at
    // the default font size.
    right_y.out_margin = 32.0;

    Config {
        chart_area: chart_area(),
        top_x,
        bottom_x: axis_options_x(),
        left_y: axis_options_y(),
        right_y,
        chart_title: chart_title_options(),
        grid: grid_options(),
        legend: legend(),
        picked_data: None,
        // No z dimension until a field series needs one — see
        // `default_colorbar_options`.
        colorbar: None,
        // Precise mode. Stylized modes are opt-in (`DrawStyle::Sketch(..)`).
        draw_style: DrawStyle::Precise,
        coordinates: crate::coordinates::Coordinates::Cartesian,
        overlays: Vec::new(),
        subplots: Vec::new(),
        active_layer: LayerRef::Base,
        annotations: AnnotationLayer::empty(),
        // Grouped vertical bars until a chart sets its own.
        bar_groups: None,
        // Every series drawn where its values are.
        line_offsets: None,
        plot_background: None,
        background: None,
    }
}

/// The documented default of [`ConstellationOptions`]: the value a document that
/// omits it takes.
pub(crate) fn constellation_options() -> ConstellationOptions {
    ConstellationOptions {
        star_opacity: 1.0,
        line_opacity: 0.45,
    }
}

/// The documented default of [`DataSelectionsConfig`]: the value a document that
/// omits it takes.
pub(crate) fn data_selections_config() -> DataSelectionsConfig {
    DataSelectionsConfig {
        visible: true,
        refs: Vec::new(),
        highlight_color: Color::from_rgb8(255, 215, 0),
        outline_width_px: 2.0,
        point_radius_extra_px: 3.0,
        contour_width_extra_px: 2.0,
    }
}

/// The documented default of [`DrawStyle`]: the value a document that
/// omits it takes.
pub(crate) const fn draw_style() -> DrawStyle {
    DrawStyle::Precise
}

/// The documented default of [`GridOptions`]: the value a document that
/// omits it takes.
pub(crate) fn grid_options() -> GridOptions {
    GridOptions {
        show_major_x: true,
        major_x_color: Color::from_rgb8(200, 200, 200),
        major_x_width: 1.0,
        major_x_style: LineStylePreset::Solid,

        show_major_y: true,
        major_y_color: Color::from_rgb8(200, 200, 200),
        major_y_width: 1.0,
        major_y_style: LineStylePreset::Solid,

        show_minor_x: false,
        minor_x_color: Color::from_rgb8(230, 230, 230),
        minor_x_width: 0.5,
        minor_x_style: LineStylePreset::Dot,

        show_minor_y: false,
        minor_y_color: Color::from_rgb8(230, 230, 230),
        minor_y_width: 0.5,
        minor_y_style: LineStylePreset::Dot,
    }
}

/// The documented default of [`LabelStyle`] (x): the value a document that
/// omits it takes.
pub(crate) fn label_style_x() -> LabelStyle {
    LabelStyle {
        visible: true,
        color: Color::BLACK,
        font_size: 18.0,
        label_visible: true,
        label_font: String::new(),
        label_offset_x: 0.0,
        label_offset_y: 0.0,
        format: LabelFormat::Decimal,
        significant_digits: 3,
    }
}

/// The documented default of [`LabelStyle`] (y): the value a document that
/// omits it takes.
pub(crate) fn label_style_y() -> LabelStyle {
    label_style_x()
}

/// The documented default of [`Legend`]: the value a document that
/// omits it takes.
pub(crate) fn legend() -> Legend {
    Legend {
        visible: false,
        content: RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 14.0,
            font: String::new(),
            align: TextAlign::Left,
        },
        corner: LegendCorner::TopRight,
        offset_x: 0.0,
        offset_y: 0.0,
        padding: 8.0,
        bg_color: Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.85,
        },
        border_color: Color {
            r: 0.6,
            g: 0.6,
            b: 0.6,
            a: 1.0,
        },
    }
}

/// The documented default of [`LogTickPolicy`]: the value a document that
/// omits it takes.
pub(crate) const fn log_tick_policy() -> LogTickPolicy {
    LogTickPolicy::Decades
}

/// The documented default of [`MilkywayOptions`]: the value a document that
/// omits it takes.
pub(crate) fn milkyway_options() -> MilkywayOptions {
    MilkywayOptions {
        star_density: 14.0,
        ribbon_width_px: 14.0,
        ribbon_intensity: 0.30,
        star_scale: 1.0,
        star_brightness: 1.0,
        spread_px: 2.5,
        structure_scale: 1.0,
        faint_bias: 3.0,
        glow: 0.55,
        nebula: 1.0,
        dust: 1.0,
        planet_rim: 0.34,
        seed: 0,
    }
}

/// The documented default of [`PieceSizing`]: the value a document that
/// omits it takes.
pub(crate) const fn piece_sizing() -> PieceSizing {
    PieceSizing::ByValue
}

/// The documented default of [`SketchOptions`]: the value a document that
/// omits it takes.
pub(crate) fn sketch_options() -> SketchOptions {
    SketchOptions {
        amplitude_px: 1.5,
        wavelength_px: 60.0,
        seed: 0,
    }
}
