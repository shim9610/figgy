#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use renderer::{
    Chart, Color, DataRenderType, RegisteredChartDrawItem, Renderer, RendererDevice, SeriesConfig,
    data::Column,
    data_config::{BarOrientation, DataBarBinStyleConfig, DataBarStyleConfig, DataBarStyleOverride},
    data_render::{create_instance, request_adapter_async, request_device_async},
    default,
    layout::{ChartArea, Rect},
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn gpu_method(device: &wgpu::webgpu::GpuDevice, name: &str) -> js_sys::Function {
    js_sys::Reflect::get(device.as_ref(), &JsValue::from_str(name))
        .expect("GPUDevice method lookup failed")
        .dyn_into()
        .expect("GPUDevice method is not a function")
}

async fn validation_error(device: &wgpu::Device) -> Option<String> {
    let raw = device.as_webgpu().expect("browser WebGPU backend required");
    let promise = gpu_method(raw, "popErrorScope")
        .call0(raw.as_ref())
        .expect("popErrorScope failed")
        .unchecked_into::<js_sys::Promise>();
    let error = JsFuture::from(promise).await.expect("error scope rejected");
    if error.is_null() || error.is_undefined() {
        None
    } else {
        Some(
            js_sys::Reflect::get(&error, &JsValue::from_str("message"))
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_else(|| format!("{error:?}")),
        )
    }
}

#[wasm_bindgen_test(async)]
async fn dawn_accepts_stream_histogram_shader_entries_on_resident_prepare() {
    let instance = create_instance();
    let adapter = request_adapter_async(&instance)
        .await
        .expect("Chrome WebGPU adapter required");
    let (device, queue) = request_device_async(&adapter)
        .await
        .expect("Chrome WebGPU device required");
    let device = Arc::new(device);
    let queue = Arc::new(queue);
    let mut renderer = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4096,
    )
    .expect("renderer creation failed");
    renderer
        .add_column(
            "browser-edges",
            &Column {
                data: (0..17).map(|i| i as f32 * 0.25).collect(),
                min: 0.0,
                max: 4.0,
            },
        )
        .expect("edge upload failed");
    renderer
        .add_column(
            "browser-values",
            &Column {
                data: vec![2.0; 16],
                min: 2.0,
                max: 2.0,
            },
        )
        .expect("value upload failed");
    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let series = SeriesConfig {
        series_id: "browser-histogram".into(),
        source_id: None,
        label: None,
        x_column: "browser-edges".into(),
        y_column: "browser-values".into(),
        render_type: DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: Color::new(0.0, 0.6, 0.0, 1.0),
                border_color: Color::BLACK,
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 0.0,
                width_ratio: 1.0,
                orientation: BarOrientation::Vertical,
                bar_style_overrides: Some(vec![DataBarStyleOverride {
                    index: 9,
                    style: DataBarBinStyleConfig {
                        fill_color: Some(Color::new(1.0, 0.0, 0.0, 1.0)),
                        ..Default::default()
                    },
                }]),
            },
        },
    };
    let chart_id = renderer
        .register_chart(config.clone(), vec![series])
        .expect("histogram registration failed");
    let chart = Chart::new(config.clone());
    let view = renderer
        .create_chart_view(&chart, config.chart_area.0)
        .expect("chart view creation failed");
    let raw = device.as_webgpu().expect("browser WebGPU backend required");
    gpu_method(raw, "pushErrorScope")
        .call1(raw.as_ref(), &JsValue::from_str("validation"))
        .expect("pushErrorScope failed");
    renderer
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id,
            view: &view,
        }])
        .expect("histogram preparation failed");
    let error = validation_error(&device).await;
    assert!(error.is_none(), "Chrome/Dawn rejected histogram pipelines: {error:?}");
}
