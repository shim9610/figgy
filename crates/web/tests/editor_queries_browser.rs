//! The editor queries of the raw `FiggyChart` (#312) on a real GPU, at 1x
//! and 2x: the preview of an annotation that is not accepted yet and its
//! anchor conversion, the displayed layout of a document and its panels,
//! one legend row for a series, and the whole frame's render warnings.
//! Every query is read-only: the document, the selection and the raster
//! are checked to stay as they were.
#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use renderer::config::AxisScale;
use renderer::layout::{ChartArea, Rect};
use renderer::{Anchor, Annotation, AnnotationAxis, AnnotationKind, Color, LayerRef};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// The document's size; the canvas is this times the display scale.
const DOC: (u32, u32) = (480, 320);

fn canvas() -> web_sys::HtmlCanvasElement {
    let canvas: web_sys::HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(DOC.0);
    canvas.set_height(DOC.1);
    canvas
}

fn config_of(chart: &FiggyChart) -> renderer::Config {
    serde_json::from_str(&chart.get_config().unwrap()).unwrap()
}

fn set_config(chart: &mut FiggyChart, config: &renderer::Config) {
    chart
        .set_config(&serde_json::to_string(config).unwrap())
        .unwrap();
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

fn code(error: &JsValue) -> Option<String> {
    js_sys::Reflect::get(error, &JsValue::from_str("code"))
        .ok()
        .and_then(|code| code.as_string())
}

fn rect(value: &serde_json::Value) -> [f64; 4] {
    let n = |i: usize| value[i].as_f64().unwrap();
    [n(0), n(1), n(2), n(3)]
}

fn close(a: [f64; 4], b: [f64; 4], tolerance: f64) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= tolerance)
}

/// A chart whose document is `DOC` with axes 0..10 and 0..1, shown at
/// `scale` canvas pixels per document pixel.
async fn chart_at(scale: u32) -> FiggyChart {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let mut config = config_of(&chart);
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: DOC.0,
        height: DOC.1,
    });
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 10.0;
    config.left_y.min = 0.0;
    config.left_y.max = 1.0;
    set_config(&mut chart, &config);
    chart.resize(DOC.0 * scale, DOC.1 * scale).unwrap();
    chart.frame().unwrap();
    chart
}

fn note(anchor: Anchor) -> Annotation {
    Annotation::new(AnnotationKind::Text {
        anchor,
        text: renderer::text::RichText::plain("peak", Color::BLACK, 12.0, ""),
        box_style: Some(Default::default()),
        align: Default::default(),
    })
}

/// The preview of an item is where the layer draws it once accepted,
/// in canvas and document pixels; previewing changes nothing (#312).
#[wasm_bindgen_test(async)]
async fn annotation_preview_is_where_the_layer_draws_the_item() {
    for scale in [1u32, 2] {
        let mut chart = chart_at(scale).await;
        let item = serde_json::to_string(&note(Anchor::Data { x: 4.0, y: 0.5 })).unwrap();
        let before = chart.get_config().unwrap();
        let listed = chart.annotations().unwrap();
        let preview = json(&chart.annotation_preview(&item).unwrap());
        assert_eq!(chart.get_config().unwrap(), before, "{scale}");
        assert_eq!(chart.annotations().unwrap(), listed);
        assert_eq!(chart.selected_annotation(), None);
        assert_eq!(preview["kind"], "Text");
        assert_eq!(preview["drawn"], true);
        assert_eq!(preview["visible"], true);
        assert_eq!(preview["canvas"]["shape"], "text");
        let canvas = rect(&preview["canvas"]["bounds"]);
        let document = rect(&preview["document"]["bounds"]);
        assert!(
            close(
                canvas,
                document.map(|v| v * scale as f64),
                2.0 * scale as f64
            ),
            "{scale}: {canvas:?} vs {document:?}"
        );

        // Accepted, the drawn item's selection box is the preview's bounds
        // padded by the selection padding.
        let index = chart.add_annotation(&item).unwrap();
        chart.frame().unwrap();
        let drawn = json(&chart.annotations().unwrap());
        let pad = renderer::select::SELECTION_PADDING as f64;
        let [x, y, w, h] = canvas;
        assert!(
            close(
                rect(&drawn[index as usize]["rect"]),
                [x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad],
                1e-3
            ),
            "{scale}: {drawn} vs {canvas:?}"
        );

        // A shape off the plot is not drawn and says why.
        let off = Annotation::new(AnnotationKind::ReferenceLine {
            axis: AnnotationAxis::X,
            value: 50.0,
            style: Default::default(),
            label: None,
        });
        let off = json(
            &chart
                .annotation_preview(&serde_json::to_string(&off).unwrap())
                .unwrap(),
        );
        assert_eq!(off["drawn"], false);
        assert_eq!(off["reason"], "OutOfRange");
        // A hidden item is placed as it is drawn once shown.
        let mut hidden = note(Anchor::Data { x: 4.0, y: 0.5 });
        hidden.visible = false;
        let hidden = json(
            &chart
                .annotation_preview(&serde_json::to_string(&hidden).unwrap())
                .unwrap(),
        );
        assert_eq!(hidden["visible"], false);
        assert_eq!(rect(&hidden["canvas"]["bounds"]), canvas);

        // Malformed input is refused with a code.
        let error = chart.annotation_preview("{").unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("annotation_json"));
        let mut bad = note(Anchor::Ratio { x: 0.5, y: 0.5 });
        bad.id = "a\u{7}b".into();
        let error = chart
            .annotation_preview(&serde_json::to_string(&bad).unwrap())
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("annotation_invalid"));
    }
}

