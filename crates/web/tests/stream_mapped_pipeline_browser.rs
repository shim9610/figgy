#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use renderer::{
    Chart, Color, DataRenderType, RegisteredChartDrawItem, Renderer, RendererDevice, SeriesConfig,
    data::Column,
    data_config::{
        DataErrorBarPointStyleConfig, DataErrorBarPointStyleOverride, DataErrorBarStyleConfig,
        DataScatterPointStyleConfig, DataScatterPointStyleOverride, DataScatterStyleConfig,
        ErrorRef, ScatterShape,
    },
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
async fn dawn_accepts_mapped_scatter_and_errorbar_entries() {
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
    for (id, values) in [
        ("x", vec![0.2, 0.4, 0.6, 0.8]),
        ("y", vec![0.3, 0.7, 0.5, 0.2]),
        ("e", vec![0.1; 4]),
        ("scatter-index", vec![0.0, 1.0, 0.0, 1.0]),
        ("error-index", vec![1.0, 0.0, 1.0, 0.0]),
    ] {
        renderer
            .add_column(
                id,
                &Column {
                    data: values,
                    min: 0.0,
                    max: 1.0,
                },
            )
            .expect("browser column upload failed");
    }
    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let series = SeriesConfig {
        series_id: "browser-mapped".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::ScatterErrorbarX {
            scatter: DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_shape: ScatterShape::CircleFilled,
                point_size: 7.0,
                point_style_table: Some(vec![DataScatterPointStyleConfig::default()]),
                point_style_index_column: Some("scatter-index".into()),
                point_style_overrides: Some(vec![DataScatterPointStyleOverride {
                    index: 2,
                    style: DataScatterPointStyleConfig::default(),
                }]),
            },
            err_x: ErrorRef::Symmetric { column: "e".into() },
            err_style: DataErrorBarStyleConfig {
                error_bar_color: Color::BLACK,
                error_bar_width: 2.0,
                error_bar_cap_size: 5.0,
                cap_width: 2.0,
                error_bar_style_table: Some(vec![DataErrorBarPointStyleConfig::default()]),
                error_bar_style_index_column: Some("error-index".into()),
                error_bar_style_overrides: Some(vec![DataErrorBarPointStyleOverride {
                    index: 2,
                    style: DataErrorBarPointStyleConfig::default(),
                }]),
            },
        },
    };
    let chart_id = renderer
        .register_chart(config.clone(), vec![series])
        .expect("mapped chart registration failed");
    let view = renderer
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
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
        .expect("mapped chart preparation failed");
    let error = validation_error(&device).await;
    assert!(
        error.is_none(),
        "Chrome/Dawn rejected mapped pipelines: {error:?}"
    );
}
