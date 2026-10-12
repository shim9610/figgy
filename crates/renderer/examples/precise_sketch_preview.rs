//! Matching Precise and Sketch growth-curve previews for the README style row.
//! cargo run --locked -p figgy-renderer --example precise_sketch_preview -- target/precise-sketch
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::{
    Chart, Color,
    config::{DrawStyle, LegendCorner, LegendEntryKind, SketchOptions},
    data_config::DataRenderType,
    default,
    layout::{ChartArea, Rect},
};

fn main() -> Result {
    let out = output_directory("target/precise-sketch")?;
    with_renderer(|r| {
        let times: Vec<f64> = vec![0., 5., 10., 15., 20., 30., 45., 60., 90., 120.];
        r.add_column("time", &column(times.clone()))?;
        for (id, ceiling, tau) in [("control", 0.87, 40.), ("treated", 1.04, 19.)] {
            r.add_column(
                id,
                &column(
                    times
                        .iter()
                        .map(|t| 0.06 + (ceiling - 0.06) * (1. - (-t / tau).exp()))
                        .collect(),
                ),
            )?;
        }
        for (name, style, colors) in [
            (
                "precise",
                DrawStyle::Precise,
                [Color::BLACK, Color::from_rgb8(230, 45, 50)],
            ),
            (
                "sketch",
                DrawStyle::Sketch(SketchOptions {
                    amplitude_px: 2.2,
                    wavelength_px: 55.,
                    seed: 1,
                }),
                [
                    Color::from_rgb8(218, 157, 200),
                    Color::from_rgb8(147, 193, 207),
                ],
            ),
        ] {
            let mut cfg = default::default_config();
            cfg.chart_area = ChartArea(Rect {
                x: 0,
                y: 0,
                width: 1280,
                height: 1024,
            });
            cfg.chart_title.top_margin = 60.;
            cfg.left_y.out_margin = 200.;
            cfg.bottom_x.out_margin = 145.;
            cfg.top_x.out_margin = 32.;
            cfg.right_y.out_margin = 64.;
            cfg.draw_style = style;
            for a in [
                &mut cfg.bottom_x,
                &mut cfg.top_x,
                &mut cfg.left_y,
                &mut cfg.right_y,
            ] {
                a.label_style.font_size = 25.;
                a.title_option.text.font_size = 32.;
            }
            cfg.grid.show_minor_x = false;
            cfg.grid.show_minor_y = false;
            cfg.legend.corner = LegendCorner::TopLeft;
            cfg.legend.content.font_size = 23.;
            cfg.legend.padding = 9.;
            let mut chart = Chart::new(cfg)
                .with_x_title("Time [min]")
                .with_y_title("Response mean");
            chart.set_x_range(-6., 126.);
            chart.set_y_range(-0.05, 1.08);
            let mut specs = Vec::new();
            for ((id, label), color) in [("control", "Control mean"), ("treated", "Treated mean")]
                .into_iter()
                .zip(colors)
            {
                specs.push(series(
                    id,
                    "time",
                    id,
                    DataRenderType::ScatterLine {
                        line: line(color, 2.5),
                        scatter: scatter(color, 8.),
                    },
                ));
                chart = chart.with_legend_entry(label, color, 2.5, LegendEntryKind::LineScatter);
            }
            save(
                r,
                &chart,
                &specs,
                &out.join(format!("style-growth-response-{name}.png")),
            )?;
        }
        Ok(())
    })
}
