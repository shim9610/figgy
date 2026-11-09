//! 3D charts: `Config.view_3d` turns the data area into a box seen through a
//! camera. The host computes every height, path point and vector component;
//! the renderer places them through the camera and shades them on every draw.
//! - `terrain` writes `gallery-surface.png`: a heatmap's grid drawn as a lit,
//!   wireframed surface over its contours on the box floor, with a hiking
//!   trail and its waypoints as series with a height column (`z_column`).
//! - `vortex` writes `gallery-quiver3d.png`: a Burgers vortex sampled on a
//!   lattice, one arrow per sample along (u, v, w), coloured by a speed
//!   column the host computes.
//!
//! cargo run --locked -p figgy-renderer --example surface_3d -- target/surface_3d
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::{
    Chart, Color, ColorMap, LayerRef, Material, Renderer, SeriesConfig,
    data_config::{
        ContourConfig, DataLineStyleConfig, DataRenderType, FieldFillConfig, FieldSeries,
        MatrixRef, ScatterShape, Shading, VectorColorMap, VectorComponents, VectorPivot,
        VectorScale, VectorStyle,
    },
    text::RichText,
    view3d::{SurfaceStyle, SurfaceWireframe, View3d},
};
use std::path::Path;
use styled::{cartesian, colorbar, export, extent, rgb, rgba, save};

/// The box's z axis is `right_y`: labels and a title in the chrome's style.
fn z_axis(chart: &mut Chart, title: &str, spacing: f64) {
    let config = chart.config_mut();
    let mut z = config.left_y.clone();
    z.title_option.text = RichText::plain(
        title,
        z.title_option.text.color,
        z.title_option.text.font_size,
        z.title_option.text.font.clone(),
    );
    z.label_style.label_visible = true;
    z.major_spacing = spacing;
    config.right_y = z;
}

/// Short tick labels on the three box axes and the colourbar: `1`, not `1.00`.
fn short_labels(chart: &mut Chart) {
    let config = chart.config_mut();
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
}

/// Round the fitted z range out to half units so the box ends on a label.
fn nice_z(chart: &mut Chart, spacing: f64) {
    let z = &mut chart.config_mut().right_y;
    z.min = (z.min * 2.).floor() / 2.;
    z.max = (z.max * 2.).ceil() / 2.;
    z.major_spacing = spacing;
}

fn colorbar_title(chart: &mut Chart, title: &str, ink: Color) {
    if let Some(bar) = chart.config_mut().colorbar.as_mut() {
        bar.axis.title_option.visible = true;
        bar.axis.title_option.text = RichText::plain(title, ink, 19., "");
    }
}

/// A point on a cubic Bezier segment.
fn bezier(p: [[f64; 2]; 4], t: f64) -> [f64; 2] {
    let s = 1. - t;
    let w = [s * s * s, 3. * s * s * t, 3. * s * t * t, t * t * t];
    [0, 1].map(|k| (0..4).map(|i| w[i] * p[i][k]).sum())
}

