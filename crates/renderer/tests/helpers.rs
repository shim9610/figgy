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
    use renderer::theme::{ChartKind, Theme};
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let (x, y) = sine();
    renderer.add_column("x", &x).unwrap();
    renderer.add_column("y", &y).unwrap();
    let theme = Theme::named(ChartKind::Line, "midnight").expect("the line kind's midnight");
    let mut series = [SeriesConfig::line("sine", "x", "y").label("sin x")];
    let mut chart = Chart::sized(480, 320)
        .with_title("A sine wave")
        .with_legend_from(&series);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-1.2, 1.2);
    chart
        .apply_theme("midnight", &mut series)
        .expect("a family on a line chart");

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
        // The series draws in its palette colour somewhere on the chart.
        let line = theme.series.palette[0];
        let target = [line.r, line.g, line.b].map(|c| (c * 255.0).round() as i32);
        assert!(
            rgba.chunks_exact(4)
                .any(|p| (0..3).all(|i| (i32::from(p[i]) - target[i]).abs() <= 8)),
            "{scale}x: no pixel in the series colour"
        );
    }
}

// A theme is refused, changing neither the chart nor its series, when its
// draw style cannot draw a series (Constellation draws lines only with
// their markers), when it is made for another kind of chart, and when no
// kind has its name.
#[test]
fn a_theme_is_refused_for_its_draw_style_and_for_another_kind() {
    use renderer::theme::{ChartKind, SubplotScope, Theme, ThemeError, ThemeTarget};
    let mut series = [SeriesConfig::line("sine", "x", "y").label("sin x")];
    let mut chart = Chart::sized(480, 320)
        .with_title("A sine wave")
        .with_legend_from(&series);
    let (config, declared) = (chart.config().clone(), series.clone());

    let constellation =
        Theme::named(ChartKind::Line, "constellation").expect("the line kind's constellation");
    assert_eq!(
        chart.apply_custom_theme(&constellation, &mut series),
        Err(ThemeError::UnsupportedDrawStyle {
            series_id: "sine".into()
        })
    );
    assert_eq!(chart.config(), &config);
    assert_eq!(series, declared);

    let histogram =
        Theme::named(ChartKind::Histogram, "gallery").expect("the histogram kind's gallery");
    assert_eq!(
        chart.apply_custom_theme(&histogram, &mut series),
        Err(ThemeError::WrongKind {
            name: None,
            theme: ChartKind::Histogram,
            chart: ChartKind::Line,
            layer: renderer::LayerRef::Base,
        })
    );
    assert_eq!(chart.config(), &config);
    assert_eq!(series, declared);

    // By name: the draw style, a name of another kind's catalogue and a
    // name no kind has.
    assert_eq!(
        chart.apply_theme("constellation", &mut series),
        Err(ThemeError::UnsupportedDrawStyle {
            series_id: "sine".into()
        })
    );
    assert_eq!(
        chart.apply_theme("parchment", &mut series),
        Err(ThemeError::WrongKind {
            name: Some("parchment".into()),
            theme: ChartKind::Feynman,
            chart: ChartKind::Line,
            layer: renderer::LayerRef::Base,
        })
    );
    assert_eq!(
        chart.apply_theme("neon", &mut series),
        Err(ThemeError::UnknownName {
            name: "neon".into(),
            target: ThemeTarget::Chart(ChartKind::Line),
            names: Theme::names(ChartKind::Line),
        })
    );
    assert_eq!(chart.config(), &config);
    assert_eq!(series, declared);

    // The same chart takes a theme of its kind that its series can draw in.
    let gallery = Theme::named(ChartKind::Line, "gallery").expect("the line kind's gallery");
    assert_eq!(
        chart.apply_custom_theme_to(&gallery, &mut series, SubplotScope::All),
        Ok(())
    );
    assert_eq!(chart.config().chart_title.text.color, gallery.chrome.ink);
}

