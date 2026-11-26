//! The pipeline cache a native host saves and loads (#478), through the
//! public API: a renderer built with no cache, an empty one, a saved one and
//! bytes that are not a cache exports the same picture at 1x and 2x on the
//! catalogue scenes, and a saved cache comes back from
//! `RendererDevice::pipeline_cache_data`. The cache changes how fast a
//! pipeline is created, never what it draws.
//!
//! The device is opened by figgy's own `request_device_async`, which asks
//! for `Features::PIPELINE_CACHE` where the adapter has it (wgpu: Vulkan).
//! Elsewhere there is no cache and every variant is the no-cache renderer.
#![cfg(not(target_arch = "wasm32"))]

use test_gpu::BlockOn as _;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/playground_scenes.rs"]
mod scenes;
#[path = "support/gpu.rs"]
mod test_gpu;

use renderer::{Chart, ExportOptions, RasterImage, Renderer, RendererDevice};
use scenes::SceneKind;

const POOL_BYTES: u64 = 64 * 1024 * 1024;

/// Scenes that between them create the field, contour, contour label, polar,
/// probability, broken-axis, line, scatter, histogram, vector, triangle mesh
/// and 3D surface pipelines.
const SCENES: [SceneKind; 10] = [
    SceneKind::Lines,
    SceneKind::Scatter,
    SceneKind::Histogram,
    SceneKind::Field,
    SceneKind::PolarHeatmap,
    SceneKind::VectorField,
    SceneKind::Surface,
    SceneKind::TriMesh,
    SceneKind::ProbabilityPaper,
    SceneKind::BrokenAxes,
];

/// A bundle on a device figgy opened itself.
fn bundle() -> RendererDevice {
    let instance = test_gpu::instance();
    let adapter = test_gpu::adapter(&instance);
    let (device, queue) = renderer::gpu::request_device_async(&adapter)
        .block_on()
        .expect("required GPU test: device initialization failed");
    RendererDevice::new(device, queue).expect("figgy poll thread")
}

/// Every scene exported at 1x and 2x by a renderer built from `gpu`.
fn exports(gpu: RendererDevice) -> Vec<(SceneKind, f32, RasterImage)> {
    let mut r = Renderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm, POOL_BYTES)
        .expect("required GPU test: renderer initialization failed");
    let mut out = Vec::new();
    for kind in SCENES {
        let scene = scenes::build(&mut r, kind, 0).expect("scene builds");
        let chart = Chart::new(scene.config.clone());
        for scale in [1.0f32, 2.0] {
            let image = scenes::export_scene(
                &mut r,
                &chart,
                &scene.series,
                &scene.monotonic,
                |r, source| {
                    r.export_rgba(source, ExportOptions::panel(scale))
                        .block_on()
                },
            )
            .expect("export");
            out.push((kind, scale, image));
        }
    }
    out
}

fn assert_same(
    what: &str,
    got: &[(SceneKind, f32, RasterImage)],
    want: &[(SceneKind, f32, RasterImage)],
) {
    assert_eq!(got.len(), want.len());
    for ((kind, scale, a), (_, _, b)) in got.iter().zip(want) {
        assert_eq!(
            (a.width, a.height),
            (b.width, b.height),
            "{what}: {kind:?} {scale}x"
        );
        assert!(a.rgba == b.rgba, "{what}: {kind:?} at {scale}x differs");
    }
}

#[test]
fn a_saved_pipeline_cache_draws_the_same_pictures() {
    let gpu = bundle();
    let cached = gpu.pipeline_cache_data().is_some();
    let without = exports(gpu.clone().without_pipeline_cache());

    // An empty cache, then the one it filled, saved and loaded again.
    let first = exports(gpu.clone());
    assert_same("an empty cache", &first, &without);
    let saved = gpu.pipeline_cache_data();
    assert_eq!(saved.is_some(), cached);
    if let Some(saved) = &saved {
        assert!(!saved.is_empty(), "a filled cache has data");
    }
    let loaded = exports(gpu.clone().with_pipeline_cache(saved.as_deref()));
    assert_same("a saved cache", &loaded, &without);

    // Bytes that are not a cache: discarded, the cache starts empty.
    let garbage = exports(
        gpu.clone()
            .with_pipeline_cache(Some(b"not a pipeline cache")),
    );
    assert_same("unusable cache data", &garbage, &without);
    let truncated = saved.as_deref().map(|bytes| &bytes[..bytes.len() / 2]);
    let truncated = exports(gpu.with_pipeline_cache(truncated));
    assert_same("a truncated cache", &truncated, &without);
}

/// `pipeline_cache_key` names a Vulkan adapter's cache file and has no name
/// for a backend without pipeline caches.
#[test]
fn the_pipeline_cache_key_follows_the_backend() {
    let instance = test_gpu::instance();
    let info = test_gpu::adapter(&instance).get_info();
    let key = RendererDevice::pipeline_cache_key(&info);
    assert_eq!(
        key.is_some(),
        info.backend == wgpu::Backend::Vulkan,
        "{info:?}"
    );
    if let Some(key) = key {
        assert!(!key.is_empty() && !key.contains('/'), "{key}");
    }
}
