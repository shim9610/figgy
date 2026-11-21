//! The two shader sets draw the same chart (#299).
//!
//! A renderer compiles the basic data modules until it is first handed a
//! chart with an axis break, a probability axis or polar coordinates, and
//! the full modules from then on (`shader_assembly::ShaderSet`). A chart
//! with none of the three may therefore be drawn by either, depending on
//! what its renderer was handed before. This suite draws every Cartesian
//! scene of the playground catalogue (`support/playground_scenes.rs`)
//! through both and holds the exports to the same pixels
//! ([`same_picture`]), so no fragment of one set can come to place, shade
//! or clip a plain chart differently from its twin. The scenes that use one
//! of the three features move their renderer to the full modules
//! themselves, and only those do; the suite holds the renderer's choice to
//! its own reading of each scene's model.
//!
//! The two sets are two shader programs, and a shader compiler may round
//! the same arithmetic differently in each. WGSL lets an implementation
//! reassociate floating-point operations, and DXC compiles with fast-math
//! flags: on #304 the DXIL of the arrow corner computation in
//! `vector_points` and `vector_points_3d` is `(a + b) - c` in the basic set
//! and `(a - c) + b` in the full one. `@invariant` cannot pin this, as it
//! applies to the position output only and only between programs with the
//! same control flow. Measured on #155 and #301
//! (DX12 with WARP and DXC, and Metal), a scene drawn twice through one set
//! is identical, while the two sets differ in a few edge pixels: by one
//! level in one channel (the quiver: 82 pixels at 1x, 262 at 2x; the
//! streamlines: 17 and 52; a few in the line, annotation and large-data
//! scenes), and in arrowhead tip pixels of the 3D vector field by up to 51
//! levels of alpha (152 against 203 at 1x; 0 against 6 at 2x), partly
//! covered pixels. Vulkan draws
//! both sets identically. A misplaced, recoloured or clipped element changes
//! fully covered or empty pixels by far more than one level, which the
//! comparison still refuses.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;
use harness::headless_renderer as renderer;
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
                .filter(|(a, b)| !same_picture(a, b))
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

/// Whether two exports show the same pixel. The colours are compared
/// premultiplied by their alpha, so an empty pixel's colour channels, which
/// show nothing, take no part. A pixel is the same when every premultiplied
/// channel is within one level (a shader compiler's rounding), or when
/// either side is partly covered (an alpha strictly between 0 and 255):
/// there an edge's coverage may round differently between the two
/// programs, down to empty or up to fully covered. A pixel that is fully
/// covered or empty in both exports and differs by more than one level is
/// refused. See the module documentation for the measured differences.
fn same_picture(a: &[u8], b: &[u8]) -> bool {
    let premultiplied = |pixel: &[u8], channel: usize| -> u16 {
        let alpha = u16::from(pixel.get(3).copied().unwrap_or(255));
        let value = u16::from(pixel.get(channel).copied().unwrap_or(0));
        if channel == 3 {
            value
        } else {
            (value * alpha + 127) / 255
        }
    };
    let within_one = (0..a.len().min(b.len()))
        .all(|channel| premultiplied(a, channel).abs_diff(premultiplied(b, channel)) <= 1);
    let partly_covered = |pixel: &[u8]| pixel.get(3).is_some_and(|alpha| (1..=254).contains(alpha));
    within_one || partly_covered(a) || partly_covered(b)
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

/// The comparison allows a compiler's rounding and nothing a misplaced,
/// recoloured or clipped element produces.
#[test]
fn the_comparison_refuses_what_a_real_difference_changes() {
    // Rounding: one level in a channel, anywhere.
    assert!(same_picture(&[10, 20, 30, 255], &[11, 20, 30, 255]));
    assert!(same_picture(&[0, 0, 0, 0], &[0, 0, 0, 1]));
    // An empty pixel's colour shows nothing: only its alpha counts.
    assert!(same_picture(&[0, 0, 0, 0], &[191, 221, 54, 0]));
    // An edge pixel covered differently in both exports, or rounded to
    // empty or to fully covered in one of them (DX12: alpha 0 against 6 at
    // a 3D arrowhead at 2x).
    assert!(same_picture(&[53, 183, 121, 152], &[53, 183, 121, 203]));
    assert!(same_picture(&[0, 0, 0, 0], &[191, 221, 54, 6]));
    assert!(same_picture(&[53, 183, 121, 255], &[53, 183, 121, 250]));
    // A pixel an element moved onto or off: covered against empty.
    assert!(!same_picture(&[53, 183, 121, 255], &[0, 0, 0, 0]));
    assert!(!same_picture(&[53, 183, 121, 255], &[0, 0, 0, 255]));
    // A recoloured, fully covered pixel.
    assert!(!same_picture(&[10, 20, 30, 255], &[12, 20, 30, 255]));
}
