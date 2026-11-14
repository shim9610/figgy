//! The built-in themes, one catalogue per chart kind (#291), and the
//! families' members (#295).
//!
//! A kind's own themes are gallery looks: the values the gallery images
//! use. A family ([`ThemeFamily`]) has a member for every kind: one of
//! those looks where its chrome is the family's, a theme designed in the
//! family's chrome otherwise ([`member`]). Bases carry the chrome several
//! looks share, and each kind's themes add their palette and their
//! family's part:
//! - [`gallery`]: white paper, a navy title, a slate frame and a pale grid;
//! - [`scene`]: the designed scenes' chrome, an opaque background with no
//!   axis lines or ticks, a horizontal grid and large text;
//! - [`document`]: the legacy documents' chrome, a transparent background,
//!   an open frame without ticks in the text colour and a pale value grid;
//! - [`cool_slate`], [`midnight`] and [`warm_paper`]: the scene chrome in
//!   the colours of those families.

use super::{ChartKind, RadialTheme, Theme, ThemeFamily};
use crate::color::Color;
use crate::colormap::ColorMap;
use crate::line::LineStylePreset;
use crate::preset::AxisPreset;

mod bars;
mod fields;
mod radial;
mod science;
mod xy;

/// The names `kind` takes: the families, then `default` and the kind's own.
pub(super) fn names(kind: ChartKind) -> &'static [&'static str] {
    match kind {
        ChartKind::Line => xy::LINE,
        ChartKind::Scatter => xy::SCATTER,
        ChartKind::ErrorBars => xy::ERROR_BARS,
        ChartKind::Stem => xy::STEM,
        ChartKind::Probability => xy::PROBABILITY,
        ChartKind::Histogram => bars::HISTOGRAM,
        ChartKind::CategoryBars => bars::CATEGORY_BARS,
        ChartKind::Distribution => bars::DISTRIBUTION,
        ChartKind::RangeBars => bars::RANGE_BARS,
        ChartKind::Field => fields::FIELD,
        ChartKind::VectorField => fields::VECTOR_FIELD,
        ChartKind::Surface3d => fields::SURFACE_3D,
        ChartKind::TriMesh => fields::TRI_MESH,
        ChartKind::Polar => fields::POLAR,
        ChartKind::Radar => fields::RADAR,
        ChartKind::Parallel => fields::PARALLEL,
        ChartKind::Ternary => fields::TERNARY,
        ChartKind::EnergyLevels => science::ENERGY_LEVELS,
        ChartKind::Feynman => science::FEYNMAN,
    }
}

/// `family`'s member for `kind`.
pub(super) fn member(family: ThemeFamily, kind: ChartKind) -> Theme {
    match kind {
        ChartKind::Line
        | ChartKind::Scatter
        | ChartKind::ErrorBars
        | ChartKind::Stem
        | ChartKind::Probability => xy::member(family, kind),
        ChartKind::Histogram
        | ChartKind::CategoryBars
        | ChartKind::Distribution
        | ChartKind::RangeBars => bars::member(family, kind),
        ChartKind::Field
        | ChartKind::VectorField
        | ChartKind::Surface3d
        | ChartKind::TriMesh
        | ChartKind::Polar
        | ChartKind::Radar
        | ChartKind::Parallel
        | ChartKind::Ternary => fields::member(family, kind),
        ChartKind::EnergyLevels | ChartKind::Feynman => science::member(family, kind),
    }
}

/// `family`'s radial member.
pub(super) fn radial_member(family: ThemeFamily) -> RadialTheme {
    radial::member(family)
}

