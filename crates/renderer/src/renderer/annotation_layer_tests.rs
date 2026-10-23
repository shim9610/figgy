//! The Cartesian annotation layer: its own raster revision, the compositing
//! order around the data and the chrome, and cache parity with fresh renders.
use super::super::*;
use super::{declaration, read_draw_target};
use crate::annotation::{
    Anchor, AnchorKind, Annotation, AnnotationAxis, AnnotationKind, AnnotationSpace, BoxStyle,
};
use crate::config::LegendCorner;
use crate::layout::{ChartArea, NudgeResult, RectF};
use crate::text::RichText;
use crate::{AnnotationSkip, LayerRef, RenderWarning, SelectionGeometry};

const PANEL: Rect = Rect {
    x: 0,
    y: 0,
    width: 320,
    height: 240,
};

fn text(s: &str) -> RichText {
    RichText::plain(s, Color::BLACK, 11.0, "")
}

/// Title, axes, a blue legend box in the top-left corner and a red line at
/// y = 0.5 across the whole x range.
fn base_config() -> Config {
    let mut c = crate::default::default_config();
    c.chart_area = ChartArea(PANEL);
    c.chart_title.text = text("Annotations");
    c.chart_title.top_margin = 20.0;
    c.legend.visible = true;
    c.legend.corner = LegendCorner::TopLeft;
    c.legend.content = text("Curve");
    c.legend.bg_color = Color::new(0.2, 0.3, 0.9, 1.0);
    for a in [&mut c.bottom_x, &mut c.left_y, &mut c.top_x, &mut c.right_y] {
        a.min = 0.0;
        a.max = 2.0;
        a.major_spacing = 0.5;
        a.minor_count = 1;
        a.out_margin = 30.0;
        a.label_style.font_size = 10.0;
    }
    c
}

fn span(from: f64, to: f64) -> Annotation {
    Annotation::new(AnnotationKind::Span {
        axis: AnnotationAxis::X,
        from,
        to,
        fill: Color::new(1.0, 0.0, 0.0, 0.5),
    })
}

fn annotated_config() -> Config {
    let mut c = base_config();
    c.annotations.items = vec![
        span(0.5, 1.0),
        Annotation::new(AnnotationKind::ReferenceLine {
            axis: AnnotationAxis::Y,
            value: 1.5,
            style: Default::default(),
            label: Some(text("limit")),
        }),
        Annotation::new(AnnotationKind::Text {
            anchor: Anchor::Ratio { x: 0.0, y: 0.0 },
            text: text("over the legend"),
            box_style: Some(BoxStyle {
                fill: Color::WHITE,
                border_color: Color::WHITE,
                border_width: 1.0,
                padding: 12.0,
            }),
        }),
        Annotation::new(AnnotationKind::Arrow {
            from: Anchor::Data { x: 1.2, y: 0.2 },
            to: Anchor::Data { x: 1.8, y: 1.0 },
            style: Default::default(),
            text: Some(text("peak")),
        }),
    ];
    c
}

fn fixture(config: &Config) -> (Renderer, ChartId) {
    let (device, queue) = data_render::shared_device().unwrap();
    let mut r = Renderer::try_new_with_sample_count(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
        1,
    )
    .unwrap();
    // A flat line at y = 0.5 so a span's tint can be told from the data.
    r.add_column(
        "x",
        &crate::Column {
            data: vec![0.0f32, 2.0],
            min: 0.0,
            max: 2.0,
        },
    )
    .unwrap();
    r.add_column(
        "a",
        &crate::Column {
            data: vec![0.5f32, 0.5],
            min: 0.5,
            max: 0.5,
        },
    )
    .unwrap();
    let mut series = declaration("s", "x", "a");
    if let DataRenderType::Line { line } = &mut series.render_type {
        line.line_width = 6.0;
    }
    let id = r.register_chart(config.clone(), vec![series]).unwrap();
    (r, id)
}

