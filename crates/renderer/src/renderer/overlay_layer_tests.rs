//! Overlay layers through the registry: per-layer raster caches, the atomic
//! linked-range edit, the active-layer API, streaming rejection, document
//! export and the pick inputs. The pixel oracle is always a fresh renderer
//! given the final state, never the implementation's own invalidation rule.
use super::*;
use crate::layout::{ChartArea, Rect};
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

/// Refresh every panel of `id` with the element `selected` names selected
/// (`None` clears the selection).
fn refresh(r: &mut Renderer, id: ChartId, view: &mut ChartView, selected: Option<&str>) {
    let config = r.chart_config(id).unwrap().clone();
    r.refresh_axis_with_selection(view, &Chart::new(config), PANEL, selected)
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
    refresh(&mut r, id, &mut view, None);
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
        refresh(&mut r, id, &mut view, None);
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
    // The data area is shared by every layer; its box is routed to the
    // topmost panel.
    let config = r.chart_config(id).unwrap().clone();
    check(
        r.selection_box(&Chart::new(config), "data_area").is_some(),
        "the data area has no selection box".into(),
    );
    refresh(&mut r, id, &mut view, Some("data_area"));
    check(
        view.layer_raster_generations() == vec![3, 4, 4],
        format!(
            "selection re-rastered more than the top panel: {:?}",
            view.layer_raster_generations()
        ),
    );
    let selected = frame_with_view(&mut r, id, &view);
    check(selected != previous, "selection box drew nothing".into());
    refresh(&mut r, id, &mut view, None);
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

    refresh(&mut r, id, &mut view, None);
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
    let LayerPickInputs {
        panel,
        series: subset,
        ..
    } = active_layer_pick_inputs(&config, &series).unwrap();
    let expansion = config.expand().unwrap();
    assert_eq!(panel, expansion.panels[0].config);
    assert_eq!(subset, vec![declaration("s0", "x", "a")]);

    config.set_active_layer(LayerRef::Overlay(1));
    let LayerPickInputs {
        panel,
        series: subset,
        ..
    } = active_layer_pick_inputs(&config, &series).unwrap();
    assert_eq!(panel, expansion.panels[2].config);
    assert_eq!(panel.left_y.min, -1.5);
    assert_eq!(panel.left_y.max, 1.5);
    assert_eq!(panel.data_area().unwrap(), expansion.data_area);
    assert_eq!(subset, vec![declaration("s2", "x", "c")]);

    config.set_active_layer(LayerRef::Annotations);
    let LayerPickInputs {
        panel,
        series: subset,
        ..
    } = active_layer_pick_inputs(&config, &series).unwrap();
    assert_eq!(panel, expansion.panels[0].config);
    assert_eq!(subset.len(), 1);

    // A document without overlays is its own panel.
    let plain = base_config();
    let LayerPickInputs {
        panel,
        series: subset,
        ..
    } = active_layer_pick_inputs(&plain, &series).unwrap();
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

// An adaptive log axis (#42) re-plans its ticks on every zoom with no help
// from the host: `Chart::set_y_range` and `set_chart_view_state` rewrite the
// range and keep the policy, on the base axes and an overlay's Y axis, the
// zoom 3e3..8e5 -> 1e4..8e4 -> 1e4..1e8 walks the 1-2-5, 1..9 and decade
// ladders, every frame is a fresh render of its state, and the document
// exports at 1x and 2x with the adaptive chrome.
#[test]
fn an_adaptive_log_axis_replans_on_every_zoom() {
    use crate::config::{AxisScale, LogTickPolicy};
    let fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut config = base_config();
    for axis in [&mut config.left_y, &mut config.right_y] {
        axis.scale = AxisScale::Logarithmic;
        axis.log_ticks = LogTickPolicy::Adaptive;
    }
    let mut layer = overlay(3e3, 8e5, "s1", 40.0);
    layer.y_axis.scale = AxisScale::Logarithmic;
    layer.y_axis.log_ticks = LogTickPolicy::Adaptive;
    config.overlays.push(layer);
    let series = vec![declaration("s0", "x", "a"), declaration("s1", "x", "b")];
    let mut chart = Chart::new(config);
    chart.set_y_range(3e3, 8e5);
    chart.set_overlay_y_range(0, 3e3, 8e5);
    let mut r = renderer();
    let id = r
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(chart.config().clone()), PANEL)
        .unwrap();
    let majors =
        |axis: &crate::config::AxisOptions| crate::tick::major_ticks(axis).collect::<Vec<_>>();
    let mut failures = Vec::new();
    let mut frames = Vec::new();
    for (min, max, expected) in [
        (3e3, 8e5, vec![5e3, 1e4, 2e4, 5e4, 1e5, 2e5, 5e5]),
        (1e4, 8e4, vec![1e4, 2e4, 3e4, 4e4, 5e4, 6e4, 7e4, 8e4]),
        (1e4, 1e8, vec![1e4, 1e5, 1e6, 1e7, 1e8]),
    ] {
        chart.set_y_range(min, max);
        let edited = chart.config().clone();
        for axis in [&edited.left_y, &edited.right_y] {
            if axis.log_ticks != LogTickPolicy::Adaptive || majors(axis) != expected {
                failures.push(format!("set_y_range {min:e}..{max:e}: {:?}", majors(axis)));
            }
        }
        r.set_chart_config(id, edited.clone()).unwrap();
        refresh(&mut r, id, &mut view, None);
        let frame = frame_with_view(&mut r, id, &view);
        if frame != fresh_frame(&edited, series.clone()) {
            failures.push(format!("set_y_range {min:e}..{max:e}: not a fresh render"));
        }
        frames.push(frame);

        // The same zoom through the view state moves the overlay too.
        let mut state = r.chart_view_state(id).unwrap();
        state.overlay_y[0].min = min;
        state.overlay_y[0].max = max;
        r.set_chart_view_state(id, state).unwrap();
        let viewed = r.chart_config(id).unwrap().clone();
        let axis = &viewed.overlays[0].y_axis;
        if axis.log_ticks != LogTickPolicy::Adaptive || majors(axis) != expected {
            failures.push(format!("view state {min:e}..{max:e}: {:?}", majors(axis)));
        }
        if viewed.left_y.log_ticks != LogTickPolicy::Adaptive || majors(&viewed.left_y) != expected
        {
            failures.push(format!("view state {min:e}..{max:e}: left_y"));
        }
        refresh(&mut r, id, &mut view, None);
        if frame_with_view(&mut r, id, &view) != fresh_frame(&viewed, series.clone()) {
            failures.push(format!("view state {min:e}..{max:e}: not a fresh render"));
        }
        chart = Chart::new(viewed);
    }
    if frames[0] == frames[1] || frames[1] == frames[2] {
        failures.push("a zoom left the frame unchanged".into());
    }

    // PNG export at 1x and 2x draws the adaptive chrome: inside one
    // decade, where the decade ladder draws a single tick.
    let mut adaptive = Chart::new(r.chart_config(id).unwrap().clone());
    adaptive.set_y_range(1e4, 8e4);
    let mut decades_config = adaptive.config().clone();
    for axis in [&mut decades_config.left_y, &mut decades_config.right_y] {
        axis.log_ticks = LogTickPolicy::Decades;
    }
    let decades = Chart::new(decades_config);
    let mut exports = Vec::new();
    for scale in [1.0f32, 2.0] {
        let image = r.export_document_rgba(&adaptive, &series, scale).unwrap();
        let other = r.export_document_rgba(&decades, &series, scale).unwrap();
        let png = r
            .export_document_png_bytes(&adaptive, &series, scale)
            .unwrap();
        exports.push((scale, image.width, image.rgba != other.rgba, png));
    }
    drop(fonts);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    for (scale, width, differs, png) in exports {
        assert_eq!(width, (320.0 * scale) as u32);
        assert!(
            differs,
            "{scale}x: the adaptive chrome is not in the export"
        );
        assert_eq!(&png[1..4], b"PNG", "{scale}x");
    }
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
    let axes: Vec<WarningAxis> = view
        .render_warnings()
        .iter()
        .filter_map(|w| w.axis())
        .collect();
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

// Offset spectra (`Config::line_offsets`, #50): each member is a panel of its
// own. The pixel oracle is the same picture drawn by a host that shifted the
// data itself: columns `a + k * step` on a document without line offsets.

const OFFSET_STEP: f64 = 0.25;

/// `id` drawing `y` over `x` as a 2 px line of `color`.
fn colored(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
    let mut series = declaration(id, x, y);
    if let DataRenderType::Line { line } = &mut series.render_type {
        line.line_color = color;
    }
    series
}

const MEMBER_COLORS: [Color; 3] = [
    Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    },
    Color {
        r: 0.0,
        g: 0.6,
        b: 0.0,
        a: 1.0,
    },
    Color {
        r: 0.6,
        g: 0.0,
        b: 0.6,
        a: 1.0,
    },
];

