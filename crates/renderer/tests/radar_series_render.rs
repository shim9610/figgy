//! End-to-end pixel verification of radar series (`DataRenderType::Radar`)
//! on a polar chart with a categorical angle axis, through the public
//! renderer API and the headless `export_panel_rgba`: the closed outline
//! with its closing segment, the fill to the centre or the hole, the vertex
//! markers, a NaN vertex, a sector, several series, the polygon and circle
//! grids with the category spokes and labels, the picked vertex's ring, the
//! radius-axis fit from the pool's statistics, the validation errors, the
//! annotation layer, a subplot, the SSOT edits of every new field, and a
//! released document opened through `Config::from_radar`, at 1x and 2x.
//!
//! The outline is red, the fill green, the markers of a second series blue,
//! all opaque and flat, so a colour predicate attributes a pixel to a part.
//! Expected positions come from the chart's own `polar_geometry` and
//! `polar_angle_map`, the circle and the angle mapping the renderer draws
//! with.
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::{Category, TickVisibility};
use renderer::data::Column;
use renderer::layout::{ChartArea, Rect};
use renderer::polar::{GridShape, PolarInterpolation, PolarOptions};
use renderer::{
    Anchor, Annotation, AnnotationKind, Chart, Color, Config, DataRenderType, DataSelectionsConfig,
    FiggyError, LayerRef, PickedDataRef, RadarLabels, RadarMarker, RadarSeriesLabels,
    RadarSeriesStyle, RasterImage, Renderer, RendererDevice, SeriesConfig, Subplot,
};

const WIDTH: u32 = 400;
const HEIGHT: u32 = 360;
const RED: Color = Color::from_rgb8(255, 0, 0);
const GREEN: Color = Color::from_rgb8(0, 200, 0);
const BLUE: Color = Color::from_rgb8(0, 0, 255);
const CLEAR: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};
/// The values of the first series over five categories, on a 0..1 radius.
const ONE: [f64; 5] = [0.9, 0.5, 0.8, 0.4, 0.7];
const TWO: [f64; 5] = [0.3, 0.6, 0.35, 0.65, 0.4];

fn shared_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    static DEVICE: OnceLock<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(|| {
        let (device, queue) = test_gpu::device();
        (Arc::new(device), Arc::new(queue))
    });
    (Arc::clone(device), Arc::clone(queue))
}

fn col(data: Vec<f64>) -> Column<f64> {
    let finite = data.iter().copied().filter(|v| v.is_finite());
    let min = finite.clone().fold(f64::INFINITY, f64::min);
    let max = finite.fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

/// A renderer holding the category indices and the two series' values, the
/// first one also with a NaN vertex.
fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("cat", &col(vec![0.0, 1.0, 2.0, 3.0, 4.0]))
        .unwrap();
    r.add_column("one", &col(ONE.to_vec())).unwrap();
    r.add_column("two", &col(TWO.to_vec())).unwrap();
    let mut gap = ONE.to_vec();
    gap[1] = f64::NAN;
    r.add_column("gap", &col(gap)).unwrap();
    r
}

fn categories(n: usize) -> Vec<Category> {
    (0..n)
        .map(|i| Category::new(format!("c{i}"), format!("Cat {i}")))
        .collect()
}

/// A polar chart over the five categories and a 0..1 radius, first spoke
/// up and clockwise, with no grid, chrome or legend: the only ink is the
/// series.
fn chart(polar: PolarOptions) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.chart_title.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.label_style.visible = false;
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.title_option.visible = false;
    }
    config.bottom_x.categories = Some(categories(5));
    config.polar = Some(polar);
    let mut chart = Chart::new(config);
    chart.set_y_range(0.0, 1.0);
    chart
}

fn up_clockwise() -> PolarOptions {
    PolarOptions {
        zero_angle_degrees: 90.0,
        clockwise: true,
        ..PolarOptions::default()
    }
}

/// An opaque outline, an opaque fill and no marker.
fn outline_and_fill() -> RadarSeriesStyle {
    RadarSeriesStyle {
        line_width_px: Some(3.0),
        fill: Some(GREEN),
        marker: Some(RadarMarker::None),
        ..Default::default()
    }
}

fn radar(id: &str, values: &str, color: Color, style: RadarSeriesStyle) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "cat".into(),
        y_column: values.into(),
        z_column: None,
        render_type: DataRenderType::Radar {
            color,
            style,
            labels: RadarSeriesLabels::default(),
        },
    }
}

