//! Default builders for a figgy chart, for hosts (#266).
//!
//! Each function returns a type's documented default: the value its crate-private
//! `omitted()` source holds, the same value serde gives a field a document
//! omits. The module, and `Default` on the document types, exist only with the
//! `helpers` feature (on by default); the model and the renderer build without
//! it, so their own code cannot fall back on a default.
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
    DrawStyle, GridOptions, LabelStyle, Legend, LegendCorner, LogTickPolicy, TickVisibility,
};
use crate::format::LabelFormat;
use crate::layer::{AnnotationLayer, LayerRef};
use crate::layout::{ChartArea, Rect, Side};
use crate::line::LineStylePreset;
use crate::text::{RichText, TextAlign};

/// Chart title style. Text starts empty and is filled in by a builder.
impl ChartTitleOptions {
    /// The value this type takes in a new document: the one source of
    /// [`default_chart_title_options`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
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
}

/// Chart title style. Text starts empty and is filled in by a builder.
#[cfg(feature = "helpers")]
pub fn default_chart_title_options() -> ChartTitleOptions {
    ChartTitleOptions::omitted()
}

/// Axis title style. Text starts empty and is filled in by a builder.
impl AxisTitleOptions {
    /// The value this type takes in a new document: the one source of
    /// [`default_axis_title_options`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
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
}

/// Axis title style. Text starts empty and is filled in by a builder.
#[cfg(feature = "helpers")]
pub fn default_axis_title_options() -> AxisTitleOptions {
    AxisTitleOptions::omitted()
}

/// Tick label style (X axis).
impl LabelStyle {
    /// The value this type takes in a new document: the one source of
    /// [`default_label_style_x`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted_x() -> Self {
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
}

/// Tick label style (X axis).
#[cfg(feature = "helpers")]
pub fn default_label_style_x() -> LabelStyle {
    LabelStyle::omitted_x()
}

/// Tick label style (Y axis). Currently identical to X.
impl LabelStyle {
    /// The value this type takes in a new document: the one source of
    /// [`default_label_style_y`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted_y() -> Self {
        LabelStyle::omitted_x()
    }
}

/// Tick label style (Y axis). Currently identical to X.
#[cfg(feature = "helpers")]
pub fn default_label_style_y() -> LabelStyle {
    LabelStyle::omitted_y()
}

/// X axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_x_range` or `auto_fit_x`.
impl AxisOptions {
    /// The value this type takes in a new document: the one source of
    /// [`default_axis_options_x`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted_x() -> Self {
        AxisOptions {
            scale: AxisScale::Linear,
            min: 0.0,
            max: 1.0,
            major_spacing: 0.2,
            minor_count: 4,
            inverted: false,
            label_style: LabelStyle::omitted_x(),
            tick: TickVisibility::Inside,
            title_option: AxisTitleOptions::omitted(),
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
}

/// X axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_x_range` or `auto_fit_x`.
#[cfg(feature = "helpers")]
pub fn default_axis_options_x() -> AxisOptions {
    AxisOptions::omitted_x()
}

/// Y axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_y_range` or `auto_fit_y`.
impl AxisOptions {
    /// The value this type takes in a new document: the one source of
    /// [`default_axis_options_y`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted_y() -> Self {
        AxisOptions {
            scale: AxisScale::Linear,
            min: 0.0,
            max: 1.0,
            major_spacing: 0.2,
            minor_count: 4,
            inverted: false,
            label_style: LabelStyle::omitted_y(),
            tick: TickVisibility::Inside,
            title_option: AxisTitleOptions::omitted(),
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
}

/// Y axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_y_range` or `auto_fit_y`.
#[cfg(feature = "helpers")]
pub fn default_axis_options_y() -> AxisOptions {
    AxisOptions::omitted_y()
}

/// The colourbar's z axis. Range is a 0..1 placeholder — replace with the
/// matrix's z statistics.
///
/// Differs from the chart axes in two places, both because the strip is not a
/// data area: `line_visible` is off (the strip's own border draws that edge, and
/// a second line on top of it is just a thicker border), and ticks point
/// `Outside` so they sit in the label margin instead of over the colours.
impl AxisOptions {
    /// The value this type takes in a new document: the one source of
    /// [`default_axis_options_colorbar`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted_colorbar() -> Self {
        let mut title_option = AxisTitleOptions::omitted();
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
            label_style: LabelStyle::omitted_y(),
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
}

/// The colourbar's z axis. Range is a 0..1 placeholder — replace with the
/// matrix's z statistics.
///
/// Differs from the chart axes in two places, both because the strip is not a
/// data area: `line_visible` is off (the strip's own border draws that edge, and
/// a second line on top of it is just a thicker border), and ticks point
/// `Outside` so they sit in the label margin instead of over the colours.
#[cfg(feature = "helpers")]
pub fn default_axis_options_colorbar() -> AxisOptions {
    AxisOptions::omitted_colorbar()
}

/// Colourbar defaults — visible, on the right, three quarters of the data
/// area's height, Viridis. `nan_color` is fully transparent: a value the ramp
/// cannot place reads as absent rather than as some particular colour.
///
/// Not `Default::default()` on purpose, following this module: `axis.min` /
/// `axis.max` carry user (or data) intent and the placeholder range is only
/// meaningful as a starting point a caller replaces.
impl ColorBarOptions {
    /// The value this type takes in a new document: the one source of
    /// [`default_colorbar_options`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
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
            axis: AxisOptions::omitted_colorbar(),
        }
    }
}