/// The base series `s0` and the members `m0`, `m1`, `m2`, all drawing `a`;
/// with `host_shifted` the members draw the host-shifted columns `a1`, `a2`.
fn offset_series(host_shifted: bool) -> Vec<SeriesConfig> {
    let mut series = vec![colored("s0", "x", "c", Color::BLACK)];
    for (k, color) in MEMBER_COLORS.into_iter().enumerate() {
        let column = match (host_shifted, k) {
            (true, 1) => "a1",
            (true, 2) => "a2",
            _ => "a",
        };
        series.push(colored(&format!("m{k}"), "x", column, color));
    }
    series
}

/// `base_config` with `m0`, `m1`, `m2` stacked `OFFSET_STEP` apart.
fn offset_config() -> Config {
    let mut config = base_config();
    config.line_offsets = Some(crate::data_config::LineOffsets {
        series: vec!["m0".into(), "m1".into(), "m2".into()],
        step_y: OFFSET_STEP,
        step_x: 0.0,
    });
    config
}

/// A renderer with the shared columns plus the host-shifted `a1`, `a2`.
fn offset_renderer() -> Renderer {
    let mut r = renderer();
    let a = [0.2f32, 0.6, 0.3, 0.8, 0.4];
    for k in 1..=2 {
        let shifted = a
            .iter()
            .map(|v| v + k as f32 * OFFSET_STEP as f32)
            .collect();
        r.add_column(&format!("a{k}"), &column(shifted)).unwrap();
    }
    r.enable_gpu_picking().unwrap();
    r
}

