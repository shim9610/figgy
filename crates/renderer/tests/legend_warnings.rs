//! The legend guard (#483): a registered chart's view reports, as
//! `RenderWarning::LegendSymbolMismatch`, every legend symbol that names a
//! series but does not draw that series' mark, re-derived whenever
//! `prepare_registered` redraws it after a config or series edit. A view of
//! a held `Chart` has no series and reports none.
//!
//! The model side (what is a mismatch, every legend of a chart, a JSON round
//! trip) is in the model's `legend::mismatch_tests`.

#![cfg(not(target_arch = "wasm32"))]

#[path = "support/gpu.rs"]
mod test_gpu;
use test_gpu::shared_device;

use renderer::config::{legend_entry_count, series_legend_entry};
use renderer::data_config::{DataLineStyleConfig, LineInterpolation, XySeries};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::text::RichSegment;
use renderer::{
    Chart, ChartId, ChartView, Color, Column, Config, DataRenderType, LayerRef, LegendMismatch,
    RegisteredChartDrawItem, RenderWarning, Renderer, RendererDevice, SeriesConfig,
};

const RED: Color = Color::from_rgb8(255, 0, 0);
const BLUE: Color = Color::from_rgb8(0, 0, 255);

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column(
        "x",
        &Column {
            data: vec![0.0, 1.0],
        },
    )
    .unwrap();
    r.add_column(
        "y",
        &Column {
            data: vec![0.0, 1.0],
        },
    )
    .unwrap();
    r
}

fn line(id: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Xy(XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: color,
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: LineInterpolation::Linear,
            }),
            errors: None,
            stem: None,
        }),
    }
}

/// A chart whose legend has one entry per series, built from the series.
fn config(series: &[SeriesConfig]) -> Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 1.0;
    config.left_y.min = 0.0;
    config.left_y.max = 1.0;
    config.legend.content.segments.clear();
    for (i, cfg) in series.iter().enumerate() {
        if i > 0 {
            config
                .legend
                .content
                .segments
                .push(RichSegment::plain('\n'));
        }
        config
            .legend
            .content
            .segments
            .extend(series_legend_entry(cfg, None));
    }
    config
}

/// The segment index where the symbol naming `series_id` starts.
fn symbol_start(config: &Config, series_id: &str) -> usize {
    config
        .legend
        .content
        .segments
        .iter()
        .position(|s| s.series_id.as_deref() == Some(series_id))
        .unwrap()
}

/// A host frame of registered chart `id` on `view`.
fn frame(r: &mut Renderer, id: ChartId, view: &mut ChartView) {
    r.prepare_registered(&mut [RegisteredChartDrawItem { chart_id: id, view }])
        .unwrap();
}

fn new_view(r: &Renderer, id: ChartId) -> ChartView {
    let (config, panel, scale) = r.chart_display_config(id).unwrap();
    let chart = Chart::new(config.into_owned());
    r.create_chart_view_at(&chart, panel, scale).unwrap()
}

fn legend_warnings(view: &ChartView) -> Vec<&RenderWarning> {
    view.render_warnings()
        .iter()
        .filter(|w| matches!(w, RenderWarning::LegendSymbolMismatch { .. }))
        .collect()
}

#[test]
fn a_registered_view_reports_legend_symbols_that_do_not_draw_their_series() {
    let mut r = renderer();
    let series = vec![line("a", RED), line("b", BLUE)];
    let id = r
        .register_chart_as_given(config(&series), series.clone())
        .unwrap();
    let mut view = new_view(&r, id);
    frame(&mut r, id, &mut view);
    assert!(legend_warnings(&view).is_empty());

    // The host recolours `b`'s symbol by hand.
    let mut edited = r.chart_config(id).unwrap().clone();
    let start = symbol_start(&edited, "b");
    edited.legend.content.segments[start].color = Some(RED);
    r.set_chart_config(id, edited.clone()).unwrap();
    frame(&mut r, id, &mut view);
    let expected = RenderWarning::LegendSymbolMismatch {
        layer: LayerRef::Base,
        entry: 1,
        series_id: "b".into(),
        reason: LegendMismatch::ColorDiffers,
    };
    assert_eq!(legend_warnings(&view), [&expected]);
    assert!(expected.to_string().starts_with("legend.content entry 1"));
    // Clean frames keep describing the raster.
    frame(&mut r, id, &mut view);
    assert_eq!(legend_warnings(&view), [&expected]);

    // A series edit re-derives it: `b` is gone, so its entry names nothing.
    r.set_chart_series(id, vec![line("a", RED)]).unwrap();
    frame(&mut r, id, &mut view);
    assert_eq!(
        legend_warnings(&view),
        [&RenderWarning::LegendSymbolMismatch {
            layer: LayerRef::Base,
            entry: 1,
            series_id: "b".into(),
            reason: LegendMismatch::UnknownSeries,
        }]
    );

    // A legend that matches its series again clears it.
    r.set_chart_config(id, config(&[line("a", RED)])).unwrap();
    frame(&mut r, id, &mut view);
    assert!(legend_warnings(&view).is_empty());
}

#[test]
fn a_view_of_a_held_chart_reports_no_legend_warning() {
    let r = renderer();
    let series = [line("a", RED)];
    let mut config = config(&series);
    let start = symbol_start(&config, "a");
    config.legend.content.segments[start].color = Some(BLUE);
    // The model still says what is wrong.
    assert_eq!(config.legend_warnings(&series).len(), 1);
    let chart = Chart::new(config);
    let view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    assert!(legend_warnings(&view).is_empty());
}

/// Rule 10: a legend of many entries over many series is checked in full.
#[test]
fn every_entry_of_a_large_registered_legend_is_checked() {
    const COUNT: usize = 600;
    let mut r = renderer();
    let series: Vec<_> = (0..COUNT)
        .map(|i| line(&format!("s{i}"), Color::from_rgb8(0, (i % 251) as u8, 0)))
        .collect();
    let mut config = config(&series);
    assert_eq!(legend_entry_count(&config.legend.content), COUNT);
    let id = r
        .register_chart_as_given(config.clone(), series.clone())
        .unwrap();
    let mut view = new_view(&r, id);
    frame(&mut r, id, &mut view);
    assert!(legend_warnings(&view).is_empty());

    let last = format!("s{}", COUNT - 1);
    let start = symbol_start(&config, &last);
    config.legend.content.segments[start].color = Some(BLUE);
    r.set_chart_config(id, config).unwrap();
    frame(&mut r, id, &mut view);
    assert_eq!(
        legend_warnings(&view),
        [&RenderWarning::LegendSymbolMismatch {
            layer: LayerRef::Base,
            entry: COUNT - 1,
            series_id: last,
            reason: LegendMismatch::ColorDiffers,
        }]
    );
}