/// Converting the anchors keeps the drawn position; an anchor without one
/// is refused (#312).
#[wasm_bindgen_test(async)]
async fn anchor_conversion_keeps_the_drawn_position() {
    for scale in [1u32, 2] {
        let mut chart = chart_at(scale).await;
        let arrow = Annotation::new(AnnotationKind::Arrow {
            from: Anchor::Data { x: 2.0, y: 0.2 },
            to: Anchor::Ratio { x: 0.8, y: 0.1 },
            style: Default::default(),
            text: None,
        });
        let item = serde_json::to_string(&arrow).unwrap();
        let before = chart.get_config().unwrap();
        let shape = |chart: &FiggyChart, item: &str| {
            json(&chart.annotation_preview(item).unwrap())["canvas"]["points"].clone()
        };
        for kind in ["Ratio", "Data"] {
            let converted = chart.convert_annotation_anchor(&item, kind).unwrap();
            let parsed: Annotation = serde_json::from_str(&converted).unwrap();
            let AnnotationKind::Arrow { from, to, .. } = parsed.kind else {
                panic!("{converted}")
            };
            for anchor in [from, to] {
                assert_eq!(format!("{:?}", anchor.kind()), kind);
            }
            let (a, b) = (shape(&chart, &item), shape(&chart, &converted));
            for i in 0..2 {
                for axis in 0..2 {
                    let (a, b) = (a[i][axis].as_f64().unwrap(), b[i][axis].as_f64().unwrap());
                    assert!((a - b).abs() < 1e-2, "{scale} {kind}: {a} vs {b}");
                }
            }
        }
        assert_eq!(chart.get_config().unwrap(), before);
        // A span has no anchor and comes back unchanged.
        let span = Annotation::new(AnnotationKind::Span {
            axis: AnnotationAxis::X,
            from: 1.0,
            to: 2.0,
            fill: Color::BLACK,
        });
        let span_json = serde_json::to_string(&span).unwrap();
        let back: Annotation = serde_json::from_str(
            &chart
                .convert_annotation_anchor(&span_json, "Ratio")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(back, span);
        let error = chart.convert_annotation_anchor(&item, "Pixel").unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("anchor_kind"));
        // y = -1 has no position on a logarithmic axis.
        let mut config = config_of(&chart);
        config.left_y.scale = AxisScale::Logarithmic;
        config.left_y.min = 1.0;
        config.left_y.max = 100.0;
        set_config(&mut chart, &config);
        let below = serde_json::to_string(&note(Anchor::Data { x: 1.0, y: -1.0 })).unwrap();
        let error = chart
            .convert_annotation_anchor(&below, "Ratio")
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("anchor_unplaceable"));
        assert_eq!(
            json(&chart.annotation_preview(&below).unwrap())["reason"],
            "OutOfRange"
        );
    }
}

/// A parent with one overlay and a subplot to its right.
fn layered(chart: &FiggyChart) -> renderer::Config {
    let mut config = config_of(chart);
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: DOC.1,
    });
    config
        .overlays
        .push(serde_json::from_str::<renderer::OverlayLayer>("{}").unwrap());
    let mut subplot: renderer::Subplot = serde_json::from_str("{}").unwrap();
    subplot.config = renderer::default::default_config();
    subplot.config.chart_area = ChartArea(Rect {
        x: 320,
        y: 0,
        width: 160,
        height: DOC.1,
    });
    config.subplots.push(subplot);
    config
}

