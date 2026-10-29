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
                    CategoryBarDirection::Horizontal => {
                        assert!(r1 < b0 || b1 < r0, "{case} row {k}")
                    }
                }
            }
        }
    }
}

/// `series` with every option a waterfall or a candle draws: whiskers,
/// direction colours with a flagged total, a connector and progress.
fn decorated(mut series: SeriesConfig) -> SeriesConfig {
    let (whiskers, color_by, progress, connector, _) = fields(&mut series);
    *whiskers = Some(RangeWhiskers {
        low: "low".into(),
        high: "high".into(),
        width: 2.0,
        color: Some(YELLOW),
    });
    *color_by = RangeColor::Direction {
        up: GREEN,
        down: BLUE,
        total: Some(RangeTotal {
            column: "total".into(),
            color: CYAN,
        }),
    };
    *connector = Some(RangeConnector {
        color: ORANGE,
        width: 2.0,
    });
    *progress = None;
    series
}

fn registered_pick(
    r: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    scale: f32,
    at: (f32, f32),
    reach: f32,
) -> Option<(String, usize)> {
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0, at.1],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: (WIDTH as f32 * scale) as u32,
            height: (HEIGHT as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: reach,
    };
    let hit = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    match hit {
        Some(renderer::PickedData::HistogramBin {
            series_id,
            bin_index,
            ..
        }) => Some((series_id, bin_index)),
        None => None,
        Some(other) => panic!("picked {other:?}"),
    }
}

/// The direction of the given ends chooses up or down, a row whose total
/// flag reads 1 takes the total colour, and a state index takes its style
/// (a state with no style takes the series colour).
#[test]
fn direction_total_and_state_colour_each_bar() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let series = decorated(ranges("one", RED));
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_panel_rgba(&chart, std::slice::from_ref(&series), scale)
            .unwrap();
        assert!(bbox(&img, slot(&chart, direction, 0, scale), is_green).is_some());
        assert!(bbox(&img, slot(&chart, direction, 1, scale), is_blue).is_some());
        assert!(bbox(&img, slot(&chart, direction, 2, scale), is_cyan).is_some());
        assert_eq!(bbox(&img, (0, 0, img.width, img.height), is_red), None);
    }
    let mut states = ranges("one", RED);
    *fields(&mut states).1 = RangeColor::State {
        column: "state".into(),
        styles: vec![
            RangeStateStyle { color: GREEN },
            RangeStateStyle { color: BLUE },
        ],
    };
    let img = r.export_panel_rgba(&chart, &[states], 1.0).unwrap();
    // States 1, 0 and 7: the third has no style.
    assert!(bbox(&img, slot(&chart, direction, 0, 1.0), is_blue).is_some());
    assert!(bbox(&img, slot(&chart, direction, 1, 1.0), is_green).is_some());
    assert!(bbox(&img, slot(&chart, direction, 2, 1.0), is_red).is_some());
}

