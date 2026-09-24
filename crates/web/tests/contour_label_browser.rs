#![cfg(target_arch = "wasm32")]

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use figgy::FiggyChart;
use renderer::data::Column;
use renderer::data_config::{
    ContourConfig, ContourLabelConfig, DataLineStyleConfig, GridLayout, MatrixOrientation,
    MatrixRef,
};
use renderer::data_render::{create_instance, request_adapter_async, request_device_async};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, ChartDrawItem, Color, DataRenderType, Renderer, RendererDevice, Series, SeriesConfig,
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const SIZE: u32 = 200;

fn raw_device_method(device: &wgpu::webgpu::GpuDevice, name: &str) -> js_sys::Function {
    js_sys::Reflect::get(device.as_ref(), &JsValue::from_str(name))
        .expect("GPUDevice method lookup failed")
        .dyn_into()
        .expect("GPUDevice property was not a function")
}

fn push_raw_error_scope(device: &wgpu::Device, filter: &str) {
    let raw = device
        .as_webgpu()
        .expect("browser test requires the WebGPU backend");
    raw_device_method(raw, "pushErrorScope")
        .call1(raw.as_ref(), &JsValue::from_str(filter))
        .expect("GPUDevice.pushErrorScope failed");
}

async fn pop_raw_error_scope(device: &wgpu::Device) -> Option<String> {
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
    let name = js_sys::Reflect::get(&error, &JsValue::from_str("name"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| "GPUError".into());
    let message = js_sys::Reflect::get(&error, &JsValue::from_str("message"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| format!("{error:?}"));
    Some(format!("{name}: {message}"))
}

fn column(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

async fn renderer() -> Renderer {
    let instance = create_instance();
    let adapter = request_adapter_async(&instance)
        .await
        .expect("Chrome/Dawn must expose a WebGPU adapter");
    let (device, queue) = request_device_async(&adapter)
        .await
        .expect("Chrome/Dawn must create a WebGPU device");
    Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer initialization failed")
}

fn add_contour_columns(renderer: &mut Renderer) {
    renderer
        .add_columns(&[
            (
                "gx",
                &column(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &column(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "p0",
                &column(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "p1",
                &column(vec![1.0, 2.0, 3.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "p2",
                &column(vec![2.0, 3.0, 4.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("browser contour label fixture upload");
}

fn force_raw_validation_error(device: &wgpu::Device) {
    let raw = device
        .as_webgpu()
        .expect("browser test requires the WebGPU backend");
    let descriptor = js_sys::Object::new();
    js_sys::Reflect::set(
        &descriptor,
        &JsValue::from_str("size"),
        &JsValue::from_f64(4.0),
    )
    .expect("buffer size property");
    js_sys::Reflect::set(
        &descriptor,
        &JsValue::from_str("usage"),
        &JsValue::from_f64(0.0),
    )
    .expect("invalid buffer usage property");
    raw_device_method(raw, "createBuffer")
        .call1(raw.as_ref(), &descriptor)
        .expect("invalid descriptor must report through the validation scope");
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn chart() -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: SIZE,
        height: SIZE,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.min = 0.0;
    bar.axis.max = 4.0;
    bar.axis.major_spacing = 1.0;
    config.colorbar = Some(bar);
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 2.0);
    chart.set_y_range(0.0, 2.0);
    chart
}

fn automatic_series(levels: Vec<f64>) -> SeriesConfig {
    let count = levels.len();
    let transparent = Color::new(0.0, 0.0, 0.0, 0.0);
    SeriesConfig {
        series_id: "browser-label-capacity".into(),
        source_id: None,
        label: None,
        x_column: "gx".into(),
        y_column: "gy".into(),
        render_type: DataRenderType::Contour {
            matrix: MatrixRef {
                columns: vec!["p0".into(), "p1".into(), "p2".into()],
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            contour: ContourConfig {
                levels,
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: transparent,
                    line_width: 1.0,
                },
                per_level_color: Some(vec![transparent; count]),
                labels: Some(ContourLabelConfig {
                    visible: true,
                    font_size: 8.0,
                    color: Color::BLACK,
                    format: renderer::format::LabelFormat::Decimal,
                    significant_digits: 0,
                    spacing_px: 2000.0,
                    anchors: Vec::new(),
                    bg_color: Some(Color::new(1.0, 0.0, 1.0, 1.0)),
                    bg_padding_px: 2.0,
                }),
            },
        },
    }
}

#[wasm_bindgen_test(async)]
async fn chrome_webgpu_keeps_all_1024_automatic_contour_labels() {
    let mut renderer = renderer().await;
    push_raw_error_scope(renderer.device(), "internal");
    push_raw_error_scope(renderer.device(), "validation");
    add_contour_columns(&mut renderer);
    let chart = chart();
    let all_levels: Vec<f64> = (0..renderer::MAX_CONTOUR_LEVELS)
        .map(|index| 0.1 + 3.8 * index as f64 / (renderer::MAX_CONTOUR_LEVELS - 1) as f64)
        .collect();
    let config = automatic_series(all_levels);
    let style = renderer.create_style_for_series(&config).unwrap();
    let series = [Series {
        config: &config,
        style: &style,
    }];
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .expect("1024-label browser view");
    let items = [ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &series,
    }];
    let prepared = renderer
        .prepare(&items)
        .expect("1024-label browser prepare");
    let count = renderer
        .contour_label_instance_count_for_test(&prepared, 0, 0)
        .await
        .expect("1024-label browser indirect readback")
        .expect("automatic contour label snapshot");
    let image = renderer
        .export_panel_rgba_async(&chart, std::slice::from_ref(&config), 1.0)
        .await
        .expect("Chrome must consume the 1024-label indirect draw");
    let label_ink = image
        .rgba
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 180 && pixel[1] < 80 && pixel[2] > 180 && pixel[3] > 180)
        .count();

    let validation_error = pop_raw_error_scope(renderer.device()).await;
    let internal_error = pop_raw_error_scope(renderer.device()).await;
    assert!(
        validation_error.is_none() && internal_error.is_none(),
        "Chrome contour-label WebGPU error: validation={validation_error:?}, internal={internal_error:?}"
    );

    assert_eq!(count, 1024, "Chrome indirect label count must be exact");
    assert!(
        label_ink > 0,
        "Chrome consumed the indirect count but produced no automatic-label pixels"
    );
}

#[wasm_bindgen_test(async)]
async fn cancelled_contour_export_does_not_cross_the_host_error_scope() {
    let mut renderer = renderer().await;
    add_contour_columns(&mut renderer);
    let chart = chart();
    let configs = [automatic_series(vec![1.0, 2.0, 3.0])];

    push_raw_error_scope(renderer.device(), "validation");
    let mut export = Box::pin(renderer.export_panel_rgba_async(&chart, &configs, 1.0));
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    assert!(
        matches!(export.as_mut().poll(&mut context), Poll::Pending),
        "browser export must yield while WebGPU work completes"
    );
    drop(export);

    force_raw_validation_error(renderer.device());
    let host_error = pop_raw_error_scope(renderer.device()).await;
    assert!(
        host_error
            .as_deref()
            .is_some_and(|error| error.contains("Buffer usages must not be 0")),
        "cancelled export left a scope above the host validation scope: {host_error:?}"
    );

    renderer
        .export_panel_rgba_async(&chart, &configs, 1.0)
        .await
        .expect("contour export after cancellation");
}

fn canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window()
        .expect("window")
        .document()
        .expect("document");
    let canvas = document
        .create_element("canvas")
        .expect("canvas element")
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .expect("HtmlCanvasElement");
    canvas.set_width(200);
    canvas.set_height(160);
    canvas
}

fn facade_series(spacing_px: f32, visible: bool, explicit: bool) -> serde_json::Value {
    let anchors = if explicit {
        serde_json::json!([{
            "level_index": 0,
            "x": 1.0,
            "y": 1.0,
            "tx": 1.0,
            "ty": 0.0
        }])
    } else {
        serde_json::json!([])
    };
    serde_json::json!([{
        "series_id": "facade-label",
        "x_column": "gx",
        "y_column": "gy",
        "render_type": {
            "Contour": {
                "matrix": {
                    "columns": ["p0", "p1", "p2"],
                    "orientation": "ColumnsAreX",
                    "grid_layout": "Centers"
                },
                "contour": {
                    "levels": [2.0],
                    "line": {
                        "line_style": "Solid",
                        "line_color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
                        "line_width": 1.0
                    },
                    "labels": {
                        "visible": visible,
                        "font_size": 10.0,
                        "format": "Decimal",
                        "significant_digits": 2,
                        "spacing_px": spacing_px,
                        "anchors": anchors
                    }
                }
            }
        }
    }])
}

#[wasm_bindgen_test(async)]
async fn wasm_facade_preserves_state_and_rebuilds_after_font_registration() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    for (id, values) in [
        ("gx", [0.0, 1.0, 2.0]),
        ("gy", [0.0, 1.0, 2.0]),
        ("p0", [0.0, 1.0, 2.0]),
        ("p1", [1.0, 2.0, 3.0]),
        ("p2", [2.0, 3.0, 4.0]),
    ] {
        chart
            .register_column_f64(id, &values)
            .expect("register fixture column");
    }
    let mut config: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get config")).expect("config JSON");
    config["chart_area"] = serde_json::json!({"x": 0, "y": 0, "width": 200, "height": 160});
    config["bottom_x"]["min"] = serde_json::json!(0.0);
    config["bottom_x"]["max"] = serde_json::json!(2.0);
    config["left_y"]["min"] = serde_json::json!(0.0);
    config["left_y"]["max"] = serde_json::json!(2.0);
    config["bottom_x"]["label_style"]["label_font"] = serde_json::json!("Comic Neue");
    let mut colorbar_axis = config["left_y"].clone();
    colorbar_axis["min"] = serde_json::json!(0.0);
    colorbar_axis["max"] = serde_json::json!(4.0);
    config["colorbar"] = serde_json::json!({
        "visible": false,
        "side": "Right",
        "thickness_px": 18.0,
        "gap_px": 24.0,
        "length_frac": 0.75,
        "align": "Center",
        "offset_x": 0.0,
        "offset_y": 0.0,
        "colormap": "Viridis",
        "nan_color": {"r": 0.5, "g": 0.5, "b": 0.5, "a": 1.0},
        "border_color": {"r": 0.31, "g": 0.31, "b": 0.31, "a": 1.0},
        "border_width": 1.0,
        "axis": colorbar_axis
    });
    chart
        .set_config(&config.to_string())
        .expect("set contour config");

    let accepted = facade_series(80.0, true, false).to_string();
    chart.set_series(&accepted).expect("set valid labels");
    let accepted_snapshot = chart.get_series().expect("normalized valid series");
    chart.frame().expect("initial labelled frame");
    chart
        .export_png(1.0)
        .await
        .expect("initial labelled export");

    let rejected = facade_series(-1.0, false, true).to_string();
    let error = chart
        .set_series(&rejected)
        .expect_err("invalid spacing must reject");
    assert!(
        error
            .as_string()
            .unwrap_or_default()
            .contains("spacing_px must be finite"),
        "unexpected spacing error: {error:?}"
    );
    assert_eq!(
        chart.get_series().expect("series after rejection"),
        accepted_snapshot
    );
    chart.frame().expect("frame after rejected spacing");
    chart
        .export_png(1.0)
        .await
        .expect("export after rejected spacing");

    chart
        .register_font(include_bytes!("../../renderer/fonts/ComicNeue-Regular.ttf"))
        .expect("register browser font");
    chart.frame().expect("frame after font generation change");
    chart
        .export_png(1.0)
        .await
        .expect("export after font generation change");

    let automatic_overflow = facade_series(f32::MAX, true, false).to_string();
    chart
        .set_series(&automatic_overflow)
        .expect("finite automatic spacing");
    chart
        .frame()
        .expect("frame before export-only scale overflow");
    chart
        .export_png(f32::INFINITY)
        .await
        .expect_err("clamped automatic spacing product must overflow");
    chart.frame().expect("frame after rejected export");

    let explicit = facade_series(f32::MAX, true, true).to_string();
    chart
        .set_series(&explicit)
        .expect("finite explicit spacing");
    chart
        .export_png(f32::INFINITY)
        .await
        .expect("explicit export must not multiply spacing");
}
