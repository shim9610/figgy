#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use renderer::CpuTextMeasure;
use renderer::config::TickVisibility;
use renderer::layout::{ChartArea, Rect, Side};
use renderer::line::LineStylePreset;
use renderer::select::{
    ColorBarAxisElement, ColorBarLabelElement, ColorBarTitleElement, Selectable,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window()
        .expect("window")
        .document()
        .expect("document");
    let canvas = document
        .create_element("canvas")
        .expect("canvas element")
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .expect("HtmlCanvasElement");
    canvas.set_width(960);
    canvas.set_height(640);
    canvas
}

fn title_offsets(chart: &FiggyChart) -> (f64, f64) {
    let config: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    (
        config["chart_title"]["offset_x"].as_f64().unwrap(),
        config["chart_title"]["offset_y"].as_f64().unwrap(),
    )
}

fn chart_config(chart: &FiggyChart) -> renderer::Config {
    serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json")
}

#[wasm_bindgen_test(async)]
async fn chart_title_hit_press_move_and_release_updates_config_offset() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    chart.set_title("Drag me").expect("set title");

    let title_center = (480.0, 16.0);
    assert_eq!(
        chart.hit_test(title_center.0, title_center.1).as_deref(),
        Some("chart_title")
    );
    assert_eq!(title_offsets(&chart), (0.0, 0.0));

    assert!(
        chart
            .on_press(title_center.0, title_center.1)
            .expect("press title")
    );
    assert!(chart.has_selection());
    chart.on_move(18.0, 7.0).expect("drag title");
    assert_eq!(title_offsets(&chart), (18.0, 7.0));
    assert_eq!(
        chart
            .hit_test(title_center.0 + 18.0, title_center.1 + 7.0)
            .as_deref(),
        Some("chart_title")
    );

    chart.on_release();
    chart.on_move(11.0, 13.0).expect("move after release");
    assert_eq!(title_offsets(&chart), (18.0, 7.0));
}

#[wasm_bindgen_test(async)]
async fn colorbar_axis_labels_and_title_edit_select_and_drag_independently() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");

    let mut config: renderer::Config =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    // Match the canvas exactly so pointer deltas and document offsets are 1:1.
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 960,
        height: 640,
    });
    let mut bar = renderer::default::default_colorbar_options();
    bar.side = Side::Right;
    bar.length_frac = 0.5;
    config.colorbar = Some(bar);
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .expect("set colorbar config");

    chart
        .set_colorbar_title("intensity")
        .expect("set colorbar title");
    let mut axis = {
        let config: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
        config.colorbar.unwrap().axis
    };
    axis.tick = TickVisibility::Both;
    axis.line_style = LineStylePreset::ShortDot;
    axis.inverted = true;
    chart
        .set_colorbar_axis(&serde_json::to_string(&axis).unwrap())
        .expect("set colorbar axis");

    let cfg = chart_config(&chart);
    let bar = cfg.colorbar.as_ref().expect("colourbar");
    assert_eq!(bar.axis.tick, TickVisibility::Both);
    assert_eq!(bar.axis.line_style, LineStylePreset::ShortDot);
    assert!(bar.axis.inverted);
    assert!(bar.axis.title_option.visible);

    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let title = ColorBarTitleElement
        .bounds(&cfg, &measure)
        .expect("title bounds");
    let title_point = (title.x + title.width * 0.5, title.y + title.height * 0.5);
    assert_eq!(
        chart.hit_test(title_point.0, title_point.1).as_deref(),
        Some("colorbar_title")
    );
    chart.on_press(title_point.0, title_point.1).unwrap();
    chart.on_move(3.0, 7.0).unwrap();
    chart.on_release();
    let cfg = chart_config(&chart);
    let title = &cfg.colorbar.as_ref().unwrap().axis.title_option;
    assert_eq!((title.offset_x, title.offset_y), (7.0, -3.0));

    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let labels = ColorBarLabelElement
        .bounds(&cfg, &measure)
        .expect("label bounds");
    let label_point = (labels.x + labels.width * 0.5, labels.y + 1.0);
    assert_eq!(
        chart.hit_test(label_point.0, label_point.1).as_deref(),
        Some("colorbar_tick_labels")
    );
    chart.on_press(label_point.0, label_point.1).unwrap();
    chart.on_move(4.0, -2.0).unwrap();
    chart.on_release();
    let cfg = chart_config(&chart);
    let labels = &cfg.colorbar.as_ref().unwrap().axis.label_style;
    assert_eq!((labels.label_offset_x, labels.label_offset_y), (4.0, -2.0));

    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let axis = ColorBarAxisElement
        .bounds(&cfg, &measure)
        .expect("axis bounds");
    let axis_point = (axis.x + axis.width * 0.5, axis.y + axis.height * 0.5);
    assert_eq!(
        chart.hit_test(axis_point.0, axis_point.1).as_deref(),
        Some("colorbar_axis")
    );
    chart.on_press(axis_point.0, axis_point.1).unwrap();
    chart.on_move(6.0, 99.0).unwrap();
    chart.on_release();
    assert_eq!(
        chart_config(&chart)
            .colorbar
            .as_ref()
            .unwrap()
            .axis
            .line_offset,
        6.0
    );
}

