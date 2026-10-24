//! The annotation layer on bounded charts: one shared pass around each
//! family's frame, its own cache beside the frame and label raster caches,
//! export parity with fresh renderers, and the common hit-map contract.
#![cfg(not(target_arch = "wasm32"))]
#[path = "support/bounded_decorations.rs"]
mod bounded_decorations;
#[path = "support/gpu.rs"]
mod test_gpu;
use ::renderer::{
    layout::{NudgeResult, RectF},
    *,
};
use bounded_decorations::Decorations;
use std::sync::{Arc, Mutex, OnceLock};
static LOCK: Mutex<()> = Mutex::new(());
fn gpu() -> RendererDevice {
    static GPU: OnceLock<RendererDevice> = OnceLock::new();
    GPU.get_or_init(|| {
        let (d, q) = test_gpu::device();
        RendererDevice::new(Arc::new(d), Arc::new(q))
    })
    .clone()
}
const SIZE: (u32, u32) = (800, 520);

fn text(s: &str) -> text::RichText {
    text::RichText::plain(s, Color::BLACK, 12., "")
}
fn span(axis: AnnotationAxis, from: f64, to: f64) -> Annotation {
    Annotation::new(AnnotationKind::Span {
        axis,
        from,
        to,
        fill: Color::new(1., 0., 0., 0.5),
    })
}
fn boxed_text(anchor: Anchor) -> Annotation {
    Annotation::new(AnnotationKind::Text {
        anchor,
        text: text("note"),
        box_style: Some(BoxStyle {
            fill: Color::WHITE,
            border_color: Color::BLACK,
            border_width: 1.,
            padding: 10.,
        }),
    })
}
fn ink(image: &RasterImage, r: RectF, scale: f32) -> usize {
    let x0 = (r.x * scale).floor().max(0.) as u32;
    let y0 = (r.y * scale).floor().max(0.) as u32;
    let x1 = ((r.x + r.width) * scale).ceil().min(image.width as f32) as u32;
    let y1 = ((r.y + r.height) * scale).ceil().min(image.height as f32) as u32;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| ((y * image.width + x) * 4) as usize))
        .filter(|&i| image.rgba[i..i + 3].iter().any(|v| *v < 200))
        .count()
}

/// A point on the first part the raster places for an item: a stroke's
/// midpoint, a head's centroid, a box's centre. The centre of the item's
/// bounds can be empty, for example beside a label.
fn on_item(map: &AnnotationMap, layer: &AnnotationLayer, id: HitId) -> [f32; 2] {
    use ::renderer::sink::{DrawnPart, Parts};
    let mut first = None;
    map.element(id).unwrap().emit(
        layer,
        &mut Parts(&mut |part| {
            first.get_or_insert(part);
        }),
    );
    match first.expect("the item places a part") {
        DrawnPart::Segment { a, b, .. } => [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5],
        DrawnPart::Triangle(t) => [
            (t[0][0] + t[1][0] + t[2][0]) / 3.0,
            (t[0][1] + t[1][1] + t[2][1]) / 3.0,
        ],
        DrawnPart::Rect(r) | DrawnPart::Text(r) => [r.x + r.width * 0.5, r.y + r.height * 0.5],
    }
}

