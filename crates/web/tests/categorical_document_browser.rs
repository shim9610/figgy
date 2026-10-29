#![cfg(target_arch = "wasm32")]

//! A legacy categorical document opens in the Cartesian wrapper
//! (`FiggyChart::set_categorical_document`): its bars are `CategoryBars`
//! series over a categorical axis, a selected bar reports its category and
//! value read back from the GPU column, `step_selection` walks the
//! categories and the series of the bar group, and the chart exports.

use figgy::FiggyChart;
use renderer::config::{DataSelectionsConfig, PickedDataRef};
use renderer::{Config, DataRenderType, SeriesConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const DOCUMENT: &str = r#"{
    "categories": [
        {"id": "q1", "label": "Q1"},
        {"id": "q2", "label": "Q2"},
        {"id": "q3", "label": "Q3"}
    ],
    "series": [
        {"id": "north", "label": "North", "values": [12.0, 18.0, 9.0],
         "color": {"r": 0.33, "g": 0.56, "b": 0.83, "a": 1.0}},
        {"id": "south", "label": "South", "values": [18.0, null, 14.0],
         "color": {"r": 1.0, "g": 0.65, "b": 0.41, "a": 1.0}}
    ],
    "mode": "Stacked",
    "value_title": "Revenue"
}"#;

fn canvas() -> HtmlCanvasElement {
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

fn select(chart: &mut FiggyChart, series_id: &str, bin_index: usize) {
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

async fn details(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&chart.selection_details().await.unwrap()).unwrap()
}

#[wasm_bindgen_test(async)]
async fn a_categorical_document_opens_as_category_bars() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_categorical_document(DOCUMENT).unwrap();

    let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let series: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    assert_eq!(series.len(), 2);
    assert!(
        series
            .iter()
            .all(|s| matches!(s.render_type, DataRenderType::CategoryBars { .. }))
    );
    assert_eq!(config.bottom_x.category_count(), Some(3));
    assert_eq!(
        config.bar_groups.as_ref().map(|g| g.mode),
        Some(renderer::CategoryBarMode::Stacked)
    );
    // The value axis covers the largest stack, 30.
    assert!(config.left_y.min <= 0.0 && config.left_y.max >= 30.0);
    chart.frame().unwrap();

    select(&mut chart, "south", 0);
    let selected = details(&chart).await;
    assert_eq!(selected["category_id"], "q1");
    assert_eq!(selected["value"], 18.0);
    assert_eq!(selected["mode"], "Stacked");
    // Along the categories, then across the bar group.
    assert!(chart.step_selection(1, 0).unwrap());
    let selected = details(&chart).await;
    assert_eq!(selected["category_id"], "q2");
    assert!(selected["value"].is_null() || selected["value"].as_f64().unwrap().is_nan());
    assert!(chart.step_selection(1, 0).unwrap());
    assert!(!chart.step_selection(1, 0).unwrap());
    assert!(chart.step_selection(0, -1).unwrap());
    let selected = details(&chart).await;
    assert_eq!(selected["series_id"], "north");
    assert_eq!(selected["value"], 9.0);

    let png = chart.export_png(2.0).await.unwrap();
    assert!(png.length() > 0);

    // A second document replaces the first one's series and columns.
    chart.set_categorical_document(DOCUMENT).unwrap();
    let series: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    assert_eq!(series.len(), 2);
}

/// A value label is a drag target: the document's labels are written,
/// `press_value_label` selects one by its `label_id` (an id naming no label
/// changes nothing), `on_move` writes the series' `labels.offsets`,
/// `step_selection` does not step it and `selection_details` reports its
/// category.
#[wasm_bindgen_test(async)]
async fn a_value_label_picks_selects_and_drags() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_categorical_document(DOCUMENT).unwrap();
    chart.frame().unwrap();
    let series: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    // The reader wrote one string per category.
    assert_eq!(series[0].value_label_text().unwrap().len(), 3);

    assert!(!chart.press_value_label("axis_left").unwrap());
    assert!(!chart.press_value_label("series:north:label:9").unwrap());
    assert!(chart.press_value_label("series:north:label:1").unwrap());
    let selected = details(&chart).await;
    assert_eq!(selected["category_id"], "q2");
    assert_eq!(selected["label_id"], "series:north:label:1");
    assert!(!chart.step_selection(1, 0).unwrap());

    chart.on_move(6.0, -4.0).unwrap();
    chart.on_release();
    let series: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    let [dx, dy] = series[0].value_label_offset(1);
    assert!(dx > 0.0 && dy < 0.0, "the drag wrote ({dx}, {dy})");
    assert_eq!(series[0].value_label_offset(0), [0.0, 0.0]);
    // After the release a move drags nothing.
    chart.on_move(6.0, -4.0).unwrap();
    let after: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    assert_eq!(after, series);
    chart.frame().unwrap();
}
