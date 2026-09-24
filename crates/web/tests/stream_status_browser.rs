#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn canvas() -> web_sys::HtmlCanvasElement {
    let canvas = web_sys::window().unwrap().document().unwrap()
        .create_element("canvas").unwrap().dyn_into::<web_sys::HtmlCanvasElement>().unwrap();
    canvas.set_width(320);
    canvas.set_height(240);
    canvas
}

fn status(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&chart.stream_status().unwrap()).unwrap()
}

fn sources(values: &js_sys::Float32Array) -> js_sys::Array {
    let result = js_sys::Array::new();
    result.push(values);
    result.push(values);
    result
}

async fn browser_turn() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window().unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0).unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test(async)]
async fn raw_status_budget_completion_and_cancel_are_observable_without_source_copies() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.configure_streaming(1, 2, 2, 1024.0, 4096.0).unwrap();
    let values = js_sys::Float32Array::from(&[0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0][..]);
    let ids = vec!["x".to_owned(), "y".to_owned()];
    let revisions = vec![1.0, 1.0];
    let before = status(&chart);
    assert_eq!(before["status"], "idle");
    let admission: serde_json::Value = serde_json::from_str(
        &chart.inspect_column_admission(vec![6.0, 6.0]).unwrap()).unwrap();
    assert_eq!(admission["reservation"], false);
    assert_eq!(status(&chart), before);
    chart.register_streaming_columns(ids.clone(), revisions.clone(), sources(&values)).unwrap();
    chart.add_line_series("s", "x", "y", 1.0, "line").unwrap();
    chart.frame().unwrap();
    let capability: serde_json::Value = serde_json::from_str(&chart.streaming_capabilities()).unwrap();
    assert_eq!(capability["render_supported"], true);
    assert_eq!(capability["contour"], false);
    let started = chart.request_auto_streaming_chart(2.0).unwrap();
    let job = started.job_id().unwrap();
    assert_eq!(started.total_primitives(), 5.0);
    let initial = status(&chart);
    assert_eq!(status(&chart), initial);
    chart.set_stream_chunk_budget(1.0).unwrap();
    assert_eq!(chart.request_auto_streaming_chart(2.0).unwrap().job_id().as_deref(), Some(job.as_str()));
    let mut complete = false;
    for _ in 0..128 {
        let progress = chart.auto_stream_chart_step(ids.clone(), revisions.clone(), sources(&values)).unwrap();
        chart.frame().unwrap();
        if progress.status() == "complete" {
            assert_eq!(progress.submitted_primitives(), 5.0);
            assert_eq!(progress.total_primitives(), 5.0);
            assert_eq!(serde_json::Value::from(progress.revision()), status(&chart)["revision"]);
            complete = true;
            break;
        }
        browser_turn().await;
    }
    assert!(complete, "automatic stream must complete");
    let completed = status(&chart);
    assert_eq!(completed["status"], "complete");
    assert_eq!(completed["submitted_primitives"], 5);
    assert_eq!(completed["total_primitives"], 5);
    let repeated = chart.request_auto_streaming_chart(2.0).unwrap();
    assert_eq!(repeated.status(), "complete");
    assert_eq!(repeated.submitted_primitives(), 5.0);
    assert_eq!(repeated.total_primitives(), 5.0);
    assert_eq!(repeated.job_id().as_deref(), Some(job.as_str()));
    assert_eq!(status(&chart), completed);
    chart.cancel_streaming_and_wait().await.unwrap();
    let cancelled = status(&chart);
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["job_id"], job);
    assert_eq!(cancelled["submitted_primitives"], 5);
    assert_eq!(cancelled["in_flight_chunks"], 0);
    assert_eq!(cancelled["reserved_gpu_bytes"], 0);
    assert_eq!(chart.streaming_usage().active_charts(), 0);
    chart.resize(640, 480).unwrap();
    chart.frame().unwrap();
    chart.set_title("Cancelled stream keeps decorations drawable").unwrap();
    chart.frame().unwrap();
    assert_eq!(status(&chart)["status"], "cancelled");
    assert_eq!(chart.streaming_usage().active_charts(), 0);
}

