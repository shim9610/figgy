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

use crate::Color;
use crate::config::{AxisOptions, ColorBarOptions, Config, Legend, TickVisibility};
use crate::parallel::ParallelOptions;

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

/// A parallel coordinates chart's chrome (#357), in the layout web
/// parallel-coordinate charts share: the axis names above the axes, light
/// axes with outward major ticks, and each tick label on a white box so it
/// stays readable over the rows.
///
/// - `parallel`: no axes yet, `names_on_top`, a `label_backdrop` of white
///   at 0.8. The host writes the axes (`parallel_mut().axes`) and the
///   names (`bottom_x.categories`), one per axis, before the chart is
///   registered.
/// - `left_y`, which every parallel axis is drawn in the style of: the
///   normalized band `0..1`, title hidden, grey line and outward major
///   ticks without minor ticks, 14 px tick labels.
/// - `bottom_x`, which names the axes: no line, no ticks.
/// - No grid, and no frame: `top_x` and `right_y` draw no line and no
///   ticks.
///
/// Every choice is a stored field, so a saved chart reads back with the
/// same look.
pub fn default_parallel_config() -> Config {
    let mut config = default_config();
    let axis_ink = Color::from_rgb8(110, 116, 128);
    let y = &mut config.left_y;
    y.min = 0.0;
    y.max = 1.0;
    y.title_option.visible = false;
    y.minor_count = 0;
    y.tick = TickVisibility::Outside;
    y.line_color = axis_ink;
    y.label_style.font_size = 14.0;
    let names = &mut config.bottom_x;
    names.line_visible = false;
    names.tick = TickVisibility::None;
    names.minor_count = 0;
    for frame in [&mut config.top_x, &mut config.right_y] {
        frame.line_visible = false;
        frame.tick = TickVisibility::None;
        frame.minor_count = 0;
    }
    let grid = &mut config.grid;
    grid.show_major_x = false;
    grid.show_major_y = false;
    grid.show_minor_x = false;
    grid.show_minor_y = false;
    config.set_parallel(Some(ParallelOptions {
        axes: Vec::new(),
        names_on_top: true,
        label_backdrop: Some(Color {
            a: 0.8,
            ..Color::WHITE
        }),
    }));
    config
}
