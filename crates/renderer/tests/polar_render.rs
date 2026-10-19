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

/// `polar_px` moved `px` pixels toward the centre: a sample on the outer
/// ring is cut there, so its ink is tested just inside.
fn inward(config: &Config, theta: f64, r: f64, px: f64) -> (f64, f64) {
    let geometry = config.polar_geometry().expect("geometry");
    let (x, y) = polar_px(config, theta, r);
    let (dx, dy) = (
        x - f64::from(geometry.center[0]),
        y - f64::from(geometry.center[1]),
    );
    let len = dx.hypot(dy).max(1e-6);
    (x - dx / len * px, y - dy / len * px)
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
            let p = inward(config, theta, r, 3.0);
            assert!(
                is_red(at(&img, p)),
                "{name}: ({theta}, {r}) should draw at {p:?}, got {:?}",
                at(&img, p)
            );
        }
        // The compass variant puts 0 degrees at the top and 90 on the right.
        let top = (
            f64::from(geometry.center[0]),
            f64::from(geometry.center[1] - geometry.radius) + 3.0,
        );
        let right = (
            f64::from(geometry.center[0] + geometry.radius) - 3.0,
            f64::from(geometry.center[1]),
        );
        assert!(is_red(at(&img, top)) && is_red(at(&img, right)), "{name}");
        // Nothing draws where a rectangular chart would put (0, 1): the top
        // left corner of the data area.
        let area = config.data_area().unwrap().0;
        let corner = (area.x as f64 + 4.0, area.y as f64 + 4.0);
        assert!(!is_red(at(&img, corner)), "{name}: Cartesian corner");
        // Four half markers cut at the ring, one whole one, nothing else.
        let ink = count(&img, is_red);
        assert!(
            (3 * 120..5 * 260).contains(&ink),
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
    let ring_mid = inward(config, 45.0, 1.0, 1.0);
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
        let p = inward(config, theta, 1.0, 1.0);
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
        !is_red(at(&img, inward(config, 180.0, 1.0, 1.0))),
        "a sample outside the sector is not drawn"
    );
    assert!(
        !is_red(at(&img, inward(config, 225.0, 1.0, 1.0))),
        "nor the segment to it"
    );
}