/// Paint the registered chart through `view` into a white target.
fn paint(r: &mut Renderer, id: ChartId, view: &ChartView) -> Vec<u8> {
    let target = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("annotation layer test target"),
        size: wgpu::Extent3d {
            width: PANEL.width,
            height: PANEL.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let prepared = r
        .prepare_registered(&[RegisteredChartDrawItem { chart_id: id, view }])
        .unwrap();
    let attachment = target.create_view(&Default::default());
    let mut encoder = r.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &attachment,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        r.paint_prepared(&mut pass, (PANEL.width, PANEL.height), &prepared)
            .unwrap();
    }
    r.queue.submit([encoder.finish()]);
    read_draw_target(r, &target)
}

/// A brand-new renderer and view for `config`, painted once.
fn fresh(config: &Config) -> Vec<u8> {
    let (mut r, id) = fixture(config);
    let view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    paint(&mut r, id, &view)
}

fn pixel(rgba: &[u8], x: f32, y: f32) -> [u8; 4] {
    let i = ((y as u32 * PANEL.width + x as u32) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

fn near(actual: [u8; 4], expected: [u8; 4], tolerance: u8) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| a.abs_diff(e) <= tolerance)
}

// Editing an annotation bumps only the annotation revision; refreshing the
// annotation layer rewrites neither the grid nor the decoration texture, and
// the cached view plus that refresh paints the same bytes as a renderer that
// never saw the earlier state. Toggling `visible` skips the composite without
// touching any texture, and paints exactly what a chart without annotations
// paints.
#[test]
fn annotation_edits_refresh_only_their_layer_and_match_fresh_renders() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let config = annotated_config();
    let (mut r, id) = fixture(&config);
    let mut view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    assert!(view.render_warnings().is_empty());
    assert_eq!(view.annotation_map().entries().count(), 4);
    let first = paint(&mut r, id, &view);
    assert_eq!(
        first,
        fresh(&config),
        "initial paint differs from a fresh render"
    );
    let stamps = view.layer_stamps();
    assert_eq!(
        stamps,
        ChartViewLayerStamps {
            grid: 1,
            decoration: 1,
            annotations: 1
        }
    );

    // 1) Item edits: annotation revision only, annotation textures only.
    let mut edited = config.clone();
    edited.annotations.items[0] = span(1.2, 1.6);
    match &mut edited.annotations.items[2].kind {
        AnnotationKind::Text { anchor, .. } => *anchor = Anchor::Ratio { x: 0.4, y: 0.6 },
        _ => unreachable!(),
    }
    let before = r.chart_render_stamp(id).unwrap();
    r.set_chart_config(id, edited.clone()).unwrap();
    let after = r.chart_render_stamp(id).unwrap();
    assert!(after.needs_draw_since(Some(&before)));
    assert!(!after.needs_raster_since(Some(&before)));
    assert!(after.needs_annotation_raster_since(Some(&before)));
    r.refresh_annotations(&mut view, &Chart::new(edited.clone()), PANEL, None)
        .unwrap();
    assert_eq!(
        view.layer_stamps(),
        ChartViewLayerStamps {
            grid: 1,
            decoration: 1,
            annotations: 2
        },
        "an annotation refresh must not rewrite the grid or decoration texture"
    );
    let cached = paint(&mut r, id, &view);
    assert_ne!(cached, first);
    assert_eq!(
        cached,
        fresh(&edited),
        "cached layers plus an annotation refresh differ from a fresh render"
    );

    // 2) `visible = false`: a draw, no raster of any kind, and the bytes of
    //    a chart that has no annotations at all.
    let mut hidden = edited.clone();
    hidden.annotations.visible = false;
    let before = after;
    r.set_chart_config(id, hidden.clone()).unwrap();
    let after = r.chart_render_stamp(id).unwrap();
    assert!(after.needs_draw_since(Some(&before)));
    assert!(!after.needs_raster_since(Some(&before)));
    assert!(!after.needs_annotation_raster_since(Some(&before)));
    let hidden_pixels = paint(&mut r, id, &view);
    assert_eq!(
        view.layer_stamps().annotations,
        2,
        "a visible toggle re-rasterized"
    );
    assert_eq!(hidden_pixels, fresh(&hidden));
    let mut bare = edited.clone();
    bare.annotations.items.clear();
    assert_eq!(
        hidden_pixels,
        fresh(&bare),
        "a hidden layer must composite nothing"
    );
    assert_ne!(hidden_pixels, cached);

    // 3) A chrome edit refreshes every layer, annotations included, so a
    //    data-anchored item follows the moved axes.
    let mut chrome = edited.clone();
    chrome.left_y.max = 4.0;
    let before = after;
    r.set_chart_config(id, chrome.clone()).unwrap();
    let after = r.chart_render_stamp(id).unwrap();
    assert!(after.needs_raster_since(Some(&before)));
    assert!(!after.needs_annotation_raster_since(Some(&before)));
    r.refresh_axis(&mut view, &Chart::new(chrome.clone()), PANEL)
        .unwrap();
    assert_eq!(
        view.layer_stamps(),
        ChartViewLayerStamps {
            grid: 2,
            decoration: 2,
            annotations: 3
        }
    );
    assert_eq!(paint(&mut r, id, &view), fresh(&chrome));

    // 4) Identical replacement: no revision moves at all.
    let before = after;
    r.set_chart_config(id, chrome).unwrap();
    assert_eq!(r.chart_render_stamp(id).unwrap(), before);
}