/// The whisker runs from the low to the high value at the bar's middle,
/// under the body; the connector runs at each bar's end from its edge to
/// the next bar's, and the last bar has none.
#[test]
fn whiskers_and_connectors_stand_where_their_values_are() {
    let mut r = renderer();
    for direction in DIRECTIONS {
        let chart = chart(direction);
        let series = [decorated(ranges("one", RED))];
        for scale in [1.0f32, 2.0] {
            let case = format!("{direction:?} {scale}x");
            let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
            for k in 0..3 {
                let region = slot(&chart, direction, k, scale);
                let wick =
                    bbox(&img, region, is_yellow).unwrap_or_else(|| panic!("{case} wick {k}"));
                let (a, b) = value_span(wick, direction);
                let (ea, eb) = (
                    value_px(&chart, direction, LOW[k], scale),
                    value_px(&chart, direction, HIGH[k], scale),
                );
                assert!(
                    (a - ea).abs() <= 1.0 && (b - eb).abs() <= 1.0,
                    "{case} wick {k}: {a}..{b}, expected {ea}..{eb}"
                );
                let (w0, w1) = across_span(wick, direction);
                assert!(w1 - w0 <= (2.0 * scale) as u32 + 1, "{case} wick {k} width");
            }
            let full = (0, 0, img.width, img.height);
            let joins = bbox(&img, full, is_orange).expect("connectors paint");
            // Both connectors sit between the first and the last bar's ends.
            let (lo, hi) = value_span(joins, direction);
            let (end0, end1) = (
                value_px(&chart, direction, END[0], scale),
                value_px(&chart, direction, END[1], scale),
            );
            let (near, far) = if direction == CategoryBarDirection::Vertical {
                (end0.max(end1), end0.min(end1))
            } else {
                (end0.min(end1), end0.max(end1))
            };
            assert!(
                (lo - near).abs() <= 2.0 * scale + 1.0 && (hi - far).abs() <= 2.0 * scale + 1.0,
                "{case}: connectors {lo}..{hi}, ends {near}..{far}"
            );
            // Nothing leaves the last bar.
            let last = slot(&chart, direction, 2, scale);
            let after = match direction {
                CategoryBarDirection::Vertical => {
                    (last.0 + (last.2 - last.0) / 2, 0, img.width, img.height)
                }
                CategoryBarDirection::Horizontal => {
                    (0, 0, img.width, last.1 + (last.3 - last.1) / 2)
                }
            };
            assert_eq!(bbox(&img, after, is_orange), None, "{case}");
        }
    }
}

/// Progress draws a bar whole up to `start + p · (end − start)` and the
/// rest at the remaining opacity; a NaN progress draws the bar whole.
#[test]
fn progress_fades_the_unfinished_part() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Horizontal;
    let chart = chart(direction);
    let mut series = ranges("one", RED);
    *fields(&mut series).2 = Some(RangeProgress {
        column: "progress".into(),
        remaining_opacity: 0.3,
    });
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_panel_rgba(&chart, std::slice::from_ref(&series), scale)
            .unwrap();
        // Row 0 runs 20 → 60 at progress 0.5: solid red to 40.
        let solid = bbox(&img, slot(&chart, direction, 0, scale), is_red).expect("the solid part");
        let split = value_px(&chart, direction, 40.0, scale);
        let (a, b) = value_span(solid, direction);
        assert!(
            (a - value_px(&chart, direction, 20.0, scale)).abs() <= 1.0,
            "{scale}x {a}"
        );
        assert!((b - split).abs() <= 1.5, "{scale}x split {b} vs {split}");
        // Past the split the bar is red at the remaining opacity (the
        // export is straight alpha over a transparent background).
        let y = (solid.1 + solid.3) / 2;
        let x = (split + 4.0 * scale) as u32;
        let p = pixel(&img, x, y);
        assert!(
            p[0] > 200 && p[1] < 40 && (66..=88).contains(&p[3]),
            "{scale}x faded {p:?}"
        );
        // A NaN progress (row 1) and progress 1 (row 2) draw whole.
        for k in [1, 2] {
            let bar = bbox(&img, slot(&chart, direction, k, scale), is_red).expect("whole");
            let (a, b) = value_span(bar, direction);
            let (lo, hi) = (START[k].min(END[k]), START[k].max(END[k]));
            assert!(
                (a - value_px(&chart, direction, lo, scale)).abs() <= 1.0,
                "{k}"
            );
            assert!(
                (b - value_px(&chart, direction, hi, scale)).abs() <= 1.0,
                "{k}"
            );
        }
    }
}

