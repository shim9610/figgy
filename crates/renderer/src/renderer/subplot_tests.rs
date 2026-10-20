//! Subplots through the registry: per-panel raster caches, linked axes moved
//! in one registry call, the document union in display, pick and export,
//! the annotation route, warnings and streaming rejection. The pixel oracle
//! is always a fresh renderer given the final state.
use super::*;
use crate::layout::{ChartArea, Rect};
use crate::select::SelectionBox;
use crate::text::RichText;
use crate::{CellRef, HitMap, LayerRef, OverlayLayer, Subplot, SubplotAxis, WarningAxis};

const UPPER: Rect = Rect {
    x: 0,
    y: 0,
    width: 320,
    height: 120,
};

fn column(data: Vec<f32>) -> crate::Column<f32> {
    let min = data.iter().copied().fold(f32::INFINITY, f32::min);
    let max = data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    crate::Column { data, min, max }
}

fn renderer() -> Renderer {
    let (device, queue) = data_render::shared_device().unwrap();
    let mut r = Renderer::try_new_with_sample_count(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
        1,
    )
    .unwrap();
    for (id, values) in [
        ("x", column(vec![0.1, 0.3, 0.5, 0.7, 0.9])),
        ("a", column(vec![0.2, 0.6, 0.3, 0.8, 0.4])),
        ("b", column(vec![10.0, 30.0, 20.0, 50.0, 40.0])),
        ("c", column(vec![-1.0, 0.0, 1.0, 0.5, -0.5])),
    ] {
        r.add_column(id, &values).unwrap();
    }
    r
}

/// A compact chart: small bands so a 120 px high area keeps a data area.
fn chart(area: Rect, y: (f64, f64)) -> Config {
    let mut c = crate::default::default_config();
    c.chart_area = ChartArea(area);
    c.chart_title.visible = true;
    c.chart_title.text = RichText::plain("Chart", Color::BLACK, 10.0, "");
    c.chart_title.top_margin = 14.0;
    for a in [&mut c.bottom_x, &mut c.left_y, &mut c.top_x, &mut c.right_y] {
        a.min = 0.0;
        a.max = 1.0;
        a.major_spacing = 0.25;
        a.minor_count = 1;
        a.out_margin = 18.0;
        a.label_style.font_size = 9.0;
        a.title_option.visible = false;
    }
    c.left_y.min = y.0;
    c.left_y.max = y.1;
    c.left_y.major_spacing = (y.1 - y.0) / 4.0;
    c
}

/// The parent (`s0`) in the upper half; subplot 0 in the lower left draws
/// `s1` on its own axes and `s2` on an overlay; subplot 1 in the lower right
/// draws nothing of its own but links X to the parent and Y to subplot 0.
fn document() -> Config {
    let mut c = chart(UPPER, (0.0, 1.0));
    let mut first = chart(
        Rect {
            x: 0,
            y: 120,
            width: 160,
            height: 120,
        },
        (0.0, 60.0),
    );
    first.bottom_x.min = -0.5;
    let mut overlay = OverlayLayer::default();
    overlay.y_axis.min = -1.5;
    overlay.y_axis.max = 1.5;
    overlay.y_axis.major_spacing = 0.75;
    overlay.y_axis.out_margin = 18.0;
    overlay.y_axis.label_style.font_size = 9.0;
    overlay.y_axis.title_option.visible = false;
    overlay.series = vec!["s2".into()];
    first.overlays.push(overlay);
    let second = chart(
        Rect {
            x: 160,
            y: 120,
            width: 160,
            height: 120,
        },
        (0.0, 1.0),
    );
    c.subplots = vec![
        Subplot {
            config: first,
            series: vec!["s1".into()],
            link_x: None,
            link_y: None,
        },
        Subplot {
            config: second,
            series: Vec::new(),
            link_x: Some(CellRef::Parent),
            link_y: Some(CellRef::Subplot(0)),
        },
    ];
    c.sync_subplot_links(None);
    c
}

fn series() -> Vec<SeriesConfig> {
    vec![
        declaration("s0", "x", "a"),
        declaration("s1", "x", "b"),
        declaration("s2", "x", "c"),
    ]
}

fn frame_with_view(r: &mut Renderer, id: ChartId, view: &ChartView) -> Vec<u8> {
    let prepared = r
        .prepare_registered(&[RegisteredChartDrawItem { chart_id: id, view }])
        .unwrap();
    paint_frame_pixels(r, &prepared, 1)
}

