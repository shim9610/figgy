//! The radius labels' ray over polar bars (#417): the layout's GPU gather
//! of each row's drawn stack (`Renderer::fit_radial_labels`, run by
//! `fit_ranges` and a registered chart's fits) chooses the ray and the
//! innermost labels to skip, writes them into the document only where they
//! are `None`, and refuses a streamed source. Checked on #417's stacked
//! 16-sector wind rose against the exported pixels at 1x and 2x: no drawn
//! radius label box covers a bar pixel.
#![cfg(not(target_arch = "wasm32"))]

use test_gpu::BlockOn as _;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;

use harness::{flat_style, measure};
use renderer::config::Category;
use renderer::data_config::{BarGroups, CategoryBarDirection, CategoryBarMode};
use renderer::format::LabelFormat;
use renderer::layout::{Element, RectF, Side};
use renderer::polar::PolarOptions;
use renderer::sink::Parts;
use renderer::{
    CategoryBarSeries, Chart, Color, Config, DataRenderType, FiggyError, RasterImage, Renderer,
    SeriesConfig,
};

/// The speed-band shares of #417's wind rose: sixteen directions from N
/// clockwise, four bands each, as fractions of all hours.
const WIND_ROSE: [[f64; 4]; 16] = [
    [0.009636, 0.015251, 0.007903, 0.002426],
    [0.010121, 0.014627, 0.006932, 0.003674],
    [0.009983, 0.013241, 0.008666, 0.005338],
    [0.009775, 0.015598, 0.011092, 0.009497],
    [0.011023, 0.016846, 0.011924, 0.010260],
    [0.010121, 0.018718, 0.012478, 0.008042],
    [0.011924, 0.017192, 0.009636, 0.004367],
    [0.011231, 0.015737, 0.007140, 0.003536],
    [0.010884, 0.018094, 0.009151, 0.006447],
    [0.012894, 0.022114, 0.013934, 0.013033],
    [0.011854, 0.024749, 0.018302, 0.015945],
    [0.013449, 0.031612, 0.023847, 0.015598],
    [0.011646, 0.035563, 0.030364, 0.024957],
    [0.014627, 0.033899, 0.030225, 0.037712],
    [0.012548, 0.031612, 0.032721, 0.046863],
    [0.011854, 0.020867, 0.015945, 0.012756],
];
const NAMES: [&str; 16] = [
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW",
    "NNW",
];
/// The bands' flat fills: far from the black labels, the white ground and
/// the grey grid.
const BANDS: [Color; 4] = [
    Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    },
    Color {
        r: 0.0,
        g: 0.7,
        b: 0.0,
        a: 1.0,
    },
    Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    },
    Color {
        r: 1.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    },
];

fn bars(band: usize) -> SeriesConfig {
    let id = format!("band{band}");
    SeriesConfig {
        render_type: DataRenderType::CategoryBars(CategoryBarSeries {
            color: BANDS[band],
            style: flat_style(),
            labels: Default::default(),
            label_format: Default::default(),
            overrides: None,
        }),
        ..SeriesConfig::category_bars(id.as_str(), "directions", id.as_str())
    }
}

/// A headless renderer holding the wind rose's columns, and its series.
fn wind_rose_renderer() -> (Renderer, Vec<SeriesConfig>) {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let directions: Vec<f64> = (0..WIND_ROSE.len()).map(|i| i as f64).collect();
    renderer.add_column("directions", &directions).unwrap();
    for band in 0..4 {
        let values: Vec<f64> = WIND_ROSE.iter().map(|row| row[band]).collect();
        renderer
            .add_column(&format!("band{band}"), &values)
            .unwrap();
    }
    (renderer, (0..4).map(bars).collect())
}

