//! Polar coordinates (`Config.polar`) through the public renderer API: the
//! headless export, the GPU and CPU pickers and the validation errors.
//!
//! House pattern (mirrors `vector_render.rs`): one process-wide device, a
//! renderer per test, and assertions that name the pixel a (θ, r) sample
//! lands on through `Config::polar_geometry`, so a wrong angle, direction,
//! radius or sector fails rather than passes.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::AxisScale;
use renderer::data::Column;
use renderer::data_config::{BarOrientation, DataBarStyleConfig, LineFill, LineInterpolation};
use renderer::format::LabelFormat;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::pick::{PointColumnLookup, PointPickOptions, pick_nearest_point};
use renderer::polar::{AngleUnit, PolarInterpolation, PolarOptions};
use renderer::{
    Chart, Color, Config, DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, FiggyError,
    GpuPickRequest, OverlayLayer, RasterImage, Renderer, RendererDevice, ScatterShape,
    SeriesConfig, Subplot, display_config_for_surface,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 400;

const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

fn col(data: Vec<f64>) -> Column<f64> {
    let min = data
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f64::INFINITY, f64::min);
    let max = data
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

fn shared_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    static DEVICE: OnceLock<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(|| {
        let (device, queue) = test_gpu::device();
        (Arc::new(device), Arc::new(queue))
    });
    (Arc::clone(device), Arc::clone(queue))
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Bgra8Unorm,
        8 * 1024 * 1024,
    )
    .expect("renderer init")
}

/// A polar chart over 0..360 degrees and 0..1 radius with no grid, legend or
/// labels, so the only red ink is the series.
fn bare_chart(polar: PolarOptions) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    // No axis chrome either: the ring and the ray would paint over the
    // samples drawn on them.
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.label_style.visible = false;
        axis.line_visible = false;
        axis.tick = renderer::config::TickVisibility::None;
    }
    config.polar = Some(polar);
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 360.0);
    chart.set_y_range(0.0, 1.0);
    chart
}

fn scatter(id: &str, size: f32) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "theta".into(),
        y_column: "r".into(),
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: RED,
                point_shape: ScatterShape::CircleFilled,
                point_size: size,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
        },
    }
}

fn line_style(width: f32) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: LineStylePreset::Solid,
        line_color: RED,
        line_width: width,
        fill: None,
        fill_color: None,
        interpolation: LineInterpolation::Linear,
    }
}

fn line(id: &str, line: DataLineStyleConfig) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "theta".into(),
        y_column: "r".into(),
        render_type: DataRenderType::Line { line },
    }
}

fn upload(renderer: &mut Renderer, theta: Vec<f64>, r: Vec<f64>) {
    renderer
        .add_columns(&[
            ("theta", &col(theta) as &dyn renderer::ColumnSource),
            ("r", &col(r) as &dyn renderer::ColumnSource),
        ])
        .expect("upload");
}

