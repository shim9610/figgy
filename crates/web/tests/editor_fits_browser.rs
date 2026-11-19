//! Editor fits on the browser chart (#312): a candidate config fitted
//! without installing it (`fit_config`, `fit_y_axes_config`, the scoped
//! `fit_config_scoped` of #334), one series'
//! style domain from its columns' statistics (`fit_point_size_domain`,
//! `fit_vector_scale`), and the mapped radius `selection_details` reports
//! for a size-mapped point.
#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use renderer::{
    Chart, Color, DataRenderType, SeriesConfig, VectorSeries,
    config::{AxisOptions, Config, DataSelectionsConfig, PickedDataRef},
    data_config::{MatrixRef, VectorScale},
};
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

mod support;
use support::canvas_480x320 as canvas;

wasm_bindgen_test_configure!(run_in_browser);

/// The error's rule code, or its message for an error without one.
fn code(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &JsValue::from_str("code"))
        .ok()
        .and_then(|code| code.as_string())
        .or_else(|| error.as_string())
        .unwrap_or_default()
}

#[track_caller]
fn assert_range(axis: &AxisOptions, min: f64, max: f64) {
    assert!(
        (axis.min - min).abs() < 1e-9 && (axis.max - max).abs() < 1e-9,
        "{:?} is not {:?}",
        (axis.min, axis.max),
        (min, max)
    );
}

fn config_of(chart: &FiggyChart) -> Config {
    serde_json::from_str(&chart.get_config().unwrap()).unwrap()
}

fn series_of(chart: &FiggyChart) -> Vec<SeriesConfig> {
    serde_json::from_str(&chart.get_series().unwrap()).unwrap()
}

/// A chart with columns of eleven values (`t` 0..10, `a` -1..1, `b`
/// 100..200, `u` 20..30, `flat` all 3, `zero` all 0) and `pair` (0, 1), a
/// line `a` on the base layer and a line `b` on an overlay with its own X
/// axis.
async fn layered_chart() -> FiggyChart {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let column = |from: f64, step: f64| -> Vec<f64> {
        (0..=10).map(|i| from + step * f64::from(i)).collect()
    };
    for (id, from, step) in [
        ("t", 0.0, 1.0),
        ("a", -1.0, 0.2),
        ("b", 100.0, 10.0),
        ("u", 20.0, 1.0),
        ("flat", 3.0, 0.0),
        ("zero", 0.0, 0.0),
    ] {
        chart.register_column_f64(id, &column(from, step)).unwrap();
    }
    chart.register_column_f64("pair", &[0.0, 1.0]).unwrap();
    let series = [
        SeriesConfig::line("a", "t", "a"),
        SeriesConfig::line("b", "u", "b"),
    ];
    chart
        .set_series(&serde_json::to_string(&series).unwrap())
        .unwrap();
    let mut config = config_of(&chart);
    let area = config.chart_area.clone();
    config = Chart::new(config)
        .with_overlay_xy(
            AxisOptions::overlay_y("U", Color::BLACK),
            AxisOptions::overlay_y("B", Color::BLACK),
            &["b"],
        )
        .config()
        .clone();
    config.chart_area = area;
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .unwrap();
    chart
}

// The candidate fit returns fitted ranges for every layer and changes
// nothing installed; at 5% it is what `fit_ranges` then installs.
#[wasm_bindgen_test(async)]
async fn fit_config_fits_a_candidate_without_installing_it() {
    let mut chart = layered_chart().await;
    let installed = chart.get_config().unwrap();
    let fitted: Config = serde_json::from_str(&chart.fit_config(&installed, 0.1).unwrap()).unwrap();
    assert_eq!(
        chart.get_config().unwrap(),
        installed,
        "the chart is untouched"
    );
    assert_range(&fitted.bottom_x, -1.0, 11.0);
    assert_range(&fitted.left_y, -1.2, 1.2);
    assert_range(&fitted.overlays[0].x_axis, 19.0, 31.0);
    assert_range(&fitted.overlays[0].y_axis, 90.0, 210.0);

    let at_five: Config =
        serde_json::from_str(&chart.fit_config(&installed, 0.05).unwrap()).unwrap();
    chart.fit_ranges().unwrap();
    let after = config_of(&chart);
    assert_eq!(after.bottom_x, at_five.bottom_x, "one fit path");
    assert_eq!(after.left_y, at_five.left_y);
    assert_eq!(after.overlays[0], at_five.overlays[0]);

    // The fitted candidate installs and draws at 1x and 2x.
    chart
        .set_config(&serde_json::to_string(&fitted).unwrap())
        .unwrap();
    for scale in [1.0, 2.0] {
        let png = chart.export_png(scale).await.unwrap();
        assert!(png.length() > 0, "{scale}x");
    }
}

