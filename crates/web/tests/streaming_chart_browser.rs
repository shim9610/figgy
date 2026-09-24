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
    let before_pick = chart.stream_status().expect("completed stream status");
    assert!(chart.begin_stream_pick_point(320.0, 180.0, 16.0, 2.0).await.is_err());
    assert!(chart.begin_stream_pick_data(320.0, 180.0, 16.0, 2.0).await.is_err());
    let after_pick = chart.stream_status().expect("status after disabled picking");
    assert_eq!(after_pick, before_pick);

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

#[wasm_bindgen_test(async)]
async fn packed_view_pages_reach_gpu_during_stream_and_narrow_view_reuses_them() {
    let mut chart = FiggyChart::create(canvas()).await.expect("FiggyChart.create");
    chart.configure_streaming(1, 2, 2, 1024.0 * 1024.0, 16.0 * 1024.0 * 1024.0)
        .expect("configure streaming");
    chart.configure_auto_residency(256.0 * 1024.0 * 1024.0, 2000.0)
        .expect("configure external view limit");
    let x = js_sys::Float32Array::from(&[0.15f32, 0.25, 0.35, 0.45, 0.55, 0.65][..]);
    let y = js_sys::Float64Array::from(&[0.15f64, 0.25, 0.35, 0.45, 0.55, 0.65][..]);
    let ids = vec!["view-x".to_owned(), "view-y".to_owned()];
    let revisions = vec![1.0, 1.0];
    chart.register_streaming_columns(ids.clone(), revisions.clone(), sources(&x, &y))
        .expect("register source columns");
    for index in 0..9 {
        chart.add_line_series(&format!("batch-{index}"), "view-x", "view-y", 1.5, "")
            .expect("line series forces another GPU page");
    }
    assert_eq!(chart.request_auto_streaming_chart(2.0).unwrap().status(), "started");

    let mut saw_live_candidate_before_complete = false;
    let mut completed = false;
    for _ in 0..128 {
        let step = chart.auto_stream_chart_step(ids.clone(), revisions.clone(), sources(&x, &y))
            .expect("stream step");
        let memory: serde_json::Value = serde_json::from_str(&chart.gpu_memory_status()).unwrap();
        let live_view_bytes = memory["resources"].as_array().unwrap().iter()
            .find(|row| row["kind"] == "view resident")
            .and_then(|row| row["live_bytes"].as_u64()).unwrap();
        if step.status() != "complete" && live_view_bytes > 0 {
            saw_live_candidate_before_complete = true;
        }
        chart.frame().expect("present stream progress");
        match step.status().as_str() {
            "submitted" | "backpressure" | "all_submitted" => yield_to_browser().await,
            "complete" => { completed = true; break; }
            other => panic!("unexpected stream status {other}"),
        }
    }
    assert!(completed, "automatic stream did not finish");
    assert!(saw_live_candidate_before_complete, "packed page must reach GPU before stream completion");
    let status: serde_json::Value = serde_json::from_str(&chart.stream_status().unwrap()).unwrap();
    assert_eq!(status["view_residency"]["state"], "resident");
    assert!(status["view_residency"]["needed_bytes"].as_u64().unwrap() <= 2000);
    assert_eq!(status["view_residency"]["picking_available"], true);

    let mut config: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config.bottom_x.min = 0.25;
    config.bottom_x.max = 0.65;
    config.left_y.min = 0.25;
    config.left_y.max = 0.65;
    chart.set_config(&serde_json::to_string(&config).unwrap()).expect("narrow view");
    assert_eq!(chart.request_auto_streaming_chart(2.0).unwrap().status(), "started");
    let mut redrawn = false;
    for _ in 0..128 {
        let request = chart.auto_stream_chart_request_ranges().expect("cached view request");
        match request.status().as_str() {
            "all_submitted" | "backpressure" => {
                chart.frame().expect("present packed redraw");
                yield_to_browser().await;
            }
            "complete" => { redrawn = true; break; }
            "ready" => panic!("narrower view reread the source instead of using GPU pages"),
            other => panic!("unexpected packed redraw status {other}"),
        }
    }
    assert!(redrawn, "cached narrow-view redraw did not finish");
    let area = config.data_area().unwrap().0;
    let picked = chart.pick_point(
        area.x as f32 + area.width as f32 * 0.5,
        area.y as f32 + area.height as f32 * 0.5,
        16.0,
    ).await.expect("GPU packed-view pick");
    let hit: serde_json::Value = serde_json::from_str(&picked.as_string().expect("point hit"))
        .expect("point hit JSON");
    assert_eq!(hit["point_index"], 3, "GPU pick must return the original source row");
    assert_eq!(hit["series_id"], "batch-8", "the next submission keeps paint-order ties");
}

