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
/// A pixel picked at distance zero must be at least this far from the
/// series-free export: zero distance means at least half coverage.
const HALF_COVERED: u8 = 96;
/// How far a painted fringe pixel may be from the picked geometry: one
/// anti-aliasing width, `fwidth` of a unit-gradient distance at most √2.
const FRINGE_PX: f32 = 1.5;

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
                .pick(renderer, x, y, FRINGE_PX)
                .filter(|hit| hit.series_id == subject);
            if ink >= PAINTED {
                painted += 1;
                if hit.is_none() {
                    failures.push(format!("({x}, {y}) painted (ink {ink}) but not picked"));
                }
            }
            if let Some(hit) = hit
                && hit.distance_px == 0.0
                && ink < HALF_COVERED
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
