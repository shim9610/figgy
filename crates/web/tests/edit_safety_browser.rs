#![cfg(target_arch = "wasm32")]

//! Edit safety of the browser `FiggyChart` (#311):
//!
//! - `clear_selection` ends the element and annotation selections and any
//!   drag, and keeps the config, the series and the data highlight;
//! - the legacy document calls replace the chart atomically: a failure at
//!   any stage leaves the config, series, columns, selection and image as
//!   they were and names the stage, and a success keeps the documented
//!   column ids and removes the previous document's columns;
//! - a chunked column upload (`begin_column_upload`) refuses overlapping,
//!   gapped, oversized and mistyped chunks, incomplete and stale commits,
//!   a commit on another chart and a budget that cannot hold it, and
//!   installs the column at once on commit;
//! - a column transaction (`begin_columns_transaction`, #334) stages
//!   several chunked columns and the next Config and Series, and installs
//!   them together on commit; a gapped chunk, a stale column, a budget that
//!   cannot hold the commit and a state that does not hold are refused with
//!   `stage` and `column`, and leave values, revisions, Config, Series,
//!   selection and pixels as they were; cancel releases everything;
//! - `inspect_streaming_capabilities` checks a candidate without changing
//!   the chart;
//! - an explicit `decorations.data_area` of a legacy document is the
//!   converted chart's data area, on the canvas at DPR 1 and 2 and in PNG
//!   export (#334);
//! - a legacy document previewed with `convert_*_document` on the chart's
//!   chart area reports what `set_*_document` installs and returns, and a
//!   loss policy (`refuse`) that a note matches refuses the document before
//!   anything changes (#334).

use figgy::{
    FiggyChart, convert_boxplot_document, convert_categorical_document, convert_radar_document,
};
use renderer::config::{DataSelectionsConfig, PickedDataRef};
use renderer::{Config, OverlayLayer, SeriesConfig};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

mod support;
use support::rect;

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
    chart.set_radar_document(RADAR, None).unwrap();
    let before = snapshot(&mut chart).await;
    for (call, error) in [
        ("radar", chart.set_radar_document("{", None).unwrap_err()),
        (
            "categorical",
            chart.set_categorical_document("{", None).unwrap_err(),
        ),
        (
            "boxplot",
            chart.set_boxplot_document("[]", None).unwrap_err(),
        ),
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

    // A documented id held by another kind of source: the box plot record
    // id taken by a host column.
    let mut fresh = FiggyChart::create(canvas()).await.unwrap();
    fresh
        .register_column_f64("boxplot.record.a", &[0.0, 1.0])
        .unwrap();
    let before = snapshot(&mut fresh).await;
    let error = fresh.set_boxplot_document(BOXPLOT, None).unwrap_err();
    assert_eq!(error_field(&error, "stage").as_deref(), Some("validate"));
    assert!(
        error_message(&error).contains("boxplot.record.a"),
        "{}",
        error_message(&error)
    );
    assert_unchanged(&mut fresh, &before, "held ids").await;

    // A documented id held by a host column of the same kind is replaced in
    // place.
    let mut host = FiggyChart::create(canvas()).await.unwrap();
    host.register_column_f64("radar.positions", &[7.0, 8.0])
        .unwrap();
    host.set_radar_document(RADAR, None).unwrap();
    assert_eq!(series(&host)[0].x_column, "radar.positions");
    assert_eq!(column_ids(&host), ["radar.positions", "radar.values.a"]);
}

#[wasm_bindgen_test(async)]
async fn a_document_whose_columns_do_not_fit_the_budget_changes_nothing() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_boxplot_document(BOXPLOT, None).unwrap();
    // A point highlight and an element selection the failure must keep.
    chart.on_press(240.0, 16.0).unwrap();
    let before = snapshot(&mut chart).await;

    // The same series with three million outliers: the styles fit the
    // margin, the outlier values (24 MB as `(hi, lo)` pairs) do not. The web
    // pool starts at 16 MiB, so a 16 MB column would fit it without growth
    // and the transaction's peak would stay near the budget; 24 MB needs a
    // grown candidate slab on top of its staging, past the margin.
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
    let error = chart.set_boxplot_document(&large, None).unwrap_err();
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
    chart.set_boxplot_document(&large, None).unwrap();
}

