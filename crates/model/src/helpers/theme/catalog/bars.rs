//! Histogram, category-bar, distribution and range-bar themes.

use super::super::{ChartKind, Theme, ThemeFamily};
use super::{
    NIGHT_BG, NIGHT_GRID, NIGHT_INK, alpha, document, family_chrome, gallery, palette, rgb, scene,
};
use crate::color::Color;
use crate::material::Material;

pub(super) const HISTOGRAM: &[&str] =
    &["gallery", "cool-slate", "midnight", "warm-paper", "default"];
pub(super) const CATEGORY_BARS: &[&str] = &[
    "gallery",
    "cool-slate",
    "midnight",
    "warm-paper",
    "default",
    "document",
    "revenue",
];
pub(super) const DISTRIBUTION: &[&str] = &[
    "gallery",
    "cool-slate",
    "midnight",
    "warm-paper",
    "default",
    "document",
];
pub(super) const RANGE_BARS: &[&str] = &[
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
        (ChartKind::Histogram, ThemeFamily::Gallery) => histogram_gallery(),
        (ChartKind::CategoryBars, ThemeFamily::Gallery) => category_bars_document(),
        (ChartKind::Distribution, ThemeFamily::Gallery) => distribution_document(),
        (ChartKind::Distribution, ThemeFamily::CoolSlate) => distribution_cool_slate(),
        (ChartKind::Distribution, ThemeFamily::WarmPaper) => distribution_warm_paper(),
        (ChartKind::RangeBars, ThemeFamily::CoolSlate) => range_bars_cool_slate(),
        (ChartKind::RangeBars, ThemeFamily::Midnight) => range_bars_midnight(),
        (ChartKind::RangeBars, ThemeFamily::WarmPaper) => range_bars_warm_neutral(),
        (ChartKind::Histogram, _) => histogram(family),
        (ChartKind::CategoryBars, _) => category_bars(family),
        (ChartKind::Distribution, _) => distribution_midnight(),
        _ => range_bars_gallery(),
    }
}

/// The theme `name` of `kind`'s own catalogue, past `default`.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    match (kind, name) {
        (ChartKind::CategoryBars, "document") => Some(category_bars_document()),
        (ChartKind::CategoryBars, "revenue") => Some(category_bars_revenue()),
        (ChartKind::Distribution, "document") => Some(distribution_document()),
        (ChartKind::RangeBars, "warm-neutral") => Some(range_bars_warm_neutral()),
        _ => None,
    }
}

/// The gallery's histogram: teal bars with a darker teal border, two
/// pixels apart, on white paper.
fn histogram_gallery() -> Theme {
    let mut theme = gallery(ChartKind::Histogram);
    theme.series.palette = vec![rgb(19, 154, 139)];
    let bars = &mut theme.series.bars;
    bars.border_color = rgb(10, 102, 95);
    bars.border_width = 0.8;
    bars.gap = 2.0;
    theme
}

/// A categorical document: matte blue, orange and teal bars with rounded
/// ends on a transparent background, the open frame and the text in navy.
/// A single series takes one colour per category.
fn category_bars_document() -> Theme {
    let mut theme = document(ChartKind::CategoryBars, rgb(35, 45, 65));
    theme.series.palette = vec![
        rgb(85, 143, 211),
        rgb(255, 166, 105),
        rgb(63, 169, 167),
        rgb(98, 194, 192),
    ];
    let bars = &mut theme.series.bars;
    bars.material = Material::Matte;
    bars.color_by_category = true;
    theme
}

/// The product analytics card: matte violet, lavender and teal bars with
/// well-rounded ends and no texture, slate text on a cool off-white, no
/// grid and a legend without a box.
fn category_bars_revenue() -> Theme {
    let bg = rgb(247, 248, 253);
    let mut theme = document(ChartKind::CategoryBars, rgb(52, 57, 85));
    let chrome = &mut theme.chrome;
    chrome.background = Some(bg);
    chrome.title_size = 30.0;
    chrome.axis_title_size = 20.0;
    chrome.tick_label_size = 20.0;
    chrome.legend_size = 20.0;
    chrome.grid.major_y.show = false;
    chrome.legend_fill = Color::new(0.0, 0.0, 0.0, 0.0);
    chrome.legend_border = Color::new(0.0, 0.0, 0.0, 0.0);
    chrome.legend_padding = 4.0;
    theme.series.palette = vec![rgb(98, 86, 225), rgb(157, 141, 242), rgb(37, 181, 167)];
    let bars = &mut theme.series.bars;
    bars.material = Material::Matte;
    bars.corner_radius = 12.0;
    bars.texture_strength = 0.0;
    theme
}