/// The largest channel difference between two images and how many pixels
/// differ at all.
fn image_difference(a: &RasterImage, b: &RasterImage) -> (u8, usize) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut largest = 0;
    let mut count = 0;
    for (p, q) in a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)) {
        let d = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        largest = largest.max(d);
        count += usize::from(d > 0);
    }
    (largest, count)
}

/// Data `(x, y)` of `config`'s base axes to a canvas pixel at `scale`.
fn offset_px(config: &Config, x: f64, y: f64, scale: f32) -> [f32; 2] {
    let scaled = config.scaled(scale);
    let da = scaled.data_area().unwrap().0;
    let tx = (x - scaled.bottom_x.min) / (scaled.bottom_x.max - scaled.bottom_x.min);
    let ty = (y - scaled.left_y.min) / (scaled.left_y.max - scaled.left_y.min);
    [
        (f64::from(da.x) + tx * f64::from(da.width)) as f32,
        (f64::from(da.y + da.height) - ty * f64::from(da.height)) as f32,
    ]
}

fn offset_pick(
    r: &mut Renderer,
    id: ChartId,
    at: [f32; 2],
    max_distance_px: f32,
) -> Option<crate::PickedPoint> {
    let request = GpuPickRequest {
        canvas_position_px: at,
        display_panel_px: PANEL,
        display_scale: 1.0,
        max_distance_px,
    };
    pollster::block_on(r.pick_chart(id, request).unwrap().resolve()).unwrap()
}

