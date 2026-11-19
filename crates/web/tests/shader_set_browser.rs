//! The shader set a chart is created on and upgraded to, in a real browser
//! (#299): which modules `create` hands the browser, what an upgrade issues,
//! and what the options refuse.
#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

mod support;
use support::canvas_320x240 as canvas;

wasm_bindgen_test_configure!(run_in_browser);

/// Record, for every shader module created from now on, whether it is a
/// data module of the basic set or of the full one, and the module of every
/// `createRenderPipelineAsync` call. A data module splices one of the two
/// axis-cut fragments, each of which opens with a comment only it has.
const HOOK: &str = r#"
(() => {
  const S = globalThis.__sets = { basic: 0, full: 0, asyncBasic: 0, asyncFull: 0, open: 0, kind: new WeakMap() };
  const P = GPUDevice.prototype;
  const create = P.createShaderModule;
  P.createShaderModule = function (d) {
    const module = create.call(this, d);
    const kind = d.code.includes('twin of `axis_cuts.wgsl`') ? 'basic'
      : d.code.includes('fn cut_piece_slope') ? 'full' : null;
    if (kind) { S[kind] += 1; S.kind.set(module, kind); }
    return module;
  };
  const pipeline = P.createRenderPipelineAsync;
  P.createRenderPipelineAsync = function (d) {
    const kind = S.kind.get(d.vertex.module);
    if (kind === 'basic') S.asyncBasic += 1;
    if (kind === 'full') S.asyncFull += 1;
    S.open += 1;
    const entry = { label: (kind || '?') + ' ' + (d.label || ''), at: performance.now() };
    (S.flying = S.flying || new Set()).add(entry);
    const p = pipeline.call(this, d);
    const done = () => { S.open -= 1; S.flying.delete(entry); (S.took = S.took || []).push([entry.label, Math.round(performance.now() - entry.at)]); };
    p.then(done, done);
    return p;
  };
})()
"#;

fn count(name: &str) -> u32 {
    js_sys::eval(&format!("globalThis.__sets.{name}"))
        .unwrap()
        .as_f64()
        .unwrap() as u32
}

fn options(pairs: &[(&str, JsValue)]) -> JsValue {
    let object = js_sys::Object::new();
    for (key, value) in pairs {
        js_sys::Reflect::set(&object, &JsValue::from_str(key), value).unwrap();
    }
    object.into()
}

async fn turn() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 50)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Wait until no compile is in flight: the chart's background queue has
/// issued everything it holds and all of it has settled (#299). Fails after
/// `seconds`, naming the compiles still in flight and the slowest settled.
async fn drain(chart: &FiggyChart, seconds: f64) {
    let start = js_sys::Date::now();
    loop {
        turn().await;
        if count("open") == 0 {
            return;
        }
        if js_sys::Date::now() - start > seconds * 1000.0 {
            let report = js_sys::eval(
                "JSON.stringify({ flying: [...globalThis.__sets.flying].map(e => [e.label, Math.round(performance.now() - e.at)]), slowest: (globalThis.__sets.took || []).sort((a, b) => b[1] - a[1]).slice(0, 8) })",
            )
            .unwrap()
            .as_string()
            .unwrap();
            panic!(
                "the queue did not drain in {seconds} s: status {} {report}",
                js_sys::JSON::stringify(&chart.pipeline_status().unwrap()).unwrap()
            );
        }
    }
}

fn message(error: JsValue) -> String {
    error
        .dyn_into::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .unwrap_or_else(|other| format!("{other:?}"))
}

/// A chart created without options is on the basic set: every data module
/// it creates and every compile it issues is a basic one, and none is of
/// the full set. Asking for the full set then issues the full set's render
/// pipelines through the asynchronous compiler, as `create` issued the
/// basic ones, and a second request issues nothing. The compiles go through
/// the chart's throttled queue, so the counts are read once it has drained.
#[wasm_bindgen_test(async)]
async fn the_default_is_the_basic_set_and_an_upgrade_issues_the_full_one() {
    js_sys::eval(HOOK).unwrap();
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    drain(&chart, 120.0).await;
    assert_eq!(chart.shader_set(), "basic");
    assert!(count("basic") > 0);
    assert_eq!(count("full"), 0, "a new chart creates no full module");
    let issued = count("asyncBasic");
    assert!(issued > 20, "create issues the basic set: {issued}");
    assert_eq!(count("asyncFull"), 0);

    // "basic" is the set it is on: nothing happens.
    chart.require_shader_set("basic".into()).unwrap();
    turn().await;
    assert_eq!((count("full"), count("asyncFull")), (0, 0));

    chart.require_shader_set("full".into()).unwrap();
    assert_eq!(chart.shader_set(), "full");
    drain(&chart, 480.0).await;
    assert!(count("full") > 0);
    assert_eq!(
        count("asyncFull"),
        issued,
        "the upgrade issues the full twin of every pipeline create issued"
    );
    assert_eq!(count("asyncBasic"), issued);

    // Idempotent, and never back to basic.
    let (modules, compiles) = (count("full"), count("asyncFull"));
    chart.require_shader_set("full".into()).unwrap();
    chart.require_shader_set("basic".into()).unwrap();
    turn().await;
    assert_eq!(chart.shader_set(), "full");
    assert_eq!((count("full"), count("asyncFull")), (modules, compiles));

    let refused = chart.require_shader_set("turbo".into()).unwrap_err();
    assert_eq!(
        message(refused),
        "shaders must be \"basic\" or \"full\", got \"turbo\""
    );
}

