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

    // A name without a known extension saves with the format named (#297),
    // and the refusal of the extension says so.
    let named = scratch("pie.image");
    let refused = renderer
        .save_blocking(&chart, (240, 240), options, &named)
        .unwrap_err();
    assert!(matches!(refused, SaveError::UnknownFormat(_)));
    assert!(
        refused.to_string().contains("save_as_blocking"),
        "{refused}"
    );
    renderer
        .save_as_blocking(&chart, (240, 240), options, &named, ExportFormat::Png)
        .unwrap();
    assert_eq!(
        std::fs::read(&named).unwrap(),
        std::fs::read(&path).unwrap()
    );
}

// `Renderer::fit_ranges` (#297): every axis the series use, fitted from the
// registered columns' statistics in one call.

/// A headless renderer with five columns of eleven values: `t` 0..10, `a`
/// -1..1, `b` 100..200, `u` 20..30 and `z` 5..10.
fn fit_renderer() -> Renderer {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let column = |from: f64, step: f64| -> Vec<f64> {
        (0..=10).map(|i| from + step * f64::from(i)).collect()
    };
    for (id, from, step) in [
        ("t", 0.0, 1.0),
        ("a", -1.0, 0.2),
        ("b", 100.0, 10.0),
        ("u", 20.0, 1.0),
        ("z", 5.0, 0.5),
    ] {
        renderer.add_column(id, &column(from, step)).unwrap();
    }
    renderer
}

/// `axis` runs from `min` to `max`, to rounding.
#[track_caller]
fn assert_range(axis: &renderer::config::AxisOptions, min: f64, max: f64) {
    assert!(
        (axis.min - min).abs() < 1e-9 && (axis.max - max).abs() < 1e-9,
        "{:?} is not {:?}",
        (axis.min, axis.max),
        (min, max)
    );
}

// The base chart: both axes take the columns' extents with 5% padding, as
// the existing layer fit gives them; the fit marks the chart changed, a
// second one changes nothing, and a refused one leaves the chart alone.
#[test]
fn fit_ranges_fits_the_base_chart_and_marks_it_changed() {
    let renderer = fit_renderer();
    let mut series = [SeriesConfig::line("a", "t", "a")];
    let mut chart = Chart::sized(480, 320);
    assert_range(&chart.config().bottom_x, 0.0, 1.0);
    chart.consume_data_dirty();
    chart.consume_raster_dirty();
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    assert_range(&chart.config().bottom_x, -0.5, 10.5);
    assert_range(&chart.config().top_x, -0.5, 10.5);
    assert_range(&chart.config().left_y, -1.1, 1.1);
    assert!(chart.data_dirty() && chart.raster_dirty());
    assert!(chart.config().validate().is_ok());

    let mut by_layer = Chart::sized(480, 320);
    by_layer
        .auto_fit_layer_union(
            renderer.pool(),
            renderer::LayerRef::Base,
            &series,
            renderer::FIT_RANGES_PADDING,
        )
        .unwrap();
    assert_eq!(chart.config(), by_layer.config(), "the existing fit path");

    chart.consume_data_dirty();
    chart.consume_raster_dirty();
    let fitted = chart.config().clone();
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    assert_eq!(chart.config(), &fitted, "idempotent");
    assert!(!chart.data_dirty() && !chart.raster_dirty());

    // A column that was never registered, and a monotonic series.
    let mut missing = [
        SeriesConfig::line("a", "t", "b"),
        SeriesConfig::line("m", "t", "nowhere"),
    ];
    let error = renderer.fit_ranges(&mut chart, &mut missing).unwrap_err();
    assert!(
        matches!(&error, renderer::FiggyError::UnknownColumn { id } if id == "nowhere"),
        "{error}"
    );
    assert!(error.to_string().contains("add_column"), "{error}");
    let mut monotonic = [SeriesConfig::monotonic_line("m", "daq", 0)];
    assert!(matches!(
        renderer.fit_ranges(&mut chart, &mut monotonic),
        Err(renderer::FiggyError::InvalidSeriesConfig {
            kind: renderer::RuleKind::MonotonicMappingEdit,
            ..
        })
    ));
    assert_eq!(chart.config(), &fitted, "a refused fit changes nothing");
    assert!(!chart.data_dirty() && !chart.raster_dirty());
}