#[wasm_bindgen_test(async)]
async fn external_view_limits_disable_or_refuse_cache_without_stopping_exact_stream() {
    for (limit, reason) in [(0.0, "disabled"), (1.0, "working_set_exceeded")] {
        let mut chart = FiggyChart::create(canvas()).await.expect("FiggyChart.create");
        chart.configure_streaming(1, 2, 2, 1024.0 * 1024.0, 16.0 * 1024.0 * 1024.0)
            .expect("configure streaming");
        chart.configure_auto_residency(256.0 * 1024.0 * 1024.0, limit)
            .expect("configure external view limit");
        let x = js_sys::Float32Array::from(&[0.1f32, 0.3, 0.5, 0.7][..]);
        let y = js_sys::Float64Array::from(&[0.2f64, 0.4, 0.6, 0.8][..]);
        let ids = vec!["limit-x".to_owned(), "limit-y".to_owned()];
        let revisions = vec![1.0, 1.0];
        chart.register_streaming_columns(ids.clone(), revisions.clone(), sources(&x, &y))
            .expect("register source columns");
        chart.add_line_series("limit-line", "limit-x", "limit-y", 1.5, "")
            .expect("line series");
        assert_eq!(chart.request_auto_streaming_chart(2.0).unwrap().status(), "started");
        let mut complete = false;
        for _ in 0..128 {
            let step = chart.auto_stream_chart_step(ids.clone(), revisions.clone(), sources(&x, &y))
                .expect("exact fallback stream step");
            chart.frame().expect("present exact stream");
            match step.status().as_str() {
                "submitted" | "backpressure" | "all_submitted" => yield_to_browser().await,
                "complete" => { complete = true; break; }
                other => panic!("unexpected stream status {other}"),
            }
        }
        assert!(complete, "exact stream must complete at configured limit {limit}");
        let status: serde_json::Value = serde_json::from_str(&chart.stream_status().unwrap()).unwrap();
        assert_eq!(status["view_residency"]["state"], "streamed");
        assert_eq!(status["view_residency"]["refusal_reason"], reason);
        let memory: serde_json::Value = serde_json::from_str(&chart.gpu_memory_status()).unwrap();
        let live_view_bytes = memory["resources"].as_array().unwrap().iter()
            .find(|row| row["kind"] == "view resident")
            .and_then(|row| row["live_bytes"].as_u64()).unwrap();
        assert_eq!(live_view_bytes, 0, "rejected candidate must not retain GPU pages");
    }
}

#[wasm_bindgen_test(async)]
async fn cancelling_after_gpu_page_upload_releases_unpublished_view_candidate() {
    let mut chart = FiggyChart::create(canvas()).await.expect("FiggyChart.create");
    chart.configure_streaming(1, 2, 2, 1024.0 * 1024.0, 16.0 * 1024.0 * 1024.0)
        .expect("configure streaming");
    chart.configure_auto_residency(256.0 * 1024.0 * 1024.0, 1000.0)
        .expect("configure external view limit");
    let x = js_sys::Float32Array::from(&[0.15f32, 0.25, 0.35, 0.45, 0.55, 0.65][..]);
    let y = js_sys::Float64Array::from(&[0.15f64, 0.25, 0.35, 0.45, 0.55, 0.65][..]);
    let ids = vec!["cancel-x".to_owned(), "cancel-y".to_owned()];
    let revisions = vec![1.0, 1.0];
    chart.register_streaming_columns(ids.clone(), revisions.clone(), sources(&x, &y))
        .expect("register source columns");
    chart.add_line_series("first", "cancel-x", "cancel-y", 1.5, "").unwrap();
    chart.add_line_series("second", "cancel-x", "cancel-y", 1.5, "").unwrap();
    chart.request_auto_streaming_chart(2.0).expect("start automatic stream");

    let mut candidate_uploaded = false;
    for _ in 0..128 {
        let step = chart.auto_stream_chart_step(ids.clone(), revisions.clone(), sources(&x, &y))
            .expect("stream step before cancellation");
        let memory: serde_json::Value = serde_json::from_str(&chart.gpu_memory_status()).unwrap();
        let live_view_bytes = memory["resources"].as_array().unwrap().iter()
            .find(|row| row["kind"] == "view resident")
            .and_then(|row| row["live_bytes"].as_u64()).unwrap();
        if live_view_bytes > 0 {
            assert_ne!(step.status(), "complete", "test must cancel an unpublished candidate");
            candidate_uploaded = true;
            break;
        }
        chart.frame().expect("present partial stream");
        yield_to_browser().await;
    }
    assert!(candidate_uploaded, "test never reached an in-progress GPU page");
    chart.cancel_streaming_and_wait().await.expect("cancel and retire GPU page");
    let memory: serde_json::Value = serde_json::from_str(&chart.gpu_memory_status()).unwrap();
    let view_row = memory["resources"].as_array().unwrap().iter()
        .find(|row| row["kind"] == "view resident").unwrap();
    assert_eq!(view_row["live_bytes"], 0);
    assert_eq!(view_row["retired_bytes"], 0);
}

