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

// `Config::background` fills the whole chart area under everything, at 1x
// and 2x, in the PNG and in the SVG; without it the corner stays the clear
// colour. A document without the key reads it as `None` and writes no key.
#[test]
fn the_chart_background_fills_the_chart_area() {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let (x, y) = sine();
    renderer.add_column("x", &x).unwrap();
    renderer.add_column("y", &y).unwrap();
    let series = [SeriesConfig::line("sine", "x", "y")];
    let paper = Color::from_rgb8(250, 246, 236);
    let mut chart = Chart::sized(320, 240);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-1.2, 1.2);
    let plain = chart.config().clone();
    chart.config_mut().background = Some(paper);
    for scale in [1.0f32, 2.0] {
        for (config, expected) in [(chart.config(), [250, 246, 236, 255]), (&plain, [0; 4])] {
            let held = Chart::new(config.clone());
            let png = renderer
                .export_png_blocking(
                    ExportSource::Chart {
                        chart: &held,
                        series: &series,
                    },
                    ExportOptions::document(scale),
                )
                .unwrap();
            let mut reader = png::Decoder::new(std::io::Cursor::new(png))
                .read_info()
                .unwrap();
            let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut rgba).unwrap();
            assert_eq!(info.color_type, png::ColorType::Rgba);
            // The top-left corner sits in the margin, outside the data area.
            let corner = (2 * info.width as usize + 2) * 4;
            assert_eq!(rgba[corner..corner + 4], expected, "{scale}x");
        }
    }
    let svg = renderer
        .export_svg_blocking(
            ExportSource::Chart {
                chart: &chart,
                series: &series,
            },
            ExportOptions::document(1.0),
        )
        .unwrap();
    assert!(svg.contains("#faf6ec"), "the SVG carries the fill");
}

#[cfg(feature = "serde")]
#[test]
fn the_chart_background_is_an_additive_key() {
    let plain = renderer::config::Config::default();
    let mut filled_config = plain.clone();
    filled_config.background = Some(Color::from_rgb8(250, 246, 236));
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("\"background\""));
    let back: renderer::config::Config = serde_json::from_str(&json).unwrap();
    assert_eq!(back.background, None);
    let filled = serde_json::to_string(&filled_config).unwrap();
    let back: renderer::config::Config = serde_json::from_str(&filled).unwrap();
    assert_eq!(back, filled_config);
    assert_eq!(serde_json::to_string(&back).unwrap(), filled);
}

// A theme writes its look into the chart and its series and refits the
// layout to its text sizes; the themed chart exports at 1x and 2x with the
// theme's background and series colours.
#[test]
fn a_themed_chart_exports_with_its_look() {
    use renderer::theme::{SubplotScope, Theme};
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let (x, y) = sine();
    renderer.add_column("x", &x).unwrap();
    renderer.add_column("y", &y).unwrap();
    let theme = Theme::dark();
    let mut series = [SeriesConfig::line("sine", "x", "y").label("sin x")];
    let mut chart = Chart::sized(480, 320)
        .with_title("A sine wave")
        .with_legend_from(&series);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-1.2, 1.2);
    chart.apply_theme(&theme, &mut series, SubplotScope::All);

    let measure = renderer::CpuTextMeasure::for_style(&chart.config().draw_style);
    let fitted = renderer::layout::auto_layout(chart.config(), &measure);
    assert_eq!(
        chart.config(),
        &fitted,
        "the layout is fitted after the theme"
    );
    assert_eq!(chart.config().background, theme.chrome.background);
    assert_eq!(
        (chart.config().bottom_x.min, chart.config().bottom_x.max),
        (0.0, 10.0)
    );

    let bg = theme.chrome.background.unwrap();
    let expected = [bg.r, bg.g, bg.b].map(|c| (c * 255.0).round() as u8);
    for scale in [1.0f32, 2.0] {
        let png = renderer
            .export_png_blocking(
                ExportSource::Chart {
                    chart: &chart,
                    series: &series,
                },
                ExportOptions::document(scale),
            )
            .unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut rgba).unwrap();
        let corner = (2 * info.width as usize + 2) * 4;
        assert_eq!(rgba[corner..corner + 3], expected, "{scale}x");
        // The series draws in its cycle colour somewhere on the chart.
        let line = theme.series.cycle.color(0);
        let target = [line.r, line.g, line.b].map(|c| (c * 255.0).round() as i32);
        assert!(
            rgba.chunks_exact(4)
                .any(|p| (0..3).all(|i| (i32::from(p[i]) - target[i]).abs() <= 8)),
            "{scale}x: no pixel in the series colour"
        );
    }
}

