#![cfg(target_arch = "wasm32")]

use std::{cell::RefCell, rc::Rc};

use renderer::data_config::{
    BarOrientation, DataBarStyleConfig, FieldFillConfig, FieldSeries, FillMode, GridLayout,
    MatrixOrientation, MatrixRef, Shading,
};
use renderer::{Color, DataRenderType, SeriesConfig};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

/// What `create` waits for (#299): the window, the renderer's resources and
/// the first frame. No pipeline's asynchronous compile is a startup stage:
/// they all run after `create` has resolved.
const STARTUP_STAGES: [(&str, &str); 13] = [
    ("window", "instance"),
    ("window", "surface"),
    ("window", "adapter"),
    ("window", "device"),
    ("window", "configure"),
    ("window", "figgy frame msaa target"),
    ("renderer", "capabilities"),
    ("renderer", "figgy column pool"),
    ("renderer", "shader modules"),
    ("renderer", "figgy fullscreen textured pipeline"),
    ("renderer", "identity"),
    ("web.create", "chart resources"),
    ("web.create", "first frame"),
];

#[derive(Debug, PartialEq, Eq)]
struct ProgressEvent {
    scope: String,
    stage: String,
    phase: String,
}

fn string_property(value: &JsValue, name: &str) -> String {
    js_sys::Reflect::get(value, &JsValue::from_str(name))
        .unwrap_or_else(|_| panic!("missing progress property {name}"))
        .as_string()
        .unwrap_or_else(|| panic!("progress property {name} was not a string"))
}

fn canvas() -> HtmlCanvasElement {
    let document = web_sys::window()
        .expect("window")
        .document()
        .expect("document");
    let canvas = document
        .create_element("canvas")
        .expect("canvas element")
        .dyn_into::<HtmlCanvasElement>()
        .expect("HtmlCanvasElement");
    canvas.set_width(400);
    canvas.set_height(300);
    canvas
}

