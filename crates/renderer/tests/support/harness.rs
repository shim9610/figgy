//! Helpers the integration tests share (#341): renderers on the shared test
//! device, columns, colours, pixel probes, exports and artifact writers.
//! Each was copied byte for byte into several test files; it lives here
//! once. Where copies of one helper differed, each version keeps its own
//! name here, and a test file imports the one it used under its old name
//! (`use harness::renderer_rgba as renderer;`).
//!
//! A binary that includes this file also includes `support/gpu.rs` as
//! `test_gpu`, which holds the shared device.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use crate::test_gpu::{self, BlockOn as _};
use ::renderer::config::{AxisScale, Category};
use ::renderer::data::Column;
use ::renderer::data_config::{
    CategoryBarDirection, CategoryBarStyle, DataScatterStyleConfig, ScatterShape, XySeries,
};
use ::renderer::layout::{Rect, RectF};
use ::renderer::radial::RadialRenderer;
use ::renderer::{
    Chart, Color, Config, CpuTextMeasure, DataRenderType, GpuPickRequest, Material, PickedData,
    RasterImage, Renderer, RendererDevice, SeriesConfig,
};

// ── Renderers ───────────────────────────────────────────────────────────

/// The shared test device in a bundle of its own per call, so each renderer
/// a test builds charges its own ledger and budget (#160).
pub fn gpu() -> RendererDevice {
    let (d, q) = test_gpu::shared_device();
    RendererDevice::new(d, q).expect("figgy poll thread")
}

/// A device of its own, created for this call.
pub fn fresh_gpu() -> RendererDevice {
    let (device, queue) = test_gpu::device();
    RendererDevice::new(device, queue).expect("figgy poll thread")
}

/// A renderer on the shared device, `Bgra8Unorm`, with a `budget`-byte pool.
pub fn renderer_with(budget: u64) -> Renderer {
    let (device, queue) = test_gpu::shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Bgra8Unorm,
        budget,
    )
    .expect("renderer init")
}

/// [`renderer_with`] a 4 MiB pool, for the files whose renderer helper
/// returned an `Option`.
pub fn try_renderer() -> Option<Renderer> {
    Some(renderer_with(4 * 1024 * 1024))
}

/// [`renderer_with`] a 16 MiB pool.
pub fn renderer_16mib() -> Renderer {
    renderer_with(16 * 1024 * 1024)
}

/// [`renderer_with`] a 64 MiB pool.
pub fn renderer_bgra() -> Renderer {
    renderer_with(64 * 1024 * 1024)
}

/// A renderer on the shared device, `Rgba8Unorm`, with a 4 MiB pool.
pub fn renderer_rgba() -> Renderer {
    let (device, queue) = test_gpu::shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init")
}

pub fn headless_renderer() -> Renderer {
    Renderer::headless_blocking().expect("headless renderer")
}

pub fn radial_renderer() -> RadialRenderer {
    RadialRenderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap()
}

pub fn measure(cfg: &Config) -> CpuTextMeasure {
    CpuTextMeasure::for_style(&cfg.draw_style)
}

// ── Data ────────────────────────────────────────────────────────────────

pub fn col(data: Vec<f64>) -> Column<f64> {
    Column { data }
}

pub fn upload(r: &mut Renderer, id: &str, data: Vec<f64>) {
    r.add_column(id, &Column { data }).unwrap();
}

pub fn categories(n: usize) -> Vec<Category> {
    (0..n)
        .map(|i| Category::new(format!("c{i}"), format!("Cat {i}")))
        .collect()
}

pub const DIRECTIONS: [CategoryBarDirection; 2] = [
    CategoryBarDirection::Vertical,
    CategoryBarDirection::Horizontal,
];

pub const SCALES: [f32; 2] = [1.0, 2.0];

pub fn flat_style() -> CategoryBarStyle {
    CategoryBarStyle {
        material: Material::Flat,
        corner_radius: 0.0,
        outline: false,
        ..CategoryBarStyle::default()
    }
}