// The fixed order grid -> underlay -> data -> decoration -> overlay: a
// translucent span never tints the line drawn over it, and a text box covers
// the legend it overlaps.
#[test]
fn spans_sit_below_the_data_and_text_boxes_above_the_legend() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let plain = base_config();
    let config = annotated_config();
    let (mut r, id) = fixture(&config);
    let view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    let with = paint(&mut r, id, &view);
    let without = fresh(&plain);
    let space = crate::annotation::pixel_space_for_config(&config);
    let x = space.value_to_px(AnnotationAxis::X, 0.75).unwrap();
    let line_y = space.value_to_px(AnnotationAxis::Y, 0.5).unwrap();
    // Inside the span, away from the line: the span's tint over white.
    let tinted = pixel(&with, x, line_y + 20.0);
    assert!(near(tinted, [255, 128, 128, 255], 3), "{tinted:?}");
    assert_eq!(pixel(&without, x, line_y + 20.0), [255, 255, 255, 255]);
    // On the line inside the span: the line's own colour, untinted, exactly
    // as it is drawn without the span.
    let on_line = pixel(&with, x, line_y);
    assert_eq!(on_line, pixel(&without, x, line_y), "span tinted the data");
    assert!(near(on_line, [255, 0, 0, 255], 3), "{on_line:?}");
    // The text box's padding covers the legend corner it overlaps: probe
    // the legend box's bottom-left, inside the box's padding and below the
    // box's own glyphs.
    let map = view.annotation_map();
    let text_id = map.find(2).unwrap();
    let rect = map.bounds(text_id).unwrap();
    let legend = crate::layout::legend_rect(
        &config.data_area().unwrap(),
        config.legend.corner,
        config.legend.padding,
        (config.legend.offset_x, config.legend.offset_y),
        crate::text_render::measure_rich_text(
            &config.legend.content,
            crate::text_render::FontPolicy::Standard,
        ),
    );
    let probe = (legend.x + 2.0, legend.y + legend.height - 2.0);
    let glyphs = crate::text_render::measure_rich_text(
        &text("over the legend"),
        crate::text_render::FontPolicy::Standard,
    );
    assert!(
        rect.contains(probe.0, probe.1) && probe.1 > rect.y + 12.0 + glyphs.height(),
        "probe {probe:?} must sit in the box padding of {rect:?}"
    );
    assert!(
        near(pixel(&without, probe.0, probe.1), [51, 77, 230, 255], 3),
        "legend not where expected: {:?}",
        pixel(&without, probe.0, probe.1)
    );
    assert_eq!(
        pixel(&with, probe.0, probe.1),
        [255, 255, 255, 255],
        "text box did not cover the legend"
    );
}

