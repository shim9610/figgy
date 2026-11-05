//! CI probe only (not for merge): which pipeline creation overflows a
//! thread's stack on the DX12 job, and how much stack each needs.
#[path = "support/gpu.rs"]
mod test_gpu;

use renderer::{InitEvent, Renderer, RendererDevice};
use std::io::Write as _;

#[test]
fn probe() {
    let kb: usize = std::env::var("PROBE_STACK_KB")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2048);
    let handle = std::thread::Builder::new()
        .stack_size(kb * 1024)
        .spawn(move || {
            let (device, queue) = test_gpu::device();
            let mut observer = |e: InitEvent| {
                eprintln!("PROBE {kb}K {} {} {:?}", e.scope, e.stage, e.phase);
                let _ = std::io::stderr().flush();
            };
            let mut r = Renderer::try_new_observed(
                RendererDevice::new(device, queue).expect("device"),
                wgpu::TextureFormat::Bgra8Unorm,
                8 * 1024 * 1024,
                &mut observer,
            )
            .expect("renderer init");
            eprintln!("PROBE {kb}K renderer ready");
            if std::env::var("PROBE_PREWARM").as_deref() == Ok("1") {
                r.prewarm_all_observed_blocking(&mut observer)
                    .expect("prewarm");
                eprintln!("PROBE {kb}K prewarm ready");
            }
            r.enable_gpu_picking_observed(&mut observer)
                .expect("GPU picking");
            eprintln!("PROBE {kb}K picking ready");
        })
        .expect("spawn");
    handle.join().expect("probe thread");
}
