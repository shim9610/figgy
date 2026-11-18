//! A chart's background pipeline compiles in a real browser (#299): at most
//! as many in flight as the browser reports cores, four when it reports
//! none, and what the screen asks for issued before the rest.
#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use renderer::data_config::{
    FieldFillConfig, FieldSeries, FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
};
use renderer::{DataRenderType, SeriesConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas = document
        .create_element("canvas")
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();
    canvas.set_width(320);
    canvas.set_height(240);
    canvas
}

/// Report `cores` as `navigator.hardwareConcurrency` (`undefined` for
/// `None`), and record every `createRenderPipelineAsync` and
/// `createComputePipelineAsync` from now on: its label in issue order, how
/// many are open and the most ever open. A synchronous creation of either
/// kind before an async compile of it resolved is recorded as cold.
fn hook(cores: Option<u32>) {
    let cores = cores.map_or("undefined".to_owned(), |cores| cores.to_string());
    js_sys::eval(&format!(
        r#"(() => {{
  Object.defineProperty(navigator, 'hardwareConcurrency', {{ configurable: true, get: () => {cores} }});
  const Q = globalThis.__queue = {{ labels: [], open: 0, peak: 0, reject: null, hold: null, device: null, resolved: new Set(), cold: [] }};
  const P = GPUDevice.prototype;
  if (!P.__figgyOriginalAsync) P.__figgyOriginalAsync = P.createRenderPipelineAsync;
  if (!P.__figgyOriginalSync) P.__figgyOriginalSync = P.createRenderPipeline;
  const original = P.__figgyOriginalAsync;
  // A synchronous creation of a label no async compile has resolved yet is
  // a cold compile on the draw path.
  // A pipeline is told apart by its label and its first target's format.
  const key = (d) => (d.label || '') + '|' + (d.fragment?.targets?.[0]?.format ?? '');
  Q.key = key;
  Q.keys = [];
  P.createRenderPipeline = function (d) {{
    if (!Q.resolved.has(key(d))) Q.cold.push(key(d));
    return P.__figgyOriginalSync.call(this, d);
  }};
  if (!P.__figgyOriginalComputeAsync) P.__figgyOriginalComputeAsync = P.createComputePipelineAsync;
  if (!P.__figgyOriginalCompute) P.__figgyOriginalCompute = P.createComputePipeline;
  P.createComputePipeline = function (d) {{
    if (!Q.resolved.has(key(d))) Q.cold.push(key(d));
    return P.__figgyOriginalCompute.call(this, d);
  }};
  P.createComputePipelineAsync = function (d) {{
    Q.labels.push(d.label || '');
    Q.keys.push(key(d));
    Q.open += 1;
    Q.peak = Math.max(Q.peak, Q.open);
    const p = P.__figgyOriginalComputeAsync.call(this, d);
    const done = () => {{ Q.open -= 1; }};
    p.then(() => {{ Q.resolved.add(key(d)); done(); }}, done);
    return p;
  }};
  P.createRenderPipelineAsync = function (d) {{
    Q.device = this;
    Q.labels.push(d.label || '');
    Q.keys.push(key(d));
    Q.open += 1;
    Q.peak = Math.max(Q.peak, Q.open);
    // A label the test makes the browser reject, or holds until the device
    // is lost (then it resolves, as the specification says).
    const p = d.label === Q.reject ? Promise.reject(new Error('rejected by the test'))
      : d.label === Q.hold ? this.lost.then(() => undefined)
      : original.call(this, d);
    const done = () => {{ Q.open -= 1; }};
    p.then(() => {{ Q.resolved.add(key(d)); done(); }}, done);
    return p;
  }};
}})()"#
    ))
    .unwrap();
}

fn number(expression: &str) -> u32 {
    js_sys::eval(expression).unwrap().as_f64().unwrap() as u32
}

