#![cfg(target_arch = "wasm32")]
//! Regression tests for the pre-release sweep of the web wrapper (#468):
//!
//! - `remove_column` and `load_demo` act on the chart the kernel shows, and
//!   keep the hidden general chart's document (item 1);
//! - every streamed call refuses a monotonic chart (item 2);
//! - the general chart is placed on the surface again after a resize made
//!   while a monotonic chart was shown (item 3);
//! - `set_series` takes monotonic series before their first reduction and
//!   refuses them on the general chart with `MonotonicGeneralChart`
//!   (item 4);
//! - Feynman drags and `place_feynman_vertices` edit the monotonic chart,
//!   which commits a diagram's layout on registration (item 5);
//! - the annotation selection ends with the list it indexed (item 6);
//! - a column upload committed on another chart is closed (item 10);
//! - a non-finite clear colour component reads as 0 (item 11);
//! - `add_line_series` keeps the colour of an existing marker series
//!   (item 12);
//! - a monotonic transfer chunk that cannot be allocated refuses the call
//!   (item 13);
//! - a surface past the device limit is refused and changes nothing, a
//!   non-array `columns` is refused, and a press ends a Feynman drag whose
//!   release never came (suspicions confirmed in #468).

use figgy::FiggyChart;
use renderer::feynman::{FeynmanSeries, FeynmanVertex, ParticleLine, Propagator, VertexRole};
use renderer::layout::{ChartArea, Rect};
use renderer::{Chart, Color, DataRenderType, SeriesConfig};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

mod support;
use support::code;

wasm_bindgen_test_configure!(run_in_browser);

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const N: usize = 1_000;

fn canvas() -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(WIDTH);
    canvas.set_height(HEIGHT);
    canvas
}

/// A document the size of the canvas over 0..1 on both axes.
fn config() -> renderer::Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    chart.config().clone()
}

macro_rules! to_json {
    ($value:expr) => {
        serde_json::to_string($value).unwrap()
    };
}

fn series_of(chart: &FiggyChart) -> Vec<SeriesConfig> {
    serde_json::from_str(&chart.get_series().unwrap()).unwrap()
}

fn config_of(chart: &FiggyChart) -> renderer::Config {
    serde_json::from_str(&chart.get_config().unwrap()).unwrap()
}

fn message(error: &JsValue) -> String {
    error
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| error.as_string())
        .unwrap_or_default()
}

/// The shown document as JSON, without its legend.
fn without_legend(chart: &FiggyChart) -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("legend");
    value
}

/// Whether the legend has a row whose symbol names series `id`.
fn legend_names(config: &renderer::Config, id: &str) -> bool {
    config
        .legend
        .content
        .segments
        .iter()
        .any(|segment| segment.series_id.as_deref() == Some(id))
}

fn register_xy(chart: &mut FiggyChart) {
    let xs: Vec<f64> = (0..16).map(|i| i as f64 / 15.0).collect();
    let ys: Vec<f64> = xs.iter().map(|x| x * x).collect();
    chart.register_column_f64("x", &xs).unwrap();
    chart.register_column_f64("y", &ys).unwrap();
}

/// Group `daq`'s whole source, one channel, as the calls that take
/// sources take it.
fn sources(values: &[f64]) -> js_sys::Array {
    let source = js_sys::Object::new();
    let set = |key: &str, value: &JsValue| {
        js_sys::Reflect::set(&source, &JsValue::from_str(key), value).unwrap();
    };
    let channels = js_sys::Array::new();
    channels.push(&js_sys::Float64Array::from(values));
    set("group", &JsValue::from_str("daq"));
    set("start", &JsValue::from_f64(0.0));
    set("interval", &JsValue::from_f64(1.0 / (N - 1) as f64));
    set("data", &channels.into());
    let list = js_sys::Array::new();
    list.push(&source);
    list
}

fn wave() -> Vec<f64> {
    (0..N)
        .map(|i| 0.5 + 0.4 * (i as f64 * 0.02).sin())
        .collect()
}

