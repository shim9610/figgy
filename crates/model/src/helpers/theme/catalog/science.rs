//! Energy-level and Feynman themes.

use super::super::{ChartKind, TextLook, Theme};
use super::{alpha, gallery, rgb, scene};
use crate::color::Color;
use crate::feynman::Wiggle;

pub(super) const ENERGY_LEVELS: &[&str] = &["default", "grotrian"];
pub(super) const FEYNMAN: &[&str] = &["default", "parchment"];

/// The theme `name` of `kind`'s catalogue, past `default`.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    match (kind, name) {
        (ChartKind::EnergyLevels, "grotrian") => Some(grotrian()),
        (ChartKind::Feynman, "parchment") => Some(parchment()),
        _ => None,
    }
}

fn text(color: Color, size: f32) -> Option<TextLook> {
    Some(TextLook { color, size })
}

/// A Grotrian diagram as a paper prints it: navy levels, orange transition
/// arrows with their wavelengths in ochre, grey level labels, an indigo
/// limit under a pale blue continuum and navy headers over the term
/// columns, in the Grotrian frame on white paper.
fn grotrian() -> Theme {
    let mut theme = gallery(ChartKind::EnergyLevels);
    let chrome = &mut theme.chrome;
    // Read at a column's width: ticks at 18, titles at 20.
    chrome.axis_title_size = 20.0;
    chrome.tick_label_size = 18.0;
    let grid = &mut chrome.grid;
    for line in [
        &mut grid.major_x,
        &mut grid.major_y,
        &mut grid.minor_x,
        &mut grid.minor_y,
    ] {
        line.show = false;
    }
    theme.series.palette = vec![rgb(34, 52, 88)];
    let label = rgb(70, 76, 90);
    let levels = &mut theme.series.energy_levels;
    levels.level_width = 2.2;
    levels.transition_color = rgb(214, 120, 20);
    levels.transition_width = 1.3;
    levels.level_labels = text(label, 16.0);
    levels.band_labels = text(label, 16.0);
    levels.transition_labels = text(rgb(150, 90, 0), 15.0);
    levels.limit_labels = text(rgb(40, 50, 120), 16.0);
    levels.group_headers = text(rgb(24, 39, 63), 19.0);
    levels.column_labels = text(rgb(30, 30, 30), 20.0);
    levels.continuum_fill = Some(alpha(rgb(80, 100, 220), 36.0 / 255.0));
    theme
}

/// Diagrams on a parchment card table: ink fermions with bold arrowheads,
/// blue photons, green gluons, an orange W and violet scalars, ink labels,
/// no frame and no grid. The legend box is a white card with a sand
/// border, the look of the cards a host lays under its diagrams.
fn parchment() -> Theme {
    let bg = rgb(244, 241, 234);
    let ink = rgb(38, 42, 58);
    let mut theme = scene(ChartKind::Feynman, bg, ink, bg);
    let chrome = &mut theme.chrome;
    chrome.legend_fill = Color::WHITE;
    chrome.legend_border = rgb(222, 216, 204);
    chrome.legend_size = 20.0;
    chrome.grid.major_y.show = false;
    theme.series.palette = vec![ink];
    let feynman = &mut theme.series.feynman;
    feynman.width = 2.2;
    feynman.wave = Wiggle {
        amplitude: 5.0,
        wavelength: 15.0,
    };
    feynman.coil = Wiggle {
        amplitude: 6.0,
        wavelength: 10.0,
    };
    feynman.head_length = 11.0;
    feynman.head_width = 8.0;
    feynman.dot_radius = 3.5;
    feynman.photon = Some(rgb(36, 110, 200));
    feynman.gluon = Some(rgb(24, 140, 92));
    feynman.boson = Some(rgb(222, 118, 36));
    feynman.scalar = Some(rgb(150, 70, 178));
    feynman.vertex_labels = text(ink, 22.0);
    feynman.line_labels = text(ink, 21.0);
    theme
}