/// A row with a NaN start or end, or with equal ends, draws nothing, and
/// its whisker and connectors go with it.
#[test]
fn missing_rows_and_equal_ends_draw_nothing() {
    let mut r = renderer();
    r.add_column("gappy.start", &col(vec![20.0, NAN, 50.0]))
        .unwrap();
    r.add_column("gappy.end", &col(vec![60.0, 40.0, 50.0]))
        .unwrap();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let mut series = decorated(ranges("one", RED));
    series.y_column = "gappy.end".into();
    let DataRenderType::RangeBars {
        start, color_by, ..
    } = &mut series.render_type
    else {
        unreachable!()
    };
    *start = "gappy.start".into();
    *color_by = RangeColor::Single;
    let img = r.export_panel_rgba(&chart, &[series], 1.0).unwrap();
    assert!(bbox(&img, slot(&chart, direction, 0, 1.0), is_red).is_some());
    for k in [1, 2] {
        let region = slot(&chart, direction, k, 1.0);
        assert_eq!(bbox(&img, region, is_red), None, "row {k}");
        assert_eq!(bbox(&img, region, is_yellow), None, "row {k} whisker");
    }
    assert_eq!(bbox(&img, (0, 0, img.width, img.height), is_orange), None);
}

/// The picked highlight traces the painted body of its bar and no other,
/// and the pick answers the body, the whisker and the connector with their
/// bar, at 1x and 2x.
#[test]
fn the_highlight_traces_the_body_and_every_part_picks_its_bar() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut chart = chart(direction);
    let mut series = decorated(ranges("one", RED));
    if let DataRenderType::RangeBars { style, .. } = &mut series.render_type {
        style.corner_radius = 3.0;
    }
    let series = [series];
    for scale in [1.0f32, 2.0] {
        chart.config_mut().picked_data = None;
        let painted = r.export_panel_rgba(&chart, &series, scale).unwrap();
        let body = bbox(&painted, slot(&chart, direction, 1, scale), is_blue).expect("body");
        chart.config_mut().picked_data = Some(selection("one", 1));
        let outlined = r.export_panel_rgba(&chart, &series, scale).unwrap();
        let full = (0, 0, outlined.width, outlined.height);
        let ring = bbox(&outlined, full, is_magenta).expect("the selected bar is outlined");
        let edges = [
            (ring.0, body.0),
            (ring.1, body.1),
            (ring.2, body.2),
            (ring.3, body.3),
        ];
        assert!(
            edges.iter().all(|(a, b)| a.abs_diff(*b) <= 1),
            "{scale}x: the outline {ring:?} does not trace the body {body:?}"
        );
        chart.config_mut().picked_data = None;
        // The body, the whisker above it and the connector beside it.
        let centre = (
            (body.0 + body.2) as f32 / 2.0,
            (body.1 + body.3) as f32 / 2.0,
        );
        assert_eq!(
            registered_pick(&mut r, &chart, &series, scale, centre, 0.0),
            Some(("one".into(), 1)),
            "{scale}x body"
        );
        let wick = bbox(&painted, slot(&chart, direction, 1, scale), is_yellow).expect("wick");
        let above = ((wick.0 + wick.2) as f32 / 2.0 + 0.5, wick.1 as f32 + 1.5);
        assert_eq!(
            registered_pick(&mut r, &chart, &series, scale, above, 0.0),
            Some(("one".into(), 1)),
            "{scale}x whisker"
        );
        let join = bbox(
            &painted,
            (body.2 + 2, 0, painted.width, painted.height),
            is_orange,
        )
        .expect("the connector of row 1");
        let on = (
            (join.0 + join.2) as f32 / 2.0,
            (join.1 + join.3) as f32 / 2.0 + 0.5,
        );
        assert_eq!(
            registered_pick(&mut r, &chart, &series, scale, on, 0.0),
            Some(("one".into(), 1)),
            "{scale}x connector"
        );
        // Empty space picks nothing.
        let empty = (body.0 as f32 + 2.0, (body.1 as f32 - 40.0 * scale).max(1.0));
        assert_eq!(
            registered_pick(&mut r, &chart, &series, scale, empty, 0.0),
            None
        );
    }
}