// The chart answers the theme questions about itself (#295): its kind, the
// names it takes, the theme a name resolves to for editing, and a name
// applied to some subplots only.
#[test]
fn a_chart_names_its_kind_and_themes() {
    use renderer::theme::{ChartKind, SubplotScope, ThemeError, ThemeFamily};
    let mut series = vec![SeriesConfig::line("sine", "x", "y").label("sin x")];
    let mut chart = Chart::sized(480, 320).with_legend_from(&series);
    assert_eq!(chart.chart_kind(&[]), None);
    assert_eq!(chart.theme_names(&[]), ThemeFamily::NAMES);
    assert_eq!(chart.chart_kind(&series), Some(ChartKind::Line));
    let names = chart.theme_names(&series);
    assert_eq!(names[..4], *ThemeFamily::NAMES);
    assert!(names.contains(&"editorial") && names.contains(&"default"));

    // A member to edit, then applied as a custom theme.
    let mut theme = chart.theme("midnight", &series).expect("a family member");
    assert_eq!(theme, ThemeFamily::Midnight.theme(ChartKind::Line));
    let orange = Color::from_rgb8(255, 120, 0);
    theme.series.palette[0] = orange;
    chart
        .apply_custom_theme(&theme, &mut series)
        .expect("a line theme on a line chart");
    let renderer::DataRenderType::Xy(xy) = &series[0].render_type else {
        panic!("an x-y series");
    };
    assert_eq!(xy.line.as_ref().map(|line| line.line_color), Some(orange));
    assert_eq!(
        chart.theme("editorial", &series).map(|theme| theme.kind),
        Ok(ChartKind::Line)
    );
    assert!(matches!(
        chart.theme("neon", &series),
        Err(ThemeError::UnknownName { .. })
    ));
    assert!(matches!(
        chart.theme("parchment", &series),
        Err(ThemeError::WrongKind { .. })
    ));

    // A subplot of another kind: the whole document takes a family, and a
    // scope leaves the subplot alone.
    let mut config = chart.config().clone();
    config.subplots.push(renderer::Subplot::new(
        renderer::Config::default(),
        &["dots"],
    ));
    series.push(SeriesConfig::scatter("dots", "x", "y"));
    let mut chart = Chart::new(config);
    let before = chart.config().subplots[0].clone();
    chart
        .apply_theme_to("cool-slate", &mut series, SubplotScope::None)
        .expect("the base chart alone");
    // The layout is fitted again; the subplot's look stays.
    let cell = &chart.config().subplots[0].config;
    assert_eq!(cell.grid, before.config.grid);
    assert_eq!(
        cell.bottom_x.label_style.color,
        before.config.bottom_x.label_style.color
    );
    assert_eq!(cell.background, before.config.background);
    chart
        .apply_theme("cool-slate", &mut series)
        .expect("a family over both kinds");
    assert_eq!(
        chart.config().subplots[0].config.background,
        ThemeFamily::CoolSlate
            .theme(ChartKind::Scatter)
            .chrome
            .background
    );
}

// A custom theme without a scope themes the charts of its kind and leaves
// the others; with a scope it is the theme of every chart in scope. A scope
// past the subplots is refused with the index (#295).
#[test]
fn a_custom_theme_themes_the_charts_of_its_kind() {
    use renderer::theme::{ChartKind, SubplotIndexError, SubplotScope, ThemeError, ThemeFamily};
    let mut series = vec![
        SeriesConfig::line("sine", "x", "y"),
        SeriesConfig::scatter("dots", "x", "y"),
    ];
    let mut config = Chart::sized(480, 320).config().clone();
    config.subplots.push(renderer::Subplot::new(
        renderer::Config::default(),
        &["dots"],
    ));
    let mut chart = Chart::new(config);
    let (before, before_series) = (chart.config().clone(), series.clone());

    // The inset's theme by name: its own kind's member.
    assert_eq!(
        chart.subplot_theme(0, "midnight", &series),
        Ok(ThemeFamily::Midnight.theme(ChartKind::Scatter))
    );
    assert_eq!(
        chart.subplot_theme(2, "midnight", &series),
        Err(ThemeError::SubplotIndex(SubplotIndexError {
            index: 2,
            count: 1
        }))
    );
    assert!(matches!(
        chart.subplot_theme(0, "editorial", &series),
        Err(ThemeError::WrongKind {
            chart: ChartKind::Scatter,
            ..
        })
    ));
    let mut scatter = chart
        .subplot_theme(0, "midnight", &series)
        .expect("a member");
    scatter.series.palette[0] = Color::from_rgb8(255, 120, 0);
    chart
        .apply_custom_theme(&scatter, &mut series)
        .expect("the scatter subplot takes it");
    assert_eq!(
        chart.config().background,
        before.background,
        "the line base stays"
    );
    assert_eq!(series[0], before_series[0]);
    assert_eq!(
        chart.config().subplots[0].config.background,
        scatter.chrome.background
    );
    // With a scope, the line base refuses it.
    assert!(matches!(
        chart.apply_custom_theme_to(&scatter, &mut series, SubplotScope::All),
        Err(ThemeError::WrongKind {
            name: None,
            theme: ChartKind::Scatter,
            chart: ChartKind::Line,
            ..
        })
    ));
    // A scope past the subplots.
    let line = ThemeFamily::Midnight.theme(ChartKind::Line);
    assert_eq!(
        chart.apply_custom_theme_to(&line, &mut series, SubplotScope::Only(&[3])),
        Err(ThemeError::SubplotIndex(SubplotIndexError {
            index: 3,
            count: 1
        }))
    );
    assert_eq!(
        chart.apply_theme_to("gallery", &mut series, SubplotScope::Only(&[3])),
        Err(ThemeError::SubplotIndex(SubplotIndexError {
            index: 3,
            count: 1
        }))
    );
    // No chart of a polar theme's kind.
    let polar = ThemeFamily::Midnight.theme(ChartKind::Polar);
    assert!(matches!(
        chart.apply_custom_theme(&polar, &mut series),
        Err(ThemeError::WrongKind {
            layer: renderer::LayerRef::Base,
            ..
        })
    ));
}

