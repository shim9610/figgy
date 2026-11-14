#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use renderer::CpuTextMeasure;
use renderer::config::TickVisibility;
use renderer::layout::{ChartArea, Rect, Side};
use renderer::line::LineStylePreset;
use renderer::select::{
    ColorBarAxisElement, ColorBarLabelElement, ColorBarTitleElement, SelectionGeometry,
};
use wasm_bindgen::{JsCast, JsValue};
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
        chart
            .hit_test(title_center.0, title_center.1)
            .unwrap()
            .as_deref(),
        Some("chart_title")
    );
    assert_eq!(title_offsets(&chart), (0.0, 0.0));

    assert!(
        chart
            .on_press(title_center.0, title_center.1)
            .expect("press title")
    );
    assert!(chart.has_selection().unwrap());
    chart.on_move(18.0, 7.0).expect("drag title");
    assert_eq!(title_offsets(&chart), (18.0, 7.0));
    assert_eq!(
        chart
            .hit_test(title_center.0 + 18.0, title_center.1 + 7.0)
            .unwrap()
            .as_deref(),
        Some("chart_title")
    );

    chart.on_release().unwrap();
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
        chart
            .hit_test(title_point.0, title_point.1)
            .unwrap()
            .as_deref(),
        Some("colorbar_title")
    );
    chart.on_press(title_point.0, title_point.1).unwrap();
    chart.on_move(3.0, 7.0).unwrap();
    chart.on_release().unwrap();
    let cfg = chart_config(&chart);
    let title = &cfg.colorbar.as_ref().unwrap().axis.title_option;
    assert_eq!((title.offset_x, title.offset_y), (7.0, -3.0));

    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let labels = ColorBarLabelElement
        .bounds(&cfg, &measure)
        .expect("label bounds");
    let label_point = (labels.x + labels.width * 0.5, labels.y + 1.0);
    assert_eq!(
        chart
            .hit_test(label_point.0, label_point.1)
            .unwrap()
            .as_deref(),
        Some("colorbar_tick_labels")
    );
    chart.on_press(label_point.0, label_point.1).unwrap();
    chart.on_move(4.0, -2.0).unwrap();
    chart.on_release().unwrap();
    let cfg = chart_config(&chart);
    let labels = &cfg.colorbar.as_ref().unwrap().axis.label_style;
    assert_eq!((labels.label_offset_x, labels.label_offset_y), (4.0, -2.0));

    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let axis = ColorBarAxisElement
        .bounds(&cfg, &measure)
        .expect("axis bounds");
    let axis_point = (axis.x + axis.width * 0.5, axis.y + axis.height * 0.5);
    assert_eq!(
        chart
            .hit_test(axis_point.0, axis_point.1)
            .unwrap()
            .as_deref(),
        Some("colorbar_axis")
    );
    chart.on_press(axis_point.0, axis_point.1).unwrap();
    chart.on_move(6.0, 99.0).unwrap();
    chart.on_release().unwrap();
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
        align: Default::default(),
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
        chart.hit_test(point.0, point.1).unwrap().as_deref(),
        Some("annotation_0")
    );
    chart.on_press(point.0, point.1).unwrap();
    chart.on_release().unwrap();
    assert_eq!(chart.selected_annotation(), None);

    // Annotations: the item answers on top, a press selects it and a move
    // drags it, keeping its ratio anchor.
    assert_eq!(
        chart.set_active_layer(r#""Annotations""#).unwrap(),
        r#""Annotations""#
    );
    assert_eq!(
        chart.hit_test(point.0, point.1).unwrap().as_deref(),
        Some("annotation_0")
    );
    assert!(chart.on_press(point.0, point.1).unwrap());
    assert_eq!(chart.selected_annotation(), Some(0));
    assert!(chart.has_selection().unwrap());
    chart.on_move(24.0, 16.0).unwrap();
    chart.on_release().unwrap();
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

/// The drawn box of the first tick label whose text starts with `lead`.
struct FindLabel {
    lead: char,
    found: Option<renderer::layout::RectF>,
}

impl renderer::sink::Sink for FindLabel {
    fn text(&mut self, _: renderer::sink::Slot, text: &renderer::sink::PlacedText<'_>) {
        // A plain label or a rich one (a power of ten): its first character.
        let lead = match text.text {
            renderer::sink::TextRef::Rich(rich) => rich.segments.first().map(|s| s.text),
            renderer::sink::TextRef::Plain { text, .. } => text.chars().next(),
        };
        if self.found.is_none() && lead == Some(self.lead) {
            self.found = Some(text.rect());
        }
    }
    fn stroke(
        &mut self,
        _: renderer::sink::Slot,
        _: [f32; 2],
        _: [f32; 2],
        _: &renderer::sink::StrokeStyle<'_>,
    ) {
    }
    fn rect(
        &mut self,
        _: renderer::sink::Slot,
        _: renderer::layout::RectF,
        _: Option<renderer::Color>,
        _: Option<renderer::sink::Border<'_>>,
    ) {
    }
    fn triangle(&mut self, _: renderer::sink::Slot, _: [[f32; 2]; 3], _: renderer::Color) {}
    fn path(
        &mut self,
        _: renderer::sink::Slot,
        _: &[renderer::sink::PathSegment],
        _: &renderer::sink::StrokeStyle<'_>,
    ) {
    }
    fn polygon(
        &mut self,
        _: renderer::sink::Slot,
        _: &[[f32; 2]],
        _: Option<renderer::Color>,
        _: Option<&renderer::sink::StrokeStyle<'_>>,
    ) {
    }
    fn ramp(
        &mut self,
        _: renderer::sink::Slot,
        _: renderer::layout::RectF,
        _: &renderer::ColorBarOptions,
    ) {
    }
}

fn label_starting_with(cfg: &renderer::Config, side: Side, lead: char) -> renderer::layout::RectF {
    use renderer::Selectable;
    let mut find = FindLabel { lead, found: None };
    renderer::AxisLabelElement { side }.emit(
        cfg,
        &CpuTextMeasure::for_style(&cfg.draw_style),
        &mut find,
    );
    find.found
        .unwrap_or_else(|| panic!("no label starting with {lead:?}"))
}

// The adaptive log tick policy (#42) through the JSON SSoT: `set_config` and
// `set_colorbar_axis` take `"log_ticks": "Adaptive"`, `get_config` writes it
// back and leaves the default out, and a 2e4 label of the 1-2-5 ladder is
// inside the side's `decoration_bounds`, hits as the side's tick labels and
// drags with them.
#[wasm_bindgen_test(async)]
async fn an_adaptive_log_axis_round_trips_and_its_2e4_label_hits_and_drags() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    let mut json: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    assert!(json["left_y"].get("log_ticks").is_none());
    json["chart_area"] = serde_json::json!({ "x": 0, "y": 0, "width": 960, "height": 640 });
    json["left_y"]["scale"] = "Logarithmic".into();
    json["left_y"]["min"] = 1e4.into();
    json["left_y"]["max"] = 8e4.into();
    json["left_y"]["log_ticks"] = "Adaptive".into();
    json["left_y"]["label_style"]["format"] = "Power".into();
    json["left_y"]["label_style"]["significant_digits"] = 1.into();
    chart
        .set_config(&json.to_string())
        .expect("set adaptive config");
    let written: serde_json::Value =
        serde_json::from_str(&chart.get_config().unwrap()).expect("config json");
    assert_eq!(written["left_y"]["log_ticks"], "Adaptive");
    assert!(written["bottom_x"].get("log_ticks").is_none());
    let cfg = chart_config(&chart);
    assert_eq!(
        cfg.left_y.log_ticks,
        renderer::config::LogTickPolicy::Adaptive
    );

    let label = label_starting_with(&cfg, Side::Left, '2');
    let (cx, cy) = label.center();
    assert_eq!(
        chart.hit_test(cx, cy).unwrap().as_deref(),
        Some("tick_labels_left")
    );
    let bounds: serde_json::Value =
        serde_json::from_str(&chart.decoration_bounds("tick_labels_left").unwrap()).unwrap();
    let rect: Vec<f32> = bounds["rect"]
        .as_array()
        .expect("bounds rect")
        .iter()
        .map(|v| v.as_f64().unwrap() as f32)
        .collect();
    assert!(
        rect[0] <= label.x
            && rect[1] <= label.y
            && rect[0] + rect[2] >= label.x + label.width
            && rect[1] + rect[3] >= label.y + label.height,
        "{rect:?} does not hold {label:?}"
    );
    assert!(chart.on_press(cx, cy).expect("press the labels"));
    chart.on_move(5.0, 12.0).expect("drag the labels");
    chart.on_release().unwrap();
    let cfg = chart_config(&chart);
    let style = &cfg.left_y.label_style;
    assert_eq!((style.label_offset_x, style.label_offset_y), (5.0, 12.0));
    let moved = label_starting_with(&cfg, Side::Left, '2');
    assert_eq!((moved.x - label.x, moved.y - label.y), (5.0, 12.0));
    let (mx, my) = moved.center();
    assert_eq!(
        chart.hit_test(mx, my).unwrap().as_deref(),
        Some("tick_labels_left")
    );

    // The colourbar's axis takes the policy through its focused setter.
    let mut config = chart_config(&chart);
    config.colorbar = Some(renderer::default::default_colorbar_options());
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .expect("set colorbar config");
    let mut axis = serde_json::to_value(&chart_config(&chart).colorbar.unwrap().axis).unwrap();
    assert!(axis.get("log_ticks").is_none());
    axis["scale"] = "Logarithmic".into();
    axis["min"] = 1.0.into();
    axis["max"] = 100.0.into();
    axis["log_ticks"] = "Adaptive".into();
    chart
        .set_colorbar_axis(&axis.to_string())
        .expect("set colorbar axis");
    let written: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    assert_eq!(written["colorbar"]["axis"]["log_ticks"], "Adaptive");
}

