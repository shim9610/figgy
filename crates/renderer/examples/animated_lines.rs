//! Moving waves and a Lissajous trail; export frames before encoding GIFs.
//! All inputs are synthetic.
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example animated_lines -- target/animated_lines

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::{Color, Renderer, SeriesConfig, data_config::ScatterShape};
use std::f64::consts::TAU;
use std::path::Path;

pub fn render(r: &mut Renderer, out: &Path, frames: usize) -> Result {
    let blue = Color::from_rgb8(34, 104, 198);
    let coral = Color::from_rgb8(225, 81, 79);
    let x: Vec<f64> = (0..401).map(|i| i as f64 / 400.0 * TAU * 2.0).collect();
    r.add_column("wave-x", &x)?;
    r.add_column("wave-a", &vec![0.0; x.len()])?;
    r.add_column("wave-b", &vec![0.0; x.len()])?;
    let specs = [
        SeriesConfig::line("primary", "wave-x", "wave-a")
            .color(blue)
            .width(3.0),
        SeriesConfig::line("harmonic", "wave-x", "wave-b")
            .color(coral)
            .width(2.5),
    ];
    let mut wave = chart("Waves in motion", "Position", "Amplitude", (720, 460));
    wave.set_x_range(0.0, TAU * 2.0);
    wave.set_y_range(-1.6, 1.6);

    let curve: Vec<f64> = (0..501).map(|i| TAU * i as f64 / 500.0).collect();
    let orbit_x: Vec<f64> = curve.iter().map(|t| (2.0 * t).sin()).collect();
    let orbit_y: Vec<f64> = curve.iter().map(|t| (3.0 * t + 0.4).sin()).collect();
    r.add_column("orbit-x", &orbit_x)?;
    r.add_column("orbit-y", &orbit_y)?;
    for id in ["trail-x", "trail-y"] {
        r.add_column(id, &vec![0.0; 101])?;
    }
    for id in ["head-x", "head-y"] {
        r.add_column(id, &vec![0.0])?;
    }
    let orbit_specs = [
        SeriesConfig::line("path", "orbit-x", "orbit-y")
            .color(Color::from_rgb8(186, 204, 226))
            .width(1.5),
        SeriesConfig::line("trail", "trail-x", "trail-y")
            .color(blue)
            .width(3.5),
        SeriesConfig::scatter("head", "head-x", "head-y")
            .color(coral)
            .marker(ScatterShape::CircleFilled, 11.0),
    ];
    let mut orbit = chart(
        "A 2:3 phase portrait",
        "sin(2t)",
        "sin(3t + 0.4)",
        (720, 460),
    );
    orbit.set_x_range(-1.2, 1.2);
    orbit.set_y_range(-1.2, 1.2);
    for name in ["wave", "orbit"] {
        std::fs::create_dir_all(out.join(name))?;
    }
    for i in 0..frames {
        let phase = TAU * i as f64 / frames as f64;
        let wave_a: Vec<f64> = x.iter().map(|x| (x - phase).sin()).collect();
        let wave_b: Vec<f64> = x.iter().map(|x| 0.55 * (2.0 * x + phase).sin()).collect();
        r.upsert_column("wave-a", &wave_a)?;
        r.upsert_column("wave-b", &wave_b)?;
        save(r, &wave, &specs, &out.join(format!("wave/{i:03}.png")))?;
        let trail: Vec<f64> = (0..101).map(|j| phase - 0.8 + j as f64 * 0.008).collect();
        let trail_x: Vec<f64> = trail.iter().map(|t| (2.0 * t).sin()).collect();
        let trail_y: Vec<f64> = trail.iter().map(|t| (3.0 * t + 0.4).sin()).collect();
        r.upsert_column("trail-x", &trail_x)?;
        r.upsert_column("trail-y", &trail_y)?;
        r.upsert_column("head-x", &vec![(2.0 * phase).sin()])?;
        r.upsert_column("head-y", &vec![(3.0 * phase + 0.4).sin()])?;
        save(
            r,
            &orbit,
            &orbit_specs,
            &out.join(format!("orbit/{i:03}.png")),
        )?;
    }
    Ok(())
}

fn main() -> Result {
    let out = output_directory("target/animated_lines")?;
    let frames = std::env::args()
        .nth(2)
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(48);
    if !(2..=240).contains(&frames) {
        return Err("frames must be between 2 and 240".into());
    }
    with_renderer(|r| render(r, &out, frames))
}
