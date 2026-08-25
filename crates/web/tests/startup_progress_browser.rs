#![cfg(target_arch = "wasm32")]

use std::{cell::RefCell, rc::Rc};

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const STARTUP_STAGES: [(&str, &str); 38] = [
    ("window", "instance"),
    ("window", "surface"),
    ("window", "adapter"),
    ("window", "device"),
    ("window", "configure"),
    ("window", "figgy frame msaa target"),
    ("renderer", "capabilities"),
    ("renderer", "figgy column pool"),
    ("renderer", "shader modules"),
    ("renderer.prewarm.async", "fullscreen textured"),
    ("renderer.prewarm.async", "precise line"),
    ("renderer.prewarm.async", "hand-drawn line"),
    ("renderer.prewarm.async", "milkyway ribbon"),
    ("renderer.prewarm.async", "milkyway stars"),
    ("renderer.prewarm.async", "constellation line"),
    ("renderer.prewarm.async", "precise scatter"),
    ("renderer.prewarm.async", "mapped scatter"),
    ("renderer.prewarm.async", "picked point ring"),
    ("renderer.prewarm.async", "mapped picked point ring"),
    ("renderer.prewarm.async", "hand-drawn scatter"),
    ("renderer.prewarm.async", "milkyway planets"),
    ("renderer.prewarm.async", "constellation stars"),
    ("renderer.prewarm.async", "precise errorbar"),
    ("renderer.prewarm.async", "mapped errorbar"),
    ("renderer.prewarm.async", "hand-drawn errorbar"),
    ("renderer.prewarm.async", "milkyway jets"),
    ("renderer.prewarm.async", "histogram bars"),
    ("renderer.prewarm.async", "mapped histogram bars"),
    ("renderer.prewarm.async", "selected histogram bin"),
    ("renderer.prewarm.async", "heatmap field"),
    ("renderer.prewarm.async", "contour field"),
    ("renderer.prewarm.async", "label-gapped contour field"),
    ("renderer.prewarm.async", "selected field data"),
    ("renderer.prewarm.async", "contour labels"),
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
