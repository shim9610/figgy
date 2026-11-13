//! Moving waves and a Lissajous trail; export frames before encoding GIFs.
//! All inputs are synthetic.
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example animated_lines -- target/animated_lines

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::{Color, Renderer, SeriesConfig, theme::ChartKind};
use std::f64::consts::TAU;
use std::path::Path;

pub fn render(r: &mut Renderer, out: &Path, frames: usize) -> Result {
    let theme = theme_named(ChartKind::Line, "gallery")?;
    let x: Vec<f64> = (0..401).map(|i| i as f64 / 400.0 * TAU * 2.0).collect();
    r.add_column("wave-x", &x)?;
    r.add_column("wave-a", &vec![0.0; x.len()])?;
    r.add_column("wave-b", &vec![0.0; x.len()])?;
    let mut specs = [
        SeriesConfig::line("primary", "wave-x", "wave-a"),
        SeriesConfig::line("harmonic", "wave-x", "wave-b"),
    ];
    let mut wave = titled("Waves in motion", "Position", "Amplitude", (720, 460));
    wave.set_x_range(0.0, TAU * 2.0);
    wave.set_y_range(-1.6, 1.6);
    themed(&mut wave, &theme, &mut specs)?;

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
    // The path is a pale tint of the trail's colour on the white paper, the
    // head the second colour. The tint is opaque, so the path's joints do
    // not darken where its segments overlap.
    let mut orbit_theme = theme.clone();
    let [blue, coral, ..] = theme.series.palette[..] else {
        return Err("the gallery theme has two colours".into());
    };
    let tint = |c: f32| 1.0 - 0.33 * (1.0 - c);
    let pale = Color::from_rgba(tint(blue.r), tint(blue.g), tint(blue.b), 1.0);
    orbit_theme.series.palette = vec![pale, blue, coral];
    let mut orbit_specs = [
        SeriesConfig::line("path", "orbit-x", "orbit-y"),
        SeriesConfig::line("trail", "trail-x", "trail-y"),
        SeriesConfig::scatter("head", "head-x", "head-y"),
    ];
    let mut orbit = titled(
        "A 2:3 phase portrait",
        "sin(2t)",
        "sin(3t + 0.4)",
        (720, 460),
    );
    orbit.set_x_range(-1.2, 1.2);
    orbit.set_y_range(-1.2, 1.2);
    themed(&mut orbit, &orbit_theme, &mut orbit_specs)?;
    // The path is a hairline guide under the trail.
    orbit_specs[0] = orbit_specs[0].clone().width(1.5);
    for name in ["wave", "orbit"] {
        std::fs::create_dir_all(out.join(name))?;
    }
    for i in 0..frames {
        let phase = TAU * i as f64 / frames as f64;
        let wave_a: Vec<f64> = x.iter().map(|x| (x - phase).sin()).collect();
        let wave_b: Vec<f64> = x.iter().map(|x| 0.55 * (2.0 * x + phase).sin()).collect();
        r.upsert_column("wave-a", &wave_a)?;
        r.upsert_column("wave-b", &wave_b)?;
        save(r, &mut wave, &specs, &out.join(format!("wave/{i:03}.png")))?;
        let trail: Vec<f64> = (0..101).map(|j| phase - 0.8 + j as f64 * 0.008).collect();
        let trail_x: Vec<f64> = trail.iter().map(|t| (2.0 * t).sin()).collect();
        let trail_y: Vec<f64> = trail.iter().map(|t| (3.0 * t + 0.4).sin()).collect();
        r.upsert_column("trail-x", &trail_x)?;
        r.upsert_column("trail-y", &trail_y)?;
        r.upsert_column("head-x", &vec![(2.0 * phase).sin()])?;
        r.upsert_column("head-y", &vec![(3.0 * phase + 0.4).sin()])?;
        save(
            r,
            &mut orbit,
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