/// The checks every family shares, written once over the family's
/// renderer, chart and frame types.
macro_rules! family_contract {
    ($name:ident, $renderer:ident, $chart:ident, $build:expr, $edit:expr) => {
        #[test]
        fn $name() {
            let _guard = LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            fn fresh(c: &$chart, scale: f32) -> Vec<u8> {
                let mut r = $renderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
                r.export_rgba(c, SIZE, scale).unwrap().rgba
            }
            let mut r = $renderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
            let mut c: $chart = $build;
            let edit: fn(&mut $chart) = $edit;
            // 0) An empty layer: hiding it reuses the frame and its label
            //    raster and changes no pixel, the cached export equals a
            //    fresh one, and a title edit still rebuilds the frame.
            {
                let mut empty = c.clone();
                empty.annotations.items.clear();
                let f = r.prepare(&empty, SIZE, 1.).unwrap();
                let before = r.export_rgba(&empty, SIZE, 1.).unwrap().rgba;
                empty.annotations.visible = false;
                let g = r.prepare(&empty, SIZE, 1.).unwrap();
                assert!(Arc::ptr_eq(&f, &g), "empty layer: hiding rebuilt the frame");
                assert!(
                    f.shares_annotations_with(&g),
                    "empty layer: hiding redrew the label raster"
                );
                let cached = r.export_rgba(&empty, SIZE, 1.).unwrap().rgba;
                assert!(cached == before, "empty layer: hiding changed pixels");
                assert!(
                    cached == fresh(&empty, 1.),
                    "empty layer: cached export differs from fresh"
                );
                empty.title = "Edited".into();
                assert!(!Arc::ptr_eq(&f, &r.prepare(&empty, SIZE, 1.).unwrap()));
                assert!(r.export_rgba(&empty, SIZE, 1.).unwrap().rgba == fresh(&empty, 1.));
                r.end_frame();
            }
            for scale in [1., 2.] {
                // The annotation frame rides the frame's anchor space; both
                // come from the same cache state as a fresh renderer.
                let f = r.prepare(&c, SIZE, scale).unwrap();
                let a = r.prepare_annotations(&c, SIZE, scale).unwrap();
                assert!(a.render_warnings().is_empty(), "{:?}", a.render_warnings());
                assert_eq!(a.annotations().entries().count(), c.annotations.items.len());
                assert_eq!(a.size(), f.size());
                let before = r.export_rgba(&c, SIZE, scale).unwrap();
                assert!(
                    before.rgba == fresh(&c, scale),
                    "scale {scale}: export differs from fresh"
                );
                // Hit bounds are logical and identical at every output scale;
                // the export ink of each item scales with the image.
                for (id, e) in a.annotations().entries() {
                    let bounds = a.annotations().bounds(id).unwrap();
                    assert!(
                        ink(&before, bounds, scale) > 0,
                        "scale {scale}: item {} drew nothing in {bounds:?}",
                        e.index
                    );
                    // Items may overlap; the topmost one under the centre wins,
                    // and it must at least contain the point.
                    let centre = on_item(a.annotations(), &c.annotations, id);
                    let hit = a.annotations().hit_test(centre).expect("item centre picks");
                    let picked = a.annotations().element(hit).unwrap().index;
                    assert!(
                        picked == e.index
                            || a.annotations()
                                .bounds(hit)
                                .unwrap()
                                .contains(centre[0], centre[1]),
                        "item {} centre picked {picked}",
                        e.index
                    );
                }

                // 1) An annotation edit leaves the frame and its label raster
                //    cached, re-rasterizes the annotation layer only, and the
                //    cached export equals a fresh one.
                let mut edited = c.clone();
                edit(&mut edited);
                let g = r.prepare(&edited, SIZE, scale).unwrap();
                assert!(
                    Arc::ptr_eq(&f, &g),
                    "scale {scale}: annotation edit rebuilt the frame"
                );
                assert!(f.shares_annotations_with(&g));
                let b = r.prepare_annotations(&edited, SIZE, scale).unwrap();
                assert!(
                    !b.shares_rasters_with(&a),
                    "edit must re-rasterize the layer"
                );
                let after = r.export_rgba(&edited, SIZE, scale).unwrap();
                assert!(
                    after.rgba != before.rgba,
                    "scale {scale}: edit changed nothing"
                );
                assert!(
                    after.rgba == fresh(&edited, scale),
                    "scale {scale}: cached export differs"
                );
                assert!(Arc::ptr_eq(&g, &r.prepare(&edited, SIZE, scale).unwrap()));

                // 2) `visible = false`: same frame, same rasters, nothing
                //    composited: the bytes of the chart without any items.
                let mut hidden = edited.clone();
                hidden.annotations.visible = false;
                assert!(Arc::ptr_eq(&f, &r.prepare(&hidden, SIZE, scale).unwrap()));
                let h = r.prepare_annotations(&hidden, SIZE, scale).unwrap();
                assert!(h.shares_rasters_with(&b), "a visible toggle re-rasterized");
                assert!(!h.draws_underlay() && !h.draws_overlay());
                let hidden_pixels = r.export_rgba(&hidden, SIZE, scale).unwrap();
                let mut bare = edited.clone();
                bare.annotations.items.clear();
                assert!(
                    hidden_pixels.rgba == fresh(&bare, scale),
                    "scale {scale}: hidden layer drew"
                );
                assert!(hidden_pixels.rgba == fresh(&hidden, scale));

                // 3) Selecting an annotation draws its box on the overlay
                //    without rebuilding the frame or the label raster.
                let mut selected = edited.clone();
                selected.decorations.selected = Some(DecorationId::Annotation(0));
                let s = r.prepare(&selected, SIZE, scale).unwrap();
                assert!(
                    Arc::ptr_eq(&f, &s),
                    "scale {scale}: selection rebuilt the frame"
                );
                let selected_pixels = r.export_rgba(&selected, SIZE, scale).unwrap();
                assert!(selected_pixels.rgba != after.rgba, "selection box missing");
                assert!(selected_pixels.rgba == fresh(&selected, scale));

                // 4) A drag through the map writes the SSOT, a rejected one
                //    leaves it alone, and the result round-trips JSON.
                let hit = b.annotations().find(0).unwrap();
                let old = b.annotations().bounds(hit).unwrap();
                let mut dragged = edited.clone();
                assert_eq!(
                    b.annotations()
                        .drag_by(&mut dragged.annotations, hit, 6., 4.),
                    Some(NudgeResult::Moved)
                );
                let untouched = dragged.clone();
                assert!(matches!(
                    b.annotations()
                        .drag_by(&mut dragged.annotations, hit, 1e6, 1e6),
                    Some(NudgeResult::Rejected(_))
                ));
                assert_eq!(dragged, untouched);
                let d = r.prepare_annotations(&dragged, SIZE, scale).unwrap();
                let moved = d
                    .annotations()
                    .bounds(d.annotations().find(0).unwrap())
                    .unwrap();
                assert!(
                    (moved.x - old.x).abs() <= 6. + 1e-3 && (moved.y - old.y).abs() <= 4. + 1e-3,
                    "{old:?} -> {moved:?}"
                );
                assert!(moved != old);
                #[cfg(feature = "serde")]
                {
                    let restored: $chart =
                        serde_json::from_str(&serde_json::to_string(&dragged).unwrap()).unwrap();
                    assert_eq!(restored, dragged);
                    assert!(
                        r.export_rgba(&restored, SIZE, scale).unwrap().rgba
                            == r.export_rgba(&dragged, SIZE, scale).unwrap().rgba
                    );
                }
                // A title edit still rebuilds the frame.
                let mut retitled = edited.clone();
                retitled.title = "Edited".into();
                assert!(!Arc::ptr_eq(
                    &f,
                    &r.prepare(&retitled, SIZE, scale).unwrap()
                ));
                c.decorations = Default::default();
                r.end_frame();
            }
        }
    };
}

