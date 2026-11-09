//! Matching Precise and Sketch growth-curve previews for the README style row.
//! cargo run --locked -p figgy-renderer --example precise_sketch_preview -- target/precise-sketch
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::{
    Chart, Color, SeriesConfig,
    config::{DrawStyle, LegendCorner, SketchOptions},
    data_config::ScatterShape,
    theme::{SubplotScope, Theme},
};

fn main() -> Result {
    let out = output_directory("target/precise-sketch")?;
    with_renderer(|r| {
        let times: Vec<f64> = vec![0., 5., 10., 15., 20., 30., 45., 60., 90., 120.];
        r.add_column("time", &times)?;
        for (id, ceiling, tau) in [("control", 0.87, 40.), ("treated", 1.04, 19.)] {
            let values: Vec<f64> = times
                .iter()
                .map(|t| 0.06 + (ceiling - 0.06) * (1. - (-t / tau).exp()))
                .collect();
            r.add_column(id, &values)?;
        }
        // The style row's text sizes, without minor grid lines.
        let mut theme = Theme::light();
        theme.chrome.tick_label_size = 25.;
        theme.chrome.axis_title_size = 32.;
        theme.chrome.legend_size = 23.;
        theme.chrome.grid.minor_x.show = false;
        theme.chrome.grid.minor_y.show = false;
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
            let mut chart = Chart::sized(1280, 1024);
            theme.apply_chrome(chart.config_mut(), SubplotScope::All);
            let cfg = chart.config_mut();
            // The four style previews share these margins, so their plot
            // areas line up in the row whatever face a draw style uses.
            cfg.chart_title.top_margin = 60.;
            cfg.left_y.out_margin = 200.;
            cfg.bottom_x.out_margin = 145.;
            cfg.top_x.out_margin = 32.;
            cfg.right_y.out_margin = 64.;
            // After the theme, which writes the draw style too.
            cfg.draw_style = style;
            cfg.legend.corner = LegendCorner::TopLeft;
            cfg.legend.padding = 9.;
            let [control, treated] = colors;
            let specs = [
                ("control", "Control mean", control),
                ("treated", "Treated mean", treated),
            ]
            .map(|(id, label, color)| {
                SeriesConfig::scatter_line(id, "time", id)
                    .label(label)
                    .color(color)
                    .width(2.5)
                    .marker(ScatterShape::CircleFilled, 8.)
            });
            let mut chart = chart
                .with_x_title("Time [min]")
                .with_y_title("Response mean")
                .with_legend_from(&specs);
            chart.set_x_range(-6., 126.);
            chart.set_y_range(-0.05, 1.08);
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
