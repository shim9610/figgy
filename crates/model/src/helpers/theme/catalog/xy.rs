//! Line, scatter, error-bar, stem and probability-paper themes.

use super::super::{ChartKind, Theme, ThemeFamily};
use super::{colormap, family_chrome, gallery, gridded, palette, rgb, scene};
use crate::colormap::ColorMap;
use crate::config::{DrawStyle, MilkywayOptions, SketchOptions};

pub(super) const LINE: &[&str] = &[
    "gallery",
    "cool-slate",
    "midnight",
    "warm-paper",
    "default",
    "editorial",
    "milkyway",
    "constellation",
];
pub(super) const SCATTER: &[&str] = &["gallery", "cool-slate", "midnight", "warm-paper", "default"];
pub(super) const ERROR_BARS: &[&str] =
    &["gallery", "cool-slate", "midnight", "warm-paper", "default"];
pub(super) const STEM: &[&str] = &["gallery", "cool-slate", "midnight", "warm-paper", "default"];
pub(super) const PROBABILITY: &[&str] = &[
    "gallery",
    "cool-slate",
    "midnight",
    "warm-paper",
    "default",
    "warm-neutral",
];

/// `family`'s member for `kind`.
pub(super) fn member(family: ThemeFamily, kind: ChartKind) -> Theme {
    match (kind, family) {
        (ChartKind::Line, ThemeFamily::Gallery) => line_gallery(),
        (ChartKind::Line, ThemeFamily::CoolSlate) => line_cool_slate(),
        (ChartKind::Line, ThemeFamily::Midnight) => line_midnight(),
        (ChartKind::Scatter, ThemeFamily::CoolSlate) => scatter_cool_slate(),
        (ChartKind::Scatter, ThemeFamily::WarmPaper) => scatter_warm_paper(),
        (ChartKind::ErrorBars, ThemeFamily::Gallery) => error_bars_gallery(),
        (ChartKind::Stem, ThemeFamily::CoolSlate) => stem_cool_slate(),
        (ChartKind::Stem, ThemeFamily::WarmPaper) => stem_warm_paper(),
        (ChartKind::Probability, ThemeFamily::WarmPaper) => probability_warm_neutral(),
        (ChartKind::Line, _) => line(family),
        (ChartKind::Scatter, _) => scatter(family),
        (ChartKind::ErrorBars, _) => error_bars(family),
        (ChartKind::Stem, _) => stem(family),
        _ => probability(family),
    }
}

/// The theme `name` of `kind`'s own catalogue, past `default`.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    match (kind, name) {
        (ChartKind::Line, "editorial") => Some(line_editorial()),
        (ChartKind::Line, "milkyway") => {
            // Every field is named (#46); the scales and the seed are the
            // documented defaults.
            let documented = crate::defaults::milkyway_options();
            Some(night_sky(DrawStyle::Milkyway(MilkywayOptions {
                star_density: 70.0,
                ribbon_width_px: 24.0,
                ribbon_intensity: 0.08,
                star_scale: documented.star_scale,
                star_brightness: 0.32,
                spread_px: 8.0,
                structure_scale: documented.structure_scale,
                faint_bias: 4.5,
                glow: 0.35,
                nebula: 0.6,
                dust: 0.7,
                planet_rim: 0.0,
                seed: documented.seed,
            })))
        }
        (ChartKind::Line, "constellation") => Some(night_sky(DrawStyle::Constellation(
            crate::defaults::constellation_options(),
        ))),
        (ChartKind::Probability, "warm-neutral") => Some(probability_warm_neutral()),
        _ => None,
    }
}

/// The gallery's line charts (the travelling waves, the orbit): a blue and
/// a coral line on white paper.
fn line_gallery() -> Theme {
    let mut theme = gallery(ChartKind::Line);
    theme.series.palette = vec![rgb(34, 104, 198), rgb(225, 81, 79), rgb(224, 123, 38)];
    theme.series.lines.line_width = 3.0;
    theme.series.lines.marker_size = 11.0;
    theme
}

/// The light dashboard scenes (forecast, thermostat, small multiples):
/// slate ink on a cool off-white, teal, coral, violet and amber lines with
/// a light fill.
fn line_cool_slate() -> Theme {
    let mut theme = scene(
        ChartKind::Line,
        rgb(247, 248, 253),
        rgb(52, 57, 85),
        rgb(228, 231, 240),
    );
    theme.series.palette = vec![
        rgb(37, 150, 160),
        rgb(232, 104, 84),
        rgb(112, 92, 212),
        rgb(228, 170, 50),
    ];
    theme.series.lines.line_width = 3.0;
    theme.series.lines.fill_opacity = Some(0.1);
    theme
}

