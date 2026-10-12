//! Explicit bin edges and counts, in vertical and horizontal orientations.
//! All inputs are synthetic.
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example histogram -- target/histogram

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::{
    Color, Renderer,
    data_config::{BarOrientation, DataBarStyleConfig, DataRenderType},
};
use std::path::Path;

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let teal = Color::from_rgb8(19, 154, 139);
    let edges: Vec<f64> = (0..41).map(|i| -4.0 + i as f64 * 0.2).collect();
    let counts = edges
        .windows(2)
        .map(|v| {
            let x = (v[0] + v[1]) * 0.5;
            (100.0 * (-0.5 * ((x + 1.2) / 0.65).powi(2)).exp()
                + 68.0 * (-0.5 * ((x - 1.1) / 0.9).powi(2)).exp())
            .round()
        })
        .collect();
    r.add_column("bin-edges", &column(edges))?;
    r.add_column("bin-counts", &column(counts))?;
    let mut specs = [series(
        "distribution",
        "bin-edges",
        "bin-counts",
        DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: teal,
                border_color: Color::from_rgb8(10, 102, 95),
                border_width: 0.8,
                baseline: 0.0,
                gap_px: 2.0,
                width_ratio: 1.0,
                orientation: BarOrientation::Vertical,
                bar_style_overrides: None,
            },
        },
    )];
    let mut c = chart(
        "A two-population distribution",
        "Measured value",
        "Count",
        (960, 600),
    );
    c.set_x_range(-4.2, 4.2);
    c.set_y_range(0.0, 120.0);
    save(r, &c, &specs, &out.join("gallery-histogram.png"))?;

    // Orientation defines the column roles; reuse the uploaded columns and
    // exchange their bindings together with the axis ranges and titles.
    let s = &mut specs[0];
    std::mem::swap(&mut s.x_column, &mut s.y_column);
    let DataRenderType::Histogram { bar } = &mut s.render_type else {
        unreachable!()
    };
    bar.orientation = BarOrientation::Horizontal;
    let mut horizontal = chart(
        "A two-population distribution",
        "Count",
        "Measured value",
        (960, 600),
    );
    horizontal.set_x_range(0.0, 120.0);
    horizontal.set_y_range(-4.2, 4.2);
    save(
        r,
        &horizontal,
        &specs,
        &out.join("gallery-histogram-horizontal.png"),
    )
}

fn main() -> Result {
    let out = output_directory("target/histogram")?;
    with_renderer(|r| render(r, &out))
}
