//! Direct renderer WebGPU tests: no Studio/figgy JS facade involved.
#![cfg(target_arch = "wasm32")]
use renderer::{Category, Color, RendererDevice, boxplot::*};
use std::sync::Arc;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);
async fn gpu() -> (wgpu::Instance, RendererDevice) {
    let instance = renderer::data_render::create_instance();
    let adapter = renderer::data_render::request_adapter_async(&instance)
        .await
        .expect("WebGPU adapter required");
    let (device, queue) = renderer::data_render::request_device_async(&adapter)
        .await
        .unwrap();
    assert!(device.as_webgpu().is_some(), "test must use browser WebGPU");
    (
        instance,
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
    )
}
fn scope(device: &wgpu::Device) {
    let raw = device.as_webgpu().unwrap();
    let method = js_sys::Reflect::get(raw.as_ref(), &JsValue::from_str("pushErrorScope"))
        .unwrap()
        .dyn_into::<js_sys::Function>()
        .unwrap();
    method
        .call1(raw.as_ref(), &JsValue::from_str("validation"))
        .unwrap();
}
async fn check_scope(device: &wgpu::Device) {
    let raw = device.as_webgpu().unwrap();
    let method = js_sys::Reflect::get(raw.as_ref(), &JsValue::from_str("popErrorScope"))
        .unwrap()
        .dyn_into::<js_sys::Function>()
        .unwrap();
    let promise = method
        .call0(raw.as_ref())
        .unwrap()
        .dyn_into::<js_sys::Promise>()
        .unwrap();
    let error = JsFuture::from(promise).await.unwrap();
    assert!(
        error.is_null() || error.is_undefined(),
        "WebGPU validation error: {error:?}"
    );
}
fn chart() -> BoxPlotChart {
    let mut v = BoxSummary::new(10., 30., 40., 60., 80.);
    v.outliers = vec![95.];
    v.median_ci = Some([35., 45.]);
    v.mean = Some(50.);
    BoxPlotChart {
        categories: vec![Category::new("a", "A"), Category::new("b", "B")],
        series: vec![BoxPlotSeries::new(
            "s",
            "S",
            vec![Some(v.clone()), Some(v)],
            Color::from_rgb8(30, 110, 210),
        )],
        value_range: Some([0., 100.]),
        grid: false,
        legend: false,
        ..Default::default()
    }
}
#[wasm_bindgen_test(async)]
async fn boxplot_notches_materials_selection_cache_and_png_work_on_webgpu() {
    let (_instance, gpu) = gpu().await;
    scope(gpu.device());
    let mut r = BoxPlotRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut c = chart();
    c.style.notched = true;
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let before = r.export_rgba_async(&c, (640, 440), 1.).await.unwrap();
    let t = BoxPlotTarget::new("a", "s");
    let b = f.box_rect(&t).unwrap();
    let med_y = f.plot_rect()[3] - 0.4 * (f.plot_rect()[3] - f.plot_rect()[1]);
    assert!(f.hit_test([b[0] + 1., med_y]).is_none());
    assert_eq!(
        f.hit_test([(b[0] + b[2]) * 0.5, med_y]).unwrap().part,
        BoxPlotPart::Median
    );
    c.style.material = BoxPlotMaterial::SatinMetal;
    c.style.show_mean = true;
    c.hovered = Some(t);
    let changed = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(f.shares_annotations_with(&changed));
    let after = r.export_rgba_async(&c, (640, 440), 1.).await.unwrap();
    assert_ne!(before.rgba, after.rgba);
    assert!(Arc::ptr_eq(
        &changed,
        &r.prepare(&c, (640, 440), 1.).unwrap()
    ));
    c.direction = BoxPlotDirection::Horizontal;
    let image = r.export_rgba_async(&c, (640, 440), 2.).await.unwrap();
    assert_eq!((image.width, image.height), (1280, 880));
    let mut reader = png::Decoder::new(std::io::Cursor::new(renderer::encode_png(&image).unwrap()))
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(&bytes[..info.buffer_size()], &image.rgba);
    assert!(image.rgba.chunks_exact(4).any(|p| p[2] > 150 && p[0] < 100));
    check_scope(gpu.device()).await;
}
#[wasm_bindgen_test(async)]
async fn boxplot_srgb_clipping_individual_edits_and_degenerate_statistics_work_on_webgpu() {
    let (_instance, gpu) = gpu().await;
    scope(gpu.device());
    let mut r = BoxPlotRenderer::new(gpu.clone(), wgpu::TextureFormat::Bgra8UnormSrgb).unwrap();
    let mut c = chart();
    c.direction = BoxPlotDirection::Horizontal;
    let t = BoxPlotTarget::new("a", "s");
    let mut edit = BoxPlotOverride::new(t.clone());
    edit.color = Some(Color::from_rgb8(20, 200, 70));
    c.overrides.push(edit);
    c.series[0].values[1] = Some(BoxSummary::new(42., 42., 42., 42., 42.));
    c.value_range = Some([35., 55.]);
    let f = r.prepare(&c, (640, 440), 1.5).unwrap();
    assert!(
        f.part_rect(&BoxPlotPick {
            target: t.clone(),
            part: BoxPlotPart::Outlier(0)
        })
        .is_none()
    );
    let image = r.export_rgba_async(&c, (640, 440), 1.5).await.unwrap();
    assert!(
        image
            .rgba
            .chunks_exact(4)
            .any(|p| p[1] > 150 && p[0] < 70 && p[2] < 110)
    );
    let old = r.gpu_memory_usage();
    c.series[0].values[0].as_mut().unwrap().median = f64::NAN;
    assert!(r.prepare(&c, (640, 440), 1.5).is_err());
    assert_eq!(old, r.gpu_memory_usage());
    check_scope(gpu.device()).await;
}
