//! Area editing has a separate browser binary so shader startup and pixel
//! checks retain the existing per-suite timeout as coverage grows.
#![cfg(target_arch = "wasm32")]
use renderer::RendererDevice;
use std::sync::Arc;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);
#[path = "support/bounded_area.rs"]
mod bounded_area;
#[wasm_bindgen_test(async)]
async fn bounded_area_select_resize_move_pick_and_export_on_webgpu() {
    let instance = renderer::data_render::create_instance();
    let adapter = renderer::data_render::request_adapter_async(&instance)
        .await
        .unwrap();
    let (device, queue) = renderer::data_render::request_device_async(&adapter)
        .await
        .unwrap();
    assert!(device.as_webgpu().is_some());
    bounded_area::check(RendererDevice::new(Arc::new(device), Arc::new(queue))).await;
}