fn annotated_radar() -> RadarChart {
    RadarChart {
        title: "Annotated radar".into(),
        categories: vec![
            Category::new("a", "Speed"),
            Category::new("b", "Safety"),
            Category::new("c", "Size"),
            Category::new("d", "Docs"),
        ],
        series: vec![RadarSeries::new(
            "s",
            "Release",
            vec![6., 8., 4., 7.],
            Color::from_rgb8(100, 145, 185),
        )],
        annotations: AnnotationLayer {
            visible: true,
            items: vec![
                boxed_text(Anchor::Ratio { x: 0.05, y: 0.05 }),
                Annotation::new(AnnotationKind::Arrow {
                    from: Anchor::Ratio { x: 0.1, y: 0.9 },
                    to: Anchor::Ratio { x: 0.4, y: 0.6 },
                    style: Default::default(),
                    text: Some(text("centre")),
                }),
            ],
        },
        ..Default::default()
    }
}

fn annotated_radial() -> RadialChart {
    let mut chart = RadialChart {
        title: "Annotated donut".into(),
        kind: RadialKind::Donut { inner_radius: 0.5 },
        slices: vec![
            RadialSlice::new("High", 35., Color::from_rgb8(100, 145, 185)),
            RadialSlice::new("Low", 65., Color::from_rgb8(175, 95, 140)),
        ],
        annotations: AnnotationLayer {
            visible: true,
            items: vec![
                boxed_text(Anchor::Ratio { x: 0.05, y: 0.05 }),
                Annotation::new(AnnotationKind::Arrow {
                    from: Anchor::Ratio { x: 0.1, y: 0.9 },
                    to: Anchor::Ratio { x: 0.4, y: 0.6 },
                    style: Default::default(),
                    text: Some(text("hole")),
                }),
            ],
        },
        ..Default::default()
    };
    chart.format_labels();
    chart
}