/// One timer turn of 20 ms. A timer, not an animation frame: a page whose
/// frames stop must still reach the time limits below.
async fn turn() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 20)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Wait until `done` holds, at most `seconds`; past that, fail with the
/// chart's compile status, so a regression fails instead of hanging.
async fn within(chart: &FiggyChart, seconds: f64, what: &str, mut done: impl FnMut() -> bool) {
    let start = js_sys::Date::now();
    while !done() {
        assert!(
            js_sys::Date::now() - start < seconds * 1000.0,
            "{what} not within {seconds} s: status {}, {} issued, {} open",
            status_text(chart),
            number("globalThis.__queue.labels.length"),
            number("globalThis.__queue.open"),
        );
        turn().await;
    }
}

/// The chart's compile status as JSON text, for messages.
fn status_text(chart: &FiggyChart) -> String {
    js_sys::JSON::stringify(&chart.pipeline_status().unwrap())
        .unwrap()
        .as_string()
        .unwrap()
}

/// The chart's compile status.
fn status(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&status_text(chart)).unwrap()
}

/// Draw frames until one draws every series, at most `seconds`: no frame
/// waits on a compile for good.
async fn complete_within(chart: &mut FiggyChart, seconds: f64, what: &str) {
    let start = js_sys::Date::now();
    loop {
        chart.frame().unwrap();
        if chart.frame_complete() {
            return;
        }
        assert!(
            js_sys::Date::now() - start < seconds * 1000.0,
            "{what}: no complete frame within {seconds} s: status {}",
            status_text(chart)
        );
        turn().await;
    }
}

/// The in-flight count never exceeds the reported core count, and reaches
/// it; with no core count, or a count of zero, it is four.
#[wasm_bindgen_test(async)]
async fn the_in_flight_compiles_never_exceed_the_core_count() {
    for (cores, limit) in [(Some(2), 2), (Some(3), 3), (None, 4), (Some(0), 4)] {
        hook(cores);
        let chart = FiggyChart::create(canvas()).await.unwrap();
        within(&chart, 120.0, "21 compiles issued", || {
            number("globalThis.__queue.labels.length") > 20
        })
        .await;
        assert_eq!(
            number("globalThis.__queue.peak"),
            limit,
            "{cores:?}: the most compiles in flight at once"
        );
    }
}

fn series(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type,
    }
}

/// Show a heatmap, on columns registered here.
fn show_heatmap(chart: &mut FiggyChart) {
    chart
        .register_column_f32("fx", &[0.0, 1.0, 2.0, 3.0])
        .unwrap();
    chart.register_column_f32("fy", &[0.0, 1.0, 2.0]).unwrap();
    for i in 0..3 {
        chart
            .register_column_f32(&format!("z{i}"), &[i as f32, 1.0 + i as f32])
            .unwrap();
    }
    let mut config: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config["bottom_x"]["min"] = 0.0.into();
    config["bottom_x"]["max"] = 3.0.into();
    config["left_y"]["min"] = 0.0.into();
    config["left_y"]["max"] = 2.0.into();
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.min = 0.0;
    bar.axis.max = 3.0;
    bar.axis.major_spacing = 1.0;
    config["colorbar"] = serde_json::to_value(&bar).unwrap();
    chart.set_config(&config.to_string()).unwrap();
    let field = series(
        "field",
        "fx",
        "fy",
        DataRenderType::Field(FieldSeries {
            matrix: MatrixRef {
                columns: (0..3).map(|i| format!("z{i}")).collect(),
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Edges,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            }),
            contour: None,
            surface: None,
        }),
    );
    chart
        .set_series(&serde_json::to_string(&vec![field]).unwrap())
        .unwrap();
}

/// A heatmap shown while the compiles of a new chart run is issued before
/// every background compile that still waits: its pipeline is issued as
/// soon as a slot frees, not in its place in the set (position 51 of 61),
/// and the chart is complete once it has resolved.
#[wasm_bindgen_test(async)]
async fn a_pipeline_the_screen_asks_for_is_issued_first() {
    hook(Some(1));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    show_heatmap(&mut chart);
    // No await since `create`: the compiles issued so far are the ones the
    // screen could not move ahead of.
    let before = number("globalThis.__queue.labels.length");
    chart.frame().unwrap();
    assert!(
        !chart.frame_complete(),
        "the heatmap waits for its pipeline"
    );
    complete_within(&mut chart, 120.0, "the heatmap").await;
    let labels: Vec<String> =
        js_sys::Array::from(&js_sys::eval("globalThis.__queue.labels").unwrap())
            .iter()
            .map(|label| label.as_string().unwrap())
            .collect();
    let field_at = labels
        .iter()
        .position(|label| label == "figgy field columnar pipeline")
        .expect("the heatmap's pipeline was issued");
    assert_eq!(
        field_at as u32, before,
        "the heatmap's pipeline is the next issued after the screen asked for it: {labels:?}"
    );
    assert_eq!(number("globalThis.__queue.peak"), 1);
}

