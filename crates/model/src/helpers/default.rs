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

use crate::Color;
use crate::config::{AxisOptions, ColorBarOptions, Config, LayoutState, Legend, TickVisibility};
use crate::data_config::{BarGroups, CategoryBarDirection, SeriesConfig};
use crate::format::{AutoNumberFormat, LabelFormat};
use crate::parallel::ParallelOptions;
use crate::preset::AxisPreset;

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
/// The series take the default palette ([`Config::apply_palette`] with
/// [`crate::preset::ColorCycle::DEFAULT`]): series `i` draws in its colour
/// `i`, so a chart of several series built from the constructors does not
/// draw them all in one colour.
///
/// The frame and the colours are written into the returned config and
/// the series, a plain document: they are chosen once, where the chart is
/// made, never at draw time, so a host that turns the axes back on
/// (`Config::apply_axis_preset`) or recolours a series keeps it. Its
/// layout is a new chart's ([`LayoutState::Fitted`]): the chart object
/// made from it fits the layout to the frame and content it holds, and a
/// renderer registering it fits the axis ranges once
/// (`Renderer::register_chart`). Nothing is fitted here.
pub fn config_for_series(series: &mut [SeriesConfig]) -> Config {
    let mut config = default_config();
    config.layout_state = LayoutState::Fitted;
    let frame = crate::helpers::theme::Theme::default().family_frame(
        &config,
        series,
        crate::layer::LayerRef::Base,
    );
    if let Some(preset) = frame {
        config.apply_axis_preset(preset);
    }
    config.apply_palette(series, crate::preset::ColorCycle::DEFAULT.colors());
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::AxisPreset;

    fn framed(preset: AxisPreset) -> Config {
        let mut config = unframed();
        config.apply_axis_preset(preset);
        config
    }

    /// [`default_config`] as a new chart's: its layout kept fitted (#357).
    fn unframed() -> Config {
        Config {
            layout_state: LayoutState::Fitted,
            ..default_config()
        }
    }

    fn feynman() -> SeriesConfig {
        SeriesConfig::feynman("d", crate::defaults::feynman_series())
    }

    fn levels() -> SeriesConfig {
        SeriesConfig::energy_levels("e", crate::defaults::energy_level_series())
    }

    #[test]
    fn a_chart_takes_the_frame_of_its_family() {
        assert_eq!(
            config_for_series(&mut [feynman()]),
            framed(AxisPreset::Hidden)
        );
        assert_eq!(
            config_for_series(&mut [levels()]),
            framed(AxisPreset::Grotrian)
        );
        assert_eq!(config_for_series(&mut []), unframed());
        let line = SeriesConfig::line("l", "x", "y");
        assert_eq!(config_for_series(&mut [line.clone()]), unframed());
        // The first framed family decides, as a theme's frame does.
        assert_eq!(
            config_for_series(&mut [line, feynman(), levels()]),
            framed(AxisPreset::Hidden)
        );
        let hidden = config_for_series(&mut [feynman()]);
        assert!(!hidden.bottom_x.line_visible && !hidden.left_y.line_visible);
        assert!(!hidden.grid.show_major_x && !hidden.grid.show_major_y);
    }

    #[test]
    fn the_series_take_the_default_palette_in_order() {
        use crate::preset::ColorCycle;
        let mut series: Vec<SeriesConfig> = (0..12)
            .map(|i| SeriesConfig::line(format!("s{i}"), "x", "y"))
            .collect();
        let mut expected = series.clone();
        for (i, item) in expected.iter_mut().enumerate() {
            ColorCycle::DEFAULT.apply_to_series(item, i);
        }
        let config = config_for_series(&mut series);
        assert_eq!(series, expected);
        // Colouring writes no layout: the config is the unframed default.
        assert_eq!(config, unframed());
        assert_ne!(
            ColorCycle::DEFAULT.color(0),
            ColorCycle::DEFAULT.color(1),
            "no chart of two series draws them in one colour"
        );
    }

    #[test]
    fn the_axes_turn_back_on_explicitly() {
        // The frame is plain document data: nothing re-applies it.
        let mut config = config_for_series(&mut [feynman()]);
        config.apply_axis_preset(AxisPreset::BoxedInward);
        assert!(config.bottom_x.line_visible && config.left_y.line_visible);
        assert!(
            config.bottom_x.label_style.label_visible && config.left_y.label_style.label_visible
        );
        // A preset only switches the grid off; the host turns it back on.
        config.grid.show_major_x = true;
        config.grid.show_major_y = true;
        assert_eq!(config, unframed());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn the_framed_config_round_trips() {
        for mut series in [vec![feynman()], vec![levels()], vec![]] {
            let config = config_for_series(&mut series);
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
/// - `layout_state` is [`LayoutState::Fitted`], as a new chart's is: the
///   names above the plot need a top band, so the chart object made from
///   the config fits the layout to them and to the host's axes and names,
///   and its edits keep it fitted.
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
    config.layout_state = LayoutState::Fitted;
    config
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
///   light grey and no tick marks: the gridlines carry the values;
/// - `layout_state` is [`LayoutState::Fitted`], as a new chart's is: the
///   row labels on the left are as wide as the host's task names, so the
///   chart made from the config is created with its layout fitted to them,
///   and its edits keep it fitted.
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
    config.layout_state = LayoutState::Fitted;
    config
}
