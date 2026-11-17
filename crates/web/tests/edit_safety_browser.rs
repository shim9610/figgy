#![cfg(target_arch = "wasm32")]

//! Edit safety of the browser `FiggyChart` (#311):
//!
//! - `clear_selection` ends the element and annotation selections and any
//!   drag, and keeps the config, the series and the data highlight;
//! - the legacy document calls replace the chart atomically: a failure at
//!   any stage leaves the config, series, columns, selection and image as
//!   they were and names the stage, and a success alternates the column ids
//!   and removes the previous document's columns;
//! - a chunked column upload (`begin_column_upload`) refuses overlapping,
//!   gapped, oversized and mistyped chunks, incomplete and stale commits,
//!   a commit on another chart and a budget that cannot hold it, and
//!   installs the column at once on commit;
//! - `inspect_streaming_capabilities` checks a candidate without changing
//!   the chart.

use figgy::FiggyChart;
use renderer::config::{DataSelectionsConfig, PickedDataRef};
use renderer::{Config, OverlayLayer, SeriesConfig};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const RADAR: &str = r#"{
    "categories": [
        {"id": "speed", "label": "Speed"},
        {"id": "range", "label": "Range"},
        {"id": "cost", "label": "Cost"}
    ],
    "series": [
        {"id": "a", "label": "A", "values": [4, 3.5, 2],
         "color": {"r": 0.25, "g": 0.56, "b": 0.82, "a": 1.0}}
    ]
}"#;

const CATEGORICAL: &str = r#"{
    "categories": [
        {"id": "q1", "label": "Q1"},
        {"id": "q2", "label": "Q2"}
    ],
    "series": [
        {"id": "north", "label": "North", "values": [12.0, 18.0],
         "color": {"r": 0.33, "g": 0.56, "b": 0.83, "a": 1.0}},
        {"id": "south", "label": "South", "values": [18.0, 14.0],
         "color": {"r": 1.0, "g": 0.65, "b": 0.41, "a": 1.0}}
    ]
}"#;

const BOXPLOT: &str = r#"{
    "categories": [
        {"id": "ctl", "label": "Control"},
        {"id": "low", "label": "Low"}
    ],
    "series": [
        {"id": "a", "label": "A", "color": {"r": 0.25, "g": 0.56, "b": 0.82, "a": 1.0},
         "values": [
            {"whisker_low": 1, "q1": 2, "median": 3, "q3": 4, "whisker_high": 5,
             "outliers": [9.0]},
            {"whisker_low": 2, "q1": 3, "median": 4, "q3": 5, "whisker_high": 6}
         ]}
    ]
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

fn config(chart: &FiggyChart) -> Config {
    serde_json::from_str(&chart.get_config().unwrap()).unwrap()
}

fn series(chart: &FiggyChart) -> Vec<SeriesConfig> {
    serde_json::from_str(&chart.get_series().unwrap()).unwrap()
}

