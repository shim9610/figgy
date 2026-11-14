//! Matching Precise and Sketch growth-curve previews for the README style row.
//! cargo run --locked -p figgy-renderer --example precise_sketch_preview -- target/precise-sketch
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::{
    Chart, Color, SeriesConfig,
    config::{DrawStyle, LegendCorner, SketchOptions},
    theme::ChartKind,
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
        // The documented look at the style row's text sizes, without minor
        // grid lines, with heavier lines and markers.
        let mut theme = theme_named(ChartKind::Line, "default")?;
        theme.chrome.tick_label_size = 25.;
        theme.chrome.axis_title_size = 32.;
        theme.chrome.legend_size = 23.;
        theme.chrome.legend_padding = 9.;
        theme.chrome.grid.minor_x.show = false;
        theme.chrome.grid.minor_y.show = false;
        theme.series.lines.line_width = 2.5;
        theme.series.lines.marker_size = 8.;
        // Each draw style with its own pair of colours.
        for (name, style, palette) in [
            (
                "precise",
                DrawStyle::Precise,
                vec![Color::BLACK, Color::from_rgb8(230, 45, 50)],
            ),
            (
                "sketch",
                DrawStyle::Sketch(SketchOptions {
                    amplitude_px: 2.2,
                    wavelength_px: 55.,
                    seed: 1,
                }),
                vec![
                    Color::from_rgb8(218, 157, 200),
                    Color::from_rgb8(147, 193, 207),
                ],
            ),
        ] {
            theme.chrome.draw_style = style;
            theme.series.palette = palette;
            let mut specs = [("control", "Control mean"), ("treated", "Treated mean")]
                .map(|(id, label)| SeriesConfig::scatter_line(id, "time", id).label(label));
            let mut chart = Chart::sized(1280, 1024)
                .with_x_title("Time [min]")
                .with_y_title("Response mean");
            chart.set_x_range(-6., 126.);
            chart.set_y_range(-0.05, 1.08);
            themed(&mut chart, &theme, &mut specs)?;
            let mut chart = chart.with_legend_from(&specs);
            chart.config_mut().legend.corner = LegendCorner::TopLeft;
            save(
                r,
                &mut chart,
                &specs,
                &out.join(format!("style-growth-response-{name}.png")),
            )?;
        }
        Ok(())
    })
}
