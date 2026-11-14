//! Radial (pie, donut, sunburst) themes.

use super::super::RadialTheme;
use super::rgb;
use crate::material::Material;

pub(super) const NAMES: &[&str] = &["default", "ceramic", "sunburst", "pop-art", "portfolio"];

/// The radial theme `name`, past `default`.
pub(super) fn named(name: &str) -> Option<RadialTheme> {
    match name {
        "ceramic" => Some(ceramic()),
        "sunburst" => Some(sunburst()),
        "pop-art" => Some(pop_art()),
        "portfolio" => Some(portfolio()),
        _ => None,
    }
}

/// The gallery's raised chart: glazed ceramic slices in blue, lime, slate,
/// orange and cyan, seen from an oblique camera with rounded corners, a
/// narrow gap and a soft shadow on white.
fn ceramic() -> RadialTheme {
    let mut theme = RadialTheme::documented();
    theme.palette = vec![
        rgb(51, 132, 245),
        rgb(165, 216, 72),
        rgb(80, 96, 121),
        rgb(255, 151, 62),
        rgb(48, 192, 230),
    ];
    theme.material = Material::Ceramic;
    theme.tilt_degrees = 55.0;
    theme.depth = 0.16;
    theme.bevel = 0.025;
    theme.gap_degrees = 1.4;
    theme.inner_corner = 0.045;
    theme.outer_corner = 0.045;
    theme.roughness = 0.3;
    theme.texture_strength = 0.12;
    theme.shadow = true;
    theme
}

/// A flat sunburst: green, rust and violet groups, each ring a paler step
/// of the one inside it, with hairline gaps and barely rounded corners.
fn sunburst() -> RadialTheme {
    let mut theme = RadialTheme::documented();
    theme.palette = vec![rgb(38, 140, 112), rgb(196, 112, 64), rgb(128, 110, 196)];
    theme.child_tint = Some(0.22);
    theme.label_color = rgb(40, 44, 52);
    theme.material = Material::Flat;
    theme.gloss = 0.15;
    theme.gap_degrees = 0.6;
    theme.inner_corner = 0.012;
    theme.outer_corner = 0.012;
    theme
}

/// The pop-art pie: toon slices in coral, periwinkle, yellow and mint with
/// a bold ink rim and separators, tilted on cream, large text.
fn pop_art() -> RadialTheme {
    let ink = rgb(49, 43, 59);
    let mut theme = RadialTheme::documented();
    theme.palette = vec![
        rgb(245, 103, 87),
        rgb(111, 126, 241),
        rgb(247, 191, 65),
        rgb(95, 196, 170),
    ];
    theme.background = rgb(255, 248, 237);
    theme.label_color = ink;
    theme.font_size = 23.0;
    theme.title_size = 34.5;
    theme.material = Material::Toon;
    theme.tilt_degrees = 45.0;
    theme.depth = 0.14;
    theme.bevel = 0.0;
    theme.gap_degrees = 2.0;
    theme.inner_corner = 0.0;
    theme.outer_corner = 0.02;
    theme.texture_strength = 0.0;
    theme.shadow = false;
    theme.outline.rim = true;
    theme.outline.separators = true;
    theme.outline.width = 2.0;
    theme.outline.color = ink;
    theme
}

/// The night portfolio: pearl slices in periwinkle, mint, rose and sand,
/// deep and rounded under an oblique camera, with a shadow on midnight and
/// pale text.
fn portfolio() -> RadialTheme {
    let mut theme = RadialTheme::documented();
    theme.palette = vec![
        rgb(145, 159, 244),
        rgb(99, 204, 192),
        rgb(236, 166, 187),
        rgb(218, 200, 146),
    ];
    theme.background = rgb(19, 22, 39);
    theme.label_color = rgb(218, 224, 241);
    theme.font_size = 23.0;
    theme.title_size = 34.5;
    theme.material = Material::Pearl;
    theme.gloss = 0.65;
    theme.roughness = 0.3;
    theme.texture_strength = 0.45;
    theme.texture_scale = 1.0;
    theme.tilt_degrees = 55.0;
    theme.depth = 0.19;
    theme.bevel = 0.025;
    theme.gap_degrees = 2.0;
    theme.inner_corner = 0.06;
    theme.outer_corner = 0.045;
    theme.shadow = true;
    theme
}
