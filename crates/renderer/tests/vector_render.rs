//! End-to-end verification of the vector field (quiver) render path through
//! the public renderer API: the headless `export_panel_rgba`, the GPU pickers,
//! the selection highlights and the series extent engine, and of the
//! streamline arrowheads the vector glyph draws along a line.
//!
//! House pattern (mirrors `heatmap_render.rs`): one process-wide device, a
//! renderer per test, and assertions that name the pixel a data coordinate
//! lands on, so a wrong projection, pivot or scale fails rather than passes.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::{AxisOptions, AxisScale, PickedPointRef, PickedPointsConfig};
use renderer::data::Column;
use renderer::data_config::{
    ContourConfig, DataLineStyleConfig, FieldFillConfig, FillMode, GridLayout, LineInterpolation,
    MatrixOrientation, MatrixRef, PointColorMap, Shading, StreamlineArrows, VectorColorMap,
    VectorComponents, VectorPivot, VectorScale, VectorStyle,
};
use renderer::gpu_errorbar::GpuSeriesExtent;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, Color, ColorBarOptions, ColorMap, DataRenderType, DataSelectionsConfig, FiggyError,
    GpuPickRequest, OverlayLayer, PickedData, PickedDataRef, RasterImage, Renderer, RendererDevice,
    SeriesConfig, Subplot, display_config_for_surface,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;

const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const GREEN: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
const BLUE: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let finite = |acc: f64, v: f64| if v.is_finite() { acc.min(v) } else { acc };
    let min = data.iter().copied().fold(f64::INFINITY, finite);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, |acc, v| {
        if v.is_finite() { acc.max(v) } else { acc }
    });
    Column { data, min, max }
}

fn shared_device() -> Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
    static DEVICE: OnceLock<Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let (device, queue) = test_gpu::device();
            Some((Arc::new(device), Arc::new(queue)))
        })
        .as_ref()
        .map(|(device, queue)| (Arc::clone(device), Arc::clone(queue)))
}

fn try_renderer() -> Option<Renderer> {
    let (device, queue) = shared_device()?;
    Some(
        Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .expect("renderer init"),
    )
}

/// Grid and legend off: the only non-vector ink is the black deco frame.
fn bare_chart() -> Chart {
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
    chart.set_x_range(0.0, 4.0);
    chart.set_y_range(0.0, 4.0);
    chart
}

fn style(scale: VectorScale) -> VectorStyle {
    VectorStyle {
        color: RED,
        width_px: 4.0,
        scale,
        pivot: VectorPivot::Tail,
        head_length_px: 10.0,
        head_width_px: 10.0,
        min_length_px: 0.0,
        color_map: None,
    }
}

fn pixels(px_per_unit: f64) -> VectorStyle {
    style(VectorScale::Pixels { px_per_unit })
}

fn data(factor: f64) -> VectorStyle {
    style(VectorScale::Data { factor })
}

fn points(id: &str, style: VectorStyle) -> SeriesConfig {
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
            style,
        },
    }
}

fn matrix(
    columns: &[&str],
    orientation: &MatrixOrientation,
    grid_layout: &GridLayout,
) -> MatrixRef {
    MatrixRef {
        columns: columns.iter().map(|id| (*id).to_string()).collect(),
        orientation: orientation.clone(),
        grid_layout: grid_layout.clone(),
    }
}

fn grid(
    u: &[&str],
    v: &[&str],
    orientation: &MatrixOrientation,
    grid_layout: &GridLayout,
    style: VectorStyle,
) -> SeriesConfig {
    SeriesConfig {
        series_id: "quiver".into(),
        source_id: None,
        label: None,
        x_column: "gx".into(),
        y_column: "gy".into(),
        render_type: DataRenderType::Vector {
            components: VectorComponents::Grid {
                u: matrix(u, orientation, grid_layout),
                v: matrix(v, orientation, grid_layout),
            },
            style,
        },
    }
}

/// One anchor per entry with its (u, v) components.
fn point_fixture(renderer: &mut Renderer, anchors: &[(f64, f64)], uv: &[(f64, f64)]) {
    let col = |pick: fn(&(f64, f64)) -> f64, rows: &[(f64, f64)]| {
        col_f64(rows.iter().map(pick).collect())
    };
    renderer
        .add_columns(&[
            ("x", &col(|p| p.0, anchors) as &dyn renderer::ColumnSource),
            ("y", &col(|p| p.1, anchors) as &dyn renderer::ColumnSource),
            ("u", &col(|p| p.0, uv) as &dyn renderer::ColumnSource),
            ("v", &col(|p| p.1, uv) as &dyn renderer::ColumnSource),
        ])
        .expect("point upload");
}

/// A 3 x 2 cell grid: x edges [0,1,2,3], y edges [0,1,2] (or the matching
/// centres), every cell carrying the same (20, 0) vector. The `u` and `v`
/// grids are declared in both orientations: `ux*` / `vx*` run along x
/// (3 columns of 2), `uy*` / `vy*` along y (2 columns of 3).
fn grid_fixture(renderer: &mut Renderer, layout: &GridLayout) {
    let (gx, gy) = match layout {
        GridLayout::Edges => (vec![0.0, 1.0, 2.0, 3.0], vec![0.0, 1.0, 2.0]),
        GridLayout::Centers => (vec![0.5, 1.5, 2.5], vec![0.5, 1.5]),
    };
    let two_u = col_f64(vec![20.0, 20.0]);
    let two_v = col_f64(vec![0.0, 0.0]);
    let three_u = col_f64(vec![20.0, 20.0, 20.0]);
    let three_v = col_f64(vec![0.0, 0.0, 0.0]);
    renderer
        .add_columns(&[
            ("gx", &col_f64(gx) as &dyn renderer::ColumnSource),
            ("gy", &col_f64(gy) as &dyn renderer::ColumnSource),
            ("ux0", &two_u as &dyn renderer::ColumnSource),
            ("ux1", &two_u as &dyn renderer::ColumnSource),
            ("ux2", &two_u as &dyn renderer::ColumnSource),
            ("vx0", &two_v as &dyn renderer::ColumnSource),
            ("vx1", &two_v as &dyn renderer::ColumnSource),
            ("vx2", &two_v as &dyn renderer::ColumnSource),
            ("uy0", &three_u as &dyn renderer::ColumnSource),
            ("uy1", &three_u as &dyn renderer::ColumnSource),
            ("vy0", &three_v as &dyn renderer::ColumnSource),
            ("vy1", &three_v as &dyn renderer::ColumnSource),
        ])
        .expect("grid upload");
}

