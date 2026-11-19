//! #363: a browser renderer compiles a registered chart's exact pick
//! pipelines, and the picker's gate and reduce pipelines, ahead of its
//! first pick, on its compile queue; a frame creates each one once it has
//! compiled, and the first pick finds them created (or waits for the
//! compile) instead of starting its own. Also reports the first pick's latency (`PICK_WARMUP` lines), right
//! after the first frame and after idle waits, while the window's render
//! compiles still run in the background.
#![cfg(target_arch = "wasm32")]

use renderer::data::Column;
use renderer::{
    Color, DataRenderType, RadarPolygonSeries, RadarSeriesStyle, RegisteredChartDrawItem, Renderer,
    SeriesConfig,
};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
let restore = [];
let exact = 0;
let settled = 0;
export function begin_exact_count() {
  // A test that panicked left its wrapper in place: remove it first.
  end_exact_count();
  exact = 0;
  settled = 0;
  const previous = GPUDevice.prototype.createComputePipelineAsync;
  GPUDevice.prototype.createComputePipelineAsync = function(desc) {
    const promise = previous.call(this, desc);
    if (desc && desc.label === 'figgy GPU pick exact candidate pipeline') {
      exact++;
      promise.then(() => { settled++; }, () => { settled++; });
    }
    return promise;
  };
  restore.push(() => GPUDevice.prototype.createComputePipelineAsync = previous);
}
export function exact_issued() { return exact; }
export function exact_settled() { return settled; }
export function end_exact_count() { for (const f of restore.reverse()) f(); restore = []; }
export function chart_canvas() {
  const canvas = document.createElement('canvas'); canvas.width = 480; canvas.height = 360;
  document.body.append(canvas); return canvas;
}
export function now_ms() { return performance.now(); }
export function sleep(ms) { return new Promise(resolve => setTimeout(resolve, ms)); }
export function report(line) {
  // The runner prints the output element; a passing test's log is not.
  console.log(line);
  const output = document.querySelector('#output');
  if (output) output.append(line + '\n');
}
"#)]
extern "C" {
    fn begin_exact_count();
    fn exact_issued() -> u32;
    fn exact_settled() -> u32;
    fn end_exact_count();
    fn chart_canvas() -> JsValue;
    fn now_ms() -> f64;
    fn sleep(ms: f64) -> js_sys::Promise;
    fn report(line: &str);
}

struct Counting;
impl Drop for Counting {
    fn drop(&mut self) {
        end_exact_count();
    }
}

/// A five-category radar chart filling the canvas, with markers and a fill
/// (the cold pick of #363).
fn radar_config() -> renderer::Config {
    let mut config = renderer::default::default_config();
    config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
        x: 0,
        y: 0,
        width: 480,
        height: 360,
    });
    config.legend.visible = false;
    config.bottom_x.categories = Some(
        (0..5)
            .map(|i| renderer::config::Category::new(format!("c{i}"), format!("C{i}")))
            .collect(),
    );
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 4.0;
    config.left_y.min = 0.0;
    config.left_y.max = 10.0;
    config.set_polar(Some(renderer::polar::PolarOptions::default()));
    config
}

/// What one first pick saw.
struct FirstPick {
    /// Exact compiles that had ended when the pick started: the warm-up's.
    warmed: u32,
    /// Exact `createComputePipelineAsync` calls the pick made itself.
    issued_by_pick: u32,
    /// Exact `createComputePipelineAsync` calls in all, the warm-up's
    /// included.
    issued: u32,
}