// A chart without series, and one whose series are all DataLess with
// nothing to fit, keep the placeholder range and are not marked changed.
#[test]
fn fit_ranges_without_series_changes_nothing() {
    let renderer = fit_renderer();
    let mut chart = Chart::sized(480, 320)
        .with_overlay(renderer::config::Config::default().left_y, &[])
        .with_subplot(renderer::Subplot::new(
            Chart::placed(240, 0, 240, 160).config().clone(),
            &[],
        ));
    let before = chart.config().clone();
    chart.consume_data_dirty();
    chart.consume_raster_dirty();
    renderer.fit_ranges(&mut chart, &mut []).unwrap();
    assert_eq!(chart.config(), &before);
    assert!(!chart.data_dirty() && !chart.raster_dirty());
}

// An overlay layer's own y axis fits its series; the base y axis fits the
// base series only, and the x axis both share fits both.
#[test]
fn fit_ranges_fits_an_overlay_layer() {
    let renderer = fit_renderer();
    let mut series = [
        SeriesConfig::line("a", "t", "a"),
        SeriesConfig::line("b", "u", "b"),
    ];
    let axis = renderer::config::AxisOptions::overlay_y("B", Color::BLACK);
    let mut chart = Chart::sized(480, 320).with_overlay(axis, &["b"]);
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    let config = chart.config();
    // The shared x axis holds `t` (0..10) and `u` (20..30).
    assert_range(&config.bottom_x, -1.5, 31.5);
    assert_range(&config.left_y, -1.1, 1.1);
    assert_range(&config.overlays[0].y_axis, 95.0, 205.0);
    assert!(config.validate().is_ok());
}

// Each subplot's axes fit its own series, and the parent's fit its own.
#[test]
fn fit_ranges_fits_every_subplot() {
    let renderer = fit_renderer();
    let mut series = [
        SeriesConfig::line("a", "t", "a"),
        SeriesConfig::scatter("b", "u", "b"),
        SeriesConfig::line("z", "t", "z"),
    ];
    let cell = |x: u32| Chart::placed(x, 0, 320, 240).config().clone();
    let inner = Chart::new(cell(640))
        .with_overlay(
            renderer::config::AxisOptions::overlay_y("Z", Color::BLACK),
            &["z"],
        )
        .config()
        .clone();
    let mut chart = Chart::sized(320, 240)
        .with_subplot(renderer::Subplot::new(cell(320), &["b"]))
        .with_subplot(renderer::Subplot::new(inner, &["z"]));
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    let config = chart.config();
    assert_range(&config.bottom_x, -0.5, 10.5);
    assert_range(&config.left_y, -1.1, 1.1);
    let first = &config.subplots[0].config;
    assert_range(&first.bottom_x, 19.5, 30.5);
    assert_range(&first.left_y, 95.0, 205.0);
    // The second subplot draws its series on an overlay: the overlay's y
    // axis is fitted, and the subplot's x axis, which the overlay follows.
    let second = &config.subplots[1].config;
    assert_range(&second.bottom_x, -0.5, 10.5);
    assert_range(&second.left_y, 0.0, 1.0);
    assert_range(&second.overlays[0].y_axis, 4.75, 10.25);
    assert!(config.validate().is_ok());
}