/// The dark monitoring card (observability, twin axes, a broken-axis
/// spectrum): pale ink on midnight navy, mint, lavender, cyan and amber
/// lines.
fn line_midnight() -> Theme {
    let mut theme = scene(
        ChartKind::Line,
        rgb(14, 22, 38),
        rgb(182, 199, 222),
        rgb(36, 49, 70),
    );
    theme.series.palette = vec![
        rgb(65, 227, 194),
        rgb(139, 154, 255),
        rgb(96, 208, 232),
        rgb(242, 186, 78),
    ];
    theme.series.lines.line_width = 3.5;
    theme.series.lines.fill_opacity = Some(0.12);
    theme
}

/// The warm editorial chart: hand-drawn strokes and round markers on
/// newsprint.
fn line_editorial() -> Theme {
    let mut theme = scene(
        ChartKind::Line,
        rgb(250, 245, 232),
        rgb(78, 67, 56),
        rgb(225, 215, 195),
    );
    theme.chrome.draw_style = DrawStyle::Sketch(SketchOptions {
        amplitude_px: 1.5,
        wavelength_px: 64.0,
        seed: 7,
    });
    theme.series.palette = vec![rgb(190, 81, 51), rgb(49, 121, 107)];
    theme.series.lines.line_width = 3.0;
    theme.series.lines.marker_size = 7.0;
    theme
}

/// The night-sky style previews: pale ink on deep space, no grid, pastel
/// pink and cyan lines drawn in `style`.
fn night_sky(style: DrawStyle) -> Theme {
    let bg = rgb(11, 15, 23);
    let ink = rgb(225, 228, 235);
    let soft = rgb(190, 195, 205);
    let mut theme = Theme::documented(ChartKind::Line);
    let chrome = &mut theme.chrome;
    chrome.background = Some(bg);
    chrome.draw_style = style;
    chrome.ink = ink;
    chrome.frame_ink = soft;
    chrome.label_ink = soft;
    chrome.tick_label_size = 25.0;
    chrome.axis_title_size = 32.0;
    chrome.legend_size = 23.0;
    chrome.legend_fill = bg;
    chrome.legend_border = rgb(90, 95, 106);
    chrome.legend_padding = 9.0;
    chrome.colorbar.border = rgb(140, 145, 155);
    chrome.highlight.color = rgb(255, 214, 10);
    for line in [
        &mut chrome.grid.major_x,
        &mut chrome.grid.major_y,
        &mut chrome.grid.minor_x,
        &mut chrome.grid.minor_y,
    ] {
        line.show = false;
    }
    theme.series.palette = vec![rgb(225, 161, 206), rgb(165, 215, 229)];
    theme.series.lines.line_width = 2.0;
    theme
}

/// The light scatter scenes (a colour-mapped spiral, gated cytometry
/// events): slate ink on a cool white with a full grid, small translucent
/// markers in grey, teal, amber and violet, and a warm-to-indigo
/// colourbar.
fn scatter_cool_slate() -> Theme {
    let ink = rgb(52, 57, 85);
    let mut theme = scene(
        ChartKind::Scatter,
        rgb(250, 250, 252),
        ink,
        rgb(232, 234, 242),
    );
    theme.chrome.grid.major_x.show = true;
    theme.chrome.colorbar.title_size = 19.0;
    theme.series.palette = vec![
        rgb(150, 156, 174),
        rgb(22, 150, 140),
        rgb(222, 132, 30),
        rgb(136, 84, 196),
    ];
    theme.series.lines.marker_size = 5.0;
    theme.series.lines.marker_opacity = Some(0.5);
    theme.series.fields.colormap = ColorMap::Custom {
        stops: vec![
            rgb(244, 190, 72),
            rgb(238, 110, 92),
            rgb(170, 70, 160),
            rgb(70, 60, 160),
            rgb(24, 32, 96),
        ],
    };
    theme
}

/// The warm bubble chart: brown ink on cream paper, coral, teal and violet
/// markers at a translucency that keeps overlaps readable.
fn scatter_warm_paper() -> Theme {
    let mut theme = scene(
        ChartKind::Scatter,
        rgb(252, 249, 244),
        rgb(70, 60, 56),
        rgb(235, 228, 216),
    );
    theme.series.palette = vec![rgb(222, 101, 83), rgb(45, 140, 135), rgb(118, 92, 196)];
    theme.series.lines.marker_size = 8.0;
    theme.series.lines.marker_opacity = Some(0.62);
    theme
}