// A mountain massif: two summits joined by a saddle and a basin to the
// south. A trail climbs from the trailhead over the saddle to the higher
// summit; the host lifts it a little above the ground it follows.
fn terrain(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(248, 247, 243);
    let ink = rgb(58, 62, 72);
    let height = |x: f64, y: f64| {
        let peak = |cx: f64, cy: f64, h: f64, w: f64| {
            h * (-((x - cx).powi(2) + (y - cy).powi(2)) / w).exp()
        };
        peak(1.1, 0.6, 2.3, 1.3)
            + peak(-1.2, 1.1, 1.6, 1.0)
            + peak(-0.2, -1.6, -0.45, 1.4)
            + 0.16 * (1.7 * x).sin() * (1.3 * y + 0.4).cos()
            + 1.85
    };
    let axis = |n: usize| -> Vec<f64> {
        (0..n)
            .map(|i| -3. + 6. * i as f64 / (n - 1) as f64)
            .collect()
    };
    let (xs, ys) = (axis(61), axis(61));
    r.add_column("terrain-x", &xs)?;
    r.add_column("terrain-y", &ys)?;
    let mut ids = Vec::new();
    for (i, x) in xs.iter().enumerate() {
        let id = format!("terrain-z-{i}");
        let z: Vec<f64> = ys.iter().map(|y| height(*x, *y)).collect();
        r.add_column(id.clone(), &z)?;
        ids.push(id);
    }
    let trailhead = [-2.5, -2.3];
    let saddle = [-0.1, 0.85];
    let summit = [1.1, 0.6];
    let legs = [
        [trailhead, [-1.9, -0.6], [-0.9, 1.4], saddle],
        [saddle, [0.3, 0.45], [0.6, 0.2], summit],
    ];
    let (mut tx, mut ty, mut tz) = (Vec::new(), Vec::new(), Vec::new());
    for (leg, steps) in legs.iter().zip([110, 50]) {
        for i in 0..steps {
            let [x, y] = bezier(*leg, i as f64 / steps as f64);
            tx.push(x);
            ty.push(y);
            tz.push(height(x, y) + 0.06);
        }
    }
    tx.push(summit[0]);
    ty.push(summit[1]);
    tz.push(height(summit[0], summit[1]) + 0.06);
    r.add_column("hike-x", &tx)?;
    r.add_column("hike-y", &ty)?;
    r.add_column("hike-z", &tz)?;
    let stops = [trailhead, saddle, summit];
    let (sx, sy, sz): (Vec<f64>, Vec<f64>, Vec<f64>) = (
        stops.iter().map(|p| p[0]).collect(),
        stops.iter().map(|p| p[1]).collect(),
        stops.iter().map(|p| height(p[0], p[1]) + 0.08).collect(),
    );
    r.add_column("stop-x", &sx)?;
    r.add_column("stop-y", &sy)?;
    r.add_column("stop-z", &sz)?;

    let ramp = ColorMap::Custom {
        stops: vec![
            rgb(38, 84, 98),
            rgb(66, 140, 112),
            rgb(152, 186, 110),
            rgb(226, 206, 140),
            rgb(176, 124, 86),
            rgb(246, 244, 240),
        ],
    };
    let mut chart = cartesian(
        "Alpine massif / trail over the saddle",
        "East (km)",
        "North (km)",
        bg,
        ink,
        rgb(222, 220, 212),
    );
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_x = true;
    chart.set_x_range(-3., 3.);
    chart.set_y_range(-3., 3.);
    chart.config_mut().bottom_x.major_spacing = 1.;
    chart.config_mut().left_y.major_spacing = 1.;
    z_axis(&mut chart, "Elevation (km)", 1.);
    colorbar(chart.config_mut(), ramp.clone(), 1.5, 4.5, 0.5, ink);
    // The floor contours take the ramp's colour at their level, darkened so
    // the light levels stay visible on the light floor.
    let mut contour = ContourConfig {
        levels: vec![],
        line: DataLineStyleConfig {
            line_color: ink,
            line_width: 2.6,
            ..Default::default()
        },
        per_level_color: None,
        labels: None,
    };
    let bar = chart.config().colorbar.as_ref().unwrap().axis.clone();
    contour
        .set_nice_levels(&bar, 12)
        .map_err(|e| format!("terrain contour levels: {e:?}"))?;
    contour.per_level_color = Some(
        contour
            .levels
            .iter()
            .map(|level| {
                let c = ramp.sample(((level - bar.min) / (bar.max - bar.min)) as f32);
                Color::from_rgba(c.r * 0.72, c.g * 0.72, c.b * 0.72, 1.)
            })
            .collect(),
    );
    chart.config_mut().set_view_3d(Some(View3d {
        azimuth_degrees: -58.,
        elevation_degrees: 26.,
        distance: 2.3,
        ..View3d::default()
    }));
    let trail_red = rgb(206, 44, 58);
    let trail = SeriesConfig::line("trail", "hike-x", "hike-y")
        .color(trail_red)
        .width(3.5)
        .z("hike-z");
    let waypoints = SeriesConfig::scatter("waypoints", "stop-x", "stop-y")
        .color(trail_red)
        .marker(ScatterShape::CircleFilled, 13.)
        .z("stop-z");
    let specs = [
        SeriesConfig::of_kind(
            "terrain",
            "terrain-x",
            "terrain-y",
            DataRenderType::Field(FieldSeries {
                matrix: MatrixRef::columns_are_x(ids),
                fill: Some(FieldFillConfig {
                    shading: Shading::Interpolated,
                    opacity: 0.9,
                    ..Default::default()
                }),
                contour: Some(contour),
                surface: Some(SurfaceStyle {
                    wireframe: Some(SurfaceWireframe {
                        color: rgba(40, 44, 52, 0.2),
                        width: 0.7,
                    }),
                    material: Material::Matte,
                    ..SurfaceStyle::default()
                }),
            }),
        ),
        trail,
        waypoints,
    ];
    chart.auto_fit_view_3d_z(r.pool(), LayerRef::Base, &specs, 0.)?;
    nice_z(&mut chart, 1.);
    // The floor sits at sea level, well below the massif's base.
    chart.config_mut().right_y.min = 0.;
    short_labels(&mut chart);
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-surface.png"))
}

