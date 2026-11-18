//! Export the error bar, histogram, contour and animation gallery examples
//! and the 0.19 chart scenes (ternary, 3D surface, triangle mesh, parallel
//! coordinates, range bars, stems, broken axes, Feynman diagrams) with one
//! shared GPU device.
//! cargo run --locked -p figgy-renderer --example readme_gallery -- target/readme-gallery
//! Optional second argument: frame count (2–240, default 48).
//! Open the individual example files for their data and chart settings.
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::Result;
#[allow(dead_code)]
mod animated_lines;
#[allow(dead_code)]
mod broken_axes;
#[allow(dead_code)]
mod error_bars;
#[allow(dead_code)]
mod feynman;
#[allow(dead_code)]
mod heatmap_contours;
#[allow(dead_code)]
mod histogram;
#[allow(dead_code)]
mod parallel;
#[allow(dead_code)]
mod range_bars;
#[allow(dead_code)]
mod stem;
#[allow(dead_code)]
mod surface_3d;
#[allow(dead_code)]
mod ternary;
#[allow(dead_code)]
mod trimesh;

fn main() -> Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() > 2 {
        return Err("usage: readme_gallery [output-directory] [frames]".into());
    }
    let out = std::path::PathBuf::from(
        args.first()
            .map(String::as_str)
            .unwrap_or("target/readme-gallery"),
    );
    let frames = args
        .get(1)
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(48);
    if !(2..=240).contains(&frames) {
        return Err("frames must be between 2 and 240".into());
    }
    std::fs::create_dir_all(&out)?;
    gallery::with_renderer(|renderer| {
        error_bars::render(renderer, &out)?;
        histogram::render(renderer, &out)?;
        heatmap_contours::render(renderer, &out)?;
        ternary::render(renderer, &out)?;
        surface_3d::render(renderer, &out)?;
        trimesh::render(renderer, &out)?;
        parallel::render(renderer, &out)?;
        range_bars::render(renderer, &out)?;
        stem::render(renderer, &out)?;
        broken_axes::render(renderer, &out)?;
        feynman::render(renderer, &out)?;
        animated_lines::render(renderer, &out, frames)
    })
}
