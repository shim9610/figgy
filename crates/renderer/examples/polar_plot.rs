//! Polar plots: ordinary line and scatter series drawn in (angle, radius)
//! through `Config.polar`. The columns stay the host's (θ, r); the renderer
//! projects them, and the angle and radius axes keep their ticks, labels and
//! grid as rings and spokes.
//! cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;

use renderer::{
    Color, DataRenderType,
    data_config::LineFill,
    format::LabelFormat,
    line::LineStylePreset,
    polar::{AngleUnit, PolarInterpolation, PolarOptions},
};
use std::f64::consts::PI;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = gallery::output_directory("target/polar-plot")?;
    gallery::with_renderer(|r| {
        let size = (960, 640);
        let degrees: Vec<f64> = (0..=360).map(f64::from).collect();
        r.add_column("deg", &gallery::column(degrees.clone()))?;

        // 1) An antenna pattern: a line filled to the centre, degrees around
        //    the ring with a degree sign, rings every 0.25.
        let pattern: Vec<f64> = degrees
            .iter()
            .map(|d| {
                let t = d.to_radians();
                (0.15 + 0.85 * (2.0 * t).cos().abs()) * (0.6 + 0.4 * t.cos().max(0.0))
            })
            .collect();
        r.add_column("pattern", &gallery::column(pattern))?;
        let mut chart = gallery::chart("Antenna pattern", "Azimuth", "Gain", size);
        chart.set_x_range(0.0, 360.0);
        chart.set_y_range(0.0, 1.0);
        let mut line = gallery::line(Color::from_rgb8(30, 110, 210), 2.5);
        line.fill = Some(LineFill::ToBaseline(0.0));
        line.fill_color = Some(Color::new(0.12, 0.43, 0.82, 0.25));
        let series = vec![gallery::series(
            "gain",
            "deg",
            "pattern",
            DataRenderType::Line { line },
        )];
        {
            let config = chart.config_mut();
            config.polar = Some(PolarOptions::default());
            config.bottom_x.major_spacing = 30.0;
            config.bottom_x.label_style.format = LabelFormat::Degrees;
            config.left_y.major_spacing = 0.25;
        }
        gallery::save(r, &chart, &series, &out.join("01-antenna-pattern.png"))?;

        // 2) A compass rose: clockwise from the top, markers on a three-leaf
        //    rose with arc segments between the samples.
        let samples: Vec<f64> = (0..72).map(|i| f64::from(i) * 5.0).collect();
        let rose: Vec<f64> = samples
            .iter()
            .map(|d| 2.0 + (3.0 * d.to_radians()).cos())
            .collect();
        r.add_column("rose_deg", &gallery::column(samples))?;
        r.add_column("rose", &gallery::column(rose))?;
        let mut chart = gallery::chart("Compass rose", "Bearing", "Strength", size);
        chart.set_x_range(0.0, 360.0);
        chart.set_y_range(0.0, 3.0);
        let series = vec![gallery::series(
            "rose",
            "rose_deg",
            "rose",
            DataRenderType::ScatterLine {
                scatter: gallery::scatter(Color::from_rgb8(200, 60, 40), 7.0),
                line: gallery::line(Color::from_rgb8(200, 60, 40), 1.5),
            },
        )];
        {
            let config = chart.config_mut();
            config.polar = Some(PolarOptions {
                zero_angle_degrees: 90.0,
                clockwise: true,
                interpolation: PolarInterpolation::Arc { pieces: 6 },
                radial_label_angle_degrees: 45.0,
                ..PolarOptions::default()
            });
            config.bottom_x.major_spacing = 45.0;
            config.bottom_x.label_style.format = LabelFormat::Degrees;
            config.left_y.major_spacing = 1.0;
        }
        gallery::save(r, &chart, &series, &out.join("02-compass-rose.png"))?;

        // 3) A dashed spiral from 24 sparse samples over two turns: the arc
        //    interpolation bends every segment along the data, and the dash
        //    pattern walks the bent path.
        let turns: Vec<f64> = (0..24).map(|i| f64::from(i) * 30.0).collect();
        let spiral: Vec<f64> = turns.iter().map(|d| d / 720.0 * 5.0).collect();
        r.add_column("spiral_deg", &gallery::column(turns))?;
        r.add_column("spiral", &gallery::column(spiral))?;
        let mut chart = gallery::chart("Spiral, arc interpolation", "Angle", "Radius", size);
        chart.set_x_range(0.0, 360.0);
        chart.set_y_range(0.0, 5.0);
        let mut dashed = gallery::line(Color::from_rgb8(40, 150, 90), 3.0);
        dashed.line_style = LineStylePreset::Dash;
        let series = vec![gallery::series(
            "spiral",
            "spiral_deg",
            "spiral",
            DataRenderType::ScatterLine {
                scatter: gallery::scatter(Color::from_rgb8(40, 150, 90), 6.0),
                line: dashed,
            },
        )];
        {
            let config = chart.config_mut();
            config.polar = Some(PolarOptions {
                interpolation: PolarInterpolation::Arc { pieces: 16 },
                ..PolarOptions::default()
            });
            config.bottom_x.major_spacing = 45.0;
            config.bottom_x.label_style.format = LabelFormat::Degrees;
            config.left_y.major_spacing = 1.0;
        }
        gallery::save(r, &chart, &series, &out.join("03-spiral-arcs.png"))?;

        // 4) A half-circle sector in radians with a hole in the middle: a
        //    phase response over -π/2..π/2, labelled in multiples of π.
        let phase: Vec<f64> = (0..=60)
            .map(|i| -PI / 2.0 + PI * f64::from(i) / 60.0)
            .collect();
        let response: Vec<f64> = phase
            .iter()
            .map(|p| 1.0 + 0.8 * (2.0 * p).cos().powi(2) + 0.2 * (6.0 * p).sin())
            .collect();
        r.add_column("phase", &gallery::column(phase))?;
        r.add_column("response", &gallery::column(response))?;
        let mut chart = gallery::chart("Sector in radians", "Phase", "Response", size);
        chart.set_x_range(-PI / 2.0, PI / 2.0);
        chart.set_y_range(0.0, 2.0);
        let mut line = gallery::line(Color::from_rgb8(150, 60, 170), 3.0);
        line.fill = Some(LineFill::ToBaseline(0.0));
        line.fill_color = Some(Color::new(0.59, 0.24, 0.67, 0.2));
        let series = vec![gallery::series(
            "response",
            "phase",
            "response",
            DataRenderType::Line { line },
        )];
        {
            let config = chart.config_mut();
            config.polar = Some(PolarOptions {
                angle_unit: AngleUnit::Radians,
                sector: Some([-PI / 2.0, PI / 2.0]),
                inner_radius_fraction: 0.2,
                radial_label_angle_degrees: 90.0,
                ..PolarOptions::default()
            });
            config.bottom_x.major_spacing = PI / 4.0;
            config.bottom_x.label_style.format = LabelFormat::Radians;
            config.left_y.major_spacing = 0.5;
        }
        gallery::save(r, &chart, &series, &out.join("04-sector-radians.png"))?;
        Ok(())
    })
}
