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
use crate::{AnnotationSkip, RenderWarning};

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
    assert_eq!(selection.handles.len(), 2);
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
        // Geometry doubles; the reference line's grab strip is a pointer
        // affordance of fixed thickness, so only its centre and length do.
        let centre = |r: RectF| [r.x + r.width * 0.5, r.y + r.height * 0.5];
        let (ca, cb) = (centre(a), centre(b));
        assert!(
            (cb[0] - ca[0] * 2.0).abs() <= 2.0 && (cb[1] - ca[1] * 2.0).abs() <= 2.0,
            "item {index}: {b:?} is not {a:?} doubled"
        );
        if index != 1 {
            assert!(
                (b.width - a.width * 2.0).abs() <= 2.0 && (b.height - a.height * 2.0).abs() <= 2.0,
                "item {index}: {b:?} is not {a:?} doubled"
            );
        } else {
            // The strip adds a fixed 3 px on each end at every scale.
            assert!(
                ((b.width - 6.0) - (a.width - 6.0) * 2.0).abs() <= 2.0,
                "item {index}: {b:?} vs {a:?}"
            );
        }
    }
}
