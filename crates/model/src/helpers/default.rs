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

use crate::color::Color;
use crate::config::{AxisOptions, ColorBarOptions, Config, Legend, TickVisibility};
use crate::data_config::{BarGroups, CategoryBarDirection};
use crate::preset::AxisPreset;

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

/// Legend defaults — hidden, top-right corner, standard padding. Content
/// starts empty; its font/font_size are live and consumed at draw time.
pub fn default_legend() -> Legend {
    crate::defaults::legend()
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

/// The share of a row's band a timeline's bars cover: between Plotly's
/// (0.8, `bargap` 0.2) and Vega-Lite's (0.9, band padding 0.1).
const TIMELINE_GROUP_WIDTH: f32 = 0.8;

/// The light grey of a timeline's axis lines.
const TIMELINE_FRAME: Color = Color::from_rgb8(160, 166, 176);

/// The pale grey of a timeline's value gridlines.
const TIMELINE_GRID: Color = Color::from_rgb8(225, 228, 233);

/// A timeline (Gantt) chart in the look of the common web timelines
/// (#357), for [`crate::data_config::SeriesConfig::timeline_bars`]:
/// [`default_config`] with
///
/// - horizontal bars covering 0.8 of each row's band
///   ([`Config::bar_groups`]);
/// - the first row at the top: `left_y` is inverted, so its first category
///   (or the smallest position on a numeric axis) sits at the top;
/// - gridlines along the value axis only, in a pale grey, and no minor
///   gridlines;
/// - the open frame ([`AxisPreset::OpenOutward`]) with its axis lines in a
///   light grey and no tick marks: the gridlines carry the values.
///
/// Every choice is a stored field, so a saved timeline reads back with the
/// same look; the documented default ([`default_config`]) is unchanged.
pub fn default_timeline_config() -> Config {
    let mut config = default_config();
    config.apply_axis_preset(AxisPreset::OpenOutward);
    config.bar_groups = Some(BarGroups {
        direction: CategoryBarDirection::Horizontal,
        group_width: TIMELINE_GROUP_WIDTH,
        ..crate::defaults::bar_groups()
    });
    config.left_y.inverted = true;
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.tick = TickVisibility::None;
        axis.line_color = TIMELINE_FRAME;
    }
    let grid = &mut config.grid;
    grid.show_major_y = false;
    grid.show_minor_x = false;
    grid.show_minor_y = false;
    grid.major_x_color = TIMELINE_GRID;
    config
}