#[wasm_bindgen_test(async)]
async fn a_failed_fit_changes_nothing() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_categorical_document(CATEGORICAL, None).unwrap();
    let before = snapshot(&mut chart).await;
    // Every value missing: the value axis has nothing to fit. Whether the
    // fit refuses it or keeps a default range, a failure must change
    // nothing and name its stage.
    let empty = CATEGORICAL
        .replace("[12.0, 18.0]", "[null, null]")
        .replace("[18.0, 14.0]", "[null, null]");
    match chart.set_categorical_document(&empty, None) {
        Ok(_) => assert_eq!(series(&chart).len(), 2),
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
async fn replaced_documents_keep_the_documented_ids_and_remove_the_stale_columns() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.register_column_f64("host", &[1.0, 2.0]).unwrap();

    chart.set_radar_document(RADAR, None).unwrap();
    assert_eq!(series(&chart)[0].x_column, "radar.positions");
    assert_eq!(series(&chart)[0].y_column, "radar.values.a");
    assert_eq!(
        column_ids(&chart),
        ["host", "radar.positions", "radar.values.a"]
    );

    // The same family again, larger: two series over four categories. The
    // ids stay the documented ones, replaced in place.
    let wide = r#"{
        "categories": [
            {"id": "speed", "label": "Speed"},
            {"id": "range", "label": "Range"},
            {"id": "cost", "label": "Cost"},
            {"id": "weight", "label": "Weight"}
        ],
        "series": [
            {"id": "a", "label": "A", "values": [4, 3.5, 2, 1],
             "color": {"r": 0.25, "g": 0.56, "b": 0.82, "a": 1.0}},
            {"id": "b", "label": "B", "values": [1, 2, 3, 4],
             "color": {"r": 0.82, "g": 0.36, "b": 0.25, "a": 1.0}}
        ]
    }"#;
    chart.set_radar_document(wide, None).unwrap();
    let radar = series(&chart);
    assert_eq!(radar.len(), 2);
    assert_eq!(radar[0].x_column, "radar.positions");
    assert_eq!(radar[1].y_column, "radar.values.b");
    assert_eq!(
        column_ids(&chart),
        [
            "host",
            "radar.positions",
            "radar.values.a",
            "radar.values.b"
        ]
    );

    // A document of another family: the radar columns go with its series.
    chart.set_categorical_document(CATEGORICAL, None).unwrap();
    let categorical = series(&chart);
    assert_eq!(categorical[0].x_column, "categorical.positions");
    assert_eq!(categorical[1].y_column, "categorical.values.south");
    assert_eq!(
        column_ids(&chart),
        [
            "categorical.positions",
            "categorical.values.north",
            "categorical.values.south",
            "host",
        ]
    );

    // A box plot: its record and ragged outlier column take the documented
    // ids, and the categorical columns go.
    chart.set_boxplot_document(BOXPLOT, None).unwrap();
    let boxes = series(&chart);
    assert_eq!(boxes[0].x_column, "boxplot.positions");
    assert_eq!(boxes[0].y_column, "boxplot.record.a");
    assert_eq!(column_ids(&chart), ["boxplot.positions", "host"]);
    chart.set_boxplot_document(BOXPLOT, None).unwrap();
    assert_eq!(series(&chart)[0].y_column, "boxplot.record.a");

    // Back to a smaller radar: the box plot's record and outliers are gone,
    // so their ids take plain columns again.
    chart.set_radar_document(RADAR, None).unwrap();
    assert_eq!(
        column_ids(&chart),
        ["host", "radar.positions", "radar.values.a"]
    );
    chart
        .register_column_f64("boxplot.record.a", &[1.0])
        .expect("the record is gone");
    chart
        .register_column_f64("boxplot.outliers.a", &[1.0])
        .expect("the ragged column is gone");
    chart.frame().unwrap();
    assert!(!chart.export_png(2.0).await.unwrap().to_vec().is_empty());
}