/// A box plot document: matte blue, teal and amber boxes with navy
/// outlines, medians and whiskers on a transparent background. A single
/// series takes one colour per category.
fn distribution_document() -> Theme {
    let mut theme = document(ChartKind::Distribution, rgb(32, 46, 60));
    theme.series.palette = vec![rgb(65, 144, 208), rgb(42, 168, 162), rgb(230, 169, 64)];
    theme.series.boxes.color_by_category = true;
    theme
}

/// The scene chrome at the distribution scenes' 960 x 640 size: smaller
/// text and a tighter legend.
fn distribution_scene(bg: Color, ink: Color, grid: Color) -> Theme {
    let mut theme = scene(ChartKind::Distribution, bg, ink, grid);
    let chrome = &mut theme.chrome;
    chrome.title_size = 28.0;
    chrome.axis_title_size = 19.0;
    chrome.tick_label_size = 18.0;
    chrome.legend_size = 18.0;
    chrome.legend_padding = 12.0;
    theme
}

/// The light violin scene: slate ink on a cool white, blue and coral
/// violins outlined in slate.
fn distribution_cool_slate() -> Theme {
    let mut theme = distribution_scene(rgb(250, 250, 252), rgb(44, 50, 72), rgb(232, 234, 242));
    theme.series.palette = vec![rgb(86, 136, 204), rgb(234, 130, 100)];
    theme.series.boxes.ink = rgb(58, 64, 88);
    theme
}

/// The warm raincloud scene: brown ink on cream paper with a vertical
/// grid, unoutlined clouds in teal, coral, violet, green and amber, one
/// colour per category of a single series, slim boxes without caps, a bold
/// median and translucent raw points.
fn distribution_warm_paper() -> Theme {
    let ink = rgb(70, 60, 56);
    let mut theme = distribution_scene(rgb(252, 249, 244), ink, rgb(235, 228, 216));
    let grid = &mut theme.chrome.grid;
    grid.major_x.show = true;
    grid.major_y.show = false;
    theme.series.palette = vec![
        rgb(56, 158, 168),
        rgb(214, 104, 88),
        rgb(124, 112, 196),
        rgb(118, 170, 104),
        rgb(232, 168, 64),
    ];
    let boxes = &mut theme.series.boxes;
    boxes.ink = ink;
    boxes.median_width = 2.0;
    boxes.whisker_width = 1.4;
    boxes.caps = false;
    boxes.violin_outline = false;
    boxes.point_opacity = Some(0.55);
    boxes.color_by_category = true;
    theme
}

/// The profit bridge: green rises, coral falls and navy totals with small
/// round corners, grey connectors, slate ink on a warm off-white.
fn range_bars_warm_neutral() -> Theme {
    let total = rgb(56, 86, 140);
    let mut theme = scene(
        ChartKind::RangeBars,
        rgb(251, 250, 247),
        rgb(46, 52, 64),
        rgb(232, 230, 224),
    );
    theme.series.palette = vec![total];
    theme.series.bars.corner_radius = 3.0;
    let range = &mut theme.series.range_bars;
    range.up = Some(rgb(42, 157, 122));
    range.down = Some(rgb(221, 96, 78));
    range.total = Some(total);
    range.connector_color = rgb(150, 150, 146);
    range.connector_width = 1.5;
    theme
}

/// The dark trading card: square green and red candles with their wicks
/// and an amber line, pale ink on midnight with a full grid.
fn range_bars_midnight() -> Theme {
    let up = rgb(38, 184, 150);
    let mut theme = scene(
        ChartKind::RangeBars,
        rgb(16, 21, 31),
        rgb(176, 186, 204),
        rgb(34, 42, 57),
    );
    theme.chrome.grid.major_x.show = true;
    theme.series.palette = vec![up, rgb(242, 186, 78)];
    theme.series.lines.line_width = 2.5;
    theme.series.bars.corner_radius = 0.0;
    let range = &mut theme.series.range_bars;
    range.up = Some(up);
    range.down = Some(rgb(238, 86, 82));
    range.whisker_width = 1.5;
    theme
}

