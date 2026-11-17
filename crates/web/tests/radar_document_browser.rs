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
    let polar = config.polar().unwrap();
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

// A document whose series styles cannot be created is rejected before the
// chart changes: the previous document stays committed (#152).
#[wasm_bindgen_test(async)]
async fn a_rejected_radar_document_keeps_the_previous_one() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_radar_document(DOCUMENT).unwrap();
    let config = chart.get_config().unwrap();
    let series = chart.get_series().unwrap();
    // A budget at the current usage leaves no room for the new styles.
    let status: serde_json::Value = serde_json::from_str(&chart.gpu_memory_status()).unwrap();
    let total = status["total_bytes"].as_f64().unwrap();
    chart.configure_auto_residency(total, total).unwrap();
    assert!(chart.set_radar_document(DOCUMENT).is_err());
    assert_eq!(chart.get_config().unwrap(), config);
    assert_eq!(chart.get_series().unwrap(), series);
}

/// The centre of a decoration's measured box, from `decoration_bounds`.
fn bounds(chart: &FiggyChart, id: &str) -> serde_json::Value {
    serde_json::from_str(&chart.decoration_bounds(id).unwrap()).unwrap()
}

fn centre(b: &serde_json::Value) -> (f32, f32) {
    let r = &b["rect"];
    let f = |i: usize| r[i].as_f64().unwrap() as f32;
    (f(0) + f(2) * 0.5, f(1) + f(3) * 0.5)
}

// The category labels of a radar document (#309): a moved spoke label opens
// as its category's own offset; each label answers the hit test and drags by
// itself; its bounds carry the dashed ring, and a press on the ring while the
// label is selected drags every label along its spoke.
#[wasm_bindgen_test(async)]
async fn radar_category_labels_move_alone_and_together_through_the_ring() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let mut document: serde_json::Value = serde_json::from_str(DOCUMENT).unwrap();
    document["decorations"] = serde_json::json!({
        "offsets": [{"element": {"SpokeLabel": {"category_id": "cost"}}, "offset": [6.0, 0.0]}]
    });
    chart.set_radar_document(&document.to_string()).unwrap();
    chart.frame().unwrap();
    let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let categories = config.bottom_x.categories.as_deref().unwrap();
    assert_ne!(categories[2].label_offset, [0.0, 0.0]);
    assert_eq!(categories[0].label_offset, [0.0, 0.0]);

    // A press on a label selects that label, which a drag moves alone.
    let label = bounds(&chart, "category_label_0");
    let (x, y) = centre(&label);
    assert_eq!(
        chart.hit_test(x, y).unwrap().as_deref(),
        Some("category_label_0")
    );
    assert!(chart.on_press(x, y).unwrap());
    chart.on_move(0.0, -8.0).unwrap();
    chart.on_release().unwrap();
    let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let categories = config.bottom_x.categories.as_deref().unwrap();
    assert_ne!(categories[0].label_offset, [0.0, 0.0]);
    assert_eq!(categories[1].label_offset, [0.0, 0.0]);
    assert_eq!(config.bottom_x.label_style.label_offset_y, 0.0);

    // The selected label's bounds carry the ring; a press on it grabs it and
    // the drag moves every label outward through the shared offset.
    let label = bounds(&chart, "category_label_0");
    let ring = &label["ring"];
    let center = [
        ring["center"][0].as_f64().unwrap() as f32,
        ring["center"][1].as_f64().unwrap() as f32,
    ];
    let radius = ring["radius"].as_f64().unwrap() as f32;
    // A point of the ring between two labels, straight down from the
    // centre-right diagonal, which no label covers on this four-spoke chart.
    let angle = std::f32::consts::FRAC_PI_4;
    let (rx, ry) = (
        center[0] + radius * angle.cos(),
        center[1] + radius * angle.sin(),
    );
    assert!(chart.on_press(rx, ry).unwrap());
    let (dx, dy) = (angle.cos() * 5.0, angle.sin() * 5.0);
    chart.on_move(dx, dy).unwrap();
    chart.on_release().unwrap();
    let moved: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    assert!((moved.bottom_x.label_style.label_offset_y - 5.0).abs() < 0.01);
    assert_eq!(
        moved.bottom_x.categories.as_deref().unwrap()[0].label_offset,
        config.bottom_x.categories.as_deref().unwrap()[0].label_offset
    );
    chart.frame().unwrap();
    let png = chart.export_png(1.0).await.unwrap();
    assert!(png.length() > 0);
}