/// #417's chart: 1200x800, N up and clockwise, the four bands stacked over
/// 75% of each slot, the radius labels in `Percent`; the ray and the skip
/// count left to the layout unless a test sets them.
fn wind_rose_config(categories: usize) -> Config {
    let mut config = renderer::default::default_config();
    config.chart_area.0.width = 1200;
    config.chart_area.0.height = 800;
    config.legend.visible = false;
    config.chart_title.visible = false;
    config.bottom_x.categories = Some(
        (0..categories)
            .map(|i| Category::new(format!("row-{i}"), NAMES.get(i).copied().unwrap_or("")))
            .collect(),
    );
    config.bottom_x.title_option.visible = false;
    config.left_y.title_option.visible = false;
    config.left_y.label_style.format = LabelFormat::Percent;
    config.set_polar(Some(PolarOptions {
        clockwise: true,
        zero_angle_degrees: 90.0,
        ..PolarOptions::default()
    }));
    config.bar_groups = Some(BarGroups {
        direction: CategoryBarDirection::Vertical,
        mode: CategoryBarMode::Stacked,
        group_width: 0.75,
        bar_gap: 0.0,
    });
    config
}

/// The drawn radius label boxes of `config`, in logical pixels.
fn radius_label_boxes(config: &Config) -> Vec<RectF> {
    let mut out = Vec::new();
    renderer::select::on_drawn_panel(config, |panel| {
        renderer::select::emit_element(
            panel,
            &Element::AxisLabel(Side::Left),
            &measure(panel),
            &mut Parts(&mut |part| out.push(part.extent())),
        );
    });
    out
}

/// Whether `pixel` is a band's fill (antialiased edges included).
fn is_bar(pixel: [u8; 4]) -> bool {
    BANDS.iter().any(|band| {
        let target = [band.r, band.g, band.b].map(|c| (c * 255.0).round() as i32);
        let d: i32 = (0..3)
            .map(|i| (i32::from(pixel[i]) - target[i]).abs())
            .max()
            .unwrap();
        d < 70
    })
}

/// The bar pixels inside `rect` (logical pixels) of an export at `scale`.
fn bar_pixels(image: &RasterImage, rect: RectF, scale: f32) -> usize {
    let x0 = (rect.x * scale).floor().max(0.0) as u32;
    let y0 = (rect.y * scale).floor().max(0.0) as u32;
    let x1 = (((rect.x + rect.width) * scale).ceil() as u32).min(image.width);
    let y1 = (((rect.y + rect.height) * scale).ceil() as u32).min(image.height);
    let mut count = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * image.width + x) * 4) as usize;
            let p = [
                image.rgba[i],
                image.rgba[i + 1],
                image.rgba[i + 2],
                image.rgba[i + 3],
            ];
            count += usize::from(is_bar(p));
        }
    }
    count
}

fn export(
    renderer: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    scale: f32,
) -> RasterImage {
    renderer
        .export_rgba(
            renderer::ExportSource::chart(chart, series),
            renderer::ExportOptions::panel(scale),
        )
        .block_on()
        .unwrap()
}

/// The bar pixels under each drawn radius label at 1x and 2x, and the
/// number of labels drawn.
fn covered(renderer: &mut Renderer, chart: &Chart, series: &[SeriesConfig]) -> ([usize; 2], usize) {
    let boxes = radius_label_boxes(chart.config());
    let mut out = [0usize; 2];
    for (k, scale) in [1.0f32, 2.0].into_iter().enumerate() {
        let image = export(renderer, chart, series, scale);
        out[k] = boxes
            .iter()
            .map(|rect| bar_pixels(&image, *rect, scale))
            .sum();
    }
    (out, boxes.len())
}