/// An upgrade to the full set while basic compiles are still in flight:
/// the request returns at once, the full set is scheduled, the basic ones
/// in flight run out, and a heatmap shown then completes. A second request
/// schedules nothing more.
#[wasm_bindgen_test(async)]
async fn an_upgrade_while_basic_compiles_are_in_flight_completes() {
    hook(Some(2));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let basic = status(&chart);
    assert!(basic["all"]["compiling"].as_u64().unwrap() > 0, "{basic}");
    assert_eq!(
        number("globalThis.__queue.open"),
        2,
        "basic compiles in flight"
    );
    chart.require_shader_set("full".into()).unwrap();
    let full = status(&chart);
    assert_eq!(chart.shader_set(), "full");
    assert!(full["all"]["total"].as_u64().unwrap() > 20, "{full}");
    chart.require_shader_set("full".into()).unwrap();
    assert_eq!(
        status(&chart)["all"]["total"],
        full["all"]["total"],
        "a second request schedules nothing"
    );
    show_heatmap(&mut chart);
    complete_within(&mut chart, 180.0, "the heatmap after a mid-compile upgrade").await;
    assert_eq!(status(&chart)["all"]["failed"], 0);
}

/// An upgrade once every basic compile has settled schedules the full set,
/// and a heatmap shown then completes.
#[wasm_bindgen_test(async)]
async fn an_upgrade_after_the_basic_compiles_settled_completes() {
    hook(Some(4));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    within(&chart, 180.0, "the basic set compiled", || {
        status(&chart)["all"]["compiling"] == 0
    })
    .await;
    chart.require_shader_set("full".into()).unwrap();
    let full = status(&chart);
    assert!(full["all"]["total"].as_u64().unwrap() > 20, "{full}");
    assert!(full["all"]["compiling"].as_u64().unwrap() > 0, "{full}");
    show_heatmap(&mut chart);
    complete_within(&mut chart, 180.0, "the heatmap after the upgrade").await;
}

/// A compile the browser rejects is counted as failed, blocks no other
/// compile, and the series that needs it is drawn: the draw creates the
/// pipeline itself.
#[wasm_bindgen_test(async)]
async fn a_rejected_compile_blocks_nothing() {
    hook(Some(2));
    js_sys::eval("globalThis.__queue.reject = 'figgy field columnar pipeline'").unwrap();
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    show_heatmap(&mut chart);
    complete_within(&mut chart, 120.0, "the heatmap with a rejected compile").await;
    within(&chart, 180.0, "the rest of the set compiled", || {
        status(&chart)["all"]["compiling"] == 0
    })
    .await;
    let after = status(&chart);
    assert!(after["all"]["failed"].as_u64().unwrap() >= 1, "{after}");
    assert_eq!(
        after["all"]["ready"].as_u64().unwrap() + after["all"]["failed"].as_u64().unwrap(),
        after["all"]["total"].as_u64().unwrap()
    );
}

/// A chart waiting in `first_frame_ready` for a compile that settles only
/// when the device is lost: losing the device ends the wait, and every
/// open compile ends as device lost.
#[wasm_bindgen_test(async)]
async fn device_loss_ends_a_wait_on_an_open_compile() {
    hook(Some(1));
    js_sys::eval("globalThis.__queue.hold = 'figgy field columnar pipeline'").unwrap();
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    show_heatmap(&mut chart);
    chart.frame().unwrap();
    assert!(
        !chart.frame_complete(),
        "the heatmap waits for its pipeline"
    );
    // Lose the device once the held compile is in flight.
    js_sys::eval(
        "(async () => { const Q = globalThis.__queue; \
          while (!Q.labels.includes(Q.hold)) await new Promise(r => setTimeout(r, 20)); \
          Q.device.destroy(); })()",
    )
    .unwrap();
    // Ends with the device-loss error or with a frame; it must not hang
    // (the runner's time limit fails a hang).
    let _ = chart.first_frame_ready().await;
    let after = status(&chart);
    assert_eq!(after["all"]["compiling"], 0, "{after}");
}

