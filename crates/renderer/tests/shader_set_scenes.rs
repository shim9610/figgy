//! The two shader sets draw the same chart (#299).
//!
//! A renderer compiles the basic data modules until it is first handed a
//! chart with an axis break, a probability axis or polar coordinates, and
//! the full modules from then on (`shader_assembly::ShaderSet`). A chart
//! with none of the three may therefore be drawn by either, depending on
//! what its renderer was handed before. This suite draws every Cartesian
//! scene of the playground catalogue (`support/playground_scenes.rs`)
//! through both and holds the exports to the same pixels, so no fragment of
//! one set can come to place, shade or clip a plain chart differently
//! from its twin. The scenes that use one of the three features move their
//! renderer to the full modules themselves, and only those do; the suite
//! holds the renderer's choice to its own reading of each scene's model.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/playground_scenes.rs"]
mod scenes;

use renderer::config::{AxisScale, ProbabilityScale};
use renderer::shader_assembly::ShaderSet;
use renderer::{Chart, Config, ExportOptions, RasterImage, Renderer};

use scenes::SceneKind;

/// Points of the large-data scene: enough to draw it, small enough for an
/// unoptimized build on a software GPU.
const BIG_DATA_POINTS: usize = 20_000;

/// Scenes known to use each of the three features, so the oracle below is
/// held to something: polar coordinates, an axis break and a probability
/// axis.
const KNOWN_FULL_SCENES: &[SceneKind] = &[
    SceneKind::Polar,
    SceneKind::PolarHeatmap,
    SceneKind::BrokenAxes,
    SceneKind::ProbabilityPaper,
];

/// Whether any layer of `config` has an axis break that cuts, a probability
/// axis or polar coordinates: this suite's own reading of the model, which
/// the renderer's choice of variant is held to.
fn uses_a_full_feature(config: &Config) -> bool {
    let axis = |axis: &renderer::config::AxisOptions| {
        axis.breaks
            .as_ref()
            .is_some_and(|breaks| !breaks.ranges.is_empty())
            || matches!(axis.scale, AxisScale::Probability(_))
    };
    config.polar().is_some()
        || [
            &config.bottom_x,
            &config.left_y,
            &config.top_x,
            &config.right_y,
        ]
        .into_iter()
        .any(axis)
        || config
            .overlays
            .iter()
            .any(|overlay| axis(&overlay.x_axis) || axis(&overlay.y_axis))
        || config
            .subplots
            .iter()
            .any(|subplot| uses_a_full_feature(&subplot.config))
}

fn renderer() -> Renderer {
    Renderer::headless_blocking().expect("headless renderer")
}

/// A renderer that has been handed a chart with a probability axis, so it
/// compiles the full modules for whatever it draws next.
fn full_renderer() -> Renderer {
    let mut r = renderer();
    let mut config = renderer::default::default_config();
    config.left_y.scale = AxisScale::Probability(ProbabilityScale::Normal);
    config.left_y.min = 0.05;
    config.left_y.max = 0.95;
    let id = r
        .register_chart(config, Vec::new())
        .expect("a chart with a probability axis");
    r.remove_chart(id).unwrap();
    assert_eq!(r.shader_set(), ShaderSet::Full);
    r
}

/// Scene `kind` built on `r` and exported as a document at `scale`, and
/// whether its chart uses one of the three features.
fn draw(r: &mut Renderer, kind: SceneKind, scale: f32) -> (RasterImage, bool) {
    let scene = scenes::build(r, kind, BIG_DATA_POINTS)
        .unwrap_or_else(|error| panic!("{}: {error}", kind.label()));
    let chart = Chart::new(scene.config.clone());
    let image = scenes::export_scene(r, &chart, &scene.series, &scene.monotonic, |r, source| {
        r.export_rgba_blocking(source, ExportOptions::document(scale))
    })
    .unwrap_or_else(|error| panic!("{} at {scale}x: {error}", kind.label()));
    (image, uses_a_full_feature(&scene.config))
}