// The members draw exactly where a host-shifted copy draws, at 1x and 2x,
// through the document export and its PNG; the shift is not in the base
// panel's decorations.
#[test]
fn offset_lines_export_at_1x_and_2x() {
    let fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut r = offset_renderer();
    let offset = Chart::new(offset_config());
    let host = Chart::new(base_config());
    let mut results = Vec::new();
    for scale in [1.0f32, 2.0] {
        let drawn = r
            .export_document_rgba(&offset, &offset_series(false), scale)
            .unwrap();
        let expected = r
            .export_document_rgba(&host, &offset_series(true), scale)
            .unwrap();
        let unshifted = r
            .export_document_rgba(&host, &offset_series(false), scale)
            .unwrap();
        let png = r
            .export_document_png_bytes(&offset, &offset_series(false), scale)
            .unwrap();
        results.push((scale, drawn, expected, unshifted, png));
    }
    drop(fonts);
    for (scale, drawn, expected, unshifted, png) in results {
        assert_eq!(drawn.width, (320.0 * scale) as u32, "{scale}x");
        let (largest, count) = image_difference(&drawn, &expected);
        assert!(
            largest <= 2,
            "{scale}x: {count} pixels differ from the host-shifted picture, by up to {largest}"
        );
        assert_ne!(drawn.rgba, unshifted.rgba, "{scale}x: nothing is shifted");
        assert_eq!(&png[1..4], b"PNG", "{scale}x");
    }
}

// The expansion: one panel per member after the overlays, each the base
// panel with its range moved by the shift and nothing but its data drawn;
// the view holds a panel view per member, and a member's tick warnings are
// not repeated.
#[test]
fn offset_members_are_panels_of_their_own() {
    let config = offset_config();
    let panels = expand_for_render(&config).unwrap();
    assert_eq!(panels.len(), 4);
    assert_eq!(
        config.panel_layers(),
        vec![
            LayerRef::Base,
            LayerRef::Base,
            LayerRef::Base,
            LayerRef::Base
        ]
    );
    for (k, panel) in panels.iter().enumerate().skip(1) {
        let shift = (k - 1) as f64 * OFFSET_STEP;
        assert_eq!(config.panel_member(k), Some(format!("m{}", k - 1).as_str()));
        assert_eq!(panel.left_y.min, -shift);
        assert_eq!(panel.left_y.max, 1.0 - shift);
        assert_eq!(panel.data_area().unwrap(), panels[0].data_area().unwrap());
        assert!(!panel.legend.visible && !panel.chart_title.visible);
        assert!(!panel.left_y.line_visible && !panel.bottom_x.label_style.visible);
    }
    assert_eq!(config.panel_member(0), None);
    assert_eq!(config.panel_of_series("s0"), 0);
    assert_eq!(config.panel_of_series("m2"), 3);
    assert_eq!(panels[0].line_offsets, None);
    let r = renderer();
    let view = r.create_chart_view(&Chart::new(config), PANEL).unwrap();
    assert_eq!(view.overlay_count(), 3);
    assert_eq!(view.layer_raster_generations(), vec![1, 1, 1, 1]);
}

