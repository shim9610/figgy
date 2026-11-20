//! A renderer handed a chart that needs the full shader set prewarms the
//! full modules in a browser (#299). Kept apart from `shader_prewarm_browser`
//! so each prewarm runs on a page of its own: on a software adapter the full
//! modules' prewarm alone takes minutes.
#![cfg(target_arch = "wasm32")]

use renderer::{Chart, Renderer};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
let restore = [];
let calls = 0;
export function begin_compile_count() {
  calls = 0;
  for (const name of ['createShaderModule','createComputePipeline','createRenderPipeline','createComputePipelineAsync','createRenderPipelineAsync']) {
    const previous = GPUDevice.prototype[name];
    GPUDevice.prototype[name] = function(...args) { calls++; return previous.apply(this,args); };
    restore.push(() => GPUDevice.prototype[name] = previous);
  }
}
export function compile_count() { return calls; }
export function end_compile_count() { for(const f of restore.reverse()) f(); restore=[]; }
export function chart_canvas() {
  const canvas=document.createElement('canvas'); canvas.width=384; canvas.height=256;
  document.body.append(canvas); return canvas;
}
export function now_ms() { return performance.now(); }
"#)]
extern "C" {
    fn begin_compile_count();
    fn compile_count() -> u32;
    fn end_compile_count();
    fn chart_canvas() -> JsValue;
    fn now_ms() -> f64;
}

struct Counting;
impl Drop for Counting {
    fn drop(&mut self) {
        end_compile_count();
    }
}

/// #299: a renderer that has been handed a chart with an axis break
/// prewarms the full modules in a browser. The prewarm contract stores
/// each pipeline once, under its module's basic twin, so every lookup
/// here goes through the twin; a module or a descriptor the twin lacks would
/// fail the prewarm. A second prewarm creates nothing.
#[wasm_bindgen_test(async)]
async fn a_renderer_with_a_broken_chart_prewarms_the_full_modules() {
    use renderer::shader_assembly::ShaderSet;
    let mut renderer = Renderer::for_window_async(
        wgpu::SurfaceTarget::Canvas(chart_canvas().unchecked_into()),
        (384, 256),
        64 * 1024,
    )
    .await
    .expect("actual browser renderer initialization");
    assert_eq!(renderer.shader_set(), ShaderSet::Basic);
    let mut chart = Chart::new(renderer::default::default_config());
    chart.set_y_range(0., 1.);
    chart.config_mut().left_y.breaks = Some(renderer::config::AxisBreaks {
        ranges: vec![renderer::config::AxisBreak::new(0.4, 0.6)],
        ..Default::default()
    });
    renderer
        .register_chart(chart.config().clone(), Vec::new())
        .expect("a chart with a broken axis");
    assert_eq!(renderer.shader_set(), ShaderSet::Full);

    begin_compile_count();
    let _counting = Counting;
    let start = now_ms();
    renderer
        .prewarm_all()
        .await
        .expect("the full modules prewarm from their twins' descriptors");
    console_log!("SHADER_PREWARM_FULL total_ms={}", now_ms() - start);
    let calls = compile_count();
    assert!(calls > 50, "must exercise the real pipeline constructors");
    renderer.prewarm_all().await.unwrap();
    assert_eq!(
        compile_count(),
        calls,
        "cached prewarm must not re-create any shader or pipeline"
    );
    assert_eq!(renderer.shader_set(), ShaderSet::Full);
}