/// The ids of every registered column, sorted.
fn column_ids(chart: &FiggyChart) -> Vec<String> {
    let status: serde_json::Value = serde_json::from_str(&chart.gpu_memory_status()).unwrap();
    let mut ids: Vec<String> = status["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|source| source["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

fn total_bytes(chart: &FiggyChart) -> f64 {
    let status: serde_json::Value = serde_json::from_str(&chart.gpu_memory_status()).unwrap();
    status["total_bytes"].as_f64().unwrap()
}

/// The `field` of an `Error` a call threw, as a string.
fn error_field(error: &JsValue, field: &str) -> Option<String> {
    js_sys::Reflect::get(error, &JsValue::from_str(field))
        .ok()
        .and_then(|value| value.as_string())
}

fn error_message(error: &JsValue) -> String {
    error
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| error.as_string())
        .unwrap_or_default()
}

/// Everything a failed edit must leave as it was.
struct Snapshot {
    config: String,
    series: String,
    columns: Vec<String>,
    has_selection: bool,
    png: Vec<u8>,
}

async fn snapshot(chart: &mut FiggyChart) -> Snapshot {
    Snapshot {
        config: chart.get_config().unwrap(),
        series: chart.get_series().unwrap(),
        columns: column_ids(chart),
        has_selection: chart.has_selection().unwrap(),
        png: chart.export_png(1.0).await.unwrap().to_vec(),
    }
}

async fn assert_unchanged(chart: &mut FiggyChart, before: &Snapshot, what: &str) {
    assert_eq!(chart.get_config().unwrap(), before.config, "{what}: config");
    assert_eq!(chart.get_series().unwrap(), before.series, "{what}: series");
    assert_eq!(column_ids(chart), before.columns, "{what}: columns");
    assert_eq!(
        chart.has_selection().unwrap(),
        before.has_selection,
        "{what}: selection"
    );
    assert_eq!(
        chart.export_png(1.0).await.unwrap().to_vec(),
        before.png,
        "{what}: image"
    );
}

// ---- clear_selection ----

#[wasm_bindgen_test(async)]
async fn clear_selection_ends_element_and_annotation_selection_and_keeps_the_data_highlight() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.load_demo().unwrap();
    chart.set_title("Clear me").unwrap();
    chart.frame().unwrap();

    // A data highlight on the first demo series.
    let first = series(&chart)[0].series_id.clone();
    let mut with_pick = config(&chart);
    with_pick.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: first,
            point_index: 3,
        }],
        ..Default::default()
    });
    chart
        .set_config(&serde_json::to_string(&with_pick).unwrap())
        .unwrap();

    // Press the title: it is selected and drags.
    let title = (240.0, 16.0);
    assert_eq!(
        chart.hit_test(title.0, title.1).unwrap().as_deref(),
        Some("chart_title")
    );
    assert!(chart.on_press(title.0, title.1).unwrap());
    assert!(chart.has_selection().unwrap());
    let before = (chart.get_config().unwrap(), chart.get_series().unwrap());
    let layer = config(&chart).active_layer;

    chart.clear_selection().unwrap();
    assert!(!chart.has_selection().unwrap());
    // The drag ended with it: a move changes nothing.
    chart.on_move(20.0, 10.0).unwrap();
    assert_eq!(chart.get_config().unwrap(), before.0);
    assert_eq!(chart.get_series().unwrap(), before.1);
    let after = config(&chart);
    assert_eq!(after.active_layer, layer);
    assert_eq!(after.picked_data, with_pick.picked_data);

    // An annotation selection goes too.
    let item = renderer::Annotation::new(renderer::AnnotationKind::Text {
        anchor: renderer::Anchor::Ratio { x: 0.4, y: 0.4 },
        text: renderer::text::RichText::plain("note", renderer::Color::BLACK, 14.0, ""),
        box_style: None,
        align: Default::default(),
    });
    let index = chart
        .add_annotation(&serde_json::to_string(&item).unwrap())
        .unwrap();
    chart.select_annotation(Some(index)).unwrap();
    assert_eq!(chart.selected_annotation(), Some(index));
    chart.clear_selection().unwrap();
    assert_eq!(chart.selected_annotation(), None);
    assert!(!chart.has_selection().unwrap());
    // It redraws, and a second call is harmless.
    chart.frame().unwrap();
    chart.clear_selection().unwrap();
    assert_eq!(config(&chart).picked_data, with_pick.picked_data);
}

// ---- atomic document replace ----

#[wasm_bindgen_test(async)]
async fn a_document_that_does_not_validate_changes_nothing_and_names_the_stage() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_radar_document(RADAR).unwrap();
    let before = snapshot(&mut chart).await;
    for (call, error) in [
        ("radar", chart.set_radar_document("{").unwrap_err()),
        (
            "categorical",
            chart.set_categorical_document("{").unwrap_err(),
        ),
        ("boxplot", chart.set_boxplot_document("[]").unwrap_err()),
    ] {
        assert_eq!(
            error_field(&error, "stage").as_deref(),
            Some("validate"),
            "{call}"
        );
        assert!(
            error_message(&error).contains("validate failed"),
            "{call}: {}",
            error_message(&error)
        );
    }
    assert_unchanged(&mut chart, &before, "malformed documents").await;

    // The documented and the alternate id both held by host columns.
    let mut fresh = FiggyChart::create(canvas()).await.unwrap();
    fresh
        .register_column_f64("radar.positions", &[0.0, 1.0])
        .unwrap();
    fresh
        .register_column_f64("radar.positions#alt", &[0.0, 1.0])
        .unwrap();
    let before = snapshot(&mut fresh).await;
    let error = fresh.set_radar_document(RADAR).unwrap_err();
    assert_eq!(error_field(&error, "stage").as_deref(), Some("validate"));
    assert_unchanged(&mut fresh, &before, "held ids").await;
}

