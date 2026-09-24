#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window()
        .expect("window")
        .document()
        .expect("document");
    let canvas = document
        .create_element("canvas")
        .expect("canvas element")
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .expect("HtmlCanvasElement");
    canvas.set_width(640);
    canvas.set_height(360);
    canvas
}

fn sources(x: &js_sys::Float32Array, y: &js_sys::Float64Array) -> js_sys::Array {
    let sources = js_sys::Array::new();
    sources.push(x);
    sources.push(y);
    sources
}

async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .expect("window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
            .expect("setTimeout");
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .expect("browser task yield");
}

#[wasm_bindgen_test(async)]
async fn decoration_change_preserves_wasm_stream_cursor_and_mixed_typed_sources() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    chart
        .configure_streaming(1, 2, 2, 1024.0 * 1024.0, 16.0 * 1024.0 * 1024.0)
        .expect("configure streaming");

    let x = js_sys::Float32Array::from(&[0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0][..]);
    let y = js_sys::Float64Array::from(&[0.0f64, 1.0, 4.0, 9.0, 16.0, 25.0][..]);
    let ids = vec!["stream-x".to_owned(), "stream-y".to_owned()];
    let revisions = vec![1.0, 1.0];
    chart
        .register_streaming_columns(ids.clone(), revisions.clone(), sources(&x, &y))
        .expect("register mixed streamed columns");
    chart
        .add_line_series("stream-line", "stream-x", "stream-y", 1.5, "stream")
        .expect("stream series accepts logical non-resident columns");
    chart.begin_streaming(2.0).expect("begin streaming");

    let first = chart
        .stream_step(ids.clone(), revisions.clone(), sources(&x, &y))
        .expect("first chunk");
    assert_eq!(first.status(), "submitted");
    assert!(first.submitted_primitives() > 0.0);
    chart.frame().expect("present first partial frame");

    let before = first.submitted_primitives();
    chart
        .set_title("decoration changed during streaming")
        .expect("decoration-only config change");
    chart.frame().expect("recompose decoration over prefix");

    let second = chart
        .stream_step(ids, revisions, sources(&x, &y))
        .expect("stream continues after decoration");
    assert_eq!(second.status(), "submitted");
    assert!(
        second.submitted_primitives() > before,
        "renderer-owned cursor must continue instead of restarting"
    );
    assert_eq!(chart.streaming_usage().active_charts(), 1);

    let mut config: renderer::Config =
        serde_json::from_str(&chart.get_config().expect("get config")).expect("config json");
    config.bottom_x.min -= 1.0;
    chart
        .set_config(&serde_json::to_string(&config).expect("serialize config"))
        .expect("geometry config change");
    assert!(
        chart
            .stream_step(
                vec!["stream-x".to_owned(), "stream-y".to_owned()],
                vec![1.0, 1.0],
                sources(&x, &y),
            )
            .is_err(),
        "data-coordinate changes must still invalidate the old stream"
    );
    assert_eq!(chart.streaming_usage().active_charts(), 0);

    // Keep an explicit JS use in this browser test so wasm-bindgen validates
    // the heterogeneous Array ABI rather than optimizing it into Rust-only data.
    let _: JsValue = x.into();
}

#[wasm_bindgen_test(async)]
async fn automatic_streaming_collects_progressive_fit_and_completes_through_public_api() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    chart
        .configure_streaming(1, 2, 2, 1024.0 * 1024.0, 16.0 * 1024.0 * 1024.0)
        .expect("configure streaming");

    let x = js_sys::Float32Array::from(&[10.0f32, 11.0, 12.0, 13.0, 14.0, 15.0][..]);
    let y = js_sys::Float64Array::from(&[-20.0f64, -19.0, -18.0, -17.0, -16.0, -15.0][..]);
    let ids = vec!["stream-x".to_owned(), "stream-y".to_owned()];
    let revisions = vec![1.0, 1.0];
    chart
        .register_streaming_columns(ids.clone(), revisions.clone(), sources(&x, &y))
        .expect("register mixed streamed columns");
    chart
        .add_line_series("stream-line", "stream-x", "stream-y", 1.5, "stream")
        .expect("stream series accepts logical non-resident columns");
    chart.auto_fit_all(0.0).await.expect("request stream fit");

    let request = chart
        .request_auto_streaming_chart(2.0)
        .expect("start automatic stream");
    assert_eq!(request.status(), "started");
    assert_eq!(request.source_ids(), ids);
    assert_eq!(request.source_revisions(), revisions);

    let mut completed = false;
    let mut statuses = Vec::new();
    for _ in 0..128 {
        let step = chart
            .auto_stream_chart_step(ids.clone(), revisions.clone(), sources(&x, &y))
            .expect("automatic stream step");
        chart.frame().expect("present automatic stream progress");
        let status = step.status();
        statuses.push((
            status.clone(),
            step.submitted_primitives(),
            step.total_primitives(),
        ));
        match status.as_str() {
            "submitted" | "backpressure" | "all_submitted" => yield_to_browser().await,
            "complete" => {
                completed = true;
                break;
            }
            status => panic!("unexpected automatic stream status {status}"),
        }
    }
    assert!(
        completed,
        "automatic stream did not reach terminal display state: {statuses:?}"
    );

    let config: renderer::Config =
        serde_json::from_str(&chart.get_config().expect("get fitted config"))
            .expect("fitted config json");
    assert_eq!((config.bottom_x.min, config.bottom_x.max), (10.0, 15.0));
    assert_eq!((config.left_y.min, config.left_y.max), (-20.0, -15.0));
}