/// A chart that shows a monotonic document (#271) of one group, `daq`.
async fn monotonic_chart() -> FiggyChart {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let mut config = renderer::Chart::sized(480, 320);
    config.set_x_range(0.0, 99.0);
    config.set_y_range(-1.0, 1.0);
    let series = vec![SeriesConfig::monotonic_line("a", "daq", 0)];
    chart
        .set_monotonic_document(
            &serde_json::to_string(config.config()).unwrap(),
            &serde_json::to_string(&series).unwrap(),
        )
        .unwrap();
    assert_eq!(chart.monotonic_groups().unwrap(), r#"["daq"]"#);
    chart
}

#[wasm_bindgen_test(async)]
async fn a_document_replaces_a_shown_monotonic_document_under_the_documented_ids() {
    let mut chart = monotonic_chart().await;
    chart.set_categorical_document(CATEGORICAL, None).unwrap();
    // The monotonic chart ended with the commit; the document is shown.
    assert_eq!(chart.monotonic_groups().unwrap(), "[]");
    let shown = series(&chart);
    assert_eq!(shown.len(), 2);
    assert_eq!(shown[0].x_column, "categorical.positions");
    assert_eq!(shown[0].y_column, "categorical.values.north");
    assert_eq!(shown[1].y_column, "categorical.values.south");
    assert_eq!(
        column_ids(&chart),
        [
            "categorical.positions",
            "categorical.values.north",
            "categorical.values.south",
        ]
    );
    chart.frame().unwrap();
    assert!(!chart.export_png(1.0).await.unwrap().to_vec().is_empty());
}

#[wasm_bindgen_test(async)]
async fn a_failed_document_replace_leaves_a_shown_monotonic_document_as_it_was() {
    let mut chart = monotonic_chart().await;
    let state = |chart: &FiggyChart| {
        (
            chart.get_config().unwrap(),
            chart.get_series().unwrap(),
            column_ids(chart),
            chart.monotonic_groups().unwrap(),
        )
    };
    let before = state(&chart);

    // Validate: a malformed document, and a documented id held by another
    // kind of source.
    let error = chart.set_radar_document("{", None).unwrap_err();
    assert_eq!(error_field(&error, "stage").as_deref(), Some("validate"));
    assert_eq!(state(&chart), before, "malformed document");
    chart
        .register_column_f64("boxplot.record.a", &[0.0, 1.0])
        .unwrap();
    let before = state(&chart);
    let error = chart.set_boxplot_document(BOXPLOT, None).unwrap_err();
    assert_eq!(error_field(&error, "stage").as_deref(), Some("validate"));
    assert_eq!(state(&chart), before, "held id");

    // Upload: a budget the transaction's peak does not fit (24 MB of
    // outliers on a 16 MiB pool, see the budget test above).
    let outliers = (0..3_000_000)
        .map(|i| format!("{}", 100.0 + i as f64 * 1e-3))
        .collect::<Vec<_>>()
        .join(",");
    let total = total_bytes(&chart);
    let margin = 2.0 * 1024.0 * 1024.0;
    chart
        .configure_auto_residency(total + margin, total + margin)
        .unwrap();
    // Series `b`: its record id is free, so the failure is the upload's.
    let large_boxes = BOXPLOT
        .replace(
            "\"outliers\": [9.0]",
            &format!("\"outliers\": [{outliers}]"),
        )
        .replace("\"id\": \"a\"", "\"id\": \"b\"");
    let error = chart.set_boxplot_document(&large_boxes, None).unwrap_err();
    assert_eq!(
        error_field(&error, "stage").as_deref(),
        Some("upload"),
        "{}",
        error_message(&error)
    );
    assert_eq!(state(&chart), before, "upload failure");

    // With room again a document replaces it.
    chart.configure_auto_residency(1e12, 1e12).unwrap();
    chart.set_radar_document(RADAR, None).unwrap();
    assert_eq!(chart.monotonic_groups().unwrap(), "[]");
    assert_eq!(series(&chart)[0].x_column, "radar.positions");
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

// ---- column transaction (#334) ----

/// A chart that draws `y` over `x` (two f64 values each), with a title
/// pressed so an element selection is held.
async fn two_column_chart() -> FiggyChart {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.register_column_f64("x", &[0.0, 1.0]).unwrap();
    chart.register_column_f64("y", &[0.0, 1.0]).unwrap();
    let line = SeriesConfig::line("xy", "x", "y");
    chart
        .set_series(&serde_json::to_string(&[line]).unwrap())
        .unwrap();
    chart.fit_ranges().unwrap();
    chart.set_title("Transaction").unwrap();
    chart.frame().unwrap();
    assert!(chart.on_press(240.0, 16.0).unwrap());
    assert!(chart.has_selection().unwrap());
    chart
}

fn f64s(values: &[f64]) -> JsValue {
    js_sys::Float64Array::from(values).into()
}

fn revisions(chart: &FiggyChart) -> [Option<f64>; 2] {
    [chart.column_revision("x"), chart.column_revision("y")]
}

/// A transaction error's `code`, `stage` and `column`.
fn scoped(error: &JsValue) -> (Option<String>, Option<String>, Option<String>) {
    (
        error_field(error, "code"),
        error_field(error, "stage"),
        error_field(error, "column"),
    )
}

#[wasm_bindgen_test(async)]
async fn a_column_transaction_refuses_a_gapped_chunk_then_commits_columns_and_state_together() {
    let mut chart = two_column_chart().await;
    let before = snapshot(&mut chart).await;
    let [x_revision, y_revision] = revisions(&chart);

    // The issue's scenario: both staged at their revisions with a 16-byte
    // chunk allowance, x written whole, y pushed at 1 without 0.
    let mut tx = chart.begin_columns_transaction(16.0).unwrap();
    assert!(tx.is_open());
    tx.stage(&chart, "x", 2.0, "f64", x_revision).unwrap();
    tx.stage(&chart, "y", 2.0, "f64", y_revision).unwrap();
    assert_eq!(tx.staged_count(), 2.0);
    tx.push("x", 0.0, f64s(&[10.0, 20.0])).unwrap();
    let error = tx.push("y", 1.0, f64s(&[7.0])).unwrap_err();
    assert_eq!(
        scoped(&error),
        (
            Some("invalid_chunk".into()),
            Some("push".into()),
            Some("y".into())
        ),
        "{}",
        error_message(&error)
    );
    // Too many bytes for the allowance, and a column that is not staged.
    let error = tx.push("y", 0.0, f64s(&[1.0, 2.0, 3.0])).unwrap_err();
    assert_eq!(
        error_field(&error, "code").as_deref(),
        Some("invalid_chunk")
    );
    let error = tx.push("z", 0.0, f64s(&[1.0])).unwrap_err();
    assert_eq!(scoped(&error).2.as_deref(), Some("z"));
    // The same id twice is refused at stage.
    let error = tx.stage(&chart, "x", 2.0, "f64", x_revision).unwrap_err();
    assert_eq!(scoped(&error).1.as_deref(), Some("stage"));
    assert_eq!(scoped(&error).2.as_deref(), Some("x"));
    assert!(tx.is_open());
    assert_eq!(tx.written("x"), Some(2.0));
    assert_eq!(tx.written("y"), Some(0.0));
    assert_unchanged(&mut chart, &before, "refused chunk").await;
    assert_eq!(revisions(&chart), [x_revision, y_revision]);

    // The rest of y, and the next Config and Series that read the columns.
    tx.push("y", 0.0, f64s(&[30.0, 40.0])).unwrap();
    let mut next = config(&chart);
    next.bottom_x.min = 0.0;
    next.bottom_x.max = 25.0;
    next.left_y.min = 0.0;
    next.left_y.max = 50.0;
    let line = SeriesConfig::line("next", "x", "y");
    tx.set_state(
        &serde_json::to_string(&next).unwrap(),
        Some(serde_json::to_string(&[line]).unwrap()),
    )
    .unwrap();
    assert!(tx.has_state());
    tx.commit(&mut chart).unwrap();
    assert!(!tx.is_open());
    let [x_after, y_after] = revisions(&chart);
    assert!(x_after.is_some() && x_after != x_revision);
    assert!(y_after.is_some() && y_after != y_revision);
    assert_eq!(series(&chart)[0].series_id, "next");
    assert_eq!(config(&chart).bottom_x.max, 25.0);
    chart.frame().unwrap();
    assert_ne!(chart.export_png(1.0).await.unwrap().to_vec(), before.png);

    // Closed after the commit.
    let error = tx.push("y", 0.0, f64s(&[1.0])).unwrap_err();
    assert_eq!(
        error_field(&error, "code").as_deref(),
        Some("upload_closed")
    );
    let error = tx.stage(&chart, "w", 1.0, "f64", None).unwrap_err();
    assert_eq!(
        error_field(&error, "code").as_deref(),
        Some("upload_closed")
    );

    // Without a state the chart keeps its Config and Series.
    let kept = (chart.get_config().unwrap(), chart.get_series().unwrap());
    let mut tx = chart.begin_columns_transaction(64.0).unwrap();
    tx.stage(&chart, "y", 2.0, "f32", y_after).unwrap();
    tx.push(
        "y",
        0.0,
        js_sys::Float32Array::from(&[1.0f32, 2.0][..]).into(),
    )
    .unwrap();
    tx.commit(&mut chart).unwrap();
    assert_ne!(chart.column_revision("y"), y_after);
    assert_eq!(chart.column_revision("x"), x_after);
    assert_eq!(
        (chart.get_config().unwrap(), chart.get_series().unwrap()),
        kept
    );
}

#[wasm_bindgen_test(async)]
async fn a_stale_column_or_a_state_that_does_not_hold_refuses_the_whole_commit() {
    let mut chart = two_column_chart().await;
    let [x_revision, y_revision] = revisions(&chart);
    let mut tx = chart.begin_columns_transaction(64.0).unwrap();
    tx.stage(&chart, "x", 2.0, "f64", x_revision).unwrap();
    tx.stage(&chart, "y", 2.0, "f64", y_revision).unwrap();
    tx.push("x", 0.0, f64s(&[5.0, 6.0])).unwrap();
    tx.push("y", 0.0, f64s(&[7.0, 8.0])).unwrap();
    // y changes while the transaction is open.
    chart.update_register_column_f64("y", &[0.5, 0.25]).unwrap();
    chart.frame().unwrap();
    let before = snapshot(&mut chart).await;
    let current = revisions(&chart);
    let error = tx.commit(&mut chart).unwrap_err();
    assert_eq!(
        scoped(&error),
        (
            Some("stale".into()),
            Some("commit".into()),
            Some("y".into())
        ),
        "{}",
        error_message(&error)
    );
    assert!(!tx.is_open());
    assert_unchanged(&mut chart, &before, "stale commit").await;
    assert_eq!(revisions(&chart), current);
    assert_eq!(current[0], x_revision);

    // A state whose series reads a column nothing registers.
    let mut tx = chart.begin_columns_transaction(64.0).unwrap();
    tx.stage(&chart, "x", 2.0, "f64", current[0]).unwrap();
    tx.push("x", 0.0, f64s(&[5.0, 6.0])).unwrap();
    let line = SeriesConfig::line("next", "x", "nope");
    tx.set_state(
        &chart.get_config().unwrap(),
        Some(serde_json::to_string(&[line]).unwrap()),
    )
    .unwrap();
    let error = tx.commit(&mut chart).unwrap_err();
    assert_eq!(
        error_field(&error, "stage").as_deref(),
        Some("state"),
        "{}",
        error_message(&error)
    );
    assert_unchanged(&mut chart, &before, "state that does not hold").await;
    assert_eq!(revisions(&chart), current);

    // JSON that does not parse is refused at set_state; the
    // transaction stays open.
    let mut tx = chart.begin_columns_transaction(64.0).unwrap();
    let error = tx.set_state("{", None).unwrap_err();
    assert_eq!(error_field(&error, "stage").as_deref(), Some("state"));
    assert!(tx.is_open());
    assert!(!tx.has_state());
    tx.cancel();
}

#[wasm_bindgen_test(async)]
async fn a_budget_that_cannot_hold_the_commit_refuses_it_and_cancel_releases_the_staging() {
    let mut chart = two_column_chart().await;
    let [x_revision, y_revision] = revisions(&chart);
    let mut tx = chart.begin_columns_transaction(1e6).unwrap();
    tx.stage(&chart, "x", 1_000.0, "f64", x_revision).unwrap();
    tx.stage(&chart, "y", 1_000.0, "f64", y_revision).unwrap();
    let values: Vec<f64> = (0..1_000).map(f64::from).collect();
    tx.push("x", 0.0, f64s(&values)).unwrap();
    tx.push("y", 0.0, f64s(&values)).unwrap();
    let before = snapshot(&mut chart).await;
    // The staging fits, and so does an export; the candidate pool on top
    // of it (the web pool starts at 16 MiB) does not.
    let total = total_bytes(&chart);
    let margin = 8.0 * 1024.0 * 1024.0;
    chart
        .configure_auto_residency(total + margin, total + margin)
        .unwrap();
    let error = tx.commit(&mut chart).unwrap_err();
    assert_eq!(
        scoped(&error).0.as_deref(),
        Some("budget_exceeded"),
        "{}",
        error_message(&error)
    );
    assert_eq!(scoped(&error).1.as_deref(), Some("commit"));
    for field in ["requested", "limit"] {
        assert!(
            js_sys::Reflect::get(&error, &JsValue::from_str(field))
                .unwrap()
                .as_f64()
                .is_some(),
            "{field}"
        );
    }
    assert_unchanged(&mut chart, &before, "budget").await;
    assert_eq!(revisions(&chart), [x_revision, y_revision]);
    chart.configure_auto_residency(1e12, 1e12).unwrap();

    // Cancel: nothing registered, the staging released, the handle closed.
    let before = snapshot(&mut chart).await;
    let idle = total_bytes(&chart);
    let mut tx = chart.begin_columns_transaction(1e6).unwrap();
    tx.stage(&chart, "big", 100_000.0, "f64", None).unwrap();
    tx.stage(&chart, "x", 2.0, "f64", x_revision).unwrap();
    tx.push("x", 0.0, f64s(&[1.0, 2.0])).unwrap();
    assert!(total_bytes(&chart) > idle);
    tx.cancel();
    assert!(!tx.is_open());
    assert_eq!(tx.staged_count(), 0.0);
    let error = tx.commit(&mut chart).unwrap_err();
    assert_eq!(
        error_field(&error, "code").as_deref(),
        Some("upload_closed")
    );
    assert!(chart.column_revision("big").is_none());
    assert_unchanged(&mut chart, &before, "cancel").await;
    assert_eq!(revisions(&chart), [x_revision, y_revision]);
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

// ---- explicit data area (#334) ----

/// The 640 x 480 logical document of the #334 reproduction.
const LEGACY_DOC: (u32, u32) = (640, 480);

/// The rectangle the #334 reproduction sets, `[x, y, w, h]`.
const LEGACY_AREA: [f64; 4] = [80.0, 70.0, 420.0, 260.0];

/// `document` with `decorations.data_area` set to `area`.
fn with_data_area(document: &str, area: [f64; 4]) -> String {
    let mut value: serde_json::Value = serde_json::from_str(document).unwrap();
    value["decorations"] = serde_json::json!({
        "data_area": {"x": area[0], "y": area[1], "width": area[2], "height": area[3]},
        "offsets": [],
        "selected": null,
    });
    value.to_string()
}

/// The PNG's size and the bounding box `[x, y, w, h]`, in its pixels, of
/// its blue-dominant pixels (the first fixture series' colour), decoded by
/// the browser; `null` when there is none.
async fn blue_bounds(png: &js_sys::Uint8Array) -> serde_json::Value {
    let decode: js_sys::Function = js_sys::eval(
        r#"(async (bytes) => {
  const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }),
    { premultiplyAlpha: 'none', colorSpaceConversion: 'none' });
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const g = canvas.getContext('2d');
  g.drawImage(bitmap, 0, 0);
  const d = g.getImageData(0, 0, bitmap.width, bitmap.height).data;
  let x0 = Infinity, y0 = Infinity, x1 = -1, y1 = -1;
  for (let y = 0; y < bitmap.height; y++) {
    for (let x = 0; x < bitmap.width; x++) {
      const i = (y * bitmap.width + x) * 4;
      if (d[i + 3] > 200 && d[i + 2] > 150 && d[i + 2] - d[i] > 60) {
        x0 = Math.min(x0, x); y0 = Math.min(y0, y);
        x1 = Math.max(x1, x); y1 = Math.max(y1, y);
      }
    }
  }
  const bounds = x1 < 0 ? null : [x0, y0, x1 + 1 - x0, y1 + 1 - y0];
  return JSON.stringify({ width: bitmap.width, height: bitmap.height, bounds });
})"#,
    )
    .unwrap()
    .dyn_into()
    .unwrap();
    let promise: js_sys::Promise = decode
        .call1(&JsValue::NULL, png)
        .unwrap()
        .dyn_into()
        .unwrap();
    let text = JsFuture::from(promise).await.unwrap().as_string().unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Each legacy family with the #334 rectangle on a 640 x 480 logical