/// Where category `i` at radius value `v` lands in an export at `scale`.
fn vertex(chart: &Chart, scale: f32, i: f64, v: f64) -> (f64, f64) {
    let config = chart.config().scaled(scale);
    let map = config.polar_angle_map().expect("polar");
    let geometry = config.polar_geometry().expect("geometry");
    let inner = f64::from(geometry.inner_radius);
    let axis = &config.left_y;
    let t = (v - axis.min) / (axis.max - axis.min);
    let radius = inner + (f64::from(geometry.radius) - inner) * t;
    let a = map.screen_angle(i);
    (
        f64::from(geometry.center[0]) + a.cos() * radius,
        f64::from(geometry.center[1]) - a.sin() * radius,
    )
}

fn lerp(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn pixel(img: &RasterImage, (x, y): (f64, f64)) -> [u8; 4] {
    let x = (x.round() as i64).clamp(0, img.width as i64 - 1) as u32;
    let y = (y.round() as i64).clamp(0, img.height as i64 - 1) as u32;
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

/// Whether `pred` holds within a pixel of `at`: a thin anti-aliased
/// stroke at an angle need not cover the rounded pixel itself.
fn near(img: &RasterImage, (x, y): (f64, f64), pred: fn([u8; 4]) -> bool) -> bool {
    (-1..=1).any(|dy| (-1..=1).any(|dx| pred(pixel(img, (x + f64::from(dx), y + f64::from(dy))))))
}

fn is_red(p: [u8; 4]) -> bool {
    p[3] > 180 && p[0] > 180 && p[1] < 90 && p[2] < 90
}

fn is_green(p: [u8; 4]) -> bool {
    p[3] > 180 && p[1] > 150 && p[0] < 90 && p[2] < 90
}

fn is_blue(p: [u8; 4]) -> bool {
    p[3] > 180 && p[2] > 180 && p[0] < 90 && p[1] < 90
}

fn is_empty(p: [u8; 4]) -> bool {
    p[3] < 16
}

fn count(img: &RasterImage, pred: fn([u8; 4]) -> bool) -> usize {
    img.rgba
        .chunks_exact(4)
        .filter(|p| pred([p[0], p[1], p[2], p[3]]))
        .count()
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_panel_rgba(chart, series, scale).unwrap()
}

/// Writes the image when `FIGGY_INTERACTION_ARTIFACTS` names a directory.
fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            std::path::Path::new(&path).join(format!("{name}.png")),
            renderer::encode_png(image).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn a_radar_polygon_closes_its_outline_and_fills_to_the_centre() {
    let mut r = renderer();
    let chart = chart(up_clockwise());
    let series = [radar("one", "one", RED, outline_and_fill())];
    for scale in [1.0, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("radar-polygon-{scale}x"), &img);
        let v = |i: usize| vertex(&chart, scale, i as f64, ONE[i]);
        // Every edge, the closing one from the last vertex back to the
        // first included, is red at its middle.
        for i in 0..5 {
            let mid = lerp(v(i), v((i + 1) % 5), 0.5);
            assert!(
                is_red(pixel(&img, mid)),
                "{scale}x: edge {i} -> {} at {mid:?}: {:?}",
                (i + 1) % 5,
                pixel(&img, mid)
            );
        }
        // The fill reaches the centre from every edge, the closing one too.
        let centre = vertex(&chart, scale, 0.0, 0.0);
        for i in 0..5 {
            let inside = lerp(centre, lerp(v(i), v((i + 1) % 5), 0.5), 0.6);
            assert!(
                is_green(pixel(&img, inside)),
                "{scale}x: the fill under edge {i} at {inside:?}: {:?}",
                pixel(&img, inside)
            );
        }
        // Outside the polygon, inside the outer ring: nothing.
        let outside = vertex(&chart, scale, 1.5, 0.95);
        assert!(is_empty(pixel(&img, outside)), "{scale}x: {outside:?}");
        // The first category points up.
        let top = v(0);
        assert!((top.0 - vertex(&chart, scale, 0.0, 0.0).0).abs() < 0.5);
        assert!(top.1 < centre.1);
    }
}

#[test]
fn the_first_category_follows_the_zero_angle_and_the_direction() {
    let mut r = renderer();
    let series = [radar(
        "one",
        "one",
        RED,
        RadarSeriesStyle {
            fill: Some(CLEAR),
            marker: Some(RadarMarker::Circle),
            marker_size_px: Some(10.0),
            ..outline_and_fill()
        },
    )];
    for polar in [
        up_clockwise(),
        PolarOptions::default(),
        PolarOptions {
            zero_angle_degrees: 200.0,
            clockwise: true,
            ..PolarOptions::default()
        },
    ] {
        let chart = chart(polar.clone());
        let img = export(&mut r, &chart, &series, 1.0);
        for (i, value) in ONE.iter().enumerate() {
            let at = vertex(&chart, 1.0, i as f64, *value);
            assert!(
                is_red(pixel(&img, at)),
                "{polar:?}: the marker of category {i} at {at:?}"
            );
        }
    }
}

#[test]
fn a_nan_vertex_breaks_the_outline_and_the_fill_beside_it() {
    let mut r = renderer();
    let chart = chart(up_clockwise());
    let series = [radar("gap", "gap", RED, outline_and_fill())];
    for scale in [1.0, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        let v = |i: usize| vertex(&chart, scale, i as f64, ONE[i]);
        let centre = vertex(&chart, scale, 0.0, 0.0);
        // The edges either side of vertex 1 and their fill are gone ...
        for (a, b) in [(0, 1), (1, 2)] {
            let mid = lerp(v(a), v(b), 0.5);
            assert!(!is_red(pixel(&img, mid)), "{scale}x: edge {a} -> {b}");
            let inside = lerp(centre, mid, 0.7);
            assert!(!is_green(pixel(&img, inside)), "{scale}x: fill {a} -> {b}");
        }
        // ... the others, the closing one included, stay.
        for (a, b) in [(2, 3), (3, 4), (4, 0)] {
            let mid = lerp(v(a), v(b), 0.5);
            assert!(is_red(pixel(&img, mid)), "{scale}x: edge {a} -> {b}");
            let inside = lerp(centre, mid, 0.6);
            assert!(is_green(pixel(&img, inside)), "{scale}x: fill {a} -> {b}");
        }
    }
}

#[test]
fn a_sector_spreads_the_categories_and_closes_the_outline_but_not_the_fill() {
    let mut r = renderer();
    let polar = PolarOptions {
        sector: Some([0.0, 180.0]),
        inner_radius_fraction: 0.25,
        ..PolarOptions::default()
    };
    let chart = chart(polar);
    let series = [radar("one", "one", RED, outline_and_fill())];
    for scale in [1.0, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("radar-sector-{scale}x"), &img);
        let config = chart.config().scaled(scale);
        let map = config.polar_angle_map().unwrap();
        // Category i at i / 5 of the half turn from the right, counter-
        // clockwise.
        for i in 0..5 {
            let expected = std::f64::consts::PI * i as f64 / 5.0;
            assert!((map.screen_angle(i as f64) - expected).abs() < 1e-9);
        }
        let v = |i: usize| vertex(&chart, scale, i as f64, ONE[i]);
        let closing = lerp(v(4), v(0), 0.5);
        assert!(
            is_red(pixel(&img, closing)),
            "{scale}x: the closing segment"
        );
        // The fill between an inner edge and the hole is drawn, the hole
        // itself stays empty.
        let hole = vertex(&chart, scale, 2.0, 0.0);
        let inside = lerp(hole, lerp(v(1), v(2), 0.5), 0.5);
        assert!(is_green(pixel(&img, inside)), "{scale}x: fill");
        let centre = config.polar_geometry().unwrap().center;
        let centre = (f64::from(centre[0]), f64::from(centre[1]));
        assert!(
            is_empty(pixel(&img, lerp(centre, hole, 0.5))),
            "{scale}x: hole"
        );
    }
}

