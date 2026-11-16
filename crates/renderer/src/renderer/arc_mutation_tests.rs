//! Tier 2 of #46: leaf mutations against `ArcResultKey`. The key reads the
//! transform's first 16 words (data range and pixel geometry) and every
//! word after the style parameters (polar, ternary and the broken axes'
//! cut table), leaving out words 16..28: a style edit must not retraverse
//! the data, so the arc prefix must not read them.
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

fn arc(r: &mut Renderer, series_id: &str, config: &Config) -> Option<(wgpu::Buffer, u64)> {
    let t = data_render::scatter_transform_from_config(config);
    let prepared = r
        .ensure_arc_prefix(
            series_id,
            "arc_x",
            "arc_y",
            &t,
            star_pitch(&config.draw_style),
            LineInterpolation::Linear,
        )?
        .unwrap();
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
                if hit != cached {
                    // The key changed: a new scan.
                    return Outcome::Held;
                }
                let words = |config: &Config| -> [u32; 76] {
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

/// A cut moves every sample past it, so the arc lengths change: the key
/// reads the transform's break words (#200), and a break edit scans again
/// to the prefix a fresh scan of the edited chart gives.
#[test]
fn a_break_edit_rescans_the_arc_prefix() {
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
    let config = baseline(DrawStyle::Precise);
    let mut cut = config.clone();
    cut.left_y.breaks = Some(crate::config::AxisBreaks {
        ranges: vec![crate::config::AxisBreak::new(0.4, 0.6)],
        ..Default::default()
    });
    let (before, len) = arc(&mut r, "arc-break", &config).expect("the baseline scans");
    let (after, after_len) = arc(&mut r, "arc-break", &cut).expect("the cut chart scans");
    assert!(after != before, "the break edit reused the unbroken prefix");
    let (fresh, fresh_len) = arc(&mut r, "arc-break-fresh", &cut).expect("a fresh scan");
    assert_eq!(
        read_buffer(&r, &after, after_len),
        read_buffer(&r, &fresh, fresh_len)
    );
    assert_ne!(
        read_buffer(&r, &before, len),
        read_buffer(&r, &fresh, fresh_len),
        "the cut changes the arc lengths"
    );
}

/// A transform that holds axis cuts, a probability scale or polar
/// coordinates is never read through the basic shaders (#299). The entries
/// admit their charts first, which this test leaves out: the scan refuses
/// the transform instead of measuring the path without the feature, and
/// the renderer is left as it was. Breaks that cut nothing (no ranges)
/// leave the transform without cuts, and the chart on the basic shaders; and
/// whatever puts cuts, a probability code or the polar flag in a transform
/// is a config the admission sees.
#[test]
fn a_transform_that_needs_the_full_shaders_is_refused_on_basic_shaders() {
    let Some(mut r) = state_test_renderer() else {
        return;
    };
    r.add_column("arc_x", &col_f64(vec![0.1, 0.5, 0.9]))
        .unwrap();
    r.add_column("arc_y", &col_f64(vec![0.1, 0.9, 0.2]))
        .unwrap();
    let config = baseline(DrawStyle::Precise);
    let with = |ranges: Vec<crate::config::AxisBreak>, style| {
        let mut cut = config.clone();
        cut.left_y.breaks = Some(crate::config::AxisBreaks {
            ranges,
            style,
            ..Default::default()
        });
        cut
    };
    let continuous = crate::config::AxisBreakStyle::Continuous;
    let cut = with(vec![crate::config::AxisBreak::new(0.4, 0.6)], continuous);
    let empty = with(Vec::new(), continuous);
    let panels = with(
        vec![crate::config::AxisBreak::new(0.4, 0.6)],
        crate::config::AxisBreakStyle::Panels,
    );
    let outside = with(vec![crate::config::AxisBreak::new(4.0, 6.0)], continuous);
    for (name, candidate) in [
        ("plain", &config),
        ("cut", &cut),
        ("no ranges", &empty),
        ("panels", &panels),
        ("outside the range", &outside),
    ] {
        let transform = data_render::scatter_transform_from_config(candidate);
        assert!(
            !transform.needs_full_shaders() || super::config_needs_full_shaders(candidate),
            "{name}: a transform with cuts comes from a config the admission sees"
        );
    }
    assert!(!data_render::scatter_transform_from_config(&config).needs_full_shaders());
    assert!(!data_render::scatter_transform_from_config(&empty).needs_full_shaders());
    assert!(!super::config_needs_full_shaders(&config));
    assert!(!super::config_needs_full_shaders(&empty));
    assert!(super::config_needs_full_shaders(&cut));

    // A probability axis and polar coordinates, each on its own.
    let mut probability = config.clone();
    probability.left_y.scale =
        crate::config::AxisScale::Probability(crate::config::ProbabilityScale::Normal);
    probability.left_y.min = 0.05;
    probability.left_y.max = 0.95;
    let mut polar = config.clone();
    polar.set_polar(Some(crate::polar::PolarOptions::default()));
    for (name, candidate) in [("probability", &probability), ("polar", &polar)] {
        assert!(super::config_needs_full_shaders(candidate), "{name}");
        assert!(
            data_render::scatter_transform_from_config(candidate).needs_full_shaders(),
            "{name}"
        );
    }

    for (name, candidate) in [
        ("cut", &cut),
        ("probability", &probability),
        ("polar", &polar),
    ] {
        let transform = data_render::scatter_transform_from_config(candidate);
        assert!(transform.needs_full_shaders(), "{name}");
        assert_eq!(r.shader_set(), ShaderSet::Basic);
        let refused = r.preparation_parts().1.ensure_arc_prefix(
            "refused",
            "arc_x",
            "arc_y",
            None,
            &transform,
            None,
            LineInterpolation::Linear,
        );
        assert!(
            matches!(&refused, Err(error) if error.to_string().contains("full shaders")),
            "{name}: {:?}",
            refused.map(|_| ())
        );
        assert_eq!(r.shader_set(), ShaderSet::Basic, "{name}");
    }
    // The unbroken chart still scans on the basic shaders, and the cut
    // one once it has been admitted.
    assert!(arc(&mut r, "plain", &config).is_some());
    assert_eq!(r.shader_set(), ShaderSet::Basic);
    assert!(arc(&mut r, "cut", &cut).is_some());
    assert_eq!(r.shader_set(), ShaderSet::Full);
}
