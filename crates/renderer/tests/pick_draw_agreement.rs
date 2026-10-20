//! Picks against drawn pixels: every pixel a data primitive paints is
//! pickable at distance zero (or within its anti-aliasing fringe), and no
//! pixel picks at distance zero where nothing is painted.
//!
//! The GPU/CPU parity tests in `gpu_pick.rs` compare the picker with itself.
//! These tests compare it with the renderer's own output instead: each case
//! exports the chart through the public `export_panel_rgba` at 1x and 2x,
//! exports it again without the series under test, and asks the GPU picker
//! (`pick_chart` at the same display scale) about every pixel of a window
//! around the primitive. A pixel counts as painted when it differs from the
//! series-free export.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::data::Column;
use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, ChartId, Color, DataRenderType, DataScatterStyleConfig, GpuPickRequest, PickedPoint,
    RasterImage, Renderer, RendererDevice, ScatterShape, SeriesConfig,
};

pub const WIDTH: u32 = 200;
pub const HEIGHT: u32 = 160;

/// A channel difference at or above this marks a painted pixel. Low enough
/// for the faintest anti-aliased fringe the 8-bit export keeps.
const PAINTED: u8 = 6;

/// How strictly a case compares picks with paint.
#[derive(Clone, Copy)]
pub struct Tolerance {
    /// Ink at or above this is a painted pixel that must pick.
    pub painted: u8,
    /// How far a painted fringe pixel may be from the picked geometry: one
    /// anti-aliasing width, `fwidth` of the coverage distance. For a
    /// unit-gradient distance that is at most √2; finite differences across
    /// a box's corner reach 2.
    pub fringe: f32,
    /// The least ink a zero-distance pick must find: zero distance is at
    /// least half the shader's coverage.
    pub zero_ink: u8,
    /// Painted pixels within this disc `(x, y, radius)` (export pixels) are
    /// not required to pick.
    pub skip_near: Option<(f64, f64, f64)>,
}

pub const STRICT: Tolerance = Tolerance {
    painted: PAINTED,
    fringe: 2.0,
    zero_ink: 96,
    skip_near: None,
};

/// A dashed stroke's quad has no anti-aliasing margin, so the export's
/// multisampling covers only part of a pixel whose centre is just inside
/// the stroke's edge: half the shader's coverage, halved again at a corner.
pub const MSAA_EDGE: Tolerance = Tolerance {
    zero_ink: 30,
    ..STRICT
};

/// A sketch marker's contour noise is a function of the angle around its
/// centre, so near the centre `fwidth` of its distance grows like `1 / r`
/// and the fragment's own anti-aliasing paints a faint haze there, far
/// inside an outline. That haze is not geometry the picker can reproduce,
/// so the pixels within three pixels of the centre are not required to pick.
pub fn sketch_haze(center: (f64, f64), scale: f32) -> Tolerance {
    Tolerance {
        skip_near: Some((center.0, center.1, 3.0 * f64::from(scale))),
        ..STRICT
    }
}

const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const BLUE: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

pub fn col(data: Vec<f64>) -> Column<f64> {
    let finite = data.iter().copied().filter(|v| v.is_finite());
    let min = finite.clone().fold(f64::INFINITY, f64::min);
    let max = finite.fold(f64::NEG_INFINITY, f64::max);
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

pub fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Bgra8Unorm,
        8 * 1024 * 1024,
    )
    .expect("renderer init");
    renderer.enable_gpu_picking().expect("GPU picking");
    renderer
}

/// A chart over 0..10 on both axes with no grid, legend, labels or axis
/// lines, so the series-free export is the frame only.
pub fn bare_chart() -> Chart {
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
    // No title, tick labels, ticks or margins: the data area is nearly the
    // whole chart, so the frame drawn over the data stays far from the
    // primitives under test.
    config.chart_title.top_margin = 0.0;
    config.chart_title.visible = false;
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.out_margin = 0.0;
        axis.major_tick_length = 0.0;
        axis.minor_tick_length = 0.0;
        axis.label_style.visible = false;
        axis.title_option.visible = false;
    }
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 10.0);
    chart
}

