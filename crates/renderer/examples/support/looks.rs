//! The gallery's scene-specific looks: chart themes built as values for one
//! picture each (#295). The public way to theme a chart is a family
//! (`ThemeFamily`: `gallery`, `cool-slate`, `midnight`, `warm-paper`); a
//! look that is not a family member is a custom `Theme` the example builds
//! here and applies with `Chart::apply_custom_theme`, or a `RadialTheme`
//! applied with `RadialTheme::apply`.
//!
//! [`documented`] is the documented default look of a kind; every look
//! starts from it.
#![allow(dead_code)]

use renderer::config::{DrawStyle, MilkywayOptions, SketchOptions, TickVisibility};
use renderer::line::LineStylePreset;
use renderer::theme::{ChartKind, RadialTheme, Theme};
use renderer::{AxisPreset, Color, ColorMap, Material};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

/// `color` at opacity `a`.
const fn alpha(color: Color, a: f32) -> Color {
    Color::new(color.r, color.g, color.b, a)
}

fn ramp<const N: usize>(stops: [Color; N]) -> ColorMap {
    ColorMap::Custom {
        stops: stops.to_vec(),
    }
}

/// The documented default look of `kind` (`Theme::default()` is the line
/// chart's): applying it to a default document changes nothing.
pub fn documented(kind: ChartKind) -> Theme {
    let mut theme = Theme::default();
    theme.kind = kind;
    theme
}

/// The designed scenes' chrome on `bg`, in `ink`, with a horizontal grid in
/// `grid`: no axis lines or ticks, large text, an opaque legend in the
/// background colour and a slim colourbar without ticks.
fn scene(kind: ChartKind, bg: Color, ink: Color, grid: Color) -> Theme {
    let mut theme = documented(kind);
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
    bar.ticks = TickVisibility::None;
    bar.label_size = 18.0;
    theme
}

/// The scene chrome with contour labels in its ink.
fn inked_scene(kind: ChartKind, bg: Color, ink: Color, grid: Color) -> Theme {
    let mut theme = scene(kind, bg, ink, grid);
    theme.series.fields.contour_label_color = ink;
    theme
}

/// A scene whose field covers the plot: no grid over it.
fn covered(mut theme: Theme) -> Theme {
    theme.chrome.grid.major_x.show = false;
    theme.chrome.grid.major_y.show = false;
    theme
}

/// A scene whose grid runs both ways.
fn gridded(mut theme: Theme) -> Theme {
    theme.chrome.grid.major_x.show = true;
    theme.chrome.grid.major_y.show = true;
    theme
}

