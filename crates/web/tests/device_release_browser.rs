#![cfg(target_arch = "wasm32")]

//! Freeing a chart with work in flight leaves the page responsive (#322).
//!
//! A guard for an upstream Chromium bug: Chromium 152 to 158 can deadlock
//! the page's main thread when a `GPUDevice` with submitted work still in
//! flight is garbage collected. A dropped renderer now keeps its device until
//! that work completes. Each round here frees a chart right after its frames
//! and an export are submitted, then allocates enough memory to bring on a
//! collection (and calls `gc()` when the runner exposes it). The upstream
//! deadlock does not reproduce reliably through figgy, so this checks that
//! the guard keeps every round answering; it is not a reproduction of the
//! bug.

use std::future::Future;

use figgy::FiggyChart;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

/// Rounds of free-then-collect.
const ROUNDS: usize = 12;

fn canvas() -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(2048);
    canvas.set_height(2048);
    canvas
}

/// Allocate and drop eight 24 MB arrays, the allocation that brought on
/// the collection in the pure WebGPU repro, then yield one timer turn so
/// the GPU replies are handled.
async fn collect() {
    for _ in 0..8 {
        let array = js_sys::Float64Array::new_with_length(3_000_000);
        array.fill(1.0, 0, 3_000_000);
    }
    // A runner started with `--js-flags=--expose-gc` collects at once.
    if js_sys::eval("typeof gc === 'function' && (gc(), true)").is_err() {
        // `eval` itself failed: nothing more to collect with.
    }
    let turn = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 20)
            .unwrap();
    });
    JsFuture::from(turn).await.unwrap();
}

#[wasm_bindgen_test(async)]
async fn a_chart_freed_with_work_in_flight_does_not_hang_the_page() {
    for _ in 0..ROUNDS {
        let mut chart = FiggyChart::create(canvas()).await.unwrap();
        let values: Vec<f32> = (0..1_000_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let xs: Vec<f32> = (0..1_000_000).map(|i| i as f32).collect();
        chart.register_column_f32("x", &xs).unwrap();
        chart.register_column_f32("y", &values).unwrap();
        chart.add_line_series("s", "x", "y", 8.0, "").unwrap();
        // The first frame waits for the pipelines; the next frames submit
        // their work and return before the GPU finishes it.
        chart.first_frame_ready().await.unwrap();
        for _ in 0..4 {
            chart.frame().unwrap();
        }
        // Start an export and free the chart before it completes: its
        // readback is submitted and mapping, as when a page closes a chart
        // mid-export.
        {
            let mut export = std::pin::pin!(chart.export_png(2.0));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(export.as_mut().poll(&mut context).is_pending());
        }
        drop(chart);
        collect().await;
    }
    // The page still answers.
    assert_eq!(js_sys::eval("1 + 1").unwrap(), JsValue::from(2));
}
