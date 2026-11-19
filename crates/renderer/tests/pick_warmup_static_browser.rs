//! #363: a browser renderer compiles a chart's pick pipelines (exact
//! keys, the picker's gate and reduce, and the typed data pick kernels its
//! series can dispatch) on its compile queue after the first frame, and
//! keeps each resolved compile until it creates its own pipeline from it.
//! On a static screen (no frame after the first) the first pick then
//! compiles nothing of its own: it waits for a compile that has not
//! resolved yet, and creates the others from the compiles the queue holds.
//! Reports the first pick's latency (`PICK_STATIC` lines) for the scenes of
//! the playground catalogue.
#![cfg(target_arch = "wasm32")]

#[path = "support/playground_scenes.rs"]
mod scenes;

use renderer::{Color, RegisteredChartDrawItem, Renderer};
use scenes::SceneKind;
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
let restore = [];
let calls = {};
let marked = {};
// The modules the compile queue's warm-up compiles from: a call on one of
// them is the warm-up's, even when a waiting pick issues it first.
const WARM_MODULES = ['figgy GPU pick prewarm', 'figgy GPU pick exact prewarm'];
function pick_label(label) {
  return typeof label === 'string'
    && (label.startsWith('figgy GPU pick ') || label.endsWith(' data pick pipeline'));
}
let origin = 0;
let timeline = [];
// The pick compiles' issue and resolve times, in ms after `start_timeline`.
export function start_timeline() { origin = performance.now(); timeline = []; }
export function timeline_text() { return timeline.join('; '); }
export function begin_count() {
  end_count();
  calls = {};
  marked = {};
  const previous = GPUDevice.prototype.createComputePipelineAsync;
  GPUDevice.prototype.createComputePipelineAsync = function(desc) {
    const label = desc && desc.label;
    if (pick_label(label)) {
      const module = desc.compute && desc.compute.module && desc.compute.module.label;
      const key = (WARM_MODULES.includes(module) ? 'warm: ' : 'own: ') + label;
      calls[key] = (calls[key] || 0) + 1;
      const issued = performance.now() - origin;
      const promise = previous.call(this, desc);
      promise.then(() => timeline.push(`${label.replace(/^figgy (GPU pick )?/, '')} ${issued.toFixed(0)}-${(performance.now() - origin).toFixed(0)}`), () => {});
      return promise;
    }
    return previous.call(this, desc);
  };
  restore.push(() => GPUDevice.prototype.createComputePipelineAsync = previous);
}
export function end_count() { for (const f of restore.reverse()) f(); restore = []; }
export function mark() { marked = Object.assign({}, calls); }
// The pick pipelines the pick compiled itself since `mark` (not the
// warm-up's), as `label xN` entries.
export function since_mark() {
  return Object.keys(calls).filter(l => l.startsWith('own: ') && calls[l] !== (marked[l] || 0))
    .map(l => `${l.slice(5)} x${calls[l] - (marked[l] || 0)}`).join('; ');
}
// The data pick kernels the warm-up compiled, as their labels.
export function data_labels() {
  return Object.keys(calls).filter(l => l.startsWith('warm: ') && l.endsWith(' data pick pipeline'))
    .map(l => l.slice(6)).sort().join('; ');
}
// Every pick pipeline compiled more than once in all.
export function repeated() {
  const total = {};
  for (const l of Object.keys(calls)) { const k = l.replace(/^(own|warm): /, ''); total[k] = (total[k] || 0) + calls[l]; }
  return Object.keys(total).filter(k => total[k] > 1).map(k => `${k} x${total[k]}`).join('; ');
}
export function chart_canvas(width, height) {
  const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
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
    fn begin_count();
    fn end_count();
    fn mark();
    fn since_mark() -> String;
    fn data_labels() -> String;
    fn repeated() -> String;
    fn start_timeline();
    fn timeline_text() -> String;
    fn chart_canvas(width: u32, height: u32) -> JsValue;
    fn now_ms() -> f64;
    fn sleep(ms: f64) -> js_sys::Promise;
    fn report(line: &str);
}

struct Counting;
impl Drop for Counting {
    fn drop(&mut self) {
        end_count();
    }
}

async fn pause(ms: f64) {
    if wasm_bindgen_futures::JsFuture::from(sleep(ms))
        .await
        .is_err()
    {
        panic!("sleep rejected");
    }
}

/// What one first pick saw.
struct FirstPick {
    /// The pick labels the pick compiled itself (empty: none).
    compiled_by_pick: String,
    /// The data pick kernels the warm-up compiled.
    data_kernels: String,
    /// The pick pipelines compiled more than once in all.
    repeated: String,
    first_pick_ms: f64,
}

