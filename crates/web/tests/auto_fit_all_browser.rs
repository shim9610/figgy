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

fn assert_close(actual: f64, expected: f64, field: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{field} = {actual}, expected {expected}"
    );
}

#[wasm_bindgen_test(async)]
async fn auto_fit_all_commits_ranges_without_a_pending_frame() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    chart
        .register_column_f32("x", &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0])
        .expect("register x");
    chart
        .register_column_f32("y", &[0.0, 1.0, 4.0, 9.0, 16.0, 25.0])
        .expect("register y");
    chart
        .add_line_series("s", "x", "y", 2.0, "")
        .expect("add line series");
    chart
        .ensure_extent_engine()
        .await
        .expect("ensure_extent_engine");
    chart.auto_fit_all(0.05).await.expect("auto_fit_all");

    let cfg: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    assert_close(
        cfg["bottom_x"]["min"].as_f64().unwrap(),
        -0.25,
        "bottom_x.min",
    );
    assert_close(
        cfg["bottom_x"]["max"].as_f64().unwrap(),
        5.25,
        "bottom_x.max",
    );
    assert_close(cfg["left_y"]["min"].as_f64().unwrap(), -1.25, "left_y.min");
    assert_close(cfg["left_y"]["max"].as_f64().unwrap(), 26.25, "left_y.max");

    chart.frame().expect("frame after auto_fit_all");
}