/// The pixel a (θ, r) sample lands on: the same circle the renderer draws
/// against, with the axis fractions of the chart's own axes.
fn polar_px(config: &Config, theta: f64, r: f64) -> (f64, f64) {
    let polar = config.polar.as_ref().expect("polar chart");
    let geometry = config.polar_geometry().expect("geometry");
    let axis = &config.left_y;
    let t = match axis.scale {
        AxisScale::Logarithmic => (r.ln() - axis.min.ln()) / (axis.max.ln() - axis.min.ln()),
        AxisScale::Linear => (r - axis.min) / (axis.max - axis.min),
    };
    let t = if axis.inverted { 1.0 - t } else { t };
    let inner = f64::from(polar.inner_radius_fraction);
    let radius = f64::from(geometry.radius) * (inner + (1.0 - inner) * t);
    let angle = polar.screen_angle(theta);
    (
        f64::from(geometry.center[0]) + angle.cos() * radius,
        f64::from(geometry.center[1]) - angle.sin() * radius,
    )
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

fn is_red(p: &[u8]) -> bool {
    p[3] > 180 && p[0] > 180 && p[1] < 80 && p[2] < 80
}

/// The half-transparent fill: red ink at any alpha (the export is not
/// composited over a background).
fn is_tinted(p: &[u8]) -> bool {
    p[3] > 60 && p[0] > 180 && p[1] < 80 && p[2] < 80
}

fn at(img: &RasterImage, (x, y): (f64, f64)) -> &[u8] {
    pixel(
        img,
        (x.round() as i64).clamp(0, img.width as i64 - 1) as u32,
        (y.round() as i64).clamp(0, img.height as i64 - 1) as u32,
    )
}

fn count(img: &RasterImage, pred: fn(&[u8]) -> bool) -> usize {
    img.rgba.chunks_exact(4).filter(|p| pred(p)).count()
}

fn export(renderer: &mut Renderer, chart: &Chart, series: &[SeriesConfig]) -> RasterImage {
    renderer.export_panel_rgba(chart, series, 1.0).unwrap()
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
fn markers_land_on_the_ring_at_their_angle_in_both_directions() {
    let mut renderer = renderer();
    upload(
        &mut renderer,
        vec![0.0, 90.0, 180.0, 270.0, 45.0],
        vec![1.0, 1.0, 1.0, 1.0, 0.5],
    );
    for (polar, name) in [
        (PolarOptions::default(), "counter-clockwise from the right"),
        (
            PolarOptions {
                zero_angle_degrees: 90.0,
                clockwise: true,
                ..PolarOptions::default()
            },
            "clockwise from the top",
        ),
    ] {
        let chart = bare_chart(polar);
        let img = export(&mut renderer, &chart, &[scatter("points", 8.0)]);
        save(&format!("polar-markers-{}", name.replace(' ', "-")), &img);
        let config = chart.config();
        let geometry = config.polar_geometry().unwrap();
        eprintln!(
            "{name}: geometry {geometry:?}, data area {:?}",
            config.data_area().unwrap().0
        );
        for (theta, r) in [
            (0.0, 1.0),
            (90.0, 1.0),
            (180.0, 1.0),
            (270.0, 1.0),
            (45.0, 0.5),
        ] {
            let p = polar_px(config, theta, r);
            assert!(
                is_red(at(&img, p)),
                "{name}: ({theta}, {r}) should draw at {p:?}, got {:?}",
                at(&img, p)
            );
        }
        // The compass variant puts 0 degrees at the top and 90 on the right.
        let top = (
            f64::from(geometry.center[0]),
            f64::from(geometry.center[1] - geometry.radius),
        );
        let right = (
            f64::from(geometry.center[0] + geometry.radius),
            f64::from(geometry.center[1]),
        );
        assert!(is_red(at(&img, top)) && is_red(at(&img, right)), "{name}");
        // Nothing draws where a rectangular chart would put (0, 1): the top
        // left corner of the data area.
        let area = config.data_area().unwrap().0;
        let corner = (area.x as f64 + 4.0, area.y as f64 + 4.0);
        assert!(!is_red(at(&img, corner)), "{name}: Cartesian corner");
        // Five markers and nothing else.
        // Five markers of radius 8 and nothing else.
        let ink = count(&img, is_red);
        assert!(
            (5 * 120..5 * 260).contains(&ink),
            "{name}: {ink} red pixels"
        );
    }
}

#[test]
fn chords_are_straight_and_arcs_follow_the_ring() {
    let mut renderer = renderer();
    upload(&mut renderer, vec![0.0, 90.0], vec![1.0, 1.0]);
    let chord = bare_chart(PolarOptions::default());
    let arc = bare_chart(PolarOptions {
        interpolation: PolarInterpolation::Arc { pieces: 16 },
        ..PolarOptions::default()
    });
    let series = [line("segment", line_style(4.0))];
    let img_chord = export(&mut renderer, &chord, &series);
    let img_arc = export(&mut renderer, &arc, &series);
    let config = chord.config();
    let a = polar_px(config, 0.0, 1.0);
    let b = polar_px(config, 90.0, 1.0);
    let chord_mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let ring_mid = polar_px(config, 45.0, 1.0);
    assert!(is_red(at(&img_chord, chord_mid)), "chord midpoint");
    assert!(
        !is_red(at(&img_chord, ring_mid)),
        "chord stays off the ring"
    );
    assert!(is_red(at(&img_arc, ring_mid)), "arc midpoint on the ring");
    assert!(!is_red(at(&img_arc, chord_mid)), "arc leaves the chord");
    // Every sixteenth of the way round the arc is on the ring.
    for k in 1..16 {
        let theta = 90.0 * f64::from(k) / 16.0;
        let p = polar_px(config, theta, 1.0);
        assert!(is_red(at(&img_arc, p)), "arc piece at {theta} degrees");
    }
}

#[test]
fn area_fills_to_the_centre_and_sectors_clip_samples_outside() {
    let mut renderer = renderer();
    upload(
        &mut renderer,
        vec![0.0, 45.0, 90.0, 180.0, 270.0],
        vec![1.0, 1.0, 1.0, 1.0, 1.0],
    );
    let mut style = line_style(2.0);
    style.fill = Some(LineFill::ToBaseline(0.0));
    style.fill_color = Some(Color::new(1.0, 0.0, 0.0, 0.5));
    let series = [line("filled", style)];
    let full = bare_chart(PolarOptions {
        interpolation: PolarInterpolation::Arc { pieces: 8 },
        ..PolarOptions::default()
    });
    let img = export(&mut renderer, &full, &series);
    save("polar-fill-full", &img);
    let config = full.config();
    // Inside the fan between 0 and 90 degrees, half way out.
    let inside = polar_px(config, 45.0, 0.5);
    assert!(
        is_tinted(at(&img, inside)),
        "fill inside at {inside:?}: {:?}",
        at(&img, inside)
    );
    assert!(
        is_tinted(at(&img, polar_px(config, 135.0, 0.5))),
        "fill between 90 and 180"
    );
    let sector = bare_chart(PolarOptions {
        sector: Some([0.0, 90.0]),
        interpolation: PolarInterpolation::Arc { pieces: 8 },
        ..PolarOptions::default()
    });
    let img = export(&mut renderer, &sector, &series);
    let config = sector.config();
    assert!(
        is_tinted(at(&img, polar_px(config, 45.0, 0.5))),
        "sector fill"
    );
    assert!(
        !is_tinted(at(&img, polar_px(config, 135.0, 0.5))),
        "outside the sector nothing is filled"
    );
    assert!(
        !is_red(at(&img, polar_px(config, 180.0, 1.0))),
        "a sample outside the sector is not drawn"
    );
    assert!(
        !is_red(at(&img, polar_px(config, 225.0, 1.0))),
        "nor the segment to it"
    );
}

#[test]
fn inner_radius_log_radius_and_below_range_samples() {
    let mut renderer = renderer();
    upload(
        &mut renderer,
        vec![0.0, 90.0, 180.0, 270.0],
        vec![10.0, 1.0, 0.0, 0.01],
    );
    let mut chart = bare_chart(PolarOptions {
        inner_radius_fraction: 0.3,
        ..PolarOptions::default()
    });
    chart.config_mut().left_y.scale = AxisScale::Logarithmic;
    chart.set_y_range(0.1, 10.0);
    let img = export(&mut renderer, &chart, &[scatter("points", 8.0)]);
    let config = chart.config();
    assert!(is_red(at(&img, polar_px(config, 0.0, 10.0))), "outer ring");
    assert!(
        is_red(at(&img, polar_px(config, 90.0, 1.0))),
        "one decade in"
    );
    let geometry = config.polar_geometry().unwrap();
    let centre = (f64::from(geometry.center[0]), f64::from(geometry.center[1]));
    assert!(!is_red(at(&img, centre)), "the hole stays empty");
    let left_inner = (
        f64::from(geometry.center[0] - geometry.radius * 0.3),
        f64::from(geometry.center[1]),
    );
    assert!(
        !is_red(at(&img, left_inner)),
        "a non-positive radius is masked on a log axis"
    );
    let bottom_inner = (
        f64::from(geometry.center[0]),
        f64::from(geometry.center[1] + geometry.radius * 0.3),
    );
    assert!(
        !is_red(at(&img, bottom_inner)),
        "a radius below the axis start is not drawn"
    );
    let ink = count(&img, is_red);
    assert!((2 * 120..2 * 260).contains(&ink), "{ink} red pixels");
}

fn pick_request(chart: &Chart, at: (f64, f64), max: f32) -> GpuPickRequest {
    let (_, panel, scale) = display_config_for_surface(chart.config(), (WIDTH, HEIGHT));
    assert_eq!(scale, 1.0);
    GpuPickRequest {
        canvas_position_px: [at.0 as f32, at.1 as f32],
        display_panel_px: panel,
        display_scale: scale,
        max_distance_px: max,
    }
}

struct Columns(Vec<(String, Vec<f32>)>);

impl PointColumnLookup for Columns {
    fn get_f32_column(&self, id: &renderer::ColumnId) -> Option<&[f32]> {
        self.0
            .iter()
            .find(|(name, _)| name == id)
            .map(|(_, values)| values.as_slice())
    }
}

#[test]
fn gpu_and_cpu_picking_agree_on_markers_chords_and_arcs() {
    let mut renderer = renderer();
    let theta = vec![0.0, 90.0, 200.0];
    let r = vec![1.0, 1.0, 0.5];
    upload(&mut renderer, theta.clone(), r.clone());
    let columns = Columns(vec![
        ("theta".into(), theta.iter().map(|v| *v as f32).collect()),
        ("r".into(), r.iter().map(|v| *v as f32).collect()),
    ]);
    for (polar, on_arc) in [
        (PolarOptions::default(), false),
        (
            PolarOptions {
                interpolation: PolarInterpolation::Arc { pieces: 16 },
                ..PolarOptions::default()
            },
            true,
        ),
    ] {
        let chart = bare_chart(polar);
        let mut style = line_style(3.0);
        style.interpolation = LineInterpolation::Linear;
        let series = vec![
            scatter("points", 10.0),
            SeriesConfig {
                series_id: "path".into(),
                ..line("path", style)
            },
        ];
        let id = renderer
            .register_chart(chart.config().clone(), series.clone())
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        let config = chart.config();
        let cpu = |at: (f64, f64), max: f32| {
            pick_nearest_point(
                config,
                &series,
                &columns,
                at.0 as f32,
                at.1 as f32,
                PointPickOptions {
                    max_distance_px: max,
                },
            )
        };
        let mut gpu = |at: (f64, f64), max: f32| {
            pollster::block_on(
                renderer
                    .pick_chart(id, pick_request(&chart, at, max))
                    .unwrap()
                    .resolve(),
            )
            .unwrap()
        };
        // The marker at 200 degrees, half way out.
        // The marker at 200 degrees, half way out. The path ends there too,
        // so either series may answer; the index is the sample's.
        let marker = polar_px(config, 200.0, 0.5);
        let hit = gpu(marker, 2.0).expect("marker hit");
        assert_eq!(hit.point_index, 2, "{hit:?}");
        assert_eq!(hit.distance_px, 0.0, "{hit:?}");
        let cpu_hit = cpu(marker, 2.0).expect("cpu marker hit");
        assert_eq!(cpu_hit.point_index, 2, "{cpu_hit:?}");
        assert_eq!(cpu_hit.distance_px, 0.0, "{cpu_hit:?}");
        // Between the first two samples: on the ring for an arc, on the
        // chord otherwise; the other path misses.
        let a = polar_px(config, 0.0, 1.0);
        let b = polar_px(config, 90.0, 1.0);
        let chord_mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
        let ring_mid = polar_px(config, 45.0, 1.0);
        let (on, off) = if on_arc {
            (ring_mid, chord_mid)
        } else {
            (chord_mid, ring_mid)
        };
        let hit = gpu(on, 3.0).expect("path hit");
        assert_eq!(hit.series_id, "path", "arc {on_arc}");
        assert!(hit.distance_px <= 1.0, "arc {on_arc}: {}", hit.distance_px);
        let cpu_hit = cpu(on, 3.0).expect("cpu path hit");
        assert_eq!(cpu_hit.series_id, "path");
        assert!(cpu_hit.distance_px <= 1.0, "{}", cpu_hit.distance_px);
        assert!(gpu(off, 3.0).is_none(), "arc {on_arc}: the other path");
        assert!(
            cpu(off, 3.0).is_none(),
            "arc {on_arc}: the other path (cpu)"
        );
        // A sample inside a sector picks; one outside no longer exists.
        renderer.remove_chart(id).unwrap();
    }
    let chart = bare_chart(PolarOptions {
        sector: Some([0.0, 100.0]),
        ..PolarOptions::default()
    });
    let series = vec![scatter("points", 10.0)];
    let id = renderer
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let config = chart.config();
    let inside = polar_px(config, 90.0, 1.0);
    let outside = polar_px(config, 200.0, 0.5);
    let hit = pollster::block_on(
        renderer
            .pick_chart(id, pick_request(&chart, inside, 2.0))
            .unwrap()
            .resolve(),
    )
    .unwrap();
    assert_eq!(hit.map(|h| h.point_index), Some(1));
    let miss = pollster::block_on(
        renderer
            .pick_chart(id, pick_request(&chart, outside, 2.0))
            .unwrap()
            .resolve(),
    )
    .unwrap();
    assert!(miss.is_none(), "{miss:?}");
    assert!(
        pick_nearest_point(
            config,
            &series,
            &columns,
            outside.0 as f32,
            outside.1 as f32,
            PointPickOptions {
                max_distance_px: 2.0
            }
        )
        .is_none()
    );
}

#[test]
fn unsupported_series_and_overlays_are_explicit_errors() {
    let mut renderer = renderer();
    upload(&mut renderer, vec![0.0, 90.0, 180.0], vec![0.2, 0.6, 1.0]);
    let chart = bare_chart(PolarOptions::default());
    let histogram = SeriesConfig {
        series_id: "bins".into(),
        source_id: None,
        label: None,
        x_column: "theta".into(),
        y_column: "r".into(),
        render_type: DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: RED,
                border_color: Color::BLACK,
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 2.0,
                width_ratio: 0.75,
                orientation: BarOrientation::Vertical,
                bar_style_overrides: None,
            },
        },
    };
    match renderer
        .export_panel_rgba(&chart, &[histogram], 1.0)
        .map(|_| ())
    {
        Err(FiggyError::InvalidSeriesConfig { series_id, reason }) => {
            assert_eq!(series_id, "bins");
            assert!(reason.contains("polar"), "{reason}");
        }
        other => panic!("histogram on polar axes: {other:?}"),
    }
    let mut stepped = line_style(2.0);
    stepped.interpolation = LineInterpolation::StepPost;
    let arc = bare_chart(PolarOptions {
        interpolation: PolarInterpolation::Arc { pieces: 4 },
        ..PolarOptions::default()
    });
    assert!(matches!(
        renderer.export_panel_rgba(&arc, &[line("step", stepped)], 1.0),
        Err(FiggyError::InvalidSeriesConfig { .. })
    ));
    let mut overlay = bare_chart(PolarOptions::default());
    overlay.config_mut().overlays.push(OverlayLayer::default());
    match renderer
        .export_panel_rgba(&overlay, &[scatter("points", 4.0)], 1.0)
        .map(|_| ())
    {
        Err(FiggyError::InvalidConfig { field, .. }) => assert_eq!(field, "polar"),
        other => panic!("overlays on a polar chart: {other:?}"),
    }
    let mut log_angle = bare_chart(PolarOptions::default());
    log_angle.config_mut().bottom_x.scale = AxisScale::Logarithmic;
    log_angle.set_x_range(1.0, 360.0);
    assert!(matches!(
        renderer.export_panel_rgba(&log_angle, &[scatter("points", 4.0)], 1.0),
        Err(FiggyError::InvalidConfig { field: "polar", .. })
    ));
    // The same series draws once the chart is valid again.
    assert!(
        renderer
            .export_panel_rgba(&chart, &[scatter("points", 4.0)], 1.0)
            .is_ok()
    );
}

