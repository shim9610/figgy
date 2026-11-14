//! Response curves with lines, markers and symmetric error bars.
//! All inputs are synthetic.
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example error_bars -- target/error_bars

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::{Result, output_directory, save, theme_named, themed, titled, with_renderer};
use renderer::{Renderer, SeriesConfig, config::LegendCorner, theme::ChartKind};
use std::path::Path;

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::ErrorBars, "gallery")?;
    let x: Vec<f64> = (0..18).map(|i| i as f64 * 0.5).collect();
    r.add_column("response-x", &x)?;
    let mut specs = Vec::new();
    // Each curve named with its time constant τ = 1/speed, in TeX.
    for (name, label, speed) in [
        ("a", r"Fast response, $\tau = 1.5$ s", 0.65),
        ("b", r"Slow response, $\tau = 3.1$ s", 0.32),
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
                .y_errors(format!("error-{name}")),
        );
    }
    // The error bars are one standard deviation.
    let mut c = titled(
        "Response curves with uncertainty",
        "Time (s)",
        r"Response $\pm 1\sigma$ (%)",
        (960, 600),
    );
    c.set_x_range(-0.3, 8.8);
    c.set_y_range(-5.0, 115.0);
    themed(&mut c, &theme, &mut specs)?;
    let mut c = c.with_legend_from(&specs);
    c.config_mut().legend.corner = LegendCorner::TopLeft;
    save(r, &mut c, &specs, &out.join("gallery-errorbars.png"))
}

fn main() -> Result {
    let out = output_directory("target/error_bars")?;
    with_renderer(|r| render(r, &out))
}
