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
use crate::config::{AxisOptions, ColorBarOptions, Config, LayoutState, Legend, TickVisibility};
use crate::data_config::SeriesConfig;
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

/// The default config for a chart of `series`: [`default_config`] in the
/// frame its family draws by default (#357). A chart that draws a Feynman
/// series is drawn without axes ([`crate::preset::AxisPreset::Hidden`]),
/// one that draws an energy-level series in the Grotrian frame
/// ([`crate::preset::AxisPreset::Grotrian`]); every other chart, and a
/// chart without series, gets [`default_config`] unchanged. The first
/// Feynman or energy-level series decides, as a theme's frame does
/// (`FeynmanTheme::axis_preset`, `EnergyLevelTheme::axis_preset` of the
/// documented look).
///
/// The frame is written into the returned config, a plain document: it is
/// chosen once, where the chart is made, never at draw time, so a host
/// that turns the axes back on (`Config::apply_axis_preset`) keeps them.
pub fn config_for_series(series: &[SeriesConfig]) -> Config {
    let mut config = default_config();
    let frame = crate::helpers::theme::Theme::default().family_frame(
        &config,
        series,
        crate::layer::LayerRef::Base,
    );
    if let Some(preset) = frame {
        config.apply_axis_preset(preset);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::AxisPreset;

    fn framed(preset: AxisPreset) -> Config {
        let mut config = default_config();
        config.apply_axis_preset(preset);
        config
    }

    fn feynman() -> SeriesConfig {
        SeriesConfig::feynman("d", crate::defaults::feynman_series())
    }

    fn levels() -> SeriesConfig {
        SeriesConfig::energy_levels("e", crate::defaults::energy_level_series())
    }

    #[test]
    fn a_chart_takes_the_frame_of_its_family() {
        assert_eq!(config_for_series(&[feynman()]), framed(AxisPreset::Hidden));
        assert_eq!(config_for_series(&[levels()]), framed(AxisPreset::Grotrian));
        assert_eq!(config_for_series(&[]), default_config());
        let line = SeriesConfig::line("l", "x", "y");
        assert_eq!(config_for_series(&[line.clone()]), default_config());
        // The first framed family decides, as a theme's frame does.
        assert_eq!(
            config_for_series(&[line, feynman(), levels()]),
            framed(AxisPreset::Hidden)
        );
        let hidden = config_for_series(&[feynman()]);
        assert!(!hidden.bottom_x.line_visible && !hidden.left_y.line_visible);
        assert!(!hidden.grid.show_major_x && !hidden.grid.show_major_y);
    }

    #[test]
    fn the_axes_turn_back_on_explicitly() {
        // The frame is plain document data: nothing re-applies it.
        let mut config = config_for_series(&[feynman()]);
        config.apply_axis_preset(AxisPreset::BoxedInward);
        assert!(config.bottom_x.line_visible && config.left_y.line_visible);
        assert!(
            config.bottom_x.label_style.label_visible && config.left_y.label_style.label_visible
        );
        // A preset only switches the grid off; the host turns it back on.
        config.grid.show_major_x = true;
        config.grid.show_major_y = true;
        assert_eq!(config, default_config());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn the_framed_config_round_trips() {
        for series in [vec![feynman()], vec![levels()], vec![]] {
            let config = config_for_series(&series);
            let json = serde_json::to_string(&config).unwrap();
            let back: Config = serde_json::from_str(&json).unwrap();
            assert_eq!(back, config);
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
    }
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
/// - `layout_state` is [`LayoutState::NeedsFit`], as a new chart's is: the
///   names above the plot need a top band, so the first default draw fits
///   the layout to them and to the host's axes and names.
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
    config.layout_state = LayoutState::NeedsFit;
    config
}
