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

fn fitted_ranges(chart: &FiggyChart) -> (f64, f64, f64, f64) {
    let config: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    (
        config["bottom_x"]["min"].as_f64().expect("bottom_x.min"),
        config["bottom_x"]["max"].as_f64().expect("bottom_x.max"),
        config["left_y"]["min"].as_f64().expect("left_y.min"),
        config["left_y"]["max"].as_f64().expect("left_y.max"),
    )
}

fn assert_close(actual: f64, expected: f64, field: &str) {
    assert!(
        (actual - expected).abs() < 1.0e-3,
        "{field} = {actual}, expected {expected}"
    );
}

#[wasm_bindgen_test(async)]
async fn public_f32_and_f64_register_and_update_routes_change_fitted_ranges() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    chart
        .ensure_extent_engine()
        .await
        .expect("ensure_extent_engine");
    chart
        .register_column_f32("x_f32", &[0.0, 1.0, 2.0])
        .expect("register_column_f32 x");
    chart
        .register_column_f32("y", &[10.0, 20.0, 30.0])
        .expect("register_column_f32 y");
    chart
        .add_line_series("series", "x_f32", "y", 2.0, "")
        .expect("add f32 series");
    chart.auto_fit_all(0.0).await.expect("fit registered f32");
    assert_eq!(fitted_ranges(&chart), (0.0, 2.0, 10.0, 30.0));

    chart
        .update_register_column_f32("x_f32", &[100.0, 101.0, 104.0])
        .expect("update_register_column_f32 x");
    chart.auto_fit_all(0.0).await.expect("fit updated f32");
    assert_eq!(fitted_ranges(&chart), (100.0, 104.0, 10.0, 30.0));

    assert!(chart.remove_series("series").expect("remove f32 series"));
    let epoch = 1_700_000_000_000.125_f64;
    let registered = [epoch, epoch + 0.25, epoch + 0.75];
    assert_eq!(registered[0] as f32, registered[2] as f32);
    chart
        .register_column_f64("x_f64", &registered)
        .expect("register_column_f64 x");
    chart
        .add_line_series("series", "x_f64", "y", 2.0, "")
        .expect("add f64 series");
    chart.auto_fit_all(0.0).await.expect("fit registered f64");
    let (x_min, x_max, y_min, y_max) = fitted_ranges(&chart);
    assert_close(x_min, epoch, "registered bottom_x.min");
    assert_close(x_max, epoch + 0.75, "registered bottom_x.max");
    assert_close(x_max - x_min, 0.75, "registered bottom_x span");
    assert_eq!((y_min, y_max), (10.0, 30.0));

    let updated = [epoch + 8.0, epoch + 8.25, epoch + 8.75];
    assert_eq!(updated[0] as f32, updated[2] as f32);
    chart
        .update_register_column_f64("x_f64", &updated)
        .expect("update_register_column_f64 x");
    chart.auto_fit_all(0.0).await.expect("fit updated f64");
    let (x_min, x_max, y_min, y_max) = fitted_ranges(&chart);
    assert_close(x_min, epoch + 8.0, "updated bottom_x.min");
    assert_close(x_max, epoch + 8.75, "updated bottom_x.max");
    assert_close(x_max - x_min, 0.75, "updated bottom_x span");
    assert_eq!((y_min, y_max), (10.0, 30.0));

    chart.frame().expect("frame after column replacements");
}