// The pointer route follows the active layer: annotation items answer and
// drag only while `"Annotations"` is active, and leaving that layer clears the
// annotation selection.
#[wasm_bindgen_test(async)]
async fn annotation_layer_routes_press_and_drag_by_active_layer() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    let mut config = chart_config(&chart);
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 960,
        height: 640,
    });
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .expect("set config");
    let item = renderer::Annotation::new(renderer::AnnotationKind::Text {
        anchor: renderer::Anchor::Ratio { x: 0.4, y: 0.4 },
        text: renderer::text::RichText::plain("note", renderer::Color::BLACK, 14.0, ""),
        box_style: None,
    });
    assert_eq!(
        chart
            .add_annotation(&serde_json::to_string(&item).unwrap())
            .expect("add annotation"),
        0
    );
    chart.frame().expect("frame");
    let drawn: serde_json::Value = serde_json::from_str(&chart.annotations().unwrap()).unwrap();
    let rect = &drawn[0]["rect"];
    let point = (
        (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap() * 0.5) as f32,
        (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap() * 0.5) as f32,
    );

    // Base: the item is not part of the route.
    assert_ne!(
        chart.hit_test(point.0, point.1).as_deref(),
        Some("annotation_0")
    );
    chart.on_press(point.0, point.1).unwrap();
    chart.on_release();
    assert_eq!(chart.selected_annotation(), None);

    // Annotations: the item answers on top, a press selects it and a move
    // drags it, keeping its ratio anchor.
    assert_eq!(
        chart.set_active_layer(r#""Annotations""#).unwrap(),
        r#""Annotations""#
    );
    assert_eq!(
        chart.hit_test(point.0, point.1).as_deref(),
        Some("annotation_0")
    );
    assert!(chart.on_press(point.0, point.1).unwrap());
    assert_eq!(chart.selected_annotation(), Some(0));
    assert!(chart.has_selection());
    chart.on_move(24.0, 16.0).unwrap();
    chart.on_release();
    let moved = match &chart_config(&chart).annotations.items[0].kind {
        renderer::AnnotationKind::Text { anchor, .. } => *anchor,
        _ => unreachable!(),
    };
    let renderer::Anchor::Ratio { x, y } = moved else {
        panic!("the anchor changed kind: {moved:?}");
    };
    assert!(x > 0.4 && y > 0.4, "{moved:?}");

    // Leaving the layer clears the annotation selection.
    chart.set_active_layer(r#""Base""#).unwrap();
    assert_eq!(chart.selected_annotation(), None);
}