/// document: `convert_*_document` notes nothing about it, and after
/// `set_*_document` the data area is the rectangle in document pixels and
/// that times the DPR on the canvas, at DPR 1 and 2. In PNG export at 1x
/// and 2x the image is the document times the scale, and the bars of the
/// categorical chart stand on the rectangle's bottom edge inside it. A
/// rectangle the chart area cannot hold is noted as dropped with a reason,
/// and its document opens with the automatic layout.
#[wasm_bindgen_test(async)]
async fn an_explicit_data_area_is_the_data_area_at_each_dpr_and_in_png() {
    type Convert = fn(&str, Option<String>) -> Result<String, JsValue>;
    let families: [(&str, &str, Convert); 3] = [
        ("categorical", CATEGORICAL, convert_categorical_document),
        ("boxplot", BOXPLOT, convert_boxplot_document),
        ("radar", RADAR, convert_radar_document),
    ];
    let open = |chart: &mut FiggyChart, family: &str, document: &str| match family {
        "categorical" => chart.set_categorical_document(document, None),
        "boxplot" => chart.set_boxplot_document(document, None),
        _ => chart.set_radar_document(document, None),
    };
    for (family, plain, convert) in families {
        let document = with_data_area(plain, LEGACY_AREA);
        let converted: serde_json::Value =
            serde_json::from_str(&convert(&document, None).unwrap()).unwrap();
        assert_eq!(converted["notes"], serde_json::json!([]), "{family}");

        for dpr in [1u32, 2] {
            let canvas = canvas();
            canvas.set_width(LEGACY_DOC.0);
            canvas.set_height(LEGACY_DOC.1);
            let mut chart = FiggyChart::create(canvas).await.unwrap();
            chart
                .resize(LEGACY_DOC.0 * dpr, LEGACY_DOC.1 * dpr)
                .unwrap();
            open(&mut chart, family, &document).unwrap();
            chart.frame().unwrap();
            let area: serde_json::Value =
                serde_json::from_str(&chart.data_area_rect().unwrap()).unwrap();
            assert_eq!(
                rect(&area["document"]),
                LEGACY_AREA,
                "{family} at DPR {dpr}"
            );
            assert_eq!(
                rect(&area["canvas"]),
                LEGACY_AREA.map(|v| v * f64::from(dpr)),
                "{family} at DPR {dpr}"
            );
            for scale in [1u32, 2] {
                let png = chart.export_png(scale as f32).await.unwrap();
                let decoded = blue_bounds(&png).await;
                assert_eq!(decoded["width"], LEGACY_DOC.0 * scale, "{family}");
                assert_eq!(decoded["height"], LEGACY_DOC.1 * scale, "{family}");
                if family != "categorical" {
                    continue;
                }
                let [x, y, w, h] = rect(&decoded["bounds"]);
                let [ax, ay, aw, ah] = LEGACY_AREA.map(|v| v * f64::from(scale));
                let what = format!("bars {:?} in {:?} at {scale}x", [x, y, w, h], LEGACY_AREA);
                assert!(x >= ax && y >= ay && x + w <= ax + aw, "{what}");
                assert!((y + h - (ay + ah)).abs() <= f64::from(scale), "{what}");
            }
        }

        // Outside the 640 x 480 chart area: dropped, automatic layout.
        let canvas = canvas();
        canvas.set_width(LEGACY_DOC.0);
        canvas.set_height(LEGACY_DOC.1);
        let mut chart = FiggyChart::create(canvas).await.unwrap();
        open(&mut chart, family, plain).unwrap();
        let automatic = chart.data_area_rect().unwrap();
        let outside = with_data_area(plain, [600.0, 400.0, 300.0, 200.0]);
        open(&mut chart, family, &outside).unwrap();
        assert_eq!(chart.data_area_rect().unwrap(), automatic, "{family}");
        // On the converter's placeholder (1000 x 800) the same rectangle
        // fits; one past it is noted.
        let past = with_data_area(plain, [900.0, 700.0, 300.0, 200.0]);
        let converted: serde_json::Value =
            serde_json::from_str(&convert(&past, None).unwrap()).unwrap();
        assert_eq!(converted["notes"][0]["path"], "decorations.data_area");
        assert_eq!(converted["notes"][0]["kind"], "dropped");
        assert!(!converted["notes"][0]["reason"].as_str().unwrap().is_empty());
    }
}

