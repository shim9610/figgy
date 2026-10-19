//! Vector fields (quiver plots) from host columns. The renderer reads the
//! components on every draw and places each arrow in the shader.
//! - `wind` writes `gallery-wind.png`: the grid form, two matrices of (u, v)
//!   on a coarse grid over a heatmap of the speed on a fine grid.
//! - `currents` writes `gallery-currents.png`: the points form, (u, v)
//!   columns at scattered anchors, coloured by a speed column through the
//!   colourbar, every arrow at most 44 px long (`fit_scale` over the u and v
//!   extents).
//! - `gradient` writes `gradient.png`: the grid form in data units
//!   (`VectorScale::Data`) with arrows drawn from their tails, coloured by a
//!   third matrix through the colourbar (`VectorColorMap::Grid`). The host
//!   computes the gradient and the speed; the renderer draws the supplied
//!   matrices.
//!
//! cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, column, series};
use renderer::{
    Color, ColorMap, Renderer,
    config::AxisScale,
    data_config::{
        DataRenderType, FieldFillConfig, FillMode, GridLayout, MatrixOrientation, MatrixRef,
        Shading, VectorColorMap, VectorComponents, VectorPivot, VectorScale, VectorStyle,
    },
    layout::Side,
};
use std::path::Path;
use styled::{cartesian, colorbar, export, extent, rgb, rgba, save};

// A sea breeze over a bay: wind speed as a smooth field, direction as arrows
// on a coarser grid of the same coordinate space.
fn wind(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(14, 22, 38);
    let ink = rgb(182, 199, 222);
    let mut chart = cartesian(
        "Sea breeze / 15:00 local",
        "East (km)",
        "North (km)",
        bg,
        ink,
        bg,
    );
    chart.set_x_range(0., 12.);
    chart.set_y_range(0., 8.);
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_y = false;
    let field = |x: f64, y: f64| -> (f64, f64) {
        // A vortex off the headland, a second weaker one inland and a
        // steady onshore drift.
        let eddy = |cx: f64, cy: f64, strength: f64, width: f64| {
            let (dx, dy) = (x - cx, y - cy);
            let r2 = dx * dx + dy * dy;
            let gain = strength / (1. + r2 / width);
            (-dy * gain, dx * gain)
        };
        let (u1, v1) = eddy(4., 5.2, 1.4, 2.2);
        let (u2, v2) = eddy(8.6, 2.6, -0.9, 1.6);
        (u1 + u2 + 0.9, v1 + v2 + 0.35)
    };
    let axis = |n: usize, span: f64| -> Vec<f64> {
        (0..n).map(|i| span * i as f64 / (n - 1) as f64).collect()
    };
    let (fx, fy) = (axis(73, 12.), axis(49, 8.));
    r.add_column("wx", &column(fx.clone()))?;
    r.add_column("wy", &column(fy.clone()))?;
    let mut speed_ids = Vec::new();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (i, x) in fx.iter().enumerate() {
        let id = format!("wind-speed-{i}");
        let speed: Vec<f64> = fy
            .iter()
            .map(|y| {
                let (u, v) = field(*x, *y);
                u.hypot(v)
            })
            .collect();
        let (a, b) = extent(&speed);
        lo = lo.min(a);
        hi = hi.max(b);
        r.add_column(id.clone(), &column(speed))?;
        speed_ids.push(id);
    }
    let (ax, ay) = (axis(25, 12.), axis(17, 8.));
    r.add_column("ax", &column(ax.clone()))?;
    r.add_column("ay", &column(ay.clone()))?;
    let (mut u_ids, mut v_ids) = (Vec::new(), Vec::new());
    for (i, x) in ax.iter().enumerate() {
        let u: Vec<f64> = ay.iter().map(|y| field(*x, *y).0).collect();
        let v: Vec<f64> = ay.iter().map(|y| field(*x, *y).1).collect();
        let (uid, vid) = (format!("wind-u-{i}"), format!("wind-v-{i}"));
        r.add_column(uid.clone(), &column(u))?;
        r.add_column(vid.clone(), &column(v))?;
        u_ids.push(uid);
        v_ids.push(vid);
    }
    let grid = |columns: &Vec<String>| MatrixRef {
        columns: columns.clone(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Centers,
    };
    let specs = [
        series(
            "speed",
            "wx",
            "wy",
            DataRenderType::Heatmap {
                matrix: grid(&speed_ids),
                fill: FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: Shading::Interpolated,
                    opacity: 1.,
                },
            },
        ),
        series(
            "wind",
            "ax",
            "ay",
            DataRenderType::Vector {
                components: VectorComponents::Grid {
                    u: grid(&u_ids),
                    v: grid(&v_ids),
                },
                style: VectorStyle {
                    color: rgba(240, 246, 255, 0.9),
                    width_px: 1.8,
                    scale: VectorScale::Pixels { px_per_unit: 16. },
                    pivot: VectorPivot::Middle,
                    head_length_px: 7.,
                    head_width_px: 6.,
                    min_length_px: 3.,
                    color_map: None,
                },
            },
        ),
    ];
    colorbar(
        chart.config_mut(),
        ColorMap::Custom {
            stops: vec![
                bg,
                rgb(25, 62, 96),
                rgb(33, 122, 142),
                rgb(72, 190, 170),
                rgb(214, 240, 196),
            ],
        },
        0.,
        (hi * 2.).round() / 2.,
        0.5,
        ink,
    );
    chart.config_mut().bottom_x.major_spacing = 2.;
    chart.config_mut().left_y.major_spacing = 2.;
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-wind.png"))
}

