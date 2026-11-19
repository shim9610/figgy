//! The layout fit of the first draw and drawing as stored (#357): a chart
//! whose layout waits for its fit (`LayoutState::NeedsFit`) is fitted once
//! by its first default draw or export, the fit is written into its config
//! (the SSOT says what is drawn), and the `as_stored` calls draw the SSOT
//! exactly as it is. An edit of a fitted layout fits again; a user's drag
//! fixes the layout, and no later draw undoes it. Older documents read as
//! `Fixed` and draw as they did (`released_documents*.rs` pin them).
#![cfg(not(target_arch = "wasm32"))]

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use renderer::layout::{Side, auto_layout};
use renderer::select::AxisTitleElement;
use renderer::{
    Chart, ChartDrawItem, Config, CpuTextMeasure, Draggable, ExportOptions, ExportSource,
    LayoutState, RasterImage, RegisteredChartDrawItem, Renderer, RendererDevice, SeriesConfig,
};

const POOL_BYTES: u64 = 16 * 1024 * 1024;

fn renderer() -> Renderer {
    let (device, queue) = test_gpu::device();
    let gpu = RendererDevice::new(device, queue).expect("figgy poll thread");
    Renderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm, POOL_BYTES).expect("renderer")
}

/// The audit's chart (#357): 800 × 600, a title, axis titles and y tick
/// labels like `-0.600`, never fitted by hand. Its columns are in `r`.
fn audit_chart(r: &mut Renderer) -> (Chart, [SeriesConfig; 1]) {
    let x: Vec<f64> = (0..=100).map(|i| f64::from(i) / 10.0).collect();
    let y: Vec<f64> = x.iter().map(|t| 0.7 * (t * 1.3).cos() + 0.05).collect();
    let _ = r.remove_column("x");
    let _ = r.remove_column("y");
    r.add_column("x", &x).unwrap();
    r.add_column("y", &y).unwrap();
    let mut chart = Chart::sized(800, 600)
        .with_title("Damped oscillation")
        .with_x_title("Time (s)")
        .with_y_title("Amplitude (V)");
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-0.6, 0.8);
    (chart, [SeriesConfig::line("signal", "x", "y")])
}

fn fitted(config: &Config) -> Config {
    auto_layout(config, &CpuTextMeasure::for_style(&config.draw_style))
}

fn export(
    r: &mut Renderer,
    chart: &mut Chart,
    series: &[SeriesConfig],
    options: ExportOptions,
) -> RasterImage {
    r.export_rgba(ExportSource::chart(chart, series), options)
        .block_on()
        .unwrap()
}

/// The left axis title's right edge and the tick labels' left edge: the
/// overlap the audit saw is a title that reaches into the labels.
fn title_clears_labels(config: &Config) -> bool {
    use renderer::layout::{Element, RectF};
    use renderer::sink::Parts;
    let rects = |element: Element| {
        let mut out: Vec<RectF> = Vec::new();
        renderer::select::on_drawn_panel(config, |panel| {
            renderer::select::emit_element(
                panel,
                &element,
                &CpuTextMeasure::for_style(&config.draw_style),
                &mut Parts(&mut |part| out.push(part.extent())),
            );
        });
        out
    };
    let title = rects(Element::AxisTitle(Side::Left));
    let labels = rects(Element::AxisLabel(Side::Left));
    let title_right = title.iter().map(|r| r.x + r.width).fold(f32::MIN, f32::max);
    let labels_left = labels.iter().map(|r| r.x).fold(f32::MAX, f32::min);
    title_right <= labels_left
}

#[test]
fn a_new_chart_overlaps_as_stored_and_its_first_export_fits_it_once() {
    let mut r = renderer();
    let (mut chart, series) = audit_chart(&mut r);
    let stored = chart.config().clone();
    assert_eq!(stored.layout_state, LayoutState::NeedsFit);
    assert!(
        !title_clears_labels(&stored),
        "the default margins put the y title over the -0.600 labels"
    );

    // As stored: the SSOT is drawn as it is and nothing is written.
    let as_stored = export(
        &mut r,
        &mut chart,
        &series,
        ExportOptions::panel(1.0).as_stored(),
    );
    assert_eq!(chart.config(), &stored, "as stored writes nothing");

    // The default export fits once and writes the fit into the chart.
    let first = export(&mut r, &mut chart, &series, ExportOptions::panel(1.0));
    let expected = fitted(&stored);
    assert_eq!(expected.layout_state, LayoutState::Fitted);
    assert_eq!(
        chart.config(),
        &expected,
        "the stored config says what is drawn"
    );
    assert!(title_clears_labels(chart.config()));
    assert_ne!(first.rgba, as_stored.rgba, "the fit moved the bands");

    // The fitted SSOT drawn as stored is the same image: no hidden layout.
    let again = export(
        &mut r,
        &mut chart,
        &series,
        ExportOptions::panel(1.0).as_stored(),
    );
    assert_eq!(again.rgba, first.rgba);
    // A second default export fits nothing more.
    let second = export(&mut r, &mut chart, &series, ExportOptions::panel(1.0));
    assert_eq!(chart.config(), &expected);
    assert_eq!(second.rgba, first.rgba);
}

