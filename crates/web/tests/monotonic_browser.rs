#![cfg(target_arch = "wasm32")]
//! Monotonic-x line series in the browser product (#271), on the kernel's
//! monotonic chart (`set_monotonic_document`): typed arrays of
//! every element type, per channel or interleaved with a stride, through
//! `set_monotonic` and `append_monotonic`; `needs_source` before the first
//! call; a zoom through `set_config` and a plain resize refused, the x range
//! set by `set_monotonic`, the resize and the layout fit applied with the
//! group's source (`resize_monotonic`, `fit_layout`); a plot-area resize
//! gesture that proposes a rect the host applies; a theme without one; a
//! pick over the kept points; and an export at 2x with the group's source.

use figgy::FiggyChart;
use renderer::layout::{ChartArea, Rect};
use renderer::{Chart, SeriesConfig};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const N: usize = 2_000;

fn canvas() -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(WIDTH);
    canvas.set_height(HEIGHT);
    canvas
}

fn config() -> renderer::Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, (N - 1) as f64);
    chart.set_y_range(-200.0, 200.0);
    chart.config().clone()
}

fn value(i: usize) -> f64 {
    150.0 * (i as f64 * 0.01).sin()
}

fn needs(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&chart.needs_source().unwrap()).unwrap()
}

#[wasm_bindgen_test(async)]
async fn typed_arrays_feed_set_append_pick_and_export() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let series = vec![
        SeriesConfig::monotonic_line("a", "daq", 0),
        SeriesConfig::monotonic_line("b", "daq", 1),
    ];
    let series_json = serde_json::to_string(&series).unwrap();
    // A general chart refuses a monotonic line series: it lives only in a
    // monotonic chart (#271).
    assert!(chart.set_series(&series_json).is_err());
    chart
        .set_monotonic_document(&serde_json::to_string(&config()).unwrap(), &series_json)
        .unwrap();
    assert_eq!(needs(&chart)[0]["change"], "NotBuilt");

    // One Float64Array per channel.
    let a: Vec<f64> = (0..N).map(value).collect();
    let b: Vec<f64> = a.iter().map(|v| -v).collect();
    let channels = js_sys::Array::new();
    channels.push(&js_sys::Float64Array::from(a.as_slice()));
    channels.push(&js_sys::Float64Array::from(b.as_slice()));
    let update: serde_json::Value = serde_json::from_str(
        &chart
            .set_monotonic(
                "daq",
                0.0,
                1.0,
                channels.into(),
                "autoscale".into(),
                None,
                None,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(update["len"], N);
    assert_eq!(needs(&chart), serde_json::json!([]));
    chart.frame().unwrap();

    // One interleaved Int16Array, a timestamp column first.
    let frames: Vec<i16> = (0..N)
        .flat_map(|i| [i as i16, value(i) as i16, -value(i) as i16])
        .collect();
    chart
        .set_monotonic(
            "daq",
            0.0,
            1.0,
            js_sys::Int16Array::from(frames.as_slice()).into(),
            x_range(0.0, (N - 1) as f64),
            Some(3),
            Some(vec![1, 2]),
        )
        .unwrap();
    chart.frame().unwrap();

    // Every other element type, per channel.
    for data in [
        JsValue::from(js_sys::Float32Array::from(
            a.iter().map(|&v| v as f32).collect::<Vec<_>>().as_slice(),
        )),
        JsValue::from(js_sys::Int32Array::from(
            a.iter().map(|&v| v as i32).collect::<Vec<_>>().as_slice(),
        )),
        JsValue::from(js_sys::Uint16Array::from(
            a.iter()
                .map(|&v| (v + 200.0) as u16)
                .collect::<Vec<_>>()
                .as_slice(),
        )),
        JsValue::from(js_sys::Uint8Array::from(
            a.iter()
                .map(|&v| (v / 2.0 + 120.0) as u8)
                .collect::<Vec<_>>()
                .as_slice(),
        )),
    ] {
        let list = js_sys::Array::new();
        list.push(&data);
        list.push(&data);
        chart
            .set_monotonic(
                "daq",
                0.0,
                1.0,
                list.into(),
                x_range(0.0, (N - 1) as f64),
                None,
                None,
            )
            .unwrap();
        chart.frame().unwrap();
    }

    // A sliding append moves the x range by the new samples' span.
    let more = js_sys::Array::new();
    more.push(&js_sys::Uint8Array::from(&[120u8; 100][..]));
    more.push(&js_sys::Uint8Array::from(&[120u8; 100][..]));
    let update: serde_json::Value = serde_json::from_str(
        &chart
            .append_monotonic("daq", more.into(), true, None, None)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(update["x_range"][0], 100.0);
    assert_eq!(needs(&chart), serde_json::json!([]));
    chart.frame().unwrap();

    // A zoom through the general config call would move the x-to-pixel
    // mapping without the source: it is refused and the document keeps its
    // range.
    let stored: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let mut zoomed = stored.clone();
    zoomed.bottom_x.max = stored.bottom_x.min + 500.0;
    assert!(
        chart
            .set_config(&serde_json::to_string(&zoomed).unwrap())
            .is_err()
    );
    let kept: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    assert_eq!(kept.bottom_x.max, stored.bottom_x.max);
    // The x range is set by the full call.
    chart
        .set_monotonic(
            "daq",
            0.0,
            1.0,
            channel_list(&a, &b).into(),
            x_range(0.0, 500.0),
            None,
            None,
        )
        .unwrap();
    let zoomed: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    assert_eq!(zoomed.bottom_x.max, 500.0);
    // A style edit needs no source.
    chart.apply_theme("dark", "all").unwrap();
    assert_eq!(needs(&chart), serde_json::json!([]));
    // A resize moves the mapping: the plain call is refused, the
    // monotonic one reduces the group again from its source.
    assert_eq!(
        chart.monotonic_groups().unwrap(),
        serde_json::json!(["daq"]).to_string()
    );
    assert!(chart.resize(WIDTH / 2, HEIGHT / 2).is_err());
    chart
        .resize_monotonic(WIDTH / 2, HEIGHT / 2, sources(&a, &b), JsValue::UNDEFINED)
        .unwrap();
    assert_eq!(needs(&chart), serde_json::json!([]));
    chart.frame().unwrap();
    chart
        .resize_monotonic(WIDTH, HEIGHT, sources(&a, &b), JsValue::UNDEFINED)
        .unwrap();
    // The layout fit takes the source too.
    chart.fit_layout(sources(&a, &b)).unwrap();
    assert_eq!(needs(&chart), serde_json::json!([]));
    chart.frame().unwrap();

    // A plot-area resize gesture proposes a plot rect and applies nothing;
    // the host applies it with the sources.
    let stored: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let plot = stored.data_area().unwrap().0;
    let measure = renderer::CpuTextMeasure::for_style(&stored.draw_style);
    let centre = (
        (plot.x + plot.width / 2) as f32,
        (plot.y + plot.height / 2) as f32,
    );
    chart.on_press(centre.0, centre.1).unwrap();
    chart.on_release().unwrap();
    let mut east = None;
    renderer::Resizable::resize_handles(
        &renderer::select::DataAreaElement,
        &stored,
        &measure,
        &mut |handle, square| {
            if handle == renderer::ResizeHandle::E {
                east = Some(square);
            }
        },
    );
    let east = east.unwrap();
    chart
        .on_press(east.x + east.width / 2.0, east.y + east.height / 2.0)
        .unwrap();
    chart.on_move(-40.0, 0.0).unwrap();
    chart.on_release().unwrap();
    let kept: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    assert_eq!(
        kept.data_area().unwrap().0,
        plot,
        "the gesture applies nothing"
    );
    let request = chart.take_plot_resize_request().unwrap().unwrap();
    assert_eq!(chart.take_plot_resize_request().unwrap(), None, "read once");
    let rect: serde_json::Value = serde_json::from_str(&request).unwrap();
    assert!(rect["width"].as_u64().unwrap() < u64::from(plot.width));
    let proposed = js_sys::JSON::parse(&request).unwrap();
    chart
        .resize_monotonic(WIDTH, HEIGHT, sources(&a, &b), proposed)
        .unwrap();
    let resized: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    assert_eq!(
        u64::from(resized.data_area().unwrap().0.width),
        rect["width"].as_u64().unwrap()
    );
    assert_eq!(needs(&chart), serde_json::json!([]));
    chart.frame().unwrap();

    // A pick lands on a kept point and reports its logical index; the
    // selection details read it from the envelope.
    let plot = config().data_area().unwrap().0;
    let at = (
        (plot.x + plot.width / 2) as f32,
        (plot.y + plot.height / 2) as f32,
    );
    let _ = chart.pick_point(at.0, at.1, 400.0).await;

    // An export at 2x maps to other device pixels: it takes the source.
    assert!(chart.export_png(2.0).await.is_err());
    let png = chart
        .export_png_monotonic(2.0, sources(&a, &b))
        .await
        .unwrap();
    assert!(png.length() > 0);
}

fn x_range(min: f64, max: f64) -> JsValue {
    let range = js_sys::Array::new();
    range.push(&JsValue::from_f64(min));
    range.push(&JsValue::from_f64(max));
    range.into()
}

fn channel_list(a: &[f64], b: &[f64]) -> js_sys::Array {
    let channels = js_sys::Array::new();
    channels.push(&js_sys::Float64Array::from(a));
    channels.push(&js_sys::Float64Array::from(b));
    channels
}

/// Group `daq`'s whole source, as the calls that take sources take it.
fn sources(a: &[f64], b: &[f64]) -> js_sys::Array {
    let source = js_sys::Object::new();
    let set = |key: &str, value: &JsValue| {
        js_sys::Reflect::set(&source, &JsValue::from_str(key), value).unwrap();
    };
    set("group", &JsValue::from_str("daq"));
    set("start", &JsValue::from_f64(0.0));
    set("interval", &JsValue::from_f64(1.0));
    set("data", &channel_list(a, b).into());
    let sources = js_sys::Array::new();
    sources.push(&source);
    sources
}