// A 3D chart's z axis (`right_y`) fits the height columns.
#[test]
fn fit_ranges_fits_the_z_axis_of_a_3d_chart() {
    let renderer = fit_renderer();
    let mut series = [SeriesConfig::scatter("p", "t", "a").z("z")];
    let mut chart = Chart::sized(480, 320);
    chart
        .config_mut()
        .set_view_3d(Some(renderer::View3d::default()));
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    let config = chart.config();
    assert_range(&config.bottom_x, -0.5, 10.5);
    assert_range(&config.left_y, -1.1, 1.1);
    assert_range(&config.right_y, 4.75, 10.25);
}

// `fit_ranges` also sizes every auto-scaled vector series to the spacing
// of its arrows over the fitted data area (#357), writing the scale into
// the series and consuming `auto_scale`: the default flow (fit, export)
// draws arrows, not specks, a second fit keeps the stored scale, the export
// draws the series as stored at 1x and 2x, and a refused fit writes no
// series.
#[test]
fn fit_ranges_sizes_auto_scaled_arrows() {
    use renderer::data_config::{VectorScale, VectorSeries};
    let mut renderer = fit_renderer();
    let scale_of = |series: &SeriesConfig| match &series.render_type {
        renderer::DataRenderType::Vector(VectorSeries { style, .. }) => {
            let VectorScale::Pixels { px_per_unit } = style.scale else {
                panic!("a pixel scale");
            };
            (px_per_unit, style.auto_scale)
        }
        _ => panic!("a vector series"),
    };
    let mut series = [
        SeriesConfig::vectors("v", "t", "a", "u", "a"),
        SeriesConfig::line("l", "t", "a"),
    ];
    let before = scale_of(&series[0]);
    assert!(before.1, "the constructor asks for a fit");
    let mut chart = Chart::sized(480, 320);
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    assert_range(&chart.config().bottom_x, -0.5, 10.5);
    let mut expected = [SeriesConfig::vectors("v", "t", "a", "u", "a")];
    renderer
        .fit_vector_scales(chart.config(), &mut expected)
        .unwrap();
    assert_eq!(series[0], expected[0], "the fitted data area's scale");
    let (scale, flag) = scale_of(&series[0]);
    assert!(!flag, "consumed");
    assert!(scale > before.0 * 2.0, "{scale} draws arrows, not specks");

    let stored = series.clone();
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    assert_eq!(series, stored, "the stored scale stays");
    let held = Chart::as_given(chart.config().clone());
    for scale in [1.0, 2.0] {
        let options = ExportOptions::panel(scale).with_clear(Color::WHITE);
        let fitted = renderer
            .export_rgba_blocking(ExportSource::chart(&chart, &series), options)
            .unwrap();
        let own = renderer
            .export_rgba_blocking(ExportSource::chart(&held, &stored), options)
            .unwrap();
        assert_eq!(fitted.rgba, own.rgba, "{scale}x");
    }

    let mut refused = [
        SeriesConfig::vectors("v", "t", "a", "u", "a"),
        SeriesConfig::vectors("w", "t", "a", "nowhere", "a"),
    ];
    let kept = refused.clone();
    let fitted = chart.config().clone();
    assert!(renderer.fit_ranges(&mut chart, &mut refused).is_err());
    assert_eq!(refused, kept);
    assert_eq!(chart.config(), &fitted);
}