fn s_channel() -> SeriesConfig {
    let vertex = |id: &str, role| FeynmanVertex::new(id, role);
    SeriesConfig {
        series_id: "d".into(),
        source_id: None,
        label: None,
        x_column: String::new(),
        y_column: String::new(),
        z_column: None,
        render_type: DataRenderType::Feynman(FeynmanSeries {
            vertices: vec![
                vertex("e-", VertexRole::Incoming),
                vertex("e+", VertexRole::Incoming),
                vertex("v1", VertexRole::Internal),
                vertex("v2", VertexRole::Internal),
                vertex("mu-", VertexRole::Outgoing),
                vertex("mu+", VertexRole::Outgoing),
            ],
            propagators: vec![
                Propagator::new("p1", "e-", "v1", ParticleLine::Fermion),
                Propagator::new("p2", "v1", "e+", ParticleLine::Fermion),
                Propagator::new("g", "v1", "v2", ParticleLine::Photon),
                Propagator::new("p3", "v2", "mu-", ParticleLine::Fermion),
                Propagator::new("p4", "mu+", "v2", ParticleLine::Fermion),
            ],
            ..FeynmanSeries::default()
        }),
    }
}

/// A text note whose id is its text.
fn text_note(text: &str) -> String {
    let mut item = renderer::Annotation::new(renderer::AnnotationKind::Text {
        anchor: renderer::Anchor::Ratio { x: 0.4, y: 0.4 },
        text: renderer::text::RichText::plain(text, Color::BLACK, 14.0, ""),
        box_style: None,
        align: Default::default(),
    });
    item.id = text.into();
    to_json!(&item)
}

// ---- item 1 ----

#[wasm_bindgen_test(async)]
async fn remove_column_edits_the_shown_monotonic_chart_and_keeps_the_general_document() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    register_xy(&mut chart);
    chart.set_config(&to_json!(&config())).unwrap();
    chart.set_title("GENERAL").unwrap();
    chart
        .add_line_series("g", "x", "y", 1.0, "general")
        .unwrap();
    let general = config_of(&chart);
    assert!(legend_names(&general, "g"));
    let general_doc = without_legend(&chart);

    let series = vec![
        SeriesConfig::monotonic_line("m", "daq", 0),
        SeriesConfig::line("l", "x", "y"),
    ];
    chart
        .set_monotonic_document(&to_json!(&config()), &to_json!(&series))
        .unwrap();
    chart.set_title("MONO").unwrap();
    assert!(chart.set_series_label("l", "line").unwrap());
    assert!(legend_names(&config_of(&chart), "l"));
    let shown_doc = without_legend(&chart);
    assert_ne!(shown_doc, general_doc);

    // The demo draws on the general chart: refused while it is hidden.
    assert!(message(&chart.load_demo().unwrap_err()).contains("clear_monotonic_document"));

    assert!(chart.remove_column("y").unwrap());
    let shown = config_of(&chart);
    assert!(!legend_names(&shown, "l"), "the removed series' row goes");
    assert_eq!(
        without_legend(&chart),
        shown_doc,
        "the shown document is kept"
    );
    assert_eq!(
        series_of(&chart)
            .iter()
            .map(|s| s.series_id.as_str())
            .collect::<Vec<_>>(),
        ["m"]
    );

    chart.clear_monotonic_document().unwrap();
    let back = config_of(&chart);
    assert_eq!(
        without_legend(&chart),
        general_doc,
        "the general document is its own"
    );
    assert!(!legend_names(&back, "g"), "its legend followed its cascade");
    assert!(series_of(&chart).is_empty());
    chart.frame().unwrap();
    chart.load_demo().unwrap();
}

// ---- item 2 ----

