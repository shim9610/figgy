#![cfg(target_arch = "wasm32")]
//! `selection_details` on an energy-level label (#318): the label is found
//! in the series' own rows by `part` and `category_index`, never in the
//! categorical `labels.text`.

use figgy::FiggyChart;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

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

/// A rich label, serialized as the model writes it.
fn label(text: &str) -> serde_json::Value {
    serde_json::to_value(renderer::text::RichText::plain(
        text,
        renderer::Color::BLACK,
        14.0,
        "",
    ))
    .unwrap()
}

/// Two levels in columns `S` and `P`, one transition between them, one
/// limit and one band, every row labelled.
fn levels() -> serde_json::Value {
    serde_json::json!([{
        "series_id": "levels",
        "x_column": "",
        "y_column": "",
        "render_type": {"EnergyLevels": {
            "levels": [
                {"id": "a", "column": "S", "energy": 0, "label": label("ground")},
                {"id": "b", "column": "P", "energy": 3, "label": label("excited"),
                 "label_offset": [4.0, -2.0]},
                {"id": "c", "column": "P", "energy": 4}
            ],
            "transitions": [
                {"id": "t", "lower": "a", "upper": "b", "label": label("589 nm")}
            ],
            "limits": [{"id": "ion", "energy": 5, "label": label("limit")}],
            "bands": [{"id": "pb", "column": "P", "first": "b", "last": "c",
                       "label": label("band")}]
        }}
    }])
}

async fn details(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&chart.selection_details().await.unwrap()).unwrap()
}

fn select(chart: &mut FiggyChart, index: usize, part: &str) {
    chart
        .set_picked_data(
            &serde_json::json!({"refs": [{"kind": "value_label", "series_id": "levels",
                "category_index": index, "part": part}]})
            .to_string(),
        )
        .unwrap();
}

#[wasm_bindgen_test(async)]
async fn an_energy_level_label_reports_its_row_for_every_part() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let mut config: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config["bottom_x"]["categories"] = serde_json::json!([
        {"id": "S", "label": "S"},
        {"id": "P", "label": "P"}
    ]);
    chart.set_config(&config.to_string()).unwrap();
    chart.set_series(&levels().to_string()).unwrap();
    chart.frame().unwrap();
    chart.first_frame_ready().await.unwrap();

    // The issue's case: the second level, part "level".
    select(&mut chart, 1, "level");
    let row = details(&chart).await;
    assert_eq!(row["part"], "level", "{row}");
    assert_eq!(row["index"], 1);
    assert_eq!(row["id"], "b");
    assert_eq!(row["text"], "excited");
    assert_eq!(row["offset"], serde_json::json!([4.0, -2.0]));

    // Transition, limit and band labels use their own lists.
    for (part, id, text) in [
        ("transition", "t", "589 nm"),
        ("limit", "ion", "limit"),
        ("band", "pb", "band"),
    ] {
        select(&mut chart, 0, part);
        let row = details(&chart).await;
        assert_eq!(row["part"], part, "{row}");
        assert_eq!(row["id"], id);
        assert_eq!(row["text"], text);
    }

    // A level without a label reports a null text, not an error.
    select(&mut chart, 2, "level");
    let row = details(&chart).await;
    assert_eq!(row["id"], "c", "{row}");
    assert!(row["text"].is_null());
    assert!(row["label"].is_null());
}
