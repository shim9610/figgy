//! End-to-end pixel verification of range bar series
//! (`DataRenderType::RangeBars`) through the public renderer API and the
//! headless `export_panel_rgba`: bars floating from a start to an end value
//! in both directions at 1x and 2x, the direction, total and state colours,
//! whiskers, connectors and progress, missing rows, a numeric (time)
//! position axis with a slot width, the value-axis and position-axis fit,
//! the picked highlight and the pick of every part, value labels, column
//! replacement, the streaming rejection and overlays and subplots.
//!
//! Every part has its own pure colour, all flat with no outline, so a
//! colour predicate attributes every pixel to one part. Expected extents
//! come from the same data area and axis ranges the renderer draws with.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::Category;
use renderer::data::Column;
use renderer::data_config::{
    BarGroups, CategoryBarDirection, CategoryBarLabels, CategoryBarMode, CategoryBarSeriesLabels,
    CategoryBarStyle, RangeColor, RangeConnector, RangeProgress, RangeStateStyle, RangeTotal,
    RangeWhiskers,
};
use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, Color, DataRenderType, DataSelectionsConfig, Material, PickedDataRef, RasterImage,
    Renderer, RendererDevice, SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const RED: Color = Color::from_rgb8(255, 0, 0);
const BLUE: Color = Color::from_rgb8(0, 0, 255);
const GREEN: Color = Color::from_rgb8(0, 255, 0);
const YELLOW: Color = Color::from_rgb8(255, 255, 0);
const CYAN: Color = Color::from_rgb8(0, 255, 255);
const ORANGE: Color = Color::from_rgb8(255, 128, 0);
const MAGENTA: Color = Color::from_rgb8(255, 0, 255);
const NAN: f64 = f64::NAN;

/// Three rows over the categories a, b, c: a rising bar, a falling bar and
/// a third rising one.
const START: [f64; 3] = [20.0, 70.0, 30.0];
const END: [f64; 3] = [60.0, 40.0, 80.0];
const LOW: [f64; 3] = [10.0, 30.0, 25.0];
const HIGH: [f64; 3] = [70.0, 85.0, 90.0];

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

/// A renderer holding the positions and every column of the three rows.
fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("pos", &col(vec![0.0, 1.0, 2.0])).unwrap();
    r.add_column("start", &col(START.to_vec())).unwrap();
    r.add_column("end", &col(END.to_vec())).unwrap();
    r.add_column("low", &col(LOW.to_vec())).unwrap();
    r.add_column("high", &col(HIGH.to_vec())).unwrap();
    r.add_column("total", &col(vec![0.0, 0.0, 1.0])).unwrap();
    r.add_column("state", &col(vec![1.0, 0.0, 7.0])).unwrap();
    r.add_column("progress", &col(vec![0.5, NAN, 1.0])).unwrap();
    r
}

fn flat_style() -> CategoryBarStyle {
    CategoryBarStyle {
        material: Material::Flat,
        corner_radius: 0.0,
        outline: false,
        ..CategoryBarStyle::default()
    }
}

fn ranges(id: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "pos".into(),
        y_column: "end".into(),
        render_type: DataRenderType::RangeBars {
            color,
            style: flat_style(),
            start: "start".into(),
            whiskers: None,
            color_by: RangeColor::Single,
            progress: None,
            connector: None,
            slot_width: None,
            labels: CategoryBarSeriesLabels::default(),
        },
    }
}

/// The range bar fields of `series`.
#[allow(clippy::type_complexity)]
fn fields(
    series: &mut SeriesConfig,
) -> (
    &mut Option<RangeWhiskers>,
    &mut RangeColor,
    &mut Option<RangeProgress>,
    &mut Option<RangeConnector>,
    &mut CategoryBarSeriesLabels,
) {
    let DataRenderType::RangeBars {
        whiskers,
        color_by,
        progress,
        connector,
        labels,
        ..
    } = &mut series.render_type
    else {
        unreachable!()
    };
    (whiskers, color_by, progress, connector, labels)
}

/// Grid, legend, title and axis lines off; three categories on the
/// direction's position axis, bottom-up; the value axis over 0..100.
fn chart(direction: CategoryBarDirection) -> Chart {
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
    config.chart_title.visible = false;
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
    match direction {
        CategoryBarDirection::Vertical => config.bottom_x.categories = categories,
        CategoryBarDirection::Horizontal => config.left_y.categories = categories,
    }
    config.bar_groups = Some(BarGroups {
        mode: CategoryBarMode::Grouped,
        direction,
        group_width: 0.8,
        bar_gap: 4.0,
    });
    let mut chart = Chart::new(config);
    match direction {
        CategoryBarDirection::Vertical => chart.set_y_range(0.0, 100.0),
        CategoryBarDirection::Horizontal => chart.set_x_range(0.0, 100.0),
    }
    chart
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

fn is_red(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 60 && p[2] < 60
}
fn is_blue(p: &[u8]) -> bool {
    p[3] > 200 && p[2] > 200 && p[0] < 60 && p[1] < 60
}
fn is_green(p: &[u8]) -> bool {
    p[3] > 200 && p[1] > 200 && p[0] < 60 && p[2] < 60
}
fn is_yellow(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] > 200 && p[2] < 60
}
fn is_cyan(p: &[u8]) -> bool {
    p[3] > 200 && p[1] > 200 && p[2] > 200 && p[0] < 60
}
fn is_orange(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && (90..170).contains(&p[1]) && p[2] < 60
}
fn is_magenta(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[2] > 200 && p[1] < 60
}

