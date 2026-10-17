//! Overlay layers through the registry: per-layer raster caches, the atomic
//! linked-range edit, the active-layer API, streaming rejection, document
//! export and the pick inputs. The pixel oracle is always a fresh renderer
//! given the final state, never the implementation's own invalidation rule.
use super::*;
use crate::layout::{ChartArea, Rect};
use crate::select::SelectionBox;
use crate::text::RichText;
use crate::{HitMap, LayerRef, OverlayLayer, WarningAxis};

const PANEL: Rect = Rect {
    x: 0,
    y: 0,
    width: 320,
    height: 240,
};

fn column(data: Vec<f32>) -> crate::Column<f32> {
    let min = data.iter().copied().fold(f32::INFINITY, f32::min);
    let max = data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    crate::Column { data, min, max }
}

fn columns() -> [(&'static str, crate::Column<f32>); 4] {
    [
        ("x", column(vec![0.1, 0.3, 0.5, 0.7, 0.9])),
        ("a", column(vec![0.2, 0.6, 0.3, 0.8, 0.4])),
        ("b", column(vec![10.0, 30.0, 20.0, 50.0, 40.0])),
        ("c", column(vec![-1.0, 0.0, 1.0, 0.5, -0.5])),
    ]
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
    for (id, values) in columns() {
        r.add_column(id, &values).unwrap();
    }
    r
}

fn base_config() -> Config {
    let mut c = crate::default::default_config();
    c.chart_area = ChartArea(PANEL);
    c.chart_title.visible = true;
    c.chart_title.text = RichText::plain("Layers", Color::BLACK, 12.0, "");
    c.chart_title.top_margin = 20.0;
    for a in [&mut c.bottom_x, &mut c.left_y, &mut c.top_x, &mut c.right_y] {
        a.min = 0.0;
        a.max = 1.0;
        a.major_spacing = 0.25;
        a.minor_count = 1;
        a.out_margin = 30.0;
        a.label_style.font_size = 10.0;
    }
    c
}

fn overlay(min: f64, max: f64, series: &str, out_margin: f32) -> OverlayLayer {
    let mut overlay = OverlayLayer::default();
    overlay.y_axis.min = min;
    overlay.y_axis.max = max;
    overlay.y_axis.major_spacing = (max - min) / 4.0;
    overlay.y_axis.minor_count = 1;
    overlay.y_axis.out_margin = out_margin;
    overlay.y_axis.label_style.font_size = 10.0;
    overlay.series = vec![series.into()];
    overlay
}

/// Base series `s0`, overlay 0 scales `s1`, overlay 1 scales `s2`.
fn two_overlay_config() -> Config {
    let mut c = base_config();
    c.overlays.push(overlay(0.0, 60.0, "s1", 40.0));
    let mut second = overlay(-1.5, 1.5, "s2", 40.0);
    second.y_axis.line_offset = 44.0;
    c.overlays.push(second);
    c
}

fn three_series() -> Vec<SeriesConfig> {
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

/// A fresh renderer, chart and view for `config`: the oracle.
fn fresh_frame(config: &Config, series: Vec<SeriesConfig>) -> Vec<u8> {
    let mut r = renderer();
    let id = r.register_chart(config.clone(), series).unwrap();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    frame_with_view(&mut r, id, &view)
}

fn refresh(r: &mut Renderer, id: ChartId, view: &mut ChartView, selection: &[SelectionBox]) {
    let config = r.chart_config(id).unwrap().clone();
    r.refresh_axis_with_selection(view, &Chart::new(config), PANEL, selection)
        .unwrap();
}

// Editing one layer re-rasters that layer's panel only; the other panels'
// textures are reused as they are, and the frame drawn from the cached view
// plus the one refreshed layer is byte-identical to a fresh render of the
// final state. A group-geometry edit (the shared right band) lifts every
// panel's margins, so every panel re-rasters; that is the one legitimate
// all-layer refresh.
#[test]
fn a_layer_edit_refreshes_only_that_layers_raster_and_matches_a_fresh_render() {
    let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut failures = Vec::new();
    let mut check = |ok: bool, msg: String| {
        if !ok {
            failures.push(msg);
        }
    };
    let config = two_overlay_config();
    let mut r = renderer();
    let id = r.register_chart(config.clone(), three_series()).unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    check(
        view.layer_raster_generations() == vec![1, 1, 1],
        format!("initial generations {:?}", view.layer_raster_generations()),
    );
    check(view.overlay_count() == 2, "view holds both overlays".into());
    let first = frame_with_view(&mut r, id, &view);
    check(
        first == fresh_frame(&config, three_series()),
        "first frame differs from a fresh render".into(),
    );
    // A refresh with nothing changed uploads nothing.
    refresh(&mut r, id, &mut view, &[]);
    check(
        view.layer_raster_generations() == vec![1, 1, 1],
        format!(
            "unchanged refresh re-rastered: {:?}",
            view.layer_raster_generations()
        ),
    );

    let cases: [(&str, fn(&mut Config), [u64; 3]); 5] = [
        (
            "overlay 1 axis colour",
            |c| c.overlays[1].y_axis.line_color = Color::new(0.9, 0.1, 0.1, 1.0),
            [1, 1, 2],
        ),
        (
            "overlay 0 range",
            |c| {
                c.overlays[0].y_axis.min = -10.0;
                c.overlays[0].y_axis.max = 70.0;
            },
            [1, 2, 2],
        ),
        (
            "base title",
            |c| c.chart_title.text = RichText::plain("Edited", Color::BLACK, 12.0, ""),
            [2, 2, 2],
        ),
        (
            "overlay 0 legend",
            |c| {
                let mut legend = crate::default::default_legend();
                legend.visible = true;
                legend.content = RichText::plain("Overlay", Color::BLACK, 10.0, "");
                c.overlays[0].legend = Some(legend);
            },
            [2, 3, 2],
        ),
        // Group geometry: the shared right band widens for every panel.
        (
            "overlay 1 band",
            |c| c.overlays[1].y_axis.out_margin = 70.0,
            [3, 4, 3],
        ),
    ];
    let mut previous = first;
    for (name, apply, expected) in cases {
        let mut next = r.chart_config(id).unwrap().clone();
        apply(&mut next);
        let before = r.chart_render_stamp(id).unwrap();
        r.set_chart_config(id, next.clone()).unwrap();
        let after = r.chart_render_stamp(id).unwrap();
        check(
            after.needs_raster_since(Some(&before)),
            format!("{name}: a layer edit must refresh the raster"),
        );
        refresh(&mut r, id, &mut view, &[]);
        check(
            view.layer_raster_generations() == expected,
            format!(
                "{name}: generations {:?}, expected {expected:?}",
                view.layer_raster_generations()
            ),
        );
        let actual = frame_with_view(&mut r, id, &view);
        check(
            actual == fresh_frame(&next, three_series()),
            format!("{name}: cached view + refreshed layer differs from a fresh render"),
        );
        check(
            actual != previous,
            format!("{name}: the edit changed no pixel"),
        );
        previous = actual;
    }

    // Selection chrome lives in the topmost panel: a selection box refresh
    // re-rasters that panel only, and clearing it restores the frame.
    let boxes = [SelectionBox {
        rect: crate::layout::RectF {
            x: 40.0,
            y: 40.0,
            width: 50.0,
            height: 30.0,
        },
        color: crate::select::SELECTION_COLOR,
        stroke_width: crate::select::SELECTION_STROKE_WIDTH,
        handles: Vec::new(),
    }];
    refresh(&mut r, id, &mut view, &boxes);
    check(
        view.layer_raster_generations() == vec![3, 4, 4],
        format!(
            "selection re-rastered more than the top panel: {:?}",
            view.layer_raster_generations()
        ),
    );
    let selected = frame_with_view(&mut r, id, &view);
    check(selected != previous, "selection box drew nothing".into());
    refresh(&mut r, id, &mut view, &[]);
    check(
        frame_with_view(&mut r, id, &view) == previous,
        "clearing the selection did not restore the frame".into(),
    );
    drop(_fonts);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// One `set_chart_view_state` moves the shared X range and every overlay's Y
// range together: the config carries both after the one call, and the next
// frame equals a fresh render of that state. No frame can show the base
// moved and an overlay not, because every panel's transform is written from
// the one config snapshot in the same prepare.
#[test]
fn a_linked_range_edit_moves_every_layer_in_one_registry_call() {
    // Pixels are gathered under the font lock and compared after it is
    // released, so a failed assertion cannot poison the lock for other tests.
    let fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let config = two_overlay_config();
    let mut r = renderer();
    let id = r.register_chart(config.clone(), three_series()).unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    let before = frame_with_view(&mut r, id, &view);

    let mut state = r.chart_view_state(id).unwrap();
    let overlay_count = state.overlay_y.len();
    state.bottom_x.min = 0.25;
    state.bottom_x.max = 0.75;
    state.overlay_y[0].min = 20.0;
    state.overlay_y[0].max = 40.0;
    state.overlay_y[1].inverted = true;
    let applied = r.set_chart_view_state(id, state.clone());
    let edited = r.chart_config(id).unwrap().clone();
    let state_after = r.chart_view_state(id).unwrap();

    refresh(&mut r, id, &mut view, &[]);
    let after = frame_with_view(&mut r, id, &view);
    let fresh = fresh_frame(&edited, three_series());

    // A view state with the wrong overlay count cannot be applied at all.
    let mut short = state.clone();
    short.overlay_y.pop();
    let rejected = r.set_chart_view_state(id, short);
    let unchanged = r.chart_config(id).unwrap().clone();
    drop(fonts);

    assert_eq!(overlay_count, 2);
    applied.unwrap();
    assert_eq!(edited.bottom_x.min, 0.25);
    assert_eq!(edited.overlays[0].y_axis.min, 20.0);
    assert_eq!(edited.overlays[0].y_axis.max, 40.0);
    assert!(edited.overlays[1].y_axis.inverted);
    assert_eq!(state_after, state);
    assert_ne!(after, before);
    assert_eq!(
        after, fresh,
        "the linked edit must land on every layer in the same frame"
    );
    assert!(matches!(
        rejected,
        Err(FiggyError::InvalidConfig {
            field: "overlays",
            ..
        })
    ));
    assert_eq!(unchanged, edited);
}

#[test]
fn the_active_layer_api_round_trips_clears_selection_and_recovers_out_of_range() {
    let mut r = renderer();
    let config = two_overlay_config();
    let id = r.register_chart(config.clone(), three_series()).unwrap();
    assert_eq!(r.active_layer(id).unwrap(), LayerRef::Base);

    // A selection on the base layer's map, then a layer change: the hit id
    // belongs to the old map, so it is cleared and the chrome re-rasters.
    let hit = HitMap::for_active_layer(&config).find("data_area").unwrap();
    r.set_chart_selection(id, Some(hit)).unwrap();
    let before = r.chart_render_stamp(id).unwrap();
    assert_eq!(
        r.set_active_layer(id, LayerRef::Overlay(1)).unwrap(),
        LayerRef::Overlay(1)
    );
    assert_eq!(r.active_layer(id).unwrap(), LayerRef::Overlay(1));
    assert_eq!(
        r.chart_config(id).unwrap().active_layer,
        LayerRef::Overlay(1)
    );
    assert_eq!(r.chart_selection(id).unwrap(), None);
    let changed = r.chart_render_stamp(id).unwrap();
    assert!(changed.needs_draw_since(Some(&before)));
    assert!(changed.needs_raster_since(Some(&before)));

    // Setting the same layer again is not an edit.
    assert_eq!(
        r.set_active_layer(id, LayerRef::Overlay(1)).unwrap(),
        LayerRef::Overlay(1)
    );
    assert_eq!(r.chart_render_stamp(id).unwrap(), changed);

    // Without a selection a layer change draws (hit map rebuild) but keeps
    // the raster: pixels do not depend on the active layer.
    assert_eq!(
        r.set_active_layer(id, LayerRef::Annotations).unwrap(),
        LayerRef::Annotations
    );
    let annotations = r.chart_render_stamp(id).unwrap();
    assert!(annotations.needs_draw_since(Some(&changed)));
    assert!(!annotations.needs_raster_since(Some(&changed)));

    // Out of range: the setter stores Base.
    assert_eq!(
        r.set_active_layer(id, LayerRef::Overlay(2)).unwrap(),
        LayerRef::Base
    );
    assert_eq!(r.chart_config(id).unwrap().active_layer, LayerRef::Base);

    // Deleting the active overlay through a config replacement recovers to
    // Base, both in the answer and in the stored document.
    r.set_active_layer(id, LayerRef::Overlay(0)).unwrap();
    let mut without = r.chart_config(id).unwrap().clone();
    without.overlays.clear();
    r.set_chart_config(id, without).unwrap();
    assert_eq!(r.active_layer(id).unwrap(), LayerRef::Base);
    assert_eq!(r.chart_config(id).unwrap().active_layer, LayerRef::Base);

    // A config written with a dangling index is stored as Base too, through
    // every registry entry point.
    let mut dangling = config.clone();
    dangling.active_layer = LayerRef::Overlay(5);
    r.set_chart_state(id, dangling.clone(), three_series())
        .unwrap();
    assert_eq!(r.chart_config(id).unwrap().active_layer, LayerRef::Base);
    let other = r.register_chart(dangling, three_series()).unwrap();
    assert_eq!(r.chart_config(other).unwrap().active_layer, LayerRef::Base);

    // Overlay series must be resident; the base series may still stream.
    r.register_streamed_columns(vec![source("sx", 1), source("sy", 1)])
        .unwrap();
    let mut streamed = two_overlay_config();
    streamed.overlays[0].series = vec!["streamed".into()];
    let series = vec![declaration("streamed", "sx", "sy")];
    assert!(matches!(
        r.register_chart(streamed.clone(), series.clone()),
        Err(FiggyError::InvalidSeriesConfig { series_id, .. }) if series_id == "streamed"
    ));
    streamed.overlays[0].series.clear();
    let layered = r.register_chart(streamed.clone(), series.clone()).unwrap();
    let mut onto_overlay = streamed.clone();
    onto_overlay.overlays[1].series = vec!["streamed".into()];
    assert!(matches!(
        r.set_chart_config(layered, onto_overlay),
        Err(FiggyError::InvalidSeriesConfig { .. })
    ));
    // Nonresident streaming rejects the layered document as a whole.
    let view = r
        .create_chart_view(&Chart::new(streamed.clone()), PANEL)
        .unwrap();
    assert!(matches!(
        r.inspect_streaming_capability(layered),
        Err(FiggyError::InvalidConfig {
            field: "overlays",
            ..
        })
    ));
    assert!(matches!(
        r.request_auto_streaming_chart(
            layered,
            &view,
            crate::StreamingChartOptions {
                size: (320, 240),
                clear_color: Color::WHITE,
                max_primitives_per_chunk: 3,
            },
        ),
        Err(FiggyError::InvalidConfig {
            field: "overlays",
            ..
        })
    ));
}

// Picks address the active layer: its panel (the overlay's Y range, the
// group data area) and only the series it scales. The base answers for the
// annotation layer, which scales no series.
#[test]
fn pick_inputs_follow_the_active_layer() {
    let mut config = two_overlay_config();
    let series = three_series();
    let (panel, subset) = active_layer_pick_inputs(&config, &series).unwrap();
    let expansion = config.expand().unwrap();
    assert_eq!(panel, expansion.panels[0].config);
    assert_eq!(subset, vec![declaration("s0", "x", "a")]);

    config.set_active_layer(LayerRef::Overlay(1));
    let (panel, subset) = active_layer_pick_inputs(&config, &series).unwrap();
    assert_eq!(panel, expansion.panels[2].config);
    assert_eq!(panel.left_y.min, -1.5);
    assert_eq!(panel.left_y.max, 1.5);
    assert_eq!(panel.data_area().unwrap(), expansion.data_area);
    assert_eq!(subset, vec![declaration("s2", "x", "c")]);

    config.set_active_layer(LayerRef::Annotations);
    let (panel, subset) = active_layer_pick_inputs(&config, &series).unwrap();
    assert_eq!(panel, expansion.panels[0].config);
    assert_eq!(subset.len(), 1);

    // A document without overlays is its own panel.
    let plain = base_config();
    let (panel, subset) = active_layer_pick_inputs(&plain, &series).unwrap();
    assert_eq!(panel, plain);
    assert_eq!(subset, series);
}

// The document export composites every layer; the panel export keeps the
// base panel. For a document without overlays the two are byte-identical.
#[test]
fn document_export_composites_every_layer() {
    let fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut r = renderer();
    let series = three_series();
    let plain = Chart::new(base_config());
    let plain_document = r.export_document_rgba(&plain, &series, 1.0).unwrap();
    let plain_panel = r.export_panel_rgba(&plain, &series, 1.0).unwrap();
    let layered = Chart::new(two_overlay_config());
    let document = r.export_document_rgba(&layered, &series, 1.0).unwrap();
    let panel = r.export_panel_rgba(&layered, &series, 1.0).unwrap();
    let large = r.export_document_rgba(&layered, &series, 2.0).unwrap();
    let large_panel = r.export_panel_rgba(&layered, &series, 2.0).unwrap();
    let png = r.export_document_png_bytes(&layered, &series, 1.0).unwrap();
    drop(fonts);

    assert_eq!((plain_document.width, plain_document.height), (320, 240));
    assert_eq!(plain_document.rgba, plain_panel.rgba);
    assert_ne!(document.rgba, panel.rgba, "the overlays must be composited");
    // The overlay axes draw in the right band, which the base panel leaves
    // empty apart from its own short right axis.
    let band = |image: &RasterImage| -> usize {
        let da = two_overlay_config().data_area().unwrap().0;
        let x0 = (da.x + da.width + 8) as usize;
        (da.y as usize..(da.y + da.height) as usize)
            .flat_map(|y| (x0..320).map(move |x| (y * 320 + x) * 4 + 3))
            .filter(|&i| image.rgba[i] != 0)
            .count()
    };
    assert!(
        band(&document) > band(&panel),
        "no overlay ink in the right band"
    );
    // Scaling the document scales the composite; the overlays still show.
    assert_eq!((large.width, large.height), (640, 480));
    assert_ne!(large.rgba, large_panel.rgba);
    assert_eq!(&png[1..4], b"PNG");
}

// Every panel's tick warnings reach the view, overlay ones tagged with their
// layer, and the overlay's hidden copies of the base axes repeat nothing.
#[test]
fn render_warnings_are_merged_with_a_layer_tag() {
    let mut config = two_overlay_config();
    config.bottom_x.major_spacing = 0.0;
    config.overlays[1].y_axis.major_spacing = f64::NAN;
    let r = renderer();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    let axes: Vec<WarningAxis> = view.render_warnings().iter().map(|w| w.axis()).collect();
    assert_eq!(axes, vec![WarningAxis::BottomX, WarningAxis::OverlayY(1)]);
    assert_eq!(
        view.render_warnings()[1].to_string().split(' ').next(),
        Some("overlays[1].y_axis.major_spacing")
    );
    // A view created with fewer layers than the document cannot draw it.
    let mut r = r;
    let id = r.register_chart(config.clone(), three_series()).unwrap();
    let mut without = config.clone();
    without.overlays.pop();
    let mut short = r.create_chart_view(&Chart::new(without), PANEL).unwrap();
    assert!(matches!(
        r.prepare_registered(&[RegisteredChartDrawItem {
            chart_id: id,
            view: &short,
        }]),
        Err(FiggyError::StaleStateToken { .. })
    ));
    // A refresh grows the view to the document's layer count.
    r.refresh_axis(&mut short, &Chart::new(config), PANEL)
        .unwrap();
    assert_eq!(short.overlay_count(), 2);
    assert!(
        r.prepare_registered(&[RegisteredChartDrawItem {
            chart_id: id,
            view: &short,
        }])
        .is_ok()
    );
}