pub fn markers(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Xy(XySeries {
            scatter: Some(DataScatterStyleConfig {
                point_color: color,
                point_shape: ScatterShape::SquareFilled,
                point_size: 8.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            }),
            line: None,
            errors: None,
            stem: None,
        }),
    }
}

pub fn picked(mut config: ::renderer::Config, series_id: &str, index: usize) -> ::renderer::Config {
    config.picked_data = Some(::renderer::DataSelectionsConfig {
        refs: vec![::renderer::PickedDataRef::Point {
            source_id: None,
            series_id: series_id.into(),
            point_index: index,
        }],
        ..Default::default()
    });
    config
}

// ── Colours ─────────────────────────────────────────────────────────────
// The files wrote these either as `Color { .. }` literals or with
// `Color::from_rgb8`; both give the same value (`255 / 255.0 == 1.0`).

pub const RED: Color = Color::from_rgb8(255, 0, 0);
pub const GREEN: Color = Color::from_rgb8(0, 255, 0);
pub const BLUE: Color = Color::from_rgb8(0, 0, 255);
pub const CYAN: Color = Color::from_rgb8(0, 255, 255);
pub const MAGENTA: Color = Color::from_rgb8(255, 0, 255);
pub const ORANGE: Color = Color::from_rgb8(255, 128, 0);
pub const YELLOW: Color = Color::from_rgb8(255, 255, 0);
/// The `GREEN` of the energy-level and radar tests.
pub const MID_GREEN: Color = Color::from_rgb8(0, 200, 0);
/// The `GREEN` of the parallel-coordinates and pick agreement tests.
pub const DARK_GREEN: Color = Color {
    r: 0.0,
    g: 0.6,
    b: 0.0,
    a: 1.0,
};

pub fn to8(v: f32) -> i32 {
    (v.clamp(0.0, 1.0) * 255.0).round() as i32
}

// ── Pixel probes ────────────────────────────────────────────────────────

/// The pixel at `(x, y)` as a slice.
pub fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

