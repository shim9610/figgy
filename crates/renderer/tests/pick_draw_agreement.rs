//! Picks against drawn pixels: every pixel a data primitive paints is
//! pickable at distance zero (or within its anti-aliasing fringe), and no
//! pixel picks at distance zero where nothing is painted.
//!
//! The GPU/CPU parity tests in `gpu_pick/tests.rs` compare the picker with itself.
//! These tests compare it with the renderer's own output instead: each case
//! exports the chart through the public `export_panel_rgba` at 1x and 2x,
//! exports it again without the series under test, and asks the GPU picker
//! (`pick_chart` at the same display scale) about the pixels of a window
//! around the primitive that [`edge_band`] selects: every pixel near a
//! painted edge and a sparse grid elsewhere. A pixel counts as painted when
//! it differs from the series-free export.

#[path = "support/edge_band.rs"]
mod edge_band;
#[path = "support/random_curves.rs"]
mod random_curves;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use edge_band::edge_band;

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

/// How close to a pixel centre a hard edge (a fill's edge, a dash's end,
/// the polar clip) may run for the rasterizer and the picker to put that
/// centre on different sides. Rasterizers snap vertices to subpixel grids
/// and interpolate varyings in f32, so a centre a few ten-thousandths of a
/// pixel from the edge is a tie: lavapipe matches the picker there, WARP
/// and Metal need not (#63). A zero-distance pick on an unpainted pixel is
/// accepted only when moving the cursor this far ends the zero distance.
pub const TIE_PX: f32 = 1.0 / 64.0;

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