// #417: the issue's wind rose, fitted and laid out as the browser fixture
// does, draws no radius label over a bar at 1x or 2x; at the old default
// ray with every label shown, labels do sit over bars (the check sees them).
#[test]
fn the_wind_rose_draws_no_radius_label_over_a_bar() {
    let (mut renderer, mut series) = wind_rose_renderer();
    let mut chart = Chart::new(wind_rose_config(16));
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    let polar = chart.config().polar().unwrap().clone();
    let angle = polar
        .radial_label_angle_degrees
        .expect("the layout chose a ray");
    let skipped = polar
        .radius_labels_skipped
        .expect("the layout chose a skip count");
    chart.fit_layout();
    let (pixels, drawn) = covered(&mut renderer, &chart, &series);
    assert_eq!(pixels, [0, 0], "ray {angle}°, skipped {skipped}");
    assert!(
        drawn >= 4,
        "{drawn} labels drawn at {angle}°, skipped {skipped}"
    );
    // The format survives the fit (#417, part 2).
    assert_eq!(
        chart.config().left_y.label_style.format,
        LabelFormat::Percent
    );

    // The old default: straight up, every label shown.
    let mut old = chart.config().clone();
    let polar = old.polar_mut().unwrap();
    polar.radial_label_angle_degrees = Some(90.0);
    polar.radius_labels_skipped = Some(0);
    let old = Chart::new(old);
    let (pixels, _) = covered(&mut renderer, &old, &series);
    assert!(pixels[0] > 0 && pixels[1] > 0, "{pixels:?}");
}

// An explicit ray is never moved: the layout only chooses the labels to
// skip there, and those clear the bars too. An explicit skip count is kept
// while the ray is chosen.
#[test]
fn explicit_values_win() {
    let (mut renderer, mut series) = wind_rose_renderer();
    let mut config = wind_rose_config(16);
    config.polar_mut().unwrap().radial_label_angle_degrees = Some(90.0);
    let mut chart = Chart::new(config);
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    let polar = chart.config().polar().unwrap();
    assert_eq!(polar.radial_label_angle_degrees, Some(90.0));
    let skipped = polar.radius_labels_skipped.unwrap();
    assert!(
        skipped > 0,
        "the N and NNW wedges reach past the first ring"
    );
    let (pixels, _) = covered(&mut renderer, &chart, &series);
    assert_eq!(pixels, [0, 0], "skipped {skipped}");

    let mut config = wind_rose_config(16);
    config.polar_mut().unwrap().radius_labels_skipped = Some(0);
    let mut chart = Chart::new(config);
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    let polar = chart.config().polar().unwrap();
    assert_eq!(polar.radius_labels_skipped, Some(0));
    assert!(polar.radial_label_angle_degrees.is_some());

    // `hide_first_radius_label` still hides the first label over a zero
    // skip count, and a second fit changes nothing.
    let mut config = chart.config().clone();
    config.polar_mut().unwrap().hide_first_radius_label = true;
    let shown = radius_label_boxes(chart.config()).len();
    assert_eq!(radius_label_boxes(&config).len(), shown - 1);
    let before = config.clone();
    assert!(!renderer.fit_radial_labels(&mut config, &series).unwrap());
    assert_eq!(config, before);
}

// A registered chart is fitted at registration; a chart without bars
// keeps `None`.
#[test]
fn registration_fits_the_ray() {
    let (mut renderer, series) = wind_rose_renderer();
    let mut fitted = wind_rose_config(16);
    fitted.layout_state = renderer::LayoutState::Fitted;
    let id = renderer.register_chart(fitted, series.clone()).unwrap();
    let polar = renderer.chart_config(id).unwrap().polar().unwrap();
    assert!(polar.radial_label_angle_degrees.is_some());
    assert!(polar.radius_labels_skipped.is_some());

    // Without bars there is nothing to choose from.
    let mut none = wind_rose_config(16);
    assert!(!renderer.fit_radial_labels(&mut none, &[]).unwrap());
    assert_eq!(none.polar().unwrap().radial_label_angle_degrees, None);
}

// The fitted document round-trips through JSON with both values; an
// unfitted one writes neither key.
#[cfg(feature = "serde")]
#[test]
fn the_fitted_document_round_trips() {
    let (renderer, mut series) = wind_rose_renderer();
    let mut chart = Chart::new(wind_rose_config(16));
    let unfitted = serde_json::to_string(chart.config()).unwrap();
    assert!(
        !unfitted.contains("radial_label_angle_degrees"),
        "{unfitted}"
    );
    assert!(!unfitted.contains("radius_labels_skipped"), "{unfitted}");
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    let json = serde_json::to_string(chart.config()).unwrap();
    assert!(json.contains("radial_label_angle_degrees"), "{json}");
    assert!(json.contains("radius_labels_skipped"), "{json}");
    let back: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(back.polar(), chart.config().polar());
    // The polar options write back byte for byte.
    let polar = |config: &Config| serde_json::to_string(&config.polar()).unwrap();
    assert_eq!(polar(&back), polar(chart.config()));
}

