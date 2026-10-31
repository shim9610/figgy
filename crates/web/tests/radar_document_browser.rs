#![cfg(target_arch = "wasm32")]

//! A legacy radar document opens in the Cartesian wrapper
//! (`FiggyChart::set_radar_document`): its polygons are `Radar` series on a
//! polar chart whose angle axis holds the categories; a selected vertex
//! reports its category and value read back from the GPU columns,
//! `step_selection` walks the vertices along the categories and across the
//! series, and the chart exports.

use figgy::FiggyChart;
use renderer::config::{DataSelectionsConfig, PickedDataRef};
use renderer::{Config, DataRenderType, GridShape, SeriesConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const DOCUMENT: &str = r#"{
    "categories": [
        {"id": "speed", "label": "Speed"},
        {"id": "range", "label": "Range"},
        {"id": "cost", "label": "Cost"},
        {"id": "comfort", "label": "Comfort"}
    ],
    "series": [
        {"id": "a", "label": "A", "values": [4, 3.5, 2, 4.5],
         "color": {"r": 0.25, "g": 0.56, "b": 0.82, "a": 1.0}},
        {"id": "b", "label": "B", "values": [3, 5, 3.25, 3],
         "color": {"r": 0.9, "g": 0.66, "b": 0.25, "a": 1.0}}
    ],
    "selected": {"series_id": "a", "category_id": "range"}
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
    canvas.set_height(360);
    canvas
}

fn select(chart: &mut FiggyChart, series_id: &str, point_index: usize) {
    let mut config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: series_id.into(),
            point_index,
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
async fn a_radar_document_opens_as_radar_series_on_a_polar_chart() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_radar_document(DOCUMENT).unwrap();

    let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let series: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    assert_eq!(series.len(), 2);
    assert!(
        series
            .iter()
            .all(|s| matches!(s.render_type, DataRenderType::Radar { .. }))
    );
    assert_eq!(config.bottom_x.category_count(), Some(4));
    let polar = config.polar.as_ref().unwrap();
    assert_eq!(polar.grid_shape, Some(GridShape::Polygon));
    // The radius axis is fitted from zero over every value.
    assert!(config.left_y.min == 0.0 && config.left_y.max >= 5.0);
    chart.frame().unwrap();

    // The document's selected vertex is the typed selection.
    let selected = details(&chart).await;
    assert_eq!(selected["category_id"], "range");
    assert_eq!(selected["y"], 3.5);
    select(&mut chart, "a", 0);
    let selected = details(&chart).await;
    assert_eq!(selected["category_label"], "Speed");
    assert_eq!(selected["y"], 4.0);
    // Along the vertices, then across the series.
    assert!(chart.step_selection(1, 0).unwrap());
    assert_eq!(details(&chart).await["category_id"], "range");
    assert!(chart.step_selection(0, 1).unwrap());
    let selected = details(&chart).await;
    assert_eq!(selected["series_id"], "b");
    assert_eq!(selected["y"], 5.0);

    let png = chart.export_png(2.0).await.unwrap();
    assert!(png.length() > 0);

    // A second document replaces the first one's series and columns.
    chart.set_radar_document(DOCUMENT).unwrap();
    let series: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    assert_eq!(series.len(), 2);
}