/// One cold first pick of scene `kind` on a static screen: a new renderer
/// (its background render compiles start, as a `FiggyChart`'s do), the
/// scene registered and drawn once, `idle_ms` without a frame, then the
/// first pick at the plot's centre, timed: enabling the picker, the
/// chart's preparation, the exact and the typed data pick compiles, and
/// both picks.
async fn first_pick(kind: SceneKind, idle_ms: f64) -> FirstPick {
    begin_count();
    let _counting = Counting;
    let (width, height) = (scenes::DOC_W, scenes::DOC_H);
    // SwiftShader sometimes reports no adapter right after an earlier
    // renderer of the page was dropped; ask again after a pause.
    let mut attempt = 0;
    let mut renderer = loop {
        match Renderer::for_window_with_shader_set_async_observed(
            wgpu::SurfaceTarget::Canvas(chart_canvas(width, height).unchecked_into()),
            (width, height),
            1024 * 1024,
            renderer::ShaderSet::Basic,
            &mut |_| {},
        )
        .await
        {
            Ok(renderer) => break renderer,
            Err(error) if attempt < 3 => {
                report(&format!("PICK_STATIC renderer retry after {error:?}"));
                attempt += 1;
                pause(1_000.0).await;
            }
            Err(error) => panic!("browser renderer: {error:?}"),
        }
    };
    let scene = scenes::build(&mut renderer, kind, 0).expect("scene");
    let chart = renderer::Chart::new(scene.config.clone());
    start_timeline();
    let id = renderer
        .register_chart(scene.config.clone(), scene.series.clone())
        .unwrap();
    let mut view = renderer
        .create_chart_view(&chart, scene.config.chart_area.0)
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
    // A static screen: no frame until the pick.
    pause(idle_ms).await;
    let area = scene.config.data_area().expect("a plot area").0;
    let at = [
        area.x as f32 + area.width as f32 * 0.5,
        area.y as f32 + area.height as f32 * 0.5,
    ];
    mark();
    let start = now_ms();
    renderer.enable_gpu_picking_async().await.unwrap();
    let enabled = now_ms();
    renderer.prepare_gpu_picking_for_chart(id).unwrap();
    let prepared = now_ms();
    renderer
        .compile_pick_pipelines_at(id, at, 4.0)
        .await
        .unwrap();
    let exact = now_ms();
    renderer
        .compile_data_pick_pipelines_at(id, at, 4.0)
        .await
        .unwrap();
    let data = now_ms();
    let _ = renderer
        .pick_chart_at(id, at, 4.0)
        .unwrap()
        .resolve()
        .await
        .unwrap();
    let _ = renderer
        .pick_chart_data_at(id, at, 4.0)
        .unwrap()
        .resolve()
        .await
        .unwrap();
    let first_pick_ms = now_ms() - start;
    let pick = FirstPick {
        compiled_by_pick: since_mark(),
        data_kernels: data_labels(),
        repeated: repeated(),
        first_pick_ms,
    };
    report(&format!(
        "PICK_STATIC scene={} idle_ms={idle_ms} first_pick_ms={first_pick_ms:.0} \
         enable_ms={:.0} prepare_ms={:.0} exact_ms={:.0} data_ms={:.0} picks_ms={:.0} \
         compiled_by_pick=[{}] repeated=[{}] warmed_data_kernels=[{}] timeline=[{}]",
        kind.label(),
        enabled - start,
        prepared - enabled,
        exact - prepared,
        data - exact,
        now_ms() - data,
        pick.compiled_by_pick,
        pick.repeated,
        pick.data_kernels,
        timeline_text()
    ));
    pick
}

/// The acceptance charts of #363: after 8 s and after 20 s on a static
/// screen the first pick compiles nothing of its own and takes under a
/// second on SwiftShader.
#[wasm_bindgen_test(async)]
async fn a_static_screen_picks_without_compiling() {
    for kind in [
        SceneKind::Radar,
        SceneKind::Violins,
        SceneKind::BoxPlot,
        SceneKind::Parallel,
        SceneKind::TriMesh,
    ] {
        for idle_ms in [8_000.0, 20_000.0] {
            let pick = first_pick(kind, idle_ms).await;
            assert_eq!(
                pick.compiled_by_pick,
                "",
                "{} after {idle_ms} ms",
                kind.label()
            );
            assert_eq!(pick.repeated, "", "{} after {idle_ms} ms", kind.label());
            assert!(
                pick.first_pick_ms < 1_000.0,
                "{} after {idle_ms} ms took {} ms",
                kind.label(),
                pick.first_pick_ms
            );
        }
    }
}

/// Every typed data pick family is warmed: picked right after its first
/// frame, a chart of each family compiles no pick pipeline of its own and
/// none twice (it waits for the warm-up's, issuing them first), and the
/// kernels the warm-up compiled cover every family's.
#[wasm_bindgen_test(async)]
async fn every_data_pick_family_is_warmed_before_its_first_pick() {
    let mut kernels = String::new();
    for kind in [
        SceneKind::Radar,
        SceneKind::Histogram,
        SceneKind::Field,
        SceneKind::VectorField,
        SceneKind::Categorical,
        SceneKind::BoxPlot,
        SceneKind::Gantt,
        SceneKind::Violins,
        SceneKind::Parallel,
        SceneKind::TriMesh,
    ] {
        let pick = first_pick(kind, 0.0).await;
        assert_eq!(pick.compiled_by_pick, "", "{}", kind.label());
        assert_eq!(pick.repeated, "", "{}", kind.label());
        kernels.push_str(&pick.data_kernels);
        kernels.push_str("; ");
    }
    for label in [
        "figgy histogram data pick pipeline",
        "figgy field contour data pick pipeline",
        "figgy field cell data pick pipeline",
        "figgy vector grid data pick pipeline",
        "figgy category bar data pick pipeline",
        "figgy box plot data pick pipeline",
        "figgy range bar data pick pipeline",
        "figgy violin data pick pipeline",
        "figgy parallel data pick pipeline",
        "figgy triangle mesh data pick pipeline",
        "figgy value label data pick pipeline",
    ] {
        assert!(
            kernels.contains(label),
            "{label} was never warmed: {kernels}"
        );
    }
}