#[test]
fn a_two_x_export_fits_the_same_layout_as_a_one_x_export() {
    let mut r = renderer();
    let (mut pending, series) = audit_chart(&mut r);
    let mut by_hand = Chart::new(pending.config().clone());
    by_hand.fit_layout();
    for scale in [1.0, 2.0] {
        let mut chart = Chart::new(pending.config().clone());
        let auto = export(&mut r, &mut chart, &series, ExportOptions::panel(scale));
        let manual = export(&mut r, &mut by_hand, &series, ExportOptions::panel(scale));
        assert_eq!(auto.rgba, manual.rgba, "{scale}x: the fit is the hand fit");
        assert_eq!(chart.config(), by_hand.config(), "{scale}x");
    }
    let _ = export(&mut r, &mut pending, &series, ExportOptions::document(2.0));
    assert_eq!(
        pending.config(),
        by_hand.config(),
        "a 2x document export fits at 1x"
    );
}

#[test]
fn the_first_draw_fits_and_draw_as_stored_does_not() {
    let mut r = renderer();
    let (mut chart, _) = audit_chart(&mut r);
    let stored = chart.config().clone();
    let mut view = r.create_chart_view(&chart, stored.chart_area.0).unwrap();
    let _ = r
        .prepare_as_stored(&mut [ChartDrawItem {
            view: &mut view,
            chart: &mut chart,
            series: &[],
        }])
        .unwrap();
    assert_eq!(chart.config(), &stored);
    let _ = r
        .prepare(&mut [ChartDrawItem {
            view: &mut view,
            chart: &mut chart,
            series: &[],
        }])
        .unwrap();
    assert_eq!(chart.config(), &fitted(&stored));
}

#[test]
fn an_edit_of_a_fitted_layout_fits_again_and_a_drag_is_never_undone() {
    let mut r = renderer();
    let (mut chart, series) = audit_chart(&mut r);
    let _ = export(&mut r, &mut chart, &series, ExportOptions::panel(1.0));
    assert_eq!(chart.config().layout_state, LayoutState::Fitted);

    // A longer y title after the first draw: the next draw fits it.
    chart.config_mut().left_y.title_option.text.segments =
        renderer::text::rich_segments_from_label("Amplitude of the damped signal (V)");
    assert_eq!(chart.config().layout_state, LayoutState::NeedsFit);
    let edited = chart.config().clone();
    let _ = export(&mut r, &mut chart, &series, ExportOptions::panel(1.0));
    assert_eq!(chart.config(), &fitted(&edited));

    // The user drags the y title; later draws keep it where it was put.
    let measure = CpuTextMeasure::for_style(&chart.config().draw_style);
    let moved =
        AxisTitleElement { side: Side::Left }.drag_by(chart.config_mut(), &measure, -6.0, 0.0);
    assert_eq!(moved, renderer::layout::NudgeResult::Moved);
    assert_eq!(chart.config().layout_state, LayoutState::Fixed);
    let dragged = chart.config().clone();
    let image = export(&mut r, &mut chart, &series, ExportOptions::panel(1.0));
    assert_eq!(
        chart.config(),
        &dragged,
        "a draw after a drag does not refit"
    );
    // Further edits keep the user's layout too.
    chart.set_y_range(-0.8, 0.9);
    assert_eq!(chart.config().layout_state, LayoutState::Fixed);
    let _ = export(&mut r, &mut chart, &series, ExportOptions::panel(1.0));
    assert_eq!(chart.config().left_y.out_margin, dragged.left_y.out_margin);
    assert_eq!(
        chart.config().left_y.title_option.offset_y,
        dragged.left_y.title_option.offset_y
    );
    assert!(!image.rgba.is_empty());
}