fn fresh_frame(config: &Config) -> Vec<u8> {
    fresh_frame_with(config, series())
}

fn fresh_frame_with(config: &Config, series: Vec<SeriesConfig>) -> Vec<u8> {
    let mut r = renderer();
    let id = r.register_chart(config.clone(), series).unwrap();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    frame_with_view(&mut r, id, &view)
}

fn refresh(r: &mut Renderer, id: ChartId, view: &mut ChartView, selection: &[SelectionBox]) {
    let config = r.chart_config(id).unwrap().clone();
    let panel = config.chart_area.0;
    r.refresh_axis_with_selection(view, &Chart::new(config), panel, selection)
        .unwrap();
}

/// The `(x, y)` pixels at which `a` and `b` (320 wide) differ.
fn differing(a: &[u8], b: &[u8]) -> Vec<(usize, usize)> {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .enumerate()
        .filter(|(_, (p, q))| p != q)
        .map(|(i, _)| (i % 320, i / 320))
        .collect()
}

fn selection_box_at(x: f32, y: f32) -> SelectionBox {
    SelectionBox {
        rect: crate::layout::RectF {
            x,
            y,
            width: 40.0,
            height: 20.0,
        },
        color: crate::select::SELECTION_COLOR,
        stroke_width: crate::select::SELECTION_STROKE_WIDTH,
        handles: Vec::new(),
        outline: crate::select::SelectionOutline::Rect,
    }
}

// Each subplot draws as a chart of its own, in its own area, with its own
// panels and raster caches: an edit of one chart re-rasters that chart's
// panels only, a linked range edit re-rasters exactly the charts of its link
// group, and every cached frame is byte-identical to a fresh render. A
// selection box re-rasters only the topmost panel of the chart it sits in.
#[test]
fn a_subplot_edit_refreshes_only_its_panels_and_matches_a_fresh_render() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut failures = Vec::new();
    let mut check = |ok: bool, msg: String| {
        if !ok {
            failures.push(msg);
        }
    };
    let config = document();
    let mut r = renderer();
    let id = r.register_chart(config.clone(), series()).unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(config.clone()), UPPER)
        .unwrap();
    check(
        view.layer_raster_generations() == vec![1, 1, 1, 1],
        format!("initial generations {:?}", view.layer_raster_generations()),
    );
    let first = frame_with_view(&mut r, id, &view);
    check(
        first == fresh_frame(&config),
        "first frame differs from a fresh render".into(),
    );
    // Subplot 0's data is drawn in its own area only.
    let pixels = differing(
        &first,
        &fresh_frame_with(&config, vec![declaration("s0", "x", "a")]),
    );
    check(
        !pixels.is_empty() && pixels.iter().all(|&(x, y)| x < 160 && y >= 120),
        "subplot 0 data outside its area".into(),
    );

    let cases: [(&str, fn(&mut Config), [u64; 4]); 6] = [
        (
            "subplot 0 overlay axis colour",
            |c| c.subplots[0].config.overlays[0].y_axis.line_color = Color::new(0.9, 0.1, 0.1, 1.0),
            [1, 1, 2, 1],
        ),
        (
            "subplot 0 title",
            |c| {
                c.subplots[0].config.chart_title.text =
                    RichText::plain("Left", Color::BLACK, 10.0, "")
            },
            [1, 2, 2, 1],
        ),
        (
            "parent title",
            |c| c.chart_title.text = RichText::plain("Top", Color::BLACK, 10.0, ""),
            [2, 2, 2, 1],
        ),
        // Subplot 1 links its X to the parent: both move, nothing else.
        (
            "parent X range",
            |c| {
                c.bottom_x.min = 0.2;
                c.bottom_x.max = 0.8;
            },
            [3, 2, 2, 2],
        ),
        // Written on the member, carried to the root (subplot 0's Y) in the
        // same registry call.
        (
            "linked Y range from the member",
            |c| c.subplots[1].config.left_y.max = 90.0,
            [3, 3, 2, 3],
        ),
        (
            "subplot 0 area width",
            |c| c.subplots[0].config.chart_area.0.width = 150,
            [3, 4, 3, 3],
        ),
    ];
    let mut previous = first;
    for (name, apply, expected) in cases {
        let mut next = r.chart_config(id).unwrap().clone();
        apply(&mut next);
        let before = r.chart_render_stamp(id).unwrap();
        r.set_chart_config(id, next).unwrap();
        let after = r.chart_render_stamp(id).unwrap();
        check(
            after.needs_raster_since(Some(&before)),
            format!("{name}: an edit must refresh the raster"),
        );
        refresh(&mut r, id, &mut view, &[]);
        check(
            view.layer_raster_generations() == expected,
            format!(
                "{name}: generations {:?}, expected {expected:?}",
                view.layer_raster_generations()
            ),
        );
        let stored = r.chart_config(id).unwrap().clone();
        let actual = frame_with_view(&mut r, id, &view);
        check(
            actual == fresh_frame(&stored),
            format!("{name}: cached view + refreshed panels differ from a fresh render"),
        );
        check(
            actual != previous,
            format!("{name}: the edit changed no pixel"),
        );
        previous = actual;
    }
    let stored = r.chart_config(id).unwrap().clone();
    check(
        stored.subplots[0].config.left_y.max == 90.0
            && stored.subplots[1].config.left_y.max == 90.0,
        "the member edit did not move the link group".into(),
    );

    refresh(&mut r, id, &mut view, &[selection_box_at(220.0, 160.0)]);
    check(
        view.layer_raster_generations() == vec![3, 4, 3, 4],
        format!("subplot 1 selection: {:?}", view.layer_raster_generations()),
    );
    let boxed = frame_with_view(&mut r, id, &view);
    check(
        differing(&boxed, &previous)
            .iter()
            .all(|&(x, y)| x >= 160 && y >= 120),
        "the selection box was drawn outside subplot 1".into(),
    );
    refresh(&mut r, id, &mut view, &[selection_box_at(60.0, 40.0)]);
    check(
        view.layer_raster_generations() == vec![4, 4, 3, 5],
        format!("parent selection: {:?}", view.layer_raster_generations()),
    );
    refresh(&mut r, id, &mut view, &[]);
    check(
        frame_with_view(&mut r, id, &view) == previous,
        "clearing the selection did not restore the frame".into(),
    );
    drop(_fonts);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// One `set_chart_view_state` call moves a link group with everything else:
// the view state reads a linked axis as its target's range and writing it
// moves the root. A view state with the wrong subplot shape is rejected.
#[test]
fn a_linked_view_state_edit_moves_the_group_in_one_call() {
    let fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut r = renderer();
    let id = r.register_chart(document(), series()).unwrap();
    let mut view = r.create_chart_view(&Chart::new(document()), UPPER).unwrap();
    let mut state = r.chart_view_state(id).unwrap();
    assert_eq!(state.subplots.len(), 2);
    assert_eq!(state.subplots[1].bottom_x, state.bottom_x);
    assert_eq!(state.subplots[1].left_y, state.subplots[0].left_y);
    assert_eq!(state.subplots[0].overlay_y.len(), 1);
    state.subplots[1].bottom_x.min = 0.3;
    state.subplots[0].left_y.max = 80.0;
    r.set_chart_view_state(id, state.clone()).unwrap();
    let edited = r.chart_config(id).unwrap().clone();
    refresh(&mut r, id, &mut view, &[]);
    let after = frame_with_view(&mut r, id, &view);
    let fresh = fresh_frame(&edited);

    let mut short = r.chart_view_state(id).unwrap();
    short.subplots.pop();
    let rejected = r.set_chart_view_state(id, short);
    drop(fonts);

    assert_eq!(
        edited.bottom_x.min, 0.3,
        "the member's X edit moved the parent"
    );
    assert_eq!(edited.subplots[1].config.bottom_x.min, 0.3);
    assert_eq!(edited.subplots[1].config.left_y.max, 80.0);
    assert_eq!(
        after, fresh,
        "the linked edit lands on every chart in one frame"
    );
    assert!(matches!(
        rejected,
        Err(FiggyError::InvalidConfig {
            field: "subplots",
            ..
        })
    ));
}