#[wasm_bindgen_test(async)]
async fn raw_create_reports_exact_startup_stage_pairs() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = Rc::clone(&events);
    let callback = Closure::wrap(Box::new(move |value: JsValue| {
        captured.borrow_mut().push(ProgressEvent {
            scope: string_property(&value, "scope"),
            stage: string_property(&value, "stage"),
            phase: string_property(&value, "phase"),
        });
    }) as Box<dyn FnMut(JsValue)>);
    let on_event = callback
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();

    let _chart = figgy::FiggyChart::create_with_progress(canvas(), on_event)
        .await
        .expect("raw chart creation failed");

    let expected = STARTUP_STAGES
        .into_iter()
        .flat_map(|(scope, stage)| {
            ["started", "finished"]
                .into_iter()
                .map(move |phase| ProgressEvent {
                    scope: scope.to_string(),
                    stage: stage.to_string(),
                    phase: phase.to_string(),
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(*events.borrow(), expected);
}

/// Count the browser's asynchronous render-pipeline compiles, on every
/// device of this page, and hold their results back: a compile the page
/// issues settles for the page only once the test calls `release()`. What
/// `create` does while they are held is then the code's doing, whatever the
/// speed of the GPU.
const COUNT_COMPILES: &str = r#"
(() => {
  if (globalThis.__figgyCompiles) return;
  const counts = globalThis.__figgyCompiles = { issued: 0, settled: 0 };
  counts.hold = () => { counts.gate = new Promise((open) => { counts.release = open; }); };
  counts.hold();
  const create = GPUDevice.prototype.createRenderPipelineAsync;
  GPUDevice.prototype.createRenderPipelineAsync = function (descriptor) {
    counts.issued += 1;
    const gate = counts.gate;
    const compiled = create.call(this, descriptor);
    const held = Promise.allSettled([compiled, gate]).then(() => compiled);
    const settle = () => { counts.settled += 1; };
    held.then(settle, settle);
    return held;
  };
})()
"#;

fn compile_gate(call: &str) {
    js_sys::eval(&format!("globalThis.__figgyCompiles.{call}()")).unwrap();
}

fn compiles() -> (u32, u32) {
    let counts = js_sys::Reflect::get(&js_sys::global(), &"__figgyCompiles".into()).unwrap();
    let count = |name: &str| {
        js_sys::Reflect::get(&counts, &name.into())
            .unwrap()
            .as_f64()
            .unwrap() as u32
    };
    (count("issued"), count("settled"))
}

/// The chart as an export draws it: every series, complete. A WebGPU
/// canvas cannot be read back in the headless browser these tests run in,
/// so the frames themselves are checked by `first_frame_ready`, which
/// rejects on a GPU error, and the picture by the export.
async fn picture(chart: &mut figgy::FiggyChart) -> Vec<u8> {
    chart.first_frame_ready().await.expect("presented frame");
    chart.export_png(1.0).await.expect("export").to_vec()
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
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

/// One chart per group of pipelines: lines (the demo), bars and a field.
fn show(chart: &mut figgy::FiggyChart, kind: &str) {
    match kind {
        "lines" => chart.load_demo().unwrap(),
        "bars" => {
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
                        fill_color: Color::BLACK,
                        border_color: Color::BLACK,
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
        }
        "field" => {
            chart
                .register_column_f32("fx", &[0.0, 1.0, 2.0, 3.0])
                .unwrap();
            chart.register_column_f32("fy", &[0.0, 1.0, 2.0]).unwrap();
            for i in 0..3 {
                chart
                    .register_column_f32(&format!("z{i}"), &[i as f32, 1.0 + i as f32])
                    .unwrap();
            }
            let mut config: renderer::Config =
                serde_json::from_str(&chart.get_config().unwrap()).unwrap();
            config.bottom_x.min = 0.0;
            config.bottom_x.max = 3.0;
            config.left_y.min = 0.0;
            config.left_y.max = 2.0;
            let mut bar = renderer::default::default_colorbar_options();
            bar.axis.min = 0.0;
            bar.axis.max = 3.0;
            bar.axis.major_spacing = 1.0;
            config.colorbar = Some(bar);
            chart
                .set_config(&serde_json::to_string(&config).unwrap())
                .unwrap();
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
        other => panic!("unknown chart kind {other}"),
    }
}

/// #299: `create` resolves, its first frame presented, without a single
/// pipeline compile having settled, and with no more compiles issued than
/// the browser reports cores. A chart shown while its pipelines compile is
/// presented without the series that wait for them (`frame_complete` is
/// false); its export waits for its own compiles and is the whole chart;
/// once the compiles resolve the frames fill in, and the chart is the one
/// the export drew.
#[wasm_bindgen_test(async)]
async fn first_frame_precedes_the_background_compiles_and_charts_draw_the_same_before_and_after() {
    js_sys::eval(COUNT_COMPILES).unwrap();
    let cores = js_sys::eval("navigator.hardwareConcurrency")
        .unwrap()
        .as_f64()
        .filter(|cores| cores.is_finite() && *cores >= 1.0)
        .map_or(4, |cores| cores as u32);
    for kind in ["lines", "bars", "field"] {
        let before = compiles();
        assert_eq!(before.0, before.1, "{kind}: the page starts idle");
        // No compile settles for the page until the gate is released, so a
        // `create` that awaited one would never resolve.
        compile_gate("hold");
        let canvas = canvas();
        let mut chart = figgy::FiggyChart::create(canvas.clone())
            .await
            .expect("raw chart creation failed");
        let (issued, settled) = compiles();
        assert!(
            issued > before.0 && issued - before.0 <= cores,
            "{kind}: at most {cores} compiles are in flight ({issued} after {before:?})"
        );
        assert_eq!(
            settled, before.1,
            "{kind}: the first frame was presented with every compile still held"
        );
        let empty = picture(&mut chart).await;

        show(&mut chart, kind);
        chart.frame().expect("a frame while the compiles are held");
        assert!(
            !chart.frame_complete(),
            "{kind}: the frame leaves out the series whose pipelines compile"
        );
        assert!(
            compiles().0 - before.0 <= cores,
            "{kind}: the screen's request stays within the limit"
        );
        // An export compiles what it draws through the browser's async
        // compiler and waits for those compiles (#299), so it ends once
        // they settle: the gate is released after the export is started.
        let export = chart.export_png(1.0);
        compile_gate("release");
        let early = export.await.expect("export").to_vec();
        assert!(early != empty, "{kind}: the export draws the whole chart");

        let late = picture(&mut chart).await;
        assert!(chart.frame_complete(), "{kind}: the chart filled in");
        assert!(early == late, "{kind}: the chart changed once compiled");
        let mut waited = 0;
        while {
            let (issued, settled) = compiles();
            settled < issued
        } {
            sleep(100).await;
            waited += 100;
            assert!(waited < 600_000, "{kind}: the compiles never settled");
        }
    }
}
