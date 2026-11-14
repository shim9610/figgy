//! Field, vector, 3D, mesh, polar, radar, parallel and ternary themes.

use super::super::{ChartKind, Theme, ThemeFamily};
use super::{
    GALLERY_INK, alpha, colormap, covered, document, family_chrome, gallery, gridded, inked_scene,
    mark_colormap, palette, ramp, rgb,
};
use crate::color::Color;
use crate::colormap::ColorMap;
use crate::material::Material;
use crate::preset::AxisPreset;

/// `family`'s member for `kind`.
pub(super) fn member(family: ThemeFamily, kind: ChartKind) -> Theme {
    match (kind, family) {
        (ChartKind::Field, ThemeFamily::Gallery) => field_gallery(),
        (ChartKind::Field, ThemeFamily::WarmPaper) => field_warm_paper(),
        (ChartKind::VectorField, ThemeFamily::Gallery) => vector_field_gallery(),
        (ChartKind::VectorField, ThemeFamily::Midnight) => vector_field_midnight(),
        (ChartKind::Surface3d, ThemeFamily::Midnight) => surface_3d_midnight(),
        (ChartKind::TriMesh, ThemeFamily::Midnight) => tri_mesh_midnight(),
        (ChartKind::Polar, ThemeFamily::Gallery) => polar_gallery(),
        (ChartKind::Polar, ThemeFamily::CoolSlate) => polar_cool_slate(),
        (ChartKind::Polar, ThemeFamily::WarmPaper) => polar_warm_paper(),
        (ChartKind::Radar, ThemeFamily::Gallery) => radar_gallery(),
        (ChartKind::Parallel, ThemeFamily::Midnight) => parallel_midnight(),
        (ChartKind::Ternary, ThemeFamily::WarmPaper) => ternary_warm_paper(),
        (ChartKind::Field, _) => field(family),
        (ChartKind::VectorField, _) => vector_field(family),
        (ChartKind::Surface3d, _) => surface_3d(family),
        (ChartKind::TriMesh, _) => tri_mesh(family),
        (ChartKind::Polar, _) => polar_midnight(),
        (ChartKind::Radar, _) => radar(family),
        (ChartKind::Parallel, _) => parallel(family),
        _ => ternary(family),
    }
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
fn field_warm_paper() -> Theme {
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
    // The palest bar still stands off the paper (a contrast of 1.5).
    theme.series.palette = vec![
        rgb(247, 196, 112),
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
fn radar_gallery() -> Theme {
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

/// A field ink that stays readable on `family`'s maps: white on the
/// gallery's and midnight's, which run dark to light, the dark slate of the
/// cool-slate isolines on its pale to navy map, a deep brown on the warm
/// one.
fn isoline_ink(family: ThemeFamily) -> Color {
    match family {
        ThemeFamily::Gallery | ThemeFamily::Midnight => Color::WHITE,
        ThemeFamily::CoolSlate => rgb(20, 28, 48),
        ThemeFamily::WarmPaper => rgb(60, 30, 30),
    }
}

/// A family's contour field (cool slate, midnight): the family's map under
/// isolines with labels in the isoline ink, empty cells unpainted, no grid.
fn field(family: ThemeFamily) -> Theme {
    let mut theme = covered(family_chrome(family, ChartKind::Field));
    theme.chrome.colorbar.title_size = 19.0;
    theme.series.palette = palette(family);
    let ink = isoline_ink(family);
    let fields = &mut theme.series.fields;
    fields.colormap = colormap(family);
    fields.nan_color = Color::new(0.0, 0.0, 0.0, 0.0);
    fields.contour_color = Some(alpha(ink, 0.75));
    fields.contour_width = 1.3;
    fields.contour_label_color = ink;
    fields.contour_label_size = 14.0;
    fields.contour_label_padding = 3.0;
    theme
}

/// A family's quiver (cool slate, warm paper): arrows in the family's ink,
/// coloured through the family's mark map when they carry a value, over a
/// muted field.
fn vector_field(family: ThemeFamily) -> Theme {
    let mut theme = family_chrome(family, ChartKind::VectorField);
    theme.chrome.colorbar.title_size = 19.0;
    theme.series.palette = vec![theme.chrome.ink];
    theme.series.fields.colormap = mark_colormap(family);
    theme.series.fields.fill_opacity = Some(0.35);
    let vectors = &mut theme.series.vectors;
    vectors.width_px = 1.8;
    vectors.head_length_px = 7.0;
    vectors.head_width_px = 6.0;
    theme
}

/// A family's 3D charts (gallery, cool slate, warm paper): a matte, finely wireframed
/// surface in the family's mark map, floor contours in the map's colours
/// darkened, lines and arrows in the family's colours, a floor grid.
fn surface_3d(family: ThemeFamily) -> Theme {
    let mut theme = gridded(family_chrome(family, ChartKind::Surface3d));
    theme.chrome.colorbar.title_size = theme.chrome.axis_title_size;
    let ink = theme.chrome.ink;
    theme.series.palette = palette(family);
    theme.series.lines.line_width = 3.0;
    theme.series.lines.marker_size = 10.0;
    let fields = &mut theme.series.fields;
    fields.colormap = mark_colormap(family);
    fields.contour_color = Some(alpha(ink, 0.85));
    fields.contour_width = 2.0;
    fields.contour_shade = Some(0.72);
    let surfaces = &mut theme.series.surfaces;
    surfaces.material = Material::Matte;
    surfaces.wireframe = true;
    surfaces.wireframe_color = alpha(ink, 0.2);
    surfaces.wireframe_width = 0.6;
    let vectors = &mut theme.series.vectors;
    vectors.width_px = 2.2;
    vectors.head_length_px = 9.0;
    vectors.head_width_px = 7.0;
    theme
}

/// A family's mesh field (gallery, cool slate, warm paper): the family's
/// map under isolines in the isoline ink, mesh edges as faint hairlines,
/// no grid.
fn tri_mesh(family: ThemeFamily) -> Theme {
    let mut theme = covered(family_chrome(family, ChartKind::TriMesh));
    theme.chrome.colorbar.title_size = 19.0;
    let ink = isoline_ink(family);
    let edges = match family {
        ThemeFamily::Gallery => GALLERY_INK,
        _ => theme.chrome.ink,
    };
    // The isolines, then the mesh edges drawn over them.
    theme.series.palette = vec![alpha(ink, 0.6), alpha(edges, 0.18)];
    theme.series.lines.line_width = 0.7;
    let fields = &mut theme.series.fields;
    fields.colormap = colormap(family);
    fields.contour_width = 1.2;
    fields.contour_label_color = ink;
    theme
}

/// The night polar plots: mint, lavender, amber and rose lines with a
/// light fill and round markers, flat rose bars, the midnight map under
/// white isolines, spokes and rings on midnight.
fn polar_midnight() -> Theme {
    let mut theme = gridded(family_chrome(ThemeFamily::Midnight, ChartKind::Polar));
    theme.chrome.colorbar.title_size = 19.0;
    theme.series.palette = palette(ThemeFamily::Midnight);
    let lines = &mut theme.series.lines;
    lines.line_width = 2.5;
    lines.fill_opacity = Some(0.2);
    lines.marker_size = 7.0;
    let bars = &mut theme.series.bars;
    bars.corner_radius = 3.0;
    bars.gloss = 0.0;
    bars.texture_strength = 0.0;
    let fields = &mut theme.series.fields;
    fields.colormap = colormap(ThemeFamily::Midnight);
    fields.contour_color = Some(alpha(Color::WHITE, 0.7));
    fields.contour_width = 1.2;
    fields.contour_label_color = Color::WHITE;
    fields.contour_label_size = 13.0;
    fields.contour_label_padding = 3.0;
    theme
}

/// A family's radar (cool slate, midnight, warm paper): outlines in the
/// family's colours with a light fill and round markers, rings in the
/// family's grid and spokes a shade stronger, no axis lines.
fn radar(family: ThemeFamily) -> Theme {
    let mut theme = gridded(family_chrome(family, ChartKind::Radar));
    let chrome = &mut theme.chrome;
    chrome.frame_ink = chrome.grid.major_y.color;
    chrome.grid.major_x.color = match family {
        ThemeFamily::Midnight => rgb(56, 72, 100),
        ThemeFamily::WarmPaper => rgb(220, 208, 190),
        _ => rgb(208, 212, 226),
    };
    theme.series.palette = palette(family);
    let radar = &mut theme.series.radar;
    radar.line_width_px = Some(2.0);
    radar.marker_size_px = Some(7.0);
    radar.fill_opacity = Some(0.25);
    theme
}

/// A family's parallel coordinates (gallery, cool slate, warm paper): thin
/// rows in the family's ink or coloured through its mark map, muted axes
/// with outward ticks and small labels under larger axis names, a slim
/// colourbar.
fn parallel(family: ThemeFamily) -> Theme {
    let mut theme = covered(family_chrome(family, ChartKind::Parallel));
    let ink = theme.chrome.ink;
    let chrome = &mut theme.chrome;
    chrome.axis_preset = AxisPreset::OpenOutward;
    chrome.frame_ink = match family {
        ThemeFamily::Gallery => chrome.frame_ink,
        ThemeFamily::WarmPaper => rgb(160, 146, 132),
        _ => rgb(150, 156, 174),
    };
    chrome.tick_label_size = 16.0;
    chrome.colorbar.thickness = 12.0;
    chrome.colorbar.length = 0.3;
    chrome.colorbar.gap = 16.0;
    chrome.colorbar.title_size = 19.0;
    theme.series.parallel.name_size = 20.0;
    theme.series.palette = vec![ink];
    theme.series.lines.line_width = 1.2;
    theme.series.fields.colormap = mark_colormap(family);
    theme
}

/// A family's ternary chart (gallery, cool slate, midnight): twelve
/// regions in the family's tints outlined in the paper colour, lines
/// through their markers in strong colours, muted axes with outward ticks.
fn ternary(family: ThemeFamily) -> Theme {
    let mut theme = gridded(family_chrome(family, ChartKind::Ternary));
    let paper = theme.chrome.background.unwrap_or(Color::WHITE);
    let ink = theme.chrome.ink;
    let chrome = &mut theme.chrome;
    chrome.axis_preset = AxisPreset::OpenOutward;
    chrome.tick_label_size = 16.0;
    chrome.legend_ink = Some(ink);
    let (frame, label) = match family {
        ThemeFamily::Midnight => (rgb(96, 110, 140), rgb(150, 164, 190)),
        ThemeFamily::CoolSlate => (rgb(150, 156, 174), rgb(96, 102, 128)),
        _ => (chrome.frame_ink, rgb(70, 80, 98)),
    };
    chrome.frame_ink = frame;
    chrome.label_ink = label;
    let (lines, regions) = match family {
        ThemeFamily::Midnight => (
            vec![rgb(240, 246, 255), rgb(242, 186, 78), rgb(65, 227, 194)],
            vec![
                rgb(34, 62, 96),
                rgb(52, 48, 98),
                rgb(28, 80, 84),
                rgb(70, 52, 86),
                rgb(40, 74, 62),
                rgb(84, 66, 48),
                rgb(30, 58, 110),
                rgb(60, 84, 110),
                rgb(46, 96, 90),
                rgb(90, 60, 80),
                rgb(70, 80, 46),
                rgb(56, 62, 120),
            ],
        ),
        ThemeFamily::CoolSlate => (
            vec![rgb(40, 60, 130), rgb(190, 60, 60), rgb(20, 120, 120)],
            vec![
                rgb(190, 214, 236),
                rgb(206, 200, 236),
                rgb(180, 226, 222),
                rgb(214, 226, 244),
                rgb(196, 232, 206),
                rgb(226, 214, 240),
                rgb(170, 206, 230),
                rgb(222, 236, 214),
                rgb(200, 220, 232),
                rgb(236, 226, 244),
                rgb(188, 220, 214),
                rgb(230, 236, 248),
            ],
        ),
        _ => (
            vec![rgb(36, 74, 140), rgb(180, 40, 60), rgb(18, 112, 96)],
            vec![
                rgb(170, 196, 230),
                rgb(236, 180, 170),
                rgb(246, 206, 150),
                rgb(170, 214, 200),
                rgb(200, 186, 226),
                rgb(214, 222, 170),
                rgb(160, 206, 226),
                rgb(240, 196, 214),
                rgb(206, 214, 226),
                rgb(226, 200, 160),
                rgb(186, 226, 190),
                rgb(246, 226, 176),
            ],
        ),
    };
    theme.series.palette = lines;
    theme.series.lines.line_width = 2.2;
    let ternary = &mut theme.series.ternary;
    ternary.regions = regions;
    ternary.region_opacity = 0.75;
    ternary.region_outline = Some((paper, 2.0));
    theme
}
