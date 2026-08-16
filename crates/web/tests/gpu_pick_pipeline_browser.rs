#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use renderer::{
    Color, DataRenderType, DataScatterStyleConfig, GpuPickRequest, Renderer, RendererDevice,
    ScatterShape, SeriesConfig,
    config::DrawStyle,
    data::Column,
    data_render::{create_instance, request_adapter_async, request_device_async},
    default,
    layout::{ChartArea, Rect},
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn raw_device_method(device: &wgpu::webgpu::GpuDevice, name: &str) -> js_sys::Function {
    js_sys::Reflect::get(device.as_ref(), &JsValue::from_str(name))
        .expect("GPUDevice method lookup failed")
        .dyn_into()
        .expect("GPUDevice property was not a function")
}

fn push_validation_scope(device: &wgpu::Device) {
    let raw = device
        .as_webgpu()
        .expect("browser test requires the WebGPU backend");
    raw_device_method(raw, "pushErrorScope")
        .call1(raw.as_ref(), &JsValue::from_str("validation"))
        .expect("GPUDevice.pushErrorScope failed");
}

async fn pop_error_scope(device: &wgpu::Device) -> Option<String> {
    let raw = device
        .as_webgpu()
        .expect("browser test requires the WebGPU backend");
    let promise = raw_device_method(raw, "popErrorScope")
        .call0(raw.as_ref())
        .expect("GPUDevice.popErrorScope failed")
        .unchecked_into::<js_sys::Promise>();
    let error = JsFuture::from(promise)
        .await
        .expect("GPUDevice.popErrorScope rejected");
    if error.is_null() || error.is_undefined() {
        return None;
    }
    let message = js_sys::Reflect::get(&error, &JsValue::from_str("message"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| format!("{error:?}"));
    Some(message)
}

#[wasm_bindgen_test(async)]
async fn chrome_dawn_creates_and_executes_gpu_pick_pipeline_without_validation_error() {
    let instance = create_instance();
    let adapter = request_adapter_async(&instance)
        .await
        .expect("Chrome/Dawn must expose a WebGPU adapter");
    let (device, queue) = request_device_async(&adapter)
        .await
        .expect("Chrome/Dawn must create a WebGPU device");
    let device = Arc::new(device);
    let queue = Arc::new(queue);
    let mut renderer = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        2 * 1024 * 1024,
    )
    .expect("renderer initialization failed");

    let point_count = 65_537usize;
    for id in ["browser-direct-x", "browser-direct-y"] {
        renderer
            .add_column(
                id,
                &Column {
                    data: vec![0.0_f32],
                    min: 0.0,
                    max: 0.0,
                },
            )
            .expect("browser direct-scan column upload failed");
    }
    renderer
        .add_column(
            "browser-pick-x",
            &Column {
                data: (0..point_count).map(|index| index as f32).collect(),
                min: 0.0,
                max: (point_count - 1) as f32,
            },
        )
        .expect("browser pick x-column upload failed");
    renderer
        .add_column(
            "browser-pick-y",
            &Column {
                data: vec![5.0_f32; point_count],
                min: 5.0,
                max: 5.0,
            },
        )
        .expect("browser pick y-column upload failed");

    let scatter = || DataScatterStyleConfig {
        point_color: Color::BLACK,
        point_shape: ScatterShape::CircleFilled,
        // The 65,537 points span 100 px. Keep the radius below neighbour
        // spacing so only the exact centre point joins the zero-distance tie.
        point_size: 0.0002,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
    };
    let series = vec![
        SeriesConfig {
            source_id: Some("browser-direct-source".into()),
            series_id: "browser-direct-series".into(),
            label: None,
            x_column: "browser-direct-x".into(),
            y_column: "browser-direct-y".into(),
            render_type: DataRenderType::Scatter { scatter: scatter() },
        },
        SeriesConfig {
            source_id: Some("browser-source".into()),
            series_id: "browser-series".into(),
            label: None,
            x_column: "browser-pick-x".into(),
            y_column: "browser-pick-y".into(),
            render_type: DataRenderType::Scatter { scatter: scatter() },
        },
    ];

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 100,
    });
    config.chart_title.top_margin = 0.0;
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.out_margin = 0.0;
        axis.major_tick_length = 0.0;
    }
    for axis in [&mut config.top_x, &mut config.bottom_x] {
        axis.min = 0.0;
        axis.max = (point_count - 1) as f64;
    }
    for axis in [&mut config.left_y, &mut config.right_y] {
        axis.min = 0.0;
        axis.max = 10.0;
    }
    config.draw_style = DrawStyle::Precise;
    assert_eq!(
        config.data_area().expect("explicit data area").0,
        config.chart_area.0
    );
    let chart_id = renderer
        .register_chart(config, series)
        .expect("chart registration failed");

    push_validation_scope(&device);
    renderer
        .enable_gpu_picking()
        .expect("GPU picker pipeline preparation failed");
    let validation_error = pop_error_scope(&device).await;
    assert!(
        validation_error.is_none(),
        "Chrome/Dawn rejected the GPU pick pipeline: {validation_error:?}"
    );

    push_validation_scope(&device);
    renderer
        .prepare_gpu_picking_for_chart(chart_id)
        .expect("browser chart pick registry preparation failed");
    let picked = renderer
        .pick_chart(
            chart_id,
            GpuPickRequest {
                canvas_position_px: [50.0, 50.0],
                display_panel_px: Rect {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                },
                display_scale: 1.0,
                max_distance_px: 0.0,
            },
        )
        .expect("browser pick submission failed")
        .resolve()
        .await
        .expect("browser pick readback failed")
        .expect("known visible point was not picked");
    let execution_error = pop_error_scope(&device).await;

    assert!(
        execution_error.is_none(),
        "Chrome/Dawn rejected GPU pick execution: {execution_error:?}"
    );
    assert_eq!(picked.source_id.as_deref(), Some("browser-source"));
    assert_eq!(picked.series_id, "browser-series");
    assert_eq!(picked.point_index, point_count / 2);
    assert_eq!(picked.distance_px, 0.0);
}
