//! Response curves with lines, markers and symmetric error bars.
//! All inputs are synthetic.
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example error_bars -- target/error_bars

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::{Result, chart, output_directory, save, with_renderer};
use renderer::{Color, Renderer, SeriesConfig, config::LegendCorner};
use std::path::Path;

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let blue = Color::from_rgb8(34, 104, 198);
    let orange = Color::from_rgb8(224, 123, 38);
    let x: Vec<f64> = (0..18).map(|i| i as f64 * 0.5).collect();
    r.add_column("response-x", &x)?;
    let mut specs = Vec::new();
    for (name, label, color, speed) in [
        ("a", "Fast response", blue, 0.65),
        ("b", "Slow response", orange, 0.32),
    ] {
        let y: Vec<f64> = x
            .iter()
            .enumerate()
            .map(|(i, t)| 100.0 * (1.0 - (-speed * t).exp()) + 1.6 * (i as f64 * 1.7).sin())
            .collect();
        let error: Vec<f64> = x.iter().map(|t| 3.0 + 0.4 * t).collect();
        r.add_column(format!("response-{name}"), &y)?;
        r.add_column(format!("error-{name}"), &error)?;
        specs.push(
            SeriesConfig::scatter_line(name, "response-x", format!("response-{name}"))
                .label(label)
                .color(color)
                .width(2.5)
                .y_errors(format!("error-{name}"))
                .error_bars(1.5, 7.0),
        );
    }
    let mut c = chart(
        "Response curves with uncertainty",
        "Time (s)",
        "Response (%)",
        (960, 600),
    )
    .with_legend_from(&specs);
    c.set_x_range(-0.3, 8.8);
    c.config_mut().legend.corner = LegendCorner::TopLeft;
    c.set_y_range(-5.0, 115.0);
    save(r, &c, &specs, &out.join("gallery-errorbars.png"))
}

fn main() -> Result {
    let out = output_directory("target/error_bars")?;
    with_renderer(|r| render(r, &out))
}