#[wasm_bindgen_test(async)]
async fn raw_cancel_drains_submitted_work_and_unsubmitted_range_reservations() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.configure_streaming(1, 2, 2, 1024.0, 4096.0).unwrap();
    let values = js_sys::Float32Array::from(&[0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0][..]);
    let ids = vec!["x".to_owned(), "y".to_owned()];
    let revisions = vec![1.0, 1.0];
    chart.register_streaming_columns(ids.clone(), revisions.clone(), sources(&values)).unwrap();
    chart.add_line_series("s", "x", "y", 1.0, "line").unwrap();
    chart.request_auto_streaming_chart(2.0).unwrap();
    chart.auto_stream_chart_step(ids, revisions, sources(&values)).unwrap();
    chart.auto_stream_chart_request_ranges().unwrap();
    let before = status(&chart);
    chart.cancel_streaming_and_wait().await.unwrap();
    let after = status(&chart);
    assert_eq!(after["status"], "cancelled");
    assert_eq!(after["job_id"], before["job_id"]);
    assert_eq!(after["submitted_primitives"], before["submitted_primitives"]);
    assert_eq!(after["in_flight_chunks"], 0);
    assert_eq!(after["reserved_gpu_bytes"], 0);
    chart.cancel_streaming_and_wait().await.unwrap();
    assert_eq!(status(&chart), after);
}

#[wasm_bindgen_test(async)]
async fn raw_handoff_decoration_request_preserves_job_and_pending_range() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.configure_streaming(1, 2, 2, 1024.0, 4096.0).unwrap();
    let values = [0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0];
    chart.register_column_f32("x", &values).unwrap();
    chart.register_column_f32("y", &values).unwrap();
    chart.add_line_series("s", "x", "y", 1.0, "line").unwrap();
    let ids = vec!["x".to_owned(), "y".to_owned()];
    let revisions = vec![9.0, 9.0];
    let encodings = vec!["f32".to_owned(), "f32".to_owned()];
    let first = chart.request_resident_stream_handoff(ids.clone(), revisions.clone(), vec![6.0, 6.0], encodings.clone(), 2.0).unwrap();
    assert_eq!(first.status(), "started");
    let pending = chart.auto_stream_chart_request_ranges().unwrap();
    chart.set_title("Decoration changes during pending source I/O").unwrap();
    let repeated = chart.request_resident_stream_handoff(ids, revisions, vec![6.0, 6.0], encodings, 2.0).unwrap();
    assert_eq!(repeated.status(), "active");
    assert_eq!(repeated.job_id(), first.job_id());
    let next = chart.auto_stream_chart_request_ranges().unwrap();
    assert_eq!(next.revision(), pending.revision());
    assert_eq!(next.source_ids(), pending.source_ids());
    assert_eq!(next.offsets(), pending.offsets());
    assert_eq!(next.lengths(), pending.lengths());
    chart.cancel_streaming_and_wait().await.unwrap();
    assert_eq!(status(&chart)["status"], "cancelled");
}

#[wasm_bindgen_test(async)]
async fn raw_gpu_ready_promise_does_not_retain_a_chart_borrow() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.configure_streaming(1, 1, 2, 1024.0, 4096.0).unwrap();
    let values = js_sys::Float32Array::from(&[0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0][..]);
    let ids = vec!["x".to_owned(), "y".to_owned()];
    let revisions = vec![1.0, 1.0];
    chart.register_streaming_columns(ids.clone(), revisions.clone(), sources(&values)).unwrap();
    chart.add_line_series("s", "x", "y", 1.0, "line").unwrap();
    chart.request_auto_streaming_chart(2.0).unwrap();
    chart.auto_stream_chart_step(ids, revisions, sources(&values)).unwrap();
    let wake = chart.streaming_gpu_ready();
    // These mutable calls occur before awaiting the promise. An async export
    // borrowing `self` would prevent this use and trap at the JS boundary.
    chart.set_stream_chunk_budget(1.0).unwrap();
    chart.set_title("Queue wake does not lock the chart").unwrap();
    chart.frame().unwrap();
    wasm_bindgen_futures::JsFuture::from(wake).await.unwrap();
    // The service boundary, not the promise, releases the exact receipt and
    // admits the next range into the single configured slot.
    let next = chart.auto_stream_chart_request_ranges().unwrap();
    assert_eq!(next.status(), "ready");
    assert_eq!(next.submitted_primitives(), 2.0);
    assert!(next.offsets().iter().all(|offset| *offset == 2.0));
    assert!(next.lengths().iter().all(|length| *length == 2.0));
    chart.cancel_streaming_and_wait().await.unwrap();
}