// The fit is an SSOT edit: an export after it draws the new ranges, the
// same image as a chart given those ranges by hand, and a registered chart
// that adopts the fitted config needs its raster and its draw again. (The
// chart is registered as given: `register_chart` would fit it once itself.)
#[test]
fn a_chart_redraws_with_the_fitted_ranges() {
    let mut renderer = fit_renderer();
    let mut series = [SeriesConfig::line("a", "t", "a").width(2.0)];
    let mut chart = Chart::sized(320, 200);
    let options = ExportOptions::document(1.0);
    let id = renderer
        .register_chart_as_given(chart.config().clone(), series.to_vec())
        .unwrap();
    let stamp = renderer.chart_render_stamp(id).unwrap();
    let placeholder = renderer
        .export_png_blocking(ExportSource::chart(&chart, &series), options)
        .unwrap();

    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    for scale in [1.0, 2.0] {
        let options = ExportOptions::document(scale);
        let fitted = renderer
            .export_png_blocking(ExportSource::chart(&chart, &series), options)
            .unwrap();
        let mut by_hand = Chart::sized(320, 200);
        by_hand.set_x_range(-0.5, 10.5);
        by_hand.set_y_range(-1.1, 1.1);
        assert_eq!(by_hand.config(), chart.config());
        let expected = renderer
            .export_png_blocking(ExportSource::chart(&by_hand, &series), options)
            .unwrap();
        assert!(fitted == expected, "{scale}x: the fitted ranges are drawn");
        if scale == 1.0 {
            assert!(fitted != placeholder, "the placeholder range is replaced");
        }
    }

    renderer
        .set_chart_config(id, chart.config().clone())
        .unwrap();
    let after = renderer.chart_render_stamp(id).unwrap();
    assert!(after.needs_draw_since(Some(&stamp)));
    assert!(after.needs_raster_since(Some(&stamp)));
    assert_eq!(renderer.chart_config(id).unwrap(), chart.config());
}

// The fit writes ranges only: the fitted document and its series
// serialize and read back unchanged.
#[cfg(feature = "serde")]
#[test]
fn a_fitted_chart_round_trips() {
    let renderer = fit_renderer();
    let mut series = [
        SeriesConfig::line("a", "t", "a"),
        SeriesConfig::line("b", "u", "b"),
    ];
    let axis = renderer::config::AxisOptions::overlay_y("B", Color::BLACK);
    let mut chart = Chart::sized(480, 320)
        .with_overlay(axis, &["b"])
        .with_subplot(renderer::Subplot::new(
            Chart::placed(480, 0, 240, 160).config().clone(),
            &[],
        ));
    let untouched = series.clone();
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    assert_eq!(series, untouched);
    let json = serde_json::to_string(chart.config()).unwrap();
    let back: renderer::config::Config = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, chart.config());
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
    let series_json = serde_json::to_string(&series).unwrap();
    let series_back: Vec<SeriesConfig> = serde_json::from_str(&series_json).unwrap();
    assert_eq!(series_back, series);
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
    use renderer::theme::{ChartKind, ThemeFamily};
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let (x, y) = sine();
    renderer.add_column("x", &x).unwrap();
    renderer.add_column("y", &y).unwrap();
    let theme = ThemeFamily::Midnight.theme(ChartKind::Line);
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

// A theme is refused, changing neither the chart nor its series, when a
// custom theme's draw style cannot draw a series (Constellation draws lines
// only with their markers), when a custom theme is made for another kind of
// chart, and when a name is not a family.
#[test]
fn a_theme_is_refused_for_its_draw_style_and_for_another_kind() {
    use renderer::config::DrawStyle;
    use renderer::theme::{ChartKind, Theme, ThemeError, ThemeFamily, ThemeScope};
    let mut series = [SeriesConfig::line("sine", "x", "y").label("sin x")];
    let mut chart = Chart::sized(480, 320)
        .with_title("A sine wave")
        .with_legend_from(&series);
    let (config, declared) = (chart.config().clone(), series.clone());

    let mut constellation = Theme::default();
    constellation.chrome.draw_style = DrawStyle::Constellation(Default::default());
    assert_eq!(
        chart.apply_custom_theme(&constellation, &mut series),
        Err(ThemeError::UnsupportedDrawStyle {
            series_id: "sine".into()
        })
    );
    assert_eq!(chart.config(), &config);
    assert_eq!(series, declared);

    let histogram = ThemeFamily::Gallery.theme(ChartKind::Histogram);
    assert_eq!(
        chart.apply_custom_theme(&histogram, &mut series),
        Err(ThemeError::WrongKind {
            theme: ChartKind::Histogram,
            chart: ChartKind::Line,
            layer: renderer::LayerRef::Base,
        })
    );
    assert_eq!(chart.config(), &config);
    assert_eq!(series, declared);

    // By name: only the families are names.
    for name in ["neon", "constellation", "default"] {
        assert_eq!(
            chart.apply_theme(name, &mut series),
            Err(ThemeError::UnknownName {
                name: name.into(),
                names: ThemeFamily::NAMES,
            })
        );
    }
    assert_eq!(chart.config(), &config);
    assert_eq!(series, declared);

    // The same chart takes a theme of its kind that its series can draw in.
    let gallery = ThemeFamily::Gallery.theme(ChartKind::Line);
    assert_eq!(
        chart.apply_custom_theme_to(&gallery, &mut series, ThemeScope::All),
        Ok(())
    );
    assert_eq!(chart.config().chart_title.text.color, gallery.chrome.ink);
}