// The Y-only fit keeps every X axis exactly as given.
#[wasm_bindgen_test(async)]
async fn fit_y_axes_config_keeps_every_x_axis() {
    let chart = layered_chart().await;
    let mut candidate = config_of(&chart);
    candidate.bottom_x.min = 2.0;
    candidate.bottom_x.max = 3.0;
    candidate.overlays[0].x_axis.min = -7.0;
    let json = serde_json::to_string(&candidate).unwrap();
    let installed = chart.get_config().unwrap();
    let fitted: Config =
        serde_json::from_str(&chart.fit_y_axes_config(&json, 0.05).unwrap()).unwrap();
    assert_eq!(
        chart.get_config().unwrap(),
        installed,
        "the chart is untouched"
    );
    assert_eq!(fitted.bottom_x, candidate.bottom_x);
    assert_eq!(fitted.top_x, candidate.top_x);
    assert_eq!(fitted.overlays[0].x_axis, candidate.overlays[0].x_axis);
    assert_range(&fitted.left_y, -1.1, 1.1);
    assert_range(&fitted.overlays[0].y_axis, 95.0, 205.0);
}

// The scoped fit moves only the axes its scope JSON selects, holds the
// kept ones, and refuses a scope it cannot read or a layer the candidate
// does not have.
#[wasm_bindgen_test(async)]
async fn fit_config_scoped_moves_only_the_scope() {
    let chart = layered_chart().await;
    let installed = chart.get_config().unwrap();
    let mut candidate = config_of(&chart);
    candidate.left_y.min = 4.0;
    candidate.left_y.max = 5.0;
    let json = serde_json::to_string(&candidate).unwrap();
    let fitted: Config = serde_json::from_str(
        &chart
            .fit_config_scoped(&json, r#"{"layer": "Base", "axes": ["x"]}"#, 0.05)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        chart.get_config().unwrap(),
        installed,
        "the chart is untouched"
    );
    assert_range(&fitted.bottom_x, -0.5, 10.5);
    assert_eq!(fitted.left_y, candidate.left_y);
    assert_eq!(fitted.overlays[0], candidate.overlays[0]);

    let overlay: Config = serde_json::from_str(
        &chart
            .fit_config_scoped(
                &json,
                r#"{"layer": {"Overlay": 0}, "axes": ["x", "y"], "linked": "target"}"#,
                0.05,
            )
            .unwrap(),
    )
    .unwrap();
    assert_range(&overlay.overlays[0].x_axis, 19.5, 30.5);
    assert_range(&overlay.overlays[0].y_axis, 95.0, 205.0);
    assert_eq!(overlay.bottom_x, candidate.bottom_x);
    assert_eq!(overlay.left_y, candidate.left_y);

    let held = chart
        .fit_config_scoped(
            &json,
            r#"{"layer": {"Overlay": 0}, "axes": ["y"],
                "keep": [{"layer": {"Overlay": 0}, "axis": "y"}]}"#,
            0.05,
        )
        .unwrap();
    let held: Config = serde_json::from_str(&held).unwrap();
    assert_eq!(held, candidate);

    let error = chart
        .fit_config_scoped(&json, r#"{"layer": {"Overlay": 4}, "axes": ["x"]}"#, 0.05)
        .unwrap_err();
    assert_eq!(code(&error), "FitScopeLayer");
    let error = chart
        .fit_config_scoped(
            &json,
            r#"{"layer": "Base", "axes": ["x"], "keep": [{"layer": {"Subplot": 0}, "axis": "x"}]}"#,
            0.05,
        )
        .unwrap_err();
    assert_eq!(code(&error), "FitScopeLayer");
    let error = chart
        .fit_config_scoped(&json, r#"{"layer": "Base", "axes": ["x"]}"#, -1.0)
        .unwrap_err();
    assert_eq!(code(&error), "FitPadding");
    assert!(
        chart
            .fit_config_scoped(&json, r#"{"layer": "Base", "axes": ["w"]}"#, 0.05)
            .is_err()
    );
    assert!(
        chart
            .fit_config_scoped(
                &json,
                r#"{"layer": "Base", "axes": ["x"], "more": 1}"#,
                0.05
            )
            .is_err()
    );
    assert_eq!(chart.get_config().unwrap(), installed);
}

// A candidate the chart cannot fit is refused, and nothing changes.
#[wasm_bindgen_test(async)]
async fn candidate_fits_refuse_what_they_cannot_fit() {
    let chart = layered_chart().await;
    let installed = chart.get_config().unwrap();
    for padding in [-0.1, f64::NAN] {
        let error = chart.fit_config(&installed, padding).unwrap_err();
        assert_eq!(code(&error), "FitPadding");
        let error = chart.fit_y_axes_config(&installed, padding).unwrap_err();
        assert_eq!(code(&error), "FitPadding");
    }
    let mut ghost = config_of(&chart);
    ghost.overlays[0].series.push("ghost".into());
    let ghost = serde_json::to_string(&ghost).unwrap();
    assert_eq!(
        code(&chart.fit_config(&ghost, 0.05).unwrap_err()),
        "LayerSeriesUnknown"
    );
    assert_eq!(
        code(&chart.fit_y_axes_config(&ghost, 0.05).unwrap_err()),
        "LayerSeriesUnknown"
    );
    assert!(chart.fit_config("{ not json", 0.05).is_err());
    assert_eq!(chart.get_config().unwrap(), installed);
}

fn size_domain(series: &SeriesConfig) -> (f64, f64) {
    match &series.render_type {
        DataRenderType::Xy(renderer::XySeries {
            scatter: Some(scatter),
            ..
        }) => {
            let map = scatter.point_size_map.as_ref().unwrap();
            (map.domain_min, map.domain_max)
        }
        other => panic!("not a scatter: {other:?}"),
    }
}

// The size domain takes the size column's extent; only that series
// changes, and a refusal changes nothing.
#[wasm_bindgen_test(async)]
async fn fit_point_size_domain_takes_the_column_extent() {
    let mut chart = layered_chart().await;
    let mut series = series_of(&chart);
    series.push(SeriesConfig::scatter("s", "t", "a").size_by("b", [0.0, 1.0], [2.0, 12.0]));
    series.push(SeriesConfig::scatter("f", "t", "a").size_by("flat", [0.0, 1.0], [2.0, 12.0]));
    chart
        .set_series(&serde_json::to_string(&series).unwrap())
        .unwrap();
    let before = series_of(&chart);
    chart.fit_point_size_domain("s").unwrap();
    let after = series_of(&chart);
    assert_eq!(size_domain(&after[2]), (100.0, 200.0));
    assert_eq!(after[..2], before[..2], "other series are untouched");
    assert_eq!(after[3], before[3]);

    for (id, expected) in [
        ("f", "PointSizeFitExtent"),
        ("a", "PointSizeFitNoMap"),
        ("missing", ""),
    ] {
        let error = chart.fit_point_size_domain(id).unwrap_err();
        if !expected.is_empty() {
            assert_eq!(code(&error), expected, "{id}");
        }
        assert_eq!(series_of(&chart), after, "{id}: unchanged");
    }
    for scale in [1.0, 2.0] {
        assert!(chart.export_png(scale).await.unwrap().length() > 0);
    }
}

fn vector_scale(series: &SeriesConfig) -> f64 {
    match &series.render_type {
        DataRenderType::Vector(VectorSeries { style, .. }) => match style.scale {
            VectorScale::Pixels { px_per_unit } => px_per_unit,
            VectorScale::Data { factor } => factor,
        },
        other => panic!("not a vector field: {other:?}"),
    }
}

// The vector scale fits points and grids to a target length; a refusal
// changes nothing.
#[wasm_bindgen_test(async)]
async fn fit_vector_scale_fits_points_and_grids() {
    let mut chart = layered_chart().await;
    let mut series = series_of(&chart);
    series.push(
        SeriesConfig::vectors("p", "t", "a", "u", "a")
            .vector_scale(VectorScale::Pixels { px_per_unit: 1.0 }),
    );
    series.push(
        SeriesConfig::vectors_grid(
            "g",
            "pair",
            "t",
            MatrixRef::columns_are_x(["a", "b"]),
            MatrixRef::columns_are_x(["t", "u"]),
        )
        .vector_scale(VectorScale::Data { factor: 1.0 }),
    );
    series.push(SeriesConfig::vectors("z", "t", "a", "zero", "zero"));
    chart
        .set_series(&serde_json::to_string(&series).unwrap())
        .unwrap();
    let before_png = chart.export_png(1.0).await.unwrap().to_vec();
    let before = series_of(&chart);
    chart.fit_vector_scale("p", 24.0).unwrap();
    let after = series_of(&chart);
    assert!((vector_scale(&after[2]) - 24.0 / 30f64.hypot(1.0)).abs() < 1e-12);
    assert_eq!(after[3], before[3], "only that series changes");
    chart.fit_vector_scale("g", 2.0).unwrap();
    let after = series_of(&chart);
    assert!((vector_scale(&after[3]) - 2.0 / 200f64.hypot(30.0)).abs() < 1e-12);
    assert_ne!(chart.export_png(1.0).await.unwrap().to_vec(), before_png);

    for (id, target, expected) in [
        ("p", 0.0, "VectorFitTarget"),
        ("p", f64::NAN, "VectorFitTarget"),
        ("a", 10.0, "VectorFitNotVector"),
        ("z", 10.0, "VectorFitExtent"),
        ("missing", 10.0, ""),
    ] {
        let error = chart.fit_vector_scale(id, target).unwrap_err();
        if !expected.is_empty() {
            assert_eq!(code(&error), expected, "{id}");
        }
        assert_eq!(series_of(&chart), after, "{id}: unchanged");
    }
    for scale in [1.0, 2.0] {
        assert!(chart.export_png(scale).await.unwrap().length() > 0);
    }
}

// A selected point of a size-mapped series reports the radius its value
// maps to, in logical pixels, and whether the size hides it.
#[wasm_bindgen_test(async)]
async fn selection_details_report_the_mapped_radius() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart
        .register_column_f32("x", &[0.0, 1.0, 2.0, 3.0])
        .unwrap();
    chart
        .register_column_f32("y", &[0.0, 1.0, 2.0, 3.0])
        .unwrap();
    chart
        .register_column_f32("size", &[100.0, 150.0, f32::NAN, 400.0])
        .unwrap();
    let series =
        [SeriesConfig::scatter("s", "x", "y").size_by("size", [100.0, 200.0], [2.0, 12.0])];
    chart
        .set_series(&serde_json::to_string(&series).unwrap())
        .unwrap();
    chart.fit_ranges().unwrap();
    let map = match &series[0].render_type {
        DataRenderType::Xy(renderer::XySeries {
            scatter: Some(scatter),
            ..
        }) => scatter.point_size_map.clone().unwrap(),
        _ => unreachable!(),
    };
    for (index, hidden) in [(0, false), (1, false), (2, true), (3, false)] {
        let mut config = config_of(&chart);
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::Point {
                source_id: None,
                series_id: "s".into(),
                point_index: index,
            }],
            ..Default::default()
        });
        chart
            .set_config(&serde_json::to_string(&config).unwrap())
            .unwrap();
        let details: serde_json::Value =
            serde_json::from_str(&chart.selection_details().await.unwrap()).unwrap();
        assert_eq!(details["size_hidden"], hidden, "{index}");
        let value = [100.0, 150.0, f64::NAN, 400.0][index];
        match map.radius_px(value) {
            Some(radius) => {
                let reported = details["mapped_radius_px"].as_f64().unwrap();
                assert!((reported - f64::from(radius)).abs() < 1e-4, "{index}");
            }
            None => assert!(details["mapped_radius_px"].is_null(), "{index}"),
        }
    }
    // The domain clamps: 400 draws at `max_px`, 100 at `min_px`.
    assert_eq!(map.radius_px(400.0), Some(12.0));
    assert_eq!(map.radius_px(100.0), Some(2.0));
}