// Surface currents: arrows at scattered buoys, length and colour by speed.
fn currents(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(9, 30, 40);
    let ink = rgb(170, 205, 205);
    let mut chart = cartesian(
        "Surface current / drifter array",
        "Longitude",
        "Latitude",
        bg,
        ink,
        rgb(19, 47, 58),
    );
    chart.set_x_range(0., 10.);
    chart.set_y_range(0., 6.);
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_x = true;
    let (mut x, mut y, mut u, mut v, mut speed) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (nx, ny) = (26u32, 16u32);
    for j in 0..ny {
        for i in 0..nx {
            // A deterministic jitter keeps the buoys off a strict lattice.
            let h = f64::from(i * 7 + j * 13) * 0.618;
            let jx = (h.sin() * 43758.5453).fract() - 0.5;
            let jy = (h.cos() * 43758.5453).fract() - 0.5;
            let px = 0.3 + 9.4 * f64::from(i) / f64::from(nx - 1) + 0.18 * jx;
            let py = 0.3 + 5.4 * f64::from(j) / f64::from(ny - 1) + 0.18 * jy;
            // A meandering jet with slower return flow on either side.
            let centre = 3. + 1.1 * (px * 0.9).sin();
            let d = (py - centre) / 0.9;
            let jet = (-d * d).exp();
            let cu = 0.25 + 1.3 * jet;
            let cv = 1.0 * (px * 0.9).cos() * jet - 0.15 * d * (1. - jet);
            x.push(px);
            y.push(py);
            u.push(cu);
            v.push(cv);
            speed.push(cu.hypot(cv));
        }
    }
    let (lo, hi) = extent(&speed);
    for (id, data) in [
        ("cx", x),
        ("cy", y),
        ("cu", u.clone()),
        ("cv", v.clone()),
        ("cspeed", speed),
    ] {
        r.add_column(id, &column(data))?;
    }
    let mut style = VectorStyle {
        color: ink,
        width_px: 2.2,
        scale: VectorScale::Pixels { px_per_unit: 1. },
        pivot: VectorPivot::Middle,
        head_length_px: 8.,
        head_width_px: 7.,
        min_length_px: 0.,
        color_map: Some(VectorColorMap::Column("cspeed".into())),
    };
    style.fit_scale(extent(&u), extent(&v), 44.);
    let specs = [series(
        "current",
        "cx",
        "cy",
        DataRenderType::Vector {
            components: VectorComponents::Points {
                u: "cu".into(),
                v: "cv".into(),
            },
            style,
        },
    )];
    colorbar(
        chart.config_mut(),
        ColorMap::Custom {
            stops: vec![
                rgb(36, 92, 120),
                rgb(70, 170, 160),
                rgb(173, 226, 150),
                rgb(250, 230, 120),
            ],
        },
        (lo * 10.).floor() / 10.,
        (hi * 10.).ceil() / 10.,
        0.4,
        ink,
    );
    chart.config_mut().bottom_x.major_spacing = 2.;
    chart.config_mut().left_y.major_spacing = 1.;
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-currents.png"))
}