#[test]
fn several_series_draw_in_order_with_their_own_look() {
    let mut r = renderer();
    let chart = chart(up_clockwise());
    let series = [
        radar("one", "one", RED, outline_and_fill()),
        radar(
            "two",
            "two",
            BLUE,
            RadarSeriesStyle {
                line_width_px: Some(2.0),
                fill: Some(CLEAR),
                marker: Some(RadarMarker::Square),
                marker_size_px: Some(10.0),
                ..Default::default()
            },
        ),
    ];
    for scale in [1.0, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("radar-two-series-{scale}x"), &img);
        // The second series' squares sit on its vertices, over the first
        // series' fill.
        for (i, value) in TWO.iter().enumerate() {
            let at = vertex(&chart, scale, i as f64, *value);
            assert!(is_blue(pixel(&img, at)), "{scale}x: square {i} at {at:?}");
        }
        // The square is a square: its corner, at half the side on both axes,
        // is still blue for the axis-aligned marker.
        let at = vertex(&chart, scale, 0.0, TWO[0]);
        let corner = (at.0 + 3.5 * f64::from(scale), at.1 + 3.5 * f64::from(scale));
        assert!(is_blue(pixel(&img, corner)), "{scale}x: square corner");
        // Its own outline is blue and it draws no fill.
        let mid = lerp(
            vertex(&chart, scale, 1.0, TWO[1]),
            vertex(&chart, scale, 2.0, TWO[2]),
            0.5,
        );
        assert!(is_blue(pixel(&img, mid)), "{scale}x: blue edge");
        assert!(count(&img, is_green) > 1000 && count(&img, is_red) > 200);
    }
}