/// The host's labels sit past each bar's end, beyond a rising end and
/// below a falling one; inside places them in the bar, and `None` draws
/// none.
#[test]
fn value_labels_sit_past_each_end() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut chart = chart(direction);
    chart.config_mut().left_y.label_style.visible = true;
    let dark = |p: &[u8]| p[3] > 200 && p[0] < 90 && p[1] < 90 && p[2] < 90;
    let mut series = ranges("one", Color::from_rgb8(255, 230, 230));
    {
        let labels = fields(&mut series).4;
        labels.placement = CategoryBarLabels::Outside;
        labels.text = vec!["+40".into(), "-30".into(), "".into()];
    }
    let img = r
        .export_panel_rgba(&chart, std::slice::from_ref(&series), 1.0)
        .unwrap();
    // Inside the data area, clear of the tick marks along its edges.
    let inner = |k: usize| {
        let (x0, y0, x1, y1) = slot(&chart, direction, k, 1.0);
        (x0 + 6, y0 + 6, x1 - 6, y1 - 6)
    };
    let ink = |k: usize| bbox(&img, inner(k), dark);
    let rising = ink(0).expect("the first label");
    assert!(
        (rising.3 as f32) < value_px(&chart, direction, END[0], 1.0),
        "{rising:?}"
    );
    let falling = ink(1).expect("the second label");
    assert!(
        (falling.1 as f32) > value_px(&chart, direction, END[1], 1.0),
        "{falling:?}"
    );
    assert_eq!(ink(2), None, "an empty string draws nothing");
    fields(&mut series).4.placement = CategoryBarLabels::None;
    let img = r
        .export_panel_rgba(&chart, std::slice::from_ref(&series), 1.0)
        .unwrap();
    for k in 0..3 {
        assert_eq!(bbox(&img, inner(k), dark), None, "{k}");
    }
}

/// A Gantt chart: horizontal bars on a categorical row axis over a time
/// value axis whose values are seconds since 1970, drawn and fitted at
/// their full precision; on a numeric position axis a bar's slot is the
/// series' `slot_width`, and the fit covers the slots.
#[test]
fn time_axes_keep_their_precision_and_fit_their_slots() {
    const DAY: f64 = 86_400.0;
    const T0: f64 = 1_767_225_600.0;
    let mut r = renderer();
    r.add_column("t.start", &col(vec![T0, T0 + 2.0 * DAY, T0 + 0.5 * DAY]))
        .unwrap();
    r.add_column(
        "t.end",
        &col(vec![T0 + DAY, T0 + 2.25 * DAY, T0 + 3.0 * DAY]),
    )
    .unwrap();
    let direction = CategoryBarDirection::Horizontal;
    let mut gantt_chart = chart(direction);
    let mut gantt = ranges("one", RED);
    gantt.y_column = "t.end".into();
    if let DataRenderType::RangeBars { start, .. } = &mut gantt.render_type {
        *start = "t.start".into();
    }
    gantt_chart
        .auto_fit_layer_union(
            r.pool(),
            renderer::LayerRef::Base,
            std::slice::from_ref(&gantt),
            0.0,
        )
        .unwrap();
    let x = &gantt_chart.config().bottom_x;
    assert_eq!((x.min, x.max), (T0, T0 + 3.0 * DAY));
    let img = r
        .export_panel_rgba(&gantt_chart, std::slice::from_ref(&gantt), 1.0)
        .unwrap();
    // The six-hour task spans a quarter of a day's pixels.
    let short = bbox(&img, slot(&gantt_chart, direction, 1, 1.0), is_red).expect("a short task");
    let day_px = value_px(&gantt_chart, direction, T0 + DAY, 1.0)
        - value_px(&gantt_chart, direction, T0, 1.0);
    let (a, b) = value_span(short, direction);
    assert!(
        ((b - a) - day_px * 0.25).abs() <= 1.0,
        "{} vs {}",
        b - a,
        day_px * 0.25
    );
    // Candles: a numeric time position axis with a slot of one day.
    let direction = CategoryBarDirection::Vertical;
    let mut candles = chart(direction);
    candles.config_mut().bottom_x.categories = None;
    r.add_column("t.pos", &col(vec![T0, T0 + DAY, T0 + 2.0 * DAY]))
        .unwrap();
    let mut bars = ranges("one", RED);
    bars.x_column = "t.pos".into();
    if let DataRenderType::RangeBars { slot_width, .. } = &mut bars.render_type {
        *slot_width = Some(DAY);
    }
    candles
        .auto_fit_layer_union(
            r.pool(),
            renderer::LayerRef::Base,
            std::slice::from_ref(&bars),
            0.0,
        )
        .unwrap();
    let x = &candles.config().bottom_x;
    assert_eq!((x.min, x.max), (T0 - 0.5 * DAY, T0 + 2.5 * DAY));
    // The value axis fits the ends, zero not forced.
    let y = &candles.config().left_y;
    assert_eq!((y.min, y.max), (20.0, 80.0));
    let img = r
        .export_panel_rgba(&candles, std::slice::from_ref(&bars), 1.0)
        .unwrap();
    let da = candles.config().data_area().unwrap();
    for k in 0..3 {
        let w = da.width as f32 / 3.0;
        let x0 = da.x as f32 + w * k as f32;
        let region = (x0 as u32, 0, (x0 + w) as u32, HEIGHT);
        let bar = bbox(&img, region, is_red).unwrap_or_else(|| panic!("candle {k}"));
        // The group covers 0.8 of the day's slot, centred on the day.
        let centre = (bar.0 + bar.2) as f32 / 2.0 + 0.5;
        assert!(
            (centre - (x0 + w * 0.5)).abs() <= 1.0,
            "{k}: {centre} vs {}",
            x0 + w * 0.5
        );
        assert!(
            ((bar.2 - bar.0 + 1) as f32 - w * 0.8).abs() <= 1.5,
            "{k}: width"
        );
    }
}

