//! The layout of a new chart (#357): a chart whose layout it keeps fitted
//! (`LayoutState::Fitted`) is created with that layout, and every chart
//! edit fits it again as part of the edit, so the stored config always
//! says what is drawn. Draws and exports draw the stored config as it is:
//! they never fit and never write. A user's drag fixes the layout, and no
//! later edit undoes it. Older documents read as `Fixed` and draw as they
//! did (`released_documents*.rs` pin them).
#![cfg(not(target_arch = "wasm32"))]

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use renderer::format::LabelFormat;
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
    audit_chart_in(r, Chart::sized(800, 600))
}

/// The audit's edits on `chart`, its columns in `r`.
fn audit_chart_in(r: &mut Renderer, chart: Chart) -> (Chart, [SeriesConfig; 1]) {
    let x: Vec<f64> = (0..=100).map(|i| f64::from(i) / 10.0).collect();
    let y: Vec<f64> = x.iter().map(|t| 0.7 * (t * 1.3).cos() + 0.05).collect();
    let _ = r.remove_column("x");
    let _ = r.remove_column("y");
    r.add_column("x", &x).unwrap();
    r.add_column("y", &y).unwrap();
    let mut chart = chart
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
    chart: &Chart,
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

/// The audit's chart on a `Fixed` config: the documented default margins,
/// drawn as stored. Its y labels are written in `Decimal`, as the audit's
/// were (`-0.600`), so they are wide enough to meet the y title at those
/// margins whatever format the helpers choose for a new chart (#370 made
/// that `Auto`, which writes `-0.6`).
fn audit_chart_fixed(r: &mut Renderer) -> (Chart, [SeriesConfig; 1]) {
    let config = Config {
        layout_state: LayoutState::Fixed,
        ..Chart::sized(800, 600).config().clone()
    };
    let documented = renderer::default::default_config();
    let config = Config {
        top_x: documented.top_x,
        bottom_x: documented.bottom_x,
        left_y: documented.left_y,
        right_y: documented.right_y,
        chart_title: documented.chart_title,
        ..config
    };
    let (chart, series) = audit_chart_in(r, Chart::new(config));
    (chart.with_label_format(LabelFormat::Decimal), series)
}

#[test]
fn a_new_chart_is_created_with_its_layout_fitted() {
    let mut r = renderer();
    let (chart, _) = audit_chart(&mut r);
    assert_eq!(chart.config().layout_state, LayoutState::Fitted);
    assert_eq!(
        chart.config(),
        &fitted(chart.config()),
        "the chart holds the fit of what it holds"
    );
    assert!(title_clears_labels(chart.config()));

    // The same edits on a `Fixed` chart keep the documented margins, which
    // put the y title over the -0.600 labels: the overlap the audit saw.
    let (fixed, _) = audit_chart_fixed(&mut r);
    assert_eq!(fixed.config().layout_state, LayoutState::Fixed);
    assert!(!title_clears_labels(fixed.config()));

    // Creating a chart object from a config the chart keeps fitted fits it.
    let mut unfitted = fixed.config().clone();
    unfitted.layout_state = LayoutState::Fitted;
    let created = Chart::new(unfitted.clone());
    assert_eq!(created.config(), &fitted(&unfitted));
    assert!(title_clears_labels(created.config()));
}

#[test]
fn draws_and_exports_never_change_the_config() {
    let mut r = renderer();
    let (mut chart, series) = audit_chart(&mut r);
    // An edit through `config_mut` is not fitted: the draw shows it as
    // stored, with no hidden layout. Larger y tick labels need a wider band.
    chart.config_mut().left_y.label_style.font_size = 30.0;
    let stored = chart.config().clone();
    assert_ne!(stored, fitted(&stored), "the edit is not fitted yet");
    for options in [
        ExportOptions::panel(1.0),
        ExportOptions::panel(2.0),
        ExportOptions::document(2.0),
    ] {
        let _ = export(&mut r, &chart, &series, options);
        assert_eq!(chart.config(), &stored, "{options:?}");
    }
    let mut view = r.create_chart_view(&chart, stored.chart_area.0).unwrap();
    let _ = r
        .prepare(&mut [ChartDrawItem {
            view: &mut view,
            chart: &chart,
            series: &[],
        }])
        .unwrap();
    assert_eq!(chart.config(), &stored);
    // The export draws exactly the stored layout: the same image as a
    // `Fixed` chart holding it.
    let mut held = stored.clone();
    held.layout_state = LayoutState::Fixed;
    let as_fixed = Chart::new(held);
    for scale in [1.0, 2.0] {
        let image = export(&mut r, &chart, &series, ExportOptions::panel(scale));
        let same = export(&mut r, &as_fixed, &series, ExportOptions::panel(scale));
        assert_eq!(image.rgba, same.rgba, "{scale}x");
    }
    // `fit_layout` fits it on request.
    chart.fit_layout();
    assert_eq!(chart.config(), &fitted(&stored));
}

#[test]
fn one_x_and_two_x_export_the_same_layout() {
    let mut r = renderer();
    let (chart, series) = audit_chart(&mut r);
    let mut by_hand = chart.config().clone();
    by_hand.layout_state = LayoutState::Fixed;
    let by_hand = Chart::new(fitted(&by_hand));
    for options in [
        ExportOptions::panel(1.0),
        ExportOptions::panel(2.0),
        ExportOptions::document(2.0),
    ] {
        let auto = export(&mut r, &chart, &series, options);
        let manual = export(&mut r, &by_hand, &series, options);
        assert_eq!(
            auto.rgba, manual.rgba,
            "{options:?}: the layout is the hand fit"
        );
    }
}

#[test]
fn a_chart_edit_fits_again_and_a_drag_is_never_undone() {
    let mut r = renderer();
    let (mut chart, _) = audit_chart(&mut r);

    // A longer y title through a chart method: the edit fits it.
    chart.with_decoration_change(|config| {
        config.left_y.title_option.text.segments =
            renderer::text::rich_segments_from_label("Amplitude of the damped signal (V)");
    });
    assert_eq!(chart.config().layout_state, LayoutState::Fitted);
    assert_eq!(chart.config(), &fitted(chart.config()));
    assert!(title_clears_labels(chart.config()));
    // New ranges are new tick labels: fitted as part of the edit.
    chart.set_y_range(-1000.0, 1000.0);
    assert_eq!(chart.config(), &fitted(chart.config()));
    assert!(title_clears_labels(chart.config()));

    // The user drags the y title; later edits keep it where it was put.
    let measure = CpuTextMeasure::for_style(&chart.config().draw_style);
    let moved =
        AxisTitleElement { side: Side::Left }.drag_by(chart.config_mut(), &measure, -6.0, 0.0);
    assert_eq!(moved, renderer::layout::NudgeResult::Moved);
    assert_eq!(chart.config().layout_state, LayoutState::Fixed);
    let dragged = chart.config().clone();
    chart.set_y_range(-0.8, 0.9);
    chart.with_decoration_change(|config| {
        config.chart_title.text.segments =
            renderer::text::rich_segments_from_label("A longer title\nover two lines");
    });
    assert_eq!(chart.config().layout_state, LayoutState::Fixed);
    assert_eq!(chart.config().left_y.out_margin, dragged.left_y.out_margin);
    assert_eq!(
        chart.config().left_y.title_option.offset_y,
        dragged.left_y.title_option.offset_y
    );
    assert_eq!(
        chart.config().chart_title.top_margin,
        dragged.chart_title.top_margin
    );
}

#[test]
fn a_registered_chart_is_fitted_when_registered_and_edited_never_when_drawn() {
    let mut r = renderer();
    let (fixed, series) = audit_chart_fixed(&mut r);
    let mut unfitted = fixed.config().clone();
    unfitted.layout_state = LayoutState::Fitted;
    let id = r.register_chart(unfitted.clone(), series.to_vec()).unwrap();
    let laid = r.chart_config(id).unwrap().clone();
    assert_eq!(laid, fitted(&unfitted), "registration fits");

    // Draws and exports draw it as stored.
    let stamp = r.chart_render_stamp(id).unwrap();
    let mut view = r.create_chart_view(&fixed, laid.chart_area.0).unwrap();
    let _ = r
        .prepare_registered(&mut [RegisteredChartDrawItem {
            chart_id: id,
            view: &mut view,
        }])
        .unwrap();
    for scale in [1.0, 2.0] {
        let _ = r
            .export_rgba(ExportSource::Registered(id), ExportOptions::panel(scale))
            .block_on()
            .unwrap();
    }
    assert_eq!(r.chart_config(id).unwrap(), &laid);
    assert_eq!(r.chart_render_stamp(id).unwrap(), stamp);

    // An edit of the chart is fitted as part of the edit.
    let mut edited = laid.clone();
    edited.chart_title.text.segments =
        renderer::text::rich_segments_from_label("A longer title\nover two lines");
    r.set_chart_config(id, edited.clone()).unwrap();
    assert_eq!(r.chart_config(id).unwrap(), &fitted(&edited));
    assert_ne!(r.chart_render_stamp(id).unwrap(), stamp);

    // A host that sets a layout of its own keeps it.
    let mut manual = r.chart_config(id).unwrap().clone();
    manual.left_y.out_margin += 30.0;
    manual.layout_state = LayoutState::Fixed;
    r.set_chart_config(id, manual.clone()).unwrap();
    assert_eq!(r.chart_config(id).unwrap(), &manual);
    let _ = r
        .export_rgba(ExportSource::Registered(id), ExportOptions::panel(1.0))
        .block_on()
        .unwrap();
    assert_eq!(r.chart_config(id).unwrap(), &manual);
}

#[test]
fn an_identical_config_changes_nothing() {
    let mut r = renderer();
    let (chart, series) = audit_chart(&mut r);
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let laid = r.chart_config(id).unwrap().clone();
    assert_eq!(&laid, chart.config(), "a fitted chart registers as it is");
    let stamp = r.chart_render_stamp(id).unwrap();
    r.set_chart_config(id, laid.clone()).unwrap();
    assert_eq!(r.chart_config(id).unwrap(), &laid);
    assert_eq!(r.chart_render_stamp(id).unwrap(), stamp);
}

/// A saved fitted chart that is loaded and made into a chart again is
/// refitted when the chart is created: a refit of a layout that is already
/// fitted is a fixed point, so the reread chart holds the saved layout and
/// draws the same image at 1x and 2x (#357). Covered for the audit chart,
/// a document with a subplot cell, and the categorical, box plot and radar
/// documents built in Rust.
#[cfg(feature = "serde")]
#[test]
fn a_saved_fitted_chart_is_created_again_with_the_same_layout() {
    use renderer::{BoxPlotChart, CategoricalChart, RadarChart, Subplot};
    let mut r = renderer();
    let (audit, audit_series) = audit_chart(&mut r);
    let mut documents: Vec<(&str, Chart, Vec<SeriesConfig>)> =
        vec![("audit", audit, audit_series.to_vec())];
    let cell = Chart::placed(480, 0, 480, 320)
        .with_title("Cell")
        .with_y_title("Amplitude (V)")
        .config()
        .clone();
    let grid = Chart::sized(480, 320)
        .with_title("Parent")
        .with_x_title("Time (s)")
        .with_subplot(Subplot::new(cell, &[]));
    documents.push(("subplot", grid, Vec::new()));
    let (categorical, box_plot, radar) = (
        CategoricalChart::default(),
        BoxPlotChart::default(),
        RadarChart::default(),
    );
    let converted = [
        Config::from_categorical(&categorical),
        Config::from_boxplot(&box_plot),
        Config::from_radar(&radar),
    ];
    // The default documents hold no values: their layout and chrome are
    // drawn without series.
    for (name, (config, _, _)) in ["categorical", "box plot", "radar"]
        .into_iter()
        .zip(converted)
    {
        assert_eq!(config.layout_state, LayoutState::Fitted, "{name}");
        documents.push((name, Chart::new(config), Vec::new()));
    }
    for (name, chart, series) in &documents {
        assert_eq!(chart.config().layout_state, LayoutState::Fitted, "{name}");
        let json = serde_json::to_string(chart.config()).unwrap();
        assert!(json.contains("\"layout_state\":\"fitted\""), "{name}");
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, chart.config(), "{name}: read back");
        let reread = Chart::new(back);
        assert_eq!(
            reread.config(),
            chart.config(),
            "{name}: a refit of a fitted layout is a fixed point"
        );
        assert_eq!(serde_json::to_string(reread.config()).unwrap(), json);
        for scale in [1.0, 2.0] {
            let options = ExportOptions::document(scale);
            let image = export(&mut r, chart, series, options);
            let again = export(&mut r, &reread, series, options);
            assert_eq!(again.rgba, image.rgba, "{name} at {scale}x");
        }
    }
}

#[test]
fn a_monotonic_chart_is_registered_fixed() {
    let mut r = renderer();
    let config = Chart::sized(400, 300).config().clone();
    assert_eq!(config.layout_state, LayoutState::Fitted);
    let id = r.register_monotonic_chart(config, Vec::new()).unwrap();
    assert_eq!(
        r.monotonic_config(id).unwrap().layout_state,
        LayoutState::Fixed
    );
}