/// The pixel at `(x, y)` as an array.
pub fn pixel_rgba(img: &RasterImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

/// The pixel containing the point `(x, y)`, clamped into the image.
pub fn pixel_at(img: &RasterImage, (x, y): (f64, f64)) -> [u8; 4] {
    let x = (x.floor() as i64).clamp(0, i64::from(img.width) - 1) as u32;
    let y = (y.floor() as i64).clamp(0, i64::from(img.height) - 1) as u32;
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

/// The polar tests' copy of [`pixel_at`], which spells the conversions with
/// `as`.
pub fn pixel_at_polar(img: &RasterImage, (x, y): (f64, f64)) -> [u8; 4] {
    let x = (x.floor() as i64).clamp(0, img.width as i64 - 1) as u32;
    let y = (y.floor() as i64).clamp(0, img.height as i64 - 1) as u32;
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

/// The pixel nearest `(x, y)`, which must lie inside the image.
pub fn pixel_rounded(img: &RasterImage, x: f32, y: f32) -> [u8; 4] {
    let (x, y) = (x.round() as u32, y.round() as u32);
    assert!(
        x < img.width && y < img.height,
        "probe ({x}, {y}) outside the image"
    );
    let i = ((y * img.width + x) * 4) as usize;
    img.rgba[i..i + 4].try_into().unwrap()
}

pub fn count(img: &RasterImage, pred: fn(&[u8]) -> bool) -> usize {
    img.rgba.chunks_exact(4).filter(|p| pred(p)).count()
}

// Opaque pure colours: alpha and the named channels above 200, the rest
// below 60.

pub fn is_red(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 60 && p[2] < 60
}

pub fn is_green(p: &[u8]) -> bool {
    p[3] > 200 && p[1] > 200 && p[0] < 60 && p[2] < 60
}

pub fn is_blue(p: &[u8]) -> bool {
    p[3] > 200 && p[2] > 200 && p[0] < 60 && p[1] < 60
}

pub fn is_cyan(p: &[u8]) -> bool {
    p[3] > 200 && p[1] > 200 && p[2] > 200 && p[0] < 60
}

pub fn is_magenta(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[2] > 200 && p[1] < 60
}

pub fn is_orange(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && (90..170).contains(&p[1]) && p[2] < 60
}

pub fn is_yellow(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] > 200 && p[2] < 60
}

// The same predicates on an array pixel.

pub fn is_red_rgba(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 60 && p[2] < 60
}

pub fn is_blue_rgba(p: [u8; 4]) -> bool {
    p[3] > 200 && p[2] > 200 && p[0] < 60 && p[1] < 60
}

pub fn is_magenta_rgba(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[2] > 200 && p[1] < 60
}

pub fn is_empty_rgba(p: [u8; 4]) -> bool {
    p[3] < 16
}

// Thresholds 180 and 80, for antialiased marks.

pub fn is_red_soft(p: &[u8]) -> bool {
    p[3] > 180 && p[0] > 180 && p[1] < 80 && p[2] < 80
}

/// The default picked-highlight magenta.
pub fn is_selection(p: &[u8]) -> bool {
    p[3] > 180 && p[0] > 180 && p[2] > 180 && p[1] < 80
}

// Line ink: any visible coverage of a mostly pure colour.

pub fn is_red_ink(p: &[u8]) -> bool {
    p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
}

pub fn is_green_ink(p: &[u8]) -> bool {
    p[3] > 16 && p[1] > 120 && p[0] < 90 && p[2] < 90
}

pub fn is_blue_ink(p: &[u8]) -> bool {
    p[3] > 16 && p[2] > 120 && p[0] < 90 && p[1] < 90
}

pub fn is_empty_ink(p: &[u8]) -> bool {
    p[3] <= 16
}

/// Within 40 of `color` on each channel.
pub fn is(color: Color) -> impl Fn([u8; 4]) -> bool {
    let [r, g, b] = [color.r, color.g, color.b].map(|c| (c * 255.0).round() as i32);
    move |p| {
        (p[0] as i32 - r).abs() < 40 && (p[1] as i32 - g).abs() < 40 && (p[2] as i32 - b).abs() < 40
    }
}

/// The centroid of the pixels matching `pred` within `radius` of `(cx, cy)`.
pub fn centroid(
    img: &RasterImage,
    (cx, cy): (f32, f32),
    radius: f32,
    pred: impl Fn([u8; 4]) -> bool,
) -> Option<(f32, f32)> {
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    let lo = |c: f32| (c - radius).max(0.0) as u32;
    for y in lo(cy)..((cy + radius) as u32).min(img.height) {
        for x in lo(cx)..((cx + radius) as u32).min(img.width) {
            if pred(pixel_rgba(img, x, y)) {
                sx += x as f32 + 0.5;
                sy += y as f32 + 0.5;
                n += 1.0;
            }
        }
    }
    (n > 0.0).then(|| (sx / n, sy / n))
}

/// Whether any pixel within `radius` of `centre` satisfies `pred`.
pub fn any_within(
    img: &RasterImage,
    centre: (f64, f64),
    radius: f64,
    pred: fn([u8; 4]) -> bool,
) -> bool {
    let n = radius.ceil() as i64;
    (-n..=n).any(|dy| {
        (-n..=n).any(|dx| {
            pred(pixel_at_polar(
                img,
                (centre.0 + dx as f64, centre.1 + dy as f64),
            ))
        })
    })
}

pub fn ink(image: &RasterImage, r: RectF, scale: f32) -> usize {
    let x0 = (r.x * scale).floor().max(0.) as u32;
    let y0 = (r.y * scale).floor().max(0.) as u32;
    let x1 = ((r.x + r.width) * scale).ceil().min(image.width as f32) as u32;
    let y1 = ((r.y + r.height) * scale).ceil().min(image.height as f32) as u32;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| ((y * image.width + x) * 4) as usize))
        .filter(|&i| image.rgba[i..i + 3].iter().any(|v| *v < 200))
        .count()
}

