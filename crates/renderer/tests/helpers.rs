//! The host helpers end to end (#265): a headless renderer, columns from
//! plain `Vec`s, a series from a constructor, and one save call by format.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/gpu.rs"]
mod test_gpu;

use renderer::radial::{RadialChart, RadialLabels, RadialRenderer, RadialSlice};
use renderer::{
    Chart, Color, ExportFormat, ExportOptions, ExportSource, Renderer, RendererDevice, SaveError,
    SeriesConfig,
};

fn scratch(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("figgy-helpers-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join(name)
}

fn sine() -> (Vec<f64>, Vec<f64>) {
    let x: Vec<f64> = (0..200).map(|i| f64::from(i) * 0.05).collect();
    let y = x.iter().map(|x| x.sin()).collect();
    (x, y)
}

// The pipeline in its explicit steps: create, upload, document, save. Each
// format's file holds what that format's export returns.
#[test]
fn a_headless_renderer_saves_each_format() {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let (x, y) = sine();
    renderer.add_column("x", &x).unwrap();
    renderer.add_column("y", &y).unwrap();
    let series = [SeriesConfig::line("sine", "x", "y")
        .label("sin x")
        .width(2.0)];
    let mut chart = Chart::new(renderer::config::Config::default()).with_title("A sine wave");
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-1.2, 1.2);
    let source = || ExportSource::Chart {
        chart: &chart,
        series: &series,
    };
    let options = ExportOptions::document(1.0);

    let png = scratch("sine.png");
    renderer.save_blocking(source(), options, &png).unwrap();
    let saved = std::fs::read(&png).unwrap();
    assert!(saved.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(
        saved,
        renderer.export_png_blocking(source(), options).unwrap(),
        "the saved PNG is the PNG export"
    );

    let svg = scratch("sine.SVG");
    renderer.save_blocking(source(), options, &svg).unwrap();
    let saved = std::fs::read_to_string(&svg).unwrap();
    assert!(saved.contains("<svg"), "{}", &saved[..saved.len().min(80)]);

    let pdf = scratch("sine.pdf");
    renderer.save_blocking(source(), options, &pdf).unwrap();
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));

    let named = scratch("sine.image");
    renderer
        .save_as_blocking(source(), options, &named, ExportFormat::Png)
        .unwrap();
    assert!(std::fs::read(&named).unwrap().starts_with(b"\x89PNG"));
    assert!(matches!(
        renderer.save_blocking(source(), options, scratch("sine.jpeg")),
        Err(SaveError::UnknownFormat(_))
    ));
    assert!(matches!(
        renderer.save_blocking(source(), options, scratch("missing/dir/sine.png")),
        Err(SaveError::Write { .. })
    ));
}

#[test]
fn the_radial_renderer_saves_by_format() {
    let (device, queue) = test_gpu::device();
    let gpu = RendererDevice::new(device, queue).expect("figgy poll thread");
    let mut renderer = RadialRenderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let chart = RadialChart {
        labels: RadialLabels::None,
        slices: vec![
            RadialSlice::new("A", 25.0, Color::new(1.0, 0.0, 0.0, 1.0)),
            RadialSlice::new("B", 75.0, Color::new(0.0, 0.0, 1.0, 1.0)),
        ],
        ..Default::default()
    };
    let options = ExportOptions::document(1.0);
    let path = scratch("pie.png");
    renderer
        .save_blocking(&chart, (240, 240), options, &path)
        .unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        renderer
            .export_bytes_blocking(&chart, (240, 240), options, ExportFormat::Png)
            .unwrap()
    );
    let svg = renderer
        .export_bytes_blocking(&chart, (240, 240), options, ExportFormat::Svg)
        .unwrap();
    assert!(String::from_utf8(svg).unwrap().contains("<svg"));
}

// A sized chart takes its area, the legend takes one entry per labelled
// series with the series' own symbol, and fitting the layout is
// `auto_layout` with the chart's own font measure.
#[test]
fn chart_conveniences_build_the_document() {
    let series = [
        SeriesConfig::line("a", "x", "y").label("first"),
        SeriesConfig::scatter("b", "x", "y"),
        SeriesConfig::scatter("c", "x", "y").label("third"),
    ];
    let mut chart = Chart::sized(640, 400)
        .with_title("Sized")
        .with_legend_from(&series);
    let area = chart.config().chart_area.0;
    assert_eq!((area.x, area.y, area.width, area.height), (0, 0, 640, 400));
    let legend = &chart.config().legend;
    assert!(legend.visible);
    let text: String = legend.content.segments.iter().map(|s| s.text).collect();
    assert!(text.contains("first") && text.contains("third"), "{text:?}");
    let ids: std::collections::BTreeSet<_> = legend
        .content
        .segments
        .iter()
        .filter_map(|s| s.series_id.as_deref())
        .collect();
    assert_eq!(ids, ["a", "c"].into_iter().collect());

    let measure = renderer::CpuTextMeasure::for_style(&chart.config().draw_style);
    let expected = renderer::layout::auto_layout(chart.config(), &measure);
    chart.fit_layout();
    assert_eq!(chart.config(), &expected);
}