// A member an overlay scales moves on that overlay's axis: its panel is the
// overlay's with the overlay range shifted, and it draws where a host-shifted
// copy on the overlay draws; it picks there while the overlay is active.
#[test]
fn offset_members_shift_on_their_overlay_axis() {
    let fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut r = offset_renderer();
    let b = [10.0f32, 30.0, 20.0, 50.0, 40.0];
    r.add_column("b10", &column(b.iter().map(|v| v + 10.0).collect()))
        .unwrap();
    let layered = |members: bool| {
        let mut config = base_config();
        config.overlays.push(overlay(0.0, 80.0, "s1", 40.0));
        config.overlays[0].series.push("t1".into());
        if members {
            config.line_offsets = Some(crate::data_config::LineOffsets {
                series: vec!["s1".into(), "t1".into()],
                step_y: 10.0,
                step_x: 0.0,
            });
        }
        config
    };
    let series = |host: bool| {
        vec![
            colored("s0", "x", "a", Color::BLACK),
            colored("s1", "x", "b", MEMBER_COLORS[0]),
            colored("t1", "x", if host { "b10" } else { "b" }, MEMBER_COLORS[1]),
        ]
    };
    let config = layered(true);
    let panels = expand_for_render(&config).unwrap();
    assert_eq!(
        config.panel_layers(),
        vec![
            LayerRef::Base,
            LayerRef::Overlay(0),
            LayerRef::Overlay(0),
            LayerRef::Overlay(0)
        ]
    );
    assert_eq!(panels[3].left_y.min, -10.0);
    assert_eq!(panels[3].left_y.max, 70.0);
    assert!(!panels[3].right_y.line_visible);
    let drawn = r
        .export_document_rgba(&Chart::new(config.clone()), &series(false), 1.0)
        .unwrap();
    let expected = r
        .export_document_rgba(&Chart::new(layered(false)), &series(true), 1.0)
        .unwrap();
    drop(fonts);
    let (largest, count) = image_difference(&drawn, &expected);
    assert!(largest <= 2, "{count} pixels differ by up to {largest}");

    let mut active = config.clone();
    active.set_active_layer(LayerRef::Overlay(0));
    let id = r.register_chart(active.clone(), series(false)).unwrap();
    // Point 3 of `t1` (50) is drawn at 60 on the overlay's 0..80 axis.
    let da = active.data_area().unwrap().0;
    let at = [
        da.x as f32 + 0.7 * da.width as f32,
        (da.y + da.height) as f32 - 60.0 / 80.0 * da.height as f32,
    ];
    let hit = offset_pick(&mut r, id, at, 3.0).expect("the shifted point picks");
    assert_eq!((hit.series_id.as_str(), hit.point_index), ("t1", 3));
}