#[test]
fn a_registered_chart_fits_on_its_first_draw_and_export_and_records_the_fit() {
    let mut r = renderer();
    let (chart, series) = audit_chart(&mut r);
    let stored = chart.config().clone();
    let id = r.register_chart(stored.clone(), series.to_vec()).unwrap();
    let mut view = r.create_chart_view(&chart, stored.chart_area.0).unwrap();

    // Drawing as stored leaves the stored layout waiting.
    let _ = r
        .prepare_registered_as_stored(&mut [RegisteredChartDrawItem {
            chart_id: id,
            view: &mut view,
        }])
        .unwrap();
    assert_eq!(r.chart_config(id).unwrap(), &stored);
    let before = r.chart_render_stamp(id).unwrap();

    // The default draw fits it through the registry, so its revisions move.
    let _ = r
        .prepare_registered(&mut [RegisteredChartDrawItem {
            chart_id: id,
            view: &mut view,
        }])
        .unwrap();
    assert_eq!(r.chart_config(id).unwrap(), &fitted(&stored));
    assert_ne!(r.chart_render_stamp(id).unwrap(), before);
    assert!(!r.fit_pending_layout(id).unwrap(), "nothing waits any more");

    // An edit of the fitted chart asks for a fit; the export makes it.
    let mut edited = r.chart_config(id).unwrap().clone();
    edited.chart_title.text.segments =
        renderer::text::rich_segments_from_label("A longer title\nover two lines");
    r.set_chart_config(id, edited).unwrap();
    let pending = r.chart_config(id).unwrap().clone();
    assert_eq!(pending.layout_state, LayoutState::NeedsFit);
    let as_stored = r
        .export_rgba(
            ExportSource::Registered(id),
            ExportOptions::panel(1.0).as_stored(),
        )
        .block_on()
        .unwrap();
    assert_eq!(r.chart_config(id).unwrap(), &pending);
    let image = r
        .export_rgba(ExportSource::Registered(id), ExportOptions::panel(1.0))
        .block_on()
        .unwrap();
    assert_eq!(r.chart_config(id).unwrap(), &fitted(&pending));
    assert_ne!(image.rgba, as_stored.rgba);
    // A host that sets a layout of its own keeps it.
    let mut manual = r.chart_config(id).unwrap().clone();
    manual.left_y.out_margin += 30.0;
    manual.layout_state = LayoutState::Fixed;
    r.set_chart_config(id, manual.clone()).unwrap();
    let _ = r
        .export_rgba(ExportSource::Registered(id), ExportOptions::panel(1.0))
        .block_on()
        .unwrap();
    assert_eq!(r.chart_config(id).unwrap(), &manual);
}

#[test]
fn an_explicit_fit_and_a_registered_fit_agree_and_an_identical_config_changes_nothing() {
    let mut r = renderer();
    let (chart, series) = audit_chart(&mut r);
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    assert!(r.fit_pending_layout(id).unwrap());
    let laid = r.chart_config(id).unwrap().clone();
    let stamp = r.chart_render_stamp(id).unwrap();
    r.set_chart_config(id, laid.clone()).unwrap();
    assert_eq!(
        r.chart_config(id).unwrap(),
        &laid,
        "an identical config stays fitted"
    );
    assert_eq!(r.chart_render_stamp(id).unwrap(), stamp);
}

#[cfg(feature = "serde")]
#[test]
fn a_fitted_chart_round_trips_and_exports_the_same_image() {
    let mut r = renderer();
    let (mut chart, series) = audit_chart(&mut r);
    let pending_json = serde_json::to_string(chart.config()).unwrap();
    assert!(pending_json.contains("\"layout_state\":\"needs_fit\""));
    for scale in [1.0, 2.0] {
        let image = export(&mut r, &mut chart, &series, ExportOptions::panel(scale));
        let json = serde_json::to_string(chart.config()).unwrap();
        assert!(json.contains("\"layout_state\":\"fitted\""));
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, chart.config());
        let mut reread = Chart::new(back);
        let again = export(&mut r, &mut reread, &series, ExportOptions::panel(scale));
        assert_eq!(again.rgba, image.rgba, "{scale}x");
        assert_eq!(serde_json::to_string(reread.config()).unwrap(), json);
    }
}

#[test]
fn a_monotonic_chart_is_registered_fixed() {
    let mut r = renderer();
    let mut config = Chart::sized(400, 300).config().clone();
    config.layout_state = LayoutState::NeedsFit;
    let id = r.register_monotonic_chart(config, Vec::new()).unwrap();
    assert_eq!(
        r.monotonic_config(id).unwrap().layout_state,
        LayoutState::Fixed
    );
}
