//! The built-in themes, one catalogue per chart kind (#291).
//!
//! Every theme is a gallery look: the values the gallery images use. Bases
//! carry the chrome several looks share, and each kind's themes add their
//! palette and their family's part:
//! - [`gallery`]: white paper, a navy title, a slate frame and a pale grid;
//! - [`scene`]: the designed scenes' chrome, an opaque background with no
//!   axis lines or ticks, a horizontal grid and large text;
//! - [`document`]: the legacy documents' chrome, a transparent background,
//!   an open frame without ticks in the text colour and a pale value grid.

use super::{ChartKind, RadialTheme, Theme};
use crate::color::Color;
use crate::line::LineStylePreset;
use crate::preset::AxisPreset;

mod bars;
mod fields;
mod radial;
mod science;
mod xy;

/// The names in `kind`'s catalogue, `default` first.
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

/// The theme `name` of `kind`'s catalogue.
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

/// The names in the radial catalogue, `default` first.
pub(super) fn radial_names() -> &'static [&'static str] {
    radial::NAMES
}

/// The radial theme `name`.
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
    // An open frame whose ticks have no length: lines on the labelled
    // sides only.
    chrome.axis_preset = AxisPreset::OpenOutward;
    chrome.major_tick_length = 0.0;
    chrome.minor_tick_length = 0.0;
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