#[test]
fn the_grid_is_rings_at_the_radius_ticks_and_spokes_at_the_categories() {
    let mut r = renderer();
    for shape in [GridShape::Circle, GridShape::Polygon] {
        let mut chart = chart(PolarOptions {
            grid_shape: Some(shape),
            ..up_clockwise()
        });
        let config = chart.config_mut();
        config.grid.show_major_x = true;
        config.grid.show_major_y = true;
        config.grid.major_x_color = BLUE;
        config.grid.major_y_color = RED;
        config.grid.major_x_width = 2.0;
        config.grid.major_y_width = 2.0;
        config.left_y.major_spacing = 0.5;
        for scale in [1.0, 2.0] {
            let img = export(&mut r, &chart, &[], scale);
            save(&format!("radar-grid-{shape:?}-{scale}x"), &img);
            // A spoke at every category, half way out.
            for i in 0..5 {
                let at = vertex(&chart, scale, i as f64, 0.25);
                assert!(near(&img, at, is_blue), "{shape:?} {scale}x: spoke {i}");
            }
            // Between two spokes the ring at 0.5 is a circle's arc or a
            // polygon's chord, which runs `cos(pi / 5)` of the way out.
            let arc = vertex(&chart, scale, 0.5, 0.5);
            let chord = lerp(
                vertex(&chart, scale, 0.0, 0.5),
                vertex(&chart, scale, 1.0, 0.5),
                0.5,
            );
            match shape {
                GridShape::Circle => {
                    assert!(near(&img, arc, is_red), "{scale}x: circle arc");
                    assert!(!is_red(pixel(&img, chord)), "{scale}x: no chord");
                }
                GridShape::Polygon => {
                    assert!(near(&img, chord, is_red), "{scale}x: polygon chord");
                    assert!(!is_red(pixel(&img, arc)), "{scale}x: no arc");
                }
            }
        }
    }
}

#[test]
fn the_category_labels_sit_outside_the_ring_at_their_spokes() {
    let mut r = renderer();
    let mut chart = chart(up_clockwise());
    chart.config_mut().bottom_x.label_style.visible = true;
    chart.config_mut().bottom_x.label_style.color = BLUE;
    for scale in [1.0, 2.0] {
        let img = export(&mut r, &chart, &[], scale);
        save(&format!("radar-labels-{scale}x"), &img);
        let config = chart.config().scaled(scale);
        let geometry = config.polar_geometry().unwrap();
        let map = config.polar_angle_map().unwrap();
        // Label ink beyond the ring along every spoke, and none along the
        // directions half way between two spokes.
        let ink_along = |angle: f64| {
            let (cx, cy) = (f64::from(geometry.center[0]), f64::from(geometry.center[1]));
            let from = f64::from(geometry.radius) + 2.0;
            let to = f64::from(geometry.radius) + 40.0 * f64::from(scale);
            let mut d = from;
            let mut ink = 0;
            while d < to {
                let at = (cx + angle.cos() * d, cy - angle.sin() * d);
                if pixel(&img, at)[3] > 100 {
                    ink += 1;
                }
                d += 1.0;
            }
            ink
        };
        for i in 0..5 {
            assert!(
                ink_along(map.screen_angle(i as f64)) > 0,
                "{scale}x: label {i}"
            );
        }
        // An empty label draws nothing at its spoke.
        let mut blank = Chart::new(chart.config().clone());
        blank.config_mut().bottom_x.categories.as_mut().unwrap()[2].label = String::new();
        let img_blank = export(&mut r, &blank, &[], scale);
        assert!(count(&img_blank, |p| p[3] > 100) < count(&img, |p| p[3] > 100));
    }
}

#[test]
fn a_picked_vertex_takes_the_picked_ring() {
    let mut r = renderer();
    let mut chart = chart(up_clockwise());
    let series = [radar(
        "one",
        "one",
        RED,
        RadarSeriesStyle {
            fill: Some(CLEAR),
            marker: Some(RadarMarker::Circle),
            marker_size_px: Some(8.0),
            ..outline_and_fill()
        },
    )];
    let plain = export(&mut r, &chart, &series, 1.0);
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "one".into(),
            point_index: 2,
        }],
        highlight_color: BLUE,
        ..Default::default()
    });
    for scale in [1.0, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        let at = vertex(&chart, scale, 2.0, ONE[2]);
        // The ring sits outside the 4 px marker radius.
        let s = f64::from(scale);
        let ring = (5..=12)
            .map(|d| (at.0 + d as f64 * s * 0.5, at.1))
            .any(|p| is_blue(pixel(&img, p)));
        assert!(ring, "{scale}x: no ring right of vertex 2");
        let elsewhere = vertex(&chart, scale, 0.0, ONE[0]);
        assert!(!is_blue(pixel(&img, (elsewhere.0 + 4.0 * s, elsewhere.1))));
    }
    assert_ne!(plain.rgba, export(&mut r, &chart, &series, 1.0).rgba);
}