// The subplot text mode on the chart: read, set and the factor it draws
// with; an index past the subplots is a typed error, never a panic (#292,
// #295).
#[test]
fn a_chart_reads_and_sets_its_subplot_text_mode() {
    use renderer::SubplotTextScale;
    use renderer::theme::SubplotIndexError;
    let mut config = Chart::sized(960, 640).config().clone();
    let mut cell = renderer::Subplot::new(renderer::Config::default(), &[]);
    cell.config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
        x: 600,
        y: 40,
        width: 240,
        height: 160,
    });
    config.subplots.push(cell);
    let mut chart = Chart::new(config);
    assert_eq!(chart.subplot_text_scale(0), Some(SubplotTextScale::Auto));
    let factor = chart.subplot_text_factor(0).expect("subplot 0");
    assert!(factor < 1.0, "{factor}");
    chart
        .set_subplot_text_scale(0, SubplotTextScale::Fixed)
        .expect("subplot 0");
    assert_eq!(chart.subplot_text_scale(0), Some(SubplotTextScale::Fixed));
    assert_eq!(chart.subplot_text_factor(0), Some(1.0));
    let before = chart.config().clone();
    assert_eq!(
        chart.set_subplot_text_scale(1, SubplotTextScale::Auto),
        Err(SubplotIndexError { index: 1, count: 1 })
    );
    assert_eq!(chart.config(), &before);
    assert_eq!(chart.subplot_text_scale(1), None);
    assert_eq!(chart.subplot_text_factor(1), None);
}