/// The gallery's measured curves: blue and orange lines through markers
/// with capped error bars, on white paper.
fn error_bars_gallery() -> Theme {
    let mut theme = gallery(ChartKind::ErrorBars);
    theme.series.palette = vec![rgb(34, 104, 198), rgb(224, 123, 38)];
    let lines = &mut theme.series.lines;
    lines.line_width = 2.5;
    lines.error_bar_width = 1.5;
    lines.error_cap_width = 1.5;
    lines.error_cap_size = 7.0;
    theme
}

/// The light stem scene (a filter's impulse response): slate ink on a cool
/// white, teal stems a shade lighter than their markers, a sand line for a
/// reference curve.
fn stem_cool_slate() -> Theme {
    let mut theme = scene(
        ChartKind::Stem,
        rgb(248, 249, 252),
        rgb(44, 52, 78),
        rgb(228, 232, 240),
    );
    theme.series.palette = vec![rgb(22, 128, 140), rgb(214, 148, 72)];
    let lines = &mut theme.series.lines;
    lines.line_width = 2.0;
    lines.marker_size = 10.0;
    lines.stem_width = 2.5;
    lines.stem_opacity = Some(0.75);
    theme
}

/// The warm lollipop chart: brown ink on cream paper, thick green and rust
/// stems with large round heads.
fn stem_warm_paper() -> Theme {
    let mut theme = scene(
        ChartKind::Stem,
        rgb(251, 249, 245),
        rgb(62, 56, 52),
        rgb(234, 228, 218),
    );
    theme.series.palette = vec![rgb(46, 140, 96), rgb(204, 88, 64)];
    let lines = &mut theme.series.lines;
    lines.line_width = 3.0;
    lines.marker_size = 18.0;
    lines.stem_width = 4.0;
    theme
}

/// Probability paper: slate ink on a warm off-white with a full grid, blue
/// and red samples with their fitted lines, compact text for side-by-side
/// papers.
fn probability_warm_neutral() -> Theme {
    let mut theme = scene(
        ChartKind::Probability,
        rgb(250, 249, 246),
        rgb(48, 52, 70),
        rgb(226, 228, 236),
    );
    theme.chrome.title_size = 28.0;
    theme.chrome.tick_label_size = 18.0;
    theme.chrome.grid.major_x.show = true;
    theme.series.palette = vec![rgb(44, 110, 190), rgb(204, 72, 52)];
    theme.series.lines.line_width = 3.0;
    theme.series.lines.marker_size = 11.0;
    theme
}

/// A family's line chart (warm paper): its colours at three pixels with a
/// light fill.
fn line(family: ThemeFamily) -> Theme {
    let mut theme = family_chrome(family, ChartKind::Line);
    theme.series.palette = palette(family);
    theme.series.lines.line_width = 3.0;
    theme.series.lines.fill_opacity = Some(0.1);
    theme
}

/// A family's scatter chart (gallery, midnight): a full grid, markers a
/// little translucent so that overlaps stay readable, and the family's map
/// for markers coloured by value.
fn scatter(family: ThemeFamily) -> Theme {
    let mut theme = gridded(family_chrome(family, ChartKind::Scatter));
    theme.series.palette = palette(family);
    theme.series.lines.marker_size = 7.0;
    theme.series.lines.marker_opacity = Some(0.7);
    theme.series.fields.colormap = colormap(family);
    theme
}

/// A family's measured curves (cool slate, midnight, warm paper): lines
/// through markers with capped error bars.
fn error_bars(family: ThemeFamily) -> Theme {
    let mut theme = family_chrome(family, ChartKind::ErrorBars);
    theme.series.palette = palette(family);
    let lines = &mut theme.series.lines;
    lines.line_width = 2.5;
    lines.marker_size = 9.0;
    lines.error_bar_width = 1.5;
    lines.error_cap_width = 1.5;
    lines.error_cap_size = 7.0;
    theme
}

/// A family's stems (gallery, midnight): stems a shade lighter than their
/// round heads.
fn stem(family: ThemeFamily) -> Theme {
    let mut theme = family_chrome(family, ChartKind::Stem);
    theme.series.palette = palette(family);
    let lines = &mut theme.series.lines;
    lines.line_width = 2.0;
    lines.marker_size = 11.0;
    lines.stem_width = 2.5;
    lines.stem_opacity = Some(0.75);
    theme
}

/// A family's probability paper (gallery, cool slate, midnight): a full
/// grid, the samples with their fitted lines, and compact text for papers
/// side by side.
fn probability(family: ThemeFamily) -> Theme {
    let mut theme = gridded(family_chrome(family, ChartKind::Probability));
    if family != ThemeFamily::Gallery {
        theme.chrome.title_size = 28.0;
        theme.chrome.tick_label_size = 18.0;
    }
    theme.series.palette = palette(family);
    theme.series.lines.line_width = 3.0;
    theme.series.lines.marker_size = 11.0;
    theme
}
