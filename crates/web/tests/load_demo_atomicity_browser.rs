#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

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
    canvas.set_width(800);
    canvas.set_height(500);
    canvas
}

fn segment_text(value: &serde_json::Value) -> String {
    value["segments"]
        .as_array()
        .expect("rich text segments")
        .iter()
        .filter_map(|segment| segment["text"].as_str())
        .collect()
}

fn assert_close(actual: f64, expected: f64, field: &str) {
    assert!(
        (actual - expected).abs() < 1.0e-5,
        "{field} = {actual}, expected {expected}"
    );
}

fn cast_bounds(values: &[f64]) -> (f64, f64) {
    values
        .iter()
        .map(|value| *value as f32 as f64)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        })
}

#[wasm_bindgen_test(async)]
async fn load_demo_publishes_complete_repeatable_state_and_recreates_extents_lazily() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    chart
        .ensure_extent_engine()
        .await
        .expect("ensure_extent_engine");

    chart.load_demo().expect("first load_demo");
    let first_config_text = chart.get_config().expect("first config");
    let first_series_text = chart.get_series().expect("first series");
    let config: serde_json::Value = serde_json::from_str(&first_config_text).expect("config json");
    let series: serde_json::Value = serde_json::from_str(&first_series_text).expect("series json");

    assert_eq!(segment_text(&config["chart_title"]["text"]), "figgy");
    assert_eq!(
        segment_text(&config["bottom_x"]["title_option"]["text"]),
        "x"
    );
    assert_eq!(segment_text(&config["left_y"]["title_option"]["text"]), "y");
    let legend = segment_text(&config["legend"]["content"]);
    assert!(legend.contains("sin(x)"));
    assert!(legend.contains("RC charge"));
    assert_eq!(series.as_array().expect("series array").len(), 2);
    assert_eq!(series[0]["series_id"], "sine");
    assert_eq!(series[0]["x_column"], "demo_x");
    assert_eq!(series[0]["y_column"], "demo_sin");
    assert_eq!(series[1]["series_id"], "rc");
    assert_eq!(series[1]["x_column"], "demo_t");
    assert_eq!(series[1]["y_column"], "demo_rc");
    let (xs, ys) = renderer::demo::sine_data(512);
    let (x_min, x_max) = cast_bounds(&xs);
    let (y_min, y_max) = cast_bounds(&ys);
    let x_padding = (x_max - x_min) * 0.02;
    let y_padding = (y_max - y_min) * 0.10;
    assert_close(
        config["bottom_x"]["min"].as_f64().expect("bottom_x.min"),
        x_min - x_padding,
        "bottom_x.min",
    );
    assert_close(
        config["bottom_x"]["max"].as_f64().expect("bottom_x.max"),
        x_max + x_padding,
        "bottom_x.max",
    );
    assert_close(
        config["left_y"]["min"].as_f64().expect("left_y.min"),
        y_min - y_padding,
        "left_y.min",
    );
    assert_close(
        config["left_y"]["max"].as_f64().expect("left_y.max"),
        y_max + y_padding,
        "left_y.max",
    );
    chart.frame().expect("frame after load_demo");

    chart.load_demo().expect("repeated load_demo");
    assert_eq!(
        chart.get_config().expect("repeated config"),
        first_config_text
    );
    assert_eq!(
        chart.get_series().expect("repeated series"),
        first_series_text
    );

    chart
        .auto_fit_all(0.0)
        .await
        .expect("postcommit lazy extent recreation");
    chart.frame().expect("frame after lazy extent recreation");
}
