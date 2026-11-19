//! Energy-level and Feynman themes.

use super::super::{ChartKind, EnergyLevelTheme, TextLook, Theme, ThemeFamily};
use super::{alpha, family_chrome, gallery, gridless, rgb, scene};
use crate::color::Color;
use crate::feynman::Wiggle;

/// `family`'s member for `kind`.
pub(super) fn member(family: ThemeFamily, kind: ChartKind) -> Theme {
    match (kind, family) {
        (ChartKind::EnergyLevels, ThemeFamily::Gallery) => energy_levels_gallery(),
        (ChartKind::Feynman, ThemeFamily::WarmPaper) => feynman_warm_paper(),
        (ChartKind::EnergyLevels, _) => energy_levels(family),
        _ => feynman(family),
    }
}

fn text(color: Color, size: f32) -> Option<TextLook> {
    Some(TextLook { color, size })
}

// The Grotrian members' text sizes (#357), one scale for every family. The
// term symbols over the columns are the diagram's headings, so they lead;
// the level labels read at the energy axis' tick size, and the wavelengths
// and band, limit and group texts a step below, so no text of the diagram
// is smaller than the chrome's beside it by more than that step.
const AXIS_TITLE_SIZE: f32 = 22.0;
const TICK_LABEL_SIZE: f32 = 20.0;
const COLUMN_LABEL_SIZE: f32 = 24.0;
const GROUP_HEADER_SIZE: f32 = 21.0;
const LEVEL_LABEL_SIZE: f32 = 20.0;
const BAND_LABEL_SIZE: f32 = 19.0;
const LIMIT_LABEL_SIZE: f32 = 19.0;
const TRANSITION_LABEL_SIZE: f32 = 18.0;

/// The chrome text of a Grotrian member: the axis title and the energy
/// ticks at the sizes above.
fn grotrian_text(theme: &mut Theme) {
    let chrome = &mut theme.chrome;
    chrome.axis_title_size = AXIS_TITLE_SIZE;
    chrome.tick_label_size = TICK_LABEL_SIZE;
}

/// The strokes of a Grotrian member (#357): 3 px levels, 2 px limits and
/// transition arrows with a 2.2 px shaft and a 13 × 10 px head, the weight
/// of a textbook term diagram's lines beside 20 px text.
fn grotrian_strokes(levels: &mut EnergyLevelTheme) {
    levels.level_width = 3.0;
    levels.limit_width = 2.0;
    levels.transition_width = 2.2;
    levels.head_length = Some(13.0);
    levels.head_width = Some(10.0);
}

/// A Grotrian diagram as a paper prints it: navy levels, orange transition
/// arrows with their wavelengths in ochre, grey level labels, an indigo
/// limit under a pale blue continuum and navy headers over the term
/// columns, in the Grotrian frame on white paper.
fn energy_levels_gallery() -> Theme {
    let mut theme = gallery(ChartKind::EnergyLevels);
    grotrian_text(&mut theme);
    let grid = &mut theme.chrome.grid;
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
    grotrian_strokes(levels);
    levels.transition_color = rgb(214, 120, 20);
    levels.level_labels = text(label, LEVEL_LABEL_SIZE);
    levels.band_labels = text(label, BAND_LABEL_SIZE);
    levels.transition_labels = text(rgb(150, 90, 0), TRANSITION_LABEL_SIZE);
    levels.limit_labels = text(rgb(40, 50, 120), LIMIT_LABEL_SIZE);
    levels.group_headers = text(rgb(24, 39, 63), GROUP_HEADER_SIZE);
    levels.column_labels = text(rgb(30, 30, 30), COLUMN_LABEL_SIZE);
    levels.continuum_fill = Some(alpha(rgb(80, 100, 220), 36.0 / 255.0));
    theme
}