#[wasm_bindgen_test(async)]
async fn large_single_phase_stream_flushes_bounded_page_before_completion() {
    // XY pairs occupy 16 bytes per row; exceed one 8 MiB GPU page even
    // without the removed per-row original-index map.
    const ROWS: u32 = 600_000;
    let mut chart = FiggyChart::create(canvas()).await.expect("FiggyChart.create");
    chart.configure_streaming(1, 2, 2, 4.0 * 1024.0 * 1024.0, 16.0 * 1024.0 * 1024.0)
        .expect("configure streaming");
    chart.configure_auto_residency(256.0 * 1024.0 * 1024.0, 20.0 * 1024.0 * 1024.0)
        .expect("configure external view limit");
    let x = js_sys::Float32Array::new_with_length(ROWS);
    let y = js_sys::Float64Array::new_with_length(ROWS);
    x.set_index(ROWS - 2, 0.5);
    y.set_index(ROWS - 2, 0.5);
    x.set_index(ROWS - 1, 0.6);
    y.set_index(ROWS - 1, 0.6);
    let ids = vec!["large-x".to_owned(), "large-y".to_owned()];
    let revisions = vec![1.0, 1.0];
    chart.register_streaming_columns(ids.clone(), revisions.clone(), sources(&x, &y))
        .expect("register large columns");
    chart.add_line_series("large-line", "large-x", "large-y", 1.0, "")
        .expect("large line");
    assert_eq!(chart.request_auto_streaming_chart(65_536.0).unwrap().status(), "started");

    let mut saw_page_before_complete = false;
    let mut complete = false;
    for _ in 0..256 {
        let step = chart.auto_stream_chart_step(ids.clone(), revisions.clone(), sources(&x, &y))
            .expect("large stream step");
        let memory: serde_json::Value = serde_json::from_str(&chart.gpu_memory_status()).unwrap();
        let live_view_bytes = memory["resources"].as_array().unwrap().iter()
            .find(|row| row["kind"] == "view resident")
            .and_then(|row| row["live_bytes"].as_u64()).unwrap();
        if step.status() != "complete" && live_view_bytes > 0 {
            saw_page_before_complete = true;
        }
        chart.frame().expect("present large stream");
        match step.status().as_str() {
            "submitted" | "backpressure" | "all_submitted" => yield_to_browser().await,
            "complete" => { complete = true; break; }
            other => panic!("unexpected large stream status {other}"),
        }
    }
    assert!(complete, "large stream did not finish");
    assert!(saw_page_before_complete, "GPU page was not flushed during the stream");
    let status: serde_json::Value = serde_json::from_str(&chart.stream_status().unwrap()).unwrap();
    assert_eq!(status["view_residency"]["state"], "resident");
    let bytes = status["view_residency"]["needed_bytes"].as_u64().unwrap();
    assert!(bytes > 8 * 1024 * 1024 && bytes <= 20 * 1024 * 1024);
    let config: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let area = config.data_area().unwrap().0;
    let picked = chart.pick_point(
        area.x as f32 + area.width as f32 * 0.58,
        area.y as f32 + area.height as f32 * 0.42,
        4.0,
    ).await.expect("multi-page GPU pick");
    let hit: serde_json::Value = serde_json::from_str(&picked.as_string().expect("point hit"))
        .expect("point hit JSON");
    assert_eq!(hit["point_index"], ROWS - 1);
    let previous = chart.next_view_point_index(None, "large-line".into(),
        f64::from(ROWS - 1), false).expect("packed view neighbor");
    assert_eq!(previous.as_f64(), Some(f64::from(ROWS - 2)));
    chart.set_picked_points(&serde_json::json!({
        "visible": true,
        "refs": [{"source_id": null, "series_id": "large-line", "point_index": ROWS - 1}],
        "ring_color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0},
        "ring_width_px": 3.0,
        "radius_extra_px": 4.0,
    }).to_string()).expect("select packed point");
    assert_eq!(chart.stream_selection_request_ranges().expect("packed selection").status(),
        "complete", "selected resident point must not request its source range");
}