/// A logarithmic value axis draws range bars (prices).
#[test]
fn a_logarithmic_value_axis_draws_the_bars() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut chart = chart(direction);
    let config = chart.config_mut();
    config.left_y.scale = renderer::config::AxisScale::Logarithmic;
    config.left_y.min = 10.0;
    config.left_y.max = 100.0;
    let img = r
        .export_panel_rgba(&chart, &[ranges("one", RED)], 1.0)
        .unwrap();
    for k in 0..3 {
        let bar =
            bbox(&img, slot(&chart, direction, k, 1.0), is_red).unwrap_or_else(|| panic!("{k}"));
        let da = chart.config().data_area().unwrap();
        let at = |v: f64| (da.y + da.height) as f32 - ((v.log10() - 1.0) as f32) * da.height as f32;
        let (lo, hi) = (START[k].min(END[k]), START[k].max(END[k]));
        let (a, b) = value_span(bar, direction);
        assert!(
            (a - at(lo)).abs() <= 1.0 && (b - at(hi)).abs() <= 1.0,
            "{k}: {a}..{b}"
        );
    }
}

/// Replacing a column redraws the bars, and a column of a different length
/// than the positions is an error naming it.
#[test]
fn a_replaced_column_redraws_and_mismatched_columns_are_errors() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let series = [ranges("one", RED)];
    let before = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    r.remove_column("start").unwrap();
    r.add_column("start", &col(START.iter().map(|v| v - 10.0).collect()))
        .unwrap();
    let after = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let region = slot(&chart, direction, 0, 1.0);
    let (a, b) = (
        bbox(&before, region, is_red).unwrap(),
        bbox(&after, region, is_red).unwrap(),
    );
    let shift = value_px(&chart, direction, 10.0, 1.0) - value_px(&chart, direction, 20.0, 1.0);
    assert!(
        ((b.3 as f32 - a.3 as f32) - shift).abs() <= 1.0,
        "the start moved by {} for {shift}",
        b.3 as f32 - a.3 as f32
    );
    r.add_column("short", &col(vec![1.0, 2.0])).unwrap();
    let mut short = ranges("one", RED);
    if let DataRenderType::RangeBars { start, .. } = &mut short.render_type {
        *start = "short".into();
    }
    let error = r
        .export_panel_rgba(&chart, &[short], 1.0)
        .err()
        .expect("an error");
    assert!(error.to_string().contains("\"short\""), "{error}");
    let mut missing = ranges("one", RED);
    if let DataRenderType::RangeBars { start, .. } = &mut missing.render_type {
        *start = "nowhere".into();
    }
    let error = r
        .export_panel_rgba(&chart, &[missing], 1.0)
        .err()
        .expect("an error");
    assert!(error.to_string().contains("nowhere"), "{error}");
}

