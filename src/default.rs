//! Reasonable default builders for a figgy chart.
//!
//! The `Default` trait is intentionally not implemented anywhere in the crate;
//! values that carry user intent (titles, data ranges) must go through builders
//! like `Chart::with_title`. This module only provides visual / layout defaults
//! that work for any chart.
//!
//! Included: visual style (colors, widths, font sizes, tick lengths, grid),
//! axis visibility policy (top/right labels and titles off, bottom/left on),
//! units / formatting (Linear, Decimal, 3 sig digits), and margins.
//!
//! Not included (caller supplies): chart and axis text (empty segments —
//! filled in by builders), axis range (0..1 placeholder updated by
//! `Chart::set_*_range` or `auto_fit_*`), and chart_area pixel size (set to
//! match the host viewport).

use crate::color::Color;
use crate::config::{
    AxisOptions, AxisScale, AxisTitleOptions, Chart, ChartTitleOptions, ChartType, Config,
    GridOptions, LabelStyle, Legend, LegendCorner, TickVisibility,
};
use crate::format::LabelFormat;
use crate::layout::{ChartArea, Rect};
use crate::line::LineStylePreset;
use crate::text::RichText;

/// Empty RichText (no segments). Filled in by builders.
pub fn default_rich_text() -> RichText {
    RichText {
        segments: Vec::new(),
        color: Color::BLACK,
        font_size: 12.0,
        font: String::new(),
    }
}

/// Chart title style. Text starts empty and is filled in by a builder.
pub fn default_chart_title_options() -> ChartTitleOptions {
    ChartTitleOptions {
        text: RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 28.0,
            font: String::new(),
        },
        visible: true,
        offset_x: 0.0,
        offset_y: 0.0,
        // 28pt title text + a bit of breathing room above and below.
        top_margin: 32.0,
    }
}

/// Axis title style. Text starts empty and is filled in by a builder.
pub fn default_axis_title_options() -> AxisTitleOptions {
    AxisTitleOptions {
        text: RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 22.0,
            font: String::new(),
        },
        visible: true,
        offset_x: 0.0,
        offset_y: 0.0,
    }
}

/// Tick label style (X axis).
pub fn default_label_style_x() -> LabelStyle {
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

/// Tick label style (Y axis). Currently identical to X.
pub fn default_label_style_y() -> LabelStyle {
    default_label_style_x()
}

/// X axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_x_range` or `auto_fit_x`.
pub fn default_axis_options_x() -> AxisOptions {
    AxisOptions {
        scale: AxisScale::Linear,
        min: 0.0,
        max: 1.0,
        major_spacing: 0.2,
        minor_count: 4,
        inverted: false,
        label_style: default_label_style_x(),
        tick: TickVisibility::Inside,
        title_option: default_axis_title_options(),
        out_margin: 80.0,
        line_visible: true,
        line_color: Color::BLACK,
        line_width: 1.0,
        line_style: LineStylePreset::Solid,
        major_tick_length: 5.0,
        minor_tick_length: 3.0,
    }
}

/// Y axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_y_range` or `auto_fit_y`.
pub fn default_axis_options_y() -> AxisOptions {
    AxisOptions {
        scale: AxisScale::Linear,
        min: 0.0,
        max: 1.0,
        major_spacing: 0.2,
        minor_count: 4,
        inverted: false,
        label_style: default_label_style_y(),
        tick: TickVisibility::Inside,
        title_option: default_axis_title_options(),
        out_margin: 110.0,
        line_visible: true,
        line_color: Color::BLACK,
        line_width: 1.0,
        line_style: LineStylePreset::Solid,
        major_tick_length: 5.0,
        minor_tick_length: 3.0,
    }
}

/// Major grid on, minor grid off. Light gray lines.
pub fn default_grid_options() -> GridOptions {
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

/// Chart metadata. `chart_id` is a placeholder.
pub fn default_chart() -> Chart {
    Chart {
        chart_id: String::from("chart"),
        chart_type: ChartType::ScatterLine,
    }
}

/// Legend defaults — hidden, top-right corner, standard padding.
pub fn default_legend() -> Legend {
    Legend {
        visible: false,
        entries: Vec::new(),
        corner: LegendCorner::TopRight,
        padding: 8.0,
        font_size: 14.0,
        line_height: 20.0,
        sample_width: 24.0,
        sample_text_gap: 8.0,
        bg_color: Color { r: 1.0, g: 1.0, b: 1.0, a: 0.85 },
        border_color: Color { r: 0.6, g: 0.6, b: 0.6, a: 1.0 },
    }
}

/// chart_area placeholder — caller resizes to match the host viewport.
pub fn default_chart_area() -> ChartArea {
    ChartArea(Rect { x: 0, y: 0, width: 1000, height: 800 })
}

/// Reasonable default Config.
///
/// - `bottom_x` / `left_y`: axis line, ticks, labels, and title all on. Text
///   is empty — fill in via `Chart::with_x_title` / `with_y_title`.
/// - `top_x` / `right_y`: axis line only (kept for the frame); labels and
///   axis title off. Enable for special charts (e.g. dual-axis).
/// - `chart_title`: title band reserved; text empty until `Chart::with_title`.
pub fn default_config() -> Config {
    let mut top_x = default_axis_options_x();
    top_x.label_style.label_visible = false;
    top_x.title_option.visible = false;
    // Labels/title off → out_margin only needs to span the gap between the
    // axis line and the title band.
    top_x.out_margin = 8.0;

    let mut right_y = default_axis_options_y();
    right_y.label_style.label_visible = false;
    right_y.title_option.visible = false;
    right_y.out_margin = 8.0;

    Config {
        chart_area: default_chart_area(),
        chart: default_chart(),
        top_x,
        bottom_x: default_axis_options_x(),
        left_y: default_axis_options_y(),
        right_y,
        chart_title: default_chart_title_options(),
        grid: default_grid_options(),
        legend: default_legend(),
    }
}