// The picked ring of a member is drawn around its shifted point, exactly
// where the host-shifted copy's ring is.
#[test]
fn offset_line_ring_sits_on_the_shifted_point() {
    let fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut r = offset_renderer();
    let select = |mut config: Config| {
        config.picked_points = Some(crate::config::PickedPointsConfig {
            visible: true,
            refs: vec![crate::config::PickedPointRef {
                source_id: None,
                series_id: "m2".into(),
                point_index: 2,
            }],
            ..Default::default()
        });
        config
    };
    let drawn = r
        .export_document_rgba(
            &Chart::new(select(offset_config())),
            &offset_series(false),
            1.0,
        )
        .unwrap();
    let unselected = r
        .export_document_rgba(&Chart::new(offset_config()), &offset_series(false), 1.0)
        .unwrap();
    let expected = r
        .export_document_rgba(
            &Chart::new(select(base_config())),
            &offset_series(true),
            1.0,
        )
        .unwrap();
    drop(fonts);
    let (largest, count) = image_difference(&drawn, &expected);
    assert!(largest <= 2, "{count} pixels differ by up to {largest}");
    // The ring's ink is centred on point 2 of `m2` shifted by two steps.
    let [cx, cy] = offset_px(&offset_config(), 0.5, 0.3 + 2.0 * OFFSET_STEP, 1.0);
    let (mut sx, mut sy, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for y in 0..drawn.height {
        for x in 0..drawn.width {
            let i = ((y * drawn.width + x) * 4) as usize;
            if drawn.rgba[i..i + 4] != unselected.rgba[i..i + 4] {
                sx += f64::from(x) + 0.5;
                sy += f64::from(y) + 0.5;
                n += 1.0;
            }
        }
    }
    assert!(n > 0.0, "no ring is drawn");
    assert!(
        (sx / n - f64::from(cx)).abs() < 2.0 && (sy / n - f64::from(cy)).abs() < 2.0,
        "the ring is centred at ({}, {}), not ({cx}, {cy})",
        sx / n,
        sy / n
    );
}

// A pick answers with what is painted under the cursor (#50 D7): the last
// member painted there (a hit at distance 0) first, since the member panels
// composite in `line_offsets.series` order; with no member painted there,
// the nearest hit within reach of any series, members and base alike.
#[test]
fn offset_picks_take_the_painted_member_first() {
    let mut r = offset_renderer();
    let mut config = offset_config();
    // Four pixels between neighbouring members at this panel height; the
    // lines are 2 px wide.
    let da = config.data_area().unwrap().0;
    let px = 1.0 / f64::from(da.height);
    config.line_offsets.as_mut().unwrap().step_y = 4.0 * px;
    // `s0` draws `a` too, under `m0`.
    let mut series = offset_series(false);
    series[0] = colored("s0", "x", "a", Color::BLACK);
    let id = r.register_chart(config.clone(), series).unwrap();
    // Exactly on point 1 of `m0` (and of `s0`), with `m1` and `m2` in reach:
    // `m0` is painted there and on top of `s0`.
    let at = offset_px(&config, 0.3, 0.6, 1.0);
    let on = offset_pick(&mut r, id, at, 10.0).unwrap();
    assert_eq!((on.series_id.as_str(), on.point_index), ("m0", 1));
    assert_eq!(on.distance_px, 0.0, "the cursor is on m0's line");
    let near = offset_pick(&mut r, id, at, 1.0).unwrap();
    assert_eq!(near.series_id, "m0", "only the bottom member is in reach");
    // Between `m0` and `m1`, 2.5 px above `m0`'s peak and 1.5 px below
    // `m1`'s, painted by neither: the nearest member answers, not the top.
    let gap = offset_pick(
        &mut r,
        id,
        offset_px(&config, 0.3, 0.6 + 2.5 * px, 1.0),
        10.0,
    )
    .unwrap();
    assert_eq!(
        gap.series_id, "m1",
        "the nearest member answers off the ink"
    );
    // Without members in reach the base panel answers.
    let mut lifted = config.clone();
    lifted.line_offsets.as_mut().unwrap().series = vec!["m1".into(), "m2".into(), "m0".into()];
    lifted.line_offsets.as_mut().unwrap().step_y = 0.0;
    r.set_chart_config(id, lifted).unwrap();
    let stacked = offset_pick(&mut r, id, at, 1.0).unwrap();
    assert_eq!(
        stacked.series_id, "m0",
        "where every member is painted, the last one paints on top"
    );
    let mut alone = config;
    alone.line_offsets.as_mut().unwrap().series = vec!["m1".into(), "m2".into()];
    alone.line_offsets.as_mut().unwrap().step_y = 0.5;
    let series = vec![
        colored("s0", "x", "a", Color::BLACK),
        colored("m1", "x", "c", MEMBER_COLORS[1]),
        colored("m2", "x", "a", MEMBER_COLORS[2]),
    ];
    r.set_chart_state(id, alone, series).unwrap();
    let base = offset_pick(&mut r, id, at, 1.0).unwrap();
    assert_eq!(base.series_id, "s0");
}

// A pan moves the members with the base range: the pick still finds each
// member where the panned frame draws it.
#[test]
fn pan_keeps_offset_picks_on_the_drawn_trace() {
    let mut r = offset_renderer();
    let id = r
        .register_chart(offset_config(), offset_series(false))
        .unwrap();
    let mut panned = Chart::new(offset_config());
    panned.set_x_range(0.2, 1.2);
    panned.set_y_range(0.3, 1.3);
    r.set_chart_config(id, panned.config().clone()).unwrap();
    let config = r.chart_config(id).unwrap().clone();
    for (k, (x, y, index)) in [(0.5, 0.3, 2), (0.7, 0.8, 3), (0.9, 0.4, 4)]
        .into_iter()
        .enumerate()
    {
        let at = offset_px(&config, x, y + k as f64 * OFFSET_STEP, 1.0);
        let hit = offset_pick(&mut r, id, at, 2.0).expect("the panned trace picks");
        assert_eq!(
            (hit.series_id.as_str(), hit.point_index),
            (format!("m{k}").as_str(), index)
        );
    }
}

// The base fit covers every member where it is drawn: its extent plus its
// shift, a factor of ten per decade on a logarithmic axis.
#[test]
fn auto_fit_includes_member_offsets() {
    let r = offset_renderer();
    let mut chart = Chart::new(offset_config());
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &offset_series(false), 0.0)
        .unwrap();
    // `s0` draws `c` (-1..1); `m2` draws `a` (0.2..0.8) two steps up.
    assert_eq!(chart.config().left_y.min, -1.0);
    assert!((chart.config().left_y.max - (0.8f32 as f64 + 2.0 * OFFSET_STEP)).abs() < 1e-9);
    let mut config = offset_config();
    config.left_y.scale = crate::config::AxisScale::Logarithmic;
    config.left_y.min = 0.1;
    config.line_offsets.as_mut().unwrap().step_y = 1.0;
    let mut log = Chart::new(config);
    let members = &offset_series(false)[1..];
    log.auto_fit_layer_union(r.pool(), LayerRef::Base, members, 0.0)
        .unwrap();
    let (min, max) = (log.config().left_y.min, log.config().left_y.max);
    assert!(min <= 0.2 + 1e-6 && max >= 80.0 - 1e-4, "{min}..{max}");
}

