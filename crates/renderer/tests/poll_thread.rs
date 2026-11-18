//! The native poll thread's lifetime, counted from the OS (#160): one thread
//! per bundle built with `RendererDevice::new`, none for
//! `RendererDevice::blocking`, shared by the bundle's clones and renderers,
//! and gone once they are dropped. A test binary of
//! its own, so no other test's bundle is counted; Linux only, where
//! `/proc/self/task` names every thread.

#![cfg(target_os = "linux")]

#[path = "support/gpu.rs"]
mod test_gpu;

use std::time::{Duration, Instant};

use renderer::{Renderer, RendererDevice};

/// The process's threads named `figgy-gpu-poll`.
fn poll_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .expect("procfs")
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .filter(|name| name.trim_end() == "figgy-gpu-poll")
        .count()
}

/// Wait until `expected` poll threads run; a thread exits on its own, just
/// after its last owner is dropped.
fn settle_at(expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while poll_threads() != expected {
        assert!(
            Instant::now() < deadline,
            "{} poll threads, expected {expected}",
            poll_threads()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_bundle_owns_one_poll_thread_until_every_owner_is_gone() {
    let (device, queue) = test_gpu::device();
    assert_eq!(poll_threads(), 0);

    // A blocking-only bundle starts none.
    let blocking = RendererDevice::blocking(device.clone(), queue.clone());
    assert_eq!(poll_threads(), 0);
    drop(blocking);

    // One per bundle, shared by its clones and the renderers built from it.
    let gpu = RendererDevice::new(device.clone(), queue.clone()).expect("figgy poll thread");
    settle_at(1);
    let renderer =
        Renderer::try_new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm, 1 << 20).expect("renderer");
    let second = RendererDevice::new(device.clone(), queue.clone()).expect("figgy poll thread");
    settle_at(2);
    drop(second);
    settle_at(1);

    // The renderer keeps the thread after the bundle itself drops. (A
    // pending ticket keeps it too: `a_ticket_outliving_its_renderer_and_
    // bundle_is_served` in the renderer's tests.)
    drop(gpu);
    settle_at(1);
    drop(renderer);
    settle_at(0);
}
