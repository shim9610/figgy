//! Tier 2 of #46: leaf mutations against `ArcResultKey`. The key reads the
//! transform's first 16 words (data range and pixel geometry) and its last
//! 8 (polar), leaving out words 16..28, the style parameters: a style edit
//! must not retraverse the data, so the arc prefix must not read them.
//!
//! For every leaf of a dashed-line chart's `Config`, in each draw style,
//! the edited transform goes through `ensure_arc_prefix` under the series
//! id the baseline was scanned under. A cache hit (the same buffer back)
//! means the key matched; the scan is then run again from nothing under a
//! fresh series id, and its prefix must equal the cached one byte for byte.
use super::*;
use crate::config::{ConstellationOptions, DrawStyle, MilkywayOptions, SketchOptions};
use crate::renderer::config_mutation::{Outcome, Tally, run};

fn read_buffer(r: &Renderer, buffer: &wgpu::Buffer, len: u64) -> Vec<u8> {
    let staging = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("arc prefix readback"),
        size: len,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, len);
    r.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    r.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("arc prefix readback");
    rx.recv().unwrap().expect("arc prefix map");
    let bytes = staging.slice(..).get_mapped_range().unwrap().to_vec();
    staging.unmap();
    bytes
}

/// The star-pass pitch `prepare_arc_items` passes for a line in `style`.
fn star_pitch(style: &DrawStyle) -> Option<f32> {
    match style {
        DrawStyle::Milkyway(c) => {
            Some((data_render::line_arc::STAR_SLOT_PITCH_FACTOR * c.structure_scale).max(1e-3))
        }
        _ => None,
    }
}

fn arc(r: &mut Renderer, series_id: &str, config: &Config) -> Option<(Arc<wgpu::Buffer>, u64)> {
    let t = data_render::scatter_transform_from_config(config);
    let prepared = r.ensure_arc_prefix(
        series_id,
        "arc_x",
        "arc_y",
        &t,
        star_pitch(&config.draw_style),
        LineInterpolation::Linear,
    )?;
    Some(prepared.prefix)
}

fn baseline(style: DrawStyle) -> Config {
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config.draw_style = style;
    let mut chart = Chart::new(config);
    // Positive, so a logarithmic scale is a valid edit.
    chart.set_x_range(0.01, 1.0);
    chart.set_y_range(0.01, 1.0);
    chart.config().clone()
}

#[test]
fn every_config_leaf_the_arc_key_leaves_out_keeps_the_arc_prefix() {
    let Some(mut r) = state_test_renderer() else {
        return;
    };
    let n = 64;
    r.add_column(
        "arc_x",
        &col_f64(
            (0..n)
                .map(|i| 0.01 + 0.99 * i as f64 / (n - 1) as f64)
                .collect(),
        ),
    )
    .unwrap();
    r.add_column(
        "arc_y",
        &col_f64((0..n).map(|i| 0.5 + 0.4 * (i as f64 * 0.3).sin()).collect()),
    )
    .unwrap();
    let mut total = Tally::default();
    let mut fresh_id = 0usize;
    // Mutations whose transform differs only in the words the key leaves
    // out: the ones this test exists for.
    let mut style_words = Vec::new();
    for (name, style) in [
        ("precise", DrawStyle::Precise),
        ("sketch", DrawStyle::Sketch(SketchOptions::default())),
        ("milkyway", DrawStyle::Milkyway(MilkywayOptions::default())),
        (
            "constellation",
            DrawStyle::Constellation(ConstellationOptions::default()),
        ),
    ] {
        let config = baseline(style);
        crate::chart::validate_renderer_config(&config).unwrap();
        let series_id = format!("arc-{name}");
        let (cached, len) = arc(&mut r, &series_id, &config).expect("the baseline scans");
        let expected = read_buffer(&r, &cached, len);
        drop(cached);
        let tally = run(
            &format!("{name}: "),
            &config,
            &["Comic Neue"],
            |label, next| {
                if crate::chart::validate_renderer_config(&next).is_err() {
                    return Outcome::Rejected;
                }
                // The bounded cache may have evicted the baseline's result;
                // asking for it again puts it back under the same key.
                let (cached, _) = arc(&mut r, &series_id, &config).expect("the baseline scans");
                let Some((hit, _)) = arc(&mut r, &series_id, &next) else {
                    return Outcome::Rejected;
                };
                if !Arc::ptr_eq(&hit, &cached) {
                    // The key changed: a new scan.
                    return Outcome::Held;
                }
                let words = |config: &Config| -> [u32; 36] {
                    bytemuck::cast(data_render::scatter_transform_from_config(config))
                };
                if words(&next)[16..28] != words(&config)[16..28] {
                    style_words.push(label.to_string());
                }
                fresh_id += 1;
                let (fresh, fresh_len) =
                    arc(&mut r, &format!("arc-fresh-{fresh_id}"), &next).expect("a fresh scan");
                let scanned = read_buffer(&r, &fresh, fresh_len);
                if scanned == expected {
                    Outcome::Held
                } else {
                    Outcome::Gap(format!(
                        "the arc key matched and a fresh scan of the edited transform differs \
                     from the cached prefix in {} of {} bytes",
                        scanned
                            .iter()
                            .zip(&expected)
                            .filter(|(a, b)| a != b)
                            .count(),
                        expected.len()
                    ))
                }
            },
        );
        eprintln!("{}", tally.summary(&format!("arc/{name}")));
        total.absorb(tally);
    }
    eprintln!("{}", total.summary("arc (all)"));
    eprintln!(
        "{} mutations kept the key and changed only the left-out style words:\n{}",
        style_words.len(),
        style_words.join("\n")
    );
    assert!(
        !style_words.is_empty(),
        "the draw-style baselines must exercise the words the key leaves out"
    );
    eprintln!("skipped:\n{}", total.skipped.join("\n"));
    assert!(
        total.gaps.is_empty(),
        "{} config leaves change the arc prefix while ArcResultKey matches:\n{}",
        total.gaps.len(),
        total.gaps.join("\n")
    );
}
