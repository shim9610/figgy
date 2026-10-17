//! Area fills and step lines on Cartesian line series: a filled area under a
//! line, a confidence band between two lines, and the three step modes.
//! cargo run --locked -p figgy-renderer --example area_step -- target/area-step
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;

use renderer::{
    Color, DataRenderType, SeriesConfig,
    data_config::{DataLineStyleConfig, LineFill, LineInterpolation},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = gallery::output_directory("target/area-step")?;
    gallery::with_renderer(|r| {
        // Area under a line, and a band between two lines sharing one x column.
        let x: Vec<f64> = (0..=60).map(|i| i as f64 / 6.0).collect();
        let mean: Vec<f64> = x.iter().map(|x| 2.0 + (x * 0.8).sin()).collect();
        let upper: Vec<f64> = mean
            .iter()
            .zip(&x)
            .map(|(m, x)| m + 0.25 + 0.05 * x)
            .collect();
        let lower: Vec<f64> = mean
            .iter()
            .zip(&x)
            .map(|(m, x)| m - 0.25 - 0.05 * x)
            .collect();
        r.add_column("x", &gallery::column(x))?;
        r.add_column("mean", &gallery::column(mean))?;
        r.add_column("upper", &gallery::column(upper))?;
        r.add_column("lower", &gallery::column(lower))?;

        let blue = Color::from_rgb8(34, 104, 198);
        let band = SeriesConfig {
            // The band is declared first so its fill sits under every line;
            // the fill loop draws fills before bodies regardless of order.
            label: None,
            ..gallery::series(
                "upper",
                "x",
                "upper",
                DataRenderType::Line {
                    line: DataLineStyleConfig {
                        line_color: Color::from_rgba(0.13, 0.41, 0.78, 0.6),
                        line_width: 1.0,
                        fill: Some(LineFill::ToSeries("lower".into())),
                        fill_color: Some(Color::from_rgba(0.13, 0.41, 0.78, 0.18)),
                        ..DataLineStyleConfig::default()
                    },
                },
            )
        };
        let lower_edge = gallery::series(
            "lower",
            "x",
            "lower",
            DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_color: Color::from_rgba(0.13, 0.41, 0.78, 0.6),
                    line_width: 1.0,
                    ..DataLineStyleConfig::default()
                },
            },
        );
        let mean_line = gallery::series(
            "mean",
            "x",
            "mean",
            DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_color: blue,
                    line_width: 2.5,
                    // The default fill colour: the line colour at a quarter alpha.
                    fill: Some(LineFill::ToBaseline(0.0)),
                    ..DataLineStyleConfig::default()
                },
            },
        );
        let mut chart =
            gallery::chart("Area and confidence band", "Time (s)", "Signal", (960, 600));
        chart.set_x_range(0.0, 10.0);
        chart.set_y_range(0.0, 4.0);
        gallery::save(
            r,
            &chart,
            &[band, lower_edge, mean_line],
            &out.join("area_band.png"),
        )?;

        // Step modes over the same sparse samples.
        let sx: Vec<f64> = (0..8).map(f64::from).collect();
        let sy = vec![1.0, 2.5, 1.5, 3.0, 2.0, 3.5, 2.5, 3.0];
        r.add_column("sx", &gallery::column(sx))?;
        r.add_column("sy", &gallery::column(sy))?;
        let steps = [
            (
                "pre",
                LineInterpolation::StepPre,
                Color::from_rgb8(198, 68, 34),
            ),
            (
                "post",
                LineInterpolation::StepPost,
                Color::from_rgb8(34, 150, 90),
            ),
            (
                "mid",
                LineInterpolation::StepMid,
                Color::from_rgb8(120, 60, 180),
            ),
        ]
        .map(|(id, interpolation, color)| {
            gallery::series(
                id,
                "sx",
                "sy",
                DataRenderType::Line {
                    line: DataLineStyleConfig {
                        line_color: color,
                        line_width: 2.0,
                        interpolation,
                        // A fill under a step line follows the steps.
                        fill: (id == "post").then_some(LineFill::ToBaseline(0.0)),
                        ..DataLineStyleConfig::default()
                    },
                },
            )
        });
        let mut chart = gallery::chart("Step interpolation", "Sample", "Value", (960, 600));
        chart.set_x_range(-0.5, 7.5);
        chart.set_y_range(0.0, 4.0);
        gallery::save(r, &chart, &steps, &out.join("steps.png"))
    })
}