/// Zooming the radius axis in: a sample past `left_y.max` projects past the
/// outer ring, and the fragment clip cuts it there, like the data-area
/// scissor cuts a rectangular chart. The hole and the sector edges cut too.
#[test]
fn primitives_are_cut_at_the_outer_ring_the_hole_and_the_sector_edges() {
    let mut ring_renderer = renderer();
    // A segment from r = 0.4 at 0 degrees to r = 2 at 90 degrees, and a
    // marker at r = 2: the axis ends at 1.
    upload(
        &mut ring_renderer,
        vec![0.0, 90.0, 180.0],
        vec![0.4, 2.0, 2.0],
    );
    let chart = bare_chart(PolarOptions {
        interpolation: PolarInterpolation::Arc { pieces: 16 },
        ..PolarOptions::default()
    });
    let series = [line("path", line_style(4.0)), scatter("points", 10.0)];
    let img = export(&mut ring_renderer, &chart, &series);
    save("polar-clip-ring", &img);
    let config = chart.config();
    let geometry = config.polar_geometry().unwrap();
    // On the arc from (0, 0.4) to (90, 2) the radius passes 1 at the
    // angle where the linear interpolation reaches it: t = 0.375, 33.75
    // degrees. Inside the ring the path draws, outside it does not.
    assert!(
        is_red(at(&img, polar_px(config, 20.0, 0.4 + 1.6 * 20.0 / 90.0))),
        "path inside the ring"
    );
    let outside = polar_px(config, 60.0, 0.4 + 1.6 * 60.0 / 90.0);
    assert!(!is_red(at(&img, outside)), "path cut past the ring");
    // The marker at (180, 2) sits past the ring inside the data area and
    // is not drawn; nothing red lies left of the ring.
    let area = config.data_area().unwrap().0;
    let left = (area.x as f64 + 3.0, f64::from(geometry.center[1]));
    assert!(area.x as f32 + 6.0 < geometry.center[0] - geometry.radius);
    assert!(!is_red(at(&img, left)), "marker past the ring at {left:?}");
    assert!(
        !is_red(at(&img, inward(config, 180.0, 1.0, -2.0))),
        "ring edge"
    );
    let whole = count(&img, is_red);
    let mut unclipped = bare_chart(PolarOptions {
        interpolation: PolarInterpolation::Arc { pieces: 16 },
        ..PolarOptions::default()
    });
    unclipped.set_y_range(0.0, 2.0);
    let unclipped = export(&mut ring_renderer, &unclipped, &series);
    assert!(
        count(&unclipped, is_red) > whole,
        "a wider radius range draws more of the path"
    );

    // A hole: a chord between opposite samples does not cross it.
    let mut hole_renderer = renderer();
    upload(&mut hole_renderer, vec![0.0, 180.0], vec![1.0, 1.0]);
    let hole = bare_chart(PolarOptions {
        inner_radius_fraction: 0.4,
        ..PolarOptions::default()
    });
    let img = export(&mut hole_renderer, &hole, &[line("path", line_style(4.0))]);
    let config = hole.config();
    let centre = (f64::from(geometry.center[0]), f64::from(geometry.center[1]));
    assert!(!is_red(at(&img, centre)), "chord cut at the hole");
    assert!(
        is_red(at(&img, polar_px(config, 0.0, 0.7))),
        "chord drawn between the hole and the ring"
    );
    let in_hole = (
        f64::from(geometry.center[0] + geometry.radius * 0.2),
        f64::from(geometry.center[1]),
    );
    assert!(!is_red(at(&img, in_hole)), "chord cut inside the hole");

    // A sector: an arc from 45 to 135 degrees is cut at 90.
    let mut sector_renderer = renderer();
    upload(&mut sector_renderer, vec![45.0, 135.0], vec![1.0, 1.0]);
    let sector = bare_chart(PolarOptions {
        sector: Some([0.0, 90.0]),
        interpolation: PolarInterpolation::Arc { pieces: 16 },
        ..PolarOptions::default()
    });
    let img = export(
        &mut sector_renderer,
        &sector,
        &[line("path", line_style(4.0))],
    );
    let config = sector.config();
    assert!(
        is_red(at(&img, inward(config, 60.0, 1.0, 1.0))),
        "arc inside the sector"
    );
    assert!(
        !is_red(at(&img, inward(config, 120.0, 1.0, 1.0))),
        "arc cut at the sector edge"
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
    assert!(
        is_red(at(&img, inward(config, 0.0, 10.0, 3.0))),
        "outer ring"
    );
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
        let ring_mid = inward(config, 45.0, 1.0, 1.0);
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
    let p = inward(&subplot, 180.0, 1.0, 3.0);
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

/// The label offsets are read in polar frames: the angle labels move along
/// their spokes (`label_offset_y`, outward positive), the radius labels
/// along and across the ray; turning the ray moves its labels with it.
#[test]
fn label_offsets_move_labels_along_spokes_and_in_the_ray_frame() {
    let mut renderer = renderer();
    upload(&mut renderer, vec![0.0], vec![0.5]);
    let chart_with = |angle: f64, bottom: [f32; 2], left: [f32; 2], radius_labels: bool| {
        let mut chart = bare_chart(PolarOptions {
            radial_label_angle_degrees: angle,
            ..PolarOptions::default()
        });
        chart.set_x_range(0.0, 360.0);
        let config = chart.config_mut();
        config.bottom_x.major_spacing = 90.0;
        config.bottom_x.label_style.visible = !radius_labels;
        config.bottom_x.label_style.label_visible = !radius_labels;
        config.bottom_x.label_style.label_offset_x = bottom[0];
        config.bottom_x.label_style.label_offset_y = bottom[1];
        config.bottom_x.line_visible = false;
        config.left_y.major_spacing = 0.5;
        config.left_y.label_style.visible = radius_labels;
        config.left_y.label_style.label_visible = radius_labels;
        config.left_y.label_style.label_offset_x = left[0];
        config.left_y.label_style.label_offset_y = left[1];
        config.left_y.line_visible = false;
        chart
    };
    let series: [SeriesConfig; 0] = [];
    let is_dark = |p: &[u8]| p[3] > 120 && p[0] < 120 && p[1] < 120 && p[2] < 120;
    let centroid = |img: &RasterImage| {
        let (mut sx, mut sy, mut n) = (0.0f64, 0.0f64, 0usize);
        for y in 0..img.height {
            for x in 0..img.width {
                if is_dark(pixel(img, x, y)) {
                    sx += f64::from(x);
                    sy += f64::from(y);
                    n += 1;
                }
            }
        }
        assert!(n > 0, "some label ink");
        (sx / n as f64, sy / n as f64)
    };
    // Angle labels at 0, 90, 180 and 270 degrees: the extreme ink moves
    // outward on every side by the radial offset, so the span grows by
    // twice the offset in both directions and `label_offset_x` changes
    // nothing.
    let span = |img: &RasterImage| {
        let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0, u32::MAX, 0);
        for y in 0..img.height {
            for x in 0..img.width {
                if is_dark(pixel(img, x, y)) {
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        (x1 - x0, y1 - y0, x0, y0)
    };
    let plain = export(
        &mut renderer,
        &chart_with(90.0, [0.0, 0.0], [0.0, 0.0], false),
        &series,
    );
    let outward = export(
        &mut renderer,
        &chart_with(90.0, [0.0, 12.0], [0.0, 0.0], false),
        &series,
    );
    let sideways = export(
        &mut renderer,
        &chart_with(90.0, [30.0, 0.0], [0.0, 0.0], false),
        &series,
    );
    save("polar-angle-labels-outward", &outward);
    let (w0, h0, _, _) = span(&plain);
    let (w1, h1, x1, y1) = span(&outward);
    assert!(
        (w1 as i64 - w0 as i64 - 24).abs() <= 2,
        "width {w0} -> {w1}"
    );
    assert!(
        (h1 as i64 - h0 as i64 - 24).abs() <= 2,
        "height {h0} -> {h1}"
    );
    let (_, _, x0, y0) = span(&plain);
    assert_eq!((x1 as i64 - x0 as i64, y1 as i64 - y0 as i64), (-12, -12));
    assert_eq!(
        plain.rgba, sideways.rgba,
        "label_offset_x has no polar effect"
    );

    // Radius labels on a rightward ray: `label_offset_x` moves the ink
    // right (along), `label_offset_y` up (across, the label side).
    let base = centroid(&export(
        &mut renderer,
        &chart_with(0.0, [0.0, 0.0], [0.0, 0.0], true),
        &series,
    ));
    let along = centroid(&export(
        &mut renderer,
        &chart_with(0.0, [0.0, 0.0], [20.0, 0.0], true),
        &series,
    ));
    let across = centroid(&export(
        &mut renderer,
        &chart_with(0.0, [0.0, 0.0], [0.0, 15.0], true),
        &series,
    ));
    assert!(
        (along.0 - base.0 - 20.0).abs() < 1.5 && (along.1 - base.1).abs() < 1.5,
        "{base:?} {along:?}"
    );
    assert!(
        (across.1 - base.1 + 15.0).abs() < 1.5 && (across.0 - base.0).abs() < 1.5,
        "{base:?} {across:?}"
    );
    // On an upward ray the same fields read up and left.
    let up = centroid(&export(
        &mut renderer,
        &chart_with(90.0, [0.0, 0.0], [0.0, 0.0], true),
        &series,
    ));
    let up_along = centroid(&export(
        &mut renderer,
        &chart_with(90.0, [0.0, 0.0], [20.0, 0.0], true),
        &series,
    ));
    let up_across = centroid(&export(
        &mut renderer,
        &chart_with(90.0, [0.0, 0.0], [0.0, 15.0], true),
        &series,
    ));
    assert!(
        (up_along.1 - up.1 + 20.0).abs() < 1.5 && (up_along.0 - up.0).abs() < 1.5,
        "{up:?} {up_along:?}"
    );
    assert!(
        (up_across.0 - up.0 + 15.0).abs() < 1.5 && (up_across.1 - up.1).abs() < 1.5,
        "{up:?} {up_across:?}"
    );
    // The hit map agrees with the raster: the rightward ray's labels sit
    // above it and move with their offset.
    let chart = chart_with(0.0, [0.0, 0.0], [0.0, 15.0], true);
    let map = renderer::select::HitMap::standard_chart();
    let measure = renderer::text_render::CpuTextMeasure::for_style(&chart.config().draw_style);
    // (At the ink's height, on the ray's middle: the ink centroid itself
    // leans left of the centre because the label at radius 0 is centred
    // on it, which the representative strip does not model.)
    let geometry = chart.config().polar_geometry().unwrap();
    let probe = (geometry.center[0] + geometry.radius * 0.6, across.1 as f32);
    let hit = map
        .hit_test(chart.config(), &measure, probe.0, probe.1)
        .and_then(|id| map.get(id))
        .map(|el| el.element_id());
    assert_eq!(hit.as_deref(), Some("tick_labels_left"), "{probe:?}");
}