/// The layout lists every panel in both spaces; `panel_at` and
/// `data_area_rect` agree with it; a candidate is inspected without
/// being installed (#312).
#[wasm_bindgen_test(async)]
async fn document_layout_panels_and_data_area() {
    for scale in [1u32, 2] {
        let mut chart = chart_at(scale).await;
        let config = layered(&chart);
        set_config(&mut chart, &config);
        chart.frame().unwrap();
        let before = chart.get_config().unwrap();
        let layout = json(&chart.inspect_document_layout(None).unwrap());
        assert!((layout["scale"].as_f64().unwrap() - scale as f64).abs() < 1e-6);
        assert_eq!(
            rect(&layout["document_area"]["document"]),
            [0.0, 0.0, DOC.0 as f64, DOC.1 as f64]
        );
        assert_eq!(
            rect(&layout["document_area"]["canvas"]),
            [0.0, 0.0, (DOC.0 * scale) as f64, (DOC.1 * scale) as f64]
        );
        let panels = layout["panels"].as_array().unwrap();
        let layers: Vec<LayerRef> = panels
            .iter()
            .map(|panel| serde_json::from_value(panel["layer"].clone()).unwrap())
            .collect();
        assert_eq!(
            layers,
            [LayerRef::Base, LayerRef::Overlay(0), LayerRef::Subplot(0)]
        );
        assert_eq!(
            rect(&panels[2]["frame"]["document"]),
            [320.0, 0.0, 160.0, DOC.1 as f64]
        );
        assert_eq!(panels[0]["data_area"], panels[1]["data_area"]);

        // The active layer's exact data area is its panel's.
        let data = json(&chart.data_area_rect().unwrap());
        assert_eq!(data["panel"], 0);
        assert_eq!(data["canvas"], panels[0]["data_area"]["canvas"]);
        let [x, y, w, h] = rect(&data["canvas"]);
        let inside = json(
            &chart
                .panel_at((x + w * 0.5) as f32, (y + h * 0.5) as f32)
                .unwrap(),
        );
        assert_eq!(inside["layer"], "Base");
        assert_eq!(inside["in_data_area"], true);
        let sub = rect(&panels[2]["frame"]["canvas"]);
        let on_subplot = json(
            &chart
                .panel_at(
                    (sub[0] + sub[2] * 0.5) as f32,
                    (sub[1] + sub[3] * 0.5) as f32,
                )
                .unwrap(),
        );
        assert_eq!(on_subplot["layer"], serde_json::json!({ "Subplot": 0 }));
        assert_eq!(on_subplot["panel"], 2);
        assert_eq!(on_subplot["active"], false);
        let document = on_subplot["document"].as_array().unwrap();
        assert!((document[0].as_f64().unwrap() - 400.0).abs() < 1.0);
        assert_eq!(chart.panel_at(-4.0, -4.0).unwrap(), "null");

        // The active overlay is reported over the parent's frame.
        chart.set_active_layer(r#"{"Overlay":0}"#).unwrap();
        let on_overlay = json(
            &chart
                .panel_at((x + w * 0.5) as f32, (y + h * 0.5) as f32)
                .unwrap(),
        );
        assert_eq!(on_overlay["layer"], serde_json::json!({ "Overlay": 0 }));
        assert_eq!(on_overlay["panel"], 1);
        assert_eq!(on_overlay["active"], true);
        chart.set_active_layer(r#"{"Subplot":0}"#).unwrap();
        let data = json(&chart.data_area_rect().unwrap());
        assert_eq!(data["panel"], 2);
        assert_eq!(data["canvas"], panels[2]["data_area"]["canvas"]);
        assert_eq!(chart.panel_at(-4.0, -4.0).unwrap(), "null");
        chart.set_active_layer(r#""Base""#).unwrap();
        assert_eq!(chart.get_config().unwrap(), before);

        // A candidate is laid out on this canvas and not installed.
        let mut candidate = config_of(&chart);
        candidate.subplots[0].config.chart_area = ChartArea(Rect {
            x: 0,
            y: DOC.1,
            width: 320,
            height: 160,
        });
        let candidate_layout = json(
            &chart
                .inspect_document_layout(Some(serde_json::to_string(&candidate).unwrap()))
                .unwrap(),
        );
        assert_eq!(chart.get_config().unwrap(), before);
        assert_eq!(
            rect(&candidate_layout["document_area"]["document"]),
            [0.0, 0.0, 320.0, (DOC.1 + 160) as f64]
        );
        assert_eq!(
            rect(&candidate_layout["panels"][2]["frame"]["document"]),
            [0.0, DOC.1 as f64, 320.0, 160.0]
        );
        let fitted =
            renderer::fit_display_panel((320, DOC.1 + 160), (DOC.0 * scale, DOC.1 * scale)).0;
        assert!((candidate_layout["scale"].as_f64().unwrap() - fitted as f64).abs() < 1e-6);
        let error = chart.inspect_document_layout(Some("{".into())).unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("config_json"));
        let mut infeasible = candidate.clone();
        infeasible.overlays[0].series = vec!["a".into()];
        infeasible.overlays.push(infeasible.overlays[0].clone());
        let error = chart
            .inspect_document_layout(Some(serde_json::to_string(&infeasible).unwrap()))
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("layout"));
        assert_eq!(chart.get_config().unwrap(), before);
    }
}

/// One legend row is the auto legend's row for the series, and appending
/// it keeps a host-written legend's text (#312).
#[wasm_bindgen_test(async)]
async fn legend_entry_is_the_auto_row_for_one_series() {
    for scale in [1u32, 2] {
        let mut chart = chart_at(scale).await;
        chart.register_column_f32("x", &[0.0, 1.0, 2.0]).unwrap();
        chart.register_column_f32("y", &[0.0, 0.5, 0.25]).unwrap();
        chart.add_line_series("a", "x", "y", 2.0, "Alpha").unwrap();
        let auto = config_of(&chart).legend.content.segments;
        let row: Vec<renderer::text::RichSegment> =
            serde_json::from_str(&chart.legend_entry("a").unwrap()).unwrap();
        assert!(auto.ends_with(&row), "{auto:?} vs {row:?}");
        assert_eq!(row[0].series_id.as_deref(), Some("a"));

        // A host-written legend; a second series added without a label.
        let mut config = config_of(&chart);
        config.legend.content.segments = renderer::text::rich_segments_from_text("user text");
        set_config(&mut chart, &config);
        chart.add_line_series("b", "x", "y", 2.0, "").unwrap();
        let mut config = config_of(&chart);
        let before = chart.get_config().unwrap();
        let row: Vec<renderer::text::RichSegment> =
            serde_json::from_str(&chart.legend_entry("b").unwrap()).unwrap();
        assert_eq!(chart.get_config().unwrap(), before);
        assert_eq!(row.last().map(|segment| segment.text), Some('\t'));
        config
            .legend
            .content
            .segments
            .push(renderer::text::RichSegment::plain('\n'));
        config.legend.content.segments.extend(row);
        set_config(&mut chart, &config);
        chart.frame().unwrap();
        let text: String = config_of(&chart)
            .legend
            .content
            .segments
            .iter()
            .map(|segment| segment.text)
            .collect();
        assert!(text.starts_with("user text\n"), "{text:?}");
        let error = chart.legend_entry("missing").unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("series"));
    }
}