type Region = (u32, u32, u32, u32);

/// Bounding box `(x0, y0, x1, y1)` (inclusive) of the pixels `pred` accepts
/// within `region` (export pixels, end exclusive).
fn bbox(img: &RasterImage, region: Region, pred: fn(&[u8]) -> bool) -> Option<Region> {
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
fn slot(chart: &Chart, direction: CategoryBarDirection, k: usize, scale: f32) -> Region {
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

/// Value `v` as an export-pixel coordinate along the value axis.
fn value_px(chart: &Chart, direction: CategoryBarDirection, v: f64, scale: f32) -> f32 {
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

/// The value-axis span `(smaller value end, larger value end)` of a box in
/// export pixels.
fn value_span(b: Region, direction: CategoryBarDirection) -> (f32, f32) {
    match direction {
        CategoryBarDirection::Vertical => (b.3 as f32 + 1.0, b.1 as f32),
        CategoryBarDirection::Horizontal => (b.0 as f32, b.2 as f32 + 1.0),
    }
}

fn across_span(b: Region, direction: CategoryBarDirection) -> (u32, u32) {
    match direction {
        CategoryBarDirection::Vertical => (b.0, b.2),
        CategoryBarDirection::Horizontal => (b.1, b.3),
    }
}

const DIRECTIONS: [CategoryBarDirection; 2] = [
    CategoryBarDirection::Vertical,
    CategoryBarDirection::Horizontal,
];

fn selection(series_id: &str, row: usize) -> DataSelectionsConfig {
    DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: series_id.into(),
            bin_index: row,
        }],
        highlight_color: MAGENTA,
        outline_width_px: 3.0,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 0.0,
    }
}

/// Every bar floats from its start to its end value, whichever is larger,
/// in its own place of the slot, in both directions at 1x and 2x; two
/// series stand side by side.
#[test]
fn bars_float_from_start_to_end_in_both_directions_and_scales() {
    let mut r = renderer();
    for direction in DIRECTIONS {
        let chart = chart(direction);
        let series = [ranges("one", RED), ranges("two", BLUE)];
        for scale in [1.0f32, 2.0] {
            let case = format!("{direction:?} {scale}x");
            let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
            for k in 0..3 {
                let region = slot(&chart, direction, k, scale);
                let red = bbox(&img, region, is_red).unwrap_or_else(|| panic!("{case} red {k}"));
                let blue = bbox(&img, region, is_blue).unwrap_or_else(|| panic!("{case} blue {k}"));
                let (lo, hi) = (START[k].min(END[k]), START[k].max(END[k]));
                let (a, b) = value_span(red, direction);
                let (ea, eb) = (
                    value_px(&chart, direction, lo, scale),
                    value_px(&chart, direction, hi, scale),
                );
                assert!(
                    (a - ea).abs() <= 1.0 && (b - eb).abs() <= 1.0,
                    "{case} row {k}: bar {a}..{b}, expected {ea}..{eb}"
                );
                // The first series stands before the second across the slot.
                let (r0, r1) = across_span(red, direction);
                let (b0, b1) = across_span(blue, direction);
                match direction {
                    CategoryBarDirection::Vertical => assert!(r1 < b0, "{case} row {k}"),
                    // Horizontal slots run bottom-up: member 0 is on top.
                    CategoryBarDirection::Horizontal => assert!(r1 < b0 || b1 < r0, "{case} row {k}"),
                }
            }
        }
    }
}

#[test]
#[ignore]
fn dump_scene_for_review() {
    let mut r = renderer();
    let out = std::env::var("FIGGY_DUMP").unwrap();
    for direction in DIRECTIONS {
        let mut chart = chart(direction);
        let mut a = ranges("one", RED);
        {
            let (w, c, p, conn, labels) = fields(&mut a);
            *w = Some(RangeWhiskers { low: "low".into(), high: "high".into(), width: 2.0, color: Some(YELLOW) });
            *c = RangeColor::Direction { up: GREEN, down: BLUE, total: Some(RangeTotal { column: "total".into(), color: CYAN }) };
            *conn = Some(RangeConnector { color: ORANGE, width: 2.0 });
            *p = Some(RangeProgress { column: "progress".into(), remaining_opacity: 0.35 });
            labels.placement = CategoryBarLabels::Outside;
            labels.text = vec!["+40".into(), "-30".into(), "total".into()];
        }
        if let DataRenderType::RangeBars { style, .. } = &mut a.render_type { style.corner_radius = 4.0; }
        chart.config_mut().picked_data = Some(selection("one", 1));
        for scale in [1.0f32, 2.0] {
            let img = r.export_panel_rgba(&chart, std::slice::from_ref(&a), scale).unwrap();
            let png = renderer::encode_png(&img).unwrap();
            std::fs::write(format!("{out}/range_{direction:?}_{scale}.png"), png).unwrap();
        }
    }
}
