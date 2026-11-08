//! Triangle meshes (`DataRenderType::TriMesh`): the host supplies the
//! vertices, its own triangulation as an index column
//! (`Renderer::upsert_triangles`) and a value per vertex; the renderer
//! interpolates the values across each triangle, draws their level set as
//! contour lines and, on a 3D chart, places the vertices at their heights.
//! - `plate` writes `gallery-trimesh.png`: the von Mises stress around a hole
//!   in a plate under tension (Kirsch's solution) on a graded mesh with a
//!   hole, filled and contoured flat, the mesh edges drawn over it.
//! - `drum` writes `gallery-trisurf.png`: a vibration mode of a circular
//!   drum on a disc mesh, drawn as a lit, wireframed surface in a 3D box with
//!   its contours on the floor.
//!
//! cargo run --locked -p figgy-renderer --example trimesh -- target/trimesh
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, column, line, series};
use renderer::{
    Chart, Color, ColorMap, LayerRef, Material, Renderer, SeriesConfig, TriangleSource,
    data::Column,
    data_config::{ContourConfig, DataRenderType, FieldFillConfig, FillMode, Shading, XySeries},
    text::RichText,
    tri_mesh::{MeshValueLocation, MeshValues, TriMeshSeries},
    view3d::{SurfaceStyle, SurfaceWireframe, View3d},
};
use std::f64::consts::TAU;
use std::path::Path;
use styled::{cartesian, colorbar, export, rgb, rgba, save};

/// A mesh series over the columns `{id}-x`, `{id}-y`, `{id}-value` and the
/// triangle column `{id}-tri`, filled and contoured.
fn mesh_series(id: &str, contour: ContourConfig, surface: Option<SurfaceStyle>) -> SeriesConfig {
    series(
        id,
        &format!("{id}-x"),
        &format!("{id}-y"),
        DataRenderType::TriMesh(TriMeshSeries {
            triangles: format!("{id}-tri").into(),
            values: MeshValues {
                column: format!("{id}-value").into(),
                at: MeshValueLocation::Vertices,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.,
            }),
            contour: Some(contour),
            surface,
        }),
    )
}

/// Contour lines at about `count` round levels of the chart's colourbar.
fn contours(chart: &Chart, color: Color, width: f32, count: usize) -> Result<ContourConfig> {
    let mut contour = ContourConfig {
        levels: vec![],
        line: line(color, width),
        per_level_color: None,
        labels: None,
    };
    contour
        .set_nice_levels(&chart.config().colorbar.as_ref().unwrap().axis, count)
        .map_err(|e| format!("contour levels: {e:?}"))?;
    Ok(contour)
}

fn colorbar_title(chart: &mut Chart, title: &str, ink: Color) {
    if let Some(bar) = chart.config_mut().colorbar.as_mut() {
        bar.axis.title_option.visible = true;
        bar.axis.title_option.text = RichText::plain(title, ink, 19., "");
        // Clear of the tick labels.
        bar.axis.title_option.offset_y = -11.;
        bar.axis.label_style.significant_digits = 1;
    }
}