/// Diagrams on a parchment card table: ink fermions with bold arrowheads,
/// blue photons, green gluons, an orange W and violet scalars, ink labels,
/// no frame and no grid. The legend box is a white card with a sand
/// border, the look of the cards a host lays under its diagrams.
fn feynman_warm_paper() -> Theme {
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

/// The colours of a family's Grotrian diagram.
struct LevelInks {
    levels: Color,
    transitions: Color,
    wavelengths: Color,
    labels: Color,
    limit: Color,
    limit_labels: Color,
    headers: Color,
    columns: Color,
    band: Color,
}

/// A family's Grotrian diagram (cool slate, midnight, warm paper), laid
/// out as the gallery's ([`energy_levels_gallery`]): levels in a strong ink, transition
/// arrows and their wavelengths in the family's accent, quieter level
/// labels, a limit under a faint continuum and headers over the term
/// columns, in the Grotrian frame without a grid.
fn energy_levels(family: ThemeFamily) -> Theme {
    let mut theme = gridless(family_chrome(family, ChartKind::EnergyLevels));
    let ink = match family {
        ThemeFamily::Midnight => LevelInks {
            levels: rgb(176, 196, 236),
            transitions: rgb(242, 186, 78),
            wavelengths: rgb(246, 206, 128),
            labels: rgb(150, 166, 192),
            limit: rgb(139, 154, 255),
            limit_labels: rgb(170, 180, 255),
            headers: rgb(206, 218, 236),
            columns: rgb(222, 230, 242),
            band: rgb(65, 227, 194),
        },
        ThemeFamily::WarmPaper => LevelInks {
            levels: rgb(84, 58, 46),
            transitions: rgb(204, 88, 64),
            wavelengths: rgb(150, 66, 40),
            labels: rgb(112, 98, 90),
            limit: rgb(45, 128, 124),
            limit_labels: rgb(32, 104, 100),
            headers: rgb(70, 60, 56),
            columns: rgb(50, 42, 38),
            band: rgb(212, 150, 48),
        },
        _ => LevelInks {
            levels: rgb(44, 62, 112),
            transitions: rgb(226, 98, 76),
            wavelengths: rgb(170, 64, 44),
            labels: rgb(90, 96, 120),
            limit: rgb(112, 92, 212),
            limit_labels: rgb(82, 64, 176),
            headers: rgb(52, 57, 85),
            columns: rgb(36, 40, 62),
            band: rgb(37, 150, 160),
        },
    };
    grotrian_text(&mut theme);
    theme.series.palette = vec![ink.levels];
    let levels = &mut theme.series.energy_levels;
    grotrian_strokes(levels);
    levels.limit_color = ink.limit;
    levels.transition_color = ink.transitions;
    levels.band_fill = alpha(ink.band, 0.16);
    levels.level_labels = text(ink.labels, LEVEL_LABEL_SIZE);
    levels.band_labels = text(ink.labels, BAND_LABEL_SIZE);
    levels.transition_labels = text(ink.wavelengths, TRANSITION_LABEL_SIZE);
    levels.limit_labels = text(ink.limit_labels, LIMIT_LABEL_SIZE);
    levels.group_headers = text(ink.headers, GROUP_HEADER_SIZE);
    levels.column_labels = text(ink.columns, COLUMN_LABEL_SIZE);
    levels.continuum_fill = Some(alpha(ink.limit, 0.16));
    theme
}

/// A family's Feynman diagrams (gallery, cool slate, midnight), drawn as
/// the parchment card's ([`feynman_warm_paper`]): fermions in the family's ink with
/// bold arrowheads, photons, gluons, the W and scalars each in a colour of
/// the family, ink labels, no frame and no grid, and a legend card a shade
/// off the paper.
fn feynman(family: ThemeFamily) -> Theme {
    let mut theme = gridless(family_chrome(family, ChartKind::Feynman));
    let ink = theme.chrome.ink;
    let (lines, card, border, [photon, gluon, boson, scalar]) = match family {
        ThemeFamily::Midnight => (
            rgb(222, 230, 244),
            rgb(22, 32, 52),
            rgb(52, 68, 96),
            [
                rgb(96, 208, 232),
                rgb(65, 227, 194),
                rgb(242, 186, 78),
                rgb(196, 160, 250),
            ],
        ),
        ThemeFamily::CoolSlate => (
            ink,
            Color::WHITE,
            rgb(214, 218, 232),
            [
                rgb(46, 116, 200),
                rgb(30, 140, 130),
                rgb(226, 112, 60),
                rgb(112, 92, 212),
            ],
        ),
        _ => (
            ink,
            Color::WHITE,
            rgb(214, 222, 232),
            [
                rgb(34, 104, 198),
                rgb(19, 140, 110),
                rgb(224, 123, 38),
                rgb(126, 87, 194),
            ],
        ),
    };
    let chrome = &mut theme.chrome;
    chrome.legend_fill = card;
    chrome.legend_border = border;
    theme.series.palette = vec![lines];
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
    feynman.photon = Some(photon);
    feynman.gluon = Some(gluon);
    feynman.boson = Some(boson);
    feynman.scalar = Some(scalar);
    let size = if family == ThemeFamily::Gallery {
        20.0
    } else {
        22.0
    };
    feynman.vertex_labels = text(lines, size);
    feynman.line_labels = text(lines, size - 1.0);
    theme
}
