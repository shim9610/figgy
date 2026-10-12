//! Moving waves and a Lissajous trail; export frames before encoding GIFs.
//! All inputs are synthetic.
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example animated_lines -- target/animated_lines

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::{Color, Renderer, data_config::DataRenderType};
use std::f64::consts::TAU;
use std::path::Path;

pub fn render(r: &mut Renderer, out: &Path, frames: usize) -> Result {
    let blue = Color::from_rgb8(34, 104, 198);
    let coral = Color::from_rgb8(225, 81, 79);
    let x: Vec<f64> = (0..401).map(|i| i as f64 / 400.0 * TAU * 2.0).collect();
    r.add_column("wave-x", &column(x.clone()))?;
    r.add_column("wave-a", &column(vec![0.0; x.len()]))?;
    r.add_column("wave-b", &column(vec![0.0; x.len()]))?;
    let specs = [
        series(
            "primary",
            "wave-x",
            "wave-a",
            DataRenderType::Line {
                line: line(blue, 3.0),
            },
        ),
        series(
            "harmonic",
            "wave-x",
            "wave-b",
            DataRenderType::Line {
                line: line(coral, 2.5),
            },
        ),
    ];
    let mut wave = chart("Waves in motion", "Position", "Amplitude", (720, 460));
    wave.set_x_range(0.0, TAU * 2.0);
    wave.set_y_range(-1.6, 1.6);

    let curve: Vec<f64> = (0..501).map(|i| TAU * i as f64 / 500.0).collect();
    r.add_column(
        "orbit-x",
        &column(curve.iter().map(|t| (2.0 * t).sin()).collect()),
    )?;
    r.add_column(
        "orbit-y",
        &column(curve.iter().map(|t| (3.0 * t + 0.4).sin()).collect()),
    )?;
    for id in ["trail-x", "trail-y"] {
        r.add_column(id, &column(vec![0.0; 101]))?;
    }
    for id in ["head-x", "head-y"] {
        r.add_column(id, &column(vec![0.0]))?;
    }
    let orbit_specs = [
        series(
            "path",
            "orbit-x",
            "orbit-y",
            DataRenderType::Line {
                line: line(Color::from_rgb8(186, 204, 226), 1.5),
            },
        ),
        series(
            "trail",
            "trail-x",
            "trail-y",
            DataRenderType::Line {
                line: line(blue, 3.5),
            },
        ),
        series(
            "head",
            "head-x",
            "head-y",
            DataRenderType::Scatter {
                scatter: scatter(coral, 11.0),
            },
        ),
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
        r.upsert_column(
            "wave-a",
            &column(x.iter().map(|x| (x - phase).sin()).collect()),
        )?;
        r.upsert_column(
            "wave-b",
            &column(x.iter().map(|x| 0.55 * (2.0 * x + phase).sin()).collect()),
        )?;
        save(r, &wave, &specs, &out.join(format!("wave/{i:03}.png")))?;
        let trail: Vec<f64> = (0..101).map(|j| phase - 0.8 + j as f64 * 0.008).collect();
        r.upsert_column(
            "trail-x",
            &column(trail.iter().map(|t| (2.0 * t).sin()).collect()),
        )?;
        r.upsert_column(
            "trail-y",
            &column(trail.iter().map(|t| (3.0 * t + 0.4).sin()).collect()),
        )?;
        r.upsert_column("head-x", &column(vec![(2.0 * phase).sin()]))?;
        r.upsert_column("head-y", &column(vec![(3.0 * phase + 0.4).sin()]))?;
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