/// The legacy documents' chrome in `text`: a transparent background, an
/// open frame without ticks in the text colour, 18 px text and a pale
/// horizontal grid.
fn document(kind: ChartKind, text: Color) -> Theme {
    let mut theme = documented(kind);
    let chrome = &mut theme.chrome;
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

/// The warm editorial chart: hand-drawn strokes and round markers on
/// newsprint.
pub fn editorial() -> Theme {
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
    let mut theme = documented(ChartKind::Line);
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

/// The milky-way preview: the night sky drawn in `DrawStyle::Milkyway`.
pub fn milkyway() -> Theme {
    let documented = MilkywayOptions::default();
    night_sky(DrawStyle::Milkyway(MilkywayOptions {
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
    }))
}

/// The constellation preview: the night sky drawn in
/// `DrawStyle::Constellation`.
pub fn constellation() -> Theme {
    night_sky(DrawStyle::Constellation(Default::default()))
}

/// The product analytics card: matte violet, lavender and teal bars with
/// well-rounded ends and no texture, slate text on a cool off-white, no
/// grid and a legend without a box.
pub fn revenue() -> Theme {
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

/// The astronomical image: pale ink on deep space, no grid, a legend
/// without a box, and a red and a cyan for the channels a legend names.
pub fn deep_space() -> Theme {
    let bg = rgb(8, 10, 18);
    let mut theme = covered(inked_scene(ChartKind::Field, bg, rgb(170, 180, 202), bg));
    let chrome = &mut theme.chrome;
    chrome.legend_fill = alpha(bg, 0.0);
    chrome.legend_border = alpha(bg, 0.0);
    chrome.legend_padding = 4.0;
    chrome.legend_size = 19.0;
    theme.series.palette = vec![rgb(236, 88, 64), rgb(60, 196, 214)];
    theme
}

/// The topographic poster: sand ink on deep teal, a teal to sand ramp
/// under pale isolines, and the documented colourbar in the poster's
/// colours.
pub fn topographic() -> Theme {
    let bg = rgb(12, 40, 43);
    let mut theme = covered(inked_scene(ChartKind::Field, bg, rgb(222, 230, 197), bg));
    let chrome = &mut theme.chrome;
    chrome.frame_ink = rgb(164, 185, 120);
    let bar = documented(ChartKind::Field).chrome.colorbar;
    chrome.colorbar.border = bar.border;
    chrome.colorbar.thickness = bar.thickness;
    chrome.colorbar.length = bar.length;
    chrome.colorbar.ticks = bar.ticks;
    let fields = &mut theme.series.fields;
    fields.colormap = ramp([
        bg,
        rgb(29, 89, 85),
        rgb(73, 132, 112),
        rgb(164, 185, 120),
        rgb(239, 221, 150),
    ]);
    fields.contour_color = Some(Color::new(0.92, 0.98, 0.8, 0.65));
    fields.contour_width = 1.1;
    theme
}

/// The ocean current chart: bold arrows coloured from deep blue to yellow,
/// sea-green ink on dark teal with a full grid.
pub fn ocean() -> Theme {
    let ink = rgb(170, 205, 205);
    let mut theme = gridded(inked_scene(
        ChartKind::VectorField,
        rgb(9, 30, 40),
        ink,
        rgb(19, 47, 58),
    ));
    theme.chrome.colorbar.title_size = 19.0;
    theme.series.palette = vec![ink];
    theme.series.fields.colormap = ramp([
        rgb(36, 92, 120),
        rgb(70, 170, 160),
        rgb(173, 226, 150),
        rgb(250, 230, 120),
    ]);
    let vectors = &mut theme.series.vectors;
    vectors.width_px = 2.2;
    vectors.head_length_px = 8.0;
    vectors.head_width_px = 7.0;
    theme
}

/// The relief map: a matte, finely wireframed surface in a lake to snow
/// ramp, bold floor contours in the ramp's colours darkened, a red trail
/// with round waypoints, slate ink on warm paper with a floor grid.
pub fn warm_terrain() -> Theme {
    let ink = rgb(58, 62, 72);
    let mut theme = gridded(inked_scene(
        ChartKind::Surface3d,
        rgb(248, 247, 243),
        ink,
        rgb(222, 220, 212),
    ));
    theme.chrome.colorbar.title_size = 19.0;
    theme.series.palette = vec![rgb(206, 44, 58)];
    theme.series.lines.line_width = 3.5;
    theme.series.lines.marker_size = 13.0;
    let fields = &mut theme.series.fields;
    fields.colormap = ramp([
        rgb(38, 84, 98),
        rgb(66, 140, 112),
        rgb(152, 186, 110),
        rgb(226, 206, 140),
        rgb(176, 124, 86),
        rgb(246, 244, 240),
    ]);
    fields.fill_opacity = Some(0.9);
    fields.contour_color = Some(ink);
    fields.contour_width = 2.6;
    fields.contour_shade = Some(0.72);
    let surfaces = &mut theme.series.surfaces;
    surfaces.material = Material::Matte;
    surfaces.wireframe = true;
    surfaces.wireframe_color = alpha(rgb(40, 44, 52), 0.2);
    surfaces.wireframe_width = 0.7;
    theme
}

/// The signed surface: a matte, finely wireframed surface in a blue to
/// red ramp through cream, floor contours in the ramp's colours darkened,
/// slate ink on warm paper with a floor grid.
pub fn warm_diverging() -> Theme {
    let ink = rgb(56, 60, 74);
    let mut theme = gridded(inked_scene(
        ChartKind::Surface3d,
        rgb(248, 247, 244),
        ink,
        rgb(222, 220, 214),
    ));
    theme.chrome.colorbar.title_size = 19.0;
    theme.series.palette = vec![ink];
    let fields = &mut theme.series.fields;
    fields.colormap = ramp([
        rgb(36, 72, 150),
        rgb(96, 150, 206),
        rgb(236, 232, 222),
        rgb(226, 132, 92),
        rgb(170, 40, 50),
    ]);
    fields.contour_color = Some(alpha(ink, 0.85));
    fields.contour_width = 2.2;
    fields.contour_shade = Some(0.7);
    let surfaces = &mut theme.series.surfaces;
    surfaces.material = Material::Matte;
    surfaces.wireframe = true;
    surfaces.wireframe_color = alpha(rgb(30, 34, 44), 0.25);
    surfaces.wireframe_width = 0.6;
    theme
}

/// A flat sunburst: green, rust and violet groups, each ring a paler step
/// of the one inside it, with hairline gaps and barely rounded corners.
pub fn sunburst() -> RadialTheme {
    let mut theme = RadialTheme::default();
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