#[test]
fn the_radius_axis_fits_the_values_and_the_angle_axis_keeps_its_categories() {
    let r = renderer();
    let mut chart = chart(up_clockwise());
    let series = [
        radar("one", "one", RED, outline_and_fill()),
        radar("two", "two", BLUE, outline_and_fill()),
    ];
    let before = chart.config().bottom_x.clone();
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
        .unwrap();
    let config = chart.config();
    assert_eq!((config.left_y.min, config.left_y.max), (0.3, 0.9));
    assert_eq!(config.bottom_x.categories, before.categories);
    assert_eq!(config.bottom_x.data_range(), before.data_range());
}

#[test]
fn radar_series_are_validated_against_their_chart_and_columns() {
    let mut r = renderer();
    let series = [radar("one", "one", RED, outline_and_fill())];
    let invalid = |result: renderer::Result<RasterImage>, what: &str| match result {
        Err(FiggyError::InvalidSeriesConfig { reason, .. }) => reason,
        Err(other) => panic!("{what}: {other}"),
        Ok(_) => panic!("{what}: drew"),
    };
    // A rectangular chart.
    let mut rectangular = chart(up_clockwise());
    rectangular.config_mut().polar = None;
    let reason = invalid(
        r.export_panel_rgba(&rectangular, &series, 1.0),
        "rectangular",
    );
    assert!(reason.contains("polar"), "{reason}");
    // A numeric angle axis.
    let mut numeric = chart(up_clockwise());
    numeric.config_mut().bottom_x.categories = None;
    let reason = invalid(r.export_panel_rgba(&numeric, &series, 1.0), "numeric");
    assert!(reason.contains("categorical angle axis"), "{reason}");
    // Arc interpolation.
    let arcs = chart(PolarOptions {
        interpolation: PolarInterpolation::Arc { pieces: 4 },
        ..up_clockwise()
    });
    let reason = invalid(r.export_panel_rgba(&arcs, &series, 1.0), "arcs");
    assert!(reason.contains("Chord"), "{reason}");
    // A column that does not hold one value per category.
    r.add_column("short", &col(vec![0.5, 0.5, 0.5])).unwrap();
    let short = [radar("short", "short", RED, outline_and_fill())];
    let reason = invalid(
        r.export_panel_rgba(&chart(up_clockwise()), &short, 1.0),
        "short",
    );
    assert!(reason.contains("one value per category"), "{reason}");
    // A label text per category or none.
    let mut labelled = radar("one", "one", RED, outline_and_fill());
    if let DataRenderType::Radar { labels, .. } = &mut labelled.render_type {
        labels.placement = RadarLabels::Values;
        labels.text = vec!["1".into(); 4];
    }
    let reason = invalid(
        r.export_panel_rgba(&chart(up_clockwise()), &[labelled], 1.0),
        "labels",
    );
    assert!(reason.contains("one string per category"), "{reason}");
    // A polygon grid needs the categorical angle axis too.
    let mut polygon = chart(PolarOptions {
        grid_shape: Some(GridShape::Polygon),
        ..up_clockwise()
    });
    polygon.config_mut().bottom_x.categories = None;
    assert!(r.export_panel_rgba(&polygon, &[], 1.0).is_err());
    // A stylized draw style draws no radar series, and says so.
    assert!(!renderer::draw_style_supports(
        &renderer::config::DrawStyle::Sketch(Default::default()),
        &series[0].render_type
    ));
    assert!(renderer::draw_style_supports(
        &renderer::config::DrawStyle::Precise,
        &series[0].render_type
    ));
}

#[test]
fn the_annotation_layer_draws_ratio_anchors_over_a_radar_chart() {
    let mut r = renderer();
    let mut chart = chart(up_clockwise());
    let series = [radar("one", "one", RED, outline_and_fill())];
    let plain = export(&mut r, &chart, &series, 1.0);
    chart.config_mut().annotations.items = vec![
        Annotation::new(AnnotationKind::Arrow {
            from: Anchor::Ratio { x: 0.05, y: 0.05 },
            to: Anchor::Ratio { x: 0.3, y: 0.3 },
            style: Default::default(),
            text: None,
        }),
        // A data anchor has no place on polar axes: skipped, not drawn.
        Annotation::new(AnnotationKind::Arrow {
            from: Anchor::Data { x: 0.0, y: 0.5 },
            to: Anchor::Data { x: 2.0, y: 0.5 },
            style: Default::default(),
            text: None,
        }),
    ];
    for scale in [1.0, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        let config = chart.config().scaled(scale);
        let da = config.data_area().unwrap().0;
        let at = (
            da.x as f64 + da.width as f64 * 0.15,
            da.y as f64 + da.height as f64 * 0.15,
        );
        assert!(
            pixel(&img, at)[3] > 100,
            "{scale}x: the ratio arrow at {at:?}"
        );
        if scale == 1.0 {
            assert_ne!(img.rgba, plain.rgba);
        }
    }
}

