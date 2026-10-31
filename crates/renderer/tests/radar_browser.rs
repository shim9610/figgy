//! Direct renderer WebGPU tests: no Studio/figgy JS facade involved.
#![cfg(target_arch = "wasm32")]
use renderer::{Category, Color, RendererDevice, radar::*};
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
fn chart() -> RadarChart {
    RadarChart {
        categories: vec![
            Category::new("a", "A"),
            Category::new("b", "B"),
            Category::new("c", "C"),
            Category::new("d", "D"),
        ],
        series: vec![RadarSeries::new(
            "s",
            "S",
            vec![60., 80., 50., 70.],
            Color::from_rgb8(30, 110, 210),
        )],
        value_range: Some([0., 100.]),
        grid: false,
        legend: false,
        ..Default::default()
    }
}
#[wasm_bindgen_test(async)]
async fn radar_markers_emphasis_selection_cache_and_png_work_on_webgpu() {
    let (_instance, gpu) = gpu().await;
    scope(gpu.device());
    let mut r = RadarRenderer::new_async(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)
        .await
        .unwrap();
    let mut c = chart();
    let f = r.prepare(&c, (640, 440), 1.).unwrap();
    let before = r.export_rgba_async(&c, (640, 440), 1.).await.unwrap();
    let t = RadarTarget::vertex("s", "a");
    let marker = f.vertex_position(&t).unwrap();
    let (center, _) = f.plot_circle();
    let clip = f.plot_rect();
    assert!(f.hit_test([clip[0] + 1., clip[1] + 1.]).is_none());
    assert_eq!(
        f.hit_test(marker).unwrap(),
        RadarPick {
            target: t.clone(),
            part: RadarPart::Marker
        }
    );
    assert_eq!(
        f.hit_test(center).unwrap(),
        RadarPick {
            target: RadarTarget::series("s"),
            part: RadarPart::Fill
        }
    );
    c.style.fill_opacity = 0.6;
    c.hovered = Some(t);
    let changed = r.prepare(&c, (640, 440), 1.).unwrap();
    assert!(f.shares_annotations_with(&changed));
    let after = r.export_rgba_async(&c, (640, 440), 1.).await.unwrap();
    assert_ne!(before.rgba, after.rgba);
    assert!(Arc::ptr_eq(
        &changed,
        &r.prepare(&c, (640, 440), 1.).unwrap()
    ));
    c.style.grid_shape = RadarGridShape::Circle;
    c.grid = true;
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
async fn radar_srgb_series_styles_gaps_and_invalid_edits_work_on_webgpu() {
    let (_instance, gpu) = gpu().await;
    scope(gpu.device());
    let mut r = RadarRenderer::new_async(gpu.clone(), wgpu::TextureFormat::Bgra8UnormSrgb)
        .await
        .unwrap();
    let mut c = chart();
    c.series[0].style = Some(RadarSeriesStyle {
        fill: Some(Color::from_rgb8(20, 200, 70)),
        marker: Some(RadarMarker::Square),
        ..Default::default()
    });
    c.series[0].values[1] = f64::NAN;
    let f = r.prepare(&c, (640, 440), 1.5).unwrap();
    assert!(f.vertex_position(&RadarTarget::vertex("s", "b")).is_none());
    assert!(
        f.part_rect(&RadarPick {
            target: RadarTarget::series("s"),
            part: RadarPart::Fill
        })
        .is_none(),
        "a gap drops the series fill"
    );
    assert!(
        f.part_rect(&RadarPick {
            target: RadarTarget::vertex("s", "a"),
            part: RadarPart::Marker
        })
        .is_some()
    );
    c.series[0].values[1] = 80.;
    let f = r.prepare(&c, (640, 440), 1.5).unwrap();
    assert!(f.vertex_position(&RadarTarget::vertex("s", "b")).is_some());
    let image = r.export_rgba_async(&c, (640, 440), 1.5).await.unwrap();
    assert!(
        image
            .rgba
            .chunks_exact(4)
            .any(|p| p[1] > 150 && p[0] < 70 && p[2] < 110)
    );
    let old = r.gpu_memory_usage();
    c.value_range = Some([1., 1.]);
    assert!(r.prepare(&c, (640, 440), 1.5).is_err());
    assert_eq!(old, r.gpu_memory_usage());
    check_scope(gpu.device()).await;
}
