//! Real WebGPU round trips through the common renderer interaction API.
#![cfg(target_arch = "wasm32")]
use renderer::{
    BarSeries, BoxPlotChart, BoxPlotRenderer, BoxPlotSeries, BoxSummary, CategoricalChart,
    CategoricalRenderer, Category, Color, DecorationId, DecorationLayout, RadarChart,
    RadarRenderer, RadarSeries, RadialChart, RadialKind, RadialRenderer, RadialSlice, RadialTarget,
    RendererDevice, decoration::DecorationMap, layout::NudgeResult,
};
use std::sync::Arc;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);
fn edit(map: &DecorationMap, state: &mut DecorationLayout, key: DecorationId) {
    let (id, element) = map.entries().find(|(_, e)| e.id == key).unwrap();
    let b = map.bounds(id).unwrap();
    assert_eq!(
        map.hit_test([b.x + b.width * 0.5, b.y + b.height * 0.5]),
        Some(id)
    );
    assert_eq!(
        map.hit_map()
            .get(id)
            .unwrap()
            .as_draggable()
            .unwrap()
            .drag_by(
                state,
                // The measure the decoration map lays its elements out with.
                &renderer::CpuTextMeasure::for_style(&Default::default()),
                0.,
                12.
            ),
        NudgeResult::Moved
    );
    state.selected = Some(element.id.clone());
}
#[wasm_bindgen_test(async)]
async fn all_bounded_decorations_select_drag_rerender_and_export_on_webgpu() {
    let instance = renderer::data_render::create_instance();
    let adapter = renderer::data_render::request_adapter_async(&instance)
        .await
        .unwrap();
    let (device, queue) = renderer::data_render::request_device_async(&adapter)
        .await
        .unwrap();
    assert!(device.as_webgpu().is_some());
    let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
    let mut r = RadialRenderer::new_async(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)
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
    edit(
        frame.decorations(),
        &mut c.decorations,
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
    let mut r = CategoricalRenderer::new_async(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)
        .await
        .unwrap();
    let mut c = CategoricalChart {
        title: "Bars".into(),
        categories: vec![Category::new("a", "Control")],
        series: vec![BarSeries::new(
            "s",
            "Distribution",
            vec![Some(35.)],
            Color::from_rgb8(110, 90, 170),
        )],
        ..Default::default()
    };
    let frame = r.prepare(&c, (800, 520), 1.).unwrap();
    let before = r.export_rgba_async(&c, (800, 520), 1.).await.unwrap();
    edit(
        frame.decorations(),
        &mut c.decorations,
        DecorationId::ChartTitle,
    );
    assert_ne!(
        before.rgba,
        r.export_rgba_async(&c, (800, 520), 1.).await.unwrap().rgba
    );
    r.end_frame();
    let mut r = BoxPlotRenderer::new_async(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)
        .await
        .unwrap();
    let mut c = BoxPlotChart {
        title: "Boxes".into(),
        categories: vec![Category::new("a", "Control")],
        series: vec![BoxPlotSeries::new(
            "s",
            "Distribution",
            vec![Some(BoxSummary::new(10., 25., 38., 52., 70.))],
            Color::from_rgb8(110, 90, 170),
        )],
        ..Default::default()
    };
    let frame = r.prepare(&c, (800, 520), 2.).unwrap();
    let before = r.export_rgba_async(&c, (800, 520), 2.).await.unwrap();
    edit(
        frame.decorations(),
        &mut c.decorations,
        DecorationId::ChartTitle,
    );
    assert_ne!(
        before.rgba,
        r.export_rgba_async(&c, (800, 520), 2.).await.unwrap().rgba
    );
    r.end_frame();
    let mut r = RadarRenderer::new_async(gpu, wgpu::TextureFormat::Rgba8Unorm)
        .await
        .unwrap();
    let mut c = RadarChart {
        title: "Spokes".into(),
        categories: vec![
            Category::new("a", "Control"),
            Category::new("b", "Treated"),
            Category::new("c", "Placebo"),
        ],
        series: vec![RadarSeries::new(
            "s",
            "Distribution",
            vec![35., 52., 70.],
            Color::from_rgb8(110, 90, 170),
        )],
        ..Default::default()
    };
    let frame = r.prepare(&c, (800, 520), 2.).unwrap();
    let before = r.export_rgba_async(&c, (800, 520), 2.).await.unwrap();
    edit(
        frame.decorations(),
        &mut c.decorations,
        DecorationId::SpokeLabel {
            category_id: "a".into(),
        },
    );
    assert_ne!(
        before.rgba,
        r.export_rgba_async(&c, (800, 520), 2.).await.unwrap().rgba
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