/// The release plan: rounded task bars in green (done), blue (in
/// progress), amber (at risk) and grey (not started), their unfinished part
/// faded, slate ink on a cool white with a vertical grid.
fn range_bars_cool_slate() -> Theme {
    let mut theme = scene(
        ChartKind::RangeBars,
        rgb(248, 249, 252),
        rgb(40, 48, 66),
        rgb(228, 232, 240),
    );
    let grid = &mut theme.chrome.grid;
    grid.major_x.show = true;
    grid.major_y.show = false;
    let states = vec![
        rgb(96, 186, 150),
        rgb(112, 156, 236),
        rgb(243, 170, 88),
        rgb(132, 144, 168),
    ];
    theme.series.palette.clone_from(&states);
    theme.series.bars.corner_radius = 5.0;
    let range = &mut theme.series.range_bars;
    range.states = states;
    range.remaining_opacity = Some(0.35);
    theme
}

/// A family's histogram (cool slate, midnight, warm paper): bars in the
/// family's colours with a border that sets them apart, two pixels apart.
fn histogram(family: ThemeFamily) -> Theme {
    let mut theme = family_chrome(family, ChartKind::Histogram);
    theme.series.palette = palette(family);
    let bars = &mut theme.series.bars;
    bars.border_color = match family {
        ThemeFamily::Midnight => NIGHT_BG,
        ThemeFamily::CoolSlate => rgb(24, 98, 106),
        _ => rgb(128, 52, 36),
    };
    bars.border_width = 0.8;
    bars.gap = 2.0;
    theme
}

/// A family's category bars (cool slate, midnight, warm paper): matte bars
/// in the family's colours with rounded ends and no texture; a single
/// series takes one colour per category.
fn category_bars(family: ThemeFamily) -> Theme {
    let mut theme = family_chrome(family, ChartKind::CategoryBars);
    theme.series.palette = palette(family);
    let bars = &mut theme.series.bars;
    bars.material = Material::Matte;
    bars.corner_radius = 6.0;
    bars.texture_strength = 0.0;
    bars.color_by_category = true;
    theme
}

/// The dark violin and box scene: pale outlines, medians and whiskers on
/// midnight, mint, lavender and amber bodies one colour per category of a
/// single series, an amber mean and translucent raw points.
fn distribution_midnight() -> Theme {
    let mut theme = distribution_scene(NIGHT_BG, NIGHT_INK, NIGHT_GRID);
    theme.series.palette = palette(ThemeFamily::Midnight);
    let boxes = &mut theme.series.boxes;
    boxes.ink = rgb(206, 216, 232);
    boxes.mean_color = rgb(255, 214, 120);
    boxes.outlier_color = rgb(206, 216, 232);
    boxes.point_opacity = Some(0.6);
    boxes.color_by_category = true;
    theme
}

/// The gallery's range bars: green rises, coral falls and blue totals with
/// small round corners, slate connectors and whiskers, state colours from
/// the gallery's palette with their unfinished part faded, on white paper.
fn range_bars_gallery() -> Theme {
    let mut theme = gallery(ChartKind::RangeBars);
    let total = rgb(34, 104, 198);
    theme.series.palette = palette(ThemeFamily::Gallery);
    theme.series.bars.corner_radius = 3.0;
    theme.series.lines.line_width = 2.5;
    let range = &mut theme.series.range_bars;
    range.up = Some(rgb(30, 150, 110));
    range.down = Some(rgb(225, 81, 79));
    range.total = Some(total);
    range.states = vec![
        rgb(30, 150, 110),
        total,
        rgb(224, 123, 38),
        rgb(140, 150, 168),
    ];
    range.whisker_width = 1.5;
    range.connector_color = alpha(rgb(88, 102, 123), 0.8);
    range.connector_width = 1.5;
    range.remaining_opacity = Some(0.35);
    theme
}
