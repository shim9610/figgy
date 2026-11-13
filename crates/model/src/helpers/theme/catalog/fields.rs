//! Field, vector, 3D, mesh, polar, radar, parallel and ternary themes.

use super::super::{ChartKind, Theme};
use super::{alpha, document, gallery, rgb, scene};
use crate::color::Color;
use crate::colormap::ColorMap;
use crate::material::Material;
use crate::preset::AxisPreset;

pub(super) const FIELD: &[&str] = &[
    "default",
    "gallery",
    "warm-neutral",
    "deep-space",
    "topographic",
];
pub(super) const VECTOR_FIELD: &[&str] = &["default", "gallery", "midnight", "ocean"];
pub(super) const SURFACE_3D: &[&str] = &["default", "warm-terrain", "warm-diverging", "midnight"];
pub(super) const TRI_MESH: &[&str] = &["default", "midnight"];
pub(super) const POLAR: &[&str] = &["default", "gallery", "cool-slate", "warm-paper"];
pub(super) const RADAR: &[&str] = &["default", "document"];
pub(super) const PARALLEL: &[&str] = &["default", "midnight"];
pub(super) const TERNARY: &[&str] = &["default", "warm-paper"];

/// The theme `name` of `kind`'s catalogue, past `default`.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    match (kind, name) {
        (ChartKind::Field, "gallery") => Some(field_gallery()),
        (ChartKind::Field, "warm-neutral") => Some(field_warm_neutral()),
        (ChartKind::Field, "deep-space") => Some(field_deep_space()),
        (ChartKind::Field, "topographic") => Some(field_topographic()),
        (ChartKind::VectorField, "gallery") => Some(vector_field_gallery()),
        (ChartKind::VectorField, "midnight") => Some(vector_field_midnight()),
        (ChartKind::VectorField, "ocean") => Some(vector_field_ocean()),
        (ChartKind::Surface3d, "warm-terrain") => Some(surface_3d_warm_terrain()),
        (ChartKind::Surface3d, "warm-diverging") => Some(surface_3d_warm_diverging()),
        (ChartKind::Surface3d, "midnight") => Some(surface_3d_midnight()),
        (ChartKind::TriMesh, "midnight") => Some(tri_mesh_midnight()),
        (ChartKind::Polar, "gallery") => Some(polar_gallery()),
        (ChartKind::Polar, "cool-slate") => Some(polar_cool_slate()),
        (ChartKind::Polar, "warm-paper") => Some(polar_warm_paper()),
        (ChartKind::Radar, "document") => Some(radar_document()),
        (ChartKind::Parallel, "midnight") => Some(parallel_midnight()),
        (ChartKind::Ternary, "warm-paper") => Some(ternary_warm_paper()),
        _ => None,
    }
}

fn ramp<const N: usize>(stops: [Color; N]) -> ColorMap {
    ColorMap::Custom {
        stops: stops.to_vec(),
    }
}

/// The scene chrome ([`scene`]) with contour labels in its ink: the
/// documented black is unreadable on the dark scenes.
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

/// A scene whose grid runs both ways: a box floor, a disc's spokes and
/// rings, a triangle's three families of lines.
fn gridded(mut theme: Theme) -> Theme {
    theme.chrome.grid.major_x.show = true;
    theme.chrome.grid.major_y.show = true;
    theme
}

/// The gallery's contour field: Viridis under white isolines with white
/// labels, no grid, and a colourbar titled like an axis.
fn field_gallery() -> Theme {
    let mut theme = covered(gallery(ChartKind::Field));
    theme.chrome.colorbar.title_size = theme.chrome.axis_title_size;
    let fields = &mut theme.series.fields;
    fields.colormap = ColorMap::Viridis;
    fields.contour_color = Some(alpha(Color::WHITE, 0.9));
    fields.contour_width = 1.3;
    fields.contour_label_color = Color::WHITE;
    fields.contour_label_size = 13.0;
    fields.contour_label_padding = 3.0;
    theme
}