#[test]
fn polar_and_cartesian_subplots_share_a_document_and_cartesian_pixels_are_unchanged() {
    let mut renderer = renderer();
    upload(&mut renderer, vec![0.0, 90.0, 180.0], vec![1.0, 0.5, 1.0]);
    // A rectangular parent on the left, a polar subplot on the right.
    let mut parent = bare_chart(PolarOptions::default());
    parent.config_mut().polar = None;
    parent.set_x_range(0.0, 360.0);
    parent.set_y_range(0.0, 1.0);
    let plain = export(&mut renderer, &parent, &[scatter("points", 8.0)]);
    let mut subplot = bare_chart(PolarOptions::default()).config().clone();
    subplot.chart_area = ChartArea(Rect {
        x: WIDTH,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    parent.config_mut().subplots.push(Subplot {
        config: subplot.clone(),
        series: vec!["polar_points".into()],
        link_x: None,
        link_y: None,
    });
    let mut polar_points = scatter("polar_points", 8.0);
    polar_points.series_id = "polar_points".into();
    let series = [scatter("points", 8.0), polar_points];
    let document = renderer
        .export_document_rgba(&parent, &series, 1.0)
        .unwrap();
    assert_eq!((document.width, document.height), (2 * WIDTH, HEIGHT));
    // The rectangular panel of the document equals its standalone export.
    for y in 0..HEIGHT {
        let row = (y * document.width * 4) as usize;
        assert_eq!(
            &document.rgba[row..row + (WIDTH * 4) as usize],
            &plain.rgba[(y * WIDTH * 4) as usize..((y + 1) * WIDTH * 4) as usize],
            "row {y} of the rectangular panel changed"
        );
    }
    // The polar panel draws its points on its ring, offset by its area.
    let p = polar_px(&subplot, 90.0, 0.5);
    assert!(is_red(at(&document, p)), "polar subplot sample at {p:?}");
    let p = polar_px(&subplot, 180.0, 1.0);
    assert!(is_red(at(&document, p)), "polar subplot sample at {p:?}");
}

#[test]
fn grid_axes_and_labels_draw_rings_spokes_and_angle_formats_at_both_scales() {
    let mut renderer = renderer();
    upload(&mut renderer, vec![0.0], vec![0.5]);
    let mut chart = bare_chart(PolarOptions {
        angle_unit: AngleUnit::Radians,
        ..PolarOptions::default()
    });
    chart.set_x_range(0.0, std::f64::consts::TAU);
    {
        let config = chart.config_mut();
        config.grid.show_major_x = true;
        config.grid.show_major_y = true;
        config.bottom_x.line_visible = true;
        config.bottom_x.tick = renderer::config::TickVisibility::Outside;
        config.left_y.line_visible = true;
        config.grid.major_x_color = Color::new(0.0, 0.0, 1.0, 1.0);
        config.grid.major_y_color = Color::new(0.0, 0.0, 1.0, 1.0);
        config.grid.major_x_width = 2.0;
        config.grid.major_y_width = 2.0;
        config.bottom_x.major_spacing = std::f64::consts::FRAC_PI_4;
        config.bottom_x.label_style.visible = true;
        config.bottom_x.label_style.format = LabelFormat::Radians;
        config.left_y.major_spacing = 0.5;
        config.left_y.label_style.visible = true;
    }
    let series = [scatter("points", 4.0)];
    let img = export(&mut renderer, &chart, &series);
    let config = chart.config();
    let is_blue = |p: &[u8]| p[3] > 180 && p[2] > 150 && p[0] < 120 && p[1] < 120;
    // A spoke at 45 degrees and the ring at r = 0.5 are blue; the space
    // between spokes at r = 0.75 is not.
    let spoke = polar_px(config, std::f64::consts::FRAC_PI_4, 0.75);
    let ring = polar_px(config, 0.3, 0.5);
    let gap = polar_px(config, 0.3, 0.75);
    assert!(
        is_blue(at(&img, spoke)),
        "spoke at {spoke:?}: {:?}",
        at(&img, spoke)
    );
    assert!(
        is_blue(at(&img, ring)),
        "ring at {ring:?}: {:?}",
        at(&img, ring)
    );
    assert!(
        !is_blue(at(&img, gap)),
        "gap at {gap:?}: {:?}",
        at(&img, gap)
    );
    // The outer ring is the angle axis line (dark).
    let outer = polar_px(config, 0.3, 1.0);
    let p = at(&img, outer);
    assert!(
        p[0] < 160 && p[1] < 160 && p[2] < 160,
        "axis ring at {outer:?}: {p:?}"
    );
    // Labels draw outside the ring: some dark ink past the ring on the
    // right and above the top.
    let geometry = config.polar_geometry().unwrap();
    let dark = |img: &RasterImage, x0: u32, y0: u32, w: u32, h: u32| {
        (y0..y0 + h)
            .flat_map(|y| (x0..x0 + w).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let p = pixel(img, *x, *y);
                p[0] < 100 && p[1] < 100 && p[2] < 100
            })
            .count()
    };
    let cx = geometry.center[0] as u32;
    let cy = geometry.center[1] as u32;
    let r = geometry.radius as u32;
    assert!(
        dark(&img, cx + r + 8, cy - 12, 40, 24) > 10,
        "angle label at 0"
    );
    assert!(
        dark(&img, cx - 30, cy - r - 24, 60, 18) > 10,
        "angle label at pi/2"
    );
    // The 2x export doubles every position.
    let img2 = renderer.export_panel_rgba(&chart, &series, 2.0).unwrap();
    assert_eq!((img2.width, img2.height), (2 * WIDTH, 2 * HEIGHT));
    let scaled = chart.config().scaled(2.0);
    let spoke2 = polar_px(&scaled, std::f64::consts::FRAC_PI_4, 0.75);
    assert!(is_blue(at(&img2, spoke2)), "2x spoke at {spoke2:?}");
    let sample2 = polar_px(&scaled, 0.0, 0.5);
    assert!(is_red(at(&img2, sample2)), "2x sample at {sample2:?}");
}
