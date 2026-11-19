#![cfg(target_arch = "wasm32")]

//! Freeing a chart with work in flight leaves the page responsive (#322).
//!
//! A guard for an upstream Chromium bug: Chromium 152 to 158 can deadlock
//! the page's main thread when a `GPUDevice` is garbage collected, because
//! Dawn delivers its per-submit callbacks only when the device is destroyed.
//! A dropped renderer that is the last holder of a device figgy requested
//! destroys that device, then keeps it until its submitted work completes.
//! Each round here frees a chart right after its frames and an export are
//! submitted, then allocates enough memory to bring on a collection (and
//! calls `gc()` when the runner exposes it). The upstream deadlock does not
//! reproduce reliably through figgy, so this checks that every round
//! answers and that the device is destroyed; it is not a reproduction of
//! the bug.
//!
//! The other tests check what is never destroyed: a device another chart
//! still draws on (`FiggyBoundedChart.create_on`) and a device the host
//! supplied (`RendererDevice::new`).

use std::future::Future;

use figgy::{FiggyChart, FiggyRadialChart};
use renderer::RendererDevice;
use renderer::radial::{RadialChart, RadialRenderer, RadialSlice};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

mod support;
use support::turn;

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

/// Count every `GPUDevice.destroy()` from now on, figgy's and the
/// browser's alike, in `globalThis.__destroyed`.
fn count_destroys() {
    js_sys::eval(
        r#"(() => {
  const P = GPUDevice.prototype;
  if (!P.__figgyDestroy) P.__figgyDestroy = P.destroy;
  globalThis.__destroyed = 0;
  P.destroy = function () {
    globalThis.__destroyed += 1;
    return P.__figgyDestroy.call(this);
  };
})()"#,
    )
    .unwrap();
}

fn destroyed() -> u32 {
    js_sys::eval("globalThis.__destroyed")
        .unwrap()
        .as_f64()
        .unwrap() as u32
}

/// Wait until `count` devices were destroyed, for at most 10 seconds: the
/// release runs after the drop, from the microtask and GPU reply queues.
async fn until_destroyed(count: u32) {
    let start = js_sys::Date::now();
    while destroyed() < count {
        assert!(
            js_sys::Date::now() - start < 10_000.0,
            "{} of {count} devices destroyed",
            destroyed()
        );
        turn().await;
    }
}

/// Let every pending release run: a few timer turns with a collection.
async fn settle() {
    for _ in 0..5 {
        collect().await;
    }
}

fn radial_document() -> String {
    let chart = RadialChart {
        slices: vec![
            RadialSlice::new("A", 25.0, renderer::Color::new(1.0, 0.0, 0.0, 1.0)),
            RadialSlice::new("B", 75.0, renderer::Color::new(0.0, 0.0, 1.0, 1.0)),
        ],
        ..Default::default()
    };
    serde_json::to_string(&chart).unwrap()
}

async fn radial_on(parent: &FiggyChart) -> FiggyRadialChart {
    FiggyRadialChart::create_on_device_of(
        parent,
        small_canvas(),
        "radial".into(),
        radial_document(),
        480,
        360,
    )
    .await
    .unwrap()
}

fn small_canvas() -> HtmlCanvasElement {
    let canvas = canvas();
    canvas.set_width(480);
    canvas.set_height(360);
    canvas
}

async fn line_chart() -> FiggyChart {
    let mut chart = FiggyChart::create(small_canvas()).await.unwrap();
    let xs: Vec<f32> = (0..1000).map(|i| i as f32).collect();
    let ys: Vec<f32> = xs.iter().map(|x| (x * 0.01).sin()).collect();
    chart.register_column_f32("x", &xs).unwrap();
    chart.register_column_f32("y", &ys).unwrap();
    chart.add_line_series("s", "x", "y", 2.0, "").unwrap();
    chart.first_frame_ready().await.unwrap();
    chart
}

#[wasm_bindgen_test(async)]
async fn a_chart_freed_with_work_in_flight_does_not_hang_the_page() {
    count_destroys();
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
    // The page still answers, and every chart's own device was destroyed
    // rather than left to the collector.
    assert_eq!(js_sys::eval("1 + 1").unwrap(), JsValue::from(2));
    until_destroyed(ROUNDS as u32).await;
}

/// A radial chart on a Cartesian chart's device keeps drawing, exporting
/// and picking after that chart is freed and collected, in either order:
/// the device is destroyed only with the last chart on it.
#[wasm_bindgen_test(async)]
async fn a_shared_device_outlives_the_chart_freed_first() {
    count_destroys();
    // The parent first.
    let parent = line_chart().await;
    let mut child = radial_on(&parent).await;
    child.first_frame_ready().await.unwrap();
    drop(parent);
    settle().await;
    assert_eq!(destroyed(), 0, "the child still draws on the device");
    child.frame().unwrap();
    child.first_frame_ready().await.unwrap();
    assert!(child.export_png(2.0).await.unwrap().length() > 0);
    drop(child);
    until_destroyed(1).await;

    // The child first.
    let mut parent = line_chart().await;
    let child = radial_on(&parent).await;
    drop(child);
    settle().await;
    assert_eq!(destroyed(), 1, "the parent still draws on the device");
    parent.frame().unwrap();
    parent.first_frame_ready().await.unwrap();
    assert!(parent.export_png(1.0).await.unwrap().length() > 0);
    drop(parent);
    until_destroyed(2).await;
}

/// A radial chart with its own device, freed with an export in flight:
/// the page answers and the device is destroyed.
#[wasm_bindgen_test(async)]
async fn a_radial_chart_freed_with_work_in_flight_destroys_its_device() {
    count_destroys();
    for round in 0..4 {
        let mut chart =
            FiggyRadialChart::create(small_canvas(), "radial", &radial_document(), 480, 360)
                .await
                .unwrap();
        chart.first_frame_ready().await.unwrap();
        for _ in 0..4 {
            chart.frame().unwrap();
        }
        {
            let mut export = std::pin::pin!(chart.export_png(2.0));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(export.as_mut().poll(&mut context).is_pending());
        }
        drop(chart);
        collect().await;
        until_destroyed(round + 1).await;
    }
    assert_eq!(js_sys::eval("1 + 1").unwrap(), JsValue::from(2));
}

/// A device the host requested and bundled with `RendererDevice::new` is
/// never destroyed by figgy: after the renderer built on it is dropped, the
/// host still maps a buffer on it.
#[wasm_bindgen_test(async)]
async fn a_host_supplied_device_is_never_destroyed() {
    count_destroys();
    let instance = renderer::gpu::create_instance();
    let adapter = renderer::gpu::request_adapter_async(&instance)
        .await
        .unwrap();
    let (device, queue) = renderer::gpu::request_device_async(&adapter).await.unwrap();
    let gpu = RendererDevice::new(device.clone(), queue.clone()).unwrap();
    let renderer = RadialRenderer::try_new_async(gpu, wgpu::TextureFormat::Rgba8Unorm)
        .await
        .unwrap();
    drop(renderer);
    settle().await;
    assert_eq!(destroyed(), 0);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("host buffer"),
        size: 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, &[7; 16]);
    queue.submit([]);
    let (sender, receiver) = futures_channel::oneshot::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            if sender.send(result).is_err() {
                // The test stopped waiting.
            }
        });
    receiver
        .await
        .unwrap()
        .expect("the host's device still maps");
    assert_eq!(&buffer.slice(..).get_mapped_range().unwrap()[..], &[7; 16]);
}
