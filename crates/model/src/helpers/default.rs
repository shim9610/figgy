//! Default builders for a figgy chart, for hosts and examples (#266).
//!
//! Each function returns a type's documented default from its one
//! crate-private source in `crate::defaults`: the same value serde gives a
//! field a document omits. This module, and `Default` on the document types,
//! exist only with the `helpers` feature (on by default); the model and the
//! renderer build without it, so their own code cannot fall back on a default.
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

use crate::config::{
    AxisOptions, AxisTitleOptions, ChartTitleOptions, ColorBarOptions, Config, GridOptions,
    LabelStyle, Legend,
};
use crate::layout::ChartArea;

/// Chart title style. Text starts empty and is filled in by a builder.
pub fn default_chart_title_options() -> ChartTitleOptions {
    crate::defaults::chart_title_options()
}

/// Axis title style. Text starts empty and is filled in by a builder.
pub fn default_axis_title_options() -> AxisTitleOptions {
    crate::defaults::axis_title_options()
}

/// Tick label style (X axis).
pub fn default_label_style_x() -> LabelStyle {
    crate::defaults::label_style_x()
}

/// Tick label style (Y axis). Currently identical to X.
pub fn default_label_style_y() -> LabelStyle {
    crate::defaults::label_style_y()
}

/// X axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_x_range` or `auto_fit_x`.
pub fn default_axis_options_x() -> AxisOptions {
    crate::defaults::axis_options_x()
}

/// Y axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_y_range` or `auto_fit_y`.
pub fn default_axis_options_y() -> AxisOptions {
    crate::defaults::axis_options_y()
}

/// The colourbar's z axis. Range is a 0..1 placeholder — replace with the
/// matrix's z statistics.
///
/// Differs from the chart axes in two places, both because the strip is not a
/// data area: `line_visible` is off (the strip's own border draws that edge, and
/// a second line on top of it is just a thicker border), and ticks point
/// `Outside` so they sit in the label margin instead of over the colours.
pub fn default_axis_options_colorbar() -> AxisOptions {
    crate::defaults::axis_options_colorbar()
}

/// Colourbar defaults — visible, on the right, three quarters of the data
/// area's height, Viridis. `nan_color` is fully transparent: a value the ramp
/// cannot place reads as absent rather than as some particular colour.
///
/// Not `Default::default()` on purpose, following this module: `axis.min` /
/// `axis.max` carry user (or data) intent and the placeholder range is only
/// meaningful as a starting point a caller replaces.
pub fn default_colorbar_options() -> ColorBarOptions {
    crate::defaults::color_bar_options()
}

/// Major grid on, minor grid off. Light gray lines.
pub fn default_grid_options() -> GridOptions {
    crate::defaults::grid_options()
}

/// Legend defaults — hidden, top-right corner, standard padding. Content
/// starts empty; its font/font_size are live and consumed at draw time.
pub fn default_legend() -> Legend {
    crate::defaults::legend()
}

/// chart_area placeholder — caller resizes to match the host viewport.
pub fn default_chart_area() -> ChartArea {
    crate::defaults::chart_area()
}

/// Reasonable default Config.
///
/// - `bottom_x` / `left_y`: axis line, ticks, labels, and title all on. Text
///   is empty — fill in via `Chart::with_x_title` / `with_y_title`.
/// - `top_x` / `right_y`: axis line and inside ticks on (kept for the frame);
///   labels and axis title off. Enable those for special charts (e.g. dual-axis).
/// - `chart_title`: title band reserved; text empty until `Chart::with_title`.
pub fn default_config() -> Config {
    crate::defaults::config()
}