/// Colourbar defaults — visible, on the right, three quarters of the data
/// area's height, Viridis. `nan_color` is fully transparent: a value the ramp
/// cannot place reads as absent rather than as some particular colour.
///
/// Not `Default::default()` on purpose, following this module: `axis.min` /
/// `axis.max` carry user (or data) intent and the placeholder range is only
/// meaningful as a starting point a caller replaces.
#[cfg(feature = "helpers")]
pub fn default_colorbar_options() -> ColorBarOptions {
    ColorBarOptions::omitted()
}

/// Major grid on, minor grid off. Light gray lines.
impl GridOptions {
    /// The value this type takes in a new document: the one source of
    /// [`default_grid_options`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
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
}

/// Major grid on, minor grid off. Light gray lines.
#[cfg(feature = "helpers")]
pub fn default_grid_options() -> GridOptions {
    GridOptions::omitted()
}

/// Legend defaults — hidden, top-right corner, standard padding. Content
/// starts empty; its font/font_size are live and consumed at draw time.
impl Legend {
    /// The value this type takes in a new document: the one source of
    /// [`default_legend`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
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
}

/// Legend defaults — hidden, top-right corner, standard padding. Content
/// starts empty; its font/font_size are live and consumed at draw time.
#[cfg(feature = "helpers")]
pub fn default_legend() -> Legend {
    Legend::omitted()
}

/// chart_area placeholder — caller resizes to match the host viewport.
impl ChartArea {
    /// The value this type takes in a new document: the one source of
    /// [`default_chart_area`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
        ChartArea(Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        })
    }
}

/// chart_area placeholder — caller resizes to match the host viewport.
#[cfg(feature = "helpers")]
pub fn default_chart_area() -> ChartArea {
    ChartArea::omitted()
}

/// Reasonable default Config.
///
/// - `bottom_x` / `left_y`: axis line, ticks, labels, and title all on. Text
///   is empty — fill in via `Chart::with_x_title` / `with_y_title`.
/// - `top_x` / `right_y`: axis line and inside ticks on (kept for the frame);
///   labels and axis title off. Enable those for special charts (e.g. dual-axis).
/// - `chart_title`: title band reserved; text empty until `Chart::with_title`.
impl Config {
    /// The value this type takes in a new document: the one source of
    /// [`default_config`] (#266).
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
        let mut top_x = AxisOptions::omitted_x();
        top_x.label_style.label_visible = false;
        top_x.title_option.visible = false;
        // Labels/title off → out_margin only needs to span the gap between the
        // axis line and the title band.
        top_x.out_margin = 8.0;

        let mut right_y = AxisOptions::omitted_y();
        right_y.label_style.label_visible = false;
        right_y.title_option.visible = false;
        // The last bottom tick label is centred on the data area's right edge, so
        // half of it lands in this margin: 32 holds a seven-character label at
        // the default font size.
        right_y.out_margin = 32.0;

        Config {
            chart_area: ChartArea::omitted(),
            top_x,
            bottom_x: AxisOptions::omitted_x(),
            left_y: AxisOptions::omitted_y(),
            right_y,
            chart_title: ChartTitleOptions::omitted(),
            grid: GridOptions::omitted(),
            legend: Legend::omitted(),
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
        }
    }
}

/// Reasonable default Config.
///
/// - `bottom_x` / `left_y`: axis line, ticks, labels, and title all on. Text
///   is empty — fill in via `Chart::with_x_title` / `with_y_title`.
/// - `top_x` / `right_y`: axis line and inside ticks on (kept for the frame);
///   labels and axis title off. Enable those for special charts (e.g. dual-axis).
/// - `chart_title`: title band reserved; text empty until `Chart::with_title`.
#[cfg(feature = "helpers")]
pub fn default_config() -> Config {
    Config::omitted()
}