// Rule 8: a streamed source is refused with an explicit error, and the
// document is not changed.
#[test]
fn a_streamed_source_is_refused() {
    let (mut renderer, mut series) = wind_rose_renderer();
    renderer
        .register_streamed_columns(vec![renderer::StreamColumn {
            id: "streamed".into(),
            len: 16,
            encoding: renderer::StreamEncoding::HiLoF32,
            replay: renderer::StreamReplay::Sequential,
            revision: 1,
            statistics: renderer::StreamStatistics::Unknown,
        }])
        .unwrap();
    series[3].y_column = "streamed".into();
    let mut config = wind_rose_config(16);
    let before = config.clone();
    match renderer.fit_radial_labels(&mut config, &series) {
        Err(FiggyError::ColumnNotResident { id }) => assert_eq!(id, "streamed"),
        other => panic!("expected ColumnNotResident, got {other:?}"),
    }
    assert_eq!(config, before);
}

// Rule 10: a rose of 2000 stacked categories (past any size a fixed
// array would hold) is gathered and chosen; the cost grows with the square
// of the categories and is printed here.
#[test]
fn a_two_thousand_category_rose_is_fitted() {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let n = 2000usize;
    renderer
        .add_column("directions", &(0..n).map(|i| i as f64).collect::<Vec<_>>())
        .unwrap();
    let series: Vec<SeriesConfig> = (0..4)
        .map(|band| {
            let values: Vec<f64> = (0..n)
                .map(|i| 0.01 + 0.002 * ((i * (band + 3)) % 17) as f64)
                .collect();
            renderer
                .add_column(&format!("band{band}"), &values)
                .unwrap();
            bars(band)
        })
        .collect();
    let mut config = wind_rose_config(n);
    config.left_y.min = 0.0;
    config.left_y.max = 0.2;
    config.left_y.major_spacing = 0.05;
    let started = std::time::Instant::now();
    assert!(renderer.fit_radial_labels(&mut config, &series).unwrap());
    eprintln!("{n} categories fitted in {:?}", started.elapsed());
    assert!(config.polar().unwrap().radial_label_angle_degrees.is_some());
}

// Rule 6: SVG and PDF draw the same label walk, so the skipped labels are
// missing there too; dragging the ray stores the turned angle, which a
// later fit keeps.
#[test]
fn exports_and_the_ray_drag_follow_the_fitted_document() {
    let (mut renderer, mut series) = wind_rose_renderer();
    let mut chart = Chart::new(wind_rose_config(16));
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    let skipped = chart
        .config()
        .polar()
        .unwrap()
        .radius_labels_skipped
        .unwrap();
    assert!(skipped > 0);
    let svg = renderer
        .export_svg_blocking(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .unwrap();
    // The zero ring's label is the innermost one: skipped.
    assert!(
        !svg.contains(">0.0%<"),
        "the skipped zero label is not drawn"
    );
    assert!(svg.contains(">12.0%<"), "the outer labels are drawn");
    for scale in [1.0, 2.0] {
        let pdf = renderer
            .export_pdf_blocking(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"), "{scale}x");
    }

    let mut config = chart.config().clone();
    let before = config.polar().unwrap().ray_degrees();
    config.polar_mut().unwrap().turn_ray(30.0);
    let turned = config.polar().unwrap().radial_label_angle_degrees.unwrap();
    assert!(((turned - before).rem_euclid(360.0) - 30.0).abs() < 1e-9);
    renderer.fit_radial_labels(&mut config, &series).unwrap();
    assert_eq!(
        config.polar().unwrap().radial_label_angle_degrees,
        Some(turned)
    );
}