// The document's annotation layer stays in the parent's area: it draws
// there only, and its items answer the pointer route there only while
// `Annotations` is active; with a subplot active the route reaches the
// subplot's own elements.
#[test]
fn annotations_stay_in_the_parent_of_a_subplot_document() {
    use crate::annotation::{Anchor, Annotation, AnnotationKind};
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut config = document();
    let plain = fresh_frame(&config);
    config.annotations.items = vec![Annotation::new(AnnotationKind::Text {
        anchor: Anchor::Ratio { x: 0.4, y: 0.3 },
        text: RichText::plain("note", Color::BLACK, 10.0, ""),
        box_style: Some(Default::default()),
    })];
    let annotated = fresh_frame(&config);
    let pixels = differing(&plain, &annotated);
    assert!(!pixels.is_empty(), "the annotation drew nothing");
    assert!(
        pixels.iter().all(|&(_, y)| y < 120),
        "annotation drawn outside the parent's area"
    );

    let r = renderer();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), UPPER)
        .unwrap();
    let map = view.annotation_map();
    let note = map.bounds(map.find(0).unwrap()).unwrap();
    let note = [note.x + note.width / 2.0, note.y + note.height / 2.0];
    let lower = config.subplot_config(0).unwrap().data_area().unwrap().0;
    let inside = [
        lower.x as f32 + lower.width as f32 / 2.0,
        lower.y as f32 + lower.height as f32 / 2.0,
    ];
    let name = |hits: &HitMap, hit: Option<crate::LayerHit>| match hit {
        Some(crate::LayerHit::Annotation { index, .. }) => format!("annotation_{index}"),
        Some(crate::LayerHit::Element(id)) => hits.get(id).unwrap().element_id(),
        None => "none".into(),
    };
    config.set_active_layer(LayerRef::Annotations);
    let hits = HitMap::for_active_layer(&config);
    let route = |p| name(&hits, view.hit_test_active_layer(&config, &hits, p));
    assert_eq!(route(note), "annotation_0");
    assert_eq!(route(inside), "none");
    config.set_active_layer(LayerRef::Subplot(0));
    let hits = HitMap::for_active_layer(&config);
    let route = |p| name(&hits, view.hit_test_active_layer(&config, &hits, p));
    assert_ne!(route(note), "annotation_0");
    assert_eq!(route(inside), "subplot_0:data_area");
}

// The document export covers the union of every chart area; the panel
// export keeps the parent's. Both scale with the export.
#[test]
fn a_subplot_document_exports_every_chart() {
    let fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut r = renderer();
    let chart = Chart::new(document());
    let document_image = r.export_document_rgba(&chart, &series(), 1.0).unwrap();
    let panel = r.export_panel_rgba(&chart, &series(), 1.0).unwrap();
    let large = r.export_document_rgba(&chart, &series(), 2.0).unwrap();
    let mut side = document();
    side.subplots[1].config.chart_area.0 = Rect {
        x: 320,
        y: 0,
        width: 200,
        height: 120,
    };
    let wide = r
        .export_document_rgba(&Chart::new(side), &series(), 1.0)
        .unwrap();
    drop(fonts);

    assert_eq!((document_image.width, document_image.height), (320, 240));
    assert_eq!((panel.width, panel.height), (320, 120));
    assert_eq!((large.width, large.height), (640, 480));
    assert_eq!((wide.width, wide.height), (520, 240));
    assert_eq!(&document_image.rgba[..320 * 120 * 4], &panel.rgba[..]);
    assert!(
        document_image.rgba[320 * 120 * 4..]
            .chunks_exact(4)
            .any(|p| p[3] != 0)
    );
    let right = (0..120)
        .flat_map(|y| (330..520).map(move |x| (y * 520 + x) * 4 + 3))
        .filter(|&i| wide.rgba[i] != 0)
        .count();
    assert!(right > 0, "no ink right of the parent");
}

