//! Response curves with lines, markers and symmetric error bars.
//! All inputs are synthetic.
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example error_bars -- target/error_bars

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::{
    Color, Renderer,
    config::LegendEntryKind,
    data_config::{DataErrorBarStyleConfig, DataRenderType, ErrorRef},
};
use std::path::Path;

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let blue = Color::from_rgb8(34, 104, 198);
    let orange = Color::from_rgb8(224, 123, 38);
    let x: Vec<f64> = (0..18).map(|i| i as f64 * 0.5).collect();
    r.add_column("response-x", &column(x.clone()))?;
    let mut specs = Vec::new();
    for (name, color, speed) in [("a", blue, 0.65), ("b", orange, 0.32)] {
        let y = x
            .iter()
            .enumerate()
            .map(|(i, t)| 100.0 * (1.0 - (-speed * t).exp()) + 1.6 * (i as f64 * 1.7).sin())
            .collect();
        r.add_column(format!("response-{name}"), &column(y))?;
        r.add_column(
            format!("error-{name}"),
            &column(x.iter().map(|t| 3.0 + 0.4 * t).collect()),
        )?;
        specs.push(series(
            name,
            "response-x",
            &format!("response-{name}"),
            DataRenderType::LineScatterErrorbarY {
                line: line(color, 2.5),
                scatter: scatter(color, 6.0),
                err_y: ErrorRef::Symmetric {
                    column: format!("error-{name}"),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: color,
                    error_bar_width: 1.5,
                    error_bar_cap_size: 7.0,
                    cap_width: 1.5,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                },
            },
        ));
    }
    let mut c = chart(
        "Response curves with uncertainty",
        "Time (s)",
        "Response (%)",
        (960, 600),
    )
    .with_legend_entry("Fast response", blue, 2.5, LegendEntryKind::Line)
    .with_legend_entry("Slow response", orange, 2.5, LegendEntryKind::Line);
    c.set_x_range(-0.3, 8.8);
    c.config_mut().legend.corner = renderer::config::LegendCorner::TopLeft;
    c.set_y_range(-5.0, 115.0);
    save(r, &c, &specs, &out.join("gallery-errorbars.png"))
}

fn main() -> Result {
    let out = output_directory("target/error_bars")?;
    with_renderer(|r| render(r, &out))
}