// A radial chart takes a theme by name, a family's radial member too; an
// unknown name lists the radial names and changes nothing (#295).
#[test]
fn a_radial_chart_takes_a_theme_by_name() {
    use renderer::theme::{RadialTheme, ThemeError, ThemeFamily, ThemeTarget};
    let mut chart = RadialChart::default();
    chart.slices = vec![
        RadialSlice::new("A", 1.0, Color::BLACK),
        RadialSlice::new("B", 2.0, Color::BLACK),
    ];
    let before = chart.clone();
    assert_eq!(
        chart.apply_theme("neon"),
        Err(ThemeError::UnknownName {
            name: "neon".into(),
            target: ThemeTarget::Radial,
            names: RadialTheme::names(),
        })
    );
    assert_eq!(chart, before);
    chart.apply_theme("midnight").expect("a family");
    assert_eq!(chart.background, ThemeFamily::Midnight.radial().background);
    chart.apply_theme("ceramic").expect("a radial theme");
    assert_eq!(
        chart.slices[1].color,
        RadialTheme::named("ceramic").unwrap().palette[1]
    );
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
    let mut resized = Chart::sized(10, 10).with_area(5, 6, 300, 180);
    let area = resized.config().chart_area.0;
    assert_eq!((area.x, area.y, area.width, area.height), (5, 6, 300, 180));
    resized.set_area(0, 0, 640, 480);
    let area = resized.config().chart_area.0;
    assert_eq!((area.x, area.y, area.width, area.height), (0, 0, 640, 480));

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

// The chart part builders write the document: annotations appended, the
// tick label format on both value axes, every axis keeping its tick
// labels, every grid line off, the colour
// bar's title, polar coordinates and a subplot. The document validates and
// draws.
#[test]
fn chart_part_builders_write_the_document() {
    use renderer::config::LabelClearanceMode;
    use renderer::format::LabelFormat;
    use renderer::{Anchor, Annotation, AnnotationAxis, CellRef, PolarOptions, Subplot};

    let red = Color::from_rgb8(200, 30, 30);
    let mut chart = Chart::sized(480, 320)
        .with_annotation(Annotation::reference_line(AnnotationAxis::Y, 0.5, red, 1.5))
        .with_annotation(Annotation::span(
            AnnotationAxis::X,
            2.0,
            4.0,
            red.with_alpha(0.15),
        ))
        .with_annotation(Annotation::text(
            Anchor::Ratio { x: 0.05, y: 0.05 },
            "note",
            red,
            16.0,
        ))
        .without_grid();
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-1.2, 1.2);
    // `set_x_range` / `set_y_range` choose a format, so the format follows.
    let chart = chart
        .with_label_format(LabelFormat::Degrees)
        .with_label_clearance(LabelClearanceMode::Keep);
    let config = chart.config();
    assert_eq!(config.annotations.items.len(), 3);
    assert_eq!(config.bottom_x.label_style.format, LabelFormat::Degrees);
    assert_eq!(config.left_y.label_style.format, LabelFormat::Degrees);
    for axis in [
        &config.bottom_x,
        &config.top_x,
        &config.left_y,
        &config.right_y,
    ] {
        assert_eq!(axis.label_style.clearance, LabelClearanceMode::Keep);
    }
    let grid = &config.grid;
    assert!(!grid.show_major_x && !grid.show_major_y);
    assert!(!grid.show_minor_x && !grid.show_minor_y);
    assert!(config.validate().is_ok(), "{:?}", config.validate());

    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let (x, y) = sine();
    renderer.add_column("x", &x).unwrap();
    renderer.add_column("y", &y).unwrap();
    let series = [SeriesConfig::line("sine", "x", "y")];
    for scale in [1.0, 2.0] {
        let png = renderer
            .export_png_blocking(
                ExportSource::Chart {
                    chart: &chart,
                    series: &series,
                },
                ExportOptions::document(scale),
            )
            .unwrap();
        assert!(png.starts_with(b"\x89PNG"));
    }

    // Without a colour bar the title builder changes nothing; with one it
    // shows the title in the axis title's style.
    let plain = Chart::sized(200, 120);
    assert_eq!(
        plain.config(),
        Chart::sized(200, 120).with_colorbar_title("z").config()
    );
    let mut barred = Chart::sized(200, 120);
    barred.config_mut().colorbar = Some(renderer::default::default_colorbar_options());
    let barred = barred.with_colorbar_title("Speed");
    let title = &barred.config().colorbar.as_ref().unwrap().axis.title_option;
    assert!(title.visible);
    let text: String = title.text.segments.iter().map(|s| s.text).collect();
    assert_eq!(text, "Speed");

    let polar = Chart::sized(300, 300).with_polar(PolarOptions::default());
    assert_eq!(polar.config().polar(), Some(&PolarOptions::default()));

    let cell = Chart::placed(480, 0, 480, 320).config().clone();
    let overlay = renderer::config::AxisOptions::overlay_y("Current (A)", red);
    let document = Chart::sized(960, 320)
        .with_overlay(overlay, &["current"])
        .with_subplot(Subplot::new(cell.clone(), &["b"]).with_link_x(CellRef::Parent));
    let config = document.config();
    assert_eq!(config.subplots.len(), 1);
    assert_eq!(config.subplots[0].config, cell);
    assert_eq!(config.subplots[0].series, ["b"]);
    assert_eq!(config.subplots[0].link_x, Some(CellRef::Parent));
    assert_eq!(config.overlays[0].y_axis.label_style.color, red);
    assert!(config.validate().is_ok(), "{:?}", config.validate());
}