/// Range bar series sit on overlays and subplots with range groups of
/// their own, and the typed pick finds them on their active layers.
#[test]
fn overlays_and_subplots_draw_their_own_range_groups() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut parent = chart(direction);
    let mut overlay = renderer::OverlayLayer::default();
    // The overlay's own axis spans twice the parent's, so its bars stand
    // at half the parent's heights and both paint.
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 200.0;
    overlay.y_axis.major_spacing = 40.0;
    overlay.series = vec!["two".into()];
    let mut sub = chart(direction).config().clone();
    sub.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT,
        width: WIDTH,
        height: HEIGHT,
    });
    let config = parent.config_mut();
    config.overlays.push(overlay);
    config.subplots.push(renderer::Subplot {
        config: sub,
        series: vec!["below".into()],
        link_x: None,
        link_y: None,
    });
    let series = [
        ranges("one", RED),
        ranges("two", BLUE),
        ranges("below", GREEN),
    ];
    for scale in [1.0f32, 2.0] {
        let img = r.export_document_rgba(&parent, &series, scale).unwrap();
        let (w, h) = (img.width, img.height);
        let first = slot(&parent, direction, 0, scale);
        let red = bbox(&img, first, is_red).expect("the parent's bars");
        let blue = bbox(&img, first, is_blue).expect("the overlay's bars");
        assert!(
            blue.0.abs_diff(red.0) <= 1 && blue.2.abs_diff(red.2) <= 1,
            "{scale}x: {red:?} {blue:?}"
        );
        let green = bbox(&img, (first.0, h / 2, first.2, h), is_green).expect("the subplot's");
        assert_eq!(bbox(&img, (0, 0, w, h / 2), is_green), None);
        r.enable_gpu_picking().unwrap();
        let id = r
            .register_chart(parent.config().clone(), series.to_vec())
            .unwrap();
        let pick = |r: &mut Renderer, x: u32, y: u32| {
            let request = renderer::GpuPickRequest {
                canvas_position_px: [x as f32 + 0.5, y as f32 + 0.5],
                display_panel_px: Rect {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                },
                display_scale: scale,
                max_distance_px: 0.0,
            };
            match pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap() {
                Some(renderer::PickedData::HistogramBin {
                    series_id,
                    bin_index,
                    ..
                }) => Some((series_id, bin_index)),
                _ => None,
            }
        };
        let (x, y) = (blue.0 + 1, blue.3 - 2);
        r.set_active_layer(id, renderer::LayerRef::Overlay(0))
            .unwrap();
        assert_eq!(pick(&mut r, x, y), Some(("two".into(), 0)), "{scale}x");
        r.set_active_layer(id, renderer::LayerRef::Subplot(0))
            .unwrap();
        let (x, y) = (green.0 + 1, green.3 - 2);
        assert_eq!(pick(&mut r, x, y).map(|p| p.0).as_deref(), Some("below"));
        r.remove_chart(id).unwrap();
    }
}