/// A chart dropped while its compiles are in flight issues no further
/// compile: those in flight settle, and the rest are never issued.
#[wasm_bindgen_test(async)]
async fn a_dropped_chart_issues_no_more_compiles() {
    hook(Some(2));
    let chart = FiggyChart::create(canvas()).await.unwrap();
    // No await between this count and the drop.
    let issued = number("globalThis.__queue.labels.length");
    assert!(issued >= 2, "{issued}");
    assert!(
        status(&chart)["all"]["total"].as_u64().unwrap() > issued as u64 + 20,
        "most of the set still waits"
    );
    drop(chart);
    let start = js_sys::Date::now();
    while number("globalThis.__queue.open") > 0 {
        assert!(
            js_sys::Date::now() - start < 60_000.0,
            "the dropped chart's compiles in flight never settled"
        );
        turn().await;
    }
    for _ in 0..30 {
        turn().await;
    }
    assert_eq!(
        number("globalThis.__queue.labels.length"),
        issued,
        "a dropped chart issues nothing more"
    );
}

/// Whether `label` was issued through the async compiler, and whether a
/// draw created it synchronously before its compile resolved.
fn issued_and_cold(label: &str) -> (bool, bool) {
    let issued = js_sys::eval(&format!("globalThis.__queue.labels.includes({label:?})"))
        .unwrap()
        .as_bool()
        .unwrap();
    let cold = js_sys::eval(&format!(
        "globalThis.__queue.cold.some(k => k.startsWith({label:?} + '|'))"
    ))
    .unwrap()
    .as_bool()
    .unwrap();
    (issued, cold)
}

/// A series of a module compiled only on demand: the first frame leaves it
/// out and issues the module's compile, the chart completes once it
/// resolves, and no draw creates `label` cold.
async fn waits_for_its_on_demand_module(chart: &mut FiggyChart, label: &str, what: &str) {
    assert!(
        !issued_and_cold(label).0,
        "{what}: {label} is not compiled before a chart needs it"
    );
    chart.frame().unwrap();
    assert!(!chart.frame_complete(), "{what}: the series waits at first");
    // Scheduled ahead of every waiting compile; issued when a slot frees.
    complete_within(chart, 120.0, what).await;
    chart.first_frame_ready().await.unwrap();
    let (issued, cold) = issued_and_cold(label);
    assert!(
        issued,
        "{what}: {label} issued on demand; status {}, cold {:?}",
        status_text(chart),
        js_sys::eval("globalThis.__queue.cold.join(' | ')")
            .unwrap()
            .as_string()
    );
    assert!(!cold, "{what}: {label} was created cold");
}

/// A 2D vector series waits for the vector module, compiled on demand.
#[wasm_bindgen_test(async)]
async fn a_vector_series_waits_for_its_module() {
    hook(Some(4));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    for (id, values) in [
        ("vx", [0.0, 1.0, 2.0]),
        ("vy", [0.0, 1.0, 2.0]),
        ("vu", [1.0, 0.5, 0.0]),
        ("vv", [0.0, 0.5, 1.0]),
    ] {
        chart.register_column_f32(id, &values).unwrap();
    }
    use renderer::Color;
    use renderer::data_config::{VectorComponents, VectorPivot, VectorScale, VectorStyle};
    let arrows = series(
        "arrows",
        "vx",
        "vy",
        DataRenderType::Vector {
            components: VectorComponents::Points {
                u: "vu".into(),
                v: "vv".into(),
                w: None,
            },
            style: VectorStyle {
                color: Color::BLACK,
                width_px: 2.0,
                scale: VectorScale::Data { factor: 1.0 },
                pivot: VectorPivot::Tail,
                head_length_px: 8.0,
                head_width_px: 8.0,
                min_length_px: 0.0,
                color_map: None,
            },
        },
    );
    chart
        .set_series(&serde_json::to_string(&vec![arrows]).unwrap())
        .unwrap();
    waits_for_its_on_demand_module(&mut chart, "figgy vector points pipeline", "vector").await;
}

