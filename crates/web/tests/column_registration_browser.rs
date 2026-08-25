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

fn register_matrix_fixture(chart: &mut FiggyChart) -> Vec<String> {
    let coords = [0.0, 1.0];
    chart
        .register_column_f64("gx", &coords)
        .expect("register gx");
    chart
        .register_column_f64("gy", &coords)
        .expect("register gy");

    let ids = vec!["z0".to_owned(), "z1".to_owned()];
    chart
        .register_columns_f64(ids.clone(), &[0.0, 1.0, 1.0, 2.0], 2)
        .expect("register matrix columns");

    let mut config: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    let mut axis = config["left_y"].clone();
    axis["min"] = serde_json::json!(0.0);
    axis["max"] = serde_json::json!(2.0);
    axis["major_spacing"] = serde_json::json!(0.5);
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
        "axis": axis,
    });
    chart
        .set_config(&config.to_string())
        .expect("set matrix colorbar");
    ids
}

fn matrix_series(kind: &str, ids: &[String], levels: &[f64]) -> serde_json::Value {
    let matrix = serde_json::json!({
        "columns": ids,
        "orientation": "ColumnsAreX",
        "grid_layout": "Centers",
    });
    let contour = serde_json::json!({
        "levels": levels,
        "line": {
            "line_style": "Solid",
            "line_color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
            "line_width": 1.5,
        },
    });
    let render_type = match kind {
        "Contour" => serde_json::json!({
            "Contour": {"matrix": matrix, "contour": contour},
        }),
        "HeatmapContour" => serde_json::json!({
            "HeatmapContour": {
                "matrix": matrix,
                "fill": {"mode": "Continuous", "shading": "Interpolated", "opacity": 1.0},
                "contour": contour,
            },
        }),
        _ => panic!("unsupported matrix render type"),
    };
    serde_json::json!([{
        "series_id": format!("grid-{kind}"),
        "x_column": "gx",
        "y_column": "gy",
        "render_type": render_type,
    }])
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

/// A matrix registered as one batch draws.
///
/// The public path end to end: `register_columns_f64` for the grid, `set_config`
/// for the colourbar a field needs, `set_series` for the declaration, then a
/// frame. A wiring mistake between the batch registry and `ensure_columns_exist`
/// shows up as a rejected `set_series` rather than a wrong picture, which is why
/// this goes through the JSON entry points rather than `add_line_series`.
#[wasm_bindgen_test(async)]
async fn a_matrix_registered_as_one_batch_draws() {
    const N: usize = 12;
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");

    let coords: Vec<f64> = (0..N).map(|i| i as f64 / (N - 1) as f64).collect();
    chart
        .register_column_f64("gx", &coords)
        .expect("register gx");
    chart
        .register_column_f64("gy", &coords)
        .expect("register gy");

    // z = x + y, laid out column-major in one flat buffer: column c holds x = c.
    let ids: Vec<String> = (0..N).map(|c| format!("z{c}")).collect();
    let z: Vec<f64> = coords
        .iter()
        .flat_map(|x| coords.iter().map(move |y| x + y))
        .collect();
    chart
        .register_columns_f64(ids.clone(), &z, N)
        .expect("register the grid as one batch");

    // A matrix series needs `Config.colorbar` — it owns the z range
    // (`requires_colorbar` covers every matrix render type, contour included).
    // The axis is taken from `left_y` rather than written out here: it is already
    // a valid serialized `AxisOptions`, so this cannot drift from the schema.
    let mut config: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    let mut axis = config["left_y"].clone();
    axis["min"] = serde_json::json!(0.0);
    axis["max"] = serde_json::json!(2.0);
    axis["major_spacing"] = serde_json::json!(0.5);
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
        "axis": axis,
    });
    chart
        .set_config(&config.to_string())
        .expect("set_config with a colourbar");

    let series = serde_json::json!([{
        "series_id": "grid",
        "x_column": "gx",
        "y_column": "gy",
        "render_type": {
            "HeatmapContour": {
                "matrix": {
                    "columns": ids,
                    "orientation": "ColumnsAreX",
                    "grid_layout": "Centers"
                },
                "fill": {"mode": "Continuous", "shading": "Interpolated", "opacity": 1.0},
                "contour": {
                    "levels": [0.5, 1.0, 1.5],
                    "line": {
                        "line_style": "Solid",
                        "line_color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
                        "line_width": 1.5
                    },
                    "labels": {
                        "visible": true,
                        "font_size": 11.0,
                        "format": "Decimal",
                        "significant_digits": 1,
                        "spacing_px": 100.0,
                        "anchors": [],
                        "bg_padding_px": 2.0
                    }
                }
            }
        }
    }]);
    chart
        .set_series(&series.to_string())
        .expect("the batch-registered ids must satisfy set_series");
    chart.frame().expect("frame a batch-registered matrix");
}

#[wasm_bindgen_test(async)]
async fn contour_level_limit_is_atomic_across_the_wasm_json_boundary() {
    for kind in ["Contour", "HeatmapContour"] {
        let mut chart = FiggyChart::create(canvas())
            .await
            .expect("FiggyChart.create");
        let ids = register_matrix_fixture(&mut chart);
        let accepted_levels: Vec<f64> = (0..1024).map(|index| index as f64 + 10.0).collect();
        let accepted = matrix_series(kind, &ids, &accepted_levels);

        chart
            .set_series(&accepted.to_string())
            .unwrap_or_else(|_| panic!("{kind} must accept 1024 contour levels"));
        let accepted_config = chart.get_config().expect("accepted config");
        let accepted_series = chart.get_series().expect("accepted series");
        let serialized: serde_json::Value =
            serde_json::from_str(&accepted_series).expect("accepted series json");
        assert_eq!(
            serialized[0]["render_type"][kind]["contour"]["levels"]
                .as_array()
                .expect("serialized contour levels")
                .len(),
            1024,
            "{kind} levels must not be truncated by the WASM facade"
        );
        chart
            .frame()
            .unwrap_or_else(|_| panic!("{kind} 1024-level frame"));

        let rejected_levels: Vec<f64> = (0..1025).map(|index| index as f64 + 10.0).collect();
        let rejected = matrix_series(kind, &ids, &rejected_levels);
        let error = chart
            .set_series(&rejected.to_string())
            .expect_err("1025 contour levels must throw through the WASM facade");
        let message = error.as_string().expect("string rejection");
        assert!(
            message.contains("contour level count 1025") && message.contains("maximum 1024"),
            "unexpected {kind} rejection: {message}"
        );
        assert_eq!(
            chart.get_config().expect("config after rejection"),
            accepted_config
        );
        assert_eq!(
            chart.get_series().expect("series after rejection"),
            accepted_series
        );
        chart
            .frame()
            .unwrap_or_else(|_| panic!("{kind} frame after rejected replacement"));
    }
}
