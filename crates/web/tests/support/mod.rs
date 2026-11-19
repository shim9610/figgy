//! Helpers the browser tests share: canvas setup, timer turns, JSON
//! readers, the WebGPU renderer and device lookups. Each test binary
//! includes this module with `mod support;` and uses what it needs.
#![allow(dead_code)]

use figgy::FiggyChart;
use renderer::config::{DataSelectionsConfig, PickedDataRef};
use renderer::data::Column;
use renderer::gpu::{create_instance, request_adapter_async, request_device_async};
use renderer::{Config, DataRenderType, Renderer, RendererDevice, SeriesConfig};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlCanvasElement;

pub fn canvas_480x320() -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(480);
    canvas.set_height(320);
    canvas
}

pub fn canvas_480x360() -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(480);
    canvas.set_height(360);
    canvas
}

pub fn canvas_800x500() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window()
        .expect("window")
        .document()
        .expect("document");
    let canvas = document
        .create_element("canvas")
        .expect("canvas element")
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .expect("HtmlCanvasElement");
    canvas.set_width(800);
    canvas.set_height(500);
    canvas
}

pub fn canvas_320x240() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas = document
        .create_element("canvas")
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();
    canvas.set_width(320);
    canvas.set_height(240);
    canvas
}

/// One timer turn of 20 ms.
pub async fn turn() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 20)
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}

pub fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

pub fn rect(value: &serde_json::Value) -> [f64; 4] {
    let n = |i: usize| value[i].as_f64().unwrap();
    [n(0), n(1), n(2), n(3)]
}

pub fn code(error: &JsValue) -> Option<String> {
    js_sys::Reflect::get(error, &JsValue::from_str("code"))
        .ok()
        .and_then(|code| code.as_string())
}

pub async fn details(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&chart.selection_details().await.unwrap()).unwrap()
}

pub fn select(chart: &mut FiggyChart, series_id: &str, bin_index: usize) {
    let mut config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: series_id.into(),
            bin_index,
            half: None,
        }],
        ..Default::default()
    });
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .unwrap();
}

pub fn series(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type,
    }
}

pub fn column(data: Vec<f64>) -> Column<f64> {
    Column { data }
}

pub async fn renderer() -> Renderer {
    let instance = create_instance();
    let adapter = request_adapter_async(&instance)
        .await
        .expect("Chrome/Dawn must expose a WebGPU adapter");
    let (device, queue) = request_device_async(&adapter)
        .await
        .expect("Chrome/Dawn must create a WebGPU device");
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer initialization failed")
}

pub fn raw_device_method(device: &wgpu::webgpu::GpuDevice, name: &str) -> js_sys::Function {
    js_sys::Reflect::get(device.as_ref(), &JsValue::from_str(name))
        .expect("GPUDevice method lookup failed")
        .dyn_into()
        .expect("GPUDevice property was not a function")
}