// A drag through the view's map writes anchors back through the same space
// the raster used: the refreshed map reports the moved bounds, a rejected
// move leaves the config untouched, and the result survives JSON.
#[test]
fn view_map_drags_write_anchors_that_round_trip() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut config = annotated_config();
    let (mut r, id) = fixture(&config);
    let mut view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    let map = view.annotation_map();
    let space = *map.space();
    let hit = map.find(2).unwrap();
    let before = map.bounds(hit).unwrap();
    let center = [
        before.x + before.width * 0.5,
        before.y + before.height * 0.5,
    ];
    assert_eq!(map.hit_test(center), Some(hit));
    assert_eq!(
        map.drag_by(&mut config.annotations, hit, 9.0, 7.0),
        Some(NudgeResult::Moved)
    );
    let untouched = config.clone();
    assert_eq!(
        map.drag_by(&mut config.annotations, hit, 1e6, 0.0),
        Some(NudgeResult::Rejected(
            crate::layout::NudgeReject::OutOfBounds
        ))
    );
    assert_eq!(config, untouched);
    // The arrow's `to` handle moves only that endpoint; `from` stays.
    let arrow = map.find(3).unwrap();
    let selection = map.selection_box(arrow).unwrap();
    assert_eq!(selection.handles().len(), 2);
    let to_px = space
        .anchor_to_px(&Anchor::Data { x: 1.8, y: 1.0 })
        .unwrap();
    let handle = map
        .hit_resize_handle(arrow, to_px)
        .expect("endpoint handle");
    assert_eq!(
        map.resize_by(&mut config.annotations, arrow, handle, -10.0, 5.0),
        Some(NudgeResult::Moved)
    );
    match &config.annotations.items[3].kind {
        AnnotationKind::Arrow { from, to, .. } => {
            assert_eq!(*from, Anchor::Data { x: 1.2, y: 0.2 });
            let moved = space.anchor_to_px(to).unwrap();
            assert!((moved[0] - to_px[0] + 10.0).abs() < 1e-2);
            assert!((moved[1] - to_px[1] - 5.0).abs() < 1e-2);
        }
        _ => unreachable!(),
    }
    r.set_chart_config(id, config.clone()).unwrap();
    r.refresh_annotations(&mut view, &Chart::new(config.clone()), PANEL, Some(2))
        .unwrap();
    let after = view
        .annotation_map()
        .bounds(view.annotation_map().find(2).unwrap())
        .unwrap();
    assert!((after.x - before.x - 9.0).abs() < 0.01 && (after.y - before.y - 7.0).abs() < 0.01);
    assert_eq!(view.annotation_selected(), Some(2));
    // The selection box lives on the overlay: pixels change, no other layer.
    let selected = paint(&mut r, id, &view);
    r.refresh_annotations(&mut view, &Chart::new(config.clone()), PANEL, None)
        .unwrap();
    assert_ne!(selected, paint(&mut r, id, &view));
    assert_eq!(view.layer_stamps().decoration, 1);
    // JSON keeps the dragged anchors to the parser's precision (serde_json
    // without `float_roundtrip` may land one ULP off), which is pixel-exact.
    #[cfg(feature = "serde")]
    {
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(
            restored.annotations.items.len(),
            config.annotations.items.len()
        );
        let placed = |c: &Config| -> Vec<Option<[f32; 2]>> {
            c.annotations
                .items
                .iter()
                .filter_map(|i| match &i.kind {
                    AnnotationKind::Text { anchor, .. } => Some(space.anchor_to_px(anchor)),
                    AnnotationKind::Arrow { from, to, .. } => Some(space.anchor_to_px(from))
                        .into_iter()
                        .chain([space.anchor_to_px(to)])
                        .next(),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(placed(&restored), placed(&config));
        assert_eq!(fresh(&restored), fresh(&config));
    }
    // Converting the anchor kind keeps the screen position.
    let ratio = Anchor::Data { x: 1.2, y: 0.2 }
        .converted(&space, AnchorKind::Ratio)
        .unwrap();
    assert_eq!(
        space
            .anchor_to_px(&ratio)
            .map(|p| [p[0].round(), p[1].round()]),
        space
            .anchor_to_px(&Anchor::Data { x: 1.2, y: 0.2 })
            .map(|p| [p[0].round(), p[1].round()])
    );
}

// Export scales the annotation layer with the chart: the 2x export's
// annotation geometry is the 1x geometry doubled, and so are the hit bounds a
// scaled config measures. Unplaceable items are warnings on the view.
#[test]
fn export_scales_annotations_and_reports_unplaceable_items() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut config = annotated_config();
    config
        .annotations
        .items
        .push(Annotation::new(AnnotationKind::ReferenceLine {
            axis: AnnotationAxis::X,
            value: f64::INFINITY,
            style: Default::default(),
            label: None,
        }));
    let (mut r, id) = fixture(&config);
    let view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    assert_eq!(
        view.render_warnings(),
        &[RenderWarning::AnnotationSkipped {
            index: 4,
            reason: AnnotationSkip::NonFinite
        }]
    );
    let series = r.chart_series(id).unwrap().to_vec();
    let one = r
        .export_panel_rgba(&Chart::new(config.clone()), &series, 1.0)
        .unwrap();
    let two = r
        .export_panel_rgba(&Chart::new(config.clone()), &series, 2.0)
        .unwrap();
    assert_eq!((two.width, two.height), (one.width * 2, one.height * 2));
    // The span's left edge, found on a plot row below the line.
    let space = crate::annotation::pixel_space_for_config(&config);
    let row = space.value_to_px(AnnotationAxis::Y, 0.2).unwrap() as u32;
    let edge = |image: &RasterImage, row: u32| {
        (0..image.width)
            .find(|x| {
                let i = ((row * image.width + x) * 4) as usize;
                image.rgba[i + 1] < 200 && image.rgba[i] > 200
            })
            .expect("span edge")
    };
    let e1 = edge(&one, row);
    let e2 = edge(&two, row * 2);
    assert!(e2.abs_diff(e1 * 2) <= 2, "1x edge {e1}, 2x edge {e2}");
    // Hit bounds measured through a scaled config are the 1x bounds doubled.
    let scaled = config.scaled(2.0);
    let scaled_view = r
        .create_chart_view(&Chart::new(scaled.clone()), scaled.chart_area.0)
        .unwrap();
    for index in 0..4 {
        let a = view
            .annotation_map()
            .bounds(view.annotation_map().find(index).unwrap())
            .unwrap();
        let b = scaled_view
            .annotation_map()
            .bounds(scaled_view.annotation_map().find(index).unwrap())
            .unwrap();
        // Geometry doubles. Bounds are the union of the drawn parts without
        // grab slack (the stroke width scales with the config), so the whole
        // bounds double with it.
        let centre = |r: RectF| [r.x + r.width * 0.5, r.y + r.height * 0.5];
        let (ca, cb) = (centre(a), centre(b));
        assert!(
            (cb[0] - ca[0] * 2.0).abs() <= 2.0 && (cb[1] - ca[1] * 2.0).abs() <= 2.0,
            "item {index}: {b:?} is not {a:?} doubled"
        );
        assert!(
            (b.width - a.width * 2.0).abs() <= 2.0 && (b.height - a.height * 2.0).abs() <= 2.0,
            "item {index}: {b:?} is not {a:?} doubled"
        );
    }
}

// Pointer routing by the active layer (#12 step 4).

/// The annotated chart, alone or as a layered document with one overlay
/// axis on the right; the overlay scales no series.
fn routed_documents() -> [Config; 2] {
    let single = annotated_config();
    let mut layered = single.clone();
    let mut overlay = crate::OverlayLayer::default();
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 40.0;
    overlay.y_axis.major_spacing = 10.0;
    overlay.y_axis.out_margin = 30.0;
    overlay.y_axis.label_style.font_size = 10.0;
    layered.overlays.push(overlay);
    [single, layered]
}

fn centre(r: RectF) -> [f32; 2] {
    [r.x + r.width * 0.5, r.y + r.height * 0.5]
}

/// The stable name of a routed hit.
fn routed_name(map: &crate::HitMap, hit: Option<crate::LayerHit>) -> Option<String> {
    match hit? {
        crate::LayerHit::Annotation { index, .. } => {
            Some(crate::DecorationId::Annotation(index).element_id())
        }
        crate::LayerHit::Element(id) => map.get(id).map(|e| e.element_id()),
    }
}

/// A brand-new renderer and view for `config` whose overlay carries the
/// selection box of annotation `selected`, painted once.
fn fresh_selected(config: &Config, selected: Option<usize>) -> Vec<u8> {
    let (mut r, id) = fixture(config);
    let mut view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    r.refresh_annotations(&mut view, &Chart::new(config.clone()), PANEL, selected)
        .unwrap();
    paint(&mut r, id, &view)
}

// While `Annotations` is active the drawn items sit on top of the shared
// elements; on `Base` and on an overlay they are not registered, so the same
// point reaches the element below (the legend under the text box). The base
// Y axes answer only on `Base`, overlay elements only on their overlay, and
// a hidden layer lets every click through. Routing is the same at 1x and 2x:
// the scaled view's items sit at the doubled points.
#[test]
fn active_layer_routes_pointer_hits_to_annotations_and_lets_others_pass() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for document in routed_documents() {
        let layered = !document.overlays.is_empty();
        let (r, _) = fixture(&document);
        let base_view = r
            .create_chart_view(&Chart::new(document.clone()), PANEL)
            .unwrap();
        let items: Vec<(usize, RectF)> = base_view
            .annotation_map()
            .entries()
            .map(|(id, e)| (e.index, base_view.annotation_map().bounds(id).unwrap()))
            .collect();
        assert_eq!(items.len(), 4);
        let axis_left = {
            let map = crate::HitMap::standard_chart();
            let id = map.find("axis_left").unwrap();
            map.get(id)
                .unwrap()
                .bounds(
                    &document,
                    &crate::CpuTextMeasure::for_style(&document.draw_style),
                )
                .unwrap()
        };
        let mut layers = vec![LayerRef::Base, LayerRef::Annotations];
        if layered {
            layers.push(LayerRef::Overlay(0));
        }
        for scale in [1.0f32, 2.0] {
            let mut config = document.scaled(scale);
            let view = r
                .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
                .unwrap();
            let at = |p: [f32; 2]| [p[0] * scale, p[1] * scale];
            for layer in &layers {
                config.set_active_layer(*layer);
                let map = crate::HitMap::for_active_layer(&config);
                // The text box covers the legend: the legend's centre is
                // under both at every scale.
                let legend = centre(
                    map.get(map.find("legend").unwrap())
                        .unwrap()
                        .bounds(
                            &config,
                            &crate::CpuTextMeasure::for_style(&config.draw_style),
                        )
                        .unwrap(),
                );
                let route =
                    |p: [f32; 2]| routed_name(&map, view.hit_test_active_layer(&config, &map, p));
                // Registration sets: annotation items never enter the
                // element map, the base Y axes are registered on `Base`
                // only and the overlay's axis on its own layer only.
                for index in 0..4 {
                    assert!(map.find(&format!("annotation_{index}")).is_none());
                }
                assert_eq!(map.find("axis_left").is_some(), *layer == LayerRef::Base);
                assert_eq!(
                    map.find("overlay_0:axis_right").is_some(),
                    *layer == LayerRef::Overlay(0)
                );
                assert!(map.find("legend").is_some() && map.find("data_area").is_some());
                let left = route(at(centre(axis_left)));
                assert_eq!(
                    left.as_deref() == Some("axis_left"),
                    *layer == LayerRef::Base,
                    "scale {scale} {layer:?}: {left:?} on the left axis"
                );
                if *layer == LayerRef::Annotations {
                    for (index, bounds) in &items {
                        let hit = view.hit_test_active_layer(&config, &map, at(centre(*bounds)));
                        let Some(crate::LayerHit::Annotation { index: picked, id }) = hit else {
                            panic!("scale {scale}: item {index} centre routed to {hit:?}");
                        };
                        // Items may overlap; the topmost wins and must contain the point.
                        assert!(
                            picked == *index
                                || view
                                    .annotation_map()
                                    .bounds(id)
                                    .unwrap()
                                    .contains(at(centre(*bounds))[0], at(centre(*bounds))[1]),
                        );
                    }
                    assert_eq!(
                        route(legend).as_deref(),
                        Some("annotation_2"),
                        "scale {scale}: the text box must sit above the legend"
                    );
                    // A hidden layer is out of the route.
                    let mut hidden = config.clone();
                    hidden.annotations.visible = false;
                    let hit = view.hit_test_active_layer(&hidden, &map, legend);
                    assert!(
                        !matches!(hit, Some(crate::LayerHit::Annotation { .. })),
                        "{hit:?}"
                    );
                } else {
                    for (_, bounds) in &items {
                        let hit = view.hit_test_active_layer(&config, &map, at(centre(*bounds)));
                        assert!(
                            !matches!(hit, Some(crate::LayerHit::Annotation { .. })),
                            "scale {scale} {layer:?}: an annotation answered"
                        );
                    }
                    assert_eq!(
                        route(legend).as_deref(),
                        Some("legend"),
                        "scale {scale} {layer:?}: the click must pass through to the legend"
                    );
                }
            }
        }
    }
}