#[wasm_bindgen_test(async)]
async fn every_streamed_call_refuses_a_monotonic_chart() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart
        .set_monotonic_document(
            &to_json!(&config()),
            &to_json!(&vec![SeriesConfig::monotonic_line("m", "daq", 0)]),
        )
        .unwrap();
    let streamed = || {
        let list = js_sys::Array::new();
        list.push(&js_sys::Float32Array::from(&[1.0f32, 2.0][..]));
        list
    };
    let refusals: Vec<(&str, Result<(), JsValue>)> = vec![
        ("begin_streaming", chart.begin_streaming(1e6)),
        (
            "stream_step",
            chart
                .stream_step(vec!["s".into()], vec![1.0], streamed())
                .map(drop),
        ),
        (
            "auto_stream_chart_step",
            chart
                .auto_stream_chart_step(vec!["s".into()], vec![1.0], streamed())
                .map(drop),
        ),
        (
            "auto_stream_chart_request_ranges",
            chart.auto_stream_chart_request_ranges().map(drop),
        ),
        (
            "stream_selection_request_ranges",
            chart.stream_selection_request_ranges().map(drop),
        ),
        (
            "set_stream_chunk_budget",
            chart.set_stream_chunk_budget(64.0),
        ),
        (
            "register_streaming_columns",
            chart.register_streaming_columns(vec!["s".into()], vec![1.0], streamed()),
        ),
        (
            "replace_streaming_columns",
            chart.replace_streaming_columns(vec!["s".into()], vec![2.0], streamed()),
        ),
        (
            "register_streaming_column_sources",
            chart.register_streaming_column_sources(
                vec!["s".into()],
                vec![1.0],
                vec![2.0],
                vec!["f32".into()],
            ),
        ),
        (
            "demote_auto_resident_columns",
            chart.demote_auto_resident_columns(vec!["s".into()], vec![1.0], streamed()),
        ),
    ];
    for (name, result) in refusals {
        let error = result.expect_err(name);
        assert!(
            message(&error).contains("monotonic chart is not streamed"),
            "{name}: {}",
            message(&error)
        );
    }
    // Nothing was registered or started.
    assert!(chart.column_revision("s").is_none());
    chart.frame().unwrap();
}

// ---- item 3 ----

#[wasm_bindgen_test(async)]
async fn the_general_chart_is_placed_again_after_a_monotonic_resize() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    register_xy(&mut chart);
    chart.set_config(&to_json!(&config())).unwrap();
    chart.add_line_series("g", "x", "y", 1.0, "").unwrap();
    chart.frame().unwrap();

    let values = wave();
    chart
        .set_monotonic_document(
            &to_json!(&config()),
            &to_json!(&vec![SeriesConfig::monotonic_line("m", "daq", 0)]),
        )
        .unwrap();
    let channels = js_sys::Array::new();
    channels.push(&js_sys::Float64Array::from(values.as_slice()));
    chart
        .set_monotonic(
            "daq",
            0.0,
            1.0 / (N - 1) as f64,
            channels.into(),
            "autoscale".into(),
            None,
            None,
        )
        .unwrap();
    chart
        .resize_monotonic(2 * WIDTH, 2 * HEIGHT, sources(&values), JsValue::UNDEFINED)
        .unwrap();
    chart.clear_monotonic_document().unwrap();

    // The document is fitted to the 2x surface it is drawn on now.
    let rects: serde_json::Value = serde_json::from_str(&chart.data_area_rect().unwrap()).unwrap();
    let (document, canvas) = (&rects["document"], &rects["canvas"]);
    let width = |rect: &serde_json::Value| rect[2].as_f64().unwrap();
    assert!(
        (width(canvas) - 2.0 * width(document)).abs() < 1.5,
        "{rects}"
    );
    chart.frame().unwrap();
}

// ---- item 4 ----

#[wasm_bindgen_test(async)]
async fn set_series_takes_monotonic_series_before_their_first_reduction() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let one = vec![SeriesConfig::monotonic_line("a", "daq", 0)];
    // The general chart names the rule.
    let error = chart.set_series(&to_json!(&one)).unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("MonotonicGeneralChart"));

    chart
        .set_monotonic_document(&to_json!(&config()), &to_json!(&one))
        .unwrap();
    let two = vec![
        SeriesConfig::monotonic_line("a", "daq", 0),
        SeriesConfig::monotonic_line("b", "daq", 1),
    ];
    chart.set_series(&to_json!(&two)).unwrap();
    assert_eq!(series_of(&chart).len(), 2);
    let needs: serde_json::Value = serde_json::from_str(&chart.needs_source().unwrap()).unwrap();
    assert_eq!(needs.as_array().unwrap().len(), 2, "{needs}");
    chart.frame().unwrap();
}

// ---- item 5 and the Feynman press (suspicion) ----

