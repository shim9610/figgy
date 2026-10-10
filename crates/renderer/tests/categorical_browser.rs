//! Direct renderer WebGPU tests: no Studio/figgy JS facade involved.
#![cfg(target_arch = "wasm32")]
use renderer::{Color, RendererDevice, categorical::*, radial::*};
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
fn chart() -> CategoricalChart {
    CategoricalChart {
        categories: vec![Category::new("a", "A"), Category::new("b", "B")],
        series: vec![
            BarSeries::new(
                "one",
                "One",
                vec![Some(12.), Some(20.)],
                Color::from_rgb8(220, 30, 50),
            ),
            BarSeries::new(
                "two",
                "Two",
                vec![Some(18.), Some(10.)],
                Color::from_rgb8(30, 80, 220),
            ),
        ],
        labels: CategoryBarLabels::Inside,
        ..Default::default()
    }
}
#[wasm_bindgen_test(async)]
async fn categorical_materials_cache_edits_and_async_png_work_on_webgpu() {
    let (_instance, gpu) = gpu().await;
    scope(gpu.device());
    let mut renderer =
        CategoricalRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut chart = chart();
    let target = CategoryBarTarget::new("a", "one");
    let first = renderer.prepare(&chart, (640, 440), 1.).unwrap();
    let usage = renderer.gpu_memory_usage();
    assert!(Arc::ptr_eq(
        &first,
        &renderer.prepare(&chart, (640, 440), 1.).unwrap()
    ));
    assert_eq!(usage, renderer.gpu_memory_usage());
    let baseline = renderer
        .export_rgba_async(&chart, (640, 440), 1.)
        .await
        .unwrap();
    let mut seen = vec![baseline.rgba.clone()];
    for material in [
        CategoryBarMaterial::Matte,
        CategoryBarMaterial::SatinMetal,
        CategoryBarMaterial::Enamel,
        CategoryBarMaterial::Paper,
    ] {
        chart.style.material = material;
        chart.style.texture_strength = 0.9;
        let frame = renderer.prepare(&chart, (640, 440), 1.).unwrap();
        assert!(first.shares_annotations_with(&frame));
        let image = renderer
            .export_rgba_async(&chart, (640, 440), 1.)
            .await
            .unwrap();
        assert!(
            seen.iter().all(|pixels| pixels != &image.rgba),
            "material {material:?} unchanged"
        );
        seen.push(image.rgba);
        renderer.end_frame();
    }
    chart.selected = Some(target.clone());
    let f = renderer.prepare(&chart, (640, 440), 1.).unwrap();
    assert!(f.shares_annotations_with(&first));
    let r = f.bar_rect(&target).unwrap();
    assert_eq!(
        f.hit_test([(r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5]),
        Some(target.clone())
    );
    let mut edit = CategoryBarOverride::new(target);
    edit.color = Some(Color::from_rgb8(30, 200, 70));
    edit.label_format = Some(CategoryBarLabelFormat::ValuePercent);
    chart.overrides.push(edit);
    let changed = renderer.prepare(&chart, (640, 440), 1.).unwrap();
    assert!(!first.shares_annotations_with(&changed));
    let image = renderer
        .export_rgba_async(&chart, (640, 440), 2.)
        .await
        .unwrap();
    assert_eq!((image.width, image.height), (1280, 880));
    let bytes = renderer::encode_png(&image).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut decoded).unwrap();
    assert_eq!((info.width, info.height), (1280, 880));
    assert_eq!(&decoded[..info.buffer_size()], &image.rgba);
    assert_ne!(image.rgba.iter().filter(|v| **v != 255).count(), 0);
    check_scope(gpu.device()).await;
}
#[wasm_bindgen_test(async)]
async fn categorical_fractional_stack_edges_and_512_entries_work_on_webgpu() {
    let (_instance, gpu) = gpu().await;
    scope(gpu.device());
    let mut renderer =
        CategoricalRenderer::new(gpu.clone(), wgpu::TextureFormat::Bgra8UnormSrgb).unwrap();
    let mut c = chart();
    c.mode = CategoryBarMode::Stacked;
    c.labels = CategoryBarLabels::None;
    c.grid = false;
    c.legend = false;
    c.categories.truncate(1);
    c.series[0].values = vec![Some(1.)];
    c.series[1].values = vec![Some(2.)];
    c.series[0].color = Color::from_rgb8(255, 0, 0);
    c.series[1].color = Color::from_rgb8(0, 0, 255);
    for direction in [
        CategoryBarDirection::Horizontal,
        CategoryBarDirection::Vertical,
    ] {
        c.direction = direction;
        let frame = renderer.prepare(&c, (646, 437), 1.5).unwrap();
        let rect = frame.bar_rect(&CategoryBarTarget::new("a", "one")).unwrap();
        let image = renderer
            .export_rgba_async(&c, (646, 437), 1.5)
            .await
            .unwrap();
        let boundary = if direction == CategoryBarDirection::Horizontal {
            rect[2]
        } else {
            rect[1]
        };
        for offset in -2..=2 {
            let t = (boundary.floor() as i32 + offset) as u32;
            let (x, y) = if direction == CategoryBarDirection::Horizontal {
                (t, ((rect[1] + rect[3]) * 0.5) as u32)
            } else {
                (((rect[0] + rect[2]) * 0.5) as u32, t)
            };
            assert_eq!(
                image.rgba[((y * image.width + x) * 4 + 1) as usize],
                0,
                "stack background seam"
            );
        }
        renderer.end_frame();
    }
    c.categories = (0..64)
        .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
        .collect();
    c.series = (0..8)
        .map(|i| {
            BarSeries::new(
                format!("s{i}"),
                format!("S{i}"),
                vec![Some(1.); 64],
                Color::from_rgb8(100, 140, 210),
            )
        })
        .collect();
    c.direction = CategoryBarDirection::Horizontal;
    c.mode = CategoryBarMode::PercentStacked;
    let frame = renderer.prepare(&c, (640, 1800), 1.).unwrap();
    let image = renderer
        .export_rgba_async(&c, (640, 1800), 1.)
        .await
        .unwrap();
    for cat in &c.categories {
        for series in &c.series {
            let target = CategoryBarTarget::new(&cat.id, &series.id);
            let rect = frame.bar_rect(&target).unwrap();
            let p = [(rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5];
            assert_eq!(frame.hit_test(p), Some(target));
            let index = ((p[1] as u32 * image.width + p[0] as u32) * 4) as usize;
            assert!(image.rgba[index + 2] > 180 && image.rgba[index] < 130);
        }
    }
    check_scope(gpu.device()).await;
}
#[wasm_bindgen_test(async)]
async fn radial_paper_material_and_hover_render_on_webgpu() {
    let (_instance, gpu) = gpu().await;
    scope(gpu.device());
    let mut renderer = RadialRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut c = RadialChart {
        slices: vec![
            RadialSlice::new("A", 40., Color::from_rgb8(50, 140, 220)),
            RadialSlice::new("B", 60., Color::from_rgb8(170, 210, 60)),
        ],
        kind: RadialKind::Donut { inner_radius: 0.5 },
        labels: RadialLabels::Outside,
        ..Default::default()
    };
    c.style.material = RadialMaterial::Paper;
    c.style.tilt_degrees = 55.;
    c.style.depth = 0.2;
    c.style.texture_strength = 0.8;
    let first = renderer.prepare(&c, (640, 440), 1.).unwrap();
    let plain = renderer
        .export_rgba_async(&c, (640, 440), 1.)
        .await
        .unwrap();
    c.interaction.hovered = Some(RadialTarget::main(0));
    c.interaction.hover_progress = 1.;
    let hover = renderer.prepare(&c, (640, 440), 1.).unwrap();
    assert!(first.shares_annotations_with(&hover));
    let raised = renderer
        .export_rgba_async(&c, (640, 440), 1.)
        .await
        .unwrap();
    assert_ne!(plain.rgba, raised.rgba);
    c.interaction = Default::default();
    assert_eq!(
        plain.rgba,
        renderer
            .export_rgba_async(&c, (640, 440), 1.)
            .await
            .unwrap()
            .rgba
    );
    check_scope(gpu.device()).await;
}
