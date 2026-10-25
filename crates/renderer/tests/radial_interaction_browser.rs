//! Real WebGPU round trips through the common renderer interaction API.
#![cfg(target_arch = "wasm32")]
use renderer::{
    Color, DecorationId, RadialChart, RadialFrame, RadialKind, RadialRenderer, RadialSlice,
    RadialTarget, RendererDevice, layout::NudgeResult,
};
use std::sync::Arc;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);
/// Select and drag a decoration of a radial frame, whose decorations place
/// their parts from the chart they edit.
fn edit_radial(frame: &RadialFrame, chart: &mut RadialChart, key: DecorationId) {
    let id = frame.decoration(&key).unwrap();
    let b = frame.decoration_bounds(chart, id).unwrap();
    // An outside label hits on its text and its leader line: press its text.
    let text = frame
        .decoration_selection_box(chart, id)
        .and_then(|s| match s.outline() {
            renderer::SelectionOutline::Parts(parts) => parts.iter().find_map(|p| match p {
                renderer::DrawnPart::Text(r) => Some(*r),
                _ => None,
            }),
            renderer::SelectionOutline::Rect => None,
        })
        .unwrap_or(b);
    assert_eq!(
        frame.decoration_at(
            chart,
            [text.x + text.width * 0.5, text.y + text.height * 0.5]
        ),
        Some(id)
    );
    assert_eq!(
        frame
            .decorations()
            .get(id)
            .unwrap()
            .as_draggable()
            .unwrap()
            .drag_by(chart, &renderer::CpuTextMeasure::default(), 0., 12.),
        NudgeResult::Moved
    );
    chart.decorations.selected = Some(key);
}
#[wasm_bindgen_test(async)]
async fn all_radial_decorations_select_drag_rerender_and_export_on_webgpu() {
    let instance = renderer::data_render::create_instance();
    let adapter = renderer::data_render::request_adapter_async(&instance)
        .await
        .unwrap();
    let (device, queue) = renderer::data_render::request_device_async(&adapter)
        .await
        .unwrap();
    assert!(device.as_webgpu().is_some());
    let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
    let mut r = RadialRenderer::new_async(gpu, wgpu::TextureFormat::Rgba8Unorm)
        .await
        .unwrap();
    let mut c = RadialChart {
        title: "Radial".into(),
        kind: RadialKind::Donut { inner_radius: 0.5 },
        slices: vec![
            RadialSlice::new("High", 35., Color::from_rgb8(110, 90, 170)),
            RadialSlice::new("Low", 65., Color::from_rgb8(75, 155, 170)),
        ],
        ..Default::default()
    };
    c.format_labels();
    let frame = r.prepare(&c, (800, 520), 2.).unwrap();
    let before = r.export_rgba_async(&c, (800, 520), 2.).await.unwrap();
    edit_radial(
        &frame,
        &mut c,
        DecorationId::RadialLabel(RadialTarget::main(0)),
    );
    let after = r.export_rgba_async(&c, (800, 520), 2.).await.unwrap();
    assert_ne!(before.rgba, after.rgba);
    assert!(
        renderer::encode_png(&after)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
    r.end_frame();
}

#[path = "support/radial_export.rs"]
mod radial_export;
#[wasm_bindgen_test(async)]
async fn radial_png_preserves_hover_progress_and_persistent_selection_on_webgpu() {
    let instance = renderer::data_render::create_instance();
    let adapter = renderer::data_render::request_adapter_async(&instance)
        .await
        .unwrap();
    let (device, queue) = renderer::data_render::request_device_async(&adapter)
        .await
        .unwrap();
    assert!(device.as_webgpu().is_some());
    radial_export::check(RendererDevice::new(Arc::new(device), Arc::new(queue))).await;
}