// A placed chart, a resized document chart, an overlay and a swatch entry
// are document values; a checked raster saves as PNG.
#[test]
fn chart_placement_overlay_swatch_and_png_save() {
    let chart = Chart::placed(40, 30, 320, 200);
    let area = chart.config().chart_area.0;
    assert_eq!(
        (area.x, area.y, area.width, area.height),
        (40, 30, 320, 200)
    );
    let resized = Chart::sized(10, 10).with_area(5, 6, 300, 180);
    let area = resized.config().chart_area.0;
    assert_eq!((area.x, area.y, area.width, area.height), (5, 6, 300, 180));

    let axis = renderer::config::Config::default().left_y;
    let red = Color::new(0.9, 0.1, 0.1, 1.0);
    let chart = Chart::sized(320, 200)
        .with_overlay(axis, &["b", "c"])
        .with_legend_swatch("Loss", red);
    let config = chart.config();
    assert_eq!(config.overlays.len(), 1);
    assert_eq!(config.overlays[0].series, ["b", "c"]);
    assert!(config.legend.visible);
    let text: String = config
        .legend
        .content
        .segments
        .iter()
        .map(|s| s.text)
        .collect();
    assert!(text.ends_with("Loss"), "{text:?}");
    assert!(
        config
            .legend
            .content
            .segments
            .iter()
            .any(|s| s.color == Some(red))
    );
    assert!(config.validate().is_ok());

    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let (x, y) = sine();
    renderer.add_column("x", &x).unwrap();
    renderer.add_column("y", &y).unwrap();
    let series = [SeriesConfig::line("sine", "x", "y")];
    let image = renderer
        .export_rgba_blocking(
            ExportSource::Chart {
                chart: &Chart::sized(200, 120),
                series: &series,
            },
            ExportOptions::panel(1.0),
        )
        .unwrap();
    let path = scratch("raster.png");
    image.save_png(&path).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        renderer::encode_png(&image).unwrap()
    );
    assert!(matches!(
        image.save_png(scratch("missing/dir/raster.png")),
        Err(SaveError::Write { .. })
    ));
}

// The radial renderer makes its own device, and a radial chart takes the
// title `with_title` gives it.
#[test]
fn a_headless_radial_renderer_draws_a_titled_chart() {
    let mut renderer = RadialRenderer::headless_blocking().expect("headless radial renderer");
    let chart = RadialChart {
        labels: RadialLabels::None,
        slices: vec![
            RadialSlice::new("A", 25.0, Color::new(1.0, 0.0, 0.0, 1.0)),
            RadialSlice::new("B", 75.0, Color::new(0.0, 0.0, 1.0, 1.0)),
        ],
        ..Default::default()
    }
    .with_title("Share");
    let title = chart.chart_title.as_ref().unwrap();
    assert_eq!(
        (title.top_margin, title.offset_x, title.offset_y),
        (0.0, 0.0, 0.0)
    );
    let png = renderer
        .export_bytes_blocking(
            &chart,
            (240, 240),
            ExportOptions::document(1.0),
            ExportFormat::Png,
        )
        .unwrap();
    assert!(png.starts_with(b"\x89PNG"));
}