/// A heatmap drawn as a 3D surface waits for the surface module, compiled
/// on demand.
#[wasm_bindgen_test(async)]
async fn a_3d_surface_waits_for_its_module() {
    hook(Some(4));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    show_heatmap(&mut chart);
    let mut series: serde_json::Value = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    series[0]["render_type"]["Heatmap"]["surface"] = serde_json::json!({});
    chart.set_series(&series.to_string()).unwrap();
    let mut config: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config.set_view_3d(Some(renderer::view3d::View3d::default()));
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .unwrap();
    waits_for_its_on_demand_module(&mut chart, "figgy surface pipeline", "3D surface").await;
}

/// A ternary region fill waits for the region module, compiled on demand.
#[wasm_bindgen_test(async)]
async fn a_ternary_region_waits_for_its_module() {
    hook(Some(4));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    for (id, values) in [
        ("ta", [60.0, 20.0, 20.0]),
        ("tb", [20.0, 60.0, 20.0]),
        ("tc", [20.0, 20.0, 60.0]),
    ] {
        chart.register_column_f32(id, &values).unwrap();
    }
    let mut config: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config.set_ternary(Some(renderer::TernaryOptions {}));
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .unwrap();
    let region = series(
        "region",
        "ta",
        "tb",
        DataRenderType::Ternary {
            c_column: "tc".into(),
            scatter: None,
            line: None,
            region: Some(renderer::TernaryRegionStyle {
                color: renderer::Color::BLACK,
                opacity: 0.5,
            }),
        },
    );
    chart
        .set_series(&serde_json::to_string(&vec![region]).unwrap())
        .unwrap();
    waits_for_its_on_demand_module(
        &mut chart,
        "figgy ternary region cover pipeline",
        "ternary region",
    )
    .await;
    // The ring-start scan the fill dispatches first was compiled in the
    // background too, never on the page's thread.
    assert_eq!(
        issued_and_cold("figgy ternary ring scan pipeline"),
        (true, false),
        "the ring-start scan compiled in the background"
    );
}

/// The status object splits what the screen needs from everything;
/// `series_ready` resolves when the chart is drawn whole and rejects with
/// the failed compile's label and kind; `pipeline_failures` lists it.
#[wasm_bindgen_test(async)]
async fn the_status_api_reports_the_screen_and_failures() {
    hook(Some(2));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    show_heatmap(&mut chart);
    chart.frame().unwrap();
    let before = status(&chart);
    assert_eq!(before["screen"]["total"], 1, "{before}");
    assert!(before["all"]["total"].as_u64().unwrap() > 20, "{before}");
    assert_eq!(before["frame_complete"], false);
    chart.series_ready().await.unwrap();
    let after = status(&chart);
    assert_eq!(after["frame_complete"], true, "{after}");
    assert_eq!(after["screen"]["ready"], 1, "{after}");
    assert_eq!(after["screen"]["compiling"], 0, "{after}");
    assert_eq!(chart.pipeline_failures().unwrap().length(), 0);

    // A second chart whose heatmap compile the browser rejects.
    hook(Some(2));
    js_sys::eval("globalThis.__queue.reject = 'figgy field columnar pipeline'").unwrap();
    let mut rejected = FiggyChart::create(canvas()).await.unwrap();
    show_heatmap(&mut rejected);
    let error = rejected.series_ready().await.unwrap_err();
    let get = |key: &str| {
        js_sys::Reflect::get(&error, &key.into())
            .unwrap()
            .as_string()
            .unwrap()
    };
    assert_eq!(get("label"), "figgy field columnar pipeline");
    assert_eq!(get("kind"), "rejected");
    // The series is drawn all the same: the draw creates the pipeline.
    assert!(rejected.frame_complete());
    let failures = rejected.pipeline_failures().unwrap();
    assert_eq!(failures.length(), 1);
    let failure = failures.get(0);
    assert_eq!(
        js_sys::Reflect::get(&failure, &"kind".into()).unwrap(),
        "rejected"
    );
    assert!(rejected.pipelines_ready().await.is_err());
}