fn grid_chart() -> Chart {
    let mut chart = bare_chart();
    chart.set_x_range(0.0, 3.0);
    chart.set_y_range(0.0, 2.0);
    chart
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

fn is_red(p: &[u8]) -> bool {
    p[3] > 180 && p[0] > 180 && p[1] < 80 && p[2] < 80
}

fn is_green(p: &[u8]) -> bool {
    p[3] > 180 && p[1] > 180 && p[0] < 80 && p[2] < 80
}

fn is_selection(p: &[u8]) -> bool {
    p[3] > 180 && p[0] > 180 && p[2] > 180 && p[1] < 80
}

fn count(img: &RasterImage, pred: fn(&[u8]) -> bool) -> usize {
    img.rgba.chunks_exact(4).filter(|p| pred(p)).count()
}

fn axis_t(axis: &AxisOptions, v: f64) -> f64 {
    let t = match axis.scale {
        AxisScale::Logarithmic => (v.ln() - axis.min.ln()) / (axis.max.ln() - axis.min.ln()),
        AxisScale::Linear => (v - axis.min) / (axis.max - axis.min),
    };
    if axis.inverted { 1.0 - t } else { t }
}

/// Data (x, y) → panel pixel at export scale `scale`, through the same data
/// area, axis scale and direction the renderer draws with.
fn data_to_px_scaled(chart: &Chart, x: f64, y: f64, scale: f32) -> (f64, f64) {
    let cfg = chart.config().scaled(scale);
    let da = cfg.data_area().expect("data area");
    let tx = axis_t(&cfg.bottom_x, x);
    let ty = axis_t(&cfg.left_y, y);
    (
        da.x as f64 + tx * da.width as f64,
        (da.y + da.height) as f64 - ty * da.height as f64,
    )
}

fn data_to_px(chart: &Chart, x: f64, y: f64) -> (f64, f64) {
    data_to_px_scaled(chart, x, y, 1.0)
}

fn at(img: &RasterImage, (x, y): (f64, f64), dx: f64, dy: f64) -> &[u8] {
    pixel(img, (x + dx).round() as u32, (y + dy).round() as u32)
}

fn assert_red(img: &RasterImage, origin: (f64, f64), dx: f64, dy: f64, what: &str) {
    let p = at(img, origin, dx, dy);
    assert!(
        is_red(p),
        "{what}: expected arrow ink at {origin:?} + ({dx}, {dy}), got {p:?}"
    );
}

fn assert_clear(img: &RasterImage, origin: (f64, f64), dx: f64, dy: f64, what: &str) {
    let p = at(img, origin, dx, dy);
    assert!(
        !is_red(p),
        "{what}: expected no arrow ink at {origin:?} + ({dx}, {dy}), got {p:?}"
    );
}

fn export(renderer: &mut Renderer, chart: &Chart, series: &[SeriesConfig]) -> RasterImage {
    renderer.export_panel_rgba(chart, series, 1.0).unwrap()
}

// ── Pixel mode ───────────────────────────────────────────────────────────────

/// `Pixels { px_per_unit }` draws `px_per_unit * |(u, v)|` pixels from the
/// anchor, so a (40, 0) vector is a 40 px arrow pointing right: shaft ink
/// along it, head ink near the tip, nothing before the tail or past the tip.
/// At 2x export every pixel size doubles with the image.
#[test]
fn pixel_scaled_arrows_have_their_length_in_pixels_at_every_export_scale() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(2.0, 2.0)], &[(40.0, 0.0)]);
    let chart = bare_chart();
    let series = [points("arrows", pixels(1.0))];

    let img = export(&mut renderer, &chart, &series);
    let a = data_to_px(&chart, 2.0, 2.0);
    assert_red(&img, a, 5.0, 0.0, "shaft start");
    assert_red(&img, a, 20.0, 0.0, "shaft middle");
    assert_red(&img, a, 36.0, 0.0, "head");
    assert_clear(&img, a, -5.0, 0.0, "before the tail");
    assert_clear(&img, a, 45.0, 0.0, "past the tip");
    assert_red(&img, a, 20.0, 1.0, "inside the 4 px shaft");
    assert_red(&img, a, 20.0, -1.0, "inside the 4 px shaft");
    assert_clear(&img, a, 20.0, 5.0, "beside the shaft");
    assert_clear(&img, a, 20.0, -5.0, "beside the shaft");
    // The head is wider than the shaft: 10 px at its base, 30 px from the tail.
    assert_red(&img, a, 31.0, 3.0, "head base");
    assert_clear(&img, a, 31.0, 7.0, "beside the head");

    let img2 = renderer.export_panel_rgba(&chart, &series, 2.0).unwrap();
    assert_eq!((img2.width, img2.height), (WIDTH * 2, HEIGHT * 2));
    let a2 = data_to_px_scaled(&chart, 2.0, 2.0, 2.0);
    assert_red(&img2, a2, 40.0, 0.0, "2x shaft middle");
    assert_red(&img2, a2, 72.0, 0.0, "2x head");
    assert_clear(&img2, a2, -8.0, 0.0, "2x before the tail");
    assert_clear(&img2, a2, 90.0, 0.0, "2x past the tip");
    assert_red(&img2, a2, 40.0, 2.0, "2x 8 px shaft");
    assert_red(&img2, a2, 40.0, -2.0, "2x 8 px shaft");
    assert_clear(&img2, a2, 40.0, 9.0, "2x beside the shaft");
    assert_clear(&img2, a2, 40.0, -9.0, "2x beside the shaft");
}

/// In pixel mode the direction follows the axes' pixel-per-unit ratio: on a
/// panel whose x axis has more pixels per unit than its y axis, a (1, 1)
/// vector leans towards x rather than drawing a 45° screen diagonal, and the
/// length is still `px_per_unit * sqrt(2)`.
#[test]
fn pixel_scaled_direction_follows_the_axis_jacobian() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(2.0, 2.0)], &[(1.0, 1.0)]);
    let mut chart = bare_chart();
    // x: 4 units over the data width, y: 16 units over the data height.
    chart.set_y_range(0.0, 16.0);
    let series = [points("arrows", pixels(40.0))];
    let img = export(&mut renderer, &chart, &series);

    let cfg = chart.config();
    let da = cfg.data_area().unwrap();
    let kx = da.width as f64 / 4.0;
    let ky = da.height as f64 / 16.0;
    let (dx, dy) = (kx, ky);
    let norm = (dx * dx + dy * dy).sqrt();
    let len = 40.0 * 2f64.sqrt();
    let dir = (dx / norm, -dy / norm);
    let a = data_to_px(&chart, 2.0, 2.0);
    let along = |s: f64| (a.0 + dir.0 * s, a.1 + dir.1 * s);
    assert_red(&img, along(len * 0.5), 0.0, 0.0, "leaning shaft middle");
    assert_red(&img, along(len - 4.0), 0.0, 0.0, "leaning head");
    assert_clear(&img, along(len + 6.0), 0.0, 0.0, "past the leaning tip");
    // The 45° screen diagonal of the same length ends somewhere else.
    let diagonal = (
        a.0 + len * std::f64::consts::FRAC_1_SQRT_2,
        a.1 - len * std::f64::consts::FRAC_1_SQRT_2,
    );
    assert_clear(&img, diagonal, 0.0, 0.0, "screen diagonal tip");
}

// ── Data mode ────────────────────────────────────────────────────────────────

/// `Data { factor }` places the tip at `anchor + factor * (u, v)` in data
/// space, through the axis transform: the arrow follows an inverted axis and
/// a logarithmic one.
#[test]
fn data_scaled_arrows_end_at_the_anchor_plus_component_through_the_axes() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(1.0, 1.0)], &[(1.0, 2.0)]);

    // Linear: tip at (2, 3).
    let chart = bare_chart();
    let series = [points("arrows", data(1.0))];
    let img = export(&mut renderer, &chart, &series);
    let tail = data_to_px(&chart, 1.0, 1.0);
    let tip = data_to_px(&chart, 2.0, 3.0);
    let lerp = |t: f64| (tail.0 + (tip.0 - tail.0) * t, tail.1 + (tip.1 - tail.1) * t);
    assert_red(&img, lerp(0.5), 0.0, 0.0, "midpoint");
    assert_red(&img, lerp(0.95), 0.0, 0.0, "head");
    assert_clear(&img, lerp(1.06), 0.0, 0.0, "past the tip");
    assert_clear(&img, lerp(-0.06), 0.0, 0.0, "before the tail");

    // Half the factor: tip at (1.5, 2).
    let half = [points("arrows", data(0.5))];
    let img = export(&mut renderer, &chart, &half);
    let tip = data_to_px(&chart, 1.5, 2.0);
    let lerp = |t: f64| (tail.0 + (tip.0 - tail.0) * t, tail.1 + (tip.1 - tail.1) * t);
    assert_red(&img, lerp(0.5), 0.0, 0.0, "half-factor midpoint");
    assert_clear(&img, lerp(1.12), 0.0, 0.0, "half-factor past the tip");

    // Inverted y: the tip moves down the screen with the axis.
    let mut inverted = bare_chart();
    inverted.config_mut().left_y.inverted = true;
    inverted.config_mut().right_y.inverted = true;
    let img = export(&mut renderer, &inverted, &series);
    let tail = data_to_px(&inverted, 1.0, 1.0);
    let tip = data_to_px(&inverted, 2.0, 3.0);
    assert!(tip.1 > tail.1, "an inverted y axis puts y = 3 below y = 1");
    let lerp = |t: f64| (tail.0 + (tip.0 - tail.0) * t, tail.1 + (tip.1 - tail.1) * t);
    assert_red(&img, lerp(0.5), 0.0, 0.0, "inverted midpoint");
    assert_red(&img, lerp(0.95), 0.0, 0.0, "inverted head");
    assert_clear(&img, lerp(1.06), 0.0, 0.0, "inverted past the tip");

    // Logarithmic y: the tip of (1, 2) + (1, 18) is at y = 20 on a 1..100
    // axis, and the arrow is still straight on screen.
    renderer.remove_column("y").unwrap();
    renderer.remove_column("v").unwrap();
    renderer.add_column("y", &col_f64(vec![2.0])).unwrap();
    renderer.add_column("v", &col_f64(vec![18.0])).unwrap();
    let mut log = bare_chart();
    log.config_mut().left_y.scale = AxisScale::Logarithmic;
    log.config_mut().right_y.scale = AxisScale::Logarithmic;
    log.set_y_range(1.0, 100.0);
    let img = export(&mut renderer, &log, &series);
    let tail = data_to_px(&log, 1.0, 2.0);
    let tip = data_to_px(&log, 2.0, 20.0);
    let lerp = |t: f64| (tail.0 + (tip.0 - tail.0) * t, tail.1 + (tip.1 - tail.1) * t);
    assert_red(&img, lerp(0.5), 0.0, 0.0, "log midpoint");
    assert_red(&img, lerp(0.95), 0.0, 0.0, "log head");
    assert_clear(&img, lerp(1.06), 0.0, 0.0, "log past the tip");
    // A linear reading of the same tip would land far lower on the screen.
    let linear_tip = {
        let da = log.config().data_area().unwrap();
        (
            tip.0,
            (da.y + da.height) as f64 - (20.0 - 1.0) / 99.0 * da.height as f64,
        )
    };
    assert_clear(
        &img,
        linear_tip,
        0.0,
        0.0,
        "linear misreading of the log tip",
    );
}

// ── Glyph ────────────────────────────────────────────────────────────────────