#[wasm_bindgen_test(async)]
async fn a_document_whose_columns_do_not_fit_the_budget_changes_nothing() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_boxplot_document(BOXPLOT).unwrap();
    // A point highlight and an element selection the failure must keep.
    chart.on_press(240.0, 16.0).unwrap();
    let before = snapshot(&mut chart).await;

    // The same series with three million outliers: the styles fit the
    // margin, the outlier column (24 MB) does not. It must also be larger
    // than the pool's free capacity (16 MiB at creation), or it is placed
    // without growing the pool and the budget never sees it.
    let outliers = (0..3_000_000)
        .map(|i| format!("{}", 100.0 + i as f64 * 1e-3))
        .collect::<Vec<_>>()
        .join(",");
    let large = BOXPLOT.replace("[9.0]", &format!("[{outliers}]"));
    let total = total_bytes(&chart);
    let margin = 2.0 * 1024.0 * 1024.0;
    chart
        .configure_auto_residency(total + margin, total + margin)
        .unwrap();
    let error = chart.set_boxplot_document(&large).unwrap_err();
    assert_eq!(
        error_field(&error, "stage").as_deref(),
        Some("upload"),
        "{}",
        error_message(&error)
    );
    assert!(
        matches!(
            error_field(&error, "code").as_deref(),
            Some("budget_exceeded" | "pool_space" | "allocation_failed")
        ),
        "{:?}",
        error_field(&error, "code")
    );
    assert!(error_message(&error).contains("boxplot document upload failed"));
    assert_unchanged(&mut chart, &before, "upload failure").await;

    // With room again the same document opens.
    chart.configure_auto_residency(1e12, 1e12).unwrap();
    chart.set_boxplot_document(&large).unwrap();
}

#[wasm_bindgen_test(async)]
async fn a_failed_fit_changes_nothing() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_categorical_document(CATEGORICAL).unwrap();
    let before = snapshot(&mut chart).await;
    // Every value missing: the value axis has nothing to fit. Whether the
    // fit refuses it or keeps a default range, a failure must change
    // nothing and name its stage.
    let empty = CATEGORICAL
        .replace("[12.0, 18.0]", "[null, null]")
        .replace("[18.0, 14.0]", "[null, null]");
    match chart.set_categorical_document(&empty) {
        Ok(()) => assert_eq!(series(&chart).len(), 2),
        Err(error) => {
            assert!(
                matches!(
                    error_field(&error, "stage").as_deref(),
                    Some("fit" | "validate")
                ),
                "{}",
                error_message(&error)
            );
            assert_unchanged(&mut chart, &before, "fit failure").await;
        }
    }
}

#[wasm_bindgen_test(async)]
async fn a_replaced_document_alternates_its_ids_and_removes_the_previous_columns() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.register_column_f64("host", &[1.0, 2.0]).unwrap();

    chart.set_radar_document(RADAR).unwrap();
    assert_eq!(series(&chart)[0].x_column, "radar.positions");
    assert_eq!(
        column_ids(&chart),
        ["host", "radar.positions", "radar.values.a"]
    );

    // The second document uploads beside the first and replaces it.
    chart.set_radar_document(RADAR).unwrap();
    let second = series(&chart);
    assert_eq!(second[0].x_column, "radar.positions#alt");
    assert_eq!(second[0].y_column, "radar.values.a#alt");
    assert_eq!(
        column_ids(&chart),
        ["host", "radar.positions#alt", "radar.values.a#alt"]
    );

    // The third goes back to the documented ids.
    chart.set_radar_document(RADAR).unwrap();
    assert_eq!(series(&chart)[0].x_column, "radar.positions");
    assert_eq!(
        column_ids(&chart),
        ["host", "radar.positions", "radar.values.a"]
    );

    // A document of another family drops the radar columns with its series.
    chart.set_categorical_document(CATEGORICAL).unwrap();
    assert_eq!(
        column_ids(&chart),
        [
            "categorical.positions",
            "categorical.values.north",
            "categorical.values.south",
            "host",
        ]
    );
    chart.frame().unwrap();
    assert!(!chart.export_png(2.0).await.unwrap().to_vec().is_empty());
}