// A plate under uniaxial tension S along x with a circular hole of radius a.
// The host meshes the plate between the hole and its rectangular edge: spokes
// from the hole to evenly spaced edge points (the corners among them), rings
// graded so the cells are small at the hole. Kirsch's solution gives the
// stresses at every node; the value is the von Mises stress over S.
fn plate(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(16, 18, 30);
    let ink = rgb(192, 198, 220);
    let (half_w, half_h, hole) = (4., 2.5, 1.);
    let perimeter = 4. * (half_w + half_h);
    let (spokes, rings) = (104usize, 18usize);
    let edge = |s: f64| -> [f64; 2] {
        // Counter-clockwise from (half_w, 0).
        let s = s.rem_euclid(perimeter);
        let legs = [half_h, 2. * half_w, 2. * half_h, 2. * half_w, half_h];
        let mut rest = s;
        for (i, leg) in legs.iter().enumerate() {
            if rest <= *leg {
                return match i {
                    0 => [half_w, rest],
                    1 => [half_w - rest, half_h],
                    2 => [-half_w, half_h - rest],
                    3 => [-half_w + rest, -half_h],
                    _ => [half_w, -half_h + rest],
                };
            }
            rest -= leg;
        }
        [half_w, 0.]
    };
    let (mut xs, mut ys, mut vm) = (Vec::new(), Vec::new(), Vec::new());
    for k in 0..rings {
        let t = k as f64 / (rings - 1) as f64;
        let grade = t.powf(1.8);
        for j in 0..spokes {
            let [ex, ey] = edge(perimeter * j as f64 / spokes as f64);
            let theta = ey.atan2(ex);
            let (hx, hy) = (hole * theta.cos(), hole * theta.sin());
            let (x, y) = (hx + (ex - hx) * grade, hy + (ey - hy) * grade);
            let (radius, angle) = (x.hypot(y), y.atan2(x));
            let (q2, q4) = ((hole / radius).powi(2), (hole / radius).powi(4));
            let (c2, s2) = ((2. * angle).cos(), (2. * angle).sin());
            let srr = 0.5 * (1. - q2) + 0.5 * (1. - 4. * q2 + 3. * q4) * c2;
            let stt = 0.5 * (1. + q2) - 0.5 * (1. + 3. * q4) * c2;
            let srt = -0.5 * (1. + 2. * q2 - 3. * q4) * s2;
            xs.push(x);
            ys.push(y);
            vm.push((srr * srr - srr * stt + stt * stt + 3. * srt * srt).sqrt());
        }
    }
    // Two triangles per cell between neighbouring rings and spokes.
    let node = |k: usize, j: usize| (k * spokes + j % spokes) as u32;
    let mut tris = Vec::new();
    for k in 0..rings - 1 {
        for j in 0..spokes {
            let (a, b, c, d) = (
                node(k, j),
                node(k, j + 1),
                node(k + 1, j),
                node(k + 1, j + 1),
            );
            tris.extend([a, b, d, a, d, c]);
        }
    }
    // The mesh edges as one polyline column pair, NaN between the pieces:
    // every ring, every spoke and every diagonal chain.
    let (mut ex, mut ey) = (Vec::new(), Vec::new());
    let mut piece = |nodes: &mut dyn Iterator<Item = u32>| {
        for n in nodes {
            ex.push(xs[n as usize]);
            ey.push(ys[n as usize]);
        }
        ex.push(f64::NAN);
        ey.push(f64::NAN);
    };
    for k in 0..rings {
        piece(&mut (0..=spokes).map(|j| node(k, j)));
    }
    for j in 0..spokes {
        piece(&mut (0..rings).map(|k| node(k, j)));
        piece(&mut (0..rings).map(|k| node(k, j + k)));
    }
    for (id, data) in [
        ("plate-x", xs.clone()),
        ("plate-y", ys.clone()),
        ("plate-value", vm),
        ("plate-edge-x", ex),
        ("plate-edge-y", ey),
    ] {
        r.add_column(id, &column(data))?;
    }
    r.upsert_triangles(
        "plate-tri",
        &TriangleSource {
            indices: &Column { data: tris },
        },
    )?;

    let mut chart = cartesian(
        "Plate with a hole / von Mises stress under tension",
        "x / a",
        "y / a",
        bg,
        ink,
        bg,
    );
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_y = false;
    chart.set_x_range(-half_w, half_w);
    chart.set_y_range(-half_h, half_h);
    chart.config_mut().bottom_x.major_spacing = 1.;
    chart.config_mut().left_y.major_spacing = 1.;
    let config = chart.config_mut();
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.label_style.significant_digits = 1;
    }
    colorbar(
        chart.config_mut(),
        ColorMap::Custom {
            stops: vec![
                rgb(20, 24, 66),
                rgb(52, 60, 150),
                rgb(40, 140, 170),
                rgb(120, 200, 140),
                rgb(246, 220, 100),
                rgb(244, 120, 60),
                rgb(200, 40, 50),
            ],
        },
        0.,
        3.,
        0.5,
        ink,
    );
    colorbar_title(&mut chart, "von Mises stress / S", ink);
    let contour = contours(&chart, rgba(255, 255, 255, 0.55), 1.2, 12)?;
    let mut edges = series(
        "mesh",
        "plate-edge-x",
        "plate-edge-y",
        DataRenderType::Xy(XySeries {
            scatter: None,
            line: Some(line(rgba(255, 255, 255, 0.16), 0.7)),
            errors: None,
            stem: None,
        }),
    );
    edges.label = None;
    let specs = [mesh_series("plate", contour, None), edges];
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-trimesh.png"))
}

/// The Bessel function of the first kind J_n(x), by its power series.
fn bessel_j(n: i32, x: f64) -> f64 {
    let half = x / 2.;
    let mut term = half.powi(n) / (1..=n).map(f64::from).product::<f64>();
    let mut sum = term;
    for m in 1..60 {
        term *= -half * half / (f64::from(m) * f64::from(m + n));
        sum += term;
    }
    sum
}