/// The pivot moves the arrow about its anchor: the tail, the middle or the
/// tip sits on the anchor.
#[test]
fn pivots_place_the_tail_middle_or_tip_on_the_anchor() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(2.0, 2.0)], &[(40.0, 0.0)]);
    let chart = bare_chart();
    let a = data_to_px(&chart, 2.0, 2.0);
    let with_pivot = |pivot| {
        let mut style = pixels(1.0);
        style.pivot = pivot;
        [points("arrows", style)]
    };

    let tail = export(&mut renderer, &chart, &with_pivot(VectorPivot::Tail));
    assert_red(&tail, a, 15.0, 0.0, "tail pivot: ahead");
    assert_clear(&tail, a, -15.0, 0.0, "tail pivot: behind");

    let middle = export(&mut renderer, &chart, &with_pivot(VectorPivot::Middle));
    assert_red(&middle, a, 15.0, 0.0, "middle pivot: ahead");
    assert_red(&middle, a, -15.0, 0.0, "middle pivot: behind");
    assert_clear(&middle, a, 25.0, 0.0, "middle pivot: past the tip");
    assert_clear(&middle, a, -25.0, 0.0, "middle pivot: before the tail");

    let tip = export(&mut renderer, &chart, &with_pivot(VectorPivot::Tip));
    assert_red(&tip, a, -15.0, 0.0, "tip pivot: behind");
    assert_clear(&tip, a, 15.0, 0.0, "tip pivot: ahead");
}

/// A short arrow scales its head down to its own length instead of growing a
/// head longer than the shaft; `min_length_px` hides anything shorter; a zero
/// vector draws nothing.
#[test]
fn short_arrows_scale_their_head_and_min_length_hides_them() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(2.0, 2.0)], &[(10.0, 0.0)]);
    let chart = bare_chart();
    let a = data_to_px(&chart, 2.0, 2.0);
    let mut style = pixels(1.0);
    style.width_px = 2.0;
    style.head_length_px = 20.0;
    style.head_width_px = 20.0;

    let img = export(&mut renderer, &chart, &[points("arrows", style.clone())]);
    // The whole 10 px arrow is head, scaled to 10 x 10: 4 px half-width 2 px
    // in from the base, never the unscaled 10 px.
    assert_red(&img, a, 2.0, 3.0, "scaled head base");
    assert_red(&img, a, 2.0, -3.0, "scaled head base");
    assert_clear(&img, a, 2.0, 7.0, "unscaled head width");
    assert_clear(&img, a, 2.0, -7.0, "unscaled head width");
    assert_clear(&img, a, 13.0, 0.0, "past the tip");

    style.min_length_px = 15.0;
    let hidden = export(&mut renderer, &chart, &[points("arrows", style.clone())]);
    assert_eq!(count(&hidden, is_red), 0, "a 10 px arrow under min 15 px");

    renderer.remove_column("u").unwrap();
    renderer.add_column("u", &col_f64(vec![0.0])).unwrap();
    style.min_length_px = 0.0;
    let zero = export(&mut renderer, &chart, &[points("arrows", style)]);
    assert_eq!(count(&zero, is_red), 0, "a zero vector draws nothing");
}

/// A non-finite component or anchor hides that arrow and nothing else.
#[test]
fn non_finite_samples_are_not_drawn() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(
        &mut renderer,
        &[(1.0, 2.0), (2.0, 2.0), (f64::NAN, 2.0), (3.0, 2.0)],
        &[
            (f64::NAN, 0.0),
            (40.0, f64::INFINITY),
            (40.0, 0.0),
            (40.0, 0.0),
        ],
    );
    let chart = bare_chart();
    let img = export(&mut renderer, &chart, &[points("arrows", pixels(1.0))]);
    assert_clear(&img, data_to_px(&chart, 1.0, 2.0), 15.0, 0.0, "NaN u");
    assert_clear(&img, data_to_px(&chart, 2.0, 2.0), 15.0, 0.0, "infinite v");
    assert_red(
        &img,
        data_to_px(&chart, 3.0, 2.0),
        15.0,
        0.0,
        "the finite arrow",
    );
    // Only the last arrow: 40 x 4 px shaft plus a 10 x 10 head, under 300 px.
    let red = count(&img, is_red);
    assert!((100..300).contains(&red), "{red} red pixels");
}

// ── Colour map ───────────────────────────────────────────────────────────────

fn two_stop_colorbar(scale: AxisScale, min: f64, max: f64) -> ColorBarOptions {
    let mut bar = renderer::default::default_colorbar_options();
    bar.visible = false;
    bar.colormap = ColorMap::Custom {
        stops: vec![RED, GREEN],
    };
    bar.nan_color = BLUE;
    bar.axis.scale = scale;
    bar.axis.min = min;
    bar.axis.max = max;
    bar
}

fn to8(v: f32) -> i32 {
    (v.clamp(0.0, 1.0) * 255.0).round() as i32
}

fn assert_colour_at(img: &RasterImage, px: (f64, f64), want: Color, what: &str) {
    let p = at(img, px, 0.0, 0.0);
    let got = [p[0] as i32, p[1] as i32, p[2] as i32, p[3] as i32];
    let expected = [to8(want.r), to8(want.g), to8(want.b), to8(want.a)];
    assert!(
        got.iter().zip(expected).all(|(g, e)| (*g - e).abs() <= 2),
        "{what} at {px:?}: got {got:?}, expected {expected:?}"
    );
}

/// A colour-mapped point vector takes its colour from the colourbar at the
/// value column, like a colour-mapped marker: the stops at the ends, the ramp
/// in between, `nan_color` for a non-finite value, and on a logarithmic bar
/// `nan_color` for a non-positive one.
#[test]
fn colour_mapped_arrows_follow_the_colourbar_ramp() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(
        &mut renderer,
        &[(0.5, 2.0), (1.5, 2.0), (2.5, 2.0), (3.5, 2.0)],
        &[(0.0, 40.0); 4],
    );
    renderer
        .add_column("c", &col_f64(vec![0.0, 5.0, 10.0, f64::NAN]))
        .unwrap();
    let mut style = pixels(1.0);
    style.width_px = 6.0;
    style.color_map = Some(VectorColorMap::Column("c".into()));
    let series = [points("arrows", style)];

    let mut chart = bare_chart();
    let bar = two_stop_colorbar(AxisScale::Linear, 0.0, 10.0);
    chart.config_mut().colorbar = Some(bar.clone());
    let img = export(&mut renderer, &chart, &series);
    let shaft = |chart: &Chart, x: f64| {
        let a = data_to_px(chart, x, 2.0);
        (a.0, a.1 - 15.0)
    };
    assert_colour_at(&img, shaft(&chart, 0.5), bar.color_for_z(0.0), "z = min");
    assert_colour_at(&img, shaft(&chart, 1.5), bar.color_for_z(5.0), "z = mid");
    assert_colour_at(&img, shaft(&chart, 2.5), bar.color_for_z(10.0), "z = max");
    assert_colour_at(&img, shaft(&chart, 3.5), BLUE, "z = NaN");

    renderer.remove_column("c").unwrap();
    renderer
        .add_column("c", &col_f64(vec![0.0, 10.0, 100.0, -3.0]))
        .unwrap();
    let bar = two_stop_colorbar(AxisScale::Logarithmic, 1.0, 100.0);
    chart.config_mut().colorbar = Some(bar.clone());
    let img = export(&mut renderer, &chart, &series);
    assert_colour_at(&img, shaft(&chart, 0.5), BLUE, "log z = 0");
    assert_colour_at(&img, shaft(&chart, 1.5), bar.color_for_z(10.0), "log mid");
    assert_colour_at(&img, shaft(&chart, 2.5), bar.color_for_z(100.0), "log max");
    assert_colour_at(&img, shaft(&chart, 3.5), BLUE, "log negative");
}

