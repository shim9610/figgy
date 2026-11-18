#![cfg(target_arch = "wasm32")]

//! The model API for editors (#314) in the browser: `default_config`,
//! `validate_config`, `normalize_config` and the legacy document converters
//! are free functions of JSON that need no chart, no canvas and no GPU, and
//! each refusal reaches JavaScript as the bindings document it.

use figgy::{
    convert_boxplot_document, convert_categorical_document, convert_radar_document, default_config,
    normalize_config, validate_config,
};
use renderer::{Config, SeriesConfig};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const COLOR: &str = r#"{"r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0}"#;

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

/// The `code` of a thrown `Error`, `None` for a thrown string.
fn code(error: &JsValue) -> Option<String> {
    let error = error.dyn_ref::<js_sys::Error>()?;
    js_sys::Reflect::get(error, &JsValue::from_str("code"))
        .ok()?
        .as_string()
}

fn note_paths(converted: &serde_json::Value) -> Vec<(String, String)> {
    converted["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            assert!(!n["reason"].as_str().unwrap().is_empty());
            (
                n["path"].as_str().unwrap().to_owned(),
                n["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[wasm_bindgen_test]
fn the_default_config_is_valid_and_already_normalized() {
    let text = default_config().unwrap();
    let config: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(config, renderer::default::default_config());
    assert_eq!(
        json(&validate_config(&text, None).unwrap()),
        serde_json::json!({ "valid": true, "errors": [] })
    );
    assert_eq!(normalize_config(&text).unwrap(), text);
}

#[wasm_bindgen_test]
fn validate_config_returns_the_refused_field_without_throwing() {
    let mut config = renderer::default::default_config();
    config.bottom_x.min = 5.0;
    config.bottom_x.max = 5.0;
    let report = json(&validate_config(&serde_json::to_string(&config).unwrap(), None).unwrap());
    assert_eq!(report["valid"], false);
    assert_eq!(report["errors"][0]["stage"], "chart");
    assert_eq!(report["errors"][0]["path"], "bottom_x");
    assert_eq!(report["errors"][0]["code"], "AxisBoundsOrder");

    let config = default_config().unwrap();
    let series = serde_json::to_string(&[
        SeriesConfig::line("a", "x", "y"),
        SeriesConfig::line("", "x", "y"),
    ])
    .unwrap();
    let report = json(&validate_config(&config, Some(series)).unwrap());
    assert_eq!(report["errors"][0]["stage"], "series");
    assert_eq!(report["errors"][0]["code"], "SeriesIdEmpty");
    assert_eq!(report["errors"][0]["path"], "series[1]");

    let report = json(&validate_config("{", None).unwrap());
    assert_eq!(report["errors"][0]["stage"], "parse");
    assert_eq!(report["errors"][0]["code"], "json");
    assert!(report["errors"][0]["line"].is_u64());
    let report = json(&validate_config(&config, Some("{}".into())).unwrap());
    assert_eq!(report["errors"][0]["path"], "series");
}

#[wasm_bindgen_test]
fn normalize_config_keeps_every_bit_and_throws_on_bad_json() {
    let mut config = renderer::default::default_config();
    config.left_y.min = 0.1 + 0.2;
    config.left_y.max = 2.0_f64.sqrt();
    let pretty = serde_json::to_string_pretty(&config).unwrap();
    let normalized = normalize_config(&pretty).unwrap();
    assert_eq!(normalized, serde_json::to_string(&config).unwrap());
    let back: Config = serde_json::from_str(&normalized).unwrap();
    assert_eq!(back.left_y.min.to_bits(), (0.1_f64 + 0.2).to_bits());
    assert_eq!(back.left_y.max.to_bits(), 2.0_f64.sqrt().to_bits());

    let error = normalize_config("not json").unwrap_err();
    assert!(
        error.as_string().is_some(),
        "a JSON error throws its message"
    );
    // A non-finite number is `null` in JSON, which an f64 field refuses.
    let mut value = serde_json::to_value(&config).unwrap();
    value["left_y"]["max"] = serde_json::Value::Null;
    assert!(normalize_config(&value.to_string()).is_err());
}

#[wasm_bindgen_test]
fn a_categorical_document_converts_with_its_notes() {
    let document = format!(
        r#"{{
            "categories": [{{"id": "a", "label": "A"}}, {{"id": "b", "label": "B"}}],
            "series": [{{"id": "s", "label": "S", "values": [2, null], "color": {COLOR}}}],
            "hovered": {{"category_id": "a", "series_id": "s"}},
            "decorations": {{
                "data_area": {{"x": 20, "y": 20, "width": 300, "height": 200}},
                "offsets": [{{"element": {{"SpokeLabel": {{"category_id": "a"}}}}, "offset": [3, 4]}}],
                "selected": null
            }}
        }}"#
    );
    let converted = json(&convert_categorical_document(&document).unwrap());
    let config: Config = serde_json::from_value(converted["config"].clone()).unwrap();
    let series: Vec<SeriesConfig> = serde_json::from_value(converted["series"].clone()).unwrap();
    assert_eq!(config.validate_category_bars(&series), Ok(()));
    assert_eq!(
        converted["uploads"],
        serde_json::json!([
            {"column": "categorical.positions", "role": "positions", "len": 2},
            {"column": "categorical.values.s", "role": "values", "len": 2},
        ])
    );
    assert_eq!(converted["columns"][1]["series_id"], "s");
    assert_eq!(
        note_paths(&converted),
        [
            ("hovered".to_owned(), "dropped".to_owned()),
            ("decorations.offsets[0]".into(), "dropped".into()),
            // Too tight for the title band, so the automatic layout stays
            // (#334); fitted last, its note comes last.
            ("decorations.data_area".into(), "dropped".into()),
        ]
    );

    // A document rule throws an `Error` with the rule's code; text that
    // does not parse as a document throws its message.
    let refused = document.replace("\"hovered\"", "\"bar_gap\": -1, \"hovered\"");
    let error = convert_categorical_document(&refused).unwrap_err();
    assert!(code(&error).is_some(), "{error:?}");
    let error = convert_categorical_document("{\"categories\":").unwrap_err();
    assert_eq!(code(&error), None);
    assert!(error.as_string().is_some());
}

#[wasm_bindgen_test]
fn a_box_plot_document_converts_with_its_record_and_outliers() {
    let document = format!(
        r#"{{
            "categories": [{{"id": "a", "label": "A"}}, {{"id": "b", "label": "B"}}],
            "series": [{{"id": "s", "label": "S", "color": {COLOR}, "values": [
                {{"whisker_low": 1, "q1": 2, "median": 3, "q3": 4, "whisker_high": 5,
                  "outliers": [0.5, 9]}},
                {{"whisker_low": 1, "q1": 2, "median": 2.5, "q3": 3, "whisker_high": 4}}
            ]}}],
            "background": {{"r": 0, "g": 0, "b": 0, "a": 1}}
        }}"#
    );
    let converted = json(&convert_boxplot_document(&document).unwrap());
    let roles: Vec<_> = converted["uploads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["positions", "box_record", "box_outliers"]);
    assert_eq!(converted["uploads"][1]["rows"], 2);
    assert_eq!(converted["uploads"][2]["len"], 2);
    assert_eq!(converted["uploads"][2]["groups"], 2);
    assert_eq!(
        converted["columns"][2],
        serde_json::json!({"series_id": "s", "role": "box_outliers", "column": "boxplot.outliers.s"})
    );
    assert_eq!(
        note_paths(&converted),
        [("background".to_owned(), "dropped".to_owned())]
    );
    assert!(convert_boxplot_document("{\"series\": 3}").is_err());
}

#[wasm_bindgen_test]
fn a_radar_document_converts_with_its_notes() {
    let document = format!(
        r#"{{
            "categories": [{{"id": "a", "label": "A"}}, {{"id": "b", "label": "B"}},
                           {{"id": "c", "label": "C"}}],
            "series": [{{"id": "r", "label": "R", "values": [1, 2, 3], "color": {COLOR}}}],
            "selected": {{"series_id": "r"}},
            "decorations": {{
                "data_area": null,
                "offsets": [{{"element": {{"SpokeLabel": {{"category_id": "b"}}}}, "offset": [2, 2]}}],
                "selected": null
            }}
        }}"#
    );
    let converted = json(&convert_radar_document(&document).unwrap());
    let config: Config = serde_json::from_value(converted["config"].clone()).unwrap();
    assert!(config.polar().is_some());
    assert_eq!(converted["uploads"][1]["len"], 3);
    assert_eq!(
        converted["columns"][1],
        serde_json::json!({"series_id": "r", "role": "values", "column": "radar.values.r"})
    );
    assert_eq!(
        note_paths(&converted),
        // The spoke label offset is carried as category "b"'s own label
        // offset (#309), so only the series selection is dropped.
        [("selected".to_owned(), "dropped".to_owned())]
    );
    assert_ne!(
        config.bottom_x.categories.as_deref().unwrap()[1].label_offset,
        [0.0, 0.0]
    );
    // A non-finite value cannot be written in JSON: `null` is refused.
    let refused = document.replace("[1, 2, 3]", "[1, null, 3]");
    assert!(convert_radar_document(&refused).is_err());
}
