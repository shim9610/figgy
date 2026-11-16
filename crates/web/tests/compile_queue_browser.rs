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
/// `None`), and record every `createRenderPipelineAsync` from now on: its
/// label in issue order, how many are open and the most ever open.
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
  P.createRenderPipeline = function (d) {{
    if (!Q.resolved.has(d.label || '')) Q.cold.push(d.label || '');
    return P.__figgyOriginalSync.call(this, d);
  }};
  P.createRenderPipelineAsync = function (d) {{
    Q.device = this;
    Q.labels.push(d.label || '');
    Q.open += 1;
    Q.peak = Math.max(Q.peak, Q.open);
    // A label the test makes the browser reject, or holds until the device
    // is lost (then it resolves, as the specification says).
    const p = d.label === Q.reject ? Promise.reject(new Error('rejected by the test'))
      : d.label === Q.hold ? this.lost.then(() => undefined)
      : original.call(this, d);
    const done = () => {{ Q.open -= 1; }};
    p.then(() => {{ Q.resolved.add(d.label || ''); done(); }}, done);
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
            chart.pipeline_status(),
            number("globalThis.__queue.labels.length"),
            number("globalThis.__queue.open"),
        );
        turn().await;
    }
}

/// The chart's compile status.
fn status(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&chart.pipeline_status()).unwrap()
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
            chart.pipeline_status()
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
    assert!(basic["compiling"].as_u64().unwrap() > 0, "{basic}");
    assert_eq!(
        number("globalThis.__queue.open"),
        2,
        "basic compiles in flight"
    );
    chart.require_shader_set("full".into()).unwrap();
    let full = status(&chart);
    assert_eq!(chart.shader_set(), "full");
    assert!(full["total"].as_u64().unwrap() > 20, "{full}");
    chart.require_shader_set("full".into()).unwrap();
    assert_eq!(
        status(&chart)["total"],
        full["total"],
        "a second request schedules nothing"
    );
    show_heatmap(&mut chart);
    complete_within(&mut chart, 180.0, "the heatmap after a mid-compile upgrade").await;
    assert_eq!(status(&chart)["failed"], 0);
}

/// An upgrade once every basic compile has settled schedules the full set,
/// and a heatmap shown then completes.
#[wasm_bindgen_test(async)]
async fn an_upgrade_after_the_basic_compiles_settled_completes() {
    hook(Some(4));
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    within(&chart, 180.0, "the basic set compiled", || {
        status(&chart)["compiling"] == 0
    })
    .await;
    chart.require_shader_set("full".into()).unwrap();
    let full = status(&chart);
    assert!(full["total"].as_u64().unwrap() > 20, "{full}");
    assert!(full["compiling"].as_u64().unwrap() > 0, "{full}");
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
        status(&chart)["compiling"] == 0
    })
    .await;
    let after = status(&chart);
    assert!(after["failed"].as_u64().unwrap() >= 1, "{after}");
    assert_eq!(
        after["ready"].as_u64().unwrap() + after["failed"].as_u64().unwrap(),
        after["total"].as_u64().unwrap()
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
    assert_eq!(after["compiling"], 0, "{after}");
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
        status(&chart)["total"].as_u64().unwrap() > issued as u64 + 20,
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
    let cold = js_sys::eval(&format!("globalThis.__queue.cold.includes({label:?})"))
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
        chart.pipeline_status(),
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
}
