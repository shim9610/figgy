//! Line, scatter, error-bar, stem and probability-paper themes.

use super::super::{ChartKind, Theme};
use super::{gallery, rgb, scene};
use crate::config::{DrawStyle, MilkywayOptions, SketchOptions};

pub(super) const LINE: &[&str] = &[
    "default",
    "gallery",
    "cool-slate",
    "midnight",
    "editorial",
    "milkyway",
    "constellation",
];
pub(super) const SCATTER: &[&str] = &["default"];
pub(super) const ERROR_BARS: &[&str] = &["default"];
pub(super) const STEM: &[&str] = &["default"];
pub(super) const PROBABILITY: &[&str] = &["default"];

/// The theme `name` of `kind`'s catalogue, past `default`.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    match (kind, name) {
        (ChartKind::Line, "gallery") => Some(line_gallery()),
        (ChartKind::Line, "cool-slate") => Some(line_cool_slate()),
        (ChartKind::Line, "midnight") => Some(line_midnight()),
        (ChartKind::Line, "editorial") => Some(line_editorial()),
        (ChartKind::Line, "milkyway") => Some(night_sky(DrawStyle::Milkyway(MilkywayOptions {
            star_density: 70.0,
            ribbon_width_px: 24.0,
            ribbon_intensity: 0.08,
            star_brightness: 0.32,
            spread_px: 8.0,
            faint_bias: 4.5,
            glow: 0.35,
            nebula: 0.6,
            dust: 0.7,
            planet_rim: 0.0,
            ..crate::defaults::milkyway_options()
        }))),
        (ChartKind::Line, "constellation") => Some(night_sky(DrawStyle::Constellation(
            crate::defaults::constellation_options(),
        ))),
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