// ---- chunked column upload ----

#[wasm_bindgen_test(async)]
async fn a_chunked_upload_installs_the_column_on_commit() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    // Past any size a fixed chunk would hold.
    let n = 200_000usize;
    let values: Vec<f64> = (0..n).map(|i| (i as f64 * 0.001).sin()).collect();
    let mut upload = chart
        .begin_column_upload("wave", n as f64, "f64", None, 131_072.0)
        .unwrap();
    assert!(upload.is_open());
    assert_eq!(upload.length(), n as f64);
    for (index, part) in values.chunks(16_384).enumerate() {
        upload
            .push(
                (index * 16_384) as f64,
                js_sys::Float64Array::from(part).into(),
            )
            .unwrap();
    }
    assert_eq!(upload.written(), n as f64);
    assert!(
        chart.column_revision("wave").is_none(),
        "nothing before commit"
    );
    upload.commit(&mut chart).unwrap();
    assert!(!upload.is_open());
    let revision = chart.column_revision("wave");
    assert!(revision.is_some());

    // A series reads every value of it.
    let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
    chart.register_column_f64("x", &x).unwrap();
    let line = SeriesConfig::line("wave", "x", "wave");
    chart
        .set_series(&serde_json::to_string(&[line]).unwrap())
        .unwrap();
    chart.frame().unwrap();
    let status: serde_json::Value = serde_json::from_str(&chart.gpu_memory_status()).unwrap();
    let wave = status["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["id"] == "wave")
        .unwrap()
        .clone();
    assert_eq!(wave["length"].as_u64(), Some(n as u64));

    // A replacement names the revision it replaces; f32 chunks work too.
    let mut again = chart
        .begin_column_upload("wave", 4.0, "f32", revision, 64.0)
        .unwrap();
    again
        .push(0.0, js_sys::Float32Array::from(&[1.0f32, 2.0][..]).into())
        .unwrap();
    again
        .push(2.0, js_sys::Float32Array::from(&[3.0f32, 4.0][..]).into())
        .unwrap();
    again.commit(&mut chart).unwrap();
    assert_ne!(chart.column_revision("wave"), revision);
    chart.frame().unwrap();
}

