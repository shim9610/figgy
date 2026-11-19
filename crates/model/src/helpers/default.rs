//! Default builders for a figgy chart, for hosts and examples (#266).
//!
//! Each function returns a type's documented default from its one
//! crate-private source in `crate::defaults`: the same value serde gives a
//! field a document omits, except the tick label format of the axes and
//! the colourbar, which is [`LabelFormat::Auto`] here and stays
//! `LabelFormat::Decimal` for a document (#357). This module, and `Default` on the document types,
//! exist only with the `helpers` feature (on by default); the model and the
//! renderer build without it, so their own code cannot fall back on a default.
//!
//! Included: visual style (colors, widths, font sizes, tick lengths, grid),
//! axis visibility policy (all axis lines and inside ticks on; top/right
//! labels and titles off, bottom/left labels and titles on), units / formatting
//! (Linear, `LabelFormat::Auto`, 3 sig digits), and margins.
//!
//! Not included (caller supplies): chart and axis text (empty segments —
//! filled in by builders), axis range (0..1 placeholder updated by
//! `Chart::set_*_range` or `auto_fit_*`), and chart_area pixel size (set to
//! match the host viewport).

use crate::config::{AxisOptions, ColorBarOptions, Config, Legend};
use crate::format::{AutoNumberFormat, LabelFormat};

/// The tick label format the helpers give a new chart's axes and colourbar
/// (#357): clean numbers with the decimals the tick step needs (`0.8`,
/// `-3`, `10`), and an exponent form for very large or small ranges.
/// A document that omits nothing keeps the format it stores.
pub fn default_label_format() -> LabelFormat {
    LabelFormat::Auto(AutoNumberFormat::DEFAULT)
}

/// `axis` with the helpers' tick label format.
pub(crate) fn with_default_label_format(mut axis: AxisOptions) -> AxisOptions {
    axis.label_style.format = default_label_format();
    axis
}

/// `config` with the helpers' tick label format on its four axes and its
/// colourbar.
pub(crate) fn config_with_default_label_format(mut config: Config) -> Config {
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.label_style.format = default_label_format();
    }
    if let Some(bar) = config.colorbar.as_mut() {
        bar.axis.label_style.format = default_label_format();
    }
    config
}

/// X axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_x_range` or `auto_fit_x`.
pub fn default_axis_options_x() -> AxisOptions {
    with_default_label_format(crate::defaults::axis_options_x())
}

/// Y axis options. Range is a 0..1 placeholder — replace with
/// `Chart::set_y_range` or `auto_fit_y`.
pub fn default_axis_options_y() -> AxisOptions {
    with_default_label_format(crate::defaults::axis_options_y())
}

/// Colourbar defaults — visible, on the right, three quarters of the data
/// area's height, Viridis. `nan_color` is fully transparent: a value the ramp
/// cannot place reads as absent rather than as some particular colour.
///
/// Not `Default::default()` on purpose, following this module: `axis.min` /
/// `axis.max` carry user (or data) intent and the placeholder range is only
/// meaningful as a starting point a caller replaces.
pub fn default_colorbar_options() -> ColorBarOptions {
    let mut bar = crate::defaults::color_bar_options();
    bar.axis = with_default_label_format(bar.axis);
    bar
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
    config_with_default_label_format(crate::defaults::config())
}
