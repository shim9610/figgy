//! Browser API usability (#315): a readiness handle that does not borrow
//! the chart, annotations selected by their stable id, and the families
//! `FiggyBoundedChart` opens.
#![cfg(target_arch = "wasm32")]

use figgy::{FiggyChart, FiggyRadialChart, FiggyReadiness};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

mod support;
use support::{canvas_320x240 as canvas, turn};

wasm_bindgen_test_configure!(run_in_browser);

/// Make every `createRenderPipelineAsync` from now on a compile that never
/// settles, so a wait on the chart's compiles stays pending for as long as
/// the test needs it; `release_compiles` restores the browser's own.
fn hold_compiles() {
    js_sys::eval(
        r#"(() => {
  const P = GPUDevice.prototype;
  if (!P.__usabilityOriginalAsync) P.__usabilityOriginalAsync = P.createRenderPipelineAsync;
  P.createRenderPipelineAsync = function () { return new Promise(() => {}); };
})()"#,
    )
    .unwrap();
}

fn release_compiles() {
    js_sys::eval(
        r#"(() => {
  const P = GPUDevice.prototype;
  if (P.__usabilityOriginalAsync) P.createRenderPipelineAsync = P.__usabilityOriginalAsync;
})()"#,
    )
    .unwrap();
}