/// One cold first pick: a new renderer (its background render compiles
/// start), a radar chart registered and drawn once, `idle_ms` of waiting,
/// then the first pick, timed.
async fn first_pick(idle_ms: f64) -> FirstPick {
    begin_exact_count();
    let _counting = Counting;
    // As a `FiggyChart` creates it: the render pipelines compile in the
    // background on the renderer's queue (#299).
    // SwiftShader sometimes reports no adapter right after an earlier
    // renderer of the page was dropped; ask again after a pause.
    let mut attempt = 0;
    let mut renderer = loop {
        match Renderer::for_window_with_shader_set_async_observed(
            wgpu::SurfaceTarget::Canvas(chart_canvas().unchecked_into()),
            (480, 360),
            1024 * 1024,
            renderer::ShaderSet::Basic,
            &mut |_| {},
        )
        .await
        {
            Ok(renderer) => break renderer,
            Err(error) if attempt < 3 => {
                report(&format!("PICK_WARMUP renderer retry after {error:?}"));
                attempt += 1;
                if wasm_bindgen_futures::JsFuture::from(sleep(1_000.0))
                    .await
                    .is_err()
                {
                    panic!("sleep rejected");
                }
            }
            Err(error) => panic!("browser renderer: {error:?}"),
        }
    };
    for (id, data) in [
        ("cat", vec![0.0, 1.0, 2.0, 3.0, 4.0]),
        ("val", vec![5.0, 8.0, 3.0, 9.0, 6.0]),
    ] {
        renderer.add_column(id, &Column { data }).unwrap();
    }
    let config = radar_config();
    let chart = renderer::Chart::new(config.clone());
    let id = renderer
        .register_chart(
            config.clone(),
            vec![SeriesConfig {
                series_id: "radar".into(),
                source_id: None,
                label: None,
                x_column: "cat".into(),
                y_column: "val".into(),
                z_column: None,
                render_type: DataRenderType::Radar(RadarPolygonSeries {
                    color: Color::BLACK,
                    style: RadarSeriesStyle::default(),
                    labels: Default::default(),
                }),
            }],
        )
        .unwrap();
    let mut view = renderer
        .create_chart_view(&chart, config.chart_area.0)
        .unwrap();
    renderer
        .draw_registered(
            Color::WHITE,
            &mut [RegisteredChartDrawItem {
                chart_id: id,
                view: &mut view,
            }],
        )
        .unwrap();
    // The host keeps drawing while it idles, as a `FiggyChart` does for its
    // background compiles: each frame creates what has compiled since.
    let mut waited = 0.0;
    let drawn = now_ms();
    let mut complete_ms = renderer.frame_complete().then_some(0.0);
    // The slowest idle frame: the one that enables the picker and creates
    // the warmed pipelines included.
    let mut slowest_frame_ms: f64 = 0.0;
    while waited < idle_ms {
        let step = (idle_ms - waited).min(500.0);
        if wasm_bindgen_futures::JsFuture::from(sleep(step))
            .await
            .is_err()
        {
            panic!("sleep rejected");
        }
        waited += step;
        let frame_start = now_ms();
        renderer
            .draw_registered(
                Color::WHITE,
                &mut [RegisteredChartDrawItem {
                    chart_id: id,
                    view: &mut view,
                }],
            )
            .unwrap();
        slowest_frame_ms = slowest_frame_ms.max(now_ms() - frame_start);
        if complete_ms.is_none() && renderer.frame_complete() {
            complete_ms = Some(now_ms() - drawn);
        }
    }
    let warmed = exact_settled();
    let start = now_ms();
    renderer.enable_gpu_picking_async().await.unwrap();
    let enabled = now_ms();
    renderer.prepare_gpu_picking_for_chart(id).unwrap();
    let before_compile = exact_issued();
    renderer
        .compile_pick_pipelines_at(id, [240.0, 180.0], 4.0)
        .await
        .unwrap();
    let compiled = now_ms();
    let issued_by_pick = exact_issued() - before_compile;
    let _ = renderer
        .pick_chart_at(id, [240.0, 180.0], 4.0)
        .unwrap()
        .resolve()
        .await
        .unwrap();
    let picked = now_ms();
    report(&format!(
        "PICK_WARMUP idle_ms={idle_ms} exact_compiled_before_pick={warmed} \
         pick_exact_calls={issued_by_pick} frame_complete_ms={complete_ms:?} \
         slowest_idle_frame_ms={slowest_frame_ms:.0} \
         enable_ms={:.0} exact_ms={:.0} readback_ms={:.0} first_pick_ms={:.0}",
        enabled - start,
        compiled - enabled,
        picked - compiled,
        picked - start
    ));
    FirstPick {
        warmed,
        issued_by_pick,
        issued: exact_issued(),
    }
}

/// The first frame schedules the chart's gate, reduce and exact pipelines;
/// after a 20 s idle wait they have compiled and a later frame has created
/// them, so the pick compiles nothing of its own. After 8 s it depends on
/// the machine's load: either the same, or the warm-up compile had not been
/// created yet and the pick waits for it and makes the one cache lookup of
/// its own. Never a second
/// compile of the key.
#[wasm_bindgen_test(async)]
async fn a_radar_chart_compiles_its_exact_pipeline_before_the_first_pick() {
    let pick = first_pick(8_000.0).await;
    assert!(
        matches!(
            (pick.warmed, pick.issued_by_pick, pick.issued),
            (1, 0, 1) | (_, 1, 2)
        ),
        "after 8 s: {:?}",
        (pick.warmed, pick.issued_by_pick, pick.issued)
    );
    let pick = first_pick(20_000.0).await;
    assert_eq!(pick.warmed, 1, "the warm-up compiled during 20 s");
    assert_eq!((pick.issued_by_pick, pick.issued), (0, 1));
}

/// Picked right after the first frame, the pick waits for the compile the
/// frame scheduled: the warm-up's compile and the pick's cache lookup are
/// the only two.
#[wasm_bindgen_test(async)]
async fn a_pick_right_after_the_first_frame_waits_for_the_warm_up() {
    assert_eq!(first_pick(0.0).await.issued, 2);
}