// The gradient of a potential on a grid. The potential is a matrix on a
// 41 x 41 grid. The quiver samples its own coarser 14 x 14 grid over its own
// coordinate columns: its gradient components are two matrices and the speed
// a third, which colours the arrows through the chart's colourbar.
fn gradient(r: &mut Renderer, out: &Path) -> Result {
    let potential = |x: f64, y: f64| {
        1.1 * (-((x + 1.0).powi(2) / 1.3 + (y - 0.7).powi(2) / 0.8)).exp()
            + 0.85 * (-((x - 1.1).powi(2) / 0.7 + (y + 0.7).powi(2) / 1.8)).exp()
    };
    let axis = |n: usize| -> Vec<f64> {
        (0..n)
            .map(|i| -3.0 + 6.0 * i as f64 / (n - 1) as f64)
            .collect()
    };
    let fine = axis(41);
    r.add_column("gx", &column(fine.clone()))?;
    r.add_column("gy", &column(fine.clone()))?;
    let mut z_ids = Vec::new();
    for (i, x) in fine.iter().enumerate() {
        let id = format!("z-{i}");
        let z: Vec<f64> = fine.iter().map(|y| potential(*x, *y)).collect();
        r.add_column(id.clone(), &column(z))?;
        z_ids.push(id);
    }
    let coarse = axis(14);
    r.add_column("qx", &column(coarse.clone()))?;
    r.add_column("qy", &column(coarse.clone()))?;
    let mut u_ids = Vec::new();
    let mut v_ids = Vec::new();
    let mut speed_ids = Vec::new();
    let (mut speed_min, mut speed_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for (i, x) in coarse.iter().enumerate() {
        let h = 1e-3;
        let gu: Vec<f64> = coarse
            .iter()
            .map(|y| (potential(x + h, *y) - potential(x - h, *y)) / (2.0 * h))
            .collect();
        let gv: Vec<f64> = coarse
            .iter()
            .map(|y| (potential(*x, y + h) - potential(*x, y - h)) / (2.0 * h))
            .collect();
        let speed: Vec<f64> = gu.iter().zip(&gv).map(|(u, v)| u.hypot(*v)).collect();
        for s in &speed {
            speed_min = speed_min.min(*s);
            speed_max = speed_max.max(*s);
        }
        for (prefix, values, ids) in [
            ("gu", gu, &mut u_ids),
            ("gv", gv, &mut v_ids),
            ("speed", speed, &mut speed_ids),
        ] {
            let id = format!("{prefix}-{i}");
            r.add_column(id.clone(), &column(values))?;
            ids.push(id);
        }
    }
    let grid = |columns: &Vec<String>| MatrixRef {
        columns: columns.clone(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Centers,
    };
    let field = series(
        "potential",
        "gx",
        "gy",
        DataRenderType::Heatmap {
            matrix: grid(&z_ids),
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 0.35,
            },
        },
    );
    let arrows = series(
        "gradient",
        "qx",
        "qy",
        DataRenderType::Vector {
            components: VectorComponents::Grid {
                u: grid(&u_ids),
                v: grid(&v_ids),
            },
            style: VectorStyle {
                color: Color::BLACK,
                width_px: 1.6,
                scale: VectorScale::Data { factor: 0.35 },
                pivot: VectorPivot::Tail,
                head_length_px: 6.0,
                head_width_px: 5.0,
                min_length_px: 2.0,
                color_map: Some(VectorColorMap::Grid(grid(&speed_ids))),
            },
        },
    );
    let mut chart = gallery::chart("Gradient of a potential", "x", "y", (960, 640));
    chart.set_x_range(-3.0, 3.0);
    chart.set_y_range(-3.0, 3.0);
    let mut bar = renderer::default::default_colorbar_options();
    bar.side = Side::Right;
    bar.colormap = ColorMap::Viridis;
    bar.axis.scale = AxisScale::Linear;
    bar.axis.min = speed_min;
    bar.axis.max = speed_max;
    bar.axis.major_spacing = (speed_max - speed_min) / 5.0;
    chart.config_mut().colorbar = Some(bar);
    gallery::save(r, &chart, &[field, arrows], &out.join("gradient.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/vector-field")?;
    gallery::with_renderer(|r| {
        wind(r, &out)?;
        currents(r, &out)?;
        gradient(r, &out)
    })
}