// A display fits the whole document and moves every chart with the parent;
// a pick of the active subplot finds its series in its own area.
#[test]
fn a_subplot_document_is_displayed_and_picked_as_one_document() {
    let config = document();
    let (display, panel, scale) = display_config_for_surface(&config, (640, 480));
    assert_eq!(scale, 2.0);
    assert_eq!(panel, display.chart_area.0);
    assert_eq!(
        display.chart_area.0,
        Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 240,
        }
    );
    assert_eq!(
        display.subplots[1].config.chart_area.0,
        Rect {
            x: 320,
            y: 240,
            width: 320,
            height: 240,
        }
    );
    let (display, panel, _) = display_config_for_surface(&config, (800, 480));
    assert_eq!(panel.x, 80);
    assert_eq!(display.subplots[0].config.chart_area.0.x, 80);
    let plain = chart(UPPER, (0.0, 1.0));
    let (display, panel, _) = display_config_for_surface(&plain, (800, 480));
    assert_eq!(display.chart_area.0, panel);
    assert_eq!(panel, fit_display_panel((320, 120), (800, 480)).1);

    let mut r = renderer();
    let mut config = document();
    config.set_active_layer(LayerRef::Subplot(0));
    let id = r.register_chart(config.clone(), series()).unwrap();
    r.enable_gpu_picking().unwrap();
    let (display, _, scale) = display_config_for_surface(&config, (320, 240));
    let area = display.subplot_config(0).unwrap().data_area().unwrap().0;
    // Point 3 of `s1`: x = 0.7 of -0.5..1, y = 50 of 0..60.
    let px = area.x as f32 + (0.7 + 0.5) / 1.5 * area.width as f32;
    let py = (area.y + area.height) as f32 - 50.0 / 60.0 * area.height as f32;
    let ticket = r
        .pick_chart(
            id,
            GpuPickRequest {
                canvas_position_px: [px, py],
                display_panel_px: fit_display_panel((320, 240), (320, 240)).1,
                display_scale: scale,
                max_distance_px: 6.0,
            },
        )
        .unwrap();
    let picked = pollster::block_on(ticket.resolve()).unwrap().unwrap();
    assert_eq!(picked.series_id, "s1");
    assert_eq!(picked.point_index, 3);
}

// A subplot's warnings reach the view tagged with the subplot; the API keeps
// layers in range; a streamed series cannot be drawn by a subplot and the
// stream passes reject a document with subplots.
#[test]
fn subplot_warnings_layers_and_streaming_rules() {
    let mut config = document();
    config.subplots[0].config.top_x.major_spacing = 0.0;
    config.subplots[0].config.overlays[0].y_axis.major_spacing = f64::NAN;
    let mut r = renderer();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), UPPER)
        .unwrap();
    let axes: Vec<WarningAxis> = view
        .render_warnings()
        .iter()
        .filter_map(|w| w.axis())
        .collect();
    assert_eq!(
        axes,
        vec![
            WarningAxis::Subplot {
                index: 0,
                axis: SubplotAxis::TopX
            },
            WarningAxis::Subplot {
                index: 0,
                axis: SubplotAxis::OverlayY(0)
            },
        ]
    );
    assert_eq!(
        view.render_warnings()[1].to_string().split(' ').next(),
        Some("subplots[0].config.overlays[0].y_axis.major_spacing")
    );

    let id = r.register_chart(document(), series()).unwrap();
    assert_eq!(
        r.set_active_layer(id, LayerRef::SubplotOverlay(0, 0))
            .unwrap(),
        LayerRef::SubplotOverlay(0, 0)
    );
    assert_eq!(
        r.set_active_layer(id, LayerRef::SubplotOverlay(1, 0))
            .unwrap(),
        LayerRef::Subplot(1)
    );
    assert_eq!(
        r.set_active_layer(id, LayerRef::Subplot(2)).unwrap(),
        LayerRef::Base
    );
    let mut fewer = document();
    fewer.active_layer = LayerRef::Subplot(1);
    fewer.subplots.pop();
    r.set_chart_config(id, fewer).unwrap();
    assert_eq!(r.chart_config(id).unwrap().active_layer, LayerRef::Base);
    let mut invalid = document();
    invalid.subplots[0].link_x = Some(CellRef::Subplot(1));
    assert!(matches!(
        r.set_chart_config(id, invalid),
        Err(FiggyError::InvalidConfig {
            field: "subplots",
            ..
        })
    ));

    r.register_streamed_columns(vec![source("sx", 1), source("sy", 1)])
        .unwrap();
    let mut streamed = document();
    streamed.subplots[0].series = vec!["streamed".into()];
    let declarations = vec![declaration("streamed", "sx", "sy")];
    assert!(matches!(
        r.register_chart(streamed.clone(), declarations.clone()),
        Err(FiggyError::InvalidSeriesConfig { series_id, .. }) if series_id == "streamed"
    ));
    streamed.subplots[0].series.clear();
    let layered = r.register_chart(streamed, declarations).unwrap();
    assert!(matches!(
        r.inspect_streaming_capability(layered),
        Err(FiggyError::InvalidConfig {
            field: "subplots",
            ..
        })
    ));
}

