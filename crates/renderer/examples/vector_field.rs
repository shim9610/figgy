//! Vector fields (quiver plots): arrows at scattered anchors from a (u, v)
//! column pair, and arrows on a heatmap's grid from two matrices, coloured
//! by a third. The renderer reads the components on every draw and places
//! each arrow in the shader; nothing per arrow is stored.
//! cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;

use renderer::{
    Color, DataRenderType,
    colormap::ColorMap,
    config::AxisScale,
    data_config::{
        FieldFillConfig, FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
        VectorColorMap, VectorComponents, VectorPivot, VectorScale, VectorStyle,
    },
    layout::Side,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = gallery::output_directory("target/vector-field")?;
    gallery::with_renderer(|r| {
        // 1) Point vectors. A rotating flow sampled at 15 x 10 scattered
        //    anchors: x / y are the anchors, u / v the components. All four
        //    are ordinary columns the host already has.
        let (nx, ny) = (15, 10);
        let mut x = Vec::new();
        let mut y = Vec::new();
        let mut u = Vec::new();
        let mut v = Vec::new();
        for j in 0..ny {
            for i in 0..nx {
                let px = 0.5 + 9.0 * i as f64 / (nx - 1) as f64;
                let py = 0.5 + 5.0 * j as f64 / (ny - 1) as f64;
                x.push(px);
                y.push(py);
                // A vortex around (5, 3) with a mild downstream drift.
                let (dx, dy) = (px - 5.0, py - 3.0);
                let r2 = dx * dx + dy * dy;
                u.push(-dy / (1.0 + 0.3 * r2) + 0.3);
                v.push(dx / (1.0 + 0.3 * r2));
            }
        }
        r.add_column("x", &gallery::column(x))?;
        r.add_column("y", &gallery::column(y))?;
        r.add_column("u", &gallery::column(u.clone()))?;
        r.add_column("v", &gallery::column(v.clone()))?;
        let reach = |values: &[f64]| {
            values
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), value| {
                    (lo.min(*value), hi.max(*value))
                })
        };
        // The longest arrow is 36 px: the scale comes from statistics the
        // host holds, never from the renderer reading the columns.
        let mut style = VectorStyle {
            color: Color::from_rgb8(20, 90, 180),
            width_px: 1.8,
            scale: VectorScale::Pixels { px_per_unit: 1.0 },
            pivot: VectorPivot::Middle,
            head_length_px: 7.0,
            head_width_px: 6.0,
            min_length_px: 0.0,
            color_map: None,
        };
        style.fit_scale(reach(&u), reach(&v), 36.0);
        let flow = gallery::series(
            "flow",
            "x",
            "y",
            DataRenderType::Vector {
                components: VectorComponents::Points {
                    u: "u".into(),
                    v: "v".into(),
                },
                style,
            },
        );
        let mut chart = gallery::chart("Vortex flow", "x (m)", "y (m)", (960, 600));
        chart.set_x_range(0.0, 10.0);
        chart.set_y_range(0.0, 6.0);
        gallery::save(r, &chart, &[flow], &out.join("points.png"))?;

        // 2) Grid vectors over a heatmap. The potential is a matrix on a
        //    41 x 41 grid. The quiver samples its own coarser 14 x 14 grid
        //    over its own coordinate columns: its gradient components are
        //    two matrices and the speed a third, which colours the arrows
        //    through the chart's colourbar.
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
        r.add_column("gx", &gallery::column(fine.clone()))?;
        r.add_column("gy", &gallery::column(fine.clone()))?;
        let mut z_ids = Vec::new();
        for (i, x) in fine.iter().enumerate() {
            let id = format!("z-{i}");
            let z: Vec<f64> = fine.iter().map(|y| potential(*x, *y)).collect();
            r.add_column(id.clone(), &gallery::column(z))?;
            z_ids.push(id);
        }
        let coarse = axis(14);
        r.add_column("qx", &gallery::column(coarse.clone()))?;
        r.add_column("qy", &gallery::column(coarse.clone()))?;
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
                r.add_column(id.clone(), &gallery::column(values))?;
                ids.push(id);
            }
        }
        let grid = |columns: &Vec<String>| MatrixRef {
            columns: columns.clone(),
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Centers,
        };
        let field = gallery::series(
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
        let gradient = gallery::series(
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
        gallery::save(r, &chart, &[field, gradient], &out.join("grid.png"))?;
        Ok(())
    })
}