/// `shaders: "full"` creates the chart on the full set with no basic
/// module created and no basic compile issued; `shaders: "basic"` is the
/// default.
#[wasm_bindgen_test(async)]
async fn the_full_option_creates_no_basic_module() {
    js_sys::eval(HOOK).unwrap();
    let chart = FiggyChart::create_with_options(canvas(), options(&[("shaders", "full".into())]))
        .await
        .unwrap();
    drain(&chart, 480.0).await;
    assert_eq!(chart.shader_set(), "full");
    assert_eq!(count("basic"), 0, "no basic module is created");
    assert_eq!(count("asyncBasic"), 0, "no basic compile is issued");
    assert!(count("full") > 0);
    assert!(count("asyncFull") > 20);

    js_sys::eval(HOOK).unwrap();
    let basic = FiggyChart::create_with_options(canvas(), options(&[("shaders", "basic".into())]))
        .await
        .unwrap();
    drain(&basic, 120.0).await;
    assert_eq!(basic.shader_set(), "basic");
    assert_eq!((count("full"), count("asyncFull")), (0, 0));
    assert!(count("asyncBasic") > 20);

    // No options at all, and an empty object: the default.
    for none in [JsValue::UNDEFINED, JsValue::NULL, options(&[])] {
        let chart = FiggyChart::create_with_options(canvas(), none)
            .await
            .unwrap();
        assert_eq!(chart.shader_set(), "basic");
    }
}

/// A `shaders` value other than the two, an option of the wrong kind and an
/// unknown option reject with an error that names it, before anything is
/// created.
#[wasm_bindgen_test(async)]
async fn invalid_create_options_are_refused() {
    js_sys::eval(HOOK).unwrap();
    let cases: [(JsValue, &str); 6] = [
        (
            options(&[("shaders", "turbo".into())]),
            "shaders must be \"basic\" or \"full\", got \"turbo\"",
        ),
        (
            options(&[("shaders", "Full".into())]),
            "shaders must be \"basic\" or \"full\", got \"Full\"",
        ),
        (
            options(&[("shaders", JsValue::from_f64(1.0))]),
            "shaders must be the string \"basic\" or \"full\"",
        ),
        (
            options(&[("shader", "full".into())]),
            "unknown create option \"shader\": the options are shaders and on_progress",
        ),
        (
            options(&[("on_progress", "later".into())]),
            "on_progress must be a function",
        ),
        (
            JsValue::from_str("full"),
            "create_with_options takes an options object",
        ),
    ];
    for (given, expected) in cases {
        let refused = FiggyChart::create_with_options(canvas(), given)
            .await
            .err()
            .expect("the options are refused");
        assert_eq!(message(refused), expected);
    }
    assert_eq!(
        (count("basic"), count("full")),
        (0, 0),
        "nothing was created"
    );
}

/// `on_progress` reports the startup stages as `create_with_progress` does.
#[wasm_bindgen_test(async)]
async fn create_with_options_reports_progress() {
    let seen = js_sys::Array::new();
    let record = seen.clone();
    let callback = wasm_bindgen::closure::Closure::wrap(Box::new(move |event: JsValue| {
        record.push(&event);
    }) as Box<dyn FnMut(JsValue)>);
    let function: js_sys::Function = callback
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    let chart = FiggyChart::create_with_options(
        canvas(),
        options(&[("shaders", "full".into()), ("on_progress", function.into())]),
    )
    .await
    .unwrap();
    assert_eq!(chart.shader_set(), "full");
    assert!(seen.length() >= 20, "{}", seen.length());
}
