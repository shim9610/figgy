//! Polar plots: ordinary line and scatter series drawn in (angle, radius)
//! through `Config.polar`. Each series' `x_column` holds the angle in
//! `polar.angle_unit` and its `y_column` the radius; the renderer projects
//! them, and the angle and radius axes keep their ticks, labels and grid as
//! spokes and rings. `set_y_range` sets the radius axis. `set_x_range` only
//! chooses which angle ticks, spokes and labels are drawn: 360° (or 2π) is
//! always one full turn, and `sector` limits the drawn part of the circle.
//! A heatmap draws its cells as annular sectors between its angle and
//! radius boundaries, with its contours and their labels (#208).
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
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(line),
                errors: None,
                stem: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions::default()));
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
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(gallery::scatter(Color::from_rgb8(200, 60, 40), 7.0)),
                line: Some(gallery::line(Color::from_rgb8(200, 60, 40), 1.5)),
                errors: None,
                stem: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions {
                zero_angle_degrees: 90.0,
                clockwise: true,
                interpolation: PolarInterpolation::Arc { pieces: 6 },
                radial_label_angle_degrees: 45.0,
                ..PolarOptions::default()
            }));
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
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(gallery::scatter(Color::from_rgb8(40, 150, 90), 6.0)),
                line: Some(dashed),
                errors: None,
                stem: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions {
                interpolation: PolarInterpolation::Arc { pieces: 16 },
                ..PolarOptions::default()
            }));
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
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(line),
                errors: None,
                stem: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions {
                angle_unit: AngleUnit::Radians,
                sector: Some([-PI / 2.0, PI / 2.0]),
                inner_radius_fraction: 0.2,
                radial_label_angle_degrees: 90.0,
                ..PolarOptions::default()
            }));
            config.bottom_x.major_spacing = PI / 4.0;
            config.bottom_x.label_style.format = LabelFormat::Radians;
            config.left_y.major_spacing = 0.5;
        }
        gallery::save(r, &chart, &series, &out.join("04-sector-radians.png"))?;

        // 5) A heatmap on the polar chart (#208): an antenna's gain in dBi
        //    over azimuth (5° cells) and frequency (rings from 1 to 6 GHz),
        //    the main lobe narrowing as the frequency rises, with labelled
        //    contours along the -3 dB and -10 dB lines.
        let azimuths = 72usize;
        let bands = 25usize;
        // Sample centres every 5°, and the first again one turn on: naming
        // its column twice closes the interpolated seam at 0°.
        let centres: Vec<f64> = (0..=azimuths).map(|i| i as f64 * 5.0 + 2.5).collect();
        let frequencies: Vec<f64> = (0..bands)
            .map(|j| 1.0 + j as f64 * 5.0 / (bands - 1) as f64)
            .collect();
        r.add_column("gain_az", &gallery::column(centres))?;
        r.add_column("gain_ghz", &gallery::column(frequencies.clone()))?;
        let mut ids: Vec<String> = (0..azimuths).map(|i| format!("gain_z{i}")).collect();
        for (i, id) in ids.iter().enumerate() {
            let az = (i as f64 * 5.0 + 2.5).to_radians();
            let gain: Vec<f64> = frequencies
                .iter()
                .map(|f| {
                    // A cosine-power main lobe whose power grows with the
                    // frequency, a back lobe 20 dB down and a floor.
                    let lobe = (0.5 * (1.0 + az.cos())).powf(1.5 * f);
                    let back = 0.01 * (0.5 * (1.0 - az.cos())).powi(4);
                    10.0 * (lobe + back + 1e-3).log10() + 6.0
                })
                .collect();
            r.add_column(id, &gallery::column(gain))?;
        }
        ids.push(ids[0].clone());
        let mut chart = gallery::chart("Gain over azimuth and frequency", "Azimuth", "GHz", size);
        chart.set_x_range(0.0, 360.0);
        chart.set_y_range(1.0, 6.0);
        let series = vec![gallery::series(
            "gain",
            "gain_az",
            "gain_ghz",
            DataRenderType::Field(renderer::data_config::FieldSeries {
                matrix: renderer::data_config::MatrixRef {
                    columns: ids,
                    orientation: renderer::data_config::MatrixOrientation::ColumnsAreX,
                    grid_layout: renderer::data_config::GridLayout::Centers,
                },
                fill: Some(renderer::data_config::FieldFillConfig {
                    mode: renderer::data_config::FillMode::Continuous,
                    shading: renderer::data_config::Shading::Interpolated,
                    opacity: 1.0,
                }),
                contour: Some(renderer::data_config::ContourConfig {
                    levels: vec![-4.0, 3.0],
                    line: gallery::line(Color::from_rgb8(25, 25, 25), 1.2),
                    per_level_color: None,
                    labels: Some(renderer::data_config::ContourLabelConfig {
                        visible: true,
                        font_size: 12.0,
                        color: Color::from_rgb8(25, 25, 25),
                        format: LabelFormat::Decimal,
                        significant_digits: 1,
                        spacing_px: 180.0,
                        anchors: Vec::new(),
                        bg_color: None,
                        bg_padding_px: 2.0,
                    }),
                }),
                surface: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions {
                zero_angle_degrees: 90.0,
                clockwise: true,
                inner_radius_fraction: 0.08,
                radial_label_angle_degrees: 90.0,
                ..PolarOptions::default()
            }));
            config.bottom_x.major_spacing = 30.0;
            config.bottom_x.label_style.format = LabelFormat::Degrees;
            config.left_y.major_spacing = 1.0;
            config.grid.show_major_x = false;
            config.grid.show_major_y = false;
            let mut bar = renderer::default::default_colorbar_options();
            bar.axis.min = -24.0;
            bar.axis.max = 6.0;
            bar.axis.major_spacing = 6.0;
            config.colorbar = Some(bar);
        }
        gallery::save(r, &chart, &series, &out.join("05-gain-heatmap.png"))?;
        Ok(())
    })
}
