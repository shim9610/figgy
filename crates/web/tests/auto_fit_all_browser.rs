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

#[wasm_bindgen_test(async)]
async fn contour_auto_fit_uses_the_exact_rendered_sample_lattice() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    chart
        .register_column_f64("gx", &[-3.1, -2.9, 2.9, 3.1])
        .expect("register gx edges");
    chart
        .register_column_f64("gy", &[-2.5, -2.3, 2.3, 2.5])
        .expect("register gy edges");
    let z_ids = vec!["z0".to_owned(), "z1".to_owned(), "z2".to_owned()];
    chart
        .register_columns_f64(
            z_ids.clone(),
            &[0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 2.0, 3.0, 4.0],
            3,
        )
        .expect("register 3x3 contour matrix");

    let mut config: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    let mut z_axis = config["left_y"].clone();
    z_axis["min"] = serde_json::json!(0.0);
    z_axis["max"] = serde_json::json!(4.0);
    z_axis["major_spacing"] = serde_json::json!(1.0);
    config["colorbar"] = serde_json::json!({
        "visible": true,
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
        "axis": z_axis,
    });
    chart
        .set_config(&config.to_string())
        .expect("set contour colorbar");
    let series = serde_json::json!([{
        "series_id": "contour-fit",
        "x_column": "gx",
        "y_column": "gy",
        "render_type": {
            "Contour": {
                "matrix": {
                    "columns": z_ids,
                    "orientation": "ColumnsAreX",
                    "grid_layout": "Edges"
                },
                "contour": {
                    "levels": [2.0],
                    "line": {
                        "line_style": "Solid",
                        "line_color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
                        "line_width": 1.5
                    }
                }
            }
        }
    }]);
    chart
        .set_series(&series.to_string())
        .expect("set contour series");
    chart
        .ensure_extent_engine()
        .await
        .expect("ensure extent engine");
    chart.auto_fit_all(0.0).await.expect("fit contour lattice");

    let fitted: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("fitted config")).expect("config json");
    // Edges describe the three cells, but contours interpolate the three sample
    // points at adjacent-edge midpoints. The fit must stop at those same points.
    assert_close(
        fitted["bottom_x"]["min"].as_f64().unwrap(),
        -3.0,
        "bottom_x.min",
    );
    assert_close(
        fitted["bottom_x"]["max"].as_f64().unwrap(),
        3.0,
        "bottom_x.max",
    );
    assert_close(
        fitted["left_y"]["min"].as_f64().unwrap(),
        -2.4,
        "left_y.min",
    );
    assert_close(fitted["left_y"]["max"].as_f64().unwrap(), 2.4, "left_y.max");
}

fn range(axis: &serde_json::Value) -> (f64, f64) {
    (axis["min"].as_f64().unwrap(), axis["max"].as_f64().unwrap())
}

#[wasm_bindgen_test(async)]
async fn auto_fit_all_fits_only_the_active_layer() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    for (id, values) in [
        ("x", [0.0, 1.0, 2.0, 3.0, 4.0]),
        ("small", [0.0, 1.0, 2.0, 3.0, 4.0]),
        ("large", [0.0, 100.0, 200.0, 300.0, 400.0]),
        ("far", [10.0, 11.0, 12.0, 13.0, 14.0]),
    ] {
        chart
            .register_column_f32(id, &values)
            .expect("register column");
    }
    chart
        .add_line_series("base", "x", "small", 1.0, "")
        .expect("base series");
    chart
        .add_line_series("right", "x", "large", 1.0, "")
        .expect("overlay series");
    chart
        .add_line_series("cell", "far", "large", 1.0, "")
        .expect("subplot series");
    let mut config: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    let cell = config.clone();
    config["overlays"] = serde_json::json!([{ "series": ["right"] }]);
    config["subplots"] = serde_json::json!([{ "config": cell, "series": ["cell"] }]);
    chart
        .set_config(&config.to_string())
        .expect("add an overlay and a subplot");
    chart
        .ensure_extent_engine()
        .await
        .expect("ensure_extent_engine");
    let fitted = |chart: &FiggyChart| -> serde_json::Value {
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json")
    };

    // Base: the overlay's and the subplot's series do not widen the base axes.
    chart.auto_fit_all(0.0).await.expect("fit the base layer");
    let base = fitted(&chart);
    assert_eq!(range(&base["bottom_x"]), (0.0, 4.0));
    assert_eq!(range(&base["left_y"]), (0.0, 4.0));
    let overlay_before = range(&base["overlays"][0]["y_axis"]);
    let subplot_before = range(&base["subplots"][0]["config"]["left_y"]);
    assert_ne!(overlay_before, (0.0, 400.0));

    // Overlay: only its Y axis moves.
    chart
        .set_active_layer(&serde_json::json!({ "Overlay": 0 }).to_string())
        .expect("activate the overlay");
    chart
        .auto_fit_all(0.0)
        .await
        .expect("fit the overlay layer");
    let overlay = fitted(&chart);
    assert_eq!(range(&overlay["overlays"][0]["y_axis"]), (0.0, 400.0));
    assert_eq!(range(&overlay["left_y"]), (0.0, 4.0));
    assert_eq!(range(&overlay["bottom_x"]), (0.0, 4.0));
    assert_eq!(
        range(&overlay["subplots"][0]["config"]["left_y"]),
        subplot_before
    );

    // Subplot: its own X and Y axes, from its own series.
    chart
        .set_active_layer(&serde_json::json!({ "Subplot": 0 }).to_string())
        .expect("activate the subplot");
    chart
        .auto_fit_all(0.0)
        .await
        .expect("fit the subplot layer");
    let subplot = fitted(&chart);
    assert_eq!(
        range(&subplot["subplots"][0]["config"]["bottom_x"]),
        (10.0, 14.0)
    );
    assert_eq!(
        range(&subplot["subplots"][0]["config"]["left_y"]),
        (0.0, 400.0)
    );
    assert_eq!(range(&subplot["bottom_x"]), (0.0, 4.0));
    assert_eq!(range(&subplot["overlays"][0]["y_axis"]), (0.0, 400.0));
    chart.frame().expect("frame after the layer fits");
}
