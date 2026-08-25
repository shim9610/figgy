//! Reasonable default builders for a figgy chart.
//!
//! The `Default` trait is intentionally not implemented for the chart types;
//! values that carry user intent (titles, data ranges) must go through builders
//! like `Chart::with_title`. This module only provides visual / layout defaults
//! that work for any chart. (Sole exceptions: `DrawStyle` / `SketchOptions` —
//! pure visual parameters with no user intent, and `serde(default)` requires
//! the impls.)
//!
//! Included: visual style (colors, widths, font sizes, tick lengths, grid),
//! axis visibility policy (all axis lines and inside ticks on; top/right
//! labels and titles off, bottom/left labels and titles on), units / formatting
//! (Linear, Decimal, 3 sig digits), and margins.
//!
//! Not included (caller supplies): chart and axis text (empty segments —
//! filled in by builders), axis range (0..1 placeholder updated by
//! `Chart::set_*_range` or `auto_fit_*`), and chart_area pixel size (set to
//! match the host viewport).

use crate::color::Color;
use crate::colormap::ColorMap;
use crate::config::{
    AxisOptions, AxisScale, AxisTitleOptions, BarAlign, ChartTitleOptions, ColorBarOptions, Config,
    DrawStyle, GridOptions, LabelStyle, Legend, LegendCorner, TickVisibility,
};
use crate::format::LabelFormat;
use crate::layout::{ChartArea, Rect, Side};
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
        line_offset: 0.0,
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
        line_offset: 0.0,
        line_visible: true,
        line_color: Color::BLACK,
        line_width: 1.0,
        line_style: LineStylePreset::Solid,
        major_tick_length: 5.0,
        minor_tick_length: 3.0,
    }
}

/// The colourbar's z axis. Range is a 0..1 placeholder — replace with the
/// matrix's z statistics.
///
/// Differs from the chart axes in two places, both because the strip is not a
/// data area: `line_visible` is off (the strip's own border draws that edge, and
/// a second line on top of it is just a thicker border), and ticks point
/// `Outside` so they sit in the label margin instead of over the colours.
pub fn default_axis_options_colorbar() -> AxisOptions {
    let mut title_option = default_axis_title_options();
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
        label_style: default_label_style_y(),
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
    }
}

/// Colourbar defaults — visible, on the right, three quarters of the data
/// area's height, Viridis. `nan_color` is fully transparent: a value the ramp
/// cannot place reads as absent rather than as some particular colour.
///
/// Not `Default::default()` on purpose, following this module: `axis.min` /
/// `axis.max` carry user (or data) intent and the placeholder range is only
/// meaningful as a starting point a caller replaces.
pub fn default_colorbar_options() -> ColorBarOptions {
    ColorBarOptions {
        visible: true,
        side: Side::Right,
        thickness_px: 18.0,
        gap_px: 24.0,
        length_frac: 0.75,
        align: BarAlign::Center,
        offset_x: 0.0,
        offset_y: 0.0,
        colormap: ColorMap::Viridis,
        nan_color: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        border_color: Color::from_rgb8(80, 80, 80),
        border_width: 1.0,
        axis: default_axis_options_colorbar(),
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

/// Legend defaults — hidden, top-right corner, standard padding. Content
/// starts empty; its font/font_size are live and consumed at draw time.
pub fn default_legend() -> Legend {
    Legend {
        visible: false,
        content: RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 14.0,
            font: String::new(),
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

/// chart_area placeholder — caller resizes to match the host viewport.
pub fn default_chart_area() -> ChartArea {
    ChartArea(Rect {
        x: 0,
        y: 0,
        width: 1000,
        height: 800,
    })
}

/// Reasonable default Config.
///
/// - `bottom_x` / `left_y`: axis line, ticks, labels, and title all on. Text
///   is empty — fill in via `Chart::with_x_title` / `with_y_title`.
/// - `top_x` / `right_y`: axis line and inside ticks on (kept for the frame);
///   labels and axis title off. Enable those for special charts (e.g. dual-axis).
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
        top_x,
        bottom_x: default_axis_options_x(),
        left_y: default_axis_options_y(),
        right_y,
        chart_title: default_chart_title_options(),
        grid: default_grid_options(),
        legend: default_legend(),
        picked_points: None,
        picked_data: None,
        // No z dimension until a field series needs one — see
        // `default_colorbar_options`.
        colorbar: None,
        // Precise mode. Stylized modes are opt-in (`DrawStyle::Sketch(..)`).
        draw_style: DrawStyle::Precise,
    }
}
