//! Area editing has a separate browser binary so shader startup and pixel
//! checks retain the existing per-suite timeout as coverage grows.
#![cfg(target_arch = "wasm32")]
use renderer::RendererDevice;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);
#[path = "support/radial_area.rs"]
mod radial_area;
#[wasm_bindgen_test(async)]
async fn radial_area_select_resize_move_pick_and_export_on_webgpu() {
    let instance = renderer::gpu::create_instance();
    let adapter = renderer::gpu::request_adapter_async(&instance)
        .await
        .unwrap();
    let (device, queue) = renderer::gpu::request_device_async(&adapter).await.unwrap();
    assert!(device.as_webgpu().is_some());
    radial_area::check(RendererDevice::new(device, queue).expect("figgy poll thread")).await;
}
