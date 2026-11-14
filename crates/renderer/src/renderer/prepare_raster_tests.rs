//! `prepare` redraws a stale text and axis raster itself (#265): from the
//! chart's raster stamp and selection for a host-held chart, and from the
//! stored document, display placement and selection for a registered one.

use super::*;

fn renderer() -> Option<Renderer> {
    let (device, queue) = crate::data_render::shared_device()?;
    Some(
        Renderer::try_new(
            RendererDevice::new(device.clone(), queue.clone()).expect("figgy poll thread"),
            wgpu::TextureFormat::Rgba8Unorm,
            1024 * 1024,
        )
        .expect("required GPU test: renderer initialization failed"),
    )
}

fn config() -> Config {
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config
}

fn prepare(r: &mut Renderer, view: &mut ChartView, chart: &Chart) {
    drop(
        r.prepare(&mut [ChartDrawItem {
            view,
            chart,
            series: &[],
        }])
        .unwrap(),
    );
}

fn drawn_selection(view: &ChartView) -> usize {
    view.raster_key
        .as_ref()
        .map_or(0, |key| key.selection.len())
}

// A view made from the chart as it stands is kept; a changed chart is
// redrawn once, and only once.
#[test]
fn prepare_redraws_a_changed_chart_once() {
    let Some(mut r) = renderer() else {
        return;
    };
    let mut chart = Chart::new(config());
    let mut view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let made = view.layer_stamps();
    prepare(&mut r, &mut view, &chart);
    assert_eq!(view.layer_stamps(), made, "a fresh view is not redrawn");

    chart.set_x_range(0.0, 5.0);
    prepare(&mut r, &mut view, &chart);
    let redrawn = view.layer_stamps();
    assert!(
        redrawn.decoration > made.decoration,
        "{redrawn:?} after {made:?}"
    );
    prepare(&mut r, &mut view, &chart);
    assert_eq!(
        view.layer_stamps(),
        redrawn,
        "an unchanged chart is not redrawn again"
    );
}

// The chart's selection is drawn by prepare, and clearing it clears the
// box.
#[test]
fn prepare_draws_the_chart_selection() {
    let Some(mut r) = renderer() else {
        return;
    };
    let mut chart = Chart::new(config());
    let mut view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    assert_eq!(drawn_selection(&view), 0);
    chart.select(Some("axis_bottom"));
    assert_eq!(chart.selected(), Some("axis_bottom"));
    prepare(&mut r, &mut view, &chart);
    assert_eq!(drawn_selection(&view), 1, "the selected axis has its box");
    chart.select(None);
    prepare(&mut r, &mut view, &chart);
    assert_eq!(drawn_selection(&view), 0);
}

// A host that still refreshes by hand with the chart as prepare would draw
// it is not redrawn a second time; another view of the same chart, never
// drawn by prepare, is.
#[test]
fn a_hand_refresh_of_the_chart_as_it_stands_is_kept() {
    let Some(mut r) = renderer() else {
        return;
    };
    let mut chart = Chart::new(config());
    let rect = chart.config().chart_area.0;
    let mut view = r.create_chart_view(&chart, rect).unwrap();
    chart.set_y_range(-2.0, 2.0);
    r.refresh_axis(&mut view, &chart, rect).unwrap();
    let refreshed = view.layer_stamps();
    prepare(&mut r, &mut view, &chart);
    assert_eq!(view.layer_stamps(), refreshed);

    // A view another chart drew is redrawn for this one.
    let other = Chart::new(config());
    let mut borrowed = r.create_chart_view(&other, rect).unwrap();
    let before = view_source(&borrowed);
    prepare(&mut r, &mut borrowed, &chart);
    assert_ne!(
        view_source(&borrowed),
        before,
        "a stamp is never shared by two charts"
    );
}

fn view_source(view: &ChartView) -> Option<RasterChart> {
    view.raster_source.map(|source| source.chart)
}

// A registered chart's view follows the stored document and selection.
#[test]
fn prepare_registered_redraws_from_the_stored_chart() {
    let Some(mut r) = renderer() else {
        return;
    };
    let config = config();
    let rect = config.chart_area.0;
    let id = r.register_chart(config.clone(), vec![]).unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(config.clone()), rect)
        .unwrap();
    let prepare = |r: &mut Renderer, view: &mut ChartView| {
        drop(
            r.prepare_registered(&mut [RegisteredChartDrawItem { chart_id: id, view }])
                .unwrap(),
        );
    };
    prepare(&mut r, &mut view);
    let first = view.layer_stamps();
    prepare(&mut r, &mut view);
    assert_eq!(
        view.layer_stamps(),
        first,
        "an unchanged chart is not redrawn"
    );

    let axis = crate::select::HitMap::for_active_layer(&config)
        .find("axis_bottom")
        .expect("the bottom axis is selectable");
    r.set_chart_selection(id, Some(axis)).unwrap();
    prepare(&mut r, &mut view);
    assert_eq!(drawn_selection(&view), 1, "the stored selection is drawn");

    let mut edited = r.chart_config(id).unwrap().clone();
    edited.bottom_x.max = 7.0;
    r.set_chart_config(id, edited).unwrap();
    let before = view.layer_stamps();
    prepare(&mut r, &mut view);
    assert!(view.layer_stamps().decoration > before.decoration);
}

// An annotation edit of a registered chart redraws the annotation layer
// alone; the panel rasters keep their textures.
#[test]
fn prepare_registered_redraws_an_annotation_edit_alone() {
    let Some(mut r) = renderer() else {
        return;
    };
    let config = config();
    let rect = config.chart_area.0;
    let id = r.register_chart(config.clone(), vec![]).unwrap();
    let mut view = r.create_chart_view(&Chart::new(config), rect).unwrap();
    let prepare = |r: &mut Renderer, view: &mut ChartView| {
        drop(
            r.prepare_registered(&mut [RegisteredChartDrawItem { chart_id: id, view }])
                .unwrap(),
        );
    };
    prepare(&mut r, &mut view);
    let first = view.layer_stamps();

    let mut edited = r.chart_config(id).unwrap().clone();
    edited
        .annotations
        .items
        .push(crate::annotation::Annotation::new(
            crate::annotation::AnnotationKind::Span {
                axis: crate::annotation::AnnotationAxis::X,
                from: 0.2,
                to: 0.4,
                fill: Color::new(1.0, 0.0, 0.0, 0.5),
            },
        ));
    r.set_chart_config(id, edited).unwrap();
    prepare(&mut r, &mut view);
    assert_eq!(
        view.layer_stamps(),
        ChartViewLayerStamps {
            annotations: first.annotations + 1,
            ..first
        }
    );
    prepare(&mut r, &mut view);
    assert_eq!(view.layer_stamps().annotations, first.annotations + 1);
}