// ── Category slots ──────────────────────────────────────────────────────

/// An export-pixel region `(x0, y0, x1, y1)`.
pub type Region = (u32, u32, u32, u32);

/// Bounding box `(x0, y0, x1, y1)` (inclusive) of the pixels `pred` accepts
/// within `region` (export pixels, end exclusive).
pub fn bbox(img: &RasterImage, region: Region, pred: fn(&[u8]) -> bool) -> Option<Region> {
    let mut found: Option<Region> = None;
    for y in region.1..region.3.min(img.height) {
        for x in region.0..region.2.min(img.width) {
            if pred(pixel(img, x, y)) {
                let b = found.get_or_insert((x, y, x, y));
                *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
    }
    found
}

/// Export-pixel region of category `k`'s slot (the whole value extent).
pub fn slot(chart: &Chart, direction: CategoryBarDirection, k: usize, scale: f32) -> Region {
    let da = chart.config().data_area().expect("data area");
    let s = |v: f32| (v * scale).round() as u32;
    match direction {
        CategoryBarDirection::Vertical => {
            let w = da.width as f32 / 3.0;
            let x0 = da.x as f32 + w * k as f32;
            (
                s(x0),
                s(da.y as f32),
                s(x0 + w),
                s((da.y + da.height) as f32),
            )
        }
        CategoryBarDirection::Horizontal => {
            let h = da.height as f32 / 3.0;
            let y1 = (da.y + da.height) as f32 - h * k as f32;
            (
                s(da.x as f32),
                s(y1 - h),
                s((da.x + da.width) as f32),
                s(y1),
            )
        }
    }
}

pub fn across_span(b: Region, direction: CategoryBarDirection) -> (u32, u32) {
    match direction {
        CategoryBarDirection::Vertical => (b.0, b.2),
        CategoryBarDirection::Horizontal => (b.1, b.3),
    }
}

/// Value `v` as an export-pixel coordinate along the value axis.
pub fn value_px(chart: &Chart, direction: CategoryBarDirection, v: f64, scale: f32) -> f32 {
    let config = chart.config();
    let da = config.data_area().expect("data area");
    match direction {
        CategoryBarDirection::Vertical => {
            let t = (v - config.left_y.min) / (config.left_y.max - config.left_y.min);
            ((da.y + da.height) as f32 - t as f32 * da.height as f32) * scale
        }
        CategoryBarDirection::Horizontal => {
            let t = (v - config.bottom_x.min) / (config.bottom_x.max - config.bottom_x.min);
            (da.x as f32 + t as f32 * da.width as f32) * scale
        }
    }
}

// ── Data to pixels ──────────────────────────────────────────────────────

/// Data (x, y) → export pixel at `scale`, through the chart's data area and
/// axis ranges.
pub fn data_to_px(chart: &Chart, x: f64, y: f64, scale: f64) -> (f64, f64) {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let tx = (x - cfg.bottom_x.min) / (cfg.bottom_x.max - cfg.bottom_x.min);
    let ty = (y - cfg.left_y.min) / (cfg.left_y.max - cfg.left_y.min);
    (
        (f64::from(da.x) + tx * f64::from(da.width)) * scale,
        (f64::from(da.y + da.height) - ty * f64::from(da.height)) * scale,
    )
}

/// Data y → panel pixel y (screen y grows downward), linear or logarithmic.
pub fn data_y_to_px(chart: &Chart, value: f64) -> f32 {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let t = match cfg.left_y.scale {
        AxisScale::Linear => (value - cfg.left_y.min) / (cfg.left_y.max - cfg.left_y.min),
        AxisScale::Logarithmic => {
            (value.log10() - cfg.left_y.min.log10())
                / (cfg.left_y.max.log10() - cfg.left_y.min.log10())
        }
        AxisScale::Probability(_) => unreachable!("these charts are linear or logarithmic"),
    };
    (da.y + da.height) as f32 - t as f32 * da.height as f32
}

// ── Polar geometry ──────────────────────────────────────────────────────

/// The ring's centre, radius and inner radius in export pixels.
pub fn geometry(chart: &Chart, scale: f32) -> ((f64, f64), f64, f64) {
    let g = chart
        .config()
        .scaled(scale)
        .polar_geometry()
        .expect("geometry");
    (
        (f64::from(g.center[0]), f64::from(g.center[1])),
        f64::from(g.radius),
        f64::from(g.inner_radius),
    )
}

/// The export pixel at screen angle `a` and radius `r` from the centre.
pub fn at(centre: (f64, f64), a: f64, r: f64) -> (f64, f64) {
    (centre.0 + a.cos() * r, centre.1 - a.sin() * r)
}

// ── Export and picks ────────────────────────────────────────────────────

pub fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_rgba(
        ::renderer::ExportSource::chart(chart, series),
        ::renderer::ExportOptions::panel(scale),
    )
    .block_on()
    .unwrap()
}

/// [`export`] at 1x.
pub fn export_1x(renderer: &mut Renderer, chart: &Chart, series: &[SeriesConfig]) -> RasterImage {
    export(renderer, chart, series, 1.0)
}

/// What a data pick at `at` (image pixels) answers, as `(series_id, point
/// index)`.
pub fn pick_at(
    r: &mut Renderer,
    config: &::renderer::Config,
    series: &[SeriesConfig],
    at: (f32, f32),
    scale: f32,
) -> Option<(String, usize)> {
    let panel = config.chart_area.0;
    let id = r.register_chart(config.clone(), series.to_vec()).unwrap();
    let request = GpuPickRequest {
        canvas_position_px: [at.0, at.1],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: ((panel.x + panel.width) as f32 * scale) as u32,
            height: ((panel.y + panel.height) as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: 6.0 * scale,
    };
    let hit = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    match hit {
        Some(PickedData::Point {
            series_id,
            point_index,
            ..
        }) => Some((series_id, point_index)),
        Some(other) => panic!("unexpected pick {other:?}"),
        None => None,
    }
}

/// The matrix cell a pick answered.
pub fn cell(picked: Option<PickedData>) -> Option<(usize, usize)> {
    match picked {
        Some(PickedData::MatrixCell {
            x_index, y_index, ..
        }) => Some((x_index, y_index)),
        None => None,
        other => panic!("picked {other:?}"),
    }
}

// ── Artifacts ───────────────────────────────────────────────────────────

/// Writes the image when `FIGGY_INTERACTION_ARTIFACTS` names a directory.
pub fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            std::path::Path::new(&path).join(format!("{name}.png")),
            ::renderer::encode_png(image).unwrap(),
        )
        .unwrap();
    }
}

/// The other copy of [`save`], which joins the path with `format!`.
pub fn save_png(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        let png = ::renderer::encode_png(image).unwrap();
        std::fs::write(format!("{path}/{name}.png"), png).unwrap();
    }
}

/// Where the saved images land: `$FIGGY_PROBE_DIR` when set (`ci/verify.py`
/// points it at the CI image directory), otherwise Cargo's per-target scratch
/// directory under `target/`, so a plain test run never writes into the
/// source tree.
pub fn probe_dir() -> String {
    let dir = std::env::var("FIGGY_PROBE_DIR")
        .unwrap_or_else(|_| env!("CARGO_TARGET_TMPDIR").to_string());
    std::fs::create_dir_all(&dir).expect("create probe directory");
    dir
}

pub fn output_dir() -> PathBuf {
    let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let configured = std::env::var_os("FIGGY_LAYOUT_PROBE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/arch-after"));
    if configured.is_absolute() {
        configured
    } else {
        package_root.join(configured)
    }
}