#[test]
fn a_polar_subplot_draws_its_own_radar_series() {
    let mut r = renderer();
    let mut parent = renderer::default::default_config();
    parent.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    parent.legend.visible = false;
    let mut cell = chart(up_clockwise()).config().clone();
    cell.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT,
        width: WIDTH,
        height: HEIGHT,
    });
    parent.subplots.push(Subplot {
        config: cell,
        series: vec!["one".into()],
        link_x: None,
        link_y: None,
    });
    let chart = Chart::new(parent);
    let series = [radar("one", "one", RED, outline_and_fill())];
    for scale in [1.0, 2.0] {
        let img = r.export_document_rgba(&chart, &series, scale).unwrap();
        save(&format!("radar-subplot-{scale}x"), &img);
        let (w, h) = (img.width as usize, img.height as usize);
        let lower = img.rgba[w * 4 * (h / 2)..]
            .chunks_exact(4)
            .filter(|p| is_green([p[0], p[1], p[2], p[3]]))
            .count();
        let upper = img.rgba[..w * 4 * (h / 2)]
            .chunks_exact(4)
            .filter(|p| is_green([p[0], p[1], p[2], p[3]]))
            .count();
        assert!(lower > 1000 && upper == 0, "{scale}x: {upper} / {lower}");
    }
}

/// Every new field the radar chart reads: an edit redraws (the export
/// differs from the one before it) and draws what a fresh renderer draws
/// for the edited state, so no cache keeps the previous look.
#[test]
fn every_radar_edit_redraws_like_a_fresh_renderer() {
    let mut r = renderer();
    let base = chart(up_clockwise());
    let series = vec![
        radar(
            "one",
            "one",
            RED,
            RadarSeriesStyle {
                marker: Some(RadarMarker::Circle),
                marker_size_px: Some(8.0),
                ..outline_and_fill()
            },
        ),
        radar("two", "two", BLUE, RadarSeriesStyle::default()),
    ];
    let mut grid = Chart::new(base.config().clone());
    grid.config_mut().grid.show_major_x = true;
    grid.config_mut().grid.show_major_y = true;
    type ChartEdit = fn(&mut Config);
    type SeriesEdit = fn(&mut RadarSeriesStyle, &mut Color);
    let chart_edits: [(&str, ChartEdit); 6] = [
        ("grid_shape Polygon", |c| {
            c.polar.as_mut().unwrap().grid_shape = Some(GridShape::Polygon)
        }),
        ("zero angle", |c| {
            c.polar.as_mut().unwrap().zero_angle_degrees = 30.0
        }),
        ("clockwise", |c| c.polar.as_mut().unwrap().clockwise = false),
        ("sector", |c| {
            c.polar.as_mut().unwrap().sector = Some([0.0, 270.0])
        }),
        ("a category more", |c| {
            c.bottom_x
                .categories
                .as_mut()
                .unwrap()
                .push(Category::new("c5", "Cat 5"))
        }),
        ("inner radius", |c| {
            c.polar.as_mut().unwrap().inner_radius_fraction = 0.3
        }),
    ];
    let series_edits: [(&str, SeriesEdit); 8] = [
        ("colour", |_, c| *c = GREEN),
        ("line width", |s, _| s.line_width_px = Some(6.0)),
        ("fill", |s, _| s.fill = Some(BLUE)),
        ("fill cleared", |s, _| s.fill = None),
        ("fill opacity", |s, _| {
            s.fill = None;
            s.fill_opacity = Some(0.9)
        }),
        ("marker", |s, _| s.marker = Some(RadarMarker::Square)),
        ("marker none", |s, _| s.marker = Some(RadarMarker::None)),
        ("marker size", |s, _| s.marker_size_px = Some(16.0)),
    ];
    for scale in [1.0, 2.0] {
        let before = export(&mut r, &grid, &series, scale);
        for (name, edit) in chart_edits {
            let mut edited = Chart::new(grid.config().clone());
            edit(edited.config_mut());
            // A category more needs one more value per column.
            if name == "a category more" {
                let mut fresh = renderer();
                fresh
                    .add_column("cat6", &col(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]))
                    .unwrap();
                fresh
                    .add_column("one6", &col(vec![0.9, 0.5, 0.8, 0.4, 0.7, 0.6]))
                    .unwrap();
                let mut six = series[0].clone();
                six.x_column = "cat6".into();
                six.y_column = "one6".into();
                r.add_column("cat6", &col(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]))
                    .ok();
                r.add_column("one6", &col(vec![0.9, 0.5, 0.8, 0.4, 0.7, 0.6]))
                    .ok();
                let after = export(&mut r, &edited, std::slice::from_ref(&six), scale);
                assert_ne!(after.rgba, before.rgba, "{name} {scale}x");
                assert_eq!(
                    after.rgba,
                    export(&mut fresh, &edited, &[six], scale).rgba,
                    "{name} {scale}x: cached"
                );
                continue;
            }
            let after = export(&mut r, &edited, &series, scale);
            assert_ne!(after.rgba, before.rgba, "{name} {scale}x");
            assert_eq!(
                after.rgba,
                export(&mut renderer(), &edited, &series, scale).rgba,
                "{name} {scale}x: cached"
            );
        }
        for (name, edit) in series_edits {
            let mut edited = series.clone();
            if let DataRenderType::Radar { style, color, .. } = &mut edited[0].render_type {
                edit(style, color);
            }
            let after = export(&mut r, &grid, &edited, scale);
            assert_ne!(after.rgba, before.rgba, "{name} {scale}x");
            assert_eq!(
                after.rgba,
                export(&mut renderer(), &grid, &edited, scale).rgba,
                "{name} {scale}x: cached"
            );
        }
        // The value labels redraw on every label edit (the placement, the
        // strings, a drag offset) like a fresh renderer: the label atlas is
        // keyed by its strings and the offsets are read on every draw.
        let mut labelled = series.clone();
        if let DataRenderType::Radar { labels, .. } = &mut labelled[0].render_type {
            labels.placement = RadarLabels::Values;
            labels.text = vec!["a".into(); 5];
        }
        let mut retexted = labelled.clone();
        if let DataRenderType::Radar { labels, .. } = &mut retexted[0].render_type {
            labels.text[2] = "bb".into();
        }
        let mut dragged = labelled.clone();
        let _ = dragged[0].drag_value_label(1, 5, 6.0, -4.0);
        let mut previous = before.rgba.clone();
        for (name, edited) in [
            ("labels", &labelled),
            ("label text", &retexted),
            ("label drag", &dragged),
        ] {
            let after = export(&mut r, &grid, edited, scale);
            assert_ne!(after.rgba, previous, "{name} {scale}x");
            assert_eq!(
                after.rgba,
                export(&mut renderer(), &grid, edited, scale).rgba,
                "{name} {scale}x: cached"
            );
            previous = after.rgba;
        }
    }
}