/// The hexbin density map: slate ink on a warm off-white, a mint to
/// indigo ramp, empty cells unpainted.
fn field_warm_neutral() -> Theme {
    let mut theme = covered(inked_scene(
        ChartKind::Field,
        rgb(250, 250, 247),
        rgb(48, 52, 66),
        rgb(234, 235, 238),
    ));
    theme.chrome.colorbar.title_size = 19.0;
    let fields = &mut theme.series.fields;
    fields.colormap = ramp([
        rgb(226, 238, 228),
        rgb(156, 210, 188),
        rgb(72, 166, 170),
        rgb(40, 106, 150),
        rgb(44, 50, 112),
        rgb(30, 16, 58),
    ]);
    fields.nan_color = Color::new(0.0, 0.0, 0.0, 0.0);
    theme
}

/// The astronomical image: pale ink on deep space, no grid, a legend
/// without a box, and a red and a cyan for the channels a legend names.
fn field_deep_space() -> Theme {
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
fn field_topographic() -> Theme {
    let bg = rgb(12, 40, 43);
    let mut theme = covered(inked_scene(ChartKind::Field, bg, rgb(222, 230, 197), bg));
    let chrome = &mut theme.chrome;
    // The colourbar's axis line and ticks.
    chrome.frame_ink = rgb(164, 185, 120);
    let bar = Theme::documented(ChartKind::Field).chrome.colorbar;
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

/// The gallery's quiver: black arrows, coloured through Viridis when they
/// carry a value, over a muted field on white paper.
fn vector_field_gallery() -> Theme {
    let mut theme = gallery(ChartKind::VectorField);
    theme.chrome.colorbar.title_size = theme.chrome.axis_title_size;
    theme.series.palette = vec![Color::BLACK];
    theme.series.fields.colormap = ColorMap::Viridis;
    theme.series.fields.fill_opacity = Some(0.35);
    let vectors = &mut theme.series.vectors;
    vectors.width_px = 1.6;
    vectors.head_length_px = 6.0;
    vectors.head_width_px = 5.0;
    theme
}

/// The night wind map: near-white arrows over a navy to mint speed field,
/// pale ink on midnight, no grid.
fn vector_field_midnight() -> Theme {
    let bg = rgb(14, 22, 38);
    let mut theme = covered(inked_scene(
        ChartKind::VectorField,
        bg,
        rgb(182, 199, 222),
        bg,
    ));
    theme.chrome.colorbar.title_size = 19.0;
    theme.series.palette = vec![alpha(rgb(240, 246, 255), 0.9)];
    theme.series.fields.colormap = ramp([
        bg,
        rgb(25, 62, 96),
        rgb(33, 122, 142),
        rgb(72, 190, 170),
        rgb(214, 240, 196),
    ]);
    let vectors = &mut theme.series.vectors;
    vectors.width_px = 1.8;
    vectors.head_length_px = 7.0;
    vectors.head_width_px = 6.0;
    theme
}

/// The ocean current chart: bold arrows coloured from deep blue to yellow,
/// sea-green ink on dark teal with a full grid.
fn vector_field_ocean() -> Theme {
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
fn surface_3d_warm_terrain() -> Theme {
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
fn surface_3d_warm_diverging() -> Theme {
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

/// The night flow volume: bold arrows coloured from blue through green to
/// orange, pale ink on midnight with a floor grid.
fn surface_3d_midnight() -> Theme {
    let ink = rgb(186, 198, 220);
    let mut theme = gridded(inked_scene(
        ChartKind::Surface3d,
        rgb(14, 20, 34),
        ink,
        rgb(40, 52, 74),
    ));
    theme.chrome.colorbar.title_size = 19.0;
    theme.series.palette = vec![ink];
    theme.series.fields.colormap = ramp([
        rgb(52, 84, 160),
        rgb(64, 160, 190),
        rgb(132, 216, 160),
        rgb(244, 228, 120),
        rgb(246, 132, 84),
    ]);
    let vectors = &mut theme.series.vectors;
    vectors.width_px = 2.4;
    vectors.head_length_px = 10.0;
    vectors.head_width_px = 8.0;
    theme
}

/// The stress map: a navy to red ramp under faint white isolines, mesh
/// edges as hairlines, pale ink on midnight, no grid.
fn tri_mesh_midnight() -> Theme {
    let bg = rgb(16, 18, 30);
    let mut theme = covered(inked_scene(ChartKind::TriMesh, bg, rgb(192, 198, 220), bg));
    theme.chrome.colorbar.title_size = 19.0;
    // The isolines, then the mesh edges drawn over them.
    theme.series.palette = vec![alpha(Color::WHITE, 0.55), alpha(Color::WHITE, 0.16)];
    theme.series.lines.line_width = 0.7;
    let fields = &mut theme.series.fields;
    fields.colormap = ramp([
        rgb(20, 24, 66),
        rgb(52, 60, 150),
        rgb(40, 140, 170),
        rgb(120, 200, 140),
        rgb(246, 220, 100),
        rgb(244, 120, 60),
        rgb(200, 40, 50),
    ]);
    fields.contour_width = 1.2;
    theme
}

/// The gallery's polar plots: blue, red, green and violet lines with a
/// light fill and round markers on white paper, near-black isolines over
/// a field.
fn polar_gallery() -> Theme {
    let dark = rgb(25, 25, 25);
    let mut theme = gallery(ChartKind::Polar);
    theme.series.palette = vec![
        rgb(30, 110, 210),
        rgb(200, 60, 40),
        rgb(40, 150, 90),
        rgb(150, 60, 170),
    ];
    let lines = &mut theme.series.lines;
    lines.line_width = 2.5;
    lines.fill_opacity = Some(0.25);
    lines.marker_size = 7.0;
    // A disc leaves the bar room: the documented gap, not the gallery's.
    theme.chrome.colorbar.gap = Theme::documented(ChartKind::Polar).chrome.colorbar.gap;
    let fields = &mut theme.series.fields;
    fields.contour_color = Some(dark);
    fields.contour_width = 1.2;
    fields.contour_label_color = dark;
    fields.contour_label_size = 12.0;
    theme
}

/// The directional spectrum: a pale green to navy ramp in annular cells
/// under dark labelled isolines, slate ink on a cool white, spokes and
/// rings over the field.
fn polar_cool_slate() -> Theme {
    let dark = rgb(20, 28, 48);
    let mut theme = gridded(inked_scene(
        ChartKind::Polar,
        rgb(249, 250, 252),
        rgb(44, 56, 78),
        rgb(214, 222, 232),
    ));
    theme.chrome.colorbar.title_size = 19.0;
    let fields = &mut theme.series.fields;
    fields.colormap = ramp([
        rgb(237, 248, 217),
        rgb(199, 233, 180),
        rgb(127, 205, 187),
        rgb(65, 182, 196),
        rgb(29, 145, 192),
        rgb(34, 94, 168),
        rgb(12, 44, 132),
    ]);
    fields.nan_color = Color::new(0.0, 0.0, 0.0, 0.0);
    fields.contour_color = Some(alpha(dark, 0.85));
    fields.contour_width = 1.4;
    fields.contour_label_color = dark;
    fields.contour_label_size = 15.0;
    fields.contour_label_padding = 3.0;
    theme
}

/// The wind rose: flat bars from sand through orange to dark red with
/// small round corners, brown ink on cream paper, spokes and rings.
fn polar_warm_paper() -> Theme {
    let mut theme = gridded(inked_scene(
        ChartKind::Polar,
        rgb(252, 249, 244),
        rgb(70, 60, 56),
        rgb(229, 221, 209),
    ));
    theme.series.palette = vec![
        rgb(253, 212, 140),
        rgb(250, 152, 72),
        rgb(222, 88, 44),
        rgb(140, 36, 30),
    ];
    let bars = &mut theme.series.bars;
    bars.corner_radius = 3.0;
    bars.gloss = 0.0;
    bars.texture_strength = 0.0;
    theme
}

/// A radar document: blue, orange, green and grey outlines with a light
/// fill and round markers on a transparent background, pale rings and
/// spokes, the text in navy.
fn radar_document() -> Theme {
    let rings = rgb(224, 229, 235);
    let mut theme = document(ChartKind::Radar, rgb(32, 46, 60));
    let chrome = &mut theme.chrome;
    // No axis lines: the rings and spokes are the grid's.
    chrome.axis_preset = AxisPreset::Minimal;
    chrome.frame_ink = rings;
    chrome.grid.major_x.show = true;
    chrome.grid.major_x.color = rgb(200, 206, 214);
    chrome.grid.major_y.color = rings;
    theme.series.palette = vec![
        rgb(65, 144, 208),
        rgb(230, 120, 64),
        rgb(42, 168, 120),
        rgb(120, 120, 130),
    ];
    let radar = &mut theme.series.radar;
    radar.line_width_px = Some(2.0);
    radar.marker_size_px = Some(7.0);
    radar.fill_opacity = Some(0.25);
    theme
}

/// The dark sweep: thin rows coloured from indigo through magenta to
/// yellow, slate axes with outward ticks and small labels under larger
/// axis names, pale ink on midnight, a slim colourbar.
fn parallel_midnight() -> Theme {
    let bg = rgb(15, 18, 30);
    let ink = rgb(186, 194, 216);
    let mut theme = covered(inked_scene(ChartKind::Parallel, bg, ink, bg));
    let chrome = &mut theme.chrome;
    chrome.axis_preset = AxisPreset::OpenOutward;
    chrome.frame_ink = rgb(96, 104, 130);
    chrome.tick_label_size = 16.0;
    chrome.colorbar.thickness = 12.0;
    chrome.colorbar.length = 0.3;
    chrome.colorbar.gap = 16.0;
    chrome.colorbar.title_size = 19.0;
    theme.series.parallel.name_size = 20.0;
    theme.series.palette = vec![ink];
    theme.series.lines.line_width = 1.2;
    theme.series.fields.colormap = ramp([
        rgb(38, 42, 72),
        rgb(62, 52, 104),
        rgb(120, 54, 146),
        rgb(214, 70, 120),
        rgb(250, 136, 84),
        rgb(255, 220, 120),
    ]);
    theme
}

/// The soil texture triangle: twelve earth-toned regions outlined in the
/// paper colour, navy, crimson and green lines through their markers,
/// taupe axes with outward ticks, brown ink on cream paper.
fn ternary_warm_paper() -> Theme {
    let bg = rgb(252, 250, 245);
    let ink = rgb(62, 56, 50);
    let mut theme = gridded(inked_scene(ChartKind::Ternary, bg, ink, rgb(226, 220, 208)));
    let chrome = &mut theme.chrome;
    chrome.axis_preset = AxisPreset::OpenOutward;
    chrome.frame_ink = rgb(150, 140, 128);
    chrome.label_ink = rgb(120, 110, 100);
    chrome.legend_ink = Some(ink);
    chrome.tick_label_size = 16.0;
    theme.series.palette = vec![rgb(36, 74, 140), rgb(150, 40, 70), rgb(18, 112, 96)];
    theme.series.lines.line_width = 2.2;
    let ternary = &mut theme.series.ternary;
    ternary.regions = vec![
        rgb(214, 160, 140),
        rgb(196, 162, 178),
        rgb(226, 178, 132),
        rgb(206, 182, 160),
        rgb(184, 178, 190),
        rgb(232, 196, 146),
        rgb(200, 196, 150),
        rgb(178, 200, 168),
        rgb(158, 194, 176),
        rgb(236, 214, 150),
        rgb(242, 224, 166),
        rgb(247, 234, 186),
    ];
    ternary.region_opacity = 0.75;
    ternary.region_outline = Some((bg, 2.0));
    theme
}