// The chart answers the theme questions about itself (#295): its kind, the
// names it takes, the theme a name resolves to for editing, and a name
// applied to some subplots only.
#[test]
fn a_chart_names_its_kind_and_themes() {
    use renderer::theme::{ChartKind, ThemeError, ThemeFamily, ThemeScope};
    let mut series = vec![SeriesConfig::line("sine", "x", "y").label("sin x")];
    let mut chart = Chart::sized(480, 320).with_legend_from(&series);
    assert_eq!(chart.chart_kind(&[]), None);
    assert_eq!(chart.theme_names(), ThemeFamily::NAMES);
    assert_eq!(chart.chart_kind(&series), Some(ChartKind::Line));

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
    // A chart without series takes the line member.
    assert_eq!(
        chart.theme("gallery", &[]),
        Ok(ThemeFamily::Gallery.theme(ChartKind::Line))
    );
    for name in ["neon", "editorial"] {
        assert!(matches!(
            chart.theme(name, &series),
            Err(ThemeError::UnknownName { .. })
        ));
    }

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
        .apply_theme_to("cool-slate", &mut series, ThemeScope::Base)
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
    use renderer::theme::{ChartKind, SubplotIndexError, ThemeError, ThemeFamily, ThemeScope};
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
        Err(ThemeError::UnknownName { .. })
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
        chart.apply_custom_theme_to(&scatter, &mut series, ThemeScope::All),
        Err(ThemeError::WrongKind {
            theme: ChartKind::Scatter,
            chart: ChartKind::Line,
            ..
        })
    ));
    // A scope past the subplots.
    let line = ThemeFamily::Midnight.theme(ChartKind::Line);
    assert_eq!(
        chart.apply_custom_theme_to(&line, &mut series, ThemeScope::BaseAnd(&[3])),
        Err(ThemeError::SubplotIndex(SubplotIndexError {
            index: 3,
            count: 1
        }))
    );
    assert_eq!(
        chart.apply_theme_to("gallery", &mut series, ThemeScope::BaseAnd(&[3])),
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

// A scope names any chart (#297): the base chart alone, a subplot alone,
// the base chart and subplots, or all, on a line chart with a scatter
// inset and an empty cell. A refusal changes nothing.
#[test]
fn a_theme_scope_names_the_base_chart_a_subplot_or_both() {
    use renderer::config::DrawStyle;
    use renderer::theme::{ChartKind, SubplotIndexError, ThemeError, ThemeFamily, ThemeScope};
    let declared_series = vec![
        SeriesConfig::line("sine", "x", "y"),
        SeriesConfig::scatter("dots", "x", "y"),
    ];
    let mut config = Chart::sized(480, 320).config().clone();
    for (x, ids) in [(480, &["dots"][..]), (720, &[][..])] {
        config.subplots.push(renderer::Subplot::new(
            Chart::placed(x, 0, 240, 160).config().clone(),
            ids,
        ));
    }
    let mut fitted = Chart::new(config);
    fitted.fit_layout();
    let declared = fitted.config().clone();
    let fresh = || (Chart::new(declared.clone()), declared_series.clone());
    let line = ThemeFamily::Midnight.theme(ChartKind::Line);
    let scatter = ThemeFamily::Midnight.theme(ChartKind::Scatter);
    // The look a theme writes on a chart: its background, frame and inks.
    let look = |chart: &renderer::Config| {
        (
            chart.background,
            chart.bottom_x.line_color,
            chart.bottom_x.label_style.color,
            chart.chart_title.text.color,
            chart.grid.clone(),
        )
    };
    let themed = |chart: &renderer::Config, theme: &renderer::theme::Theme| {
        chart.background == theme.chrome.background
            && chart.bottom_x.line_color == theme.chrome.frame_ink
    };

    // The base chart alone.
    let (mut chart, mut series) = fresh();
    chart
        .apply_custom_theme_to(&line, &mut series, ThemeScope::Base)
        .expect("the base chart alone");
    assert!(themed(chart.config(), &line));
    for index in 0..2 {
        assert_eq!(
            look(&chart.config().subplots[index].config),
            look(&declared.subplots[index].config)
        );
    }
    assert_ne!(series[0], declared_series[0]);
    assert_eq!(series[1], declared_series[1]);

    // A subplot alone: the scatter inset takes a scatter theme although the
    // base chart is a line chart, and the base chart keeps its look.
    let (mut chart, mut series) = fresh();
    chart
        .apply_custom_theme_to(&scatter, &mut series, ThemeScope::Subplots(&[0]))
        .expect("the inset alone");
    assert_eq!(look(chart.config()), look(&declared), "the base stays");
    assert!(themed(&chart.config().subplots[0].config, &scatter));
    assert_eq!(
        look(&chart.config().subplots[1].config),
        look(&declared.subplots[1].config)
    );
    assert_eq!(series[0], declared_series[0], "the base series stays");
    assert_ne!(series[1], declared_series[1]);
    // By name, the same scope gives the inset its own kind's member.
    let (mut named, mut named_series) = fresh();
    named
        .apply_theme_to("midnight", &mut named_series, ThemeScope::Subplots(&[0]))
        .expect("the inset alone, by name");
    assert_eq!(named.config(), chart.config());
    assert_eq!(named_series, series);

    // The base chart and a subplot.
    let (mut chart, mut series) = fresh();
    chart
        .apply_custom_theme_to(&line, &mut series, ThemeScope::BaseAnd(&[1]))
        .expect("the base chart and the empty cell");
    assert!(themed(chart.config(), &line));
    assert!(themed(&chart.config().subplots[1].config, &line));
    assert_eq!(
        look(&chart.config().subplots[0].config),
        look(&declared.subplots[0].config)
    );
    assert_eq!(series[1], declared_series[1]);

    // All: a family themes every chart; a custom theme is refused by the
    // first chart of another kind.
    let (mut chart, mut series) = fresh();
    chart
        .apply_theme_to("midnight", &mut series, ThemeScope::All)
        .expect("a family over every chart");
    assert!(themed(chart.config(), &line));
    assert!(themed(&chart.config().subplots[0].config, &scatter));
    let mut whole = fresh();
    whole.0.apply_theme("midnight", &mut whole.1).unwrap();
    assert_eq!(chart.config(), whole.0.config(), "the unscoped call");

    // Refusals: another kind in scope, an index past the subplots, a draw
    // style that cannot draw a series in scope. Nothing changes.
    let mut constellation = line.clone();
    constellation.chrome.draw_style = DrawStyle::Constellation(Default::default());
    let wrong = |theme: ChartKind, chart: ChartKind, layer| {
        Err(ThemeError::WrongKind {
            theme,
            chart,
            layer,
        })
    };
    let (line_kind, scatter_kind) = (ChartKind::Line, ChartKind::Scatter);
    let (base, inset) = (renderer::LayerRef::Base, renderer::LayerRef::Subplot(0));
    let past = Err(ThemeError::SubplotIndex(SubplotIndexError {
        index: 5,
        count: 2,
    }));
    for (theme, scope, expected) in [
        (
            &scatter,
            ThemeScope::Base,
            wrong(scatter_kind, line_kind, base),
        ),
        (
            &scatter,
            ThemeScope::BaseAnd(&[0]),
            wrong(scatter_kind, line_kind, base),
        ),
        (
            &scatter,
            ThemeScope::All,
            wrong(scatter_kind, line_kind, base),
        ),
        (
            &line,
            ThemeScope::Subplots(&[0]),
            wrong(line_kind, scatter_kind, inset),
        ),
        (
            &line,
            ThemeScope::BaseAnd(&[1, 0]),
            wrong(line_kind, scatter_kind, inset),
        ),
        (
            &line,
            ThemeScope::All,
            wrong(line_kind, scatter_kind, inset),
        ),
        (&line, ThemeScope::Subplots(&[5]), past.clone()),
        (&line, ThemeScope::BaseAnd(&[1, 5]), past.clone()),
        (
            &constellation,
            ThemeScope::BaseAnd(&[1]),
            Err(ThemeError::UnsupportedDrawStyle {
                series_id: "sine".into(),
            }),
        ),
    ] {
        let (mut chart, mut series) = fresh();
        assert_eq!(
            chart.apply_custom_theme_to(theme, &mut series, scope),
            expected,
            "{scope:?}"
        );
        assert_eq!(chart.config(), &declared, "{scope:?}");
        assert_eq!(series, declared_series, "{scope:?}");
    }
    let (mut chart, mut series) = fresh();
    for scope in [ThemeScope::Subplots(&[5]), ThemeScope::BaseAnd(&[5])] {
        assert_eq!(chart.apply_theme_to("gallery", &mut series, scope), past);
        assert_eq!(chart.config(), &declared);
    }
    // The draw style is checked for the series in scope only: the empty
    // cell alone takes the style the base chart's line cannot be drawn in.
    chart
        .apply_custom_theme_to(&constellation, &mut series, ThemeScope::Subplots(&[1]))
        .expect("no series in scope");
    assert_eq!(
        chart.config().subplots[1].config.draw_style,
        constellation.chrome.draw_style
    );
    assert_eq!(chart.config().draw_style, declared.draw_style);
    assert_eq!(series, declared_series);
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

// A radial chart takes a family by name, as its radial member; any other
// name lists the family names and changes nothing (#295). A custom radial
// theme is a value.
#[test]
fn a_radial_chart_takes_a_theme_by_name() {
    use renderer::theme::{RadialTheme, ThemeError, ThemeFamily};
    let mut chart = RadialChart::default();
    chart.slices = vec![
        RadialSlice::new("A", 1.0, Color::BLACK),
        RadialSlice::new("B", 2.0, Color::BLACK),
    ];
    let before = chart.clone();
    for name in ["neon", "ceramic"] {
        assert_eq!(
            chart.apply_theme(name),
            Err(ThemeError::UnknownName {
                name: name.into(),
                names: ThemeFamily::NAMES,
            })
        );
    }
    assert_eq!(chart, before);
    chart.apply_theme("midnight").expect("a family");
    assert_eq!(chart.background, ThemeFamily::Midnight.radial().background);
    let mut theme = ThemeFamily::Gallery.radial();
    theme.palette[1] = Color::from_rgb8(255, 120, 0);
    theme.apply(&mut chart);
    assert_eq!(chart.slices[1].color, theme.palette[1]);
    // The documented look undoes the family's background.
    RadialTheme::default().apply(&mut chart);
    assert_eq!(chart.background, before.background);
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

    // The title builder shows the title in the axis title's style, on the
    // chart's colour bar or, without one, on the default bar it adds (#297).
    // On `Fixed` charts, so the bar is compared field for field: a chart
    // that keeps its layout fitted fits the bar's band to the title too
    // (#357), as `fit_layout` would.
    let fixed = || {
        Chart::new(renderer::Config {
            layout_state: renderer::LayoutState::Fixed,
            ..Chart::sized(200, 120).config().clone()
        })
    };
    let mut barred = fixed();
    barred.config_mut().colorbar = Some(renderer::default::default_colorbar_options());
    for chart in [barred, fixed()] {
        let titled = chart.with_colorbar_title("Speed");
        let mut expected = renderer::default::default_colorbar_options();
        expected.axis.title_option = titled
            .config()
            .colorbar
            .as_ref()
            .unwrap()
            .axis
            .title_option
            .clone();
        assert_eq!(titled.config().colorbar.as_ref(), Some(&expected));
        let title = &expected.axis.title_option;
        assert!(title.visible);
        let text: String = title.text.segments.iter().map(|s| s.text).collect();
        assert_eq!(text, "Speed");
    }

    let polar = Chart::sized(300, 300).with_polar(PolarOptions::default());
    assert_eq!(polar.config().polar(), Some(&PolarOptions::default()));

    let cell = Chart::placed(480, 0, 480, 320).config().clone();
    let overlay = renderer::config::AxisOptions::overlay_y("Current (A)", red);
    // A `Fixed` document, so the subplot is compared as given: a document
    // that keeps its layout fitted fits the cell's bands with its own
    // (#357).
    let document = Chart::new(renderer::Config {
        layout_state: renderer::LayoutState::Fixed,
        ..Chart::sized(960, 320).config().clone()
    })
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

// The reproduction of #469 item 10: a theme keeps a `Fixed` layout (a
// dragged or saved one), as every other chart edit keeps it, and still fits
// a `Fitted` one. Each of the three theme calls is checked.
#[test]
fn a_theme_keeps_a_fixed_layout_and_fits_a_fitted_one() {
    use renderer::theme::{ChartKind, ThemeFamily, ThemeScope};
    type Apply = fn(&mut Chart, &mut [SeriesConfig]);
    let calls: [(&str, Apply); 3] = [
        ("apply_theme", |chart, series| {
            chart.apply_theme("midnight", series).unwrap()
        }),
        ("apply_custom_theme", |chart, series| {
            let theme = ThemeFamily::Midnight.theme(ChartKind::Line);
            chart.apply_custom_theme(&theme, series).unwrap()
        }),
        ("apply_custom_theme_to", |chart, series| {
            let theme = ThemeFamily::Midnight.theme(ChartKind::Line);
            chart
                .apply_custom_theme_to(&theme, series, ThemeScope::All)
                .unwrap()
        }),
    ];
    for (name, apply) in calls {
        let mut series = [SeriesConfig::line("s", "x", "y")];
        let mut chart = Chart::sized(800, 600);
        chart.config_mut().layout_state = renderer::LayoutState::Fixed;
        chart.config_mut().left_y.out_margin = 200.0;
        apply(&mut chart, &mut series);
        assert_eq!(chart.config().layout_state, renderer::LayoutState::Fixed);
        assert_eq!(
            chart.config().left_y.out_margin,
            200.0,
            "{name}: a Fixed margin moved"
        );

        let mut chart = Chart::sized(800, 600);
        chart.config_mut().left_y.out_margin = 200.0;
        apply(&mut chart, &mut series);
        let measure = renderer::CpuTextMeasure::for_style(&chart.config().draw_style);
        let fitted = renderer::layout::auto_layout(chart.config(), &measure);
        assert_eq!(chart.config(), &fitted, "{name}: a Fitted layout is fitted");
        assert_ne!(chart.config().left_y.out_margin, 200.0, "{name}");
    }
}