// Switching to the annotation layer, selecting and dragging an item through
// the route and switching back touch neither the data nor any grid or
// decoration texture: only the annotation layer re-rasterizes, and the
// cached view paints the bytes of a renderer that only ever saw the final
// state. Dragged anchors keep their kind and survive a JSON round trip.
#[test]
fn layer_switches_and_routed_drags_keep_the_other_caches_and_paint_fresh_bytes() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for config in routed_documents() {
        let (mut r, id) = fixture(&config);
        let mut view = r
            .create_chart_view(&Chart::new(config.clone()), PANEL)
            .unwrap();
        let first = paint(&mut r, id, &view);
        assert_eq!(first, fresh(&config));
        let stamps = view.layer_stamps();
        let generations = view.layer_raster_generations();
        let chrome_unchanged = |view: &ChartView| {
            let now = view.layer_stamps();
            assert_eq!((now.grid, now.decoration), (stamps.grid, stamps.decoration));
            assert_eq!(view.layer_raster_generations(), generations);
        };

        // 1) Entering the layer: a draw, no raster of any kind.
        let before = r.chart_render_stamp(id).unwrap();
        assert_eq!(
            r.set_active_layer(id, LayerRef::Annotations).unwrap(),
            LayerRef::Annotations
        );
        let after = r.chart_render_stamp(id).unwrap();
        assert!(after.needs_draw_since(Some(&before)));
        assert!(!after.needs_raster_since(Some(&before)));
        assert!(!after.needs_annotation_raster_since(Some(&before)));
        let active = r.chart_config(id).unwrap().clone();
        assert_eq!(paint(&mut r, id, &view), first);
        assert_eq!(first, fresh(&active));
        chrome_unchanged(&view);

        // 2) A routed press selects the text box: an annotation refresh only.
        let map = crate::HitMap::for_active_layer(&active);
        let text_box = view
            .annotation_map()
            .bounds(view.annotation_map().find(2).unwrap())
            .unwrap();
        let Some(crate::LayerHit::Annotation { index: 2, id: hit }) =
            view.hit_test_active_layer(&active, &map, centre(text_box))
        else {
            panic!("the text box did not answer");
        };
        r.refresh_annotations(&mut view, &Chart::new(active.clone()), PANEL, Some(2))
            .unwrap();
        chrome_unchanged(&view);
        assert_eq!(view.layer_stamps().annotations, stamps.annotations + 1);
        let selected = paint(&mut r, id, &view);
        assert_ne!(selected, first);
        assert_eq!(selected, fresh_selected(&active, Some(2)));

        // 3) Drag the routed item and the arrow's `to` handle.
        let annotations = view.annotation_map();
        let space = *annotations.space();
        let mut edited = active.clone();
        assert_eq!(
            annotations.drag_by(&mut edited.annotations, hit, 9.0, 7.0),
            Some(NudgeResult::Moved)
        );
        let to_px = space
            .anchor_to_px(&Anchor::Data { x: 1.8, y: 1.0 })
            .unwrap();
        let Some(crate::LayerHit::Annotation {
            index: 3,
            id: arrow,
        }) = view.hit_test_active_layer(&active, &map, to_px)
        else {
            panic!("the arrow tip did not answer");
        };
        let handle = annotations
            .hit_resize_handle(arrow, to_px)
            .expect("endpoint handle");
        assert_eq!(
            annotations.resize_by(&mut edited.annotations, arrow, handle, -10.0, 5.0),
            Some(NudgeResult::Moved)
        );
        match (
            &edited.annotations.items[2].kind,
            &edited.annotations.items[3].kind,
        ) {
            (AnnotationKind::Text { anchor, .. }, AnnotationKind::Arrow { from, to, .. }) => {
                assert_eq!(anchor.kind(), AnchorKind::Ratio);
                // The bounds are the drawn box (no grab slack); its corner
                // is the anchor.
                let drawn = text_box;
                let moved = space.anchor_to_px(anchor).unwrap();
                assert!((moved[0] - drawn.x - 9.0).abs() < 1e-2);
                assert!((moved[1] - drawn.y - 7.0).abs() < 1e-2);
                assert_eq!(*from, Anchor::Data { x: 1.2, y: 0.2 });
                assert_eq!(to.kind(), AnchorKind::Data);
                let tip = space.anchor_to_px(to).unwrap();
                assert!((tip[0] - to_px[0] + 10.0).abs() < 1e-2);
                assert!((tip[1] - to_px[1] - 5.0).abs() < 1e-2);
            }
            _ => unreachable!(),
        }
        let before = r.chart_render_stamp(id).unwrap();
        r.set_chart_config(id, edited.clone()).unwrap();
        let after = r.chart_render_stamp(id).unwrap();
        assert!(!after.needs_raster_since(Some(&before)));
        assert!(after.needs_annotation_raster_since(Some(&before)));
        r.refresh_annotations(&mut view, &Chart::new(edited.clone()), PANEL, Some(2))
            .unwrap();
        chrome_unchanged(&view);
        assert_eq!(paint(&mut r, id, &view), fresh_selected(&edited, Some(2)));
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string(&edited).unwrap();
            assert!(json.contains(r#""active_layer":"Annotations""#), "{json}");
            let restored: Config = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.active_layer, LayerRef::Annotations);
            for (a, b) in restored
                .annotations
                .items
                .iter()
                .zip(&edited.annotations.items)
            {
                assert_eq!(
                    std::mem::discriminant(&a.kind),
                    std::mem::discriminant(&b.kind)
                );
            }
            assert_eq!(
                fresh_selected(&restored, Some(2)),
                fresh_selected(&edited, Some(2))
            );
        }

        // 4) Leaving the layer: the host drops the annotation selection with
        //    it; still no grid or decoration raster.
        let before = r.chart_render_stamp(id).unwrap();
        assert_eq!(
            r.set_active_layer(id, LayerRef::Base).unwrap(),
            LayerRef::Base
        );
        let after = r.chart_render_stamp(id).unwrap();
        assert!(!after.needs_raster_since(Some(&before)));
        assert!(!after.needs_annotation_raster_since(Some(&before)));
        let last = r.chart_config(id).unwrap().clone();
        r.refresh_annotations(&mut view, &Chart::new(last.clone()), PANEL, None)
            .unwrap();
        chrome_unchanged(&view);
        assert_eq!(paint(&mut r, id, &view), fresh(&last));
        let map = crate::HitMap::for_active_layer(&last);
        let moved = view
            .annotation_map()
            .bounds(view.annotation_map().find(2).unwrap())
            .unwrap();
        assert!(!matches!(
            view.hit_test_active_layer(&last, &map, centre(moved)),
            Some(crate::LayerHit::Annotation { .. })
        ));
    }
}