#[test]
fn a_released_radar_document_draws_through_the_reader() {
    let fixture = include_str!("fixtures/released/model-0.12.1-radar-edited.json");
    let document: renderer::RadarChart = serde_json::from_str(fixture).unwrap();
    document.validate().unwrap();
    let (mut config, series, uploads) = Config::from_radar(&document);
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    });
    let mut r = renderer();
    for upload in &uploads {
        r.add_column(upload.column.clone(), &upload.values).unwrap();
    }
    let chart = Chart::new(config);
    for scale in [1.0, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("radar-released-edited-{scale}x"), &img);
        // Every series' colour is drawn.
        for s in &document.series {
            let [cr, cg, cb] = [s.color.r, s.color.g, s.color.b].map(|c| (c * 255.0) as i32);
            let n = img
                .rgba
                .chunks_exact(4)
                .filter(|p| {
                    p[3] > 200
                        && (i32::from(p[0]) - cr).abs() < 12
                        && (i32::from(p[1]) - cg).abs() < 12
                        && (i32::from(p[2]) - cb).abs() < 12
                })
                .count();
            assert!(n > 50 * (scale as usize), "{}: {n}", s.id);
        }
    }
}

/// A radar chart has no spoke cap (#107): 100 categories, past the old 64,
/// close their outline with every edge drawn at 1x and 2x, the last vertex
/// picks as its own point and its picked ring draws. The value labels are
/// written for every vertex (`value_labels_render` draws 300 bar labels,
/// #107 R11).
#[test]
fn a_hundred_spokes_draw_pick_and_highlight() {
    const N: usize = 100;
    const SIDE: u32 = 800;
    let values: Vec<f64> = (0..N).map(|i| 0.7 + 0.2 * (i as f64 * 0.3).cos()).collect();
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("cat", &col((0..N).map(|i| i as f64).collect()))
        .unwrap();
    r.add_column("many", &col(values.clone())).unwrap();
    let mut chart = chart(up_clockwise());
    {
        let config = chart.config_mut();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: SIDE,
            height: SIDE,
        });
        config.bottom_x.categories = Some(categories(N));
    }
    let mut series = radar(
        "many",
        "many",
        RED,
        RadarSeriesStyle {
            fill: Some(CLEAR),
            marker: Some(RadarMarker::Circle),
            marker_size_px: Some(6.0),
            ..outline_and_fill()
        },
    );
    if let DataRenderType::Radar { labels, .. } = &mut series.render_type {
        labels.text = (0..N).map(|i| format!("{i}")).collect();
    }
    let series = [series];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        let v = |i: usize| vertex(&chart, scale, i as f64, values[i]);
        // Every sampled edge, the closing one from the last vertex back to
        // the first included, is red at its middle.
        for i in [0, 31, 63, 64, 80, N - 1] {
            let mid = lerp(v(i), v((i + 1) % N), 0.5);
            assert!(near(&img, mid, is_red), "{scale}x: edge {i} at {mid:?}");
        }
    }
    // A pick at the last vertex names it.
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let at = vertex(&chart, 1.0, (N - 1) as f64, values[N - 1]);
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0 as f32, at.1 as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: SIDE,
            height: SIDE,
        },
        display_scale: 1.0,
        max_distance_px: 2.0,
    };
    let hit = pollster::block_on(r.pick_chart(id, request).unwrap().resolve())
        .unwrap()
        .expect("the last vertex picks");
    assert_eq!((hit.series_id.as_str(), hit.point_index), ("many", N - 1));
    // Its picked ring draws outside the marker, and nowhere near vertex 0.
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "many".into(),
            point_index: N - 1,
        }],
        highlight_color: BLUE,
        ..Default::default()
    });
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("radar-hundred-{scale}x"), &img);
        let at = vertex(&chart, scale, (N - 1) as f64, values[N - 1]);
        let s = f64::from(scale);
        let ring = (4..=12)
            .map(|d| (at.0 + d as f64 * s * 0.5, at.1))
            .any(|p| is_blue(pixel(&img, p)));
        assert!(ring, "{scale}x: no ring right of vertex {}", N - 1);
        let first = vertex(&chart, scale, 0.0, values[0]);
        assert!(
            !near(&img, (first.0 + 4.0 * s, first.1), is_blue),
            "{scale}x"
        );
    }
}