// `with_x_unit` / `with_y_unit` (#273) print `bottom_x` and `left_y` in
// another unit: the unit is stored, the major spacing is chosen for the
// shown range, as `set_x_range` / `set_y_range` choose it, so the ticks
// fall on round shown values; the paired axes keep theirs, as
// `with_label_format` leaves them. A refused unit is kept for
// `validate_chart` to name, without a panic.
#[test]
fn chart_unit_helpers_print_the_axes_in_shown_units() {
    use renderer::RuleKind;
    use renderer::config::{AxisScale, AxisUnit};
    use renderer::tick::{axis_major_spacing, major_ticks, shown_tick_value};

    let mut chart = Chart::sized(480, 320);
    // ADC counts on x, kelvin on y.
    chart.set_x_range(0.0, 32768.0);
    chart.set_y_range(250.0, 350.0);
    let (x_spacing, y_spacing) = (
        chart.config().bottom_x.major_spacing,
        chart.config().left_y.major_spacing,
    );
    let chart = chart
        .with_x_unit(5.0 / 32768.0, 0.0)
        .with_y_unit(1.0, -273.15);
    let config = chart.config();
    assert_eq!(
        config.bottom_x.unit,
        Some(AxisUnit::new(5.0 / 32768.0, 0.0))
    );
    assert_eq!(config.left_y.unit, Some(AxisUnit::new(1.0, -273.15)));
    assert_eq!((config.top_x.unit, config.right_y.unit), (None, None));
    // The data range stays in data units.
    assert_eq!((config.bottom_x.min, config.bottom_x.max), (0.0, 32768.0));
    assert_eq!((config.left_y.min, config.left_y.max), (250.0, 350.0));

    // 0..5 V: one volt per major, where the counts had their own spacing.
    assert_eq!(config.bottom_x.major_spacing, 1.0);
    assert_ne!(x_spacing, 1.0);
    let volts: Vec<f64> = major_ticks(&config.bottom_x)
        .map(|v| shown_tick_value(&config.bottom_x, v))
        .collect();
    assert_eq!(volts, [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    // -23.15..76.85 °C: the spacing of the shown range, every tick on a
    // round shown value.
    let celsius = &config.left_y;
    let spacing = axis_major_spacing(&AxisScale::Linear, 250.0 - 273.15, 350.0 - 273.15);
    assert_eq!(celsius.major_spacing, spacing);
    let shown: Vec<f64> = major_ticks(celsius)
        .map(|v| shown_tick_value(celsius, v))
        .collect();
    assert!(shown.len() >= 2, "{shown:?}");
    for value in &shown {
        let steps = value / spacing;
        assert!((steps - steps.round()).abs() < 1e-9, "{value} on {spacing}");
        assert!((-23.15..=76.85).contains(value), "{value}");
    }
    assert!(y_spacing > 0.0);
    assert_eq!(config.validate_chart(), Ok(()));

    // A later range keeps the unit and picks the spacing for its shown
    // range, as the helper did.
    let mut ranged = Chart::new(config.clone());
    ranged.set_x_range(0.0, 65536.0);
    assert_eq!(ranged.config().bottom_x.unit, config.bottom_x.unit);
    assert_eq!(
        ranged.config().bottom_x.major_spacing,
        axis_major_spacing(&AxisScale::Linear, 0.0, 10.0)
    );

    // Drawn at 1x and 2x.
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let x: Vec<f64> = (0..64).map(|i| f64::from(i) * 512.0).collect();
    let y: Vec<f64> = (0..64).map(|i| 250.0 + f64::from(i) * 1.5).collect();
    renderer.add_column("counts", &x).unwrap();
    renderer.add_column("kelvin", &y).unwrap();
    let series = [SeriesConfig::line("t", "counts", "kelvin")];
    for scale in [1.0, 2.0] {
        let png = renderer
            .export_png_blocking(
                ExportSource::Chart {
                    chart: &chart,
                    series: &series,
                },
                ExportOptions::document(scale),
            )
            .unwrap();
        assert!(png.starts_with(b"\x89PNG"));
    }

    // A unit the axis refuses is stored as given and refused by
    // validation; the spacing stays as the range chose it.
    let base = || {
        let mut base = Chart::sized(480, 320);
        base.set_x_range(0.0, 32768.0);
        base.set_y_range(250.0, 350.0);
        base
    };
    let refused = base().with_x_unit(0.0, 1.0);
    assert_eq!(
        refused.config().bottom_x.unit,
        Some(AxisUnit::new(0.0, 1.0))
    );
    assert_eq!(refused.config().bottom_x.major_spacing, x_spacing);
    let error = refused.config().validate_chart().unwrap_err();
    assert_eq!(
        (error.field, error.kind),
        ("bottom_x", RuleKind::AxisUnitScale)
    );
    let refused = base().with_y_unit(1.0, f64::NAN);
    assert_eq!(refused.config().left_y.major_spacing, y_spacing);
    let error = refused.config().validate_chart().unwrap_err();
    assert_eq!(
        (error.field, error.kind),
        ("left_y", RuleKind::AxisUnitOffset)
    );
}