#[wasm_bindgen_test(async)]
async fn feynman_drags_edit_the_monotonic_chart() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let series = vec![SeriesConfig::monotonic_line("m", "daq", 0), s_channel()];
    chart
        .set_monotonic_document(&to_json!(&config()), &to_json!(&series))
        .unwrap();
    // The layout is committed on registration, as on the general chart.
    let diagram = |chart: &FiggyChart| series_of(chart)[1].render_type.feynman().unwrap().clone();
    let placed = diagram(&chart);
    assert!(placed.vertices.iter().all(|v| v.at.is_some()));
    assert!(!chart.place_feynman_vertices("d").unwrap());

    assert!(chart.press_feynman("d", "feynman_vertex", 2, None).unwrap());
    chart.on_move(0.0, -20.0).unwrap();
    chart.on_release().unwrap();
    let moved = diagram(&chart).vertices[2].at.unwrap();
    let before = placed.vertices[2].at.unwrap();
    assert!((moved[1] - before[1]).abs() > 1e-3, "{moved:?} {before:?}");
    assert!(
        chart
            .press_feynman("d", "feynman_propagator", 2, None)
            .unwrap()
    );
    chart.on_move(0.0, -15.0).unwrap();
    chart.on_release().unwrap();
    assert!(diagram(&chart).propagators[2].bend.is_some());

    // A press ends a Feynman drag whose release never came: the next move
    // is the new gesture's.
    assert!(chart.press_feynman("d", "feynman_vertex", 3, None).unwrap());
    let held = diagram(&chart).vertices[3].at;
    let plot = config().data_area().unwrap().0;
    chart
        .on_press(
            (plot.x + plot.width / 2) as f32,
            (plot.y + plot.height / 2) as f32,
        )
        .unwrap();
    chart.on_move(0.0, -10.0).unwrap();
    chart.on_release().unwrap();
    assert_eq!(diagram(&chart).vertices[3].at, held);
    chart.frame().unwrap();

    // The hidden general chart was never edited.
    chart.clear_monotonic_document().unwrap();
    assert!(series_of(&chart).is_empty());
}

// ---- item 6 ----

#[wasm_bindgen_test(async)]
async fn the_annotation_selection_ends_with_the_list_it_indexed() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_config(&to_json!(&config())).unwrap();
    for text in ["a", "b", "c"] {
        chart.add_annotation(&text_note(text)).unwrap();
    }
    chart.frame().unwrap();

    // The same list keeps the selection.
    chart.select_annotation(Some(2)).unwrap();
    chart.set_config(&chart.get_config().unwrap()).unwrap();
    assert_eq!(chart.selected_annotation(), Some(2));

    // Another item at an index ends it.
    let mut swapped = config_of(&chart);
    assert_ne!(
        swapped.annotations.items[1].id,
        swapped.annotations.items[2].id
    );
    swapped.annotations.items.swap(1, 2);
    chart.set_config(&to_json!(&swapped)).unwrap();
    assert_eq!(chart.selected_annotation(), None);
    chart.select_annotation(Some(2)).unwrap();

    // Another list ends it.
    let mut one = config_of(&chart);
    one.annotations.items.truncate(1);
    chart.set_config(&to_json!(&one)).unwrap();
    assert_eq!(chart.selected_annotation(), None);
    assert!(!chart.has_selection().unwrap());
    chart.frame().unwrap();

    // A document load ends it too.
    chart.select_annotation(Some(0)).unwrap();
    chart
        .set_categorical_document(
            r#"{"categories": [{"id": "q1", "label": "Q1"}],
                "series": [{"id": "n", "label": "N", "values": [1.0],
                            "color": {"r": 0.3, "g": 0.5, "b": 0.8, "a": 1.0}}],
                "mode": "Grouped"}"#,
            None,
        )
        .unwrap();
    assert_eq!(chart.selected_annotation(), None);
    chart.frame().unwrap();
}

// ---- items 10, 11 and 12 ----

#[wasm_bindgen_test(async)]
async fn an_upload_committed_on_another_chart_is_closed() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let mut other = FiggyChart::create(canvas()).await.unwrap();
    let mut upload = chart
        .begin_column_upload("n", 1.0, "f64", None, 64.0)
        .unwrap();
    upload
        .push(0.0, js_sys::Float64Array::from(&[1.0][..]).into())
        .unwrap();
    let error = upload.commit(&mut other).unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("stale"));
    assert!(!upload.is_open(), "the handle is closed either way");
    let error = upload.commit(&mut chart).unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("upload_closed"));
}