// A Burgers vortex: swirl around the z axis inside a core, inflow towards
// the axis and an updraft that grows with height. The host computes the
// components and the speed the arrows are coloured by.
fn vortex(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(14, 20, 34);
    let ink = rgb(186, 198, 220);
    let (circulation, core, strain) = (24., 1.2, 0.16);
    let flow = |x: f64, y: f64, z: f64| -> [f64; 3] {
        let r2 = (x * x + y * y).max(1e-9);
        let swirl = circulation / (std::f64::consts::TAU * r2) * (1. - (-r2 / (core * core)).exp());
        [
            -strain * x - swirl * y,
            -strain * y + swirl * x,
            2. * strain * z,
        ]
    };
    let lattice = |n: usize, lo: f64, hi: f64| -> Vec<f64> {
        (0..n)
            .map(|i| lo + (hi - lo) * i as f64 / (n - 1) as f64)
            .collect()
    };
    let spacing = 0.75;
    let (plane, levels) = (lattice(9, -3., 3.), lattice(3, 0.5, 3.5));
    let (mut x, mut y, mut z) = (Vec::new(), Vec::new(), Vec::new());
    let (mut u, mut v, mut w, mut speed) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for pz in &levels {
        for py in &plane {
            for px in &plane {
                let [cu, cv, cw] = flow(*px, *py, *pz);
                x.push(*px);
                y.push(*py);
                z.push(*pz);
                u.push(cu);
                v.push(cv);
                w.push(cw);
                speed.push((cu * cu + cv * cv + cw * cw).sqrt());
            }
        }
    }
    let (ub, vb, wb, (lo, hi)) = (extent(&u), extent(&v), extent(&w), extent(&speed));
    for (id, data) in [
        ("vortex-x", x),
        ("vortex-y", y),
        ("vortex-z", z),
        ("vortex-u", u),
        ("vortex-v", v),
        ("vortex-w", w),
        ("vortex-speed", speed),
    ] {
        r.add_column(id, &data)?;
    }
    let mut style = VectorStyle {
        color: ink,
        width_px: 2.4,
        scale: VectorScale::Data { factor: 1. },
        pivot: VectorPivot::Middle,
        head_length_px: 10.,
        head_width_px: 8.,
        min_length_px: 2.,
        color_map: Some(VectorColorMap::Column("vortex-speed".into())),
    };
    // The longest arrow spans 0.9 of the lattice spacing.
    style.fit_scale_3d(ub, vb, wb, 0.9 * spacing);
    let components = VectorComponents::Points {
        u: "vortex-u".into(),
        v: "vortex-v".into(),
        w: Some("vortex-w".into()),
    };
    let arrows = SeriesConfig::of_kind(
        "vortex",
        "vortex-x",
        "vortex-y",
        DataRenderType::Vector { components, style },
    )
    .z("vortex-z");
    let specs = [arrows];
    let mut chart = cartesian(
        "Burgers vortex / swirl, inflow and updraft",
        "x (km)",
        "y (km)",
        bg,
        ink,
        rgb(40, 52, 74),
    );
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_x = true;
    chart.set_x_range(-3.5, 3.5);
    chart.set_y_range(-3.5, 3.5);
    chart.config_mut().bottom_x.major_spacing = 2.;
    chart.config_mut().left_y.major_spacing = 2.;
    z_axis(&mut chart, "z (km)", 1.);
    colorbar(
        chart.config_mut(),
        ColorMap::Custom {
            stops: vec![
                rgb(52, 84, 160),
                rgb(64, 160, 190),
                rgb(132, 216, 160),
                rgb(244, 228, 120),
                rgb(246, 132, 84),
            ],
        },
        (lo * 2.).floor() / 2.,
        (hi * 2.).ceil() / 2.,
        0.5,
        ink,
    );
    colorbar_title(&mut chart, "Speed (m/s)", ink);
    chart.config_mut().set_view_3d(Some(View3d {
        azimuth_degrees: -50.,
        elevation_degrees: 30.,
        distance: 2.3,
        box_aspect: Some([1., 1., 1.05]),
        ..View3d::default()
    }));
    chart.auto_fit_view_3d_z(r.pool(), LayerRef::Base, &specs, 0.)?;
    nice_z(&mut chart, 1.);
    short_labels(&mut chart);
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-quiver3d.png"))
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    terrain(r, out)?;
    vortex(r, out)
}

fn main() -> Result {
    let out = gallery::output_directory("target/surface_3d")?;
    gallery::with_renderer(|r| render(r, &out))
}
