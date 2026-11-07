#![cfg(target_arch = "wasm32")]
//! Feynman diagrams in the browser product (#234): the wrapper commits the
//! layout on registration, `pick_point` answers the parts, `press_feynman`
//! and `on_move` drag them, `selection_details` reports them and
//! `step_selection` walks them.

use figgy::FiggyChart;
use renderer::AnnotationSpace as _;
use renderer::feynman::{FeynmanSeries, FeynmanVertex, ParticleLine, Propagator, VertexRole};
use renderer::layout::{ChartArea, Rect};
use renderer::{AnnotationAxis, Chart, DataRenderType, SeriesConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;

fn canvas() -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(WIDTH);
    canvas.set_height(HEIGHT);
    canvas
}

fn s_channel() -> SeriesConfig {
    let vertex = |id: &str, role| FeynmanVertex::new(id, role);
    let mut diagram = FeynmanSeries {
        vertices: vec![
            vertex("e-", VertexRole::Incoming),
            vertex("e+", VertexRole::Incoming),
            vertex("v1", VertexRole::Internal),
            vertex("v2", VertexRole::Internal),
            vertex("mu-", VertexRole::Outgoing),
            vertex("mu+", VertexRole::Outgoing),
        ],
        propagators: vec![
            Propagator::new("p1", "e-", "v1", ParticleLine::Fermion),
            Propagator::new("p2", "v1", "e+", ParticleLine::Fermion),
            Propagator::new("g", "v1", "v2", ParticleLine::Photon),
            Propagator::new("p3", "v2", "mu-", ParticleLine::Fermion),
            Propagator::new("p4", "mu+", "v2", ParticleLine::Fermion),
        ],
        ..FeynmanSeries::default()
    };
    diagram.vertices[0].label = Some(renderer::text::RichText::plain(
        "e",
        renderer::Color::BLACK,
        14.0,
        "",
    ));
    SeriesConfig {
        series_id: "d".into(),
        source_id: None,
        label: None,
        x_column: String::new(),
        y_column: String::new(),
        z_column: None,
        render_type: DataRenderType::Feynman(diagram),
    }
}

/// A document the size of the canvas, so display pixels are document
/// pixels, over 0..1 on both axes.
fn config() -> renderer::Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    chart.config().clone()
}

fn series(chart: &FiggyChart) -> FeynmanSeries {
    let list: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    list[0].feynman().unwrap().clone()
}

async fn pick(chart: &mut FiggyChart, at: [f32; 2]) -> serde_json::Value {
    let hit = chart.pick_point(at[0], at[1], 4.0).await.unwrap();
    hit.as_string()
        .map(|json| serde_json::from_str(&json).unwrap())
        .unwrap_or(serde_json::Value::Null)
}

async fn details(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&chart.selection_details().await.unwrap()).unwrap()
}

#[wasm_bindgen_test(async)]
async fn a_diagram_is_laid_out_on_registration_picked_dragged_and_reported() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let config = config();
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .unwrap();
    chart
        .set_series(&serde_json::to_string(&vec![s_channel()]).unwrap())
        .unwrap();
    // Registration commits the layout: every vertex is placed.
    let diagram = series(&chart);
    assert!(diagram.vertices.iter().all(|v| v.at.is_some()));
    assert!(!chart.auto_layout("d").unwrap(), "nothing left to place");
    chart.frame().unwrap();

    let space = renderer::annotation::pixel_space_for_config(&config);
    let px = |p: [f64; 2]| {
        [
            space.value_to_px(AnnotationAxis::X, p[0]).unwrap(),
            space.value_to_px(AnnotationAxis::Y, p[1]).unwrap(),
        ]
    };
    let v1 = px(diagram.vertices[2].at.unwrap());
    let hit = pick(&mut chart, v1).await;
    assert_eq!(hit["kind"], "feynman_vertex", "{hit}");
    assert_eq!(hit["series_id"], "d");
    assert_eq!(hit["index"], 2);

    // Press and drag the vertex 20 px up: it moves 20 px in data units.
    assert!(chart.press_feynman("d", "feynman_vertex", 2, None).unwrap());
    chart.on_move(0.0, -20.0).unwrap();
    chart.on_release().expect("release");
    let moved = series(&chart).vertices[2].at.unwrap();
    let before = diagram.vertices[2].at.unwrap();
    let plot = config.data_area().unwrap().0;
    let want = before[1] + 20.0 / plot.height as f64;
    assert!((moved[0] - before[0]).abs() < 1e-6, "{moved:?}");
    assert!((moved[1] - want).abs() < 1e-3, "{moved:?} {want}");
    let selected = details(&chart).await;
    assert_eq!(selected["identity"]["kind"], "feynman_vertex");
    assert_eq!(selected["id"], "v1");
    assert_eq!(selected["role"], "Internal");
    assert_eq!(selected["placed"], true);

    // Step to the next vertex; a propagator bends; a label reports its
    // offset.
    assert!(chart.step_selection(1, 0).unwrap());
    assert_eq!(details(&chart).await["id"], "v2");
    assert!(
        chart
            .press_feynman("d", "feynman_propagator", 2, None)
            .unwrap()
    );
    chart.on_move(0.0, -15.0).unwrap();
    chart.on_release().expect("release");
    assert!(series(&chart).propagators[2].bend.is_some());
    let selected = details(&chart).await;
    assert_eq!(selected["particle"], "Photon");
    assert_eq!(selected["from"], "v1");
    assert!(
        chart
            .press_feynman("d", "feynman_label", 0, Some("vertex".into()))
            .unwrap()
    );
    chart.on_move(3.0, 4.0).unwrap();
    chart.on_release().expect("release");
    let selected = details(&chart).await;
    assert_eq!(selected["identity"]["kind"], "feynman_label");
    assert_eq!(selected["offset"], serde_json::json!([3.0, 4.0]));
    // Unknown parts and rows change nothing.
    assert!(
        !chart
            .press_feynman("d", "feynman_vertex", 99, None)
            .unwrap()
    );
    assert!(!chart.press_feynman("d", "feynman_label", 1, None).unwrap());
    assert!(
        !chart
            .press_feynman("missing", "feynman_vertex", 0, None)
            .unwrap()
    );
}