/// An export while the chart's pipelines compile compiles its own (the
/// export target's format) in the background and waits for them, so none
/// is created cold on the main thread, and the picture is the chart.
#[wasm_bindgen_test(async)]
async fn an_export_compiles_its_pipelines_in_the_background() {
    hook(Some(4));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    show_heatmap(&mut chart);
    chart.frame().unwrap();
    let png = chart.export_png(1.0).await.unwrap();
    assert!(png.length() > 0);
    let export_key = "figgy field columnar pipeline|rgba16float";
    let has = |list: &str| {
        js_sys::eval(&format!(
            "globalThis.__queue.{list}.includes({export_key:?})"
        ))
        .unwrap()
        .as_bool()
        .unwrap()
    };
    assert!(
        has("keys"),
        "the export's heatmap pipeline compiled in the background"
    );
    assert!(
        !has("cold"),
        "the export's heatmap pipeline was not created cold"
    );
    // The export drew the heatmap: it differs from an export without it.
    chart.set_series("[]").unwrap();
    let empty = chart.export_png(1.0).await.unwrap();
    assert_ne!(png.to_vec(), empty.to_vec());
}

/// Record every exact pick pipeline (`figgy GPU pick exact candidate
/// pipeline`), told apart by its override constants: those compiled through
/// `createComputePipelineAsync`, and those created synchronously before an
/// async compile of the same constants resolved (created cold).
fn hook_exact_picks() {
    js_sys::eval(
        r#"(() => {
  const E = globalThis.__exact = { compiled: [], resolved: new Set(), created: [], cold: [] };
  const P = GPUDevice.prototype;
  if (!P.__figgyOriginalComputeAsync) P.__figgyOriginalComputeAsync = P.createComputePipelineAsync;
  if (!P.__figgyOriginalCompute) P.__figgyOriginalCompute = P.createComputePipeline;
  const label = 'figgy GPU pick exact candidate pipeline';
  const key = (d) => JSON.stringify(Object.entries(d.compute?.constants ?? {}).sort());
  P.createComputePipelineAsync = function (d) {
    const p = P.__figgyOriginalComputeAsync.call(this, d);
    if (d.label === label) {
      E.compiled.push(key(d));
      p.then(() => E.resolved.add(key(d)), () => {});
    }
    return p;
  };
  P.createComputePipeline = function (d) {
    if (d.label === label) {
      E.created.push(key(d));
      if (!E.resolved.has(key(d))) E.cold.push(key(d));
    }
    return P.__figgyOriginalCompute.call(this, d);
  };
})()"#,
    )
    .unwrap();
}

/// The first pick compiles the exact pick pipelines it needs through the
/// browser's async API before it submits, so none is created cold on the
/// page's thread; the pick still answers.
#[wasm_bindgen_test(async)]
async fn a_first_pick_compiles_its_exact_pipelines_in_the_background() {
    hook(Some(4));
    hook_exact_picks();
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.register_column_f32("px", &[0.0, 1.0, 2.0]).unwrap();
    chart.register_column_f32("py", &[0.0, 1.0, 2.0]).unwrap();
    chart
        .add_line_series("line", "px", "py", 2.0, "line")
        .unwrap();
    let mut config: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    for axis in ["bottom_x", "left_y"] {
        config[axis]["min"] = 0.0.into();
        config[axis]["max"] = 2.0.into();
    }
    chart.set_config(&config.to_string()).unwrap();
    complete_within(&mut chart, 120.0, "the line chart").await;
    let picked = chart.pick_point(160.0, 120.0, 400.0).await.unwrap();
    let count = |list: &str| number(&format!("globalThis.__exact.{list}.length"));
    assert!(
        count("compiled") > 0,
        "the pick compiled its exact pipelines in the background"
    );
    assert!(count("created") > 0, "the pick created its exact pipelines");
    assert_eq!(
        count("cold"),
        0,
        "no exact pick pipeline was created cold: {:?}",
        js_sys::JSON::stringify(&js_sys::eval("globalThis.__exact").unwrap()).unwrap()
    );
    let picked: serde_json::Value = serde_json::from_str(&picked.as_string().unwrap()).unwrap();
    assert_eq!(picked["series_id"], "line");
}