/// The scenes of `kinds` draw the same pixels through the basic modules and
/// through the full ones, at 1x and 2x, and a scene moves a new renderer to
/// the full modules exactly when its chart uses one of the three features.
fn the_variants_draw_the_same(kinds: &[SceneKind]) {
    for &kind in kinds {
        for scale in [1.0f32, 2.0] {
            let mut basic = renderer();
            assert_eq!(basic.shader_set(), ShaderSet::Basic);
            let (first, needs_full) = draw(&mut basic, kind, scale);
            assert_eq!(
                basic.shader_set() == ShaderSet::Full,
                needs_full,
                "{}: the scene's own features select its renderer's set",
                kind.label()
            );
            assert!(
                needs_full || !KNOWN_FULL_SCENES.contains(&kind),
                "{}",
                kind.label()
            );
            let mut full = full_renderer();
            let (second, _) = draw(&mut full, kind, scale);
            assert_eq!(full.shader_set(), ShaderSet::Full);
            assert_eq!(
                (first.width, first.height),
                (second.width, second.height),
                "{}",
                kind.label()
            );
            let differing = first
                .rgba
                .chunks_exact(4)
                .zip(second.rgba.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            if differing > 0 {
                save_pair(kind, scale, &first, &second);
            }
            assert_eq!(
                differing,
                0,
                "{} at {scale}x: {differing} pixels differ between the basic and the full shaders ({})",
                kind.label(),
                difference_summary(&first, &second)
            );
        }
    }
}

/// The Cartesian scenes, in catalogue order, split so that no test runs too
/// long.
fn cartesian_scenes() -> Vec<SceneKind> {
    SceneKind::ALL
        .into_iter()
        .filter(|kind| !kind.is_radial())
        .collect()
}

#[test]
fn the_first_third_of_the_scenes_draws_the_same_through_either_variant() {
    let scenes = cartesian_scenes();
    the_variants_draw_the_same(&scenes[..scenes.len() / 3]);
}

#[test]
fn the_second_third_of_the_scenes_draws_the_same_through_either_variant() {
    let scenes = cartesian_scenes();
    the_variants_draw_the_same(&scenes[scenes.len() / 3..2 * scenes.len() / 3]);
}

#[test]
fn the_last_third_of_the_scenes_draws_the_same_through_either_variant() {
    let scenes = cartesian_scenes();
    the_variants_draw_the_same(&scenes[2 * scenes.len() / 3..]);
}

/// The scenes known to use a feature are catalogue scenes, and most of the
/// catalogue uses none, so both sets draw it.
#[test]
fn the_known_full_scenes_are_catalogue_scenes() {
    let scenes = cartesian_scenes();
    for kind in KNOWN_FULL_SCENES {
        assert!(scenes.contains(kind), "{}", kind.label());
    }
    assert!(scenes.len() > KNOWN_FULL_SCENES.len() + 30);
}

/// How the differing pixels of two same-sized exports differ: the largest
/// per-channel difference, how many pixels differ by at most 1, 8 and 32
/// levels in their worst channel, and the bounding box of the differences.
fn difference_summary(a: &RasterImage, b: &RasterImage) -> String {
    let mut worst = 0u8;
    let mut within = [0usize; 3];
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (index, (pa, pb)) in a
        .rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .enumerate()
    {
        let delta = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        if delta == 0 {
            continue;
        }
        worst = worst.max(delta);
        for (bucket, limit) in within.iter_mut().zip([1u8, 8, 32]) {
            if delta <= limit {
                *bucket += 1;
            }
        }
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        let (x, y) = (index % a.width, index / a.width);
        (min_x, min_y, max_x, max_y) = (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y));
    }
    format!(
        "largest channel difference {worst}; within 1: {}, within 8: {}, within 32: {}; box x {min_x}..={max_x}, y {min_y}..={max_y}",
        within[0], within[1], within[2]
    )
}

/// Save both exports of a scene that differs, next to the test's build
/// output, so a CI run can upload them.
fn save_pair(kind: SceneKind, scale: f32, basic: &RasterImage, full: &RasterImage) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("shader-set-differences");
    if std::fs::create_dir_all(&dir).is_err() {
        // Nothing to save into: the assertion still reports the summary.
        return;
    }
    let stem: String = kind
        .label()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    for (set, image) in [("basic", basic), ("full", full)] {
        let path = dir.join(format!("{stem}-{scale}x-{set}.png"));
        if image.save_png(&path).is_err() {
            // As above: the summary is the report; the image is an aid.
        }
    }
}