// Broken axes (#200) through the JSON SSoT: `set_config` takes `breaks` on
// `left_y`, `get_config` writes it back and leaves it out where absent, and
// a tick label on the upper piece hits as the side's tick labels and drags
// with them.
#[wasm_bindgen_test(async)]
async fn a_broken_axis_round_trips_and_its_upper_labels_hit_and_drag() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    let mut json: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    assert!(json["left_y"].get("breaks").is_none());
    json["chart_area"] = serde_json::json!({ "x": 0, "y": 0, "width": 960, "height": 640 });
    json["left_y"]["min"] = 0.0.into();
    json["left_y"]["max"] = 100.0.into();
    json["left_y"]["major_spacing"] = 10.0.into();
    json["left_y"]["breaks"] = serde_json::json!({
        "ranges": [{ "from": 15.0, "to": 85.0 }],
        "style": "Panels",
        "gap": 0.05,
    });
    chart
        .set_config(&json.to_string())
        .expect("set broken config");
    let written: serde_json::Value =
        serde_json::from_str(&chart.get_config().unwrap()).expect("config json");
    assert_eq!(written["left_y"]["breaks"]["style"], "Panels");
    assert_eq!(written["left_y"]["breaks"]["ranges"][0]["to"], 85.0);
    assert!(written["bottom_x"].get("breaks").is_none());
    let cfg = chart_config(&chart);
    assert_eq!(
        cfg.left_y.breaks.as_ref().map(|b| b.style),
        Some(renderer::config::AxisBreakStyle::Panels)
    );

    let label = label_starting_with(&cfg, Side::Left, '9');
    let (cx, cy) = label.center();
    assert_eq!(
        chart.hit_test(cx, cy).unwrap().as_deref(),
        Some("tick_labels_left")
    );
    assert!(chart.on_press(cx, cy).expect("press the labels"));
    chart.on_move(5.0, 12.0).expect("drag the labels");
    chart.on_release().unwrap();
    let cfg = chart_config(&chart);
    let moved = label_starting_with(&cfg, Side::Left, '9');
    assert_eq!((moved.x - label.x, moved.y - label.y), (5.0, 12.0));
}