#[wasm_bindgen_test(async)]
async fn refused_chunks_and_commits_change_nothing() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let chunk = |values: &[f64]| -> JsValue { js_sys::Float64Array::from(values).into() };
    let code = |error: JsValue| error_field(&error, "code");

    let mut upload = chart
        .begin_column_upload("v", 6.0, "f64", None, 32.0)
        .unwrap();
    upload.push(0.0, chunk(&[0.0; 4])).unwrap();
    // Overlap, gap, past the end, the other array type, too many bytes, and
    // something that is not a typed array.
    for (what, error) in [
        ("overlap", upload.push(2.0, chunk(&[1.0; 2])).unwrap_err()),
        ("gap", upload.push(6.0, chunk(&[1.0; 2])).unwrap_err()),
        ("past end", upload.push(4.0, chunk(&[1.0; 3])).unwrap_err()),
        (
            "type",
            upload
                .push(4.0, js_sys::Float32Array::from(&[1.0f32; 2][..]).into())
                .unwrap_err(),
        ),
        ("bytes", upload.push(4.0, chunk(&[1.0; 5])).unwrap_err()),
        (
            "array",
            upload.push(4.0, JsValue::from_str("1,2")).unwrap_err(),
        ),
    ] {
        assert_eq!(code(error).as_deref(), Some("invalid_chunk"), "{what}");
    }
    assert_eq!(upload.written(), 4.0);
    assert!(upload.is_open());

    // An incomplete commit is refused and closes the upload.
    let error = upload.commit(&mut chart).unwrap_err();
    assert_eq!(code(error).as_deref(), Some("invalid_chunk"));
    assert!(!upload.is_open());
    assert!(chart.column_revision("v").is_none());
    let error = upload.push(4.0, chunk(&[1.0; 2])).unwrap_err();
    assert_eq!(code(error).as_deref(), Some("upload_closed"));
    upload.cancel();

    // Stale at begin: a registered id without its revision, a revision the
    // id does not hold.
    chart.register_column_f64("v", &[1.0, 2.0]).unwrap();
    let revision = chart.column_revision("v");
    let error = chart
        .begin_column_upload("v", 2.0, "f64", None, 64.0)
        .err()
        .unwrap();
    assert_eq!(code(error).as_deref(), Some("stale"));
    let error = chart
        .begin_column_upload("w", 2.0, "f64", Some(7.0), 64.0)
        .err()
        .unwrap();
    assert_eq!(code(error).as_deref(), Some("stale"));
    assert!(
        chart
            .begin_column_upload("w", 2.0, "f16", None, 64.0)
            .is_err()
    );

    // Stale at commit: the column changed while the upload was open.
    let mut upload = chart
        .begin_column_upload("v", 2.0, "f64", revision, 64.0)
        .unwrap();
    upload.push(0.0, chunk(&[7.0, 8.0])).unwrap();
    chart.update_register_column_f64("v", &[3.0, 4.0]).unwrap();
    let current = chart.column_revision("v");
    let error = upload.commit(&mut chart).unwrap_err();
    assert_eq!(code(error).as_deref(), Some("stale"));
    assert_eq!(chart.column_revision("v"), current);

    // Another chart refuses it.
    let mut other = FiggyChart::create(canvas()).await.unwrap();
    let mut upload = chart
        .begin_column_upload("n", 1.0, "f64", None, 64.0)
        .unwrap();
    upload.push(0.0, chunk(&[1.0])).unwrap();
    let error = upload.commit(&mut other).unwrap_err();
    assert_eq!(code(error).as_deref(), Some("stale"));
    assert!(other.column_revision("n").is_none());
    assert!(chart.column_revision("n").is_none());

    // Cancel releases the staging buffer.
    let before = total_bytes(&chart);
    let mut upload = chart
        .begin_column_upload("c", 100_000.0, "f64", None, 1e6)
        .unwrap();
    assert!(total_bytes(&chart) > before);
    upload.cancel();
    assert!(!upload.is_open());
    assert!(chart.column_revision("c").is_none());
}

#[wasm_bindgen_test(async)]
async fn a_budget_that_cannot_hold_the_upload_refuses_it_with_a_structured_error() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let total = total_bytes(&chart);
    chart.configure_auto_residency(total, total).unwrap();
    let error = chart
        .begin_column_upload("big", 1_000_000.0, "f64", None, 1e6)
        .err()
        .unwrap();
    assert_eq!(
        error_field(&error, "code").as_deref(),
        Some("budget_exceeded")
    );
    assert!(
        js_sys::Reflect::get(&error, &JsValue::from_str("requestedBytes"))
            .unwrap()
            .as_f64()
            .is_some()
    );
    assert!(chart.column_revision("big").is_none());
}

// ---- candidate streaming check ----

#[wasm_bindgen_test(async)]
async fn a_candidate_streaming_check_changes_nothing() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.load_demo().unwrap();
    let config_json = chart.get_config().unwrap();
    let series_json = chart.get_series().unwrap();

    let current: serde_json::Value = serde_json::from_str(&chart.streaming_capabilities()).unwrap();
    let same: serde_json::Value = serde_json::from_str(
        &chart
            .inspect_streaming_capabilities(&config_json, &series_json)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(same["supported"], current["render_supported"]);

    // A layered candidate is refused with the rule's reason.
    let mut layered = config(&chart);
    layered.overlays.push(OverlayLayer::default());
    let refused: serde_json::Value = serde_json::from_str(
        &chart
            .inspect_streaming_capabilities(&serde_json::to_string(&layered).unwrap(), &series_json)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(refused["supported"], false);
    assert!(
        refused["reason"].as_str().unwrap().contains("overlays"),
        "{refused}"
    );

    // A candidate whose columns are not registered is still only checked.
    let mut unknown = series(&chart);
    unknown[0].y_column = "not registered".into();
    assert!(
        chart
            .inspect_streaming_capabilities(&config_json, &serde_json::to_string(&unknown).unwrap())
            .is_ok()
    );
    assert!(
        chart
            .inspect_streaming_capabilities("{", &series_json)
            .is_err()
    );
    assert_eq!(chart.get_config().unwrap(), config_json);
    assert_eq!(chart.get_series().unwrap(), series_json);
}