/// The JSON a refused document call carried as its `notes`.
fn error_notes(error: &JsValue) -> serde_json::Value {
    let notes = js_sys::Reflect::get(error, &JsValue::from_str("notes")).unwrap();
    let text = js_sys::JSON::stringify(&notes)
        .unwrap()
        .as_string()
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The paths of a list of conversion notes.
fn note_paths(notes: &serde_json::Value) -> Vec<String> {
    notes
        .as_array()
        .unwrap()
        .iter()
        .map(|note| note["path"].as_str().unwrap().to_owned())
        .collect()
}

/// One conversion for review and apply (#334): a document whose
/// conversion drops its background and an explicit data area the 640 x 480
/// chart area cannot hold (the 1000 x 800 placeholder can), previewed with
/// `convert_*_document` on the chart's chart area, reports exactly what
/// `set_*_document` returns, notes included. Under a `refuse` policy that
/// one of those notes matches (a listed path, or `"any"`) the setter
/// throws `conversion_loss` with the refused notes and the chart's config,
/// series, columns, selection and image are as they were; options it does
/// not know are refused the same way; a path no note reports opens it.
#[wasm_bindgen_test(async)]
async fn a_previewed_loss_is_the_applied_loss_and_a_refused_one_changes_nothing() {
    type Convert = fn(&str, Option<String>) -> Result<String, JsValue>;
    type Set = fn(&mut FiggyChart, &str, Option<String>) -> Result<String, JsValue>;
    let families: [(&str, &str, Convert, Set); 3] = [
        (
            "categorical",
            CATEGORICAL,
            convert_categorical_document,
            FiggyChart::set_categorical_document,
        ),
        (
            "boxplot",
            BOXPLOT,
            convert_boxplot_document,
            FiggyChart::set_boxplot_document,
        ),
        (
            "radar",
            RADAR,
            convert_radar_document,
            FiggyChart::set_radar_document,
        ),
    ];
    for (family, plain, convert, set) in families {
        let canvas = canvas();
        canvas.set_width(LEGACY_DOC.0);
        canvas.set_height(LEGACY_DOC.1);
        let mut chart = FiggyChart::create(canvas).await.unwrap();
        // A shown document to keep: the plain one, opened without options.
        let plain_report = set(&mut chart, plain, None).unwrap();
        let chart_area = config(&chart).chart_area.0;
        let options = serde_json::json!({
            "chart_area": [
                chart_area.x,
                chart_area.y,
                chart_area.width,
                chart_area.height
            ]
        })
        .to_string();
        assert_eq!(
            plain_report,
            convert(plain, Some(options.clone())).unwrap(),
            "{family}: plain"
        );
        chart.frame().unwrap();

        let mut lossy: serde_json::Value =
            serde_json::from_str(&with_data_area(plain, [600.0, 400.0, 300.0, 200.0])).unwrap();
        lossy["background"] = serde_json::json!({"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0});
        let lossy = lossy.to_string();
        // On the placeholder only the background is lost; on the chart's
        // chart area the data area is too.
        let placeholder: serde_json::Value =
            serde_json::from_str(&convert(&lossy, None).unwrap()).unwrap();
        assert_eq!(
            note_paths(&placeholder["notes"]),
            ["background"],
            "{family}"
        );
        let preview = convert(&lossy, Some(options.clone())).unwrap();
        let previewed: serde_json::Value = serde_json::from_str(&preview).unwrap();
        assert_eq!(
            note_paths(&previewed["notes"]),
            ["background", "decorations.data_area"],
            "{family}"
        );

        let before = snapshot(&mut chart).await;
        for (refuse, refused) in [
            (
                serde_json::json!(["decorations.data_area"]),
                vec!["decorations.data_area"],
            ),
            (
                serde_json::json!(["decorations"]),
                vec!["decorations.data_area"],
            ),
            (
                serde_json::json!("any"),
                vec!["background", "decorations.data_area"],
            ),
        ] {
            let what = format!("{family} refuse {refuse}");
            let options = serde_json::json!({ "refuse": refuse }).to_string();
            let error = set(&mut chart, &lossy, Some(options)).unwrap_err();
            assert_eq!(
                error_field(&error, "code").as_deref(),
                Some("conversion_loss"),
                "{what}: {}",
                error_message(&error)
            );
            assert_eq!(
                error_field(&error, "stage").as_deref(),
                Some("validate"),
                "{what}"
            );
            let notes = error_notes(&error);
            assert_eq!(note_paths(&notes), refused, "{what}");
            for note in notes.as_array().unwrap() {
                let reported = previewed["notes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|n| n["path"] == note["path"]);
                assert_eq!(reported, Some(note), "{what}: the previewed note");
            }
            assert_unchanged(&mut chart, &before, &what).await;
        }
        for options in [
            r#"{"refuse": "all"}"#,
            r#"{"refuse": ["background"], "chart_area": [0, 0, 640, 480]}"#,
            "[]",
        ] {
            let error = set(&mut chart, &lossy, Some(options.into())).unwrap_err();
            assert_eq!(
                error_field(&error, "code").as_deref(),
                Some("invalid_options"),
                "{family} {options}: {}",
                error_message(&error)
            );
            assert_eq!(error_field(&error, "stage").as_deref(), Some("validate"));
            assert_unchanged(&mut chart, &before, options).await;
        }
        // Options `convert_*_document` does not know are refused too.
        let error = convert(&lossy, Some(r#"{"refuse": "any"}"#.into())).unwrap_err();
        assert_eq!(
            error_field(&error, "code").as_deref(),
            Some("invalid_options")
        );

        // A path no note reports: the document opens, and the setter
        // returns the previewed report.
        let options = serde_json::json!({ "refuse": ["title", "overrides"] }).to_string();
        let report = set(&mut chart, &lossy, Some(options)).unwrap();
        assert_eq!(report, preview, "{family}: the applied report");
        chart.frame().unwrap();
        // Without options, the same report.
        assert_eq!(set(&mut chart, &lossy, None).unwrap(), preview, "{family}");
    }
}