/// The handle's status object, through JSON (it is an object, not JSON
/// text: `stringify` is what makes it text).
fn status(readiness: &FiggyReadiness) -> serde_json::Value {
    let object = readiness.status().unwrap();
    assert!(object.is_object(), "status() is a plain object");
    let text = js_sys::JSON::stringify(&object)
        .unwrap()
        .as_string()
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Whether `promise` has settled, recorded on a global named `flag`.
fn watch(promise: &js_sys::Promise, flag: &str) {
    js_sys::Reflect::set(&js_sys::global(), &flag.into(), &JsValue::FALSE).unwrap();
    let mark = js_sys::Function::new_with_args(
        "p, f",
        "p.then(() => { globalThis[f] = true; }, () => { globalThis[f] = true; });",
    );
    mark.call2(&JsValue::UNDEFINED, promise, &flag.into())
        .unwrap();
}

fn flag(flag: &str) -> bool {
    js_sys::Reflect::get(&js_sys::global(), &flag.into())
        .unwrap()
        .as_bool()
        .unwrap()
}

fn error_message(error: &JsValue) -> String {
    match error.dyn_ref::<js_sys::Error>() {
        Some(error) => error.message().into(),
        None => error.as_string().unwrap_or_default(),
    }
}

/// While a readiness wait is pending the chart is not borrowed: the handle's
/// status and the chart's own calls (status, frame, resize) all work, and
/// freeing the chart ends the pending wait, and every later one, with a
/// "chart disposed" error.
#[wasm_bindgen_test(async)]
async fn a_readiness_wait_leaves_the_chart_free_and_ends_when_it_is_freed() {
    hold_compiles();
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let readiness = chart.readiness();
    let pending = readiness.pipelines_ready();
    watch(&pending, "__usabilityPending");
    turn().await;
    assert!(
        !flag("__usabilityPending"),
        "the wait is pending while every compile is held"
    );

    // The handle's status, read while the wait is pending.
    let during = status(&readiness);
    assert_eq!(during["disposed"], false);
    assert!(
        during["all"]["total"].as_u64().unwrap() > 0,
        "a browser chart compiles in the background: {during}"
    );
    assert!(
        during["all"]["compiling"].as_u64().unwrap() > 0,
        "held compiles count as compiling: {during}"
    );
    assert!(during.get("frame_complete").is_none());

    // The chart itself stays usable meanwhile.
    assert!(chart.pipeline_status().unwrap().is_object());
    chart.frame().unwrap();
    chart.resize(300, 200).unwrap();
    chart.frame().unwrap();
    assert!(
        !flag("__usabilityPending"),
        "drawing and resizing do not end the wait"
    );

    // Free the chart: the pending waits end with an error.
    drop(chart);
    let ended = JsFuture::from(pending)
        .await
        .expect_err("freeing the chart ends the wait with an error");
    assert!(
        error_message(&ended).starts_with("chart disposed"),
        "{}",
        error_message(&ended)
    );
    // A wait begun after the chart was freed rejects the same way.
    let later = JsFuture::from(readiness.series_ready())
        .await
        .expect_err("a later wait rejects");
    assert!(error_message(&later).starts_with("chart disposed"));
    let after = status(&readiness);
    assert_eq!(after["disposed"], true);
    assert_eq!(after["all"]["total"], 0);
    release_compiles();
}

/// A readiness wait on a chart whose compiles run resolves once they have
/// ended; the status then counts every compile ended, and the chart's own
/// waits keep working. Freeing the handle ends nothing on the chart.
#[wasm_bindgen_test(async)]
async fn a_readiness_wait_resolves_once_the_compiles_end() {
    release_compiles();
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let readiness = chart.readiness();
    JsFuture::from(readiness.pipelines_ready())
        .await
        .expect("every compile ends ready");
    let done = status(&readiness);
    assert_eq!(done["disposed"], false);
    assert_eq!(done["all"]["compiling"], 0, "{done}");
    assert_eq!(done["all"]["failed"], 0, "{done}");
    assert_eq!(done["all"]["ready"], done["all"]["total"], "{done}");
    JsFuture::from(readiness.series_ready())
        .await
        .expect("nothing the screen needs is pending");
    drop(readiness);
    chart.series_ready().await.expect("the chart's own wait");
    chart.first_frame_ready().await.expect("the first frame");
}

fn annotation(id: &str) -> String {
    let mut item = renderer::Annotation::new(renderer::AnnotationKind::Text {
        anchor: renderer::Anchor::Ratio { x: 0.4, y: 0.4 },
        text: renderer::text::RichText::plain("note", renderer::Color::BLACK, 14.0, ""),
        box_style: None,
        align: Default::default(),
    });
    item.id = id.into();
    serde_json::to_string(&item).unwrap()
}

fn ids(chart: &FiggyChart) -> Vec<String> {
    chart
        .annotation_ids()
        .unwrap()
        .iter()
        .map(|id| id.as_string().unwrap())
        .collect()
}

/// An annotation is found and selected by its id, which stays with it when
/// an earlier item is removed and its index shifts. A missing, empty or
/// shared id is refused, leaving the selection as it was.
#[wasm_bindgen_test(async)]
async fn an_annotation_is_selected_by_its_stable_id() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    for id in ["peak", "", "note"] {
        chart.add_annotation(&annotation(id)).unwrap();
    }
    assert_eq!(ids(&chart), ["peak", "", "note"]);
    assert_eq!(chart.annotation_index("note").unwrap(), Some(2));
    assert_eq!(chart.annotation_index("missing").unwrap(), None);
    assert!(
        chart.annotation_index("").is_err(),
        "an empty id is refused"
    );

    chart.select_annotation_by_id("note").unwrap();
    assert_eq!(chart.selected_annotation(), Some(2));

    // Removing an earlier item shifts the index; the id still finds it and
    // the selection follows.
    chart.remove_annotation(0).unwrap();
    assert_eq!(ids(&chart), ["", "note"]);
    assert_eq!(chart.annotation_index("note").unwrap(), Some(1));
    assert_eq!(chart.selected_annotation(), Some(1));

    // Refusals leave the selection unchanged.
    assert!(chart.select_annotation_by_id("missing").is_err());
    assert!(chart.select_annotation_by_id("").is_err());
    chart.add_annotation(&annotation("note")).unwrap();
    assert!(
        chart.annotation_index("note").is_err(),
        "an id two items share is ambiguous"
    );
    assert!(chart.select_annotation_by_id("note").is_err());
    assert_eq!(chart.selected_annotation(), Some(1));

    // The ids survive a serialization round trip of the config.
    let config = chart.get_config().unwrap();
    chart.set_config(&config).unwrap();
    assert_eq!(ids(&chart), ["", "note", "note"]);
}

/// `FiggyBoundedChart` opens only the radial family; each family that moved
/// to `FiggyChart` is refused with the setter that opens it.
#[wasm_bindgen_test(async)]
async fn the_bounded_chart_lists_its_families() {
    let families: Vec<String> = FiggyRadialChart::supported_families()
        .iter()
        .map(|family| family.as_string().unwrap())
        .collect();
    assert_eq!(families, ["radial"]);
    for (family, setter) in [
        ("categorical", "set_categorical_document"),
        ("boxplot", "set_boxplot_document"),
        ("radar", "set_radar_document"),
    ] {
        let error = match FiggyRadialChart::create(canvas(), family, "{}", 320, 240).await {
            Ok(_) => panic!("{family} is not a bounded family"),
            Err(error) => error,
        };
        assert!(
            error_message(&error).contains(setter),
            "{family}: {}",
            error_message(&error)
        );
    }
}