/// A 2x export is the 1x layout doubled: every bar's extent at 2x is twice
/// its 1x extent, within the antialiasing pixel.
#[test]
fn a_2x_export_is_the_1x_layout_doubled() {
    let mut r = renderer();
    for direction in DIRECTIONS {
        let chart = chart(direction);
        let series = [decorated(ranges("one", RED))];
        let one = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let two = r.export_panel_rgba(&chart, &series, 2.0).unwrap();
        for (pred, what) in [
            (is_green as fn(&[u8]) -> bool, "up"),
            (is_blue, "down"),
            (is_cyan, "total"),
            (is_yellow, "wick"),
        ] {
            let a = bbox(&one, (0, 0, one.width, one.height), pred).expect(what);
            let b = bbox(&two, (0, 0, two.width, two.height), pred).expect(what);
            for (x1, x2) in [
                (a.0, b.0),
                (a.1, b.1),
                (a.2 + 1, b.2 + 1),
                (a.3 + 1, b.3 + 1),
            ] {
                assert!(
                    (x2 as i64 - 2 * x1 as i64).abs() <= 2,
                    "{direction:?} {what}: {a:?} {b:?}"
                );
            }
        }
    }
}

/// No count caps a range bar series (rule 10): three thousand rows on a
/// numeric position axis draw, the last one picks, and the highlight
/// outlines it, past every size the bounded charts once capped (64
/// categories, 512 bars).
#[test]
fn three_thousand_rows_draw_pick_and_highlight() {
    const N: usize = 3_000;
    const WIDE: u32 = 7_600;
    let mut r = renderer();
    r.add_column("n.pos", &col((0..N).map(|k| k as f64).collect()))
        .unwrap();
    r.add_column(
        "n.start",
        &col((0..N).map(|k| 10.0 + (k % 7) as f64).collect()),
    )
    .unwrap();
    r.add_column(
        "n.end",
        &col((0..N).map(|k| 60.0 + (k % 11) as f64).collect()),
    )
    .unwrap();
    let mut chart = chart(CategoryBarDirection::Vertical);
    let config = chart.config_mut();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDE,
        height: HEIGHT,
    });
    config.bottom_x.categories = None;
    config.bottom_x.min = -0.5;
    config.bottom_x.max = N as f64 - 0.5;
    config.bar_groups.as_mut().unwrap().bar_gap = 0.0;
    config.bar_groups.as_mut().unwrap().group_width = 0.95;
    let mut bars = ranges("one", RED);
    bars.x_column = "n.pos".into();
    bars.y_column = "n.end".into();
    if let DataRenderType::RangeBars {
        start, slot_width, ..
    } = &mut bars.render_type
    {
        *start = "n.start".into();
        *slot_width = Some(1.0);
    }
    let series = [bars];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let da = chart.config().data_area().unwrap();
    let x_of = |k: usize| da.x as f32 + (k as f32 + 0.5) / N as f32 * da.width as f32;
    let y = value_px(&chart, CategoryBarDirection::Vertical, 40.0, 1.0) as u32;
    // Bars about two pixels wide: any red ink of the bar's own pixels.
    let reddish = |p: &[u8]| p[3] > 200 && p[0] > 100 && p[1] < 40 && p[2] < 40;
    let red_near = |k: usize| {
        let x = x_of(k) as u32;
        (x.saturating_sub(1)..=x + 1).any(|x| reddish(pixel(&img, x, y)))
    };
    for k in [0, 63, 64, 511, 512, N / 2, N - 1] {
        assert!(red_near(k), "bar {k} paints");
    }
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [x_of(N - 1), y as f32 + 0.5],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: WIDE,
            height: HEIGHT,
        },
        display_scale: 1.0,
        max_distance_px: 0.0,
    };
    let started = std::time::Instant::now();
    let hit = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    eprintln!("range bar pick over {N} rows: {:?}", started.elapsed());
    assert!(
        matches!(&hit, Some(renderer::PickedData::HistogramBin { bin_index, .. }) if *bin_index == N - 1),
        "{hit:?}"
    );
    r.remove_chart(id).unwrap();
    chart.config_mut().picked_data = Some(selection("one", N - 1));
    let outlined = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let x = x_of(N - 1) as u32;
    assert!(bbox(&outlined, (x - 3, 0, x + 4, HEIGHT), is_magenta).is_some());
    assert_eq!(bbox(&outlined, (0, 0, x - 10, HEIGHT), is_magenta), None);
}