/// The theme `name` of `kind`'s own catalogue: `default` or a theme of the
/// kind's own, not a family.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    if name == "default" {
        return Some(Theme::documented(kind));
    }
    match kind {
        ChartKind::Line
        | ChartKind::Scatter
        | ChartKind::ErrorBars
        | ChartKind::Stem
        | ChartKind::Probability => xy::named(kind, name),
        ChartKind::Histogram
        | ChartKind::CategoryBars
        | ChartKind::Distribution
        | ChartKind::RangeBars => bars::named(kind, name),
        ChartKind::Field
        | ChartKind::VectorField
        | ChartKind::Surface3d
        | ChartKind::TriMesh
        | ChartKind::Polar
        | ChartKind::Radar
        | ChartKind::Parallel
        | ChartKind::Ternary => fields::named(kind, name),
        ChartKind::EnergyLevels | ChartKind::Feynman => science::named(kind, name),
    }
}

/// The radial names: the families, then `default` and the radial themes.
pub(super) fn radial_names() -> &'static [&'static str] {
    radial::NAMES
}

/// The radial theme `name` of the radial catalogue, not a family.
pub(super) fn radial_named(name: &str) -> Option<RadialTheme> {
    match name {
        "default" => Some(RadialTheme::documented()),
        _ => radial::named(name),
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

/// `color` at opacity `a`.
const fn alpha(color: Color, a: f32) -> Color {
    Color::new(color.r, color.g, color.b, a)
}

/// The gallery's white-paper chrome: the documented look with a navy
/// title, a slate frame, a pale major grid and smaller text. Tick labels and
/// the legend keep the documented black.
fn gallery(kind: ChartKind) -> Theme {
    let mut theme = Theme::documented(kind);
    let chrome = &mut theme.chrome;
    chrome.ink = rgb(24, 39, 63);
    chrome.frame_ink = rgb(88, 102, 123);
    chrome.title_size = 26.0;
    chrome.axis_title_size = 18.0;
    chrome.tick_label_size = 16.0;
    chrome.grid.major_x.color = rgb(230, 235, 242);
    chrome.grid.major_y.color = rgb(230, 235, 242);
    chrome.colorbar.gap = 24.0;
    theme
}

/// The designed scenes' chrome on `bg`, in `ink`, with a horizontal grid in
/// `grid`: no axis lines or ticks, large text, an opaque legend in the
/// background colour and a slim colourbar without ticks.
fn scene(kind: ChartKind, bg: Color, ink: Color, grid: Color) -> Theme {
    let mut theme = Theme::documented(kind);
    let chrome = &mut theme.chrome;
    chrome.background = Some(bg);
    chrome.axis_preset = AxisPreset::Minimal;
    chrome.ink = ink;
    chrome.frame_ink = ink;
    chrome.label_ink = ink;
    chrome.title_size = 34.0;
    chrome.axis_title_size = 23.0;
    chrome.tick_label_size = 21.0;
    chrome.legend_size = 22.0;
    chrome.grid.major_x.show = false;
    chrome.grid.major_x.color = grid;
    chrome.grid.major_y.show = true;
    chrome.grid.major_y.color = grid;
    chrome.grid.major_y.width = 1.0;
    chrome.grid.major_y.style = LineStylePreset::Solid;
    chrome.grid.minor_x.show = false;
    chrome.grid.minor_y.show = false;
    chrome.legend_fill = bg;
    chrome.legend_border = bg;
    chrome.legend_padding = 15.0;
    let bar = &mut chrome.colorbar;
    bar.border = alpha(ink, 0.25);
    bar.border_width = 1.0;
    bar.thickness = 16.0;
    bar.length = 0.6;
    bar.gap = 24.0;
    bar.ticks = crate::config::TickVisibility::None;
    bar.label_size = 18.0;
    theme
}

/// The legacy documents' chrome (`Config::from_categorical`,
/// `Config::from_boxplot`) in `text`: a transparent background, the left
/// and bottom axis lines in the text colour without ticks, every text in
/// the text colour at 18 px with a title half as large again, and a pale
/// horizontal grid.
fn document(kind: ChartKind, text: Color) -> Theme {
    let mut theme = Theme::documented(kind);
    let chrome = &mut theme.chrome;
    // An open frame without its ticks: lines on the labelled sides only,
    // the tick labels where the hidden ticks leave them.
    chrome.axis_preset = AxisPreset::OpenOutward;
    chrome.ticks = false;
    chrome.axis_width = 1.0;
    chrome.ink = text;
    chrome.frame_ink = text;
    chrome.label_ink = text;
    chrome.title_size = 27.0;
    chrome.axis_title_size = 18.0;
    chrome.tick_label_size = 18.0;
    chrome.legend_size = 18.0;
    let grid = &mut chrome.grid;
    grid.major_x.show = false;
    grid.major_x.color = rgb(224, 229, 235);
    grid.major_y.show = true;
    grid.major_y.color = rgb(224, 229, 235);
    grid.minor_x.show = false;
    grid.minor_y.show = false;
    theme
}

/// The cool-slate family's paper, ink and grid.
const COOL_BG: Color = rgb(247, 248, 253);
const COOL_INK: Color = rgb(52, 57, 85);
const COOL_GRID: Color = rgb(228, 231, 240);
/// The midnight family's screen, ink and grid.
const NIGHT_BG: Color = rgb(14, 22, 38);
const NIGHT_INK: Color = rgb(182, 199, 222);
const NIGHT_GRID: Color = rgb(36, 49, 70);
/// The warm-paper family's paper, ink and grid.
const WARM_BG: Color = rgb(252, 249, 244);
const WARM_INK: Color = rgb(70, 60, 56);
const WARM_GRID: Color = rgb(235, 228, 216);

/// The gallery family's ink: the gallery's navy title.
const GALLERY_INK: Color = rgb(24, 39, 63);

/// The families' series colours, in cycle order.
const GALLERY_PALETTE: [Color; 6] = [
    rgb(34, 104, 198),
    rgb(225, 81, 79),
    rgb(224, 123, 38),
    rgb(19, 154, 139),
    rgb(126, 87, 194),
    rgb(98, 112, 134),
];
const COOL_PALETTE: [Color; 6] = [
    rgb(37, 150, 160),
    rgb(232, 104, 84),
    rgb(112, 92, 212),
    rgb(228, 170, 50),
    rgb(70, 130, 206),
    rgb(150, 156, 174),
];
const NIGHT_PALETTE: [Color; 6] = [
    rgb(65, 227, 194),
    rgb(139, 154, 255),
    rgb(242, 186, 78),
    rgb(240, 128, 150),
    rgb(96, 208, 232),
    rgb(196, 160, 250),
];
const WARM_PALETTE: [Color; 6] = [
    rgb(204, 88, 64),
    rgb(45, 140, 135),
    rgb(118, 92, 196),
    rgb(212, 150, 48),
    rgb(70, 128, 70),
    rgb(160, 110, 90),
];

/// `family`'s series colours.
fn palette(family: ThemeFamily) -> Vec<Color> {
    match family {
        ThemeFamily::Gallery => GALLERY_PALETTE,
        ThemeFamily::CoolSlate => COOL_PALETTE,
        ThemeFamily::Midnight => NIGHT_PALETTE,
        ThemeFamily::WarmPaper => WARM_PALETTE,
    }
    .to_vec()
}

/// A colour map through `stops`.
fn ramp<const N: usize>(stops: [Color; N]) -> ColorMap {
    ColorMap::Custom {
        stops: stops.to_vec(),
    }
}

/// `family`'s colour map. Each ramp runs from the end that sits quietly
/// on the family's paper to the end that stands out, and the midnight ramp
/// starts at a blue that still stands off its screen.
fn colormap(family: ThemeFamily) -> ColorMap {
    match family {
        ThemeFamily::Gallery => ColorMap::Viridis,
        ThemeFamily::CoolSlate => ramp([
            rgb(237, 248, 217),
            rgb(199, 233, 180),
            rgb(127, 205, 187),
            rgb(65, 182, 196),
            rgb(29, 145, 192),
            rgb(34, 94, 168),
            rgb(12, 44, 132),
        ]),
        ThemeFamily::Midnight => ramp([
            rgb(52, 78, 170),
            rgb(50, 112, 186),
            rgb(38, 168, 180),
            rgb(110, 210, 150),
            rgb(246, 226, 110),
        ]),
        ThemeFamily::WarmPaper => ramp([
            rgb(250, 236, 204),
            rgb(242, 196, 128),
            rgb(220, 132, 76),
            rgb(170, 70, 58),
            rgb(96, 40, 56),
        ]),
    }
}

/// `family`'s colour map for marks: markers, arrows, rows and surfaces
/// coloured by value. Unlike a field's map, every stop stands off the
/// family's paper.
fn mark_colormap(family: ThemeFamily) -> ColorMap {
    match family {
        ThemeFamily::CoolSlate => ramp([
            rgb(244, 190, 72),
            rgb(238, 110, 92),
            rgb(170, 70, 160),
            rgb(70, 60, 160),
            rgb(24, 32, 96),
        ]),
        ThemeFamily::WarmPaper => ramp([
            rgb(230, 170, 70),
            rgb(214, 110, 64),
            rgb(170, 60, 70),
            rgb(100, 50, 110),
            rgb(45, 40, 90),
        ]),
        ThemeFamily::Gallery | ThemeFamily::Midnight => colormap(family),
    }
}

/// `family`'s chrome for `kind`, the base of the members designed for the
/// family. The scene families write contour labels in their ink.
fn family_chrome(family: ThemeFamily, kind: ChartKind) -> Theme {
    match family {
        ThemeFamily::Gallery => gallery(kind),
        ThemeFamily::CoolSlate => cool_slate(kind),
        ThemeFamily::Midnight => midnight(kind),
        ThemeFamily::WarmPaper => warm_paper(kind),
    }
}

/// The scene chrome ([`scene`]) with contour labels in its ink.
fn inked_scene(kind: ChartKind, bg: Color, ink: Color, grid: Color) -> Theme {
    let mut theme = scene(kind, bg, ink, grid);
    theme.series.fields.contour_label_color = ink;
    theme
}

/// The light dashboard: slate ink on a cool off-white.
fn cool_slate(kind: ChartKind) -> Theme {
    inked_scene(kind, COOL_BG, COOL_INK, COOL_GRID)
}

/// The dark screen: pale ink on midnight navy; cells a map cannot place
/// stay unpainted.
fn midnight(kind: ChartKind) -> Theme {
    let mut theme = inked_scene(kind, NIGHT_BG, NIGHT_INK, NIGHT_GRID);
    theme.series.fields.nan_color = Color::new(0.0, 0.0, 0.0, 0.0);
    theme
}

/// Warm cream paper and brown ink.
fn warm_paper(kind: ChartKind) -> Theme {
    inked_scene(kind, WARM_BG, WARM_INK, WARM_GRID)
}

/// A theme whose field covers the plot: no grid over it.
fn covered(mut theme: Theme) -> Theme {
    theme.chrome.grid.major_x.show = false;
    theme.chrome.grid.major_y.show = false;
    theme
}

/// A theme whose grid runs both ways: a box floor, a disc's spokes and
/// rings, a triangle's three families of lines.
fn gridded(mut theme: Theme) -> Theme {
    theme.chrome.grid.major_x.show = true;
    theme.chrome.grid.major_y.show = true;
    theme
}

/// A theme without any grid line.
fn gridless(mut theme: Theme) -> Theme {
    let grid = &mut theme.chrome.grid;
    for line in [
        &mut grid.major_x,
        &mut grid.major_y,
        &mut grid.minor_x,
        &mut grid.minor_y,
    ] {
        line.show = false;
    }
    theme
}