// #205: on the base layer a subplot's frame is hit on its plot, dragged and
// resized by whole pixels, placed anywhere by the frame API, and read back.
#[wasm_bindgen_test(async)]
async fn a_subplot_frame_hits_drags_resizes_and_moves_out_of_its_parent() {
    let mut chart = FiggyChart::create(canvas())
        .await
        .expect("FiggyChart.create");
    let mut json: serde_json::Value =
        serde_json::from_str(&chart.get_config().expect("get_config")).expect("config json");
    json["chart_area"] = serde_json::json!({ "x": 0, "y": 0, "width": 960, "height": 640 });
    let mut inset = json.clone();
    inset["chart_area"] = serde_json::json!({ "x": 560, "y": 80, "width": 300, "height": 220 });
    inset["plot_background"] = serde_json::json!({ "r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0 });
    json["subplots"] = serde_json::json!([{ "config": inset, "series": [] }]);
    chart
        .set_config(&json.to_string())
        .expect("a config with an inset");
    let cfg = chart_config(&chart);
    let frame = cfg.subplots[0].config.chart_area.0;
    assert_eq!(
        chart.subplot_area(0).unwrap(),
        serde_json::to_string(&frame).unwrap()
    );
    assert_eq!(chart.insets().unwrap(), "[0]");
    // The canvas is the document's size, so display and document pixels agree.
    let plot = cfg.subplot_config(0).unwrap().data_area().unwrap().0;
    let (cx, cy) = (
        plot.x as f32 + plot.width as f32 / 2.0,
        plot.y as f32 + plot.height as f32 / 2.0,
    );
    assert_eq!(
        chart.hit_test(cx, cy).unwrap().as_deref(),
        Some("subplot_0:frame")
    );
    assert!(chart.on_press(cx, cy).expect("press the inset's plot"));
    chart.on_move(30.0, 20.0).expect("drag the frame");
    chart.on_release().expect("release the frame");
    let moved = chart_config(&chart).subplots[0].config.chart_area.0;
    assert_eq!(
        (moved.x, moved.y, moved.width, moved.height),
        (frame.x + 30, frame.y + 20, frame.width, frame.height)
    );
    // The south-east handle resizes it inward.
    let bounds: serde_json::Value =
        serde_json::from_str(&chart.decoration_bounds("subplot_0:frame").unwrap()).unwrap();
    let handles = bounds["handles"].as_array().expect("handles");
    assert_eq!(handles.len(), 8);
    let centre = |h: &serde_json::Value| {
        (
            (h[0].as_f64().unwrap() + h[2].as_f64().unwrap() / 2.0) as f32,
            (h[1].as_f64().unwrap() + h[3].as_f64().unwrap() / 2.0) as f32,
        )
    };
    let (hx, hy) = handles
        .iter()
        .map(centre)
        .max_by(|a, b| (a.0 + a.1).partial_cmp(&(b.0 + b.1)).unwrap())
        .unwrap();
    assert!(chart.on_press(hx, hy).expect("press the handle"));
    chart.on_move(-40.0, -30.0).expect("drag the handle");
    chart.on_release().expect("release the handle");
    let resized = chart_config(&chart).subplots[0].config.chart_area.0;
    assert_eq!(
        (resized.x, resized.y, resized.width, resized.height),
        (moved.x, moved.y, moved.width - 40, moved.height - 30)
    );
    // The API places the frame anywhere, here beyond the parent's right
    // edge, where the subplot is a panel of its own.
    chart
        .set_subplot_area(0, 1000, 40, 300, 220)
        .expect("place the frame outside the parent");
    assert_eq!(
        chart.subplot_area(0).unwrap(),
        r#"{"x":1000,"y":40,"width":300,"height":220}"#
    );
    assert_eq!(chart.insets().unwrap(), "[]");
    // The subplot's text follows its cell (#292): the factor is the cube
    // root of the cell's smaller share of the 960x640 parent chart, the
    // resolved chart draws the stored sizes times it and the stored sizes
    // stay through a resize, a re-fit and both modes.
    let stored_size = chart_config(&chart).subplots[0]
        .config
        .chart_title
        .text
        .font_size;
    let drawn_size = |chart: &FiggyChart| {
        chart_config(chart)
            .subplot_config(0)
            .unwrap()
            .chart_title
            .text
            .font_size
    };
    assert_eq!(chart.subplot_text_scale(0).unwrap(), "Auto");
    let small = chart.subplot_text_factor(0).unwrap();
    assert!((small - (300.0f32 / 960.0).cbrt()).abs() < 1e-6, "{small}");
    assert_eq!(drawn_size(&chart), stored_size * small);
    chart
        .set_subplot_area(0, 1000, 40, 600, 440)
        .expect("a larger frame");
    let large = chart.subplot_text_factor(0).unwrap();
    assert!((large - (600.0f32 / 960.0).cbrt()).abs() < 1e-6, "{large}");
    assert_eq!(drawn_size(&chart), stored_size * large);
    chart
        .fit_layout(js_sys::Array::new())
        .expect("fit the resized cell");
    assert_eq!(drawn_size(&chart), stored_size * large);
    chart
        .set_subplot_text_scale(0, "Fixed")
        .expect("draw the stored sizes");
    assert_eq!(chart.subplot_text_scale(0).unwrap(), "Fixed");
    assert_eq!(chart.subplot_text_factor(0).unwrap(), 1.0);
    assert_eq!(drawn_size(&chart), stored_size);
    let half = chart.set_subplot_text_scale(0, "Half").unwrap_err();
    assert_eq!(
        js_sys::Reflect::get(&half, &JsValue::from_str("code"))
            .unwrap()
            .as_string()
            .as_deref(),
        Some("subplot_text_mode")
    );
    let accepted: Vec<String> =
        js_sys::Array::from(&js_sys::Reflect::get(&half, &JsValue::from_str("accepted")).unwrap())
            .iter()
            .filter_map(|mode| mode.as_string())
            .collect();
    assert_eq!(accepted, ["Auto", "Fixed"]);
    assert!(chart.set_subplot_text_scale(3, "Auto").is_err());
    assert!(chart.subplot_text_scale(3).is_err());
    assert!(chart.subplot_text_factor(3).is_err());
    chart
        .set_subplot_text_scale(0, "Auto")
        .expect("follow the cell again");
    assert_eq!(chart.subplot_text_scale(0).unwrap(), "Auto");
    assert_eq!(chart.subplot_text_factor(0).unwrap(), large);
    assert_eq!(
        chart_config(&chart).subplots[0]
            .config
            .chart_title
            .text
            .font_size,
        stored_size
    );
    chart
        .set_subplot_area(0, 1000, 40, 300, 220)
        .expect("the small frame again");
    // A frame too small for its plot is refused and nothing changes.
    let small = chart.set_subplot_area(0, 0, 0, 4, 4).unwrap_err();
    assert_eq!(
        js_sys::Reflect::get(&small, &JsValue::from_str("code"))
            .unwrap()
            .as_string()
            .as_deref(),
        Some("subplot_area")
    );
    let past = chart.set_subplot_area(7, 0, 0, 400, 400).unwrap_err();
    assert_eq!(
        js_sys::Reflect::get(&past, &JsValue::from_str("code"))
            .unwrap()
            .as_string()
            .as_deref(),
        Some("subplot_index")
    );
    assert_eq!(
        chart.subplot_area(0).unwrap(),
        r#"{"x":1000,"y":40,"width":300,"height":220}"#
    );
    chart
        .set_subplot_background(0, "null")
        .expect("clear the plot background");
    assert!(
        chart_config(&chart).subplots[0]
            .config
            .plot_background
            .is_none()
    );
    // The chart's own fill (#265) is set and cleared the same way.
    chart
        .set_background(r#"{"r":1.0,"g":0.98,"b":0.94,"a":1.0}"#)
        .expect("fill the chart area");
    assert_eq!(
        chart_config(&chart).background,
        Some(renderer::Color::new(1.0, 0.98, 0.94, 1.0))
    );
    chart.set_background("null").expect("clear the chart fill");
    assert!(chart_config(&chart).background.is_none());
    // A theme is a family by name (#291, #295): the chart draws a line, so
    // it takes the family's line member. It writes the look into the chart
    // and leaves the subplot it is not given alone; a name that is not a
    // family or a bad list changes nothing.
    use renderer::theme::{ChartKind, ThemeFamily};
    chart
        .register_column_f32("x", &[0.0, 1.0, 2.0])
        .expect("register x");
    chart
        .register_column_f32("y", &[0.0, 1.0, 4.0])
        .expect("register y");
    chart
        .add_line_series("line", "x", "y", 2.0, "")
        .expect("add a line series");
    let names: Vec<String> = chart
        .theme_names()
        .iter()
        .filter_map(|name| name.as_string())
        .collect();
    assert_eq!(names, ThemeFamily::NAMES);
    let before = chart_config(&chart);
    let series_before = chart.get_series().expect("get_series");
    assert!(chart.apply_theme("neon", "all").is_err());
    assert!(chart.apply_theme("midnight", "[nope").is_err());
    // A look that is not a family is no name.
    assert!(chart.apply_theme("parchment", "all").is_err());
    assert_eq!(chart_config(&chart), before);
    assert_eq!(chart.get_series().expect("get_series"), series_before);
    chart
        .apply_theme("midnight", "none")
        .expect("apply the midnight theme");
    let themed = chart_config(&chart);
    let dark = ThemeFamily::Midnight.theme(ChartKind::Line);
    assert_eq!(themed.background, dark.chrome.background);
    assert_eq!(themed.bottom_x.line_color, dark.chrome.frame_ink);
    assert_eq!(themed.subplots[0].config.background, None);
    assert_eq!(
        (themed.bottom_x.min, themed.bottom_x.max),
        (before.bottom_x.min, before.bottom_x.max)
    );
    chart
        .apply_theme("midnight", "[0]")
        .expect("theme the subplot too");
    assert_eq!(
        chart_config(&chart).subplots[0].config.background,
        dark.chrome.background
    );
    // A family (#295) themes a chart whose subplot is of another kind in
    // one call: the line chart takes the family's line member, the scatter
    // inset its scatter member.
    let mut series: Vec<renderer::SeriesConfig> =
        serde_json::from_str(&chart.get_series().expect("get_series")).expect("series json");
    series.push(renderer::SeriesConfig::scatter("dots", "x", "y"));
    chart
        .set_series(&serde_json::to_string(&series).unwrap())
        .expect("add a scatter series");
    let mut config = chart_config(&chart);
    config.subplots[0].series = vec!["dots".into()];
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .expect("draw the scatter series in the inset");
    let config = chart_config(&chart);
    let base = ChartKind::of(&config, &series, renderer::LayerRef::Base);
    let inset = ChartKind::of(&config, &series, renderer::LayerRef::Subplot(0));
    assert_eq!(
        (base, inset),
        (Some(ChartKind::Line), Some(ChartKind::Scatter))
    );
    chart
        .apply_theme("warm-paper", "all")
        .expect("a family over both kinds");
    let themed = chart_config(&chart);
    let line = ThemeFamily::WarmPaper.theme(ChartKind::Line);
    let scatter = ThemeFamily::WarmPaper.theme(ChartKind::Scatter);
    assert_eq!(themed.background, line.chrome.background);
    assert_eq!(themed.chart_title.text.color, line.chrome.ink);
    assert_eq!(
        themed.subplots[0].config.background,
        scatter.chrome.background
    );
    assert_eq!(
        themed.subplots[0].config.grid.show_major_x,
        scatter.chrome.grid.major_x.show
    );
    let styled: Vec<renderer::SeriesConfig> =
        serde_json::from_str(&chart.get_series().expect("get_series")).expect("series json");
    let dots = styled.iter().find(|item| item.series_id == "dots").unwrap();
    let renderer::DataRenderType::Xy(xy) = &dots.render_type else {
        panic!("an x-y series");
    };
    assert_eq!(
        xy.scatter.as_ref().map(|marker| marker.point_size),
        Some(scatter.series.lines.marker_size)
    );

    // Typed errors (#295): each refusal throws an `Error` with a `code`,
    // the names on an unknown name, the index and count on a subplot index.
    let field = |error: &JsValue, key: &str| {
        js_sys::Reflect::get(error, &JsValue::from_str(key)).expect("an error object")
    };
    let code = |error: &JsValue| field(error, "code").as_string().unwrap_or_default();
    let unknown = chart.apply_theme("neon", "all").unwrap_err();
    assert_eq!(code(&unknown), "theme_unknown_name");
    let names: Vec<String> = js_sys::Array::from(&field(&unknown, "names"))
        .iter()
        .filter_map(|name| name.as_string())
        .collect();
    assert_eq!(names, ThemeFamily::NAMES);
    assert_eq!(
        code(&chart.apply_theme("editorial", "all").unwrap_err()),
        "theme_unknown_name"
    );
    let index = chart.apply_theme("gallery", "[4]").unwrap_err();
    assert_eq!(code(&index), "subplot_index");
    assert_eq!(field(&index, "index").as_f64(), Some(4.0));
    assert_eq!(field(&index, "count").as_f64(), Some(1.0));
    for error in [
        chart.subplot_text_scale(4).unwrap_err(),
        chart.subplot_text_factor(4).unwrap_err(),
        chart.set_subplot_text_scale(4, "Auto").unwrap_err(),
        chart.subplot_area(4).unwrap_err(),
        chart.set_subplot_background(4, "null").unwrap_err(),
    ] {
        assert_eq!(code(&error), "subplot_index");
    }
    assert_eq!(
        code(&chart.theme("neon").unwrap_err()),
        "theme_unknown_name"
    );
    assert_eq!(
        code(&chart.subplot_theme(4, "midnight").unwrap_err()),
        "subplot_index"
    );
    assert_eq!(
        code(&chart.apply_theme("gallery", "[nope").unwrap_err()),
        "subplot_scope"
    );
    assert_eq!(
        code(&chart.set_subplot_background(0, "{\"red\":1}").unwrap_err()),
        "subplot_background"
    );
    assert_eq!(
        code(&chart.set_background("blue").unwrap_err()),
        "background"
    );
    // The inset's theme, as an object: its own kind's member.
    let inset = chart
        .subplot_theme(0, "cool-slate")
        .expect("the scatter member");
    let json = js_sys::JSON::stringify(&inset)
        .unwrap()
        .as_string()
        .unwrap();
    let inset: renderer::theme::Theme = serde_json::from_str(&json).expect("a theme");
    assert_eq!(inset, ThemeFamily::CoolSlate.theme(ChartKind::Scatter));

    // Edit and apply (#295): the theme a name resolves to comes out as an
    // object, reads back equal, and applies to the charts of its kind.
    let object = chart.theme("midnight").expect("the line member");
    let json = js_sys::JSON::stringify(&object)
        .unwrap()
        .as_string()
        .unwrap();
    let back: renderer::theme::Theme = serde_json::from_str(&json).expect("a theme");
    let line = ThemeFamily::Midnight.theme(ChartKind::Line);
    assert_eq!(back, line);
    js_sys::Reflect::set(
        &object,
        &JsValue::from_str("kind"),
        &JsValue::from_str("scatter"),
    )
    .unwrap();
    let before = chart_config(&chart);
    chart
        .apply_custom_theme(object.clone())
        .expect("the scatter inset takes it");
    let themed = chart_config(&chart);
    assert_eq!(themed.background, before.background, "the line base stays");
    assert_eq!(themed.subplots[0].config.background, line.chrome.background);
    assert_eq!(
        code(
            &chart
                .apply_custom_theme_to(object.clone(), "all")
                .unwrap_err()
        ),
        "theme_wrong_kind"
    );
    chart
        .apply_custom_theme_to(object, "[0]")
        .expect("the inset alone");
    assert_eq!(
        code(
            &chart
                .apply_custom_theme(JsValue::from_str("plain"))
                .unwrap_err()
        ),
        "theme_invalid"
    );
    // A custom theme in a draw style that cannot draw the line series
    // (Constellation draws lines only with their markers), built from a
    // partial object: the missing fields take the documented look's.
    let night = js_sys::JSON::parse(
        r#"{"kind": "line", "chrome": {"draw_style": {"mode": "constellation"}}}"#,
    )
    .expect("a JSON object");
    let partial: renderer::theme::Theme = serde_json::from_str(
        &js_sys::JSON::stringify(&night)
            .unwrap()
            .as_string()
            .unwrap(),
    )
    .expect("a partial theme parses");
    assert_eq!(partial.series, renderer::theme::Theme::default().series);
    let before = chart_config(&chart);
    let error = chart.apply_custom_theme_to(night, "none").unwrap_err();
    assert_eq!(code(&error), "theme_draw_style");
    assert_eq!(
        field(&error, "series_id").as_string().as_deref(),
        Some("line")
    );
    assert_eq!(chart_config(&chart), before);
}