// A line offset member must be resident (rule 8), and nonresident streaming
// rejects a document with members as a whole.
#[test]
fn streamed_series_reject_line_offsets() {
    let mut r = renderer();
    r.register_streamed_columns(vec![source("sx", 1), source("sy", 1)])
        .unwrap();
    let mut config = base_config();
    config.line_offsets = Some(crate::data_config::LineOffsets {
        series: vec!["streamed".into()],
        step_y: 0.1,
        step_x: 0.0,
    });
    let series = vec![declaration("streamed", "sx", "sy")];
    assert!(matches!(
        r.register_chart(config.clone(), series.clone()),
        Err(FiggyError::InvalidSeriesConfig { series_id, .. }) if series_id == "streamed"
    ));
    let plain = r.register_chart(base_config(), series.clone()).unwrap();
    assert!(matches!(
        r.set_chart_config(plain, config.clone()),
        Err(FiggyError::InvalidSeriesConfig { .. })
    ));
    let resident = r
        .register_chart(offset_config(), offset_series(false))
        .unwrap();
    let view = r
        .create_chart_view(&Chart::new(offset_config()), PANEL)
        .unwrap();
    assert!(matches!(
        r.inspect_streaming_capability(resident),
        Err(FiggyError::InvalidConfig {
            field: "line_offsets",
            ..
        })
    ));
    assert!(matches!(
        r.request_auto_streaming_chart(
            resident,
            &view,
            crate::StreamingChartOptions {
                size: (320, 240),
                clear_color: Color::WHITE,
                max_primitives_per_chunk: 3,
            },
        ),
        Err(FiggyError::InvalidConfig {
            field: "line_offsets",
            ..
        })
    ));
}

// A polar chart has no line offsets, and the registry rejects members it
// cannot draw: an unknown id, a series without a line, a duplicate, a
// non-finite step and a fill between a member and another series.
#[test]
fn polar_rejects_line_offsets() {
    let mut r = renderer();
    let mut polar = offset_config();
    polar.polar = Some(crate::polar::PolarOptions::default());
    assert!(matches!(
        r.register_chart(polar, offset_series(false)),
        Err(FiggyError::InvalidConfig { .. })
    ));
    let rejected = |r: &mut Renderer, edit: fn(&mut Config, &mut Vec<SeriesConfig>)| {
        let mut config = offset_config();
        let mut series = offset_series(false);
        edit(&mut config, &mut series);
        r.register_chart(config, series).is_err()
    };
    assert!(rejected(&mut r, |c, _| {
        c.line_offsets.as_mut().unwrap().series.push("nope".into())
    }));
    assert!(rejected(&mut r, |c, _| {
        c.line_offsets.as_mut().unwrap().series.push("m0".into())
    }));
    assert!(rejected(&mut r, |c, _| {
        c.line_offsets.as_mut().unwrap().step_x = f64::NAN
    }));
    assert!(rejected(&mut r, |_, s| {
        s[1].render_type = DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_shape: ScatterShape::Circle,
                point_size: 4.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
        }
    }));
    assert!(rejected(&mut r, |_, s| {
        if let DataRenderType::Line { line } = &mut s[0].render_type {
            line.fill = Some(crate::data_config::LineFill::ToSeries("m1".into()));
        }
    }));
    assert!(!rejected(&mut r, |_, _| {}));
}