/// A dashed line waits for the arc prefix scan, a compute module compiled
/// on demand.
#[wasm_bindgen_test(async)]
async fn a_dashed_line_waits_for_its_arc_scan() {
    hook(Some(4));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart.register_column_f32("lx", &[0.0, 1.0, 2.0]).unwrap();
    chart.register_column_f32("ly", &[0.0, 2.0, 1.0]).unwrap();
    chart
        .add_line_series("dashed", "lx", "ly", 2.0, "dashed")
        .unwrap();
    let mut series: serde_json::Value = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    series[0]["render_type"]["Line"]["line"]["line_style"] = "Dash".into();
    chart.set_series(&series.to_string()).unwrap();
    waits_for_its_on_demand_module(&mut chart, "figgy arc scan pipeline", "dashed line").await;
}

/// A histogram waits for its envelope's compute pass, compiled on demand.
#[wasm_bindgen_test(async)]
async fn a_histogram_waits_for_its_envelope_scan() {
    use renderer::data_config::{BarOrientation, DataBarStyleConfig};
    hook(Some(4));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart
        .register_column_f32("edges", &[0.0, 1.0, 2.0, 3.0, 4.0])
        .unwrap();
    chart
        .register_column_f32("heights", &[1.0, 3.0, 2.0, 4.0])
        .unwrap();
    let bars = series(
        "bars",
        "edges",
        "heights",
        DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: renderer::Color::BLACK,
                border_color: renderer::Color::BLACK,
                border_width: 0.0,
                baseline: 0.0,
                gap_px: 0.0,
                width_ratio: 1.0,
                orientation: BarOrientation::Vertical,
                bar_style_overrides: None,
            },
        },
    );
    chart
        .set_series(&serde_json::to_string(&vec![bars]).unwrap())
        .unwrap();
    chart.fit_ranges().unwrap();
    waits_for_its_on_demand_module(&mut chart, "histogram pixel maxima", "histogram").await;
}

/// Contour labels wait for the label anchor compute module, compiled on
/// demand.
#[wasm_bindgen_test(async)]
async fn contour_labels_wait_for_their_anchor_scan() {
    hook(Some(4));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    for (id, values) in [
        ("gx", [0.0, 1.0, 2.0]),
        ("gy", [0.0, 1.0, 2.0]),
        ("p0", [0.0, 1.0, 2.0]),
        ("p1", [1.0, 2.0, 3.0]),
        ("p2", [2.0, 3.0, 4.0]),
    ] {
        chart.register_column_f32(id, &values).unwrap();
    }
    let mut config: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    for axis in ["bottom_x", "left_y"] {
        config[axis]["min"] = 0.0.into();
        config[axis]["max"] = 2.0.into();
    }
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.min = 0.0;
    bar.axis.max = 4.0;
    bar.axis.major_spacing = 1.0;
    config["colorbar"] = serde_json::to_value(&bar).unwrap();
    chart.set_config(&config.to_string()).unwrap();
    let contour = serde_json::json!([{
        "series_id": "contour",
        "x_column": "gx",
        "y_column": "gy",
        "render_type": {
            "Contour": {
                "matrix": {
                    "columns": ["p0", "p1", "p2"],
                    "orientation": "ColumnsAreX",
                    "grid_layout": "Centers"
                },
                "contour": {
                    "levels": [2.0],
                    "line": {
                        "line_style": "Solid",
                        "line_color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
                        "line_width": 1.0
                    },
                    "labels": {
                        "visible": true,
                        "font_size": 10.0,
                        "format": "Decimal",
                        "significant_digits": 2,
                        "spacing_px": 80.0,
                        "anchors": []
                    }
                }
            }
        }
    }]);
    chart.set_series(&contour.to_string()).unwrap();
    waits_for_its_on_demand_module(
        &mut chart,
        "figgy contour anchor seed pipeline",
        "contour labels",
    )
    .await;
}