pub fn scatter(id: &str, shape: ScatterShape, size: f32, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: color,
                point_shape: shape,
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

pub fn upload_xy(renderer: &mut Renderer, xs: Vec<f64>, ys: Vec<f64>) {
    renderer
        .add_columns(&[
            ("x", &col(xs) as &dyn renderer::ColumnSource),
            ("y", &col(ys) as &dyn renderer::ColumnSource),
        ])
        .expect("upload");
}

/// Data (x, y) → export pixel at `scale`, through the chart's data area.
pub fn data_to_px(chart: &Chart, x: f64, y: f64, scale: f32) -> (f64, f64) {
    let cfg = chart.config().scaled(scale);
    let da = cfg.data_area().expect("data area");
    let tx = (x - cfg.bottom_x.min) / (cfg.bottom_x.max - cfg.bottom_x.min);
    let ty = (y - cfg.left_y.min) / (cfg.left_y.max - cfg.left_y.min);
    (
        da.x as f64 + tx * da.width as f64,
        (da.y + da.height) as f64 - ty * da.height as f64,
    )
}

/// The largest premultiplied channel difference between two exports at one
/// pixel. The export is straight alpha, so a faint fringe keeps its full
/// colour; premultiplying weighs it by its coverage.
pub fn difference(a: &RasterImage, b: &RasterImage, x: u32, y: u32) -> u8 {
    let i = ((y * a.width + x) * 4) as usize;
    let premultiplied = |img: &RasterImage, c: usize| {
        let alpha = u32::from(img.rgba[i + 3]);
        if c == 3 {
            alpha
        } else {
            (u32::from(img.rgba[i + c]) * alpha + 127) / 255
        }
    };
    (0..4)
        .map(|c| premultiplied(a, c).abs_diff(premultiplied(b, c)))
        .max()
        .unwrap() as u8
}

pub fn pixel(img: &RasterImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

/// One registered chart and the request geometry of an export at `scale`.
pub struct Probe {
    pub id: ChartId,
    pub scale: f32,
}

impl Probe {
    pub fn register(
        renderer: &mut Renderer,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Self {
        let id = renderer
            .register_chart(chart.config().clone(), series.to_vec())
            .expect("register chart");
        Self { id, scale }
    }

    fn request(&self, x: f32, y: f32, max: f32) -> GpuPickRequest {
        GpuPickRequest {
            canvas_position_px: [x, y],
            display_panel_px: Rect {
                x: 0,
                y: 0,
                width: (WIDTH as f32 * self.scale).round() as u32,
                height: (HEIGHT as f32 * self.scale).round() as u32,
            },
            display_scale: self.scale,
            max_distance_px: max,
        }
    }

    /// The point pick at the centre of export pixel `(x, y)`.
    pub fn pick(&self, renderer: &mut Renderer, x: u32, y: u32, max: f32) -> Option<PickedPoint> {
        let request = self.request(x as f32 + 0.5, y as f32 + 0.5, max);
        pollster::block_on(renderer.pick_chart(self.id, request).unwrap().resolve()).unwrap()
    }

    /// The typed data pick at the centre of export pixel `(x, y)`.
    pub fn pick_data(
        &self,
        renderer: &mut Renderer,
        x: u32,
        y: u32,
        max: f32,
    ) -> Option<renderer::PickedData> {
        let request = self.request(x as f32 + 0.5, y as f32 + 0.5, max);
        pollster::block_on(
            renderer
                .pick_chart_data(self.id, request)
                .unwrap()
                .resolve(),
        )
        .unwrap()
    }
}

/// Exports `series` and the chart without the series named `subject` at
/// `scale`, then checks every pixel of `window` (export pixels, inclusive
/// start, exclusive end; `None` takes the painted pixels' bounding box grown
/// by four pixels): a painted pixel must pick `subject` within the fringe, a
/// pixel picked at distance zero must be at least half covered. Returns the
/// number of painted pixels, so a case cannot pass by drawing nothing.
pub fn assert_pick_matches_paint(
    renderer: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    subject: &str,
    scale: f32,
    window: Option<(u32, u32, u32, u32)>,
    what: &str,
) -> usize {
    assert_pick_matches_paint_with(
        renderer, chart, series, subject, scale, window, what, STRICT,
    )
}

/// [`assert_pick_matches_paint`] at a given [`Tolerance`].
#[allow(clippy::too_many_arguments)]
pub fn assert_pick_matches_paint_with(
    renderer: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    subject: &str,
    scale: f32,
    window: Option<(u32, u32, u32, u32)>,
    what: &str,
    tolerance: Tolerance,
) -> usize {
    let with = renderer.export_panel_rgba(chart, series, scale).unwrap();
    let without_series: Vec<SeriesConfig> = series
        .iter()
        .filter(|s| s.series_id != subject)
        .cloned()
        .collect();
    let without = renderer
        .export_panel_rgba(chart, &without_series, scale)
        .unwrap();
    let (x0, y0, x1, y1) = window.unwrap_or_else(|| ink_window(&with, &without, 4));
    let probe = Probe::register(renderer, chart, series, scale);
    let mut painted = 0usize;
    let mut failures = Vec::new();
    for y in y0..y1.min(with.height) {
        for x in x0..x1.min(with.width) {
            let ink = difference(&with, &without, x, y);
            let hit = probe
                .pick(renderer, x, y, tolerance.fringe)
                .filter(|hit| hit.series_id == subject);
            let skipped = tolerance.skip_near.is_some_and(|(cx, cy, radius)| {
                (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy) <= radius
            });
            if ink >= tolerance.painted {
                painted += 1;
                if hit.is_none() && !skipped {
                    failures.push(format!("({x}, {y}) painted (ink {ink}) but not picked"));
                }
            }
            if let Some(hit) = hit
                && hit.distance_px == 0.0
                && ink < tolerance.zero_ink
            {
                failures.push(format!(
                    "({x}, {y}) picked at distance 0 (index {}) but ink is only {ink}",
                    hit.point_index
                ));
            }
        }
    }
    renderer.remove_chart(probe.id).unwrap();
    assert!(
        failures.is_empty(),
        "{what} at {scale}x: {} disagreements, first {:?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
    painted
}

/// The bounding box of every pixel where the two exports differ, grown by
/// `pad` pixels and clamped to the image.
pub fn ink_window(with: &RasterImage, without: &RasterImage, pad: u32) -> (u32, u32, u32, u32) {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..with.height {
        for x in 0..with.width {
            if difference(with, without, x, y) >= PAINTED {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    assert!(x0 < x1, "nothing is painted");
    (
        x0.saturating_sub(pad),
        y0.saturating_sub(pad),
        (x1 + pad).min(with.width),
        (y1 + pad).min(with.height),
    )
}

/// A window of `half` export pixels around data point `(x, y)`.
pub fn window_around(chart: &Chart, x: f64, y: f64, scale: f32, half: f64) -> (u32, u32, u32, u32) {
    let (cx, cy) = data_to_px(chart, x, y, scale);
    let half = half * f64::from(scale);
    (
        (cx - half).floor().max(0.0) as u32,
        (cy - half).floor().max(0.0) as u32,
        (cx + half).ceil() as u32,
        (cy + half).ceil() as u32,
    )
}

const ALL_SHAPES: [ScatterShape; 26] = [
    ScatterShape::Circle,
    ScatterShape::Square,
    ScatterShape::Triangle,
    ScatterShape::Diamond,
    ScatterShape::Cross,
    ScatterShape::CircleFilled,
    ScatterShape::SquareFilled,
    ScatterShape::TriangleFilled,
    ScatterShape::DiamondFilled,
    ScatterShape::TriangleDown,
    ScatterShape::TriangleLeft,
    ScatterShape::TriangleRight,
    ScatterShape::Plus,
    ScatterShape::Pentagon,
    ScatterShape::Hexagon,
    ScatterShape::Octagon,
    ScatterShape::Star,
    ScatterShape::TriangleDownFilled,
    ScatterShape::TriangleLeftFilled,
    ScatterShape::TriangleRightFilled,
    ScatterShape::PlusFilled,
    ScatterShape::CrossFilled,
    ScatterShape::PentagonFilled,
    ScatterShape::HexagonFilled,
    ScatterShape::OctagonFilled,
    ScatterShape::StarFilled,
];

/// G1: every marker shape picks exactly where its outline or fill is drawn,
/// at 1x and at a 2x export: square corners and cross tips pick, triangle
/// and star concavities and hollow outline interiors do not.
#[test]
fn every_marker_shape_picks_where_it_is_drawn() {
    let mut renderer = renderer();
    upload_xy(&mut renderer, vec![5.0], vec![5.0]);
    let chart = bare_chart();
    for shape in ALL_SHAPES {
        for scale in [1.0, 2.0] {
            let series = [scatter("marker", shape.clone(), 7.0, RED)];
            let window = window_around(&chart, 5.0, 5.0, scale, 16.0);
            let painted = assert_pick_matches_paint(
                &mut renderer,
                &chart,
                &series,
                "marker",
                scale,
                Some(window),
                &format!("{shape:?}"),
            );
            assert!(
                painted > 10,
                "{shape:?} at {scale}x painted {painted} pixels"
            );
        }
    }
}

/// G12: where two markers overlap, the one painted on top (the higher
/// instance index) is the one picked.
#[test]
fn overlapping_markers_pick_the_marker_painted_on_top() {
    let mut renderer = renderer();
    upload_xy(&mut renderer, vec![5.0, 5.0], vec![5.0, 5.0]);
    let chart = bare_chart();
    let series = [scatter("pair", ScatterShape::CircleFilled, 6.0, BLUE)];
    let probe = Probe::register(&mut renderer, &chart, &series, 1.0);
    let (cx, cy) = data_to_px(&chart, 5.0, 5.0, 1.0);
    let hit = probe
        .pick(&mut renderer, cx as u32, cy as u32, 0.0)
        .expect("overlap hit");
    assert_eq!(
        hit.point_index, 1,
        "the second marker paints over the first"
    );
}

/// G12 across colours: two overlapping markers of one series, the painted
/// colour under the cursor names the index the picker answers.
#[test]
fn overlap_tie_follows_the_painted_colour() {
    let mut renderer = renderer();
    upload_xy(&mut renderer, vec![5.0, 5.2], vec![5.0, 5.0]);
    renderer
        .add_columns(&[("style", &col(vec![0.0, 1.0]) as &dyn renderer::ColumnSource)])
        .unwrap();
    let chart = bare_chart();
    let mut series = scatter("pair", ScatterShape::CircleFilled, 8.0, RED);
    let DataRenderType::Scatter { scatter } = &mut series.render_type else {
        unreachable!()
    };
    scatter.point_style_index_column = Some("style".into());
    scatter.point_style_table = Some(vec![
        renderer::data_config::DataScatterPointStyleConfig {
            point_color: Some(RED),
            ..Default::default()
        },
        renderer::data_config::DataScatterPointStyleConfig {
            point_color: Some(BLUE),
            ..Default::default()
        },
    ]);
    let series = [series];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let probe = Probe::register(&mut renderer, &chart, &series, 1.0);
    let (ax, ay) = data_to_px(&chart, 5.0, 5.0, 1.0);
    let (bx, _) = data_to_px(&chart, 5.2, 5.0, 1.0);
    let mid = ((ax + bx) * 0.5) as u32;
    let p = pixel(&img, mid, ay as u32);
    assert!(
        p[2] > 200 && p[0] < 60,
        "blue (index 1) is painted on top: {p:?}"
    );
    let hit = probe.pick(&mut renderer, mid, ay as u32, 0.0).expect("hit");
    assert_eq!(hit.point_index, 1);
}

/// A polar chart over 0..360 degrees and 0..1 radius without chrome.
pub fn bare_polar_chart(polar: renderer::polar::PolarOptions) -> Chart {
    let mut chart = bare_chart();
    let config = chart.config_mut();
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.label_style.visible = false;
        axis.line_visible = false;
        axis.tick = renderer::config::TickVisibility::None;
    }
    config.polar = Some(polar);
    chart.set_x_range(0.0, 360.0);
    chart.set_y_range(0.0, 1.0);
    chart
}

/// G11: the polar fragment clip cuts markers at the outer ring, the hole
/// and the sector edges, and the picker applies the same clip at the
/// cursor: a marker straddling an edge picks on its drawn part only, and a
/// marker whose centre lies outside the sector still picks where its part
/// inside the sector is drawn.
#[test]
fn polar_clip_cuts_markers_for_picking_as_it_cuts_them_for_drawing() {
    use renderer::polar::PolarOptions;
    let mut renderer = renderer();
    // On the outer ring at 45 degrees; on the sector's start edge; just
    // outside the sector's end edge; on the hole's edge.
    upload_xy(
        &mut renderer,
        vec![45.0, 0.0, 92.0, 60.0],
        vec![1.0, 0.6, 0.6, 0.0],
    );
    for (polar, what) in [
        (
            PolarOptions {
                sector: Some([0.0, 90.0]),
                inner_radius_fraction: 0.3,
                ..PolarOptions::default()
            },
            "sector with a hole",
        ),
        (PolarOptions::default(), "full circle"),
    ] {
        let chart = bare_polar_chart(polar);
        for scale in [1.0, 2.0] {
            let series = [scatter("edge", ScatterShape::SquareFilled, 9.0, RED)];
            let painted = assert_pick_matches_paint(
                &mut renderer,
                &chart,
                &series,
                "edge",
                scale,
                None,
                what,
            );
            assert!(painted > 50, "{what}: {painted}");
        }
    }
}

pub fn line(
    id: &str,
    width: f32,
    style: renderer::line::LineStylePreset,
    interpolation: renderer::data_config::LineInterpolation,
) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Line {
            line: renderer::DataLineStyleConfig {
                line_style: style,
                line_color: RED,
                line_width: width,
                fill: None,
                fill_color: None,
                interpolation,
            },
        },
    }
}

/// G4: a solid line picks exactly its drawn stroke — at least one pixel
/// wide however thin it is declared, with square caps at both ends and at
/// every step corner — on a diagonal polyline and a stepped one, at 1x and
/// 2x.
#[test]
fn solid_lines_pick_where_their_stroke_is_drawn() {
    use renderer::data_config::LineInterpolation;
    use renderer::line::LineStylePreset;
    let mut renderer = renderer();
    upload_xy(
        &mut renderer,
        vec![4.0, 4.9, 5.4, 6.1],
        vec![4.2, 5.6, 4.6, 5.9],
    );
    let chart = bare_chart();
    for width in [0.3, 1.0, 4.5] {
        for interpolation in [LineInterpolation::Linear, LineInterpolation::StepMid] {
            for scale in [1.0, 2.0] {
                let series = [line("path", width, LineStylePreset::Solid, interpolation)];
                let painted = assert_pick_matches_paint(
                    &mut renderer,
                    &chart,
                    &series,
                    "path",
                    scale,
                    None,
                    &format!("width {width} {interpolation:?}"),
                );
                assert!(painted > 30, "{painted}");
            }
        }
    }
}

/// G4: a dashed line picks its dashes and not its gaps: the picker walks the
/// arc prefix the line was drawn with, so the dash phase is the drawn one.
#[test]
fn dashed_lines_pick_their_dashes_and_not_their_gaps() {
    use renderer::data_config::LineInterpolation;
    use renderer::line::LineStylePreset;
    let mut renderer = renderer();
    upload_xy(
        &mut renderer,
        vec![3.5, 5.0, 5.8, 6.5],
        vec![4.0, 5.5, 4.4, 6.0],
    );
    let chart = bare_chart();
    for style in [
        LineStylePreset::Dash,
        LineStylePreset::Dot,
        LineStylePreset::DashDotDot,
    ] {
        for interpolation in [LineInterpolation::Linear, LineInterpolation::StepPost] {
            for scale in [1.0, 2.0] {
                let series = [line("dashes", 2.0, style, interpolation)];
                let painted = assert_pick_matches_paint_with(
                    &mut renderer,
                    &chart,
                    &series,
                    "dashes",
                    scale,
                    None,
                    &format!("{style:?} {interpolation:?}"),
                    MSAA_EDGE,
                );
                assert!(
                    painted > 12,
                    "{style:?} {interpolation:?} at {scale}x: {painted}"
                );
            }
        }
    }
}

/// Every painted pixel of `subject` picks it within the fringe; returns the
/// painted count. The other direction does not hold for ink a styled entry
/// scatters by hash (stars) or fades softly (glows, the ribbon), which picks
/// as the envelope its shader constants bound: see
/// [`assert_nothing_picks_outside`].
pub fn assert_painted_pixels_pick(
    renderer: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    subject: &str,
    scale: f32,
    what: &str,
) -> usize {
    let with = renderer.export_panel_rgba(chart, series, scale).unwrap();
    let without_series: Vec<SeriesConfig> = series
        .iter()
        .filter(|s| s.series_id != subject)
        .cloned()
        .collect();
    let without = renderer
        .export_panel_rgba(chart, &without_series, scale)
        .unwrap();
    let (x0, y0, x1, y1) = ink_window(&with, &without, 0);
    let probe = Probe::register(renderer, chart, series, scale);
    let mut painted = 0usize;
    let mut failures = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let ink = difference(&with, &without, x, y);
            if ink < PAINTED {
                continue;
            }
            painted += 1;
            let hit = probe
                .pick(renderer, x, y, STRICT.fringe)
                .filter(|hit| hit.series_id == subject);
            if hit.is_none() {
                failures.push(format!("({x}, {y}) painted (ink {ink}) but not picked"));
            }
        }
    }
    renderer.remove_chart(probe.id).unwrap();
    assert!(
        failures.is_empty(),
        "{what} at {scale}x: {} disagreements, first {:?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
    painted
}

/// No pick of `subject` within `max` pixels at any of `pixels` (export
/// pixels at `scale`).
pub fn assert_nothing_picks_at(
    renderer: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    subject: &str,
    scale: f32,
    pixels: &[(f64, f64)],
    what: &str,
) {
    let probe = Probe::register(renderer, chart, series, scale);
    for &(x, y) in pixels {
        let hit = probe
            .pick(renderer, x.round() as u32, y.round() as u32, 0.0)
            .filter(|hit| hit.series_id == subject);
        assert!(hit.is_none(), "{what}: ({x}, {y}) picks {hit:?}");
    }
    renderer.remove_chart(probe.id).unwrap();
}

fn styled_chart(style: renderer::config::DrawStyle) -> Chart {
    let mut chart = bare_chart();
    chart.config_mut().draw_style = style;
    chart
}

/// G3, sketch: the marker wobble and the stroke displacement are reproduced
/// with the draw's own hashed noise (`sketch_marker_distance`,
/// `sketch_line_offset` on the drawn arc prefix), so a sketch chart picks as
/// exactly as a precise one.
#[test]
fn sketch_markers_and_strokes_pick_where_the_wobble_draws_them() {
    use renderer::config::{DrawStyle, SketchOptions};
    use renderer::data_config::LineInterpolation;
    use renderer::line::LineStylePreset;
    let mut renderer = renderer();
    upload_xy(
        &mut renderer,
        vec![4.0, 4.9, 5.4, 6.1],
        vec![4.2, 5.6, 4.6, 5.9],
    );
    let chart = styled_chart(DrawStyle::Sketch(SketchOptions {
        amplitude_px: 4.0,
        wavelength_px: 12.0,
        seed: 7,
    }));
    // The contour noise of `sketch_marker_distance` has a seam at the angle
    // ±π: a marker centred on a pixel row would put that row on the seam,
    // where the sign of a rounding error picks the side. These markers sit
    // between rows.
    let mut markers = crate::renderer();
    upload_xy(&mut markers, vec![5.0, 5.0], vec![5.0, 5.0]);
    for scale in [1.0, 2.0] {
        for shape in [
            ScatterShape::Circle,
            ScatterShape::SquareFilled,
            ScatterShape::Star,
        ] {
            let series = [scatter("sketched", shape.clone(), 8.0, RED)];
            let painted = assert_pick_matches_paint_with(
                &mut markers,
                &chart,
                &series,
                "sketched",
                scale,
                None,
                &format!("sketch {shape:?}"),
                sketch_haze(data_to_px(&chart, 5.0, 5.0, scale), scale),
            );
            assert!(painted > 50, "{painted}");
        }
        for style in [LineStylePreset::Solid, LineStylePreset::Dash] {
            let series = [line("wobble", 2.0, style, LineInterpolation::Linear)];
            let painted = assert_pick_matches_paint_with(
                &mut renderer,
                &chart,
                &series,
                "wobble",
                scale,
                None,
                &format!("sketch {style:?} line"),
                MSAA_EDGE,
            );
            assert!(painted > 30, "{painted}");
        }
    }
}

/// G3, Milkyway: a planet picks on its disc, its rim glow and its ring; a
/// line picks on its ribbon and on the star envelope around it (the stars
/// are hashed per arc slot, so their envelope is what picks). Every painted
/// pixel picks, and nothing picks beyond the envelope.
#[test]
fn milkyway_planets_and_ribbons_pick_within_their_drawn_envelope() {
    use renderer::config::{DrawStyle, MilkywayOptions};
    use renderer::data_config::LineInterpolation;
    use renderer::line::LineStylePreset;
    let mut renderer = renderer();
    upload_xy(&mut renderer, vec![3.0, 5.0, 7.0], vec![5.0, 5.0, 5.0]);
    let options = MilkywayOptions::default();
    let chart = styled_chart(DrawStyle::Milkyway(options.clone()));
    for scale in [1.0, 2.0] {
        let series = [scatter("planets", ScatterShape::CircleFilled, 6.0, BLUE)];
        let painted =
            assert_painted_pixels_pick(&mut renderer, &chart, &series, "planets", scale, "planets");
        assert!(painted > 100, "{painted}");
        let (cx, cy) = data_to_px(&chart, 5.0, 5.0, scale);
        let far = f64::from(6.0 * scale * 2.5 + 4.0);
        assert_nothing_picks_at(
            &mut renderer,
            &chart,
            &series,
            "planets",
            scale,
            &[(cx, cy - far), (cx, cy + far)],
            "beyond a planet's quad",
        );

        let series = [line(
            "ribbon",
            2.0,
            LineStylePreset::Solid,
            LineInterpolation::Linear,
        )];
        let painted = assert_pick_painted_ribbon(&mut renderer, &chart, &series, scale, &options);
        assert!(painted > 100, "{painted}");
    }
}

fn assert_pick_painted_ribbon(
    renderer: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    scale: f32,
    options: &renderer::config::MilkywayOptions,
) -> usize {
    let painted = assert_painted_pixels_pick(renderer, chart, series, "ribbon", scale, "ribbon");
    // The envelope: one ribbon width, or the star scatter, either side.
    let stars =
        options.spread_px * 1.7 + 4.0 * options.structure_scale + options.star_scale * 4.3 * 4.0;
    let envelope = f64::from(options.ribbon_width_px.max(1.0).max(stars) * scale);
    let (cx, cy) = data_to_px(chart, 5.0, 5.0, scale);
    assert_nothing_picks_at(
        renderer,
        chart,
        series,
        "ribbon",
        scale,
        &[(cx, cy - envelope - 2.0), (cx, cy + envelope + 2.0)],
        "beyond the ribbon's envelope",
    );
    painted
}

/// G3, Constellation: a marker star picks within the disc its PSF sprite's
/// quad inscribes (brightness and tint are hashed and textured), the line is
/// the precise stroke drawn at the style's opacity, and every painted pixel
/// picks.
#[test]
fn constellation_stars_and_lines_pick_within_their_drawn_envelope() {
    use renderer::config::{ConstellationOptions, DrawStyle};
    use renderer::data_config::LineInterpolation;
    use renderer::line::LineStylePreset;
    let mut renderer = renderer();
    upload_xy(&mut renderer, vec![4.0, 5.0, 6.0], vec![4.5, 5.5, 4.8]);
    let chart = styled_chart(DrawStyle::Constellation(ConstellationOptions::default()));
    let series = [SeriesConfig {
        render_type: DataRenderType::ScatterLine {
            scatter: DataScatterStyleConfig {
                point_color: BLUE,
                point_shape: ScatterShape::CircleFilled,
                point_size: 3.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
            line: renderer::DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: RED,
                line_width: 1.5,
                fill: None,
                fill_color: None,
                interpolation: LineInterpolation::Linear,
            },
        },
        ..scatter("sky", ScatterShape::CircleFilled, 3.0, BLUE)
    }];
    for scale in [1.0, 2.0] {
        let painted =
            assert_painted_pixels_pick(&mut renderer, &chart, &series, "sky", scale, "sky");
        assert!(painted > 50, "{painted}");
    }
    // Zero opacities draw nothing and pick nothing.
    let hidden = styled_chart(DrawStyle::Constellation(ConstellationOptions {
        star_opacity: 0.0,
        line_opacity: 0.0,
    }));
    let (ax, ay) = data_to_px(&hidden, 4.0, 4.5, 1.0);
    let (bx, by) = data_to_px(&hidden, 5.0, 5.5, 1.0);
    assert_nothing_picks_at(
        &mut renderer,
        &hidden,
        &series,
        "sky",
        1.0,
        &[(ax, ay), ((ax + bx) * 0.5, (ay + by) * 0.5)],
        "zero opacity",
    );
}

struct F32Columns(Vec<(String, Vec<f32>)>);

impl renderer::PointColumnLookup for F32Columns {
    fn get_f32_column(&self, id: &renderer::ColumnId) -> Option<&[f32]> {
        self.0
            .iter()
            .find(|(name, _)| name == id)
            .map(|(_, values)| values.as_slice())
    }
}

/// The CPU twin (`pick_nearest_point`) answers what the GPU picker answers
/// under every draw style: the styled marker distances, the sketch stroke's
/// wobble on the arc prefix the CPU sums itself, the ribbon envelope and the
/// dash phase.
#[test]
fn cpu_twin_agrees_with_the_gpu_picker_under_every_draw_style() {
    use renderer::config::{ConstellationOptions, DrawStyle, MilkywayOptions, SketchOptions};
    use renderer::data_config::LineInterpolation;
    use renderer::line::LineStylePreset;
    let xs = vec![3.8, 4.6, 5.3, 6.2];
    let ys = vec![4.4, 5.7, 4.5, 5.6];
    let mut renderer = renderer();
    upload_xy(&mut renderer, xs.clone(), ys.clone());
    let columns = F32Columns(vec![
        ("x".into(), xs.iter().map(|v| *v as f32).collect()),
        ("y".into(), ys.iter().map(|v| *v as f32).collect()),
    ]);
    let scatter_line = |shape: ScatterShape, style: LineStylePreset| SeriesConfig {
        render_type: DataRenderType::ScatterLine {
            scatter: DataScatterStyleConfig {
                point_color: BLUE,
                point_shape: shape,
                point_size: 5.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
            line: renderer::DataLineStyleConfig {
                line_style: style,
                line_color: RED,
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: LineInterpolation::Linear,
            },
        },
        ..scatter("path", ScatterShape::Circle, 5.0, BLUE)
    };
    for style in [
        DrawStyle::Precise,
        DrawStyle::Sketch(SketchOptions {
            amplitude_px: 3.0,
            wavelength_px: 15.0,
            seed: 3,
        }),
        DrawStyle::Milkyway(MilkywayOptions::default()),
        DrawStyle::Constellation(ConstellationOptions::default()),
    ] {
        let chart = styled_chart(style.clone());
        for series in [
            scatter_line(ScatterShape::Star, LineStylePreset::Solid),
            scatter_line(ScatterShape::Triangle, LineStylePreset::DashDot),
        ] {
            let series = [series];
            let probe = Probe::register(&mut renderer, &chart, &series, 1.0);
            let (x0, y0) = data_to_px(&chart, 3.5, 6.0, 1.0);
            let (x1, y1) = data_to_px(&chart, 6.5, 4.0, 1.0);
            let mut compared = 0;
            for y in (y0 as u32..y1 as u32).step_by(3) {
                for x in (x0 as u32..x1 as u32).step_by(3) {
                    let gpu = probe.pick(&mut renderer, x, y, 3.0);
                    let cpu = renderer::pick_nearest_point(
                        chart.config(),
                        &series,
                        &columns,
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                        renderer::PointPickOptions {
                            max_distance_px: 3.0,
                        },
                    );
                    match (&cpu, &gpu) {
                        (Some(cpu), Some(gpu)) => {
                            assert!(
                                (cpu.distance_px - gpu.distance_px).abs() < 0.05,
                                "{style:?} ({x}, {y}): CPU {cpu:?} GPU {gpu:?}"
                            );
                            if cpu.distance_px > 0.05 {
                                assert_eq!(
                                    cpu.point_index, gpu.point_index,
                                    "{style:?} ({x}, {y})"
                                );
                            }
                            compared += 1;
                        }
                        (None, None) => {}
                        // A distance at the threshold may round either way.
                        (Some(hit), None) | (None, Some(hit)) => assert!(
                            hit.distance_px > 2.9,
                            "{style:?} ({x}, {y}): CPU {cpu:?} GPU {gpu:?}"
                        ),
                    }
                }
            }
            assert!(compared > 20, "{style:?}: {compared} hits compared");
            renderer.remove_chart(probe.id).unwrap();
        }
    }
}

fn arrows(
    id: &str,
    scale: renderer::data_config::VectorScale,
    pivot: renderer::data_config::VectorPivot,
) -> SeriesConfig {
    use renderer::data_config::VectorComponents;
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Vector {
            components: VectorComponents::Points {
                u: "u".into(),
                v: "v".into(),
            },
            style: renderer::data_config::VectorStyle {
                color: RED,
                width_px: 3.0,
                scale,
                pivot,
                head_length_px: 9.0,
                head_width_px: 8.0,
                min_length_px: 6.0,
                color_map: None,
            },
        },
    }
}

/// G5: a point vector picks the arrow it draws — shaft and head, after the
/// pivot, the scale mode and the head scaled down on a short arrow — and an
/// arrow that is not drawn (a zero vector, one under the minimum length)
/// picks nowhere, at 1x and 2x.
#[test]
fn point_vectors_pick_the_arrow_they_draw() {
    use renderer::data_config::{VectorPivot, VectorScale};
    let mut renderer = renderer();
    // Long diagonal, short (head scaled down), zero, and under min length.
    let xs = vec![2.0, 6.5, 8.0, 3.0];
    let ys = vec![2.0, 6.5, 2.5, 8.0];
    let us = vec![30.0, -6.0, 0.0, 2.0];
    let vs = vec![18.0, 4.0, 0.0, 2.0];
    let f32s = |values: &[f64]| values.iter().map(|v| *v as f32).collect::<Vec<_>>();
    let columns = F32Columns(vec![
        ("x".into(), f32s(&xs)),
        ("y".into(), f32s(&ys)),
        ("u".into(), f32s(&us)),
        ("v".into(), f32s(&vs)),
    ]);
    renderer
        .add_columns(&[
            ("x", &col(xs) as &dyn renderer::ColumnSource),
            ("y", &col(ys) as &dyn renderer::ColumnSource),
            ("u", &col(us) as &dyn renderer::ColumnSource),
            ("v", &col(vs) as &dyn renderer::ColumnSource),
        ])
        .expect("upload");
    let chart = bare_chart();
    for (scale_mode, pivot) in [
        (VectorScale::Pixels { px_per_unit: 1.0 }, VectorPivot::Tail),
        (
            VectorScale::Pixels { px_per_unit: 1.0 },
            VectorPivot::Middle,
        ),
        (VectorScale::Data { factor: 0.05 }, VectorPivot::Tip),
    ] {
        for scale in [1.0, 2.0] {
            let series = [arrows("quiver", scale_mode.clone(), pivot.clone())];
            let what = format!("{scale_mode:?} {pivot:?}");
            let painted = assert_pick_matches_paint(
                &mut renderer,
                &chart,
                &series,
                "quiver",
                scale,
                None,
                &what,
            );
            assert!(painted > 60, "{what}: {painted}");
            let hidden = [
                data_to_px(&chart, 8.0, 2.5, scale),
                data_to_px(&chart, 3.0, 8.0, scale),
            ];
            assert_nothing_picks_at(
                &mut renderer,
                &chart,
                &series,
                "quiver",
                scale,
                &hidden,
                &format!("{what}: undrawn arrows"),
            );
        }

        // The CPU twin answers what the GPU picker answers.
        let series = [arrows("quiver", scale_mode.clone(), pivot.clone())];
        let probe = Probe::register(&mut renderer, &chart, &series, 1.0);
        let (x0, y0) = data_to_px(&chart, 1.0, 9.0, 1.0);
        let (x1, y1) = data_to_px(&chart, 8.0, 1.0, 1.0);
        let mut compared = 0;
        for y in (y0 as u32..y1 as u32).step_by(3) {
            for x in (x0 as u32..x1 as u32).step_by(3) {
                let gpu = probe.pick(&mut renderer, x, y, 3.0);
                let cpu = renderer::pick_nearest_point(
                    chart.config(),
                    &series,
                    &columns,
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    renderer::PointPickOptions {
                        max_distance_px: 3.0,
                    },
                );
                match (&cpu, &gpu) {
                    (Some(cpu), Some(gpu)) => {
                        assert!(
                            (cpu.distance_px - gpu.distance_px).abs() < 0.05
                                && cpu.point_index == gpu.point_index,
                            "{scale_mode:?} ({x}, {y}): CPU {cpu:?} GPU {gpu:?}"
                        );
                        compared += 1;
                    }
                    (None, None) => {}
                    (Some(hit), None) | (None, Some(hit)) => assert!(
                        hit.distance_px > 2.9,
                        "{scale_mode:?} ({x}, {y}): CPU {cpu:?} GPU {gpu:?}"
                    ),
                }
            }
        }
        assert!(compared > 20, "{scale_mode:?}: {compared} hits compared");
        renderer.remove_chart(probe.id).unwrap();
    }
}

/// G6: a grid vector picks (as its cell) exactly the arrow it draws: every
/// painted pixel picks a cell of the series within the fringe, nothing picks
/// at distance zero where less than half is covered, and a zero vector's
/// cell picks nowhere — in both scale modes, at 1x and 2x.
#[test]
fn grid_vectors_pick_the_arrow_they_draw() {
    use renderer::data_config::{
        GridLayout, MatrixOrientation, MatrixRef, VectorComponents, VectorPivot, VectorScale,
        VectorStyle,
    };
    let mut renderer = renderer();
    // A 3 x 2 edge grid over x 2..8, y 2..8; column k runs along x.
    let u = [[24.0, -18.0], [0.0, 30.0], [-12.0, 8.0]];
    let v = [[10.0, 20.0], [0.0, -14.0], [16.0, 3.0]];
    let mut columns: Vec<(String, Column<f64>)> = vec![
        ("gx".into(), col(vec![2.0, 4.0, 6.0, 8.0])),
        ("gy".into(), col(vec![2.0, 5.0, 8.0])),
    ];
    for k in 0..3 {
        columns.push((format!("u{k}"), col(u[k].to_vec())));
        columns.push((format!("v{k}"), col(v[k].to_vec())));
    }
    let sources: Vec<(&str, &dyn renderer::ColumnSource)> = columns
        .iter()
        .map(|(id, column)| (id.as_str(), column as &dyn renderer::ColumnSource))
        .collect();
    renderer.add_columns(&sources).expect("upload");
    let matrix = |prefix: &str| MatrixRef {
        columns: (0..3).map(|k| format!("{prefix}{k}")).collect(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Edges,
    };
    let chart = bare_chart();
    for scale_mode in [
        VectorScale::Pixels { px_per_unit: 1.0 },
        VectorScale::Data { factor: 0.06 },
    ] {
        let series = [SeriesConfig {
            series_id: "quiver".into(),
            source_id: None,
            label: None,
            x_column: "gx".into(),
            y_column: "gy".into(),
            render_type: DataRenderType::Vector {
                components: VectorComponents::Grid {
                    u: matrix("u"),
                    v: matrix("v"),
                },
                style: VectorStyle {
                    color: RED,
                    width_px: 3.0,
                    scale: scale_mode.clone(),
                    pivot: VectorPivot::Middle,
                    head_length_px: 8.0,
                    head_width_px: 7.0,
                    min_length_px: 0.0,
                    color_map: None,
                },
            },
        }];
        for scale in [1.0, 2.0] {
            let what = format!("{scale_mode:?} at {scale}x");
            let with = renderer.export_panel_rgba(&chart, &series, scale).unwrap();
            let without = renderer.export_panel_rgba(&chart, &[], scale).unwrap();
            let (x0, y0, x1, y1) = ink_window(&with, &without, 3);
            let probe = Probe::register(&mut renderer, &chart, &series, scale);
            let mut painted = 0usize;
            let mut failures = Vec::new();
            for y in y0..y1 {
                for x in x0..x1 {
                    let ink = difference(&with, &without, x, y);
                    let hit = probe.pick_data(&mut renderer, x, y, STRICT.fringe);
                    let distance = match &hit {
                        Some(renderer::PickedData::MatrixCell { distance_px, .. }) => {
                            Some(*distance_px)
                        }
                        None => None,
                        Some(other) => panic!("{what}: ({x}, {y}) picked {other:?}"),
                    };
                    if ink >= STRICT.painted {
                        painted += 1;
                        if distance.is_none() {
                            failures.push(format!("({x}, {y}) painted (ink {ink}) but not picked"));
                        }
                    }
                    if distance == Some(0.0) && ink < STRICT.zero_ink {
                        failures.push(format!("({x}, {y}) picked at 0 but ink is only {ink}"));
                    }
                }
            }
            assert!(
                failures.is_empty(),
                "{what}: {} disagreements, first {:?}",
                failures.len(),
                &failures[..failures.len().min(12)]
            );
            assert!(painted > 80, "{what}: {painted}");
            // The zero vector of cell (1, 0) draws nothing and picks nothing.
            let (cx, cy) = data_to_px(&chart, 5.0, 3.5, scale);
            assert_eq!(
                probe.pick_data(&mut renderer, cx as u32, cy as u32, 0.0),
                None,
                "{what}: the zero vector's cell"
            );
            renderer.remove_chart(probe.id).unwrap();
        }
    }
}

/// The pixels of `with` that differ from `without` by at least `threshold`,
/// as distances from `center` (export pixels): `(nearest, farthest)`, or
/// `None` when none do.
fn ink_radii(
    with: &RasterImage,
    without: &RasterImage,
    center: (f64, f64),
    threshold: u8,
) -> Option<(f64, f64)> {
    let mut radii: Option<(f64, f64)> = None;
    for y in 0..with.height {
        for x in 0..with.width {
            if difference(with, without, x, y) < threshold {
                continue;
            }
            let r = (f64::from(x) + 0.5 - center.0).hypot(f64::from(y) + 0.5 - center.1);
            radii = Some(radii.map_or((r, r), |(lo, hi)| (lo.min(r), hi.max(r))));
        }
    }
    radii
}

/// A point vector series named "marker" on the x / y columns, 40 px along +x,
/// for the ring test.
fn arrows_ringed() -> SeriesConfig {
    use renderer::data_config::{VectorComponents, VectorPivot, VectorScale, VectorStyle};
    SeriesConfig {
        series_id: "marker".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Vector {
            components: VectorComponents::Points {
                u: "ring_u".into(),
                v: "ring_v".into(),
            },
            style: VectorStyle {
                color: RED,
                width_px: 3.0,
                scale: VectorScale::Pixels { px_per_unit: 1.0 },
                pivot: VectorPivot::Tail,
                head_length_px: 9.0,
                head_width_px: 8.0,
                min_length_px: 0.0,
                color_map: None,
            },
        },
    }
}

/// G2: the selected-point ring is drawn `radius_extra_px` outside the marker
/// the chart draws — every shape, at 1x and 2x, under every draw style, and
/// around a point vector's anchor. Measured on half coverage (ink 128, the
/// edge both the coverage and the picker use), the ring's inner edge lies
/// the extra radius less the ring's half width past the marker's farthest
/// edge, within the pixel grid's discretization. A Milkyway planet and a
/// Constellation star ring their pick envelope (§5.7), which reaches past
/// their faint painted edge, so there only the clearance from every painted
/// pixel holds; a sketch marker's wobble may stay inside its amplitude, so
/// there too.
#[test]
fn picked_ring_clears_the_drawn_marker_by_radius_extra() {
    use renderer::config::{
        ConstellationOptions, DrawStyle, MilkywayOptions, PickedPointRef, PickedPointsConfig,
        SketchOptions,
    };
    let mut renderer = renderer();
    upload_xy(&mut renderer, vec![5.0], vec![5.0]);
    renderer
        .add_columns(&[
            ("ring_u", &col(vec![40.0]) as &dyn renderer::ColumnSource),
            ("ring_v", &col(vec![0.0]) as &dyn renderer::ColumnSource),
        ])
        .expect("upload");
    const EXTRA: f32 = 4.0;
    const WIDTH_PX: f32 = 2.0;
    let ring = |chart: &mut Chart| {
        chart.config_mut().picked_points = Some(PickedPointsConfig {
            visible: true,
            refs: vec![PickedPointRef {
                source_id: None,
                series_id: "marker".into(),
                point_index: 0,
            }],
            ring_color: BLUE,
            ring_width_px: WIDTH_PX,
            radius_extra_px: EXTRA,
        });
    };
    // `geometry`: the marker's coverage edge is its painted edge (else its
    // farthest painted pixel counts); `tight`: the gap has an upper bound.
    let check = |renderer: &mut Renderer,
                 chart: &Chart,
                 series: &[SeriesConfig],
                 scale: f32,
                 (geometry, tight): (bool, bool),
                 what: &str| {
        let mut ringed = Chart::new(chart.config().clone());
        ring(&mut ringed);
        let marker = renderer.export_panel_rgba(chart, series, scale).unwrap();
        let empty = renderer.export_panel_rgba(chart, &[], scale).unwrap();
        let with_ring = renderer.export_panel_rgba(&ringed, series, scale).unwrap();
        let center = data_to_px(chart, 5.0, 5.0, scale);
        const HALF: u8 = 128;
        let marker_far = ink_radii(
            &marker,
            &empty,
            center,
            if geometry { HALF } else { PAINTED },
        )
        .map_or(0.0, |(_, far)| far);
        let (ring_near, _) = ink_radii(&with_ring, &marker, center, HALF).expect("ring ink");
        let s = f64::from(scale);
        let gap = ring_near - marker_far;
        let want = f64::from(EXTRA - WIDTH_PX * 0.5) * s;
        assert!(
            gap >= want - if geometry { 1.5 } else { 2.5 },
            "{what} at {scale}x: ring {ring_near:.2} only {gap:.2} px past the marker {marker_far:.2} (want {want})"
        );
        if tight {
            assert!(
                gap <= want + 1.5,
                "{what} at {scale}x: ring {ring_near:.2} {gap:.2} px past the marker {marker_far:.2} (want {want})"
            );
        }
    };
    for scale in [1.0, 2.0] {
        let chart = bare_chart();
        for shape in ALL_SHAPES {
            let series = [scatter("marker", shape.clone(), 6.0, RED)];
            check(
                &mut renderer,
                &chart,
                &series,
                scale,
                (true, true),
                &format!("{shape:?}"),
            );
        }
        let zero = [scatter("marker", ScatterShape::Circle, 0.0, RED)];
        check(
            &mut renderer,
            &chart,
            &zero,
            scale,
            (true, true),
            "zero-size circle",
        );

        // A point vector's anchor rings at the glyph's half width (the head's
        // 4 px here) at the render scale: the inner edge at 4 + extra - half
        // the ring width.
        let mut ringed = Chart::new(chart.config().clone());
        ring(&mut ringed);
        let vector = [arrows_ringed()];
        let plain = renderer.export_panel_rgba(&chart, &vector, scale).unwrap();
        let with_ring = renderer.export_panel_rgba(&ringed, &vector, scale).unwrap();
        let center = data_to_px(&chart, 5.0, 5.0, scale);
        let (ring_near, ring_far) =
            ink_radii(&with_ring, &plain, center, 128).expect("vector ring ink");
        let s = f64::from(scale);
        let want = f64::from(4.0 + EXTRA - WIDTH_PX * 0.5) * s;
        assert!(
            (ring_near - want).abs() <= 1.5 && ring_far <= want + f64::from(WIDTH_PX) * s + 1.5,
            "point vector ring at {scale}x: {ring_near:.2}..{ring_far:.2}, want inner {want}"
        );
        // A sketch marker's coverage is its geometry (its seam haze is
        // not), but the wobble may stay inside its amplitude; a planet's
        // and a star's envelope reach past their painted edge.
        for (style, geometry) in [
            (
                DrawStyle::Sketch(SketchOptions {
                    amplitude_px: 3.0,
                    wavelength_px: 15.0,
                    seed: 3,
                }),
                true,
            ),
            (DrawStyle::Milkyway(MilkywayOptions::default()), false),
            (
                DrawStyle::Constellation(ConstellationOptions::default()),
                false,
            ),
        ] {
            let chart = styled_chart(style.clone());
            for shape in [ScatterShape::Square, ScatterShape::StarFilled] {
                // Constellation draws scatter-line series only; one point
                // draws no line.
                let marker = scatter("marker", shape.clone(), 6.0, RED);
                let DataRenderType::Scatter {
                    scatter: marker_style,
                } = marker.render_type.clone()
                else {
                    unreachable!()
                };
                let series = [SeriesConfig {
                    render_type: DataRenderType::ScatterLine {
                        scatter: marker_style,
                        line: renderer::DataLineStyleConfig {
                            line_style: renderer::line::LineStylePreset::Solid,
                            line_color: RED,
                            line_width: 1.0,
                            fill: None,
                            fill_color: None,
                            interpolation: renderer::data_config::LineInterpolation::Linear,
                        },
                    },
                    ..marker
                }];
                check(
                    &mut renderer,
                    &chart,
                    &series,
                    scale,
                    (geometry, false),
                    &format!("{style:?} {shape:?}"),
                );
            }
        }
    }
}