/// A radar chart has no series cap (#107 R3): 20 series, past the old 16,
/// draw as nested outlines. The innermost (blue) and the outermost (red,
/// the last) outline at 1x and 2x, a pick at the last series' vertex names
/// it, and its picked ring draws there and not at its first vertex.
#[test]
fn twenty_radar_series_draw_pick_and_highlight() {
    const M: usize = 20;
    let radius = |k: usize| 0.05 + 0.045 * k as f64;
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("cat", &col(vec![0.0, 1.0, 2.0, 3.0, 4.0]))
        .unwrap();
    let style = RadarSeriesStyle {
        line_width_px: Some(2.0),
        fill: Some(CLEAR),
        marker: Some(RadarMarker::Circle),
        marker_size_px: Some(4.0),
        ..Default::default()
    };
    let mut series = Vec::new();
    for k in 0..M {
        let id = format!("s{k}");
        r.add_column(&id, &col(vec![radius(k); 5])).unwrap();
        let color = if k == M - 1 { RED } else { BLUE };
        series.push(radar(&id, &id, color, style.clone()));
    }
    let mut chart = chart(up_clockwise());
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        for i in 0..5 {
            let edge = |k: usize| {
                lerp(
                    vertex(&chart, scale, i as f64, radius(k)),
                    vertex(&chart, scale, ((i + 1) % 5) as f64, radius(k)),
                    0.5,
                )
            };
            assert!(
                near(&img, edge(M - 1), is_red),
                "{scale}x: last series edge {i}"
            );
            assert!(
                near(&img, edge(0), is_blue),
                "{scale}x: first series edge {i}"
            );
        }
    }
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let at = vertex(&chart, 1.0, 3.0, radius(M - 1));
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0 as f32, at.1 as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT,
        },
        display_scale: 1.0,
        max_distance_px: 2.0,
    };
    let hit = pollster::block_on(r.pick_chart(id, request).unwrap().resolve())
        .unwrap()
        .expect("the last series' vertex picks");
    assert_eq!((hit.series_id.as_str(), hit.point_index), ("s19", 3));
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "s19".into(),
            point_index: 3,
        }],
        highlight_color: GREEN,
        ..Default::default()
    });
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        let s = f64::from(scale);
        let ring = |at: (f64, f64)| {
            (-12..=12).any(|dy: i32| {
                (-12..=12).any(|dx: i32| {
                    is_green(pixel(
                        &img,
                        (
                            at.0 + f64::from(dx) * s * 0.5,
                            at.1 + f64::from(dy) * s * 0.5,
                        ),
                    ))
                })
            })
        };
        assert!(
            ring(vertex(&chart, scale, 3.0, radius(M - 1))),
            "{scale}x: no ring at vertex 3"
        );
        assert!(
            !ring(vertex(&chart, scale, 0.0, radius(M - 1))),
            "{scale}x: a ring at vertex 0"
        );
    }
}
