//! Continuous point value maps on scatter series: points coloured by a third
//! column through the chart's colourbar, and a bubble chart whose marker area
//! follows a fourth column. The renderer reads the value columns on every
//! draw and derives colour and radius in the shader; nothing per point is
//! stored.
//! cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;

use renderer::{
    Color, DataRenderType,
    colormap::ColorMap,
    config::AxisScale,
    data_config::{PointColorMap, PointSizeMap, SizeScale},
    layout::Side,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = gallery::output_directory("target/value-maps")?;
    gallery::with_renderer(|r| {
        // A spiral of samples: x/y positions, a depth value for the colour and
        // an amplitude for the marker size. All four are ordinary columns.
        let n = 120;
        let t: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        let x: Vec<f64> = t.iter().map(|t| 5.0 + 4.0 * t * (t * 12.0).cos()).collect();
        let y: Vec<f64> = t.iter().map(|t| 5.0 + 4.0 * t * (t * 12.0).sin()).collect();
        let depth: Vec<f64> = t.iter().map(|t| 100.0 * t).collect();
        let amplitude: Vec<f64> = t
            .iter()
            .map(|t| 1.0 + 9.0 * (t * 9.0).sin().powi(2))
            .collect();
        r.add_column("x", &gallery::column(x))?;
        r.add_column("y", &gallery::column(y))?;
        r.add_column("depth", &gallery::column(depth))?;
        r.add_column("amplitude", &gallery::column(amplitude))?;

        // 1) Colour-mapped scatter. The colourbar owns the range and the ramp,
        //    exactly as it does for a heatmap; the series only names the column.
        let mut colored = gallery::scatter(Color::BLACK, 6.0);
        colored.point_color_map = Some(PointColorMap {
            column: "depth".into(),
        });
        let colored = gallery::series(
            "depth",
            "x",
            "y",
            DataRenderType::Scatter { scatter: colored },
        );
        let mut chart = gallery::chart("Colour by depth", "x (m)", "y (m)", (960, 600));
        chart.set_x_range(0.0, 10.0);
        chart.set_y_range(0.0, 10.0);
        let mut bar = renderer::default::default_colorbar_options();
        bar.side = Side::Right;
        bar.colormap = ColorMap::Viridis;
        bar.axis.scale = AxisScale::Linear;
        bar.axis.min = 0.0;
        bar.axis.max = 100.0;
        bar.axis.major_spacing = 20.0;
        chart.config_mut().colorbar = Some(bar);
        gallery::save(r, &chart, &[colored], &out.join("color_map.png"))?;

        // 2) Bubble chart: marker area follows the amplitude column over an
        //    explicit domain. `fit_domain` fills the domain from statistics the
        //    host already has; the renderer never infers it from the column.
        let mut bubbles = gallery::scatter(Color::from_rgba(0.13, 0.41, 0.78, 0.55), 6.0);
        let mut size_map = PointSizeMap {
            column: "amplitude".into(),
            domain_min: 0.0,
            domain_max: 1.0,
            min_px: 2.0,
            max_px: 18.0,
            scale: SizeScale::Area,
        };
        size_map.fit_domain(1.0, 10.0);
        bubbles.point_size_map = Some(size_map);
        let bubbles = gallery::series(
            "amplitude",
            "x",
            "y",
            DataRenderType::Scatter { scatter: bubbles },
        );
        let mut chart = gallery::chart("Bubble area by amplitude", "x (m)", "y (m)", (960, 600));
        chart.set_x_range(0.0, 10.0);
        chart.set_y_range(0.0, 10.0);
        gallery::save(r, &chart, &[bubbles], &out.join("size_map.png"))?;
        Ok(())
    })
}