#[wasm_bindgen_test(async)]
async fn automatic_range_api_requests_only_exact_typed_array_slices() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    chart
        .configure_streaming(1, 2, 2, 1024.0 * 1024.0, 16.0 * 1024.0 * 1024.0)
        .expect("configure streaming");

    let x = js_sys::Float32Array::from(&[10.0f32, 11.0, 12.0, 13.0, 14.0, 15.0][..]);
    let y = js_sys::Float64Array::from(&[-20.0f64, -19.0, -18.0, -17.0, -16.0, -15.0][..]);
    let ids = vec!["range-x".to_owned(), "range-y".to_owned()];
    let revisions = vec![1.0, 1.0];
    chart
        .register_streaming_column_sources(
            ids.clone(),
            revisions.clone(),
            vec![x.length() as f64, y.length() as f64],
            vec!["f32".to_owned(), "f64".to_owned()],
        )
        .expect("register replayable range metadata");
    chart
        .add_line_series("range-line", "range-x", "range-y", 1.5, "stream")
        .expect("range series accepts logical non-resident columns");
    chart.auto_fit_all(0.0).await.expect("request range fit");

    let request = chart
        .request_auto_streaming_chart(2.0)
        .expect("start automatic range stream");
    assert_eq!(request.status(), "started");
    assert_eq!(request.source_ids(), ids);
    assert_eq!(request.source_revisions(), revisions);

    let mut completed = false;
    let mut submitted_ranges = 0usize;
    for _ in 0..128 {
        let request = chart
            .auto_stream_chart_request_ranges()
            .expect("request exact source ranges");
        match request.status().as_str() {
            "ready" => {
                let range_ids = request.source_ids();
                let offsets = request.offsets();
                let lengths = request.lengths();
                let encodings = request.encodings();
                assert_eq!(range_ids.len(), offsets.len());
                assert_eq!(range_ids.len(), lengths.len());
                assert_eq!(range_ids.len(), encodings.len());

                let chunks = js_sys::Array::new();
                for index in 0..range_ids.len() {
                    let start = offsets[index] as u32;
                    let end = start + lengths[index] as u32;
                    let chunk: JsValue = match (range_ids[index].as_str(), encodings[index].as_str()) {
                        ("range-x", "f32") => x.subarray(start, end).into(),
                        ("range-y", "f64") => y.subarray(start, end).into(),
                        pair => panic!("unexpected range source {pair:?}"),
                    };
                    chunks.push(&chunk);
                }
                chart
                    .auto_stream_chart_submit_ranges(
                        range_ids,
                        request.source_revisions(),
                        request.source_lengths(),
                        offsets,
                        chunks,
                    )
                    .expect("submit exact source ranges");
                submitted_ranges += 1;
            }
            "backpressure" | "all_submitted" => yield_to_browser().await,
            "complete" => {
                completed = true;
                break;
            }
            status => panic!("unexpected automatic range status {status}"),
        }
        chart.frame().expect("present automatic range progress");
    }
    assert!(completed, "automatic range stream did not complete");
    assert!(submitted_ranges > 1, "small chunks must require multiple exact range requests");

    let config: renderer::Config =
        serde_json::from_str(&chart.get_config().expect("get fitted range config"))
            .expect("fitted range config json");
    assert_eq!((config.bottom_x.min, config.bottom_x.max), (10.0, 15.0));
    assert_eq!((config.left_y.min, config.left_y.max), (-20.0, -15.0));
}
