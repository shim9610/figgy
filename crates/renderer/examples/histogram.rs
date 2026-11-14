//! Explicit bin edges and counts, in vertical and horizontal orientations.
//! All inputs are synthetic.
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example histogram -- target/histogram

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::{Result, output_directory, save, themed, titled, with_renderer};
use renderer::theme::ThemeFamily;
use renderer::{Renderer, SeriesConfig, theme::ChartKind};
use std::path::Path;

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let theme = ThemeFamily::Gallery.theme(ChartKind::Histogram);
    let edges: Vec<f64> = (0..41).map(|i| -4.0 + i as f64 * 0.2).collect();
    let counts: Vec<f64> = edges
        .windows(2)
        .map(|v| {
            let x = (v[0] + v[1]) * 0.5;
            (100.0 * (-0.5 * ((x + 1.2) / 0.65).powi(2)).exp()
                + 68.0 * (-0.5 * ((x - 1.1) / 0.9).powi(2)).exp())
            .round()
        })
        .collect();
    r.add_column("bin-edges", &edges)?;
    r.add_column("bin-counts", &counts)?;
    let mut vertical = SeriesConfig::histogram("distribution", "bin-edges", "bin-counts");
    let title = "A two-population distribution";
    let mut c = titled(title, "Measured value", "Count", (960, 600));
    c.set_x_range(-4.2, 4.2);
    c.set_y_range(0.0, 120.0);
    themed(&mut c, &theme, std::slice::from_mut(&mut vertical))?;
    let specs = std::slice::from_ref(&vertical);
    save(r, &mut c, specs, &out.join("gallery-histogram.png"))?;

    // Orientation defines the column roles; reuse the uploaded columns and
    // exchange their bindings together with the axis ranges and titles.
    let mut horizontal = vertical.horizontal();
    std::mem::swap(&mut horizontal.x_column, &mut horizontal.y_column);
    let mut c = titled(title, "Count", "Measured value", (960, 600));
    c.set_x_range(0.0, 120.0);
    c.set_y_range(-4.2, 4.2);
    let mut specs = [horizontal];
    themed(&mut c, &theme, &mut specs)?;
    save(
        r,
        &mut c,
        &specs,
        &out.join("gallery-histogram-horizontal.png"),
    )
}

fn main() -> Result {
    let out = output_directory("target/histogram")?;
    with_renderer(|r| render(r, &out))
}