/// A one-point circle series of base size `base` whose point 0 carries a
/// sparse size override `size`.
fn scatter_with_size_override(base: f32, size: f32) -> SeriesConfig {
    let mut series = scatter("marker", ScatterShape::Circle, base, RED);
    let DataRenderType::Scatter { scatter } = &mut series.render_type else {
        unreachable!()
    };
    scatter.point_style_overrides =
        Some(vec![renderer::data_config::DataScatterPointStyleOverride {
            index: 0,
            style: renderer::data_config::DataScatterPointStyleConfig {
                point_size: Some(size),
                ..Default::default()
            },
        }]);
    series
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

    /// Whether the zero-distance region of `subject` ends within [`TIE_PX`]
    /// of the centre of export pixel `(x, y)`: a cursor moved that far along
    /// an axis no longer picks `subject` at distance zero.
    pub fn on_zero_edge(&self, renderer: &mut Renderer, x: u32, y: u32, subject: &str) -> bool {
        let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
        [(TIE_PX, 0.0), (-TIE_PX, 0.0), (0.0, TIE_PX), (0.0, -TIE_PX)]
            .into_iter()
            .any(|(dx, dy)| {
                let request = self.request(cx + dx, cy + dy, 0.0);
                pollster::block_on(renderer.pick_chart(self.id, request).unwrap().resolve())
                    .unwrap()
                    .filter(|hit| hit.series_id == subject)
                    .is_none_or(|hit| hit.distance_px > 0.0)
            })
    }

    /// The typed data pick at canvas position `(x, y)`.
    pub fn pick_data_at(
        &self,
        renderer: &mut Renderer,
        (x, y): (f32, f32),
        max: f32,
    ) -> Option<renderer::PickedData> {
        let request = self.request(x, y, max);
        pollster::block_on(
            renderer
                .pick_chart_data(self.id, request)
                .unwrap()
                .resolve(),
        )
        .unwrap()
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
/// `scale`, then checks the [`edge_band`] pixels of `window` (export pixels,
/// inclusive start, exclusive end; `None` takes the painted pixels' bounding
/// box grown by four pixels): a painted pixel must pick `subject` within the
/// fringe, a pixel picked at distance zero must be at least half covered.
/// Returns the number of painted pixels in the window, so a case cannot pass
/// by drawing nothing.
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
    let window = (x0, y0, x1.min(with.width), y1.min(with.height));
    let is_painted = |x, y| difference(&with, &without, x, y) >= tolerance.painted;
    let painted = count_painted(window, is_painted);
    let probe = Probe::register(renderer, chart, series, scale);
    let mut failures = Vec::new();
    for (x, y) in edge_band(window, is_painted) {
        let ink = difference(&with, &without, x, y);
        let hit = probe
            .pick(renderer, x, y, tolerance.fringe)
            .filter(|hit| hit.series_id == subject);
        let skipped = tolerance.skip_near.is_some_and(|(cx, cy, radius)| {
            (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy) <= radius
        });
        if ink >= tolerance.painted && hit.is_none() && !skipped {
            failures.push(format!("({x}, {y}) painted (ink {ink}) but not picked"));
        }
        if let Some(hit) = hit
            && hit.distance_px == 0.0
            && ink < tolerance.zero_ink
            && !probe.on_zero_edge(renderer, x, y, subject)
        {
            failures.push(format!(
                "({x}, {y}) picked at distance 0 (index {}) but ink is only {ink}",
                hit.point_index
            ));
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

/// The number of pixels of `window` for which `painted` holds.
pub fn count_painted(
    (x0, y0, x1, y1): (u32, u32, u32, u32),
    painted: impl Fn(u32, u32) -> bool,
) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| painted(x, y))
        .count()
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

/// G10: a marker drawn at radius zero still paints (an outline is a dot of
/// the stroke's half width, a filled shape its anti-aliased point) and picks
/// where it paints, whichever way the radius reaches zero: the series size,
/// a style-table slot or a size map's `min_px`. Every shape, at 1x and 2x;
/// each case paints at least one pixel and every painted pixel picks the
/// series.
#[test]
fn zero_size_markers_pick_their_drawn_dot() {
    use renderer::data_config::{DataScatterPointStyleConfig, PointSizeMap, SizeScale};
    let mut renderer = renderer();
    upload_xy(&mut renderer, vec![5.0], vec![5.0]);
    renderer
        .add_columns(&[
            ("style", &col(vec![0.0]) as &dyn renderer::ColumnSource),
            ("size", &col(vec![0.0]) as &dyn renderer::ColumnSource),
        ])
        .expect("upload");
    let chart = bare_chart();
    for shape in ALL_SHAPES {
        let size_zero = scatter("dot", shape.clone(), 0.0, RED);
        let mut slot_zero = scatter("dot", shape.clone(), 7.0, RED);
        let mut map_zero = scatter("dot", shape.clone(), 7.0, RED);
        let DataRenderType::Scatter { scatter: slot } = &mut slot_zero.render_type else {
            unreachable!()
        };
        slot.point_style_index_column = Some("style".into());
        slot.point_style_table = Some(vec![DataScatterPointStyleConfig {
            point_color: None,
            point_shape: None,
            point_size: Some(0.0),
        }]);
        let DataRenderType::Scatter { scatter: map } = &mut map_zero.render_type else {
            unreachable!()
        };
        map.point_size_map = Some(PointSizeMap {
            column: "size".into(),
            domain_min: 0.0,
            domain_max: 1.0,
            min_px: 0.0,
            max_px: 9.0,
            scale: SizeScale::Radius,
        });
        for (series, how) in [
            (size_zero, "series size 0"),
            (slot_zero, "style slot radius 0"),
            (map_zero, "size map min_px 0"),
        ] {
            for scale in [1.0, 2.0] {
                let series = [series.clone()];
                let painted = assert_painted_pixels_pick(
                    &mut renderer,
                    &chart,
                    &series,
                    "dot",
                    scale,
                    &format!("{shape:?} {how}"),
                );
                assert!(painted >= 1, "{shape:?} {how} at {scale}x painted nothing");
            }
        }
    }
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

/// Far along the arc (#107, SHADER_COMMON.md §5.6 arc pairs): the line first
/// zig-zags 301 times off-screen, about 1e8 px of arc, so its visible
/// segments start past 2^26 px, where an f32 arc lands on multiples of 8 px.
/// The dash phase and the sketch wobble the picker derives from the arc
/// prefix are still the drawn ones, precise and sketched, at 1x and 2x. The
/// off-screen points stay within 2e5 px of the view, so the pick's f32 pixel
/// geometry is exact there. 301 points put the dash phase off the few phases
/// where a sketched dash paints a sub-pixel sliver at a joint that the
/// picker misses whatever the arc's length (#114).
/// Random composite lines (#114): every case draws a seeded random
/// composition of elementary functions (`support/random_curves.rs`): a graph
/// or a parametric curve, sampled unevenly, sometimes with a NaN gap and
/// sometimes after a long off-screen lead that puts it far along the arc. A
/// random draw style (precise or sketched with random wobble), dash preset,
/// width, step mode and export scale complete it, and every painted pixel
/// must pick and every zero-distance pick must be painted. A failure names
/// the seed; `FIGGY_RANDOM_SEED=<seed>` replays that case alone and
/// `FIGGY_RANDOM_CASES=<n>` runs more.
#[test]
fn random_composite_lines_pick_where_they_draw() {
    use random_curves::{Curve, Rng};
    use renderer::config::{DrawStyle, SketchOptions};
    use renderer::data_config::LineInterpolation;
    use renderer::line::LineStylePreset;
    const PRESETS: [LineStylePreset; 11] = [
        LineStylePreset::Solid,
        LineStylePreset::Dash,
        LineStylePreset::Dot,
        LineStylePreset::DashDot,
        LineStylePreset::DashDotDot,
        LineStylePreset::ShortDash,
        LineStylePreset::ShortDot,
        LineStylePreset::ShortDashDot,
        LineStylePreset::LongDash,
        LineStylePreset::LongDashDot,
        LineStylePreset::LongDashDotDot,
    ];
    const STEPS: [LineInterpolation; 4] = [
        LineInterpolation::Linear,
        LineInterpolation::StepPre,
        LineInterpolation::StepPost,
        LineInterpolation::StepMid,
    ];
    let seeds: Vec<u64> = match std::env::var("FIGGY_RANDOM_SEED") {
        Ok(seed) => vec![seed.parse().expect("FIGGY_RANDOM_SEED is a u64")],
        Err(_) => {
            let cases = std::env::var("FIGGY_RANDOM_CASES")
                .map_or(12, |n| n.parse().expect("FIGGY_RANDOM_CASES is a count"));
            (0..cases).map(|case| 0x5EED_0000 + case).collect()
        }
    };
    let mut failures = Vec::new();
    for seed in seeds {
        let mut rng = Rng::new(seed.rotate_left(17));
        let lead = [0, 0, 2, 301, 601][rng.index(5)];
        let curve = Curve::random(seed, lead);
        let style = if rng.chance(0.5) {
            DrawStyle::Precise
        } else {
            DrawStyle::Sketch(SketchOptions {
                amplitude_px: rng.range(0.0, 5.0) as f32,
                wavelength_px: rng.range(6.0, 30.0) as f32,
                seed: rng.next_u64() as u32,
            })
        };
        let preset = PRESETS[rng.index(PRESETS.len())];
        let width = rng.range(0.3, 5.0) as f32;
        let step = STEPS[rng.index(STEPS.len())];
        let scale = if rng.chance(0.5) { 1.0 } else { 2.0 };
        let what = format!(
            "{}; {style:?}, {preset:?}, width {width:.2}, {step:?}",
            curve.what
        );
        let mut renderer = renderer();
        upload_xy(&mut renderer, curve.xs, curve.ys);
        let chart = styled_chart(style);
        let series = [line("random", width, preset, step)];
        let tolerance = if matches!(preset, LineStylePreset::Solid) {
            STRICT
        } else {
            MSAA_EDGE
        };
        // Inside the frame: a lead enters across the plot edge, where the
        // frame's own ink covers the stroke.
        let window = Some((
            (4.0 * scale) as u32,
            0,
            (WIDTH as f32 * scale) as u32,
            (HEIGHT as f32 * scale) as u32,
        ));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_pick_matches_paint_with(
                &mut renderer,
                &chart,
                &series,
                "random",
                scale,
                window,
                &what,
                tolerance,
            )
        }));
        if let Err(panic) = outcome {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "non-string panic".into());
            failures.push(message);
        }
    }
    assert!(
        failures.is_empty(),
        "{} random cases disagree:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Random composite curves under the marker and star styles (#114): the
/// same seeded curves as `random_composite_lines_pick_where_they_draw`, drawn
/// as a precise scatter-line with a random marker shape and size (every
/// painted pixel picks and every zero-distance pick is painted), or as a
/// Milkyway ribbon or a Constellation chain, whose hashed stars pick as their
/// envelope (every painted pixel picks). A failure names the seed;
/// `FIGGY_RANDOM_SEED` and `FIGGY_RANDOM_CASES` work as there.
#[test]
fn random_composite_markers_and_star_lines_pick_where_they_draw() {
    use random_curves::{Curve, Rng};
    use renderer::config::{ConstellationOptions, DrawStyle, MilkywayOptions};
    use renderer::data_config::LineInterpolation;
    use renderer::line::LineStylePreset;
    let seeds: Vec<u64> = match std::env::var("FIGGY_RANDOM_SEED") {
        Ok(seed) => vec![seed.parse().expect("FIGGY_RANDOM_SEED is a u64")],
        Err(_) => {
            let cases = std::env::var("FIGGY_RANDOM_CASES")
                .map_or(9, |n| n.parse().expect("FIGGY_RANDOM_CASES is a count"));
            (0..cases).map(|case| 0x57A2_0000 + case).collect()
        }
    };
    let mut failures = Vec::new();
    for seed in seeds {
        let mut rng = Rng::new(seed.rotate_left(29));
        let lead = [0, 0, 2, 301][rng.index(4)];
        let curve = Curve::random(seed, lead);
        let scale = if rng.chance(0.5) { 1.0 } else { 2.0 };
        let kind = seed % 3;
        let shape = ALL_SHAPES[rng.index(ALL_SHAPES.len())].clone();
        let size = rng.range(2.0, 12.0) as f32;
        let width = rng.range(0.5, 3.0) as f32;
        let scatter_line = SeriesConfig {
            render_type: DataRenderType::ScatterLine {
                scatter: DataScatterStyleConfig {
                    point_color: BLUE,
                    point_shape: shape.clone(),
                    point_size: size,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
                line: renderer::DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: RED,
                    line_width: width,
                    fill: None,
                    fill_color: None,
                    interpolation: LineInterpolation::Linear,
                },
            },
            ..scatter("random", shape.clone(), size, BLUE)
        };
        let (chart, series, what) = match kind {
            0 => (
                bare_chart(),
                scatter_line,
                format!("precise scatter-line {shape:?} size {size:.1} width {width:.2}"),
            ),
            1 => (
                styled_chart(DrawStyle::Milkyway(MilkywayOptions::default())),
                line(
                    "random",
                    width,
                    LineStylePreset::Solid,
                    LineInterpolation::Linear,
                ),
                "milkyway ribbon".to_string(),
            ),
            _ => (
                styled_chart(DrawStyle::Constellation(ConstellationOptions::default())),
                scatter_line,
                format!("constellation chain size {size:.1} width {width:.2}"),
            ),
        };
        let what = format!("{}; {what} at {scale}x", curve.what);
        let mut renderer = renderer();
        upload_xy(&mut renderer, curve.xs, curve.ys);
        let series = [series];
        let window = Some((
            (4.0 * scale) as u32,
            0,
            (WIDTH as f32 * scale) as u32,
            (HEIGHT as f32 * scale) as u32,
        ));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if kind == 0 {
                assert_pick_matches_paint_with(
                    &mut renderer,
                    &chart,
                    &series,
                    "random",
                    scale,
                    window,
                    &what,
                    STRICT,
                );
            } else {
                assert_painted_pixels_pick(&mut renderer, &chart, &series, "random", scale, &what);
            }
        }));
        if let Err(panic) = outcome {
            failures.push(
                panic
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_else(|| "non-string panic".into()),
            );
        }
    }
    assert!(
        failures.is_empty(),
        "{} random cases disagree:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn far_along_the_arc_dashes_and_wobble_pick_where_they_draw() {
    use renderer::config::{DrawStyle, SketchOptions};
    use renderer::data_config::LineInterpolation;
    use renderer::line::LineStylePreset;
    let mut renderer = renderer();
    let far = 301;
    let xs: Vec<f64> = std::iter::repeat_n(-1.0, far)
        .chain([-0.5, 4.0, 4.9, 5.4, 6.1, 7.0])
        .collect();
    let ys: Vec<f64> = (0..far)
        .map(|i| if i % 2 == 0 { 1.0e4 } else { -1.0e4 })
        .chain([5.0, 4.2, 5.6, 4.6, 5.9, 5.2])
        .collect();
    upload_xy(&mut renderer, xs, ys);
    let precise = bare_chart();
    let sketch = styled_chart(DrawStyle::Sketch(SketchOptions {
        amplitude_px: 4.0,
        wavelength_px: 12.0,
        seed: 7,
    }));
    for (what, chart) in [("precise", &precise), ("sketch", &sketch)] {
        for style in [LineStylePreset::Dash, LineStylePreset::DashDotDot] {
            for scale in [1.0, 2.0] {
                let series = [line("far", 2.0, style, LineInterpolation::Linear)];
                let painted = assert_pick_matches_paint_with(
                    &mut renderer,
                    chart,
                    &series,
                    "far",
                    scale,
                    // Inside the frame: the line enters across the plot
                    // edge, where the frame's own ink covers the stroke.
                    Some((
                        (4.0 * scale) as u32,
                        0,
                        (WIDTH as f32 * scale) as u32,
                        (HEIGHT as f32 * scale) as u32,
                    )),
                    &format!("{what} {style:?} far along the arc"),
                    MSAA_EDGE,
                );
                assert!(painted > 30, "{what} {style:?} at {scale}x: {painted}");
            }
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

#[cfg(feature = "cpu-twin")]
struct F32Columns(Vec<(String, Vec<f32>)>);

#[cfg(feature = "cpu-twin")]
impl renderer::pick::cpu_twin::PointColumnLookup for F32Columns {
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
#[cfg(feature = "cpu-twin")]
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
                    let cpu = renderer::pick::cpu_twin::pick_nearest_point(
                        chart.config(),
                        &series,
                        &columns,
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                        renderer::pick::cpu_twin::PointPickOptions {
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

#[cfg(feature = "cpu-twin")]
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
#[cfg(feature = "cpu-twin")]
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
                let cpu = renderer::pick::cpu_twin::pick_nearest_point(
                    chart.config(),
                    &series,
                    &columns,
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    renderer::pick::cpu_twin::PointPickOptions {
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
            let window = ink_window(&with, &without, 3);
            let is_painted = |x, y| difference(&with, &without, x, y) >= STRICT.painted;
            let painted = count_painted(window, is_painted);
            let probe = Probe::register(&mut renderer, &chart, &series, scale);
            let mut failures = Vec::new();
            for (x, y) in edge_band(window, is_painted) {
                let ink = difference(&with, &without, x, y);
                let hit = probe.pick_data(&mut renderer, x, y, STRICT.fringe);
                let distance = match &hit {
                    Some(renderer::PickedData::MatrixCell { distance_px, .. }) => {
                        Some(*distance_px)
                    }
                    None => None,
                    Some(other) => panic!("{what}: ({x}, {y}) picked {other:?}"),
                };
                if ink >= STRICT.painted && distance.is_none() {
                    failures.push(format!("({x}, {y}) painted (ink {ink}) but not picked"));
                }
                if distance == Some(0.0) && ink < STRICT.zero_ink {
                    failures.push(format!("({x}, {y}) picked at 0 but ink is only {ink}"));
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
    // farthest painted pixel counts); `slack`: the gap's upper bound past
    // the wanted one, if any.
    let check = |renderer: &mut Renderer,
                 chart: &Chart,
                 series: &[SeriesConfig],
                 scale: f32,
                 (geometry, slack): (bool, Option<f64>),
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
        if let Some(slack) = slack {
            assert!(
                gap <= want + slack,
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
                (true, Some(1.5)),
                &format!("{shape:?}"),
            );
        }
        let zero = [scatter("marker", ScatterShape::Circle, 0.0, RED)];
        check(
            &mut renderer,
            &chart,
            &zero,
            scale,
            (true, Some(1.5)),
            "zero-size circle",
        );
        // A size override: Precise draws it through the style map, so the
        // ring takes the mapped entry and rings the override's radius.
        let overridden = [scatter_with_size_override(4.0, 12.0)];
        check(
            &mut renderer,
            &chart,
            &overridden,
            scale,
            (true, Some(1.5)),
            "size override under Precise",
        );
        // Sketch ignores the overrides and draws the base radius, so the
        // unmapped ring rings that radius: a ring at the override's 12 px
        // would sit 8 px too far, past the wobble's 3 px amplitude.
        const SKETCH_AMPLITUDE_PX: f32 = 3.0;
        let sketch = styled_chart(DrawStyle::Sketch(SketchOptions {
            amplitude_px: SKETCH_AMPLITUDE_PX,
            wavelength_px: 15.0,
            seed: 3,
        }));
        check(
            &mut renderer,
            &sketch,
            &overridden,
            scale,
            (true, Some(f64::from(SKETCH_AMPLITUDE_PX * scale) + 1.5)),
            "size override under Sketch",
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
                    (geometry, None),
                    &format!("{style:?} {shape:?}"),
                );
            }
        }
    }
}

/// G7: a histogram picks what it paints. Full bins pick their bar (gap,
/// border and a nonzero baseline included); subpixel bins pick the pixel
/// columns the envelope pass paints, from the baseline to the column's
/// winner, and the winner is the bin picked there. Vertical and horizontal,
/// at 1x and 2x.
#[test]
fn histogram_bins_pick_what_the_bars_and_the_envelope_paint() {
    use renderer::data_config::{BarOrientation, DataBarStyleConfig};
    let mut renderer = renderer();
    // Three wide bins over 1..4, then 120 subpixel bins over 5..6, all
    // above the baseline 1.5.
    let mut edges = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let mut values = vec![4.0, 7.5, 3.0, 2.0];
    for i in 0..120 {
        edges.push(5.0 + f64::from(i + 1) / 120.0);
        values.push(2.0 + 6.0 * (f64::from(i) * 0.37).sin().abs());
    }
    renderer
        .add_columns(&[
            ("edges", &col(edges) as &dyn renderer::ColumnSource),
            ("counts", &col(values) as &dyn renderer::ColumnSource),
        ])
        .expect("upload");
    let chart = bare_chart();
    for orientation in [BarOrientation::Vertical, BarOrientation::Horizontal] {
        let (x, y) = match orientation {
            BarOrientation::Vertical => ("edges", "counts"),
            BarOrientation::Horizontal => ("counts", "edges"),
        };
        let series = [SeriesConfig {
            series_id: "hist".into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Histogram {
                bar: DataBarStyleConfig {
                    fill_color: RED,
                    border_color: BLUE,
                    border_width: 1.0,
                    baseline: 1.5,
                    gap_px: 2.0,
                    width_ratio: 1.0,
                    orientation: orientation.clone(),
                    bar_style_overrides: None,
                },
            },
        }];
        for scale in [1.0, 2.0] {
            let what = format!("{orientation:?} at {scale}x");
            let with = renderer.export_panel_rgba(&chart, &series, scale).unwrap();
            let without = renderer.export_panel_rgba(&chart, &[], scale).unwrap();
            let window = ink_window(&with, &without, 2);
            let is_painted = |x, y| difference(&with, &without, x, y) >= PAINTED;
            let painted = count_painted(window, is_painted);
            let probe = Probe::register(&mut renderer, &chart, &series, scale);
            let mut failures = Vec::new();
            for (px, py) in edge_band(window, is_painted) {
                let ink = difference(&with, &without, px, py);
                let hit = probe.pick_data(&mut renderer, px, py, 0.0);
                let bin = match &hit {
                    Some(renderer::PickedData::HistogramBin { bin_index, .. }) => Some(*bin_index),
                    None => None,
                    Some(other) => panic!("{what}: ({px}, {py}) picked {other:?}"),
                };
                // A bar is axis-aligned, so painted and picked agree pixel
                // for pixel: more than half the samples covered picks, fewer
                // does not. An edge through the pixel's centre covers
                // exactly half and may go either way.
                if ink > 128 && bin.is_none() {
                    failures.push(format!("({px}, {py}) painted (ink {ink}) but not picked"));
                }
                if ink < 128 && bin.is_some() {
                    failures.push(format!("({px}, {py}) picks {bin:?} but ink is only {ink}"));
                }
            }
            assert!(
                failures.is_empty(),
                "{what}: {} disagreements, first {:?}",
                failures.len(),
                &failures[..failures.len().min(12)]
            );
            assert!(painted > 400, "{what}: {painted}");
            renderer.remove_chart(probe.id).unwrap();
        }
    }
}

/// G8 / G14: a heatmap cell picks exactly the pixels it paints. The fill
/// shades each pixel at its centre, so the pick locates the cell at the
/// cursor's pixel centre, and a pixel whose painted colour is clear (a
/// transparent ramp end, a transparent `nan_color`) picks nothing. Flat and
/// interpolated shading, at 1x and 2x.
#[test]
fn heatmap_cells_pick_exactly_the_pixels_they_paint() {
    use renderer::data_config::{
        FieldFillConfig, FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
    };
    let mut renderer = renderer();
    // x edges 1.3..7.9, y edges 1.7..8.2: cell boundaries fall inside pixels.
    renderer
        .add_columns(&[
            (
                "hx",
                &col(vec![1.3, 3.55, 5.1, 7.9]) as &dyn renderer::ColumnSource,
            ),
            (
                "hy",
                &col(vec![1.7, 4.45, 6.0, 8.2]) as &dyn renderer::ColumnSource,
            ),
            (
                "cx",
                &col(vec![2.3, 4.55, 7.1]) as &dyn renderer::ColumnSource,
            ),
            (
                "cy",
                &col(vec![2.7, 5.45, 7.2]) as &dyn renderer::ColumnSource,
            ),
            // z_min (transparent ramp end), NaN (transparent nan_color) and
            // opaque values.
            (
                "h0",
                &col(vec![0.0, 5.0, 10.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "h1",
                &col(vec![7.0, 4.0, 3.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "h2",
                &col(vec![10.0, 0.0, f64::NAN]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
    let mut chart = bare_chart();
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = renderer::ColorMap::Custom {
        stops: vec![
            Color {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 0.0,
            },
            RED,
        ],
    };
    bar.nan_color = Color {
        r: 0.0,
        g: 1.0,
        b: 0.0,
        a: 0.0,
    };
    bar.axis.min = 0.0;
    bar.axis.max = 10.0;
    bar.axis.major_spacing = 2.0;
    bar.visible = false;
    chart.config_mut().colorbar = Some(bar);
    for shading in [Shading::Flat, Shading::Interpolated] {
        let series = [SeriesConfig {
            series_id: "field".into(),
            source_id: None,
            label: None,
            x_column: match shading {
                Shading::Flat => "hx".into(),
                _ => "cx".into(),
            },
            y_column: match shading {
                Shading::Flat => "hy".into(),
                _ => "cy".into(),
            },
            render_type: DataRenderType::Heatmap {
                matrix: MatrixRef {
                    columns: vec!["h0".into(), "h1".into(), "h2".into()],
                    orientation: MatrixOrientation::ColumnsAreX,
                    grid_layout: match shading {
                        Shading::Flat => GridLayout::Edges,
                        _ => GridLayout::Centers,
                    },
                },
                fill: FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: shading.clone(),
                    opacity: 1.0,
                },
            },
        }];
        for scale in [1.0, 2.0] {
            let what = format!("{shading:?} at {scale}x");
            let with = renderer.export_panel_rgba(&chart, &series, scale).unwrap();
            let without = renderer.export_panel_rgba(&chart, &[], scale).unwrap();
            let window = ink_window(&with, &without, 2);
            let is_painted = |x, y| difference(&with, &without, x, y) >= PAINTED;
            let painted = count_painted(window, is_painted);
            let (x0, y0, x1, y1) = window;
            let clear_inside = ((x1 - x0) * (y1 - y0)) as usize - painted;
            let probe = Probe::register(&mut renderer, &chart, &series, scale);
            let mut failures = Vec::new();
            for (px, py) in edge_band(window, is_painted) {
                let ink = difference(&with, &without, px, py);
                // Off the pixel centre, inside the pixel: the pick must
                // still answer the cell painted at the centre.
                let request = (px as f32 + 0.9, py as f32 + 0.1);
                let hit = probe.pick_data_at(&mut renderer, request, 0.0);
                let cell = match &hit {
                    Some(renderer::PickedData::MatrixCell { .. }) => true,
                    None => false,
                    Some(other) => panic!("{what}: ({px}, {py}) picked {other:?}"),
                };
                // A painted pixel picks; a pixel left exactly as it was
                // does not. (A ramp near its transparent end paints a few
                // levels of ink, and those pick too.)
                if (ink >= PAINTED && !cell) || (ink == 0 && cell) {
                    failures.push(format!("({px}, {py}) ink {ink} picked {hit:?}"));
                }
            }
            assert!(
                failures.is_empty(),
                "{what}: {} disagreements, first {:?}",
                failures.len(),
                &failures[..failures.len().min(12)]
            );
            // Counted over every pixel of the window (the scan once probed
            // every second pixel of every second row: a quarter of these).
            assert!(
                painted > 800 && clear_inside > 200,
                "{what}: {painted} / {clear_inside}"
            );
            renderer.remove_chart(probe.id).unwrap();
        }
    }
}

/// G9: a labelled contour leaves a gap under each label, and the gap is not
/// picked and not highlighted: the pick and the selected-level highlight run
/// the draw's own gap test on the draw's own label placement. Elsewhere the
/// stroke picks its level. At 1x and 2x.
#[test]
fn contour_label_gaps_pick_and_highlight_nothing() {
    use renderer::data_config::{
        ContourConfig, ContourLabelConfig, DataLineStyleConfig, GridLayout, MatrixOrientation,
        MatrixRef,
    };
    let mut renderer = renderer();
    // z = x + y on a 3 x 3 grid over the chart: straight diagonal isolines.
    renderer
        .add_columns(&[
            (
                "cgx",
                &col(vec![0.0, 5.0, 10.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "cgy",
                &col(vec![0.0, 5.0, 10.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "c0",
                &col(vec![0.0, 5.0, 10.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "c1",
                &col(vec![5.0, 10.0, 15.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "c2",
                &col(vec![10.0, 15.0, 20.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
    let contour = |labels: bool| SeriesConfig {
        series_id: "iso".into(),
        source_id: None,
        label: None,
        x_column: "cgx".into(),
        y_column: "cgy".into(),
        render_type: DataRenderType::Contour {
            matrix: MatrixRef {
                columns: vec!["c0".into(), "c1".into(), "c2".into()],
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            contour: ContourConfig {
                levels: vec![6.0, 10.0, 14.0],
                line: DataLineStyleConfig {
                    line_style: renderer::line::LineStylePreset::Solid,
                    line_color: RED,
                    line_width: 3.0,
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                },
                per_level_color: None,
                labels: labels.then(|| ContourLabelConfig {
                    visible: true,
                    font_size: 13.0,
                    color: BLUE,
                    format: renderer::format::LabelFormat::Decimal,
                    significant_digits: 2,
                    spacing_px: 70.0,
                    anchors: Vec::new(),
                    bg_color: None,
                    bg_padding_px: 3.0,
                }),
            },
        },
    };
    let mut chart = bare_chart();
    let mut bar = renderer::default::default_colorbar_options();
    bar.visible = false;
    bar.axis.min = 0.0;
    bar.axis.max = 20.0;
    bar.axis.major_spacing = 5.0;
    chart.config_mut().colorbar = Some(bar);
    // Red stroke ink (the labels are blue).
    let red = |img: &RasterImage, x: u32, y: u32| {
        let [r, g, b, a] = pixel(img, x, y);
        let alpha = u32::from(a);
        let premul = |c: u8| u32::from(c) * alpha / 255;
        (premul(r), premul(g).max(premul(b)))
    };
    for scale in [1.0, 2.0] {
        let labelled = [contour(true)];
        let with = renderer
            .export_panel_rgba(&chart, &labelled, scale)
            .unwrap();
        let plain = renderer
            .export_panel_rgba(&chart, &[contour(false)], scale)
            .unwrap();
        let probe = Probe::register(&mut renderer, &chart, &labelled, scale);
        let mut gap = 0usize;
        let mut stroke = 0usize;
        let mut failures = Vec::new();
        let mut gap_pixels = Vec::new();
        for y in 0..with.height {
            for x in 0..with.width {
                let (labelled_red, labelled_other) = red(&with, x, y);
                let (plain_red, _) = red(&plain, x, y);
                let in_gap = plain_red >= 200 && labelled_red < 6 && labelled_other < 6;
                let on_stroke = labelled_red >= 200;
                if !(in_gap || on_stroke) {
                    continue;
                }
                let hit = probe.pick_data(&mut renderer, x, y, 0.0);
                let level = matches!(hit, Some(renderer::PickedData::ContourLevel { .. }));
                if in_gap {
                    gap += 1;
                    gap_pixels.push((x, y));
                    if level {
                        failures.push(format!("({x}, {y}) in a label gap picks {hit:?}"));
                    }
                } else if !level {
                    stroke += 1;
                    failures.push(format!("({x}, {y}) on the stroke picks {hit:?}"));
                } else {
                    stroke += 1;
                }
            }
        }
        renderer.remove_chart(probe.id).unwrap();
        assert!(
            failures.is_empty(),
            "at {scale}x: {} disagreements, first {:?}",
            failures.len(),
            &failures[..failures.len().min(12)]
        );
        assert!(
            gap > 10 && stroke > 100,
            "at {scale}x: {gap} gap / {stroke} stroke"
        );

        // The selected level's highlight skips the same gaps.
        let mut selected = Chart::new(chart.config().clone());
        selected.config_mut().picked_data = Some(renderer::DataSelectionsConfig {
            visible: true,
            refs: vec![renderer::PickedDataRef::ContourLevel {
                source_id: None,
                series_id: "iso".into(),
                level_index: 1,
                x_index: 0,
                y_index: 0,
            }],
            highlight_color: Color {
                r: 0.0,
                g: 1.0,
                b: 0.0,
                a: 1.0,
            },
            outline_width_px: 2.0,
            point_radius_extra_px: 3.0,
            contour_width_extra_px: 2.0,
        });
        let highlighted = renderer
            .export_panel_rgba(&selected, &labelled, scale)
            .unwrap();
        let green = |img: &RasterImage, x: u32, y: u32| {
            let [r, g, b, _] = pixel(img, x, y);
            g >= 200 && r < 80 && b < 80
        };
        let in_gap = gap_pixels
            .iter()
            .filter(|&&(x, y)| green(&highlighted, x, y))
            .count();
        let total = (0..highlighted.height)
            .flat_map(|y| (0..highlighted.width).map(move |x| (x, y)))
            .filter(|&(x, y)| green(&highlighted, x, y))
            .count();
        assert!(total > 50, "at {scale}x: {total} highlight pixels");
        assert_eq!(
            in_gap, 0,
            "at {scale}x: the highlight paints {in_gap} gap pixels"
        );
    }
}

/// G14: a contour stroke picks within its anti-aliasing fringe wherever it
/// paints (coverage `half_width - d + 0.5` reaches half a pixel past the
/// half width, where the pick distance is under half a pixel), and picks at
/// distance zero only where it covers at least half the pixel. At 1x and 2x.
#[test]
fn contour_strokes_pick_where_they_paint() {
    use renderer::data_config::{
        ContourConfig, DataLineStyleConfig, GridLayout, MatrixOrientation, MatrixRef,
    };
    let mut renderer = renderer();
    renderer
        .add_columns(&[
            (
                "sgx",
                &col(vec![0.0, 5.0, 10.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "sgy",
                &col(vec![0.0, 5.0, 10.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "s0",
                &col(vec![0.0, 4.0, 9.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "s1",
                &col(vec![6.0, 11.0, 15.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "s2",
                &col(vec![10.0, 16.0, 20.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
    let series = [SeriesConfig {
        series_id: "iso".into(),
        source_id: None,
        label: None,
        x_column: "sgx".into(),
        y_column: "sgy".into(),
        render_type: DataRenderType::Contour {
            matrix: MatrixRef {
                columns: vec!["s0".into(), "s1".into(), "s2".into()],
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            contour: ContourConfig {
                levels: vec![7.0, 12.5],
                line: DataLineStyleConfig {
                    line_style: renderer::line::LineStylePreset::Solid,
                    line_color: RED,
                    line_width: 2.5,
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                },
                per_level_color: None,
                labels: None,
            },
        },
    }];
    let mut chart = bare_chart();
    let mut bar = renderer::default::default_colorbar_options();
    bar.visible = false;
    bar.axis.min = 0.0;
    bar.axis.max = 20.0;
    bar.axis.major_spacing = 5.0;
    chart.config_mut().colorbar = Some(bar);
    for scale in [1.0, 2.0] {
        let with = renderer.export_panel_rgba(&chart, &series, scale).unwrap();
        let without = renderer.export_panel_rgba(&chart, &[], scale).unwrap();
        // The frame draws over the data area's outer pixels.
        let border = (2.0 * scale) as u32;
        let (x0, y0, x1, y1) = ink_window(&with, &without, 2);
        let (x0, y0) = (x0.max(border), y0.max(border));
        let (x1, y1) = (x1.min(with.width - border), y1.min(with.height - border));
        let window = (x0, y0, x1, y1);
        let is_painted = |x, y| difference(&with, &without, x, y) >= PAINTED;
        let painted = count_painted(window, is_painted);
        let probe = Probe::register(&mut renderer, &chart, &series, scale);
        let mut failures = Vec::new();
        for (x, y) in edge_band(window, is_painted) {
            let ink = difference(&with, &without, x, y);
            let hit = probe.pick_data(&mut renderer, x, y, STRICT.fringe);
            let distance = match &hit {
                Some(renderer::PickedData::ContourLevel { distance_px, .. }) => Some(*distance_px),
                None => None,
                Some(other) => panic!("({x}, {y}) picked {other:?}"),
            };
            if ink >= PAINTED && distance.is_none() {
                failures.push(format!("({x}, {y}) painted (ink {ink}) but not picked"));
            }
            if distance == Some(0.0) && ink < STRICT.zero_ink {
                failures.push(format!("({x}, {y}) picked at 0 but ink is only {ink}"));
            }
        }
        renderer.remove_chart(probe.id).unwrap();
        assert!(
            failures.is_empty(),
            "at {scale}x: {} disagreements, first {:?}",
            failures.len(),
            &failures[..failures.len().min(12)]
        );
        // Counted over every pixel of the window (the scan once probed
        // every second pixel of every second row: a quarter of these).
        assert!(painted > 200, "at {scale}x: {painted}");
    }
}

/// A data value's export pixel along one axis, in f64, linear or
/// logarithmic: the reference both the CPU and the GPU must hit.
#[cfg(feature = "cpu-twin")]
fn axis_px(axis: &renderer::config::AxisOptions, value: f64, start: f64, length: f64) -> f64 {
    let t = match axis.scale {
        renderer::config::AxisScale::Linear => (value - axis.min) / (axis.max - axis.min),
        renderer::config::AxisScale::Logarithmic => {
            (value.log10() - axis.min.log10()) / (axis.max.log10() - axis.min.log10())
        }
    };
    start + t * length
}

/// The CPU (`pick_nearest_point`) and the GPU (the draw and `pick_chart`)
/// place every point where its value lies, within a pixel of each other and
/// of the f64 reference, on linear and logarithmic axes at every magnitude.
/// Past 2^24 the GPU once lost the logarithm (a log flag blended with
/// `mix`) while the CPU kept it.
#[cfg(feature = "cpu-twin")]
#[test]
fn cpu_and_gpu_place_points_alike_at_every_magnitude() {
    use renderer::config::AxisScale;
    let magnitudes = [
        1e-20, 1e-6, 1.0, 1e6, 4e6, 1.6e7, 1.7e7, 3e7, 1e8, 1e12, 1e16, 1e30,
    ];
    let fractions = [0.2, 0.5, 0.8];
    let mut checked = 0;
    for (x_scale, y_scale) in [
        (AxisScale::Linear, AxisScale::Linear),
        (AxisScale::Linear, AxisScale::Logarithmic),
        (AxisScale::Logarithmic, AxisScale::Logarithmic),
    ] {
        for magnitude in magnitudes {
            // A range around `magnitude`: two decades either side on a log
            // axis, one magnitude wide on a linear one.
            let range = |scale: &AxisScale| match scale {
                AxisScale::Linear => (magnitude, 2.0 * magnitude),
                AxisScale::Logarithmic => (magnitude / 100.0, magnitude * 100.0),
            };
            let value = |scale: &AxisScale, f: f64| {
                let (lo, hi) = range(scale);
                match scale {
                    AxisScale::Linear => lo + f * (hi - lo),
                    AxisScale::Logarithmic => lo * (hi / lo).powf(f),
                }
            };
            let xs: Vec<f64> = fractions.iter().map(|f| value(&x_scale, *f)).collect();
            let ys: Vec<f64> = fractions
                .iter()
                .map(|f| value(&y_scale, 1.0 - *f))
                .collect();
            let mut renderer = renderer();
            upload_xy(&mut renderer, xs.clone(), ys.clone());
            let mut chart = bare_chart();
            chart.config_mut().bottom_x.scale = x_scale.clone();
            chart.config_mut().left_y.scale = y_scale.clone();
            let (x0, x1) = range(&x_scale);
            let (y0, y1) = range(&y_scale);
            chart.set_x_range(x0, x1);
            chart.set_y_range(y0, y1);
            let series = [scatter("points", ScatterShape::SquareFilled, 7.0, RED)];
            let image = renderer
                .export_panel_rgba(&chart, &series, 1.0)
                .expect("export");
            let probe = Probe::register(&mut renderer, &chart, &series, 1.0);
            let columns = F32Columns(vec![
                ("x".into(), xs.iter().map(|v| *v as f32).collect()),
                ("y".into(), ys.iter().map(|v| *v as f32).collect()),
            ]);
            let cfg = chart.config().clone();
            let da = cfg.data_area().expect("data area").0;
            let case = format!("{x_scale:?}/{y_scale:?} at {magnitude:e}");
            for (index, (&x, &y)) in xs.iter().zip(&ys).enumerate() {
                let expected = (
                    axis_px(&cfg.bottom_x, x, da.x as f64, da.width as f64),
                    axis_px(
                        &cfg.left_y,
                        y,
                        (da.y + da.height) as f64,
                        -(da.height as f64),
                    ),
                );
                // The draw: the marker's red ink is centred on the value.
                let (mut sum_x, mut sum_y, mut ink) = (0.0, 0.0, 0.0);
                for py in
                    (expected.1 as i64 - 12).max(0)..(expected.1 as i64 + 12).min(HEIGHT as i64)
                {
                    for px in
                        (expected.0 as i64 - 12).max(0)..(expected.0 as i64 + 12).min(WIDTH as i64)
                    {
                        let i = ((py as u32 * image.width + px as u32) * 4) as usize;
                        let [r, g, b, a] = image.rgba[i..i + 4].try_into().unwrap();
                        if r > 128 && g < 100 && b < 100 && a > 0 {
                            let w = f64::from(r);
                            sum_x += (px as f64 + 0.5) * w;
                            sum_y += (py as f64 + 0.5) * w;
                            ink += w;
                        }
                    }
                }
                assert!(
                    ink > 0.0,
                    "{case}: point {index} is not drawn near {expected:?}"
                );
                let drawn = (sum_x / ink, sum_y / ink);
                assert!(
                    (drawn.0 - expected.0).abs() <= 1.0 && (drawn.1 - expected.1).abs() <= 1.0,
                    "{case}: point {index} drawn at {drawn:?}, expected {expected:?}"
                );
                // The GPU and the CPU pickers find it there.
                let at = (expected.0 as f32, expected.1 as f32);
                let gpu = pollster::block_on(
                    renderer
                        .pick_chart(probe.id, probe.request(at.0, at.1, 3.0))
                        .unwrap()
                        .resolve(),
                )
                .unwrap()
                .unwrap_or_else(|| panic!("{case}: GPU misses point {index} at {at:?}"));
                let cpu = renderer::pick::cpu_twin::pick_nearest_point(
                    &cfg,
                    &series,
                    &columns,
                    at.0,
                    at.1,
                    renderer::pick::cpu_twin::PointPickOptions {
                        max_distance_px: 3.0,
                    },
                )
                .unwrap_or_else(|| panic!("{case}: CPU misses point {index} at {at:?}"));
                assert_eq!((gpu.point_index, cpu.point_index), (index, index), "{case}");
                assert!(
                    gpu.distance_px <= 1.0 && cpu.distance_px <= 1.0,
                    "{case}: point {index}: GPU {} px, CPU {} px from the value",
                    gpu.distance_px,
                    cpu.distance_px
                );
                assert!(
                    (gpu.distance_px - cpu.distance_px).abs() <= 0.5,
                    "{case}: point {index}: GPU {} px, CPU {} px",
                    gpu.distance_px,
                    cpu.distance_px
                );
                checked += 1;
            }
            renderer.remove_chart(probe.id).unwrap();
        }
    }
    assert_eq!(checked, 3 * magnitudes.len() * fractions.len());
}

/// An error-bar style of stem width `width` with caps of half size `cap`
/// and width `cap_width` in `color`, without per-point styles.
pub fn errorbar_style(
    color: Color,
    width: f32,
    cap: f32,
    cap_width: f32,
) -> renderer::data_config::DataErrorBarStyleConfig {
    renderer::data_config::DataErrorBarStyleConfig {
        error_bar_color: color,
        error_bar_width: width,
        error_bar_cap_size: cap,
        cap_width,
        error_bar_style_table: None,
        error_bar_style_index_column: None,
        error_bar_style_overrides: None,
    }
}

/// A marker series `id` on the x / y columns whose error bars are `kind`:
/// "x", "y", "xy" (symmetric, `ex` / `ey`) or "asym" (Y, `ey_lo` / `ey_hi`).
pub fn errorbar_series(
    id: &str,
    kind: &str,
    err_style: renderer::data_config::DataErrorBarStyleConfig,
) -> SeriesConfig {
    use renderer::data_config::ErrorRef;
    let DataRenderType::Scatter { scatter } =
        scatter(id, ScatterShape::CircleFilled, 2.0, BLUE).render_type
    else {
        unreachable!()
    };
    let symmetric = |column: &str| ErrorRef::Symmetric {
        column: column.into(),
    };
    let render_type = match kind {
        "x" => DataRenderType::ScatterErrorbarX {
            scatter,
            err_x: symmetric("ex"),
            err_style,
        },
        "y" => DataRenderType::ScatterErrorbarY {
            scatter,
            err_y: symmetric("ey"),
            err_style,
        },
        "xy" => DataRenderType::ScatterErrorbarXY {
            scatter,
            err_x: symmetric("ex"),
            err_y: symmetric("ey"),
            err_style,
        },
        "asym" => DataRenderType::ScatterErrorbarY {
            scatter,
            err_y: ErrorRef::Asymmetric {
                lower: "ey_lo".into(),
                upper: "ey_hi".into(),
            },
            err_style,
        },
        other => panic!("no error-bar kind {other}"),
    };
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type,
    }
}

/// Three points with every error column the error-bar cases read.
fn upload_errorbar_columns(renderer: &mut Renderer) {
    upload_xy(renderer, vec![3.0, 5.0, 7.0], vec![4.0, 5.5, 6.5]);
    renderer
        .add_columns(&[
            (
                "ex",
                &col(vec![0.8, 1.2, 0.6]) as &dyn renderer::ColumnSource,
            ),
            (
                "ey",
                &col(vec![1.5, 2.0, 1.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "ey_lo",
                &col(vec![0.5, 2.5, 0.3]) as &dyn renderer::ColumnSource,
            ),
            (
                "ey_hi",
                &col(vec![2.0, 0.4, 1.8]) as &dyn renderer::ColumnSource,
            ),
            (
                "bar_style",
                &col(vec![1.0, 0.0, 1.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
}

/// G10: an error bar picks its stems and caps exactly where the error-bar
/// shader draws them, with the shared quads (`common/errorbar_segment.wgsl`):
/// X, Y and XY bars, asymmetric errors, per-point widths of the precise
/// style map, a log axis, Precise and Sketch (the hashed corner wobble), at
/// 1x and 2x. A Milkyway jet picks within its beam and knot envelope: every
/// painted pixel picks and nothing picks past the envelope. A transparent
/// bar draws nothing and picks nothing.
#[test]
fn error_bars_pick_where_their_stems_and_caps_are_drawn() {
    use renderer::config::{AxisScale, DrawStyle, MilkywayOptions, SketchOptions};
    use renderer::data_config::{DataErrorBarPointStyleConfig, DataErrorBarPointStyleOverride};
    let mut renderer = renderer();
    upload_errorbar_columns(&mut renderer);
    let plain = errorbar_style(RED, 2.0, 5.0, 1.5);
    // Point 0 and 2 take slot 1, point 1 slot 0; point 2 also has an
    // override of its cap.
    let mut mapped = errorbar_style(RED, 1.5, 4.0, 1.0);
    mapped.error_bar_style_index_column = Some("bar_style".into());
    mapped.error_bar_style_table = Some(vec![
        DataErrorBarPointStyleConfig {
            error_bar_color: None,
            error_bar_width: Some(4.0),
            error_bar_cap_size: None,
            cap_width: Some(3.0),
        },
        DataErrorBarPointStyleConfig {
            error_bar_color: Some(BLUE),
            error_bar_width: Some(1.0),
            error_bar_cap_size: Some(7.0),
            cap_width: None,
        },
    ]);
    mapped.error_bar_style_overrides = Some(vec![DataErrorBarPointStyleOverride {
        index: 2,
        style: DataErrorBarPointStyleConfig {
            error_bar_color: None,
            error_bar_width: None,
            error_bar_cap_size: Some(2.5),
            cap_width: Some(2.0),
        },
    }]);
    let precise = bare_chart();
    let mut log = bare_chart();
    log.config_mut().left_y.scale = AxisScale::Logarithmic;
    log.set_y_range(1.0, 10.0);
    let sketch = styled_chart(DrawStyle::Sketch(SketchOptions {
        amplitude_px: 3.0,
        wavelength_px: 12.0,
        seed: 11,
    }));
    for scale in [1.0, 2.0] {
        for (chart, style, what) in [
            (&precise, &plain, "precise"),
            (&precise, &mapped, "precise style map"),
            (&log, &plain, "log y"),
            (&sketch, &plain, "sketch"),
        ] {
            for kind in ["x", "y", "xy", "asym"] {
                let series = [errorbar_series("bars", kind, style.clone())];
                let painted = assert_pick_matches_paint_with(
                    &mut renderer,
                    chart,
                    &series,
                    "bars",
                    scale,
                    None,
                    &format!("{what} {kind} bars"),
                    MSAA_EDGE,
                );
                assert!(painted > 40, "{what} {kind} at {scale}x: {painted}");
            }
        }

        let options = MilkywayOptions::default();
        let milkyway = styled_chart(DrawStyle::Milkyway(options));
        let series = [errorbar_series("bars", "xy", plain.clone())];
        let painted =
            assert_painted_pixels_pick(&mut renderer, &milkyway, &series, "bars", scale, "jets");
        assert!(painted > 100, "jets at {scale}x: {painted}");
        // The envelope: a beam of 4 times the half stem (at least 1.2 px)
        // either side of the stem, a knot disc of 1.7 times the cap (at
        // least 3 px) around each bound. Point 1's Y stem runs from y 3.5
        // to 7.5 at x 5.
        let s = f64::from(scale);
        let beam = (2.0 * s * 0.5).max(1.2) * 4.0;
        let knot = (5.0 * s).max(3.0) * 1.7;
        let (cx, cy) = data_to_px(&milkyway, 5.0, 6.5, scale);
        let (_, top) = data_to_px(&milkyway, 5.0, 7.5, scale);
        assert_nothing_picks_at(
            &mut renderer,
            &milkyway,
            &series,
            "bars",
            scale,
            &[
                (cx - beam - 2.0, cy),
                (cx + beam + 2.0, cy),
                (cx, top - knot - 2.0),
            ],
            "beyond a jet's envelope",
        );

        // A transparent bar draws nothing: along point 1's Y stem, away
        // from its marker, nothing picks.
        let mut clear = plain.clone();
        clear.error_bar_color = Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
        let series = [errorbar_series("bars", "y", clear)];
        let (x, y) = data_to_px(&precise, 5.0, 6.8, scale);
        assert_nothing_picks_at(
            &mut renderer,
            &precise,
            &series,
            "bars",
            scale,
            &[(x, y)],
            "transparent bar",
        );
    }
}

/// A line series `id` on the x / y columns filled by `fill` in opaque
/// green, with a 1.5 px stroke.
pub fn filled_line(
    id: &str,
    fill: Option<renderer::data_config::LineFill>,
    interpolation: renderer::data_config::LineInterpolation,
    fill_color: Color,
) -> SeriesConfig {
    let mut series = line(
        id,
        1.5,
        renderer::line::LineStylePreset::Solid,
        interpolation,
    );
    let DataRenderType::Line { line } = &mut series.render_type else {
        unreachable!()
    };
    line.fill = fill;
    line.fill_color = Some(fill_color);
    series
}

const GREEN: Color = Color {
    r: 0.0,
    g: 0.6,
    b: 0.0,
    a: 1.0,
};

/// A fill has no anti-aliasing: its trapezoids are covered by the export's
/// multisampling only, and the fragment at the pixel centre colours every
/// covered sample. Along a straight edge a pixel whose centre lies outside
/// paints at most two of four samples (ink 128 for these opaque fills) and
/// one whose centre lies inside at least two, so a zero-distance pick must
/// find half the ink. At a corner of the region three samples can cover a
/// pixel whose centre is outside (a polar fill's base polygon), so only a
/// pixel covered by all four samples must pick.
pub const FILL_EDGE: Tolerance = Tolerance {
    painted: 192,
    fringe: 2.0,
    zero_ink: 96,
    skip_near: None,
};

/// [`FILL_EDGE`] for a fill reaching another series: that series' stroke is
/// drawn over the fill's lower edge, so a pixel there is partly the
/// stroke's. Where the stroke is a rounding step out of reach the fill picks
/// it at zero distance, with the ink the stroke leaves visible.
pub const FILL_UNDER_STROKE: Tolerance = Tolerance {
    painted: 192,
    fringe: 2.0,
    zero_ink: 40,
    skip_near: None,
};

/// G10: a line's area fill picks exactly where its trapezoids paint
/// (`common/area_piece.wgsl`), at distance zero: to a baseline, to another
/// series, in every step mode and on a polar chart, at 1x and 2x. A
/// transparent fill paints nothing and picks nothing.
#[test]
fn area_fills_pick_where_they_paint() {
    use renderer::data_config::{LineFill, LineInterpolation};
    let mut renderer = renderer();
    upload_xy(
        &mut renderer,
        vec![2.0, 4.0, 5.5, 7.5],
        vec![5.0, 7.0, 4.0, 6.5],
    );
    renderer
        .add_columns(&[
            (
                "y2",
                &col(vec![2.0, 3.0, 1.5, 2.5]) as &dyn renderer::ColumnSource,
            ),
            (
                "angle",
                &col(vec![20.0, 110.0, 200.0, 320.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "radius",
                &col(vec![0.6, 0.9, 0.5, 0.8]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
    // A stroke edge on a pixel centre would leave the tie with the fill
    // there to rounding: 1.25 px keeps the edges off the centres at 1x and
    // 2x.
    let mut lower = line(
        "lower",
        1.25,
        renderer::line::LineStylePreset::Solid,
        LineInterpolation::Linear,
    );
    lower.y_column = "y2".into();
    let chart = bare_chart();
    for scale in [1.0, 2.0] {
        for interpolation in [
            LineInterpolation::Linear,
            LineInterpolation::StepPre,
            LineInterpolation::StepPost,
            LineInterpolation::StepMid,
        ] {
            for (fill, what) in [
                (LineFill::ToBaseline(1.0), "baseline"),
                (LineFill::ToSeries("lower".into()), "to series"),
            ] {
                let mut lower = lower.clone();
                let DataRenderType::Line { line } = &mut lower.render_type else {
                    unreachable!()
                };
                line.interpolation = interpolation;
                // The second series only where the fill reaches it.
                let to_series = matches!(fill, LineFill::ToSeries(_));
                let mut series = vec![filled_line("area", Some(fill), interpolation, GREEN)];
                if to_series {
                    series.push(lower);
                }
                let painted = assert_pick_matches_paint_with(
                    &mut renderer,
                    &chart,
                    &series,
                    "area",
                    scale,
                    None,
                    &format!("{what} {interpolation:?} fill"),
                    if to_series {
                        FILL_UNDER_STROKE
                    } else {
                        FILL_EDGE
                    },
                );
                assert!(
                    painted > 2000,
                    "{what} {interpolation:?} at {scale}x: {painted}"
                );
            }
        }

        let mut options = renderer::polar::PolarOptions::default();
        options.inner_radius_fraction = 0.2;
        let polar = bare_polar_chart(options);
        let mut polar_series = filled_line(
            "area",
            Some(LineFill::ToBaseline(0.0)),
            LineInterpolation::Linear,
            GREEN,
        );
        polar_series.x_column = "angle".into();
        polar_series.y_column = "radius".into();
        let painted = assert_pick_matches_paint_with(
            &mut renderer,
            &polar,
            &[polar_series],
            "area",
            scale,
            None,
            "polar fill",
            FILL_EDGE,
        );
        assert!(painted > 1000, "polar fill at {scale}x: {painted}");

        let clear = Color {
            r: 0.0,
            g: 0.6,
            b: 0.0,
            a: 0.0,
        };
        let series = [filled_line(
            "area",
            Some(LineFill::ToBaseline(1.0)),
            LineInterpolation::Linear,
            clear,
        )];
        let inside = [
            data_to_px(&chart, 4.0, 3.0, scale),
            data_to_px(&chart, 6.0, 2.0, scale),
        ];
        assert_nothing_picks_at(
            &mut renderer,
            &chart,
            &series,
            "area",
            scale,
            &inside,
            "transparent fill",
        );
    }
}

/// G10 tie-break: a marker over a fill picks the marker, though its series
/// comes first (every fill is drawn before every series body); inside the
/// fill away from the marker the fill picks, naming the nearer endpoint of
/// the filled segment.
#[test]
fn a_marker_over_a_fill_picks_the_marker() {
    use renderer::data_config::{LineFill, LineInterpolation};
    let mut renderer = renderer();
    upload_xy(&mut renderer, vec![2.0, 8.0], vec![8.0, 8.0]);
    renderer
        .add_columns(&[
            ("dot_x", &col(vec![4.0]) as &dyn renderer::ColumnSource),
            ("dot_y", &col(vec![4.0]) as &dyn renderer::ColumnSource),
        ])
        .expect("upload");
    let chart = bare_chart();
    let mut dot = scatter("dot", ScatterShape::CircleFilled, 6.0, RED);
    dot.x_column = "dot_x".into();
    dot.y_column = "dot_y".into();
    let area = filled_line(
        "area",
        Some(LineFill::ToBaseline(1.0)),
        LineInterpolation::Linear,
        GREEN,
    );
    let series = [dot, area];
    for scale in [1.0, 2.0] {
        let probe = Probe::register(&mut renderer, &chart, &series, scale);
        let (mx, my) = data_to_px(&chart, 4.0, 4.0, scale);
        let hit = probe
            .pick(&mut renderer, mx as u32, my as u32, 0.0)
            .expect("marker hit");
        assert_eq!((hit.series_id.as_str(), hit.point_index), ("dot", 0));
        for (x, index) in [(3.0, 0), (7.0, 1)] {
            let (px, py) = data_to_px(&chart, x, 3.0, scale);
            let hit = probe
                .pick(&mut renderer, px as u32, py as u32, 0.0)
                .expect("fill hit");
            assert_eq!(
                (hit.series_id.as_str(), hit.point_index, hit.distance_px),
                ("area", index, 0.0),
                "{scale}x at x {x}"
            );
        }
        renderer.remove_chart(probe.id).unwrap();
    }
}

/// Category bars pick the (category, series) bar they paint: grouped and
/// stacked, vertical and horizontal, with rounded exposed corners, at 1x and
/// 2x. The pick answers a histogram bin of the painted series whose index is
/// the category row. Coverage is sampled at 2x2 per pixel and the pick tests
/// the pixel centre, so a pixel an edge crosses may go either way within a
/// quarter of its samples.
#[test]
fn category_bars_pick_the_bar_they_paint() {
    use renderer::config::Category;
    use renderer::data_config::{BarGroups, CategoryBarDirection, CategoryBarMode};
    let mut renderer = renderer();
    renderer
        .add_column("pos", &col(vec![0.0, 1.0, 2.0]))
        .expect("upload");
    renderer
        .add_columns(&[
            (
                "one",
                &col(vec![3.0, 6.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "two",
                &col(vec![4.0, 2.0, 5.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
    let bars = |id: &str, color: Color| {
        let mut style = renderer::data_config::CategoryBarStyle::default();
        style.material = renderer::Material::Flat;
        style.outline = false;
        style.corner_radius = 4.0;
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "pos".into(),
            y_column: id.into(),
            render_type: DataRenderType::CategoryBars {
                color,
                style,
                labels: Default::default(),
                label_format: Default::default(),
                overrides: None,
            },
        }
    };
    for mode in [CategoryBarMode::Grouped, CategoryBarMode::Stacked] {
        for direction in [
            CategoryBarDirection::Vertical,
            CategoryBarDirection::Horizontal,
        ] {
            let mut chart = bare_chart();
            let config = chart.config_mut();
            // The axis lines draw above the data on the bars' zero edge.
            for axis in [
                &mut config.top_x,
                &mut config.bottom_x,
                &mut config.left_y,
                &mut config.right_y,
            ] {
                axis.line_visible = false;
            }
            let categories = Some(vec![
                Category::new("a", "A"),
                Category::new("b", "B"),
                Category::new("c", "C"),
            ]);
            let mut series = [bars("one", RED), bars("two", BLUE)];
            match direction {
                CategoryBarDirection::Vertical => config.bottom_x.categories = categories,
                CategoryBarDirection::Horizontal => {
                    config.left_y.categories = categories;
                    for s in &mut series {
                        std::mem::swap(&mut s.x_column, &mut s.y_column);
                    }
                }
            }
            config.bar_groups = Some(BarGroups {
                mode,
                direction,
                ..BarGroups::default()
            });
            for scale in [1.0, 2.0] {
                let what = format!("{mode:?} {direction:?} at {scale}x");
                let with = renderer.export_panel_rgba(&chart, &series, scale).unwrap();
                let without = renderer.export_panel_rgba(&chart, &[], scale).unwrap();
                let window = ink_window(&with, &without, 2);
                let is_painted = |x, y| difference(&with, &without, x, y) >= PAINTED;
                let painted = count_painted(window, is_painted);
                let probe = Probe::register(&mut renderer, &chart, &series, scale);
                let mut failures = Vec::new();
                let mut picked = 0usize;
                for (px, py) in edge_band(window, is_painted) {
                    let ink = difference(&with, &without, px, py);
                    let hit = probe.pick_data(&mut renderer, px, py, 0.0);
                    let bar = match &hit {
                        Some(renderer::PickedData::HistogramBin {
                            series_id,
                            bin_index,
                            ..
                        }) => Some((series_id.clone(), *bin_index)),
                        None => None,
                        Some(other) => panic!("{what}: ({px}, {py}) picked {other:?}"),
                    };
                    if ink > 160 && bar.is_none() {
                        failures.push(format!("({px}, {py}) painted (ink {ink}) but not picked"));
                    }
                    if ink < 96 && bar.is_some() {
                        failures.push(format!("({px}, {py}) picks {bar:?} but ink is only {ink}"));
                    }
                    // A fully painted pixel is one series' colour: the pick
                    // names that series.
                    let i = ((py * with.width + px) * 4) as usize;
                    let rgba = &with.rgba[i..i + 4];
                    if let Some((id, _)) = &bar
                        && ink > 240
                    {
                        picked += 1;
                        let painted_id = if rgba[0] > 200 && rgba[2] < 60 {
                            "one"
                        } else if rgba[2] > 200 && rgba[0] < 60 {
                            "two"
                        } else {
                            continue;
                        };
                        if id != painted_id {
                            failures
                                .push(format!("({px}, {py}) painted {painted_id}, picked {id}"));
                        }
                    }
                }
                assert!(
                    failures.is_empty(),
                    "{what}: {} disagreements, first {:?}",
                    failures.len(),
                    &failures[..failures.len().min(12)]
                );
                assert!(painted > 400, "{what}: {painted}");
                assert!(picked > 40, "{what}: {picked} fully painted edge pixels");
                renderer.remove_chart(probe.id).unwrap();
            }
        }
    }
}

/// Box plots pick the box they paint: the body (plain and notched), the
/// whiskers and caps, the median, the mean and the outliers, vertical and
/// horizontal, at 1x and 2x. Every part answers its box, a histogram bin of
/// the series whose index is the category row; the row of a painted pixel
/// is the category slot it lies in. The strokes are quads drawn without an
/// anti-aliasing margin, so a pixel the multisampling covers only at one
/// sample may pick (`MSAA_EDGE`); the markers' anti-aliasing reaches one
/// fringe past their contour (`STRICT.fringe`).
#[test]
fn box_plots_pick_the_box_they_paint() {
    use renderer::config::Category;
    use renderer::data_config::{BarGroups, BoxPlotStyle, CategoryBarDirection};
    use renderer::{BOX_PLOT_RECORD, ColumnSource, RaggedSource, RecordSource};
    let mut renderer = renderer();
    renderer.add_column("pos", &col(vec![0.0, 1.0])).unwrap();
    // whisker low, q1, median, q3, whisker high, mean, ci low, ci high
    let rows = [
        [1.0, 3.0, 4.0, 6.0, 8.0, 5.0, 3.5, 4.5],
        [2.0, 4.0, 5.0, 7.0, 9.0, 6.0, 4.5, 5.5],
    ];
    let field = |k: usize| col(rows.iter().map(|r| r[k]).collect());
    let columns = [
        field(2),
        field(1),
        field(3),
        field(0),
        field(4),
        field(5),
        field(6),
        field(7),
    ];
    let names = [
        "median",
        "q1",
        "q3",
        "whisker_low",
        "whisker_high",
        "mean",
        "ci_low",
        "ci_high",
    ];
    let fields: Vec<(&str, &dyn ColumnSource)> = names
        .iter()
        .zip(&columns)
        .map(|(n, c)| (*n, c as &dyn ColumnSource))
        .collect();
    renderer
        .upsert_record(
            "record",
            &RecordSource {
                schema: BOX_PLOT_RECORD,
                fields: &fields,
            },
        )
        .unwrap();
    renderer
        .upsert_ragged(
            "outliers",
            &RaggedSource {
                values: &col(vec![0.2, 9.6, 9.8]),
                offsets: &col(vec![0.0, 1.0, 3.0]),
            },
        )
        .unwrap();
    for notched in [false, true] {
        for direction in [
            CategoryBarDirection::Vertical,
            CategoryBarDirection::Horizontal,
        ] {
            let mut chart = bare_chart();
            let config = chart.config_mut();
            for axis in [
                &mut config.top_x,
                &mut config.bottom_x,
                &mut config.left_y,
                &mut config.right_y,
            ] {
                axis.line_visible = false;
            }
            let categories = Some(vec![Category::new("a", "A"), Category::new("b", "B")]);
            match direction {
                CategoryBarDirection::Vertical => {
                    config.bottom_x.categories = categories;
                    config.left_y.min = 0.0;
                    config.left_y.max = 10.0;
                }
                CategoryBarDirection::Horizontal => {
                    config.left_y.categories = categories;
                    config.bottom_x.min = 0.0;
                    config.bottom_x.max = 10.0;
                }
            }
            config.bar_groups = Some(BarGroups {
                direction,
                ..BarGroups::default()
            });
            let series = [SeriesConfig {
                series_id: "box".into(),
                source_id: None,
                label: None,
                x_column: "pos".into(),
                y_column: "record".into(),
                render_type: DataRenderType::BoxPlot {
                    color: RED,
                    style: BoxPlotStyle {
                        material: renderer::Material::Flat,
                        notched,
                        show_mean: true,
                        outlier_filled: notched,
                        ..BoxPlotStyle::default()
                    },
                    labels: Default::default(),
                    outliers: Some("outliers".into()),
                    overrides: None,
                },
            }];
            for scale in [1.0, 2.0] {
                let what = format!("notched {notched} {direction:?} at {scale}x");
                let with = renderer.export_panel_rgba(&chart, &series, scale).unwrap();
                let without = renderer.export_panel_rgba(&chart, &[], scale).unwrap();
                let window = ink_window(&with, &without, 2);
                let is_painted = |x, y| difference(&with, &without, x, y) >= PAINTED;
                let painted = count_painted(window, is_painted);
                let da = chart.config().data_area().unwrap();
                // The category slot an export pixel lies in.
                let row_of = |x: u32, y: u32| -> usize {
                    match direction {
                        CategoryBarDirection::Vertical => {
                            let t = (x as f32 / scale - da.x as f32) / da.width as f32;
                            (t * 2.0).floor().clamp(0.0, 1.0) as usize
                        }
                        CategoryBarDirection::Horizontal => {
                            let t =
                                ((da.y + da.height) as f32 - y as f32 / scale) / da.height as f32;
                            (t * 2.0).floor().clamp(0.0, 1.0) as usize
                        }
                    }
                };
                let probe = Probe::register(&mut renderer, &chart, &series, scale);
                let mut failures = Vec::new();
                let mut picked = 0usize;
                for (px, py) in edge_band(window, is_painted) {
                    let ink = difference(&with, &without, px, py);
                    let hit = match probe.pick_data(&mut renderer, px, py, 0.0) {
                        Some(renderer::PickedData::HistogramBin {
                            series_id,
                            bin_index,
                            ..
                        }) => {
                            assert_eq!(series_id, "box", "{what}");
                            Some(bin_index)
                        }
                        None => None,
                        Some(other) => panic!("{what}: ({px}, {py}) picked {other:?}"),
                    };
                    // A marker's anti-aliasing reaches one `fwidth` past its
                    // contour: a painted pixel picks within that fringe.
                    if ink > 160
                        && hit.is_none()
                        && probe
                            .pick_data(&mut renderer, px, py, STRICT.fringe)
                            .is_none()
                    {
                        failures.push(format!("({px}, {py}) painted (ink {ink}) but not picked"));
                    }
                    if ink < MSAA_EDGE.zero_ink && hit.is_some() {
                        failures.push(format!("({px}, {py}) picks {hit:?} but ink is only {ink}"));
                    }
                    if let Some(row) = hit
                        && ink > 240
                    {
                        picked += 1;
                        if row != row_of(px, py) {
                            failures.push(format!(
                                "({px}, {py}) in slot {} picked box {row}",
                                row_of(px, py)
                            ));
                        }
                    }
                }
                assert!(
                    failures.is_empty(),
                    "{what}: {} disagreements, first {:?}",
                    failures.len(),
                    &failures[..failures.len().min(12)]
                );
                assert!(painted > 400, "{what}: {painted}");
                assert!(picked > 40, "{what}: {picked} fully painted edge pixels");
                // The outliers pick their box: the first box's outlier near
                // the bottom (or left) of the value axis, from its centre
                // (inside an open ring, within the ring's reach).
                let outlier = match direction {
                    CategoryBarDirection::Vertical => {
                        let x = da.x as f32 + da.width as f32 * 0.25;
                        let y = (da.y + da.height) as f32 - da.height as f32 * 0.02;
                        (x * scale, y * scale)
                    }
                    CategoryBarDirection::Horizontal => {
                        let x = da.x as f32 + da.width as f32 * 0.02;
                        let y = (da.y + da.height) as f32 - da.height as f32 * 0.25;
                        (x * scale, y * scale)
                    }
                };
                assert!(
                    matches!(
                        probe.pick_data_at(&mut renderer, outlier, 4.0 * scale),
                        Some(renderer::PickedData::HistogramBin { bin_index: 0, .. })
                    ),
                    "{what}: the outlier picks its box"
                );
                renderer.remove_chart(probe.id).unwrap();
            }
        }
    }
}

/// A polar chart whose angle axis holds `n` categories (radar spokes) over
/// a 0..1 radius, without chrome.
fn bare_radar_chart(n: usize, polar: renderer::polar::PolarOptions) -> Chart {
    let mut chart = bare_polar_chart(polar);
    chart.config_mut().bottom_x.categories = Some(
        (0..n)
            .map(|i| renderer::config::Category::new(format!("c{i}"), format!("C{i}")))
            .collect(),
    );
    chart
}

fn radar(id: &str, style: renderer::RadarSeriesStyle) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "cat".into(),
        y_column: "val".into(),
        render_type: DataRenderType::Radar {
            color: RED,
            style,
            labels: Default::default(),
        },
    }
}

/// G12: a radar polygon picks where it paints: its outline with the
/// closing segment from the last vertex back to the first, its vertex
/// markers and its fill to the centre (or the hole), at 1x and 2x, on the
/// full circle and in a sector (where the closing trapezoid is not filled).
/// The closing segment names its nearer endpoint, the last vertex or the
/// first.
#[test]
fn radar_polygons_pick_their_outline_closing_segment_markers_and_fill() {
    use renderer::RadarMarker;
    use renderer::polar::PolarOptions;
    let mut renderer = renderer();
    renderer
        .add_columns(&[
            (
                "cat",
                &col(vec![0.0, 1.0, 2.0, 3.0, 4.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "val",
                &col(vec![0.9, 0.55, 0.8, 0.4, 0.7]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
    let outline = renderer::RadarSeriesStyle {
        line_width_px: Some(2.5),
        fill: Some(Color::new(0.0, 0.0, 0.0, 0.0)),
        marker: Some(RadarMarker::None),
        ..Default::default()
    };
    let markers = renderer::RadarSeriesStyle {
        marker: Some(RadarMarker::Square),
        marker_size_px: Some(9.0),
        ..outline.clone()
    };
    let filled = renderer::RadarSeriesStyle {
        fill: Some(GREEN),
        marker: Some(RadarMarker::Circle),
        ..markers.clone()
    };
    for (polar, what) in [
        (
            PolarOptions {
                zero_angle_degrees: 90.0,
                clockwise: true,
                ..PolarOptions::default()
            },
            "full circle",
        ),
        (
            PolarOptions {
                sector: Some([0.0, 270.0]),
                inner_radius_fraction: 0.2,
                ..PolarOptions::default()
            },
            "sector with a hole",
        ),
    ] {
        let chart = bare_radar_chart(5, polar);
        for scale in [1.0, 2.0] {
            for (style, tolerance, case) in [
                (outline.clone(), STRICT, "outline"),
                (markers.clone(), STRICT, "outline and markers"),
                (filled.clone(), FILL_EDGE, "filled"),
            ] {
                let series = [radar("radar", style)];
                let painted = assert_pick_matches_paint_with(
                    &mut renderer,
                    &chart,
                    &series,
                    "radar",
                    scale,
                    None,
                    &format!("{what} {case}"),
                    tolerance,
                );
                assert!(painted > 200, "{what} {case} at {scale}x: {painted}");
            }
            // The closing segment picks where it is drawn and names its
            // nearer endpoint: near the last vertex the last one, near the
            // first vertex the first. (Its middle can lie outside a sector
            // or inside the hole, where it is clipped.)
            let config = chart.config().scaled(scale);
            let map = config.polar_angle_map().unwrap();
            let geometry = config.polar_geometry().unwrap();
            let inner = f64::from(geometry.inner_radius);
            let radius = |v: f64| inner + (f64::from(geometry.radius) - inner) * v;
            let vertex = |i: f64, v: f64| {
                let a = map.screen_angle(i);
                (
                    f64::from(geometry.center[0]) + a.cos() * radius(v),
                    f64::from(geometry.center[1]) - a.sin() * radius(v),
                )
            };
            let (a, b) = (vertex(4.0, 0.7), vertex(0.0, 0.9));
            let series = [radar("radar", outline.clone())];
            let image = renderer.export_panel_rgba(&chart, &series, scale).unwrap();
            let probe = Probe::register(&mut renderer, &chart, &series, scale);
            let mut probed = 0;
            for (from, to, index, end) in [(a, b, 4, "last"), (b, a, 0, "first")] {
                // The first drawn pixel of this half of the segment, from
                // its end inward; a sector edge can clip the whole half.
                let Some((x, y)) = (3..50).find_map(|k| {
                    let t = f64::from(k) / 100.0;
                    let (x, y) = (
                        (from.0 + (to.0 - from.0) * t) as u32,
                        (from.1 + (to.1 - from.1) * t) as u32,
                    );
                    (pixel(&image, x, y)[0] > 128).then_some((x, y))
                }) else {
                    continue;
                };
                let hit = probe
                    .pick(&mut renderer, x, y, 2.0)
                    .unwrap_or_else(|| panic!("{what} at {scale}x: the closing segment picks"));
                assert_eq!(
                    hit.point_index, index,
                    "{what} at {scale}x: near the {end} vertex"
                );
                probed += 1;
            }
            assert!(
                probed > 0,
                "{what} at {scale}x: the closing segment is drawn"
            );
            renderer.remove_chart(probe.id).unwrap();
        }
    }
}

/// Value labels pick where they paint (#40 item 13h). Each label of a
/// category bar series (outside its grouped bars, inside its stacked
/// segments) and of a radar series, at 1x and 2x: every pixel a label
/// paints picks that label, by its series and category, at distance zero,
/// and a label picks only within its selection box (its content box grown
/// by `SELECTION_PADDING`), never another label. A label is drawn over all
/// of its panel's data, so it wins the tie against the bar it sits inside:
/// the white text inside a segment picks the label, not the bar.
#[test]
fn value_labels_pick_where_they_paint() {
    use renderer::config::Category;
    use renderer::data_config::{
        BarGroups, CategoryBarDirection, CategoryBarLabels, CategoryBarMode,
        CategoryBarSeriesLabels,
    };
    let mut renderer = renderer();
    renderer
        .add_column("pos", &col(vec![0.0, 1.0, 2.0]))
        .expect("upload");
    renderer
        .add_columns(&[
            (
                "one",
                &col(vec![3.0, 6.0, 4.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "two",
                &col(vec![4.0, 2.0, 5.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
    renderer
        .add_column("cat", &col(vec![0.0, 1.0, 2.0, 3.0]))
        .expect("upload");
    renderer
        .add_column("radar", &col(vec![0.6, 0.5, 0.7, 0.4]))
        .expect("upload");
    let bars = |id: &str, color: Color, placement: CategoryBarLabels| {
        let mut style = renderer::data_config::CategoryBarStyle::default();
        style.material = renderer::Material::Flat;
        style.outline = false;
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "pos".into(),
            y_column: id.into(),
            render_type: DataRenderType::CategoryBars {
                color,
                style,
                labels: CategoryBarSeriesLabels {
                    placement,
                    text: vec!["3".into(), "Ab".into(), "4%".into()],
                    ..CategoryBarSeriesLabels::default()
                },
                label_format: Default::default(),
                overrides: None,
            },
        }
    };
    let hidden = |series: &[SeriesConfig]| -> Vec<SeriesConfig> {
        series
            .iter()
            .cloned()
            .map(|mut s| {
                match &mut s.render_type {
                    DataRenderType::CategoryBars { labels, .. } => {
                        labels.placement = CategoryBarLabels::None
                    }
                    DataRenderType::Radar { labels, .. } => {
                        labels.placement = renderer::RadarLabels::None
                    }
                    _ => {}
                }
                s
            })
            .collect()
    };
    let mut cases: Vec<(String, Chart, Vec<SeriesConfig>)> = Vec::new();
    for (mode, placement, top) in [
        (CategoryBarMode::Grouped, CategoryBarLabels::Outside, 8.0),
        (CategoryBarMode::Stacked, CategoryBarLabels::Inside, 12.0),
    ] {
        let mut chart = bare_chart();
        let config = chart.config_mut();
        for axis in [&mut config.bottom_x, &mut config.left_y] {
            axis.line_visible = false;
        }
        config.bottom_x.categories = Some(vec![
            Category::new("a", "A"),
            Category::new("b", "B"),
            Category::new("c", "C"),
        ]);
        config.bar_groups = Some(BarGroups {
            mode,
            direction: CategoryBarDirection::Vertical,
            ..BarGroups::default()
        });
        chart.set_y_range(0.0, top);
        cases.push((
            format!("{mode:?} bars"),
            chart,
            vec![bars("one", RED, placement), bars("two", BLUE, placement)],
        ));
    }
    let mut chart = bare_chart();
    let config = chart.config_mut();
    config.bottom_x.categories = Some(
        (0..4)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect(),
    );
    config.polar = Some(renderer::polar::PolarOptions::default());
    chart.set_y_range(0.0, 1.0);
    cases.push((
        "radar".into(),
        chart,
        vec![SeriesConfig {
            series_id: "radar".into(),
            source_id: None,
            label: None,
            x_column: "cat".into(),
            y_column: "radar".into(),
            render_type: DataRenderType::Radar {
                color: RED,
                style: renderer::RadarSeriesStyle {
                    marker: Some(renderer::RadarMarker::Circle),
                    ..Default::default()
                },
                labels: renderer::RadarSeriesLabels {
                    placement: renderer::RadarLabels::Values,
                    text: vec!["0.6".into(), "0.5".into(), "0.7".into(), "0.4".into()],
                    ..renderer::RadarSeriesLabels::default()
                },
            },
        }],
    ));
    for (name, chart, series) in &cases {
        for scale in [1.0f32, 2.0] {
            let what = format!("{name} at {scale}x");
            let with = renderer.export_panel_rgba(chart, series, scale).unwrap();
            let without = renderer
                .export_panel_rgba(chart, &hidden(series), scale)
                .unwrap();
            let is_painted = |x, y| difference(&with, &without, x, y) >= 160;
            let window = (0, 0, with.width, with.height);
            let painted = count_painted(window, is_painted);
            assert!(painted > 30, "{what}: {painted} label pixels");
            let probe = Probe::register(&mut renderer, chart, series, scale);
            // Every label pixel within the box of its pixels grown by the
            // selection padding and the glyphs' side bearings.
            let reach = renderer::select::SELECTION_PADDING * scale + 3.0 * scale + 1.0;
            let near_paint = |x: u32, y: u32| {
                let r = reach.ceil() as i64;
                (-r..=r).any(|dy| {
                    (-r..=r).any(|dx| {
                        let (px, py) = (x as i64 + dx, y as i64 + dy);
                        px >= 0
                            && py >= 0
                            && (px as u32) < with.width
                            && (py as u32) < with.height
                            && is_painted(px as u32, py as u32)
                    })
                })
            };
            let mut failures = Vec::new();
            let mut picked = 0usize;
            let step = if scale > 1.0 { 2 } else { 1 };
            for (px, py) in edge_band(window, is_painted) {
                if (px % step) != 0 || (py % step) != 0 {
                    continue;
                }
                let painted_here = is_painted(px, py);
                if !painted_here && !near_paint(px, py) {
                    continue;
                }
                let hit = probe.pick_data(&mut renderer, px, py, 0.0);
                match &hit {
                    Some(renderer::PickedData::ValueLabel { .. }) => {
                        picked += 1;
                        if !near_paint(px, py) {
                            failures.push(format!("({px}, {py}) picks {hit:?} far from paint"));
                        }
                    }
                    _ if painted_here => {
                        failures.push(format!("({px}, {py}) painted but picked {hit:?}"))
                    }
                    _ => {}
                }
            }
            assert!(
                failures.is_empty(),
                "{what}: {} disagreements, first {:?}",
                failures.len(),
                &failures[..failures.len().min(12)]
            );
            assert!(picked > 20, "{what}: {picked} label picks");
            renderer.remove_chart(probe.id).unwrap();
        }
    }
}

/// A streamline series over the `x` / `y` columns with 10 px wide heads
/// every 24 px of drawn arc.
fn streamline(
    id: &str,
    width: f32,
    style: renderer::line::LineStylePreset,
    color_map: Option<&str>,
) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Streamlines {
            line: renderer::DataLineStyleConfig {
                line_style: style,
                line_color: RED,
                line_width: width,
                fill: None,
                fill_color: None,
                interpolation: renderer::data_config::LineInterpolation::Linear,
            },
            arrows: Some(renderer::StreamlineArrows {
                spacing_px: 24.0,
                head_length_px: 9.0,
                head_width_px: 10.0,
            }),
            color_map: color_map.map(|column| renderer::data_config::PointColorMap {
                column: column.into(),
            }),
        },
    }
}

/// G-streamlines: a streamline series picks where its stroke and its
/// arrowheads paint, at 1x and 2x: two curved streamlines separated by a NaN
/// row (the gap picks nothing), solid and dashed, plain and colour-mapped,
/// on linear axes and on a logarithmic y axis, where the heads are placed in
/// data space and the pick follows the same placement. A pick on a head
/// names the nearer endpoint of the segment it sits on.
#[test]
fn streamline_strokes_and_arrowheads_pick_where_they_paint() {
    use renderer::line::LineStylePreset;
    let mut renderer = renderer();
    let xs = vec![1.5, 3.0, 4.5, 6.0, 7.5, f64::NAN, 2.0, 4.0, 6.5, 8.5];
    let ys = vec![2.0, 3.4, 3.9, 3.5, 2.4, f64::NAN, 8.5, 6.6, 6.2, 7.4];
    let speed = vec![0.0, 2.0, 4.0, 6.0, 8.0, f64::NAN, 10.0, 7.0, 4.0, 1.0];
    renderer
        .add_columns(&[
            ("x", &col(xs) as &dyn renderer::ColumnSource),
            ("y", &col(ys) as &dyn renderer::ColumnSource),
            ("speed", &col(speed) as &dyn renderer::ColumnSource),
        ])
        .expect("upload");
    let mut chart = bare_chart();
    let mut bar = renderer::default::default_colorbar_options();
    bar.visible = false;
    bar.axis.min = 0.0;
    bar.axis.max = 10.0;
    // Saturated stops, so a half-covered pixel's ink is the red line's.
    bar.colormap = renderer::ColorMap::Custom {
        stops: vec![RED, BLUE],
    };
    chart.config_mut().colorbar = Some(bar);
    let mut log = Chart::new(chart.config().clone());
    log.config_mut().left_y.scale = renderer::config::AxisScale::Logarithmic;
    log.config_mut().right_y.scale = renderer::config::AxisScale::Logarithmic;
    log.set_y_range(1.0, 10.0);
    for (chart, axes) in [(&chart, "linear"), (&log, "log y")] {
        for (style, color_map) in [
            (LineStylePreset::Solid, None),
            (LineStylePreset::Dash, Some("speed")),
        ] {
            for scale in [1.0, 2.0] {
                let series = [streamline("flow", 1.5, style, color_map)];
                let what = format!("{axes} {style:?} {color_map:?}");
                // A dashed stroke has hard, multisampled edges, as in
                // `dashed_lines_pick_their_dashes_and_not_their_gaps`.
                let tolerance = if style == LineStylePreset::Solid {
                    STRICT
                } else {
                    MSAA_EDGE
                };
                let painted = assert_pick_matches_paint_with(
                    &mut renderer,
                    chart,
                    &series,
                    "flow",
                    scale,
                    None,
                    &what,
                    tolerance,
                );
                assert!(painted > 200, "{what}: {painted}");
            }
        }
    }
    // The gap between the streamlines picks nothing.
    let series = [streamline("flow", 1.5, LineStylePreset::Solid, None)];
    let gap = [
        data_to_px(&chart, 8.0, 5.0, 1.0),
        data_to_px(&chart, 5.0, 5.0, 1.0),
    ];
    assert_nothing_picks_at(
        &mut renderer,
        &chart,
        &series,
        "flow",
        1.0,
        &gap,
        "the NaN gap",
    );
}

/// [`assert_pick_matches_paint_with`] for a line offset member
/// (`Config::line_offsets`): the document export draws each member in a panel
/// of its own (the panel export keeps the base panel), and the comparison
/// image keeps the member in the group, drawing nothing, so every other
/// member keeps its shift. The subject's columns are swapped for the
/// all-NaN `nan_x` / `nan_y`.
#[allow(clippy::too_many_arguments)]
fn assert_member_pick_matches_paint(
    renderer: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    subject: &str,
    scale: f32,
    what: &str,
    tolerance: Tolerance,
) -> usize {
    let with = renderer.export_document_rgba(chart, series, scale).unwrap();
    let blank: Vec<SeriesConfig> = series
        .iter()
        .cloned()
        .map(|mut s| {
            if s.series_id == subject {
                s.x_column = "nan_x".into();
                s.y_column = "nan_y".into();
            }
            s
        })
        .collect();
    let without = renderer.export_document_rgba(chart, &blank, scale).unwrap();
    let window = ink_window(&with, &without, 4);
    let is_painted = |x, y| difference(&with, &without, x, y) >= tolerance.painted;
    let painted = count_painted(window, is_painted);
    let probe = Probe::register(renderer, chart, series, scale);
    let mut failures = Vec::new();
    for (x, y) in edge_band(window, is_painted) {
        let ink = difference(&with, &without, x, y);
        let hit = probe
            .pick(renderer, x, y, tolerance.fringe)
            .filter(|hit| hit.series_id == subject);
        if ink >= tolerance.painted && hit.is_none() {
            failures.push(format!("({x}, {y}) painted (ink {ink}) but not picked"));
        }
        if let Some(hit) = hit
            && hit.distance_px == 0.0
            && ink < tolerance.zero_ink
            && !probe.on_zero_edge(renderer, x, y, subject)
        {
            failures.push(format!(
                "({x}, {y}) picked at distance 0 (index {}) but ink is only {ink}",
                hit.point_index
            ));
        }
    }
    // The typed pick names the member too, at the centre of its ink.
    let (cx, cy) = {
        let (mut sx, mut sy, mut n) = (0u64, 0u64, 0u64);
        for y in window.1..window.3 {
            for x in window.0..window.2 {
                if difference(&with, &without, x, y) >= 192 {
                    sx += u64::from(x);
                    sy += u64::from(y);
                    n += 1;
                }
            }
        }
        assert!(n > 0, "{what}: no opaque ink");
        // The opaque pixel nearest the centroid.
        let (mx, my) = ((sx / n) as u32, (sy / n) as u32);
        (window.1..window.3)
            .flat_map(|y| (window.0..window.2).map(move |x| (x, y)))
            .filter(|&(x, y)| difference(&with, &without, x, y) >= 192)
            .min_by_key(|&(x, y)| x.abs_diff(mx).pow(2) + y.abs_diff(my).pow(2))
            .unwrap()
    };
    match probe.pick_data(renderer, cx, cy, tolerance.fringe) {
        Some(renderer::PickedData::Point { series_id, .. }) if series_id == subject => {}
        other => failures.push(format!("typed pick at ({cx}, {cy}): {other:?}")),
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

/// Offset spectra (#50): every member picks exactly where its shifted parts
/// paint and nothing unpainted picks: a solid line with a baseline fill, a
/// dashed line with markers and error bars, a plain line two steps up, and
/// the members of a logarithmic axis stacked by decades, at 1x and 2x.
#[test]
fn offset_lines_pick_where_their_shifted_parts_paint() {
    use renderer::config::AxisScale;
    use renderer::data_config::{ErrorRef, LineFill, LineInterpolation, LineOffsets};
    use renderer::line::LineStylePreset;
    let mut renderer = renderer();
    let xs = vec![1.0, 2.5, 4.0, 5.5, 7.0, 8.5];
    let n = xs.len();
    upload_xy(&mut renderer, xs, vec![1.0, 1.8, 1.2, 2.0, 1.4, 1.6]);
    renderer
        .add_columns(&[
            (
                "ey",
                &col(vec![0.4, 0.3, 0.5, 0.3, 0.4, 0.2]) as &dyn renderer::ColumnSource,
            ),
            (
                "nan_x",
                &col(vec![f64::NAN; n]) as &dyn renderer::ColumnSource,
            ),
            (
                "nan_y",
                &col(vec![f64::NAN; n]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
    let filled = filled_line(
        "filled",
        Some(LineFill::ToBaseline(0.4)),
        LineInterpolation::Linear,
        Color::new(0.5, 0.8, 0.5, 1.0),
    );
    let DataRenderType::Scatter { scatter: markers } =
        scatter("dashed", ScatterShape::CircleFilled, 4.0, BLUE).render_type
    else {
        unreachable!()
    };
    let dashed = SeriesConfig {
        render_type: DataRenderType::LineScatterErrorbarY {
            scatter: markers,
            line: renderer::DataLineStyleConfig {
                line_style: LineStylePreset::Dash,
                line_color: BLUE,
                line_width: 2.0,
                fill: None,
                fill_color: None,
                interpolation: LineInterpolation::Linear,
            },
            err_y: ErrorRef::Symmetric {
                column: "ey".into(),
            },
            err_style: errorbar_style(GREEN, 1.5, 6.0, 1.5),
        },
        ..scatter("dashed", ScatterShape::CircleFilled, 4.0, BLUE)
    };
    let plain = line(
        "plain",
        2.0,
        LineStylePreset::Solid,
        LineInterpolation::Linear,
    );
    let series = [filled, dashed, plain];
    let mut chart = bare_chart();
    chart.config_mut().line_offsets = Some(LineOffsets {
        series: vec!["filled".into(), "dashed".into(), "plain".into()],
        step_y: 3.0,
        step_x: 0.0,
    });
    for scale in [1.0, 2.0] {
        for (subject, tolerance) in [
            ("filled", FILL_EDGE),
            ("dashed", MSAA_EDGE),
            ("plain", STRICT),
        ] {
            let painted = assert_member_pick_matches_paint(
                &mut renderer,
                &chart,
                &series,
                subject,
                scale,
                &format!("offset member {subject}"),
                tolerance,
            );
            assert!(painted > 30, "{subject} at {scale}x: {painted}");
        }
    }
    // A logarithmic axis stacks by decades; a skew moves the second member
    // along x too.
    let mut log = bare_chart();
    log.config_mut().left_y.scale = AxisScale::Logarithmic;
    log.set_y_range(0.1, 1000.0);
    log.config_mut().line_offsets = Some(LineOffsets {
        series: vec!["low".into(), "high".into()],
        step_y: 1.5,
        step_x: 0.5,
    });
    let series = [
        line(
            "low",
            2.0,
            LineStylePreset::Solid,
            LineInterpolation::Linear,
        ),
        line(
            "high",
            2.0,
            LineStylePreset::Solid,
            LineInterpolation::Linear,
        ),
    ];
    for scale in [1.0, 2.0] {
        for subject in ["low", "high"] {
            let painted = assert_member_pick_matches_paint(
                &mut renderer,
                &log,
                &series,
                subject,
                scale,
                &format!("log offset member {subject}"),
                STRICT,
            );
            assert!(painted > 30, "{subject} at {scale}x: {painted}");
        }
    }
}

/// Offset spectra (#50) whose members overlap: each member's opaque baseline
/// fill covers part of the member below it, line included. Every pixel a
/// member paints and the user can see picks that member, and a covered one
/// picks the member covering it, at 1x and 2x: the pick answers with what is
/// painted under the cursor (D7), across the member panels' composite order.
#[test]
fn overlapping_offset_members_pick_what_is_painted_on_top() {
    use renderer::data_config::{LineFill, LineInterpolation, LineOffsets};
    let mut renderer = renderer();
    let xs = vec![1.0, 2.5, 4.0, 5.5, 7.0, 8.5];
    let n = xs.len();
    upload_xy(&mut renderer, xs, vec![1.0, 1.9, 1.2, 2.0, 1.3, 1.6]);
    renderer
        .add_columns(&[
            (
                "nan_x",
                &col(vec![f64::NAN; n]) as &dyn renderer::ColumnSource,
            ),
            (
                "nan_y",
                &col(vec![f64::NAN; n]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("upload");
    // Far apart in every pair, so a fill painted over its neighbour's
    // differs by as much ink as one painted over the background.
    let fills = [
        Color::new(0.1, 0.2, 0.85, 1.0),
        Color::new(0.95, 0.85, 0.15, 1.0),
        Color::new(0.15, 0.75, 0.2, 1.0),
    ];
    let series: Vec<SeriesConfig> = ["m0", "m1", "m2"]
        .iter()
        .zip(fills)
        .map(|(id, fill)| {
            filled_line(
                id,
                Some(LineFill::ToBaseline(0.6)),
                LineInterpolation::Linear,
                fill,
            )
        })
        .collect();
    let mut chart = bare_chart();
    chart.set_x_range(0.0, 9.5);
    chart.set_y_range(0.0, 3.2);
    // Half a unit apart: member k's line (1.0..2.0 + k / 2) runs inside
    // member k + 1's fill (1.1 + k / 2 up to its line) wherever the trace
    // rises less than half a unit over its neighbour's.
    chart.config_mut().line_offsets = Some(LineOffsets {
        series: vec!["m0".into(), "m1".into(), "m2".into()],
        step_y: 0.5,
        step_x: 0.0,
    });
    for scale in [1.0, 2.0] {
        for subject in ["m0", "m1", "m2"] {
            let painted = assert_member_pick_matches_paint(
                &mut renderer,
                &chart,
                &series,
                subject,
                scale,
                &format!("overlapping member {subject}"),
                FILL_EDGE,
            );
            assert!(painted > 30, "{subject} at {scale}x: {painted}");
        }
    }
}