/// A grid vector's colour grid is read like its component grids, cell by cell.
#[test]
fn colour_mapped_grid_arrows_read_their_colour_grid() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    grid_fixture(&mut renderer, &GridLayout::Edges);
    renderer
        .add_columns(&[
            (
                "c0",
                &col_f64(vec![0.0, 10.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "c1",
                &col_f64(vec![5.0, f64::NAN]) as &dyn renderer::ColumnSource,
            ),
            (
                "c2",
                &col_f64(vec![10.0, 0.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .unwrap();
    let mut chart = grid_chart();
    let bar = two_stop_colorbar(AxisScale::Linear, 0.0, 10.0);
    chart.config_mut().colorbar = Some(bar.clone());
    let mut style = pixels(1.0);
    style.width_px = 6.0;
    style.color_map = Some(VectorColorMap::Grid(matrix(
        &["c0", "c1", "c2"],
        &MatrixOrientation::ColumnsAreX,
        &GridLayout::Edges,
    )));
    let series = [grid(
        &["ux0", "ux1", "ux2"],
        &["vx0", "vx1", "vx2"],
        &MatrixOrientation::ColumnsAreX,
        &GridLayout::Edges,
        style,
    )];
    let img = export(&mut renderer, &chart, &series);
    let shaft = |x: f64, y: f64| {
        let a = data_to_px(&chart, x, y);
        (a.0 + 8.0, a.1)
    };
    assert_colour_at(&img, shaft(0.5, 0.5), bar.color_for_z(0.0), "cell (0,0)");
    assert_colour_at(&img, shaft(0.5, 1.5), bar.color_for_z(10.0), "cell (0,1)");
    assert_colour_at(&img, shaft(1.5, 0.5), bar.color_for_z(5.0), "cell (1,0)");
    assert_colour_at(&img, shaft(1.5, 1.5), BLUE, "cell (1,1) NaN");
    assert_colour_at(&img, shaft(2.5, 0.5), bar.color_for_z(10.0), "cell (2,0)");
    assert_colour_at(&img, shaft(2.5, 1.5), bar.color_for_z(0.0), "cell (2,1)");
}

// ── Grid form ────────────────────────────────────────────────────────────────

/// Every cell of a 3 x 2 grid gets its arrow, anchored on the cell centre
/// under `Edges` and on the coordinate under `Centers`, in both orientations.
#[test]
fn grid_arrows_anchor_on_cell_centres_in_every_layout_and_orientation() {
    for layout in [&GridLayout::Edges, &GridLayout::Centers] {
        let Some(mut renderer) = try_renderer() else {
            return;
        };
        grid_fixture(&mut renderer, layout);
        let chart = grid_chart();
        for (u, v, orientation) in [
            (
                ["ux0", "ux1", "ux2"].as_slice(),
                ["vx0", "vx1", "vx2"].as_slice(),
                &MatrixOrientation::ColumnsAreX,
            ),
            (
                ["uy0", "uy1"].as_slice(),
                ["vy0", "vy1"].as_slice(),
                &MatrixOrientation::ColumnsAreY,
            ),
        ] {
            let series = [grid(u, v, orientation, layout, pixels(1.0))];
            let img = export(&mut renderer, &chart, &series);
            let what = format!("{layout:?} / {orientation:?}");
            for i in 0..3 {
                for j in 0..2 {
                    let centre = data_to_px(&chart, i as f64 + 0.5, j as f64 + 0.5);
                    assert_red(&img, centre, 6.0, 0.0, &format!("{what} cell ({i},{j})"));
                    assert_clear(&img, centre, -6.0, 0.0, &format!("{what} before ({i},{j})"));
                }
            }
            // No arrow sits on a cell edge.
            for i in 0..=3 {
                let edge = data_to_px(&chart, i as f64, 0.5);
                assert_clear(&img, edge, 6.0, 0.0, &format!("{what} edge {i}"));
            }
            // Six arrows of 20 x 4 px plus heads: more than 300, under 900.
            let red = count(&img, is_red);
            assert!((300..900).contains(&red), "{what}: {red} red pixels");
        }
    }
}

/// Under `Edges` the cell centre is the data-space midpoint of the two edges,
/// which on a logarithmic axis is not the screen midpoint.
#[test]
fn edge_grid_centres_are_data_space_midpoints_on_a_log_axis() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_columns(&[
            (
                "gx",
                &col_f64(vec![1.0, 10.0, 100.0, 1000.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &col_f64(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "ux0",
                &col_f64(vec![20.0, 20.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "ux1",
                &col_f64(vec![20.0, 20.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "ux2",
                &col_f64(vec![20.0, 20.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "vx0",
                &col_f64(vec![0.0, 0.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "vx1",
                &col_f64(vec![0.0, 0.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "vx2",
                &col_f64(vec![0.0, 0.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .unwrap();
    let mut chart = bare_chart();
    chart.config_mut().bottom_x.scale = AxisScale::Logarithmic;
    chart.config_mut().top_x.scale = AxisScale::Logarithmic;
    chart.set_x_range(1.0, 1000.0);
    chart.set_y_range(0.0, 2.0);
    let series = [grid(
        &["ux0", "ux1", "ux2"],
        &["vx0", "vx1", "vx2"],
        &MatrixOrientation::ColumnsAreX,
        &GridLayout::Edges,
        pixels(1.0),
    )];
    let img = export(&mut renderer, &chart, &series);
    for (midpoint, geometric) in [
        (5.5, 10f64.sqrt()),
        (55.0, 1000f64.sqrt()),
        (550.0, 100_000f64.sqrt()),
    ] {
        assert_red(
            &img,
            data_to_px(&chart, midpoint, 0.5),
            6.0,
            0.0,
            "data-space midpoint",
        );
        assert_clear(
            &img,
            data_to_px(&chart, geometric, 0.5),
            6.0,
            0.0,
            "screen midpoint",
        );
    }
}

/// The drawn extent is the smallest the component (and colour) grids agree
/// on, and a declaration asking for more is reported as truncated.
#[test]
fn grid_extent_is_the_smallest_common_grid_and_reports_truncation() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    grid_fixture(&mut renderer, &GridLayout::Edges);
    renderer.add_column("v_short", &col_f64(vec![0.0])).unwrap();
    let chart = grid_chart();

    let whole = renderer
        .register_chart(
            chart.config().clone(),
            vec![grid(
                &["ux0", "ux1", "ux2"],
                &["vx0", "vx1", "vx2"],
                &MatrixOrientation::ColumnsAreX,
                &GridLayout::Edges,
                pixels(1.0),
            )],
        )
        .unwrap();
    let info = renderer.series_draw_info(whole, "quiver").unwrap();
    assert_eq!((info.cols, info.rows), (Some(3), Some(2)));
    assert_eq!(info.drawn_count, 6);
    assert!(!info.truncated);

    let short_v = renderer
        .register_chart(
            chart.config().clone(),
            vec![grid(
                &["ux0", "ux1", "ux2"],
                &["vx0", "vx1", "v_short"],
                &MatrixOrientation::ColumnsAreX,
                &GridLayout::Edges,
                pixels(1.0),
            )],
        )
        .unwrap();
    let info = renderer.series_draw_info(short_v, "quiver").unwrap();
    assert_eq!(
        (info.cols, info.rows),
        (Some(3), Some(1)),
        "the shortest v column decides the rows"
    );
    assert_eq!(info.drawn_count, 3);
    assert!(info.truncated);

    let fewer_v = renderer
        .register_chart(
            chart.config().clone(),
            vec![grid(
                &["ux0", "ux1", "ux2"],
                &["vx0", "vx1"],
                &MatrixOrientation::ColumnsAreX,
                &GridLayout::Edges,
                pixels(1.0),
            )],
        )
        .unwrap();
    let info = renderer.series_draw_info(fewer_v, "quiver").unwrap();
    assert_eq!((info.cols, info.rows), (Some(2), Some(2)));
    assert!(info.truncated, "the third u column has no v");

    // The image agrees with the report: only the first row of arrows.
    let series = renderer.chart_series(short_v).unwrap().to_vec();
    let img = export(&mut renderer, &chart, &series);
    assert_red(&img, data_to_px(&chart, 0.5, 0.5), 6.0, 0.0, "row 0");
    assert_clear(&img, data_to_px(&chart, 0.5, 1.5), 6.0, 0.0, "row 1");
}

/// A vector grid can share its coordinate columns and matrices' shape with a
/// heatmap and its contour: the field paints the cells, the isoline draws
/// its level, and the arrows draw over both.
#[test]
fn grid_arrows_draw_over_a_heatmap_and_contour_on_the_same_grid() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    grid_fixture(&mut renderer, &GridLayout::Edges);
    // z steps from 10 to 0 across x, so the 5 isoline runs vertically
    // through the middle column of cells.
    renderer
        .add_columns(&[
            (
                "z0",
                &col_f64(vec![10.0, 10.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "z1",
                &col_f64(vec![5.0, 5.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "z2",
                &col_f64(vec![0.0, 0.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .unwrap();
    let mut chart = grid_chart();
    chart.config_mut().colorbar = Some(two_stop_colorbar(AxisScale::Linear, 0.0, 10.0));
    let heatmap = SeriesConfig {
        series_id: "field".into(),
        source_id: None,
        label: None,
        x_column: "gx".into(),
        y_column: "gy".into(),
        render_type: DataRenderType::HeatmapContour {
            matrix: matrix(
                &["z0", "z1", "z2"],
                &MatrixOrientation::ColumnsAreX,
                &GridLayout::Edges,
            ),
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            },
            contour: ContourConfig {
                levels: vec![5.0],
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: BLUE,
                    line_width: 3.0,
                    fill: None,
                    fill_color: None,
                    interpolation: LineInterpolation::Linear,
                },
                per_level_color: None,
                labels: None,
            },
        },
    };
    let series = [
        heatmap,
        grid(
            &["ux0", "ux1", "ux2"],
            &["vx0", "vx1", "vx2"],
            &MatrixOrientation::ColumnsAreX,
            &GridLayout::Edges,
            pixels(1.0),
        ),
    ];
    let img = export(&mut renderer, &chart, &series);
    let centre = data_to_px(&chart, 0.5, 0.5);
    assert_red(&img, centre, 8.0, 0.0, "arrow over the field");
    let p = at(&img, centre, 8.0, 20.0);
    assert!(
        is_green(p),
        "the z = 10 cell under the arrow is the green stop: {p:?}"
    );
    let blue = img
        .rgba
        .chunks_exact(4)
        .filter(|p| p[2] > 180 && p[0] < 80 && p[1] < 80 && p[3] > 180)
        .count();
    assert!(
        blue > 100,
        "the isoline draws on the shared grid ({blue} px)"
    );
    assert_red(
        &img,
        data_to_px(&chart, 2.5, 1.5),
        8.0,
        0.0,
        "arrow over the z = 0 cell",
    );
}

// ── Picking and selection ────────────────────────────────────────────────────

fn pick_request(chart: &Chart, x: f64, y: f64, dx: f64, dy: f64, max: f32) -> GpuPickRequest {
    let (_, panel, scale) = display_config_for_surface(chart.config(), (WIDTH, HEIGHT));
    assert_eq!(scale, 1.0);
    let a = data_to_px(chart, x, y);
    GpuPickRequest {
        canvas_position_px: [(a.0 + dx) as f32, (a.1 + dy) as f32],
        display_panel_px: panel,
        display_scale: scale,
        max_distance_px: max,
    }
}

/// A point vector is picked on the arrow it draws, shaft and head: inside the
/// glyph the hit is at distance 0, outside it the distance counts from the
/// glyph's edge, and past the tip or `max_distance_px` nothing hits.
#[test]
fn point_vectors_pick_on_the_drawn_arrow() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(1.0, 1.0), (3.0, 3.0)], &[(40.0, 0.0); 2]);
    let chart = bare_chart();
    let id = renderer
        .register_chart(chart.config().clone(), vec![points("arrows", pixels(1.0))])
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    let pick = |renderer: &mut Renderer, request| {
        pollster::block_on(renderer.pick_chart(id, request).unwrap().resolve()).unwrap()
    };

    // A 40 px arrow along +x: a 4 px shaft to x = 30, then a 10 px head.
    let hit = pick(&mut renderer, pick_request(&chart, 3.0, 3.0, 4.0, 0.0, 0.0)).expect("hit");
    assert_eq!((hit.series_id.as_str(), hit.point_index), ("arrows", 1));
    assert_eq!(hit.distance_px, 0.0, "on the shaft");
    let shaft = pick(
        &mut renderer,
        pick_request(&chart, 1.0, 1.0, 25.0, 1.5, 0.0),
    )
    .expect("hit");
    assert_eq!((shaft.point_index, shaft.distance_px), (0, 0.0));
    let head = pick(
        &mut renderer,
        pick_request(&chart, 1.0, 1.0, 35.0, 2.0, 0.0),
    )
    .expect("hit");
    assert_eq!((head.point_index, head.distance_px), (0, 0.0));

    let miss = pick(&mut renderer, pick_request(&chart, 3.0, 3.0, 4.0, 3.0, 0.0));
    assert!(miss.is_none(), "1 px beside a 4 px shaft: {miss:?}");
    let near = pick(&mut renderer, pick_request(&chart, 1.0, 1.0, 4.0, 3.0, 3.0)).expect("hit");
    assert_eq!(near.point_index, 0);
    assert!(
        (near.distance_px - 1.0).abs() < 0.01,
        "{}",
        near.distance_px
    );

    let past_tip = pick(
        &mut renderer,
        pick_request(&chart, 1.0, 1.0, 43.0, 0.0, 0.0),
    );
    assert!(past_tip.is_none(), "3 px past the tip: {past_tip:?}");
    let behind = pick(
        &mut renderer,
        pick_request(&chart, 1.0, 1.0, -3.0, 0.0, 3.0),
    )
    .expect("hit");
    assert!(
        (behind.distance_px - 1.0).abs() < 0.01,
        "{}",
        behind.distance_px
    );
}

/// A grid vector picks the arrow it draws, answered as the cell the arrow is
/// drawn for: on the shaft or head it hits at distance 0, beside it the
/// distance counts from the glyph's edge, and in the cell off the arrow
/// nothing hits. Both orientations report the cell's x and y indices.
#[test]
fn grid_vectors_pick_the_arrow_drawn_for_each_cell() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    grid_fixture(&mut renderer, &GridLayout::Edges);
    let mut chart = grid_chart();
    chart.set_x_range(0.0, 4.0);
    let cell = |x_index, y_index, distance_px| PickedData::MatrixCell {
        source_id: None,
        series_id: "quiver".into(),
        x_index,
        y_index,
        distance_px,
    };
    for (u, v, orientation) in [
        (
            ["ux0", "ux1", "ux2"].as_slice(),
            ["vx0", "vx1", "vx2"].as_slice(),
            &MatrixOrientation::ColumnsAreX,
        ),
        (
            ["uy0", "uy1"].as_slice(),
            ["vy0", "vy1"].as_slice(),
            &MatrixOrientation::ColumnsAreY,
        ),
    ] {
        let id = renderer
            .register_chart(
                chart.config().clone(),
                vec![grid(u, v, orientation, &GridLayout::Edges, pixels(1.0))],
            )
            .unwrap();
        renderer.enable_gpu_picking().unwrap();
        let pick = |renderer: &mut Renderer, request| {
            pollster::block_on(renderer.pick_chart_data(id, request).unwrap().resolve()).unwrap()
        };
        let what = format!("{orientation:?}");

        assert_eq!(
            pick(&mut renderer, pick_request(&chart, 1.5, 0.5, 8.0, 0.0, 0.0)),
            Some(cell(1, 0, 0.0)),
            "{what}: on the shaft"
        );
        assert_eq!(
            pick(
                &mut renderer,
                pick_request(&chart, 2.5, 1.5, 15.0, 2.0, 0.0)
            ),
            Some(cell(2, 1, 0.0)),
            "{what}: on the head"
        );
        let beside = pick(&mut renderer, pick_request(&chart, 0.5, 1.5, 8.0, 3.0, 2.0));
        let Some(PickedData::MatrixCell {
            x_index: 0,
            y_index: 1,
            distance_px,
            ..
        }) = beside
        else {
            panic!("{what}: beside the shaft {beside:?}");
        };
        assert!((distance_px - 1.0).abs() < 0.01, "{what}: {distance_px}");
        assert_eq!(
            pick(&mut renderer, pick_request(&chart, 0.2, 1.8, 0.0, 0.0, 0.0)),
            None,
            "{what}: in the cell, off the arrow"
        );
        assert_eq!(
            pick(&mut renderer, pick_request(&chart, 3.5, 1.0, 0.0, 0.0, 0.0)),
            None,
            "{what}: outside the grid"
        );
        renderer.remove_chart(id).unwrap();
    }
}

/// The picked ring draws around a point vector's anchor at the glyph radius.
#[test]
fn picked_point_vector_draws_the_ring_at_its_anchor() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(1.0, 1.0), (3.0, 3.0)], &[(40.0, 0.0); 2]);
    let mut chart = bare_chart();
    chart.config_mut().picked_points = Some(PickedPointsConfig {
        visible: true,
        refs: vec![PickedPointRef {
            source_id: None,
            series_id: "arrows".into(),
            point_index: 1,
        }],
        ring_color: GREEN,
        ring_width_px: 3.0,
        radius_extra_px: 4.0,
    });
    let img = export(&mut renderer, &chart, &[points("arrows", pixels(1.0))]);
    let a = data_to_px(&chart, 3.0, 3.0);
    // Radius 5 + 4 extra: the ring's centreline is 9 px out.
    for (dx, dy) in [(0.0, -9.0), (0.0, 9.0), (-9.0, 0.0)] {
        let p = at(&img, a, dx, dy);
        assert!(is_green(p), "ring at ({dx}, {dy}): {p:?}");
    }
    let other = data_to_px(&chart, 1.0, 1.0);
    assert!(
        !is_green(at(&img, other, 0.0, -9.0)),
        "the unpicked anchor has no ring"
    );
    let green = count(&img, is_green);
    assert!((60..400).contains(&green), "{green} ring pixels");
}

/// A selected grid cell outlines that cell of the vector grid.
#[test]
fn selected_grid_vector_cell_outlines_its_cell() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    grid_fixture(&mut renderer, &GridLayout::Edges);
    let mut chart = grid_chart();
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::MatrixCell {
            source_id: None,
            series_id: "quiver".into(),
            x_index: 1,
            y_index: 0,
        }],
        highlight_color: Color::new(1.0, 0.0, 1.0, 1.0),
        outline_width_px: 4.0,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 0.0,
    });
    let series = [grid(
        &["ux0", "ux1", "ux2"],
        &["vx0", "vx1", "vx2"],
        &MatrixOrientation::ColumnsAreX,
        &GridLayout::Edges,
        pixels(1.0),
    )];
    let img = export(&mut renderer, &chart, &series);
    let (x0, y1) = data_to_px(&chart, 1.0, 0.0);
    let (x1, y0) = data_to_px(&chart, 2.0, 1.0);
    let (x0, y1, x1, y0) = (x0 as u32, y1 as u32, x1 as u32, y0 as u32);
    let mut selected = 0usize;
    let mut leaked = 0usize;
    for y in 0..img.height {
        for x in 0..img.width {
            if is_selection(pixel(&img, x, y)) {
                if x + 3 >= x0 && x <= x1 + 3 && y + 3 >= y0 && y <= y1 + 3 {
                    selected += 1;
                } else {
                    leaked += 1;
                }
            }
        }
    }
    assert!(
        selected > 100,
        "cell selection produced only {selected} pixels"
    );
    assert_eq!(leaked, 0, "cell selection leaked onto another cell");
    // The arrows still draw with the selection.
    assert_red(&img, data_to_px(&chart, 0.5, 0.5), 8.0, 0.0, "arrow");
}

// ── Layers ───────────────────────────────────────────────────────────────────

/// A vector series on an overlay layer scales by the overlay's Y axis: a
/// data-scaled (0, 4) vector from y = 4 reaches y = 8, the top of an overlay
/// axis running 0..8, halfway up the data area, where the base axis (0..4)
/// would have put the anchor itself at the top edge. A document export draws
/// every layer (a panel export keeps the base panel only).
#[test]
fn vector_series_scale_by_their_overlay_axis() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(2.0, 4.0)], &[(0.0, 4.0)]);
    let mut chart = bare_chart();
    let mut overlay = OverlayLayer::default();
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 8.0;
    overlay.y_axis.major_spacing = 2.0;
    overlay.series = vec!["arrows".into()];
    chart.config_mut().overlays.push(overlay);
    let series = [points("arrows", data(1.0))];
    let img = renderer.export_document_rgba(&chart, &series, 1.0).unwrap();
    assert_eq!((img.width, img.height), (WIDTH, HEIGHT));

    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let overlay_y = |y: f64| {
        let axis = &cfg.overlays[0].y_axis;
        (da.y + da.height) as f64 - axis_t(axis, y) * da.height as f64
    };
    let x = da.x as f64 + axis_t(&cfg.bottom_x, 2.0) * da.width as f64;
    let tail = (x, overlay_y(4.0));
    let tip = (x, overlay_y(8.0));
    assert!(
        tip.1 < tail.1 && tail.1 > da.y as f64 + 10.0,
        "the anchor sits mid-panel"
    );
    assert_red(
        &img,
        (x, (tail.1 + tip.1) * 0.5),
        0.0,
        0.0,
        "overlay-scaled midpoint",
    );
    // The anchor's x is a major tick of the base top axis, and decorations
    // draw above every layer's data, so the tick's inward mark darkens the
    // head's centre column; the head is probed beside it.
    assert_red(&img, tip, 2.0, 6.0, "overlay-scaled head");
    assert_red(&img, tip, -2.0, 6.0, "overlay-scaled head");
    assert_red(&img, tail, 0.0, -3.0, "shaft above the mid-panel anchor");
    // On the base axis the anchor would be the top edge and the whole arrow
    // clipped above it; the shaft starting mid-panel is the overlay's doing.
    assert_clear(&img, tail, 0.0, 6.0, "below the overlay anchor");
}

/// A vector series assigned to a subplot draws inside that subplot's data
/// area, through its axes, and nowhere in the parent.
#[test]
fn vector_series_draw_inside_their_subplot() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(2.0, 2.0)], &[(40.0, 0.0)]);
    let mut chart = bare_chart();
    let mut sub = bare_chart();
    sub.config_mut().chart_area = ChartArea(Rect {
        x: WIDTH,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    chart.config_mut().subplots.push(Subplot {
        config: sub.config().clone(),
        series: vec!["arrows".into()],
        link_x: None,
        link_y: None,
    });
    let series = [points("arrows", pixels(1.0))];
    let img = renderer.export_document_rgba(&chart, &series, 1.0).unwrap();
    assert_eq!((img.width, img.height), (WIDTH * 2, HEIGHT));

    let a = data_to_px(&sub, 2.0, 2.0);
    assert!(
        a.0 >= WIDTH as f64,
        "the subplot's anchor lies in the right half"
    );
    assert_red(&img, a, 20.0, 0.0, "shaft inside the subplot");
    assert_clear(&img, a, 45.0, 0.0, "past the tip inside the subplot");
    let parent_red = (0..img.height)
        .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
        .filter(|(x, y)| is_red(pixel(&img, *x, *y)))
        .count();
    assert_eq!(parent_red, 0, "the parent draws no arrow");
}

// ── Extent ───────────────────────────────────────────────────────────────────

fn resolve_extent(renderer: &Renderer, series: &SeriesConfig) -> GpuSeriesExtent {
    let ticket = renderer
        .begin_series_fit_extent(series)
        .unwrap()
        .expect("a vector series has an extent");
    pollster::block_on(ticket.resolve()).unwrap().unwrap()
}

/// Auto fit includes the tips of data-scaled point vectors, because a tip is
/// a data coordinate; a pixel-scaled vector fits its anchors only, and a
/// grid fits its anchors.
#[test]
fn series_extent_includes_data_scaled_tips_only() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(
        &mut renderer,
        &[(1.0, 1.0), (2.0, 2.0)],
        &[(3.0, 0.5), (-1.0, 4.0)],
    );
    pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();

    let ext = resolve_extent(&renderer, &points("arrows", data(1.0)));
    assert_eq!(
        (ext.x.min, ext.x.max),
        (1.0, 4.0),
        "tips (4, 1.5) and (1, 6)"
    );
    assert_eq!((ext.y.min, ext.y.max), (1.0, 6.0));

    let ext = resolve_extent(&renderer, &points("arrows", data(2.0)));
    assert_eq!(
        (ext.x.min, ext.x.max),
        (0.0, 7.0),
        "the factor scales the tips"
    );
    assert_eq!((ext.y.min, ext.y.max), (1.0, 10.0));

    let ext = resolve_extent(&renderer, &points("arrows", pixels(1.0)));
    assert_eq!(
        (ext.x.min, ext.x.max),
        (1.0, 2.0),
        "pixel mode: anchors only"
    );
    assert_eq!((ext.y.min, ext.y.max), (1.0, 2.0));

    // A hidden arrow contributes its anchor, not a tip.
    renderer.remove_column("u").unwrap();
    renderer
        .add_column("u", &col_f64(vec![3.0, f64::NAN]))
        .unwrap();
    let ext = resolve_extent(&renderer, &points("arrows", data(1.0)));
    assert_eq!((ext.x.min, ext.x.max), (1.0, 4.0));
    assert_eq!((ext.y.min, ext.y.max), (1.0, 2.0));

    grid_fixture(&mut renderer, &GridLayout::Edges);
    let ext = resolve_extent(
        &renderer,
        &grid(
            &["ux0", "ux1", "ux2"],
            &["vx0", "vx1", "vx2"],
            &MatrixOrientation::ColumnsAreX,
            &GridLayout::Edges,
            data(1.0),
        ),
    );
    // A grid vector fits its anchors, the cell centres of an edge grid; its
    // data-scaled tips are not part of the fit.
    assert_eq!((ext.x.min, ext.x.max), (0.5, 2.5), "grid: the anchors");
    assert_eq!((ext.y.min, ext.y.max), (0.5, 1.5));
}

// ── Registration ─────────────────────────────────────────────────────────────

/// Registration rejects an invalid style, an unregistered component column,
/// mismatched component grids, a colour map of the wrong form and a colour
/// map without a colourbar.
#[test]
fn vector_series_are_validated_at_registration() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(1.0, 1.0)], &[(1.0, 1.0)]);
    grid_fixture(&mut renderer, &GridLayout::Edges);
    let config = grid_chart().config().clone();
    let reason = |result: Result<_, FiggyError>| match result {
        Err(FiggyError::InvalidSeriesConfig { reason, .. }) => reason,
        other => panic!("expected an invalid series, got {other:?}"),
    };

    let mut negative = pixels(1.0);
    negative.width_px = -1.0;
    assert_eq!(
        reason(renderer.register_chart(config.clone(), vec![points("v", negative)])),
        VectorStyle::INVALID_PIXELS_REASON
    );
    assert_eq!(
        reason(renderer.register_chart(config.clone(), vec![points("v", data(0.0))])),
        VectorStyle::INVALID_SCALE_REASON
    );

    let mut missing = points("v", pixels(1.0));
    missing.render_type = DataRenderType::Vector {
        components: VectorComponents::Points {
            u: "u".into(),
            v: "nope".into(),
        },
        style: pixels(1.0),
    };
    assert!(matches!(
        renderer.register_chart(config.clone(), vec![missing]),
        Err(FiggyError::UnknownColumn { id }) if id == "nope"
    ));

    let mut mismatched = grid(
        &["ux0", "ux1", "ux2"],
        &["vy0", "vy1"],
        &MatrixOrientation::ColumnsAreX,
        &GridLayout::Edges,
        pixels(1.0),
    );
    if let DataRenderType::Vector {
        components: VectorComponents::Grid { v, .. },
        ..
    } = &mut mismatched.render_type
    {
        v.orientation = MatrixOrientation::ColumnsAreY;
    }
    assert!(
        reason(renderer.register_chart(config.clone(), vec![mismatched])).contains("orientation")
    );

    let mut wrong_form = pixels(1.0);
    wrong_form.color_map = Some(VectorColorMap::Grid(matrix(
        &["ux0"],
        &MatrixOrientation::ColumnsAreX,
        &GridLayout::Edges,
    )));
    let mut with_bar = config.clone();
    with_bar.colorbar = Some(two_stop_colorbar(AxisScale::Linear, 0.0, 1.0));
    assert!(
        reason(renderer.register_chart(with_bar, vec![points("v", wrong_form)])).contains("form")
    );

    let mut no_bar = pixels(1.0);
    no_bar.color_map = Some(VectorColorMap::Column("u".into()));
    assert!(
        reason(renderer.register_chart(config, vec![points("v", no_bar)])).contains("colorbar")
    );
}

/// Removing a component column cascades like removing an anchor column: the
/// series that read it leave their charts.
#[test]
fn removing_a_component_column_removes_the_series() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    point_fixture(&mut renderer, &[(1.0, 1.0)], &[(1.0, 1.0)]);
    grid_fixture(&mut renderer, &GridLayout::Edges);
    let point_chart = renderer
        .register_chart(
            bare_chart().config().clone(),
            vec![points("arrows", pixels(1.0))],
        )
        .unwrap();
    let grid_chart_id = renderer
        .register_chart(
            grid_chart().config().clone(),
            vec![grid(
                &["ux0", "ux1", "ux2"],
                &["vx0", "vx1", "vx2"],
                &MatrixOrientation::ColumnsAreX,
                &GridLayout::Edges,
                pixels(1.0),
            )],
        )
        .unwrap();

    assert!(renderer.remove_column("v").unwrap());
    assert!(
        renderer.chart_series(point_chart).unwrap().is_empty(),
        "the point series read the removed v column"
    );
    assert_eq!(renderer.chart_series(grid_chart_id).unwrap().len(), 1);

    assert!(renderer.remove_column("vx1").unwrap());
    assert!(
        renderer.chart_series(grid_chart_id).unwrap().is_empty(),
        "the grid series read the removed v column"
    );
}

// ── Streamlines ──────────────────────────────────────────────────────────────

fn streamline_style(arrows: Option<StreamlineArrows>, color: Option<&str>) -> DataRenderType {
    DataRenderType::Streamlines {
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: RED,
            line_width: 4.0,
            fill: None,
            fill_color: None,
            interpolation: LineInterpolation::Linear,
        },
        arrows,
        color_map: color.map(|column| PointColorMap {
            column: column.into(),
        }),
    }
}

fn streamlines(arrows: Option<StreamlineArrows>, color: Option<&str>) -> SeriesConfig {
    SeriesConfig {
        series_id: "flow".into(),
        source_id: None,
        label: None,
        x_column: "sx".into(),
        y_column: "sy".into(),
        render_type: streamline_style(arrows, color),
    }
}

/// Host-integrated paths: one series, streamlines separated by NaN rows.
fn streamline_fixture(renderer: &mut Renderer, points: &[(f64, f64)]) {
    renderer
        .add_columns(&[
            (
                "sx",
                &col_f64(points.iter().map(|p| p.0).collect()) as &dyn renderer::ColumnSource,
            ),
            (
                "sy",
                &col_f64(points.iter().map(|p| p.1).collect()) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("streamline upload");
}

/// The line of a colour-mapped streamline series takes the colourbar's
/// colour of the value interpolated between its samples, as a colour-mapped
/// marker of that value would; `nan_color` where a sample has no value; and
/// its arrowheads take the colour of the value at their position.
#[test]
fn colour_mapped_streamlines_follow_the_colourbar_ramp() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    // Two horizontal streamlines, the second with a missing colour value.
    streamline_fixture(
        &mut renderer,
        &[
            (0.5, 1.0),
            (3.5, 1.0),
            (f64::NAN, f64::NAN),
            (0.5, 3.0),
            (3.5, 3.0),
        ],
    );
    renderer
        .add_column("speed", &col_f64(vec![0.0, 10.0, f64::NAN, 4.0, f64::NAN]))
        .unwrap();
    let mut chart = bare_chart();
    let bar = two_stop_colorbar(AxisScale::Linear, 0.0, 10.0);
    chart.config_mut().colorbar = Some(bar.clone());
    let series = [streamlines(None, Some("speed"))];
    let img = export(&mut renderer, &chart, &series);
    for x in [1.0, 2.0, 3.0] {
        let z = (x - 0.5) / 3.0 * 10.0;
        let px = data_to_px(&chart, x, 1.0);
        let p = at(&img, px, 0.0, 0.0);
        let want = bar.color_for_z(z);
        assert!(
            (p[0] as i32 - to8(want.r)).abs() <= 6 && (p[1] as i32 - to8(want.g)).abs() <= 6,
            "x = {x}: got {p:?}, expected {want:?}"
        );
    }
    assert_colour_at(&img, data_to_px(&chart, 2.0, 3.0), BLUE, "missing value");
    // With arrowheads, a head takes the colour of the value at its centre:
    // slot 1 sits 60 px along the first streamline, its base behind.
    let series = [streamlines(Some(ARROWS), Some("speed"))];
    let img = export(&mut renderer, &chart, &series);
    let a = data_to_px(&chart, 0.5, 1.0);
    let b = data_to_px(&chart, 3.5, 1.0);
    let z = 55.0 / (b.0 - a.0) * 10.0;
    let p = at(&img, (a.0 + 55.0, a.1 - 3.5), 0.0, 0.0);
    let want = bar.color_for_z(z);
    assert!(
        (p[0] as i32 - to8(want.r)).abs() <= 6 && (p[1] as i32 - to8(want.g)).abs() <= 6,
        "head: got {p:?}, expected {want:?}"
    );
    // The separator draws nothing between the two lines.
    assert_clear(
        &img,
        data_to_px(&chart, 2.0, 2.0),
        0.0,
        0.0,
        "between streamlines",
    );
}

/// 12 px long, 10 px wide heads every 40 px of drawn arc.
const ARROWS: StreamlineArrows = StreamlineArrows {
    spacing_px: 40.0,
    head_length_px: 12.0,
    head_width_px: 10.0,
};

/// Head ink beside a 1 px line: a pixel 3 px off the path is inked only
/// inside a head (10 px wide).
fn thin_streamlines(arrows: Option<StreamlineArrows>) -> SeriesConfig {
    let mut series = streamlines(arrows, None);
    if let DataRenderType::Streamlines { line, .. } = &mut series.render_type {
        line.line_width = 1.0;
    }
    series
}

/// The inked pixels across the path `s` px along it from `start` in the
/// unit screen direction `dir`, within `reach` px to either side.
fn ink_across(img: &RasterImage, start: (f64, f64), dir: (f64, f64), s: f64, reach: i32) -> usize {
    let normal = (-dir.1, dir.0);
    (-reach..=reach)
        .filter(|side| {
            let side = f64::from(*side);
            let q = (
                start.0 + dir.0 * s + normal.0 * side,
                start.1 + dir.1 * s + normal.1 * side,
            );
            is_red(at(img, q, 0.0, 0.0))
        })
        .count()
}

/// Whether a head is centred `s` px along the path from `start` in the unit
/// screen direction `dir`, pointing along `dir`: wide near its base behind
/// the centre (10 px across), narrow near its tip ahead of it. `scale` is
/// the export scale.
fn head_at_scaled(
    img: &RasterImage,
    start: (f64, f64),
    dir: (f64, f64),
    s: f64,
    scale: f64,
) -> bool {
    let reach = (8.0 * scale) as i32;
    let base = ink_across(img, start, dir, s - 4.0 * scale, reach);
    let tip = ink_across(img, start, dir, s + 4.0 * scale, reach);
    base as f64 >= 6.0 * scale && (tip as f64) <= 3.0 * scale
}

fn head_at(img: &RasterImage, start: (f64, f64), dir: (f64, f64), s: f64) -> bool {
    head_at_scaled(img, start, dir, s, 1.0)
}

/// No head across the path there: at most the 1 px line's ink.
fn no_head_at(img: &RasterImage, start: (f64, f64), dir: (f64, f64), s: f64) -> bool {
    ink_across(img, start, dir, s, 8) <= 3
}

fn unit(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let d = (b.0 - a.0, b.1 - a.1);
    let l = (d.0 * d.0 + d.1 * d.1).sqrt();
    (d.0 / l, d.1 / l)
}

/// Arrowheads sit at `(k + 0.5) * spacing_px` of drawn arc from the first
/// point, centred on the path and pointing along its tangent: along a
/// horizontal streamline, against it when the path runs backwards, and
/// along a diagonal one. Between two slots there is no head.
#[test]
fn arrowheads_sit_at_fixed_arc_spacing_and_follow_the_tangent() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let chart = bare_chart();
    for (from, to) in [
        ((0.5, 2.0), (3.5, 2.0)),
        ((3.5, 2.0), (0.5, 2.0)),
        ((0.5, 0.5), (3.5, 3.5)),
    ] {
        streamline_fixture(&mut renderer, &[from, to]);
        let img = export(&mut renderer, &chart, &[thin_streamlines(Some(ARROWS))]);
        let a = data_to_px(&chart, from.0, from.1);
        let b = data_to_px(&chart, to.0, to.1);
        let dir = unit(a, b);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let mut k = 0.0;
        while (k + 0.5) * 40.0 < len - 8.0 {
            let s = (k + 0.5) * 40.0;
            assert!(head_at(&img, a, dir, s), "{from:?} -> {to:?}: head at {s}");
            assert!(
                no_head_at(&img, a, dir, s + 20.0),
                "{from:?} -> {to:?}: gap at {}",
                s + 20.0
            );
            k += 1.0;
        }
        assert!(k >= 3.0, "{from:?} -> {to:?}: {k} heads checked");
        renderer.remove_column("sx").unwrap();
        renderer.remove_column("sy").unwrap();
    }
    // Without arrows the same path draws no head.
    streamline_fixture(&mut renderer, &[(0.5, 2.0), (3.5, 2.0)]);
    let img = export(&mut renderer, &chart, &[thin_streamlines(None)]);
    let a = data_to_px(&chart, 0.5, 2.0);
    assert!(no_head_at(&img, a, (1.0, 0.0), 20.0));
}

/// The arrowheads count the drawn arc only: a NaN row breaks the line and
/// adds no arc, so no head is drawn across the gap and the slots go on along
/// the next streamline where the arc left off.
#[test]
fn arrowheads_skip_nan_gaps() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let chart = bare_chart();
    streamline_fixture(
        &mut renderer,
        &[
            (0.5, 1.0),
            (3.5, 1.0),
            (f64::NAN, f64::NAN),
            (0.5, 3.0),
            (3.5, 3.0),
        ],
    );
    let img = export(&mut renderer, &chart, &[thin_streamlines(Some(ARROWS))]);
    let a1 = data_to_px(&chart, 0.5, 1.0);
    let b1 = data_to_px(&chart, 3.5, 1.0);
    let a2 = data_to_px(&chart, 0.5, 3.0);
    let len = b1.0 - a1.0;
    // Nothing between the two streamlines.
    for t in [0.25, 0.5, 0.75] {
        let mid = (a1.0 + len * t, (a1.1 + a2.1) * 0.5);
        assert_clear(&img, mid, 0.0, 0.0, "in the gap");
    }
    // The first slot past the first streamline's arc sits on the second.
    let first = ((len / 40.0 - 0.5).floor() + 1.0 + 0.5) * 40.0;
    let s = first - len;
    assert!(s > 6.0 && s < len - 8.0, "slot at {s}");
    assert!(head_at(&img, a2, (1.0, 0.0), s), "continued head at {s}");
    assert!(
        head_at(&img, a2, (1.0, 0.0), s + 40.0),
        "next head at {}",
        s + 40.0
    );
}

/// The arrowheads' slot pitch is their spacing in render pixels: at a 2x
/// export every head and gap doubles with the image, so the heads keep their
/// logical spacing.
#[test]
fn streamline_arrowheads_keep_pixel_spacing_at_1x_and_2x() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let chart = bare_chart();
    streamline_fixture(&mut renderer, &[(0.5, 2.0), (3.5, 2.0)]);
    let series = [thin_streamlines(Some(ARROWS))];
    let img = export(&mut renderer, &chart, &series);
    let a = data_to_px(&chart, 0.5, 2.0);
    assert!(head_at(&img, a, (1.0, 0.0), 60.0));
    assert!(no_head_at(&img, a, (1.0, 0.0), 80.0));
    let img2 = renderer.export_panel_rgba(&chart, &series, 2.0).unwrap();
    let a2 = data_to_px_scaled(&chart, 0.5, 2.0, 2.0);
    for s in [40.0, 120.0, 200.0] {
        assert!(
            head_at_scaled(&img2, a2, (1.0, 0.0), s, 2.0),
            "2x head at {s}"
        );
        assert!(
            ink_across(&img2, a2, (1.0, 0.0), s + 40.0, 16) <= 4,
            "2x gap at {}",
            s + 40.0
        );
    }
}

/// A zoom or a pan changes the drawn arc, and the heads follow it: they keep
/// their pixel spacing from the first point rather than their data
/// positions.
#[test]
fn pan_respaces_arrowheads_in_pixels() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    streamline_fixture(&mut renderer, &[(0.5, 2.0), (3.5, 2.0)]);
    let series = [thin_streamlines(Some(ARROWS))];
    let mut chart = bare_chart();
    for (min, max) in [(0.0, 4.0), (-1.0, 3.0), (0.0, 6.0)] {
        chart.set_x_range(min, max);
        let img = export(&mut renderer, &chart, &series);
        let a = data_to_px(&chart, 0.5, 2.0);
        assert!(head_at(&img, a, (1.0, 0.0), 20.0), "x {min}..{max}");
        assert!(head_at(&img, a, (1.0, 0.0), 60.0), "x {min}..{max}");
        assert!(no_head_at(&img, a, (1.0, 0.0), 40.0), "x {min}..{max}");
    }
}

/// The indirect draw the arc scan writes for the slots issues one glyph quad
/// per slot: its vertex count is the vector glyph's.
#[test]
fn arrowhead_slot_vertex_count_equals_arrow_vertices() {
    let scan = include_str!("../src/data_render/line_arc.wgsl");
    let vertices = renderer::data_render::vector::ARROW_VERTICES;
    assert!(
        scan.contains(&format!("star_args[0] = {vertices}u;")),
        "the slot kernel's vertex count is not ARROW_VERTICES = {vertices}"
    );
    let glyph = include_str!("../src/data_render/common/arrow.wgsl");
    assert!(glyph.contains(&format!("const ARROW_VERTICES: u32 = {vertices}u;")));
}

/// A picked streamline vertex draws the picked ring around it, as a line's
/// vertex does: `radius_extra_px` out from the vertex (a line has no
/// marker), nowhere else.
#[test]
fn picked_streamline_point_draws_the_ring() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    streamline_fixture(
        &mut renderer,
        &[
            (0.5, 1.0),
            (2.0, 1.5),
            (3.5, 1.0),
            (f64::NAN, f64::NAN),
            (0.5, 3.0),
            (3.5, 3.0),
        ],
    );
    let mut chart = bare_chart();
    chart.config_mut().picked_points = Some(PickedPointsConfig {
        visible: true,
        refs: vec![PickedPointRef {
            source_id: None,
            series_id: "flow".into(),
            point_index: 4,
        }],
        ring_color: GREEN,
        ring_width_px: 3.0,
        radius_extra_px: 8.0,
    });
    let img = export(&mut renderer, &chart, &[thin_streamlines(Some(ARROWS))]);
    let a = data_to_px(&chart, 0.5, 3.0);
    for (dx, dy) in [(0.0, -8.0), (0.0, 8.0), (-8.0, 0.0)] {
        let p = at(&img, a, dx, dy);
        assert!(is_green(p), "ring at ({dx}, {dy}): {p:?}");
    }
    let other = data_to_px(&chart, 2.0, 1.5);
    assert!(
        !is_green(at(&img, other, 0.0, -8.0)),
        "no ring at an unpicked vertex"
    );
    let green = count(&img, is_green);
    assert!((60..400).contains(&green), "{green} ring pixels");
}

/// Streamlines on an overlay layer scale by the overlay's Y axis, heads
/// included: a path at y = 4 on an overlay axis running 0..8 draws halfway
/// up the data area, with its heads on it. A document export draws every
/// layer.
#[test]
fn streamlines_scale_by_their_overlay_axis() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    streamline_fixture(&mut renderer, &[(0.5, 4.0), (3.5, 4.0)]);
    let mut chart = bare_chart();
    let mut overlay = OverlayLayer::default();
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 8.0;
    overlay.y_axis.major_spacing = 2.0;
    overlay.series = vec!["flow".into()];
    chart.config_mut().overlays.push(overlay);
    let series = [thin_streamlines(Some(ARROWS))];
    let img = renderer.export_document_rgba(&chart, &series, 1.0).unwrap();
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let y = (da.y + da.height) as f64 - axis_t(&cfg.overlays[0].y_axis, 4.0) * da.height as f64;
    let x = da.x as f64 + axis_t(&cfg.bottom_x, 0.5) * da.width as f64;
    assert!(y > da.y as f64 + 40.0, "the path sits mid-panel");
    assert!(
        head_at(&img, (x, y), (1.0, 0.0), 60.0),
        "overlay-scaled head"
    );
    assert!(
        no_head_at(&img, (x, y), (1.0, 0.0), 80.0),
        "between overlay heads"
    );
    // On the base axis y = 4 is the top edge: nothing is drawn there.
    let top = (x + 60.0, da.y as f64 + 1.0);
    assert_clear(&img, top, 0.0, 0.0, "the base axis position");
}

/// Streamlines assigned to a subplot draw inside its data area, through its
/// axes, heads included, and nowhere in the parent.
#[test]
fn streamlines_draw_inside_their_subplot() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    streamline_fixture(&mut renderer, &[(0.5, 2.0), (3.5, 2.0)]);
    let mut chart = bare_chart();
    let mut sub = bare_chart();
    sub.config_mut().chart_area = ChartArea(Rect {
        x: WIDTH,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    chart.config_mut().subplots.push(Subplot {
        config: sub.config().clone(),
        series: vec!["flow".into()],
        link_x: None,
        link_y: None,
    });
    let series = [thin_streamlines(Some(ARROWS))];
    let img = renderer.export_document_rgba(&chart, &series, 1.0).unwrap();
    assert_eq!((img.width, img.height), (WIDTH * 2, HEIGHT));
    let a = data_to_px(&sub, 0.5, 2.0);
    assert!(
        a.0 >= WIDTH as f64,
        "the subplot's path lies in the right half"
    );
    assert!(
        head_at(&img, a, (1.0, 0.0), 20.0),
        "head inside the subplot"
    );
    assert!(
        head_at(&img, a, (1.0, 0.0), 60.0),
        "head inside the subplot"
    );
    let parent_red = (0..img.height)
        .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
        .filter(|(x, y)| is_red(pixel(&img, *x, *y)))
        .count();
    assert_eq!(parent_red, 0, "the parent draws no streamline");
}