// A circular drum: the host meshes the unit disc in rings of 6k nodes,
// stitching neighbouring rings by angle, and evaluates the (2, 2) mode
// J_2(j_22 r) cos 2θ at every node, normalized to a peak of 1.
fn drum(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(248, 247, 244);
    let ink = rgb(56, 60, 74);
    let rings = 26usize;
    let j22 = 8.417_244_140_399_865;
    let ring_start = |k: usize| if k == 0 { 0 } else { 1 + 3 * k * (k - 1) };
    let ring_len = |k: usize| if k == 0 { 1 } else { 6 * k };
    let (mut xs, mut ys, mut zs) = (Vec::new(), Vec::new(), Vec::new());
    for k in 0..=rings {
        let radius = k as f64 / rings as f64;
        for j in 0..ring_len(k) {
            let theta = TAU * j as f64 / ring_len(k) as f64;
            xs.push(radius * theta.cos());
            ys.push(radius * theta.sin());
            zs.push(bessel_j(2, j22 * radius) * (2. * theta).cos());
        }
    }
    let peak = zs.iter().fold(0f64, |m, z| m.max(z.abs()));
    for z in &mut zs {
        *z /= peak;
    }
    // Stitch ring k - 1 to ring k: advance along whichever ring's next node
    // comes first by angle.
    let mut tris: Vec<u32> = Vec::new();
    for k in 1..=rings {
        let (inner, outer) = (ring_len(k - 1), ring_len(k));
        let at = |ring: usize, i: usize, len: usize| (ring_start(ring) + i % len) as u32;
        let (mut i, mut j) = (0, 0);
        while i < inner || j < outer {
            let next_inner = (i + 1) as f64 / inner as f64;
            let next_outer = (j + 1) as f64 / outer as f64;
            if inner > 1 && i < inner && (j == outer || next_inner <= next_outer) {
                tris.extend([
                    at(k - 1, i, inner),
                    at(k, j, outer),
                    at(k - 1, i + 1, inner),
                ]);
                i += 1;
            } else {
                tris.extend([at(k - 1, i, inner), at(k, j, outer), at(k, j + 1, outer)]);
                j += 1;
                if inner == 1 && j == outer {
                    i = inner;
                }
            }
        }
    }
    for (id, data) in [("drum-x", xs), ("drum-y", ys), ("drum-value", zs)] {
        r.add_column(id, &column(data))?;
    }
    r.upsert_triangles(
        "drum-tri",
        &TriangleSource {
            indices: &Column { data: tris },
        },
    )?;

    let mut chart = cartesian(
        "Circular drum / vibration mode (2, 2)",
        "x / R",
        "y / R",
        bg,
        ink,
        rgb(222, 220, 214),
    );
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_x = true;
    chart.set_x_range(-1., 1.);
    chart.set_y_range(-1., 1.);
    chart.config_mut().bottom_x.major_spacing = 0.5;
    chart.config_mut().left_y.major_spacing = 0.5;
    let mut z = chart.config().left_y.clone();
    z.title_option.text = RichText::plain(
        "Displacement",
        z.title_option.text.color,
        z.title_option.text.font_size,
        z.title_option.text.font.clone(),
    );
    z.label_style.label_visible = true;
    chart.config_mut().right_y = z;
    let ramp = ColorMap::Custom {
        stops: vec![
            rgb(36, 72, 150),
            rgb(96, 150, 206),
            rgb(236, 232, 222),
            rgb(226, 132, 92),
            rgb(170, 40, 50),
        ],
    };
    colorbar(chart.config_mut(), ramp.clone(), -1., 1., 0.5, ink);
    let mut contour = contours(&chart, rgba(56, 60, 74, 0.85), 2.2, 10)?;
    // The zero level runs along the nodal lines and the rim, where the
    // displacement is zero to rounding; leave it out.
    contour.levels.retain(|level| level.abs() > 1e-9);
    // Each floor contour takes the ramp's colour at its level, darkened.
    contour.per_level_color = Some(
        contour
            .levels
            .iter()
            .map(|level| {
                let c = ramp.sample(((level + 1.) / 2.) as f32);
                Color::from_rgba(c.r * 0.7, c.g * 0.7, c.b * 0.7, 1.)
            })
            .collect(),
    );
    chart.config_mut().set_view_3d(Some(View3d {
        azimuth_degrees: -55.,
        elevation_degrees: 32.,
        distance: 2.4,
        box_aspect: Some([1., 1., 0.6]),
        ..View3d::default()
    }));
    let mut drum = mesh_series(
        "drum",
        contour,
        Some(SurfaceStyle {
            wireframe: Some(SurfaceWireframe {
                color: rgba(30, 34, 44, 0.25),
                width: 0.6,
            }),
            material: Material::Matte,
            ..SurfaceStyle::default()
        }),
    );
    drum.z_column = Some("drum-value".into());
    let specs = [drum];
    chart.auto_fit_view_3d_z(r.pool(), LayerRef::Base, &specs, 0.)?;
    let config = chart.config_mut();
    // The floor sits below the drum so its contours show beside the surface.
    (config.right_y.min, config.right_y.major_spacing) = (-2., 1.);
    for axis in [
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.label_style.significant_digits = 1;
    }
    if let Some(bar) = config.colorbar.as_mut() {
        bar.axis.label_style.significant_digits = 1;
    }
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-trisurf.png"))
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    plate(r, out)?;
    drum(r, out)
}

fn main() -> Result {
    let out = gallery::output_directory("target/trimesh")?;
    gallery::with_renderer(|r| render(r, &out))
}
