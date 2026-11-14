//! Histogram, category-bar, distribution and range-bar themes.

use super::super::{ChartKind, Theme};
use super::{document, gallery, rgb, scene};
use crate::color::Color;
use crate::material::Material;

pub(super) const HISTOGRAM: &[&str] = &["default", "gallery"];
pub(super) const CATEGORY_BARS: &[&str] = &["default", "document", "revenue"];
pub(super) const DISTRIBUTION: &[&str] = &["default", "document", "cool-slate", "warm-paper"];
pub(super) const RANGE_BARS: &[&str] = &["default", "warm-neutral", "midnight", "cool-slate"];

/// The theme `name` of `kind`'s catalogue, past `default`.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    match (kind, name) {
        (ChartKind::Histogram, "gallery") => Some(histogram_gallery()),
        (ChartKind::CategoryBars, "document") => Some(category_bars_document()),
        (ChartKind::CategoryBars, "revenue") => Some(category_bars_revenue()),
        (ChartKind::Distribution, "document") => Some(distribution_document()),
        (ChartKind::Distribution, "cool-slate") => Some(distribution_cool_slate()),
        (ChartKind::Distribution, "warm-paper") => Some(distribution_warm_paper()),
        (ChartKind::RangeBars, "warm-neutral") => Some(range_bars_warm_neutral()),
        (ChartKind::RangeBars, "midnight") => Some(range_bars_midnight()),
        (ChartKind::RangeBars, "cool-slate") => Some(range_bars_cool_slate()),
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