#[wasm_bindgen_test(async)]
async fn a_non_finite_clear_colour_component_reads_as_zero() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_config(&to_json!(&config())).unwrap();
    chart.set_clear_color(0.0, 0.0, 0.0, 1.0);
    let black = chart.export_png(1.0).await.unwrap().to_vec();
    chart.set_clear_color(f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0);
    let read = chart.export_png(1.0).await.unwrap().to_vec();
    assert_eq!(read, black);
    chart.frame().unwrap();
}

#[wasm_bindgen_test(async)]
async fn add_line_series_keeps_the_colour_of_an_existing_marker_series() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    register_xy(&mut chart);
    let red = Color::from_rgba(0.9, 0.1, 0.1, 1.0);
    let series = vec![
        SeriesConfig::line("a", "x", "y"),
        SeriesConfig::line("b", "x", "y"),
        SeriesConfig::scatter("s", "x", "y").style_markers(|m| m.point_color = red),
    ];
    chart.set_series(&to_json!(&series)).unwrap();
    chart.add_line_series("s", "x", "y", 1.0, "").unwrap();
    let line = match &series_of(&chart)[2].render_type {
        DataRenderType::Xy(xy) => xy.line.clone().unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(line.line_color, red);
    chart.frame().unwrap();
}

// ---- item 13 and the monotonic `columns` (suspicion) ----

#[wasm_bindgen_test(async)]
async fn monotonic_sources_that_cannot_be_read_refuse_the_call() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart
        .set_monotonic_document(
            &to_json!(&config()),
            &to_json!(&vec![SeriesConfig::monotonic_line("m", "daq", 0)]),
        )
        .unwrap();
    // A transfer chunk of 32768 values for each of 100 000 channels does
    // not fit wasm memory: the call is refused instead of lending nothing.
    let channel = js_sys::Float64Array::new_with_length(32_768);
    let channels = js_sys::Array::new();
    for _ in 0..100_000 {
        channels.push(&channel);
    }
    let error = chart
        .set_monotonic(
            "daq",
            0.0,
            1.0,
            channels.into(),
            "autoscale".into(),
            None,
            None,
        )
        .unwrap_err();
    assert!(
        message(&error).contains("transfer chunk"),
        "{}",
        message(&error)
    );
    let needs: serde_json::Value = serde_json::from_str(&chart.needs_source().unwrap()).unwrap();
    assert_eq!(needs[0]["change"], "NotBuilt", "nothing was reduced");

    // `columns` is an array of frame columns; a number is refused.
    let frames = js_sys::Float64Array::from(&[0.0, 0.5, 1.0, 0.5][..]);
    let source = js_sys::Object::new();
    for (key, value) in [
        ("group", JsValue::from_str("daq")),
        ("start", JsValue::from_f64(0.0)),
        ("interval", JsValue::from_f64(1.0)),
        ("data", frames.into()),
        ("stride", JsValue::from_f64(2.0)),
        ("columns", JsValue::from_f64(1.0)),
    ] {
        js_sys::Reflect::set(&source, &JsValue::from_str(key), &value).unwrap();
    }
    let list = js_sys::Array::new();
    list.push(&source);
    let error = chart.fit_layout(Some(list)).unwrap_err();
    assert!(message(&error).contains("columns"), "{}", message(&error));
}

// ---- resize (suspicions) ----

#[wasm_bindgen_test(async)]
async fn a_surface_past_the_device_limit_is_refused_and_changes_nothing() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.set_config(&to_json!(&config())).unwrap();
    let before = chart.data_area_rect().unwrap();
    assert!(chart.resize(100_000, 100).is_err());
    // A negative JavaScript number arrives as a huge u32.
    assert!(chart.resize(u32::MAX, 100).is_err());
    assert_eq!(chart.data_area_rect().unwrap(), before);
    chart.frame().unwrap();

    chart
        .set_monotonic_document(
            &to_json!(&config()),
            &to_json!(&vec![SeriesConfig::monotonic_line("m", "daq", 0)]),
        )
        .unwrap();
    let before = chart.data_area_rect().unwrap();
    assert!(
        chart
            .resize_monotonic(100_000, 100, js_sys::Array::new(), JsValue::UNDEFINED)
            .is_err()
    );
    assert_eq!(chart.data_area_rect().unwrap(), before);
    chart.frame().unwrap();
}