family_contract!(
    radar_annotation_layer_contract,
    RadarRenderer,
    RadarChart,
    annotated_radar(),
    |c| c.annotations.items[0] = boxed_text(Anchor::Ratio { x: 0.7, y: 0.8 })
);

family_contract!(
    radial_annotation_layer_contract,
    RadialRenderer,
    RadialChart,
    annotated_radial(),
    |c| c.annotations.items[0] = boxed_text(Anchor::Ratio { x: 0.7, y: 0.8 })
);

/// The anchors of every item, by kind only: a drag must keep each anchor's
/// kind.
fn anchor_kinds(layer: &AnnotationLayer) -> Vec<Vec<AnchorKind>> {
    layer
        .items
        .iter()
        .map(|item| match &item.kind {
            AnnotationKind::Text { anchor, .. } => vec![anchor.kind()],
            AnnotationKind::Arrow { from, to, .. } => vec![from.kind(), to.kind()],
            _ => Vec::new(),
        })
        .collect()
}

/// The pointer route of every family (#12 step 4): the annotation items sit
/// on top of every decoration while `Annotations` is active and are not part
/// of the route otherwise; a layer switch reuses the frame and both caches
/// and exports the same bytes as a fresh renderer; it clears the decoration
/// selection and keeps the data selection; a routed handle drag keeps the
/// anchors' kinds and round-trips JSON. Logical routing is identical at 1x
/// and 2x.
macro_rules! family_routing {
    ($name:ident, $renderer:ident, $chart:ident, $build:expr, $select_data:expr, $data_selected:expr) => {
        #[test]
        fn $name() {
            let _guard = LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            fn fresh(c: &$chart, scale: f32) -> Vec<u8> {
                let mut r = $renderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
                r.export_rgba(c, SIZE, scale).unwrap().rgba
            }
            let select_data: fn(&mut $chart) = $select_data;
            let data_selected: fn(&$chart) -> bool = $data_selected;
            let mut r = $renderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
            // An empty layer: switching to it reuses the frame and its label
            // raster and changes no pixel, and the cached export equals a
            // fresh one.
            {
                let mut empty: $chart = $build;
                empty.annotations.items.clear();
                let f = r.prepare(&empty, SIZE, 1.).unwrap();
                let before = r.export_rgba(&empty, SIZE, 1.).unwrap().rgba;
                empty.active_layer = LayerRef::Annotations;
                let g = r.prepare(&empty, SIZE, 1.).unwrap();
                assert!(
                    Arc::ptr_eq(&f, &g),
                    "empty layer: the switch rebuilt the frame"
                );
                assert!(
                    f.shares_annotations_with(&g),
                    "empty layer: the switch redrew the label raster"
                );
                let cached = r.export_rgba(&empty, SIZE, 1.).unwrap().rgba;
                assert!(cached == before, "empty layer: the switch changed pixels");
                assert!(
                    cached == fresh(&empty, 1.),
                    "empty layer: cached export differs from fresh"
                );
                r.end_frame();
            }
            let mut routes = Vec::new();
            for scale in [1., 2.] {
                let mut c: $chart = $build;
                assert_eq!(c.active_layer(), LayerRef::Base);
                let f = r.prepare(&c, SIZE, scale).unwrap();
                let a = r.prepare_annotations(&c, SIZE, scale).unwrap();
                // The decorations answer for the chart the frame was prepared
                // from, while `c` is edited below.
                let shown = c.clone();
                let annotations = a.annotations();
                let centre = |b: RectF| [b.x + b.width * 0.5, b.y + b.height * 0.5];
                let items: Vec<(usize, [f32; 2])> = annotations
                    .entries()
                    .map(|(id, e)| (e.index, on_item(annotations, &c.annotations, id)))
                    .collect();
                assert_eq!(items.len(), c.annotations.items.len());
                let title_id = f.id_of(&shown, &DecorationId::ChartTitle);
                let title = centre(f.bounds(&shown, title_id).unwrap());
                assert!(
                    annotations.hit_test(title).is_none(),
                    "an item covers the title"
                );
                let route = |layer: LayerRef, p: [f32; 2]| f.route(&a, layer, &shown, p);

                // `Base` and an out-of-range overlay: items are not routed.
                for layer in [LayerRef::Base, LayerRef::Overlay(3)] {
                    for (index, p) in &items {
                        assert!(
                            !matches!(route(layer, *p), Some(LayerHit::Annotation { .. })),
                            "scale {scale} {layer:?}: item {index} answered"
                        );
                        assert_eq!(route(layer, *p), f.hit(&shown, *p).map(LayerHit::Element));
                    }
                    assert_eq!(route(layer, title), Some(LayerHit::Element(title_id)));
                }

                // Entering the layer reuses the frame and both caches and
                // exports the bytes of a fresh renderer; the decoration
                // selection is cleared, the data selection kept.
                let before = r.export_rgba(&c, SIZE, scale).unwrap().rgba;
                c.decorations.selected = Some(DecorationId::ChartTitle);
                select_data(&mut c);
                assert_eq!(
                    c.set_active_layer(LayerRef::Annotations),
                    LayerRef::Annotations
                );
                assert_eq!(c.decorations.selected, None);
                assert!(data_selected(&c));
                select_data(&mut c);
                let mut plain = c.clone();
                plain.active_layer = LayerRef::Base;
                assert!(Arc::ptr_eq(
                    &r.prepare(&plain, SIZE, scale).unwrap(),
                    &r.prepare(&c, SIZE, scale).unwrap()
                ));
                let switched = r.prepare_annotations(&c, SIZE, scale).unwrap();
                assert!(
                    switched
                        .shares_rasters_with(&r.prepare_annotations(&plain, SIZE, scale).unwrap())
                );
                let entered = r.export_rgba(&c, SIZE, scale).unwrap().rgba;
                assert!(entered == r.export_rgba(&plain, SIZE, scale).unwrap().rgba);
                assert!(
                    entered == fresh(&c, scale),
                    "scale {scale}: switch differs from fresh"
                );
                assert!(before.len() == entered.len());
                // The frame with the data selection, which the rest reuses.
                let g = r.prepare(&c, SIZE, scale).unwrap();

                // `Annotations`: every item answers on top (the topmost one
                // under its centre must contain it), decorations below.
                let mut routed = Vec::new();
                for (index, p) in &items {
                    let Some(LayerHit::Annotation { index: picked, id }) =
                        route(LayerRef::Annotations, *p)
                    else {
                        panic!("scale {scale}: item {index} was not routed");
                    };
                    assert!(
                        picked == *index || annotations.bounds(id).unwrap().contains(p[0], p[1])
                    );
                    routed.push(picked);
                }
                assert_eq!(
                    route(LayerRef::Annotations, title),
                    Some(LayerHit::Element(title_id))
                );
                routes.push(routed);
                // A hidden layer is out of the route.
                let mut hidden = c.clone();
                hidden.annotations.visible = false;
                let h = r.prepare_annotations(&hidden, SIZE, scale).unwrap();
                for (_, p) in &items {
                    assert!(!matches!(
                        f.route(&h, LayerRef::Annotations, &shown, *p),
                        Some(LayerHit::Annotation { .. })
                    ));
                }

                // A routed press selects the first item with handles; its
                // handle drags through the map and keeps the anchor kinds.
                let (index, id) = annotations
                    .entries()
                    .find(|(id, _)| !annotations.selection_box(*id).unwrap().handles().is_empty())
                    .map(|(id, e)| (e.index, id))
                    .expect("an item with handles");
                c.decorations.selected = Some(DecorationId::Annotation(index));
                let handle_square = annotations.selection_box(id).unwrap().handles()[0];
                let handle = annotations
                    .hit_resize_handle(id, centre(handle_square))
                    .expect("live handle");
                let kinds = anchor_kinds(&c.annotations);
                let mut dragged = c.clone();
                assert_eq!(
                    annotations.resize_by(&mut dragged.annotations, id, handle, 3., 3.),
                    Some(NudgeResult::Moved)
                );
                assert_ne!(dragged.annotations, c.annotations);
                assert_eq!(anchor_kinds(&dragged.annotations), kinds);
                assert!(Arc::ptr_eq(&g, &r.prepare(&dragged, SIZE, scale).unwrap()));
                assert!(
                    r.export_rgba(&dragged, SIZE, scale).unwrap().rgba == fresh(&dragged, scale)
                );
                #[cfg(feature = "serde")]
                {
                    let json = serde_json::to_string(&dragged).unwrap();
                    assert!(json.contains(r#""active_layer":"Annotations""#), "{json}");
                    let restored: $chart = serde_json::from_str(&json).unwrap();
                    assert_eq!(restored, dragged);
                }

                // Leaving the layer clears the annotation selection only.
                assert_eq!(dragged.set_active_layer(LayerRef::Base), LayerRef::Base);
                assert_eq!(dragged.decorations.selected, None);
                assert!(data_selected(&dragged));
                assert!(Arc::ptr_eq(&g, &r.prepare(&dragged, SIZE, scale).unwrap()));
                assert!(
                    r.export_rgba(&dragged, SIZE, scale).unwrap().rgba == fresh(&dragged, scale)
                );
                r.end_frame();
            }
            assert_eq!(routes[0], routes[1], "1x and 2x route differently");
        }
    };
}

family_routing!(
    radar_active_layer_routing,
    RadarRenderer,
    RadarChart,
    annotated_radar(),
    |c| c.selected = Some(RadarTarget::vertex("s", "a")),
    |c| c.selected.is_some()
);

family_routing!(
    radial_active_layer_routing,
    RadialRenderer,
    RadialChart,
    annotated_radial(),
    |c| c.interaction.selected = Some(RadialTarget::main(0)),
    |c| c.interaction.selected.is_some()
);

// Radial charts have no value axes: data anchors are warnings, ratio
// anchors draw, and the chart frame is untouched either way.
#[test]
fn radial_data_anchors_are_warnings() {
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut r = RadialRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let mut c = RadialChart {
        slices: vec![
            RadialSlice::new("High", 35., Color::from_rgb8(100, 145, 185)),
            RadialSlice::new("Low", 65., Color::from_rgb8(175, 95, 140)),
        ],
        ..Default::default()
    };
    c.format_labels();
    let plain = r.export_rgba(&c, SIZE, 1.).unwrap();
    let f = r.prepare(&c, SIZE, 1.).unwrap();
    c.annotations.items = vec![
        boxed_text(Anchor::Data { x: 1., y: 1. }),
        span(AnnotationAxis::X, 0., 1.),
        Annotation::new(AnnotationKind::ReferenceLine {
            axis: AnnotationAxis::Y,
            value: f64::NAN,
            style: Default::default(),
            label: None,
        }),
        boxed_text(Anchor::Ratio { x: 0.02, y: 0.02 }),
    ];
    let a = r.prepare_annotations(&c, SIZE, 1.).unwrap();
    assert_eq!(
        a.render_warnings(),
        &[
            RenderWarning::AnnotationSkipped {
                index: 0,
                reason: AnnotationSkip::NoValueAxis
            },
            RenderWarning::AnnotationSkipped {
                index: 1,
                reason: AnnotationSkip::NoValueAxis
            },
            RenderWarning::AnnotationSkipped {
                index: 2,
                reason: AnnotationSkip::NonFinite
            },
        ]
    );
    assert_eq!(a.annotations().entries().count(), 1);
    assert!(!a.draws_underlay() && a.draws_overlay());
    assert!(Arc::ptr_eq(&f, &r.prepare(&c, SIZE, 1.).unwrap()));
    let with = r.export_rgba(&c, SIZE, 1.).unwrap();
    assert!(with.rgba != plain.rgba);
    let bounds = a
        .annotations()
        .bounds(a.annotations().find(3).unwrap())
        .unwrap();
    assert!(ink(&with, bounds, 1.) > 0);
    use ::renderer::annotation::AnnotationSpace;
    assert_eq!(f.annotation_space().value_axes(), [false, false]);
    assert!(
        f.annotation_space()
            .px_to_anchor([100., 100.], AnchorKind::Data)
            .is_none()
    );
    r.end_frame();
}

// Each item's hit region is what the raster draws for it: every strongly
// inked pixel of the item hits it, every point that hits it lies within the
// grab slack of its ink, and its selection box encloses all of it. Covers a
// stroke wider than the minimum grab strip, a diagonal arrow, whose empty
// bounding-box corners must not hit, and the labels of both.
#[test]
fn annotation_hit_regions_follow_the_drawn_items() {
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // The radar chart is the bounded family with a decoration map left; it
    // has no value axes, so every item is ratio-anchored.
    let mut r = RadarRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let thick = ArrowStyle {
        line: AnnotationLineStyle {
            width: 12.,
            ..Default::default()
        },
        ..Default::default()
    };
    let items = [
        (
            "thick arrow",
            Annotation::new(AnnotationKind::Arrow {
                from: Anchor::Ratio { x: 0.1, y: 0.3 },
                to: Anchor::Ratio { x: 0.9, y: 0.3 },
                style: thick,
                text: Some(text("target")),
            }),
        ),
        (
            "thin arrow",
            Annotation::new(AnnotationKind::Arrow {
                from: Anchor::Ratio { x: 0.5, y: 0.1 },
                to: Anchor::Ratio { x: 0.5, y: 0.9 },
                style: Default::default(),
                text: Some(text("mid")),
            }),
        ),
        (
            "diagonal arrow",
            Annotation::new(AnnotationKind::Arrow {
                from: Anchor::Ratio { x: 0.2, y: 0.8 },
                to: Anchor::Ratio { x: 0.75, y: 0.25 },
                style: Default::default(),
                text: Some(text("rise")),
            }),
        ),
    ];
    // Anti-aliasing and the grab slack: the minimum strip half width (3 px)
    // plus the label padding (2 px), plus a pixel of rounding.
    const SLACK: f32 = 6.0;
    for (name, item) in items {
        let mut c = RadarChart {
            categories: vec![
                Category::new("a", "A"),
                Category::new("b", "B"),
                Category::new("c", "C"),
            ],
            series: vec![RadarSeries::new(
                "s",
                "S",
                vec![5., 10., 7.],
                Color::from_rgb8(60, 130, 210),
            )],
            legend: false,
            ..Default::default()
        };
        c.annotations.items = vec![item];
        for scale in [1., 2.] {
            let with = r.export_rgba(&c, SIZE, scale).unwrap();
            c.annotations.visible = false;
            let without = r.export_rgba(&c, SIZE, scale).unwrap();
            c.annotations.visible = true;
            let a = r.prepare_annotations(&c, SIZE, scale).unwrap();
            let map = a.annotations();
            let (id, _) = map
                .entries()
                .next()
                .unwrap_or_else(|| panic!("{name}: skipped {:?}", a.render_warnings()));
            let (w, h) = (with.width as usize, with.height as usize);
            let diff = |x: usize, y: usize| {
                let i = (y * w + x) * 4;
                (0..4)
                    .map(|k| with.rgba[i + k].abs_diff(without.rgba[i + k]))
                    .max()
                    .unwrap()
            };
            let selection = map.selection_box(id).unwrap().rect().expanded(1.0);
            let mut strong = 0;
            for y in 0..h {
                for x in 0..w {
                    let d = diff(x, y);
                    if d == 0 {
                        continue;
                    }
                    let p = [(x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale];
                    assert!(
                        selection.contains(p[0], p[1]),
                        "{name} at {scale}x: ink at {p:?} outside the box {selection:?}"
                    );
                    if d >= 128 {
                        strong += 1;
                        assert_eq!(
                            map.hit_test(p),
                            Some(id),
                            "{name} at {scale}x: drawn at {p:?} but not hit"
                        );
                    }
                }
            }
            assert!(strong > 0, "{name} at {scale}x drew nothing");
            // Every hit lies near the item's ink.
            let reach = (SLACK * scale).ceil() as i64;
            let inked_near = |px: f32, py: f32| {
                let (cx, cy) = ((px * scale) as i64, (py * scale) as i64);
                (cy - reach..=cy + reach).any(|y| {
                    (cx - reach..=cx + reach).any(|x| {
                        (0..w as i64).contains(&x)
                            && (0..h as i64).contains(&y)
                            && diff(x as usize, y as usize) > 0
                    })
                })
            };
            let (lw, lh) = (SIZE.0 as f32, SIZE.1 as f32);
            let mut y = 0.5;
            while y < lh {
                let mut x = 0.5;
                while x < lw {
                    if map.hit_test([x, y]) == Some(id) {
                        assert!(
                            inked_near(x, y),
                            "{name} at {scale}x: ({x}, {y}) hits with no ink nearby"
                        );
                    }
                    x += 3.0;
                }
                y += 3.0;
            }
        }
    }
    r.end_frame();
}