/// The frame's warnings cover tick settings on every axis and skipped
/// annotations, structured (#312).
#[wasm_bindgen_test(async)]
async fn render_warnings_report_the_whole_frame() {
    for scale in [1u32, 2] {
        let mut chart = chart_at(scale).await;
        assert_eq!(chart.render_warnings().unwrap(), "[]");
        let mut config = config_of(&chart);
        config.bottom_x.major_spacing = 0.0;
        config.left_y.minor_count = 5000;
        config
            .annotations
            .items
            .push(Annotation::new(AnnotationKind::ReferenceLine {
                axis: AnnotationAxis::Y,
                value: 7.0,
                style: Default::default(),
                label: None,
            }));
        set_config(&mut chart, &config);
        chart.frame().unwrap();
        let warnings = json(&chart.render_warnings().unwrap());
        let warnings = warnings.as_array().unwrap();
        let find = |kind: &str| {
            warnings
                .iter()
                .find(|w| w["kind"] == kind)
                .unwrap_or_else(|| panic!("{kind} in {warnings:?}"))
        };
        let spacing = find("MajorSpacingReplaced");
        assert_eq!(spacing["axis"], "bottom_x");
        assert_eq!(spacing["requested"], 0.0);
        assert_eq!(spacing["reason"], "NotPositive");
        assert!(spacing["message"].as_str().unwrap().contains("bottom_x"));
        let minor = find("MinorCountClamped");
        assert_eq!(minor["axis"], "left_y");
        assert_eq!(minor["requested"], 5000);
        let skipped = find("AnnotationSkipped");
        assert_eq!(skipped["index"], 0);
        assert_eq!(skipped["reason"], "OutOfRange");
        // `annotation_warnings` is the annotation part of the same list.
        let annotations = json(&chart.annotation_warnings().unwrap());
        assert_eq!(annotations.as_array().unwrap().len(), 1);
        // Fixed settings clear them on the next frame.
        let mut config = config_of(&chart);
        config.bottom_x.major_spacing = 2.0;
        config.left_y.minor_count = 1;
        config.annotations.items.clear();
        set_config(&mut chart, &config);
        chart.frame().unwrap();
        assert_eq!(chart.render_warnings().unwrap(), "[]");
    }
}