#[test]
fn a_base_auto_fit_moves_the_subplots_linked_to_the_parent() {
    let mut r = renderer();
    let id = r.register_chart(document(), series()).unwrap();
    let token = r.begin_fit_commit(id).unwrap();
    let x = crate::chart::FitExtent {
        min: 2.0,
        max: 4.0,
        min_positive: Some(2.0),
    };
    let y = crate::chart::FitExtent {
        min: -3.0,
        max: 3.0,
        min_positive: Some(3.0),
    };
    r.commit_auto_fit_all_if_current(&token, &x, &y, 0.0)
        .unwrap();
    let config = r.chart_config(id).unwrap();
    assert_eq!((config.bottom_x.min, config.bottom_x.max), (2.0, 4.0));
    // Subplot 1 links X to the parent; subplot 0 keeps its own X axis.
    let linked = &config.subplots[1].config.bottom_x;
    assert_eq!((linked.min, linked.max), (2.0, 4.0));
    assert_eq!(config.subplots[0].config.bottom_x.min, -0.5);
}

// A linked axis is one axis shared by its link group: fitting it from one
// chart's series alone would hand every other member a range that cuts its
// data. `auto_fit_layer_union` fits the whole group, overlays aside on Y.
#[test]
fn a_layer_fit_covers_its_whole_link_group() {
    let mut r = renderer();
    r.add_column("xw", &column(vec![2.0, 3.0, 4.0, 5.0, 6.0]))
        .unwrap();
    // Subplot 1 (X linked to the parent, Y to subplot 0) draws `s3` far right
    // of the parent's X data.
    let mut config = document();
    config.subplots[1].series = vec!["s3".into()];
    let mut all = series();
    all.push(declaration("s3", "xw", "a"));
    let pool = r.pool();
    let padding = 0.0;

    // The subplot's own series alone: the parent's X data falls off the left.
    let mut alone = Chart::new(config.clone());
    alone
        .auto_fit_subplot_x_union(pool, 1, &["xw"], padding)
        .unwrap();
    assert!(alone.config().bottom_x.min > 0.9);

    let mut group = Chart::new(config.clone());
    group
        .auto_fit_layer_union(pool, LayerRef::Subplot(1), &all, padding)
        .unwrap();
    let c = group.config();
    // X: the parent's `x` and the subplot's `xw`; the parent follows the link.
    assert!(
        c.bottom_x.min < 0.11 && c.bottom_x.max >= 6.0,
        "{:?}",
        c.bottom_x
    );
    let linked = &c.subplots[1].config.bottom_x;
    assert_eq!((linked.min, linked.max), (c.bottom_x.min, c.bottom_x.max));
    // Y: subplot 0's `b` (its Y link root) and the subplot's own `a`, not
    // subplot 0's overlay series `c`.
    let y = &c.subplots[1].config.left_y;
    assert!(y.min < 0.21 && y.max >= 50.0 && y.min > -1.0, "{y:?}");
    assert_eq!(c.subplots[0].config.left_y.max, y.max);
    // Subplot 0 keeps its own X axis.
    assert_eq!(c.subplots[0].config.bottom_x.min, -0.5);

    // The base layer fits the same X group, and its Y from `s0` alone.
    let mut base = Chart::new(config.clone());
    base.auto_fit_layer_union(pool, LayerRef::Base, &all, padding)
        .unwrap();
    let c = base.config();
    assert!(c.bottom_x.min < 0.11 && c.bottom_x.max >= 6.0);
    assert_eq!(c.subplots[1].config.bottom_x.max, c.bottom_x.max);
    assert!(c.left_y.min < 0.21 && c.left_y.max > 0.79 && c.left_y.max < 10.0);

    // An overlay layer fits only its own Y axis, from its own series.
    let mut overlay = Chart::new(config);
    overlay
        .auto_fit_layer_union(pool, LayerRef::SubplotOverlay(0, 0), &all, padding)
        .unwrap();
    let c = overlay.config();
    let axis = &c.subplots[0].config.overlays[0].y_axis;
    assert!(
        axis.min <= -1.0 && axis.max >= 1.0 && axis.max < 10.0,
        "{axis:?}"
    );
    assert_eq!(c.bottom_x, document().bottom_x);
}
