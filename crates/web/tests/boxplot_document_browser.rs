#![cfg(target_arch = "wasm32")]

//! A legacy box plot document opens in the Cartesian wrapper
//! (`FiggyChart::set_boxplot_document`): its boxes are `BoxPlot` series over
//! a categorical axis, reading a record and a ragged outlier column; a
//! selected box reports its category and statistics read back from the
//! record's GPU columns, `step_selection` walks the categories and the
//! series of the box group (boxes, never their outliers), and the chart
//! exports.

use figgy::FiggyChart;
use renderer::config::{DataSelectionsConfig, PickedDataRef};
use renderer::{Config, DataRenderType, SeriesConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const DOCUMENT: &str = r#"{
    "categories": [
        {"id": "ctl", "label": "Control"},
        {"id": "low", "label": "Low"},
        {"id": "high", "label": "High"}
    ],
    "series": [
        {"id": "a", "label": "A", "color": {"r": 0.25, "g": 0.56, "b": 0.82, "a": 1.0},
         "values": [
            {"whisker_low": 1, "q1": 2, "median": 3, "q3": 4, "whisker_high": 5,
             "mean": 3.2, "sample_count": 40, "outliers": [9.0]},
            null,
            {"whisker_low": 2, "q1": 3, "median": 4, "q3": 5, "whisker_high": 6}
         ]},
        {"id": "b", "label": "B", "color": {"r": 0.9, "g": 0.66, "b": 0.25, "a": 1.0},
         "values": [
            {"whisker_low": 0, "q1": 1, "median": 2, "q3": 3, "whisker_high": 4},
            {"whisker_low": 1, "q1": 2, "median": 2.5, "q3": 3, "whisker_high": 4},
            {"whisker_low": 1, "q1": 2, "median": 3, "q3": 4, "whisker_high": 5}
         ]}
    ],
    "value_title": "Response"
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
async fn a_box_plot_document_opens_as_box_plot_series() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_boxplot_document(DOCUMENT).unwrap();

    let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let series: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    assert_eq!(series.len(), 2);
    assert!(
        series
            .iter()
            .all(|s| matches!(s.render_type, DataRenderType::BoxPlot { .. }))
    );
    assert_eq!(config.bottom_x.category_count(), Some(3));
    // The value axis covers the whiskers and the outlier.
    assert!(config.left_y.min <= 0.0 && config.left_y.max >= 9.0);
    chart.frame().unwrap();

    select(&mut chart, "a", 0);
    let selected = details(&chart).await;
    assert_eq!(selected["category_id"], "ctl");
    assert_eq!(selected["median"], 3.0);
    assert_eq!(selected["sample_count"], 40.0);
    // Along the categories (a missing box included), then across the group.
    assert!(chart.step_selection(1, 0).unwrap());
    let selected = details(&chart).await;
    assert_eq!(selected["category_id"], "low");
    assert!(selected["median"].is_null());
    assert!(chart.step_selection(1, 0).unwrap());
    assert!(!chart.step_selection(1, 0).unwrap());
    assert!(chart.step_selection(0, 1).unwrap());
    let selected = details(&chart).await;
    assert_eq!(selected["series_id"], "b");
    assert_eq!(selected["q3"], 4.0);

    let png = chart.export_png(2.0).await.unwrap();
    assert!(png.length() > 0);

    // A second document replaces the first one's series and records.
    chart.set_boxplot_document(DOCUMENT).unwrap();
    let series: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    assert_eq!(series.len(), 2);
}
