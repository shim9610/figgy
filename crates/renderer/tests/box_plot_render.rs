//! End-to-end pixel verification of box plot series
//! (`DataRenderType::BoxPlot`) through the public renderer API and the
//! headless `export_panel_rgba`: boxes over a record and a ragged outlier
//! column in both directions at 1x and 2x, the median, whiskers, caps, mean
//! and outliers, notches, missing boxes and clamped statistics, per-box
//! overrides and materials, the selected box's outline, the value-axis fit
//! from the pool's statistics, record replacement, overlays and subplots,
//! the annotation layer over categories and values, and a released document
//! opened through `Config::from_boxplot`.
//!
//! Every part has its own pure colour: the first series' boxes red, the
//! second's blue, medians green, whiskers and caps yellow, means cyan and
//! outliers orange, all flat with no outline, so a colour predicate
//! attributes every pixel to one part. Expected extents come from the same
//! data area and axis ranges the renderer draws with.
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::Category;
use renderer::data::Column;
use renderer::data_config::{
    BarGroups, BoxPlotOverride, BoxPlotSeriesLabels, BoxPlotStyle, BoxPlotTarget,
    CategoryBarDirection, CategoryBarMode,
};
use renderer::layout::{ChartArea, Rect};
use renderer::{
    BOX_PLOT_RECORD, Chart, Color, ColumnSource, DataRenderType, DataSelectionsConfig, LayerRef,
    Material, PickedDataRef, RaggedSource, RasterImage, RecordSource, Renderer, RendererDevice,
    SeriesConfig,
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

/// One box: whisker low, q1, median, q3, whisker high, mean, CI low, CI
/// high.
type Stats = [f64; 8];
/// The first series' boxes over the categories a, b, c, and its outliers.
const ONE: [Stats; 3] = [
    [10.0, 20.0, 30.0, 40.0, 50.0, 35.0, 26.0, 34.0],
    [20.0, 30.0, 45.0, 60.0, 70.0, 50.0, 40.0, 50.0],
    [5.0, 15.0, 25.0, 35.0, 45.0, NAN, NAN, NAN],
];
const ONE_OUTLIERS: [&[f64]; 3] = [&[70.0, 85.0], &[90.0], &[]];
const TWO: [Stats; 3] = [
    [15.0, 25.0, 35.0, 45.0, 55.0, NAN, NAN, NAN],
    [25.0, 35.0, 50.0, 65.0, 75.0, NAN, NAN, NAN],
    [10.0, 20.0, 30.0, 40.0, 50.0, NAN, NAN, NAN],
];

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

/// Upload `stats` as the box plot record `id`: the summary fields, the mean
/// and the interval (no sample count).
fn upload_record(r: &mut Renderer, id: &str, stats: &[Stats]) {
    let field = |k: usize| col(stats.iter().map(|s| s[k]).collect());
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
        .map(|(name, column)| (*name, column as &dyn ColumnSource))
        .collect();
    r.upsert_record(
        id,
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
    )
    .unwrap();
}

fn upload_outliers(r: &mut Renderer, id: &str, groups: &[&[f64]]) {
    let values = col(groups.iter().flat_map(|g| g.iter().copied()).collect());
    let mut offsets = vec![0.0];
    for g in groups {
        offsets.push(offsets.last().unwrap() + g.len() as f64);
    }
    r.upsert_ragged(
        id,
        &RaggedSource {
            values: &values,
            offsets: &col(offsets),
        },
    )
    .unwrap();
}

/// A renderer holding the positions, the two records and the first
/// series' outliers.
fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("pos", &col(vec![0.0, 1.0, 2.0])).unwrap();
    upload_record(&mut r, "one.record", &ONE);
    upload_record(&mut r, "two.record", &TWO);
    upload_outliers(&mut r, "one.outliers", &ONE_OUTLIERS);
    r
}

fn flat_style() -> BoxPlotStyle {
    BoxPlotStyle {
        material: Material::Flat,
        corner_radius: 0.0,
        outline: false,
        median_width: 2.0,
        median_color: GREEN,
        whisker_width: 2.0,
        whisker_color: YELLOW,
        caps: true,
        cap_ratio: 0.5,
        notched: false,
        show_mean: true,
        mean_size: 8.0,
        mean_color: CYAN,
        show_outliers: true,
        outlier_size: 8.0,
        outlier_color: ORANGE,
        outlier_filled: true,
        ..BoxPlotStyle::default()
    }
}

fn boxes(id: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "pos".into(),
        y_column: format!("{id}.record"),
        render_type: DataRenderType::BoxPlot {
            color,
            style: flat_style(),
            labels: BoxPlotSeriesLabels::default(),
            outliers: (id == "one").then(|| "one.outliers".into()),
            overrides: None,
        },
    }
}

fn style(series: &mut SeriesConfig) -> &mut BoxPlotStyle {
    let DataRenderType::BoxPlot { style, .. } = &mut series.render_type else {
        unreachable!()
    };
    style
}

fn overrides(series: &mut SeriesConfig) -> &mut Vec<BoxPlotOverride> {
    let DataRenderType::BoxPlot { overrides, .. } = &mut series.render_type else {
        unreachable!()
    };
    overrides.get_or_insert_with(Vec::new)
}

/// Grid, legend, title and axis lines off; three categories on the
/// direction's category axis, bottom-up; the value axis over 0..100.
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

/// `region` narrowed across the category axis to `lo..=hi`.
fn across_of(region: Region, (lo, hi): (u32, u32), direction: CategoryBarDirection) -> Region {
    match direction {
        CategoryBarDirection::Vertical => (lo, region.1, hi + 1, region.3),
        CategoryBarDirection::Horizontal => (region.0, lo, region.2, hi + 1),
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

/// The value-axis span `(low value end, high value end)` in export pixels.
fn value_span(b: Region, direction: CategoryBarDirection) -> (f32, f32) {
    match direction {
        CategoryBarDirection::Vertical => (b.3 as f32 + 1.0, b.1 as f32),
        CategoryBarDirection::Horizontal => (b.0 as f32, b.2 as f32 + 1.0),
    }
}

/// The value-axis centre of a box.
fn value_centre(b: Region, direction: CategoryBarDirection) -> f32 {
    let (a, c) = value_span(b, direction);
    (a + c) * 0.5
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

/// Every part of a box stands where its statistic is, in its own series'
/// place of the category slot, in both directions at 1x and 2x: the body
/// from q1 to q3, the median across it, the whiskers from the box out to
/// their ends with caps of half the box's thickness, the mean and the
/// outliers at their values on the box's middle.
#[test]
fn boxes_draw_their_statistics_in_both_directions_and_scales() {
    let mut r = renderer();
    for direction in DIRECTIONS {
        let chart = chart(direction);
        let series = [boxes("one", RED), boxes("two", BLUE)];
        for scale in [1.0f32, 2.0] {
            let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
            let px = |v: f64| value_px(&chart, direction, v, scale);
            let slack = 2.0 * scale;
            let near = |(a, b): (f32, f32), (c, d): (f32, f32), slack: f32| {
                (a - c).abs() <= slack && (b - d).abs() <= slack
            };
            for k in 0..3 {
                let case = format!("{direction:?} {scale}x category {k}");
                let region = slot(&chart, direction, k, scale);
                let red = bbox(&img, region, is_red).unwrap_or_else(|| panic!("{case}: no red"));
                let blue = bbox(&img, region, is_blue).unwrap_or_else(|| panic!("{case}: no blue"));
                let [low, q1, median, q3, high, mean, ..] = ONE[k];
                assert!(
                    near(value_span(red, direction), (px(q1), px(q3)), slack),
                    "{case}: body {:?} for {}..{}",
                    value_span(red, direction),
                    px(q1),
                    px(q3)
                );
                let two = TWO[k];
                assert!(near(
                    value_span(blue, direction),
                    (px(two[1]), px(two[3])),
                    slack
                ));
                // Side by side in series order, the gap between them.
                let (r_across, b_across) =
                    (across_span(red, direction), across_span(blue, direction));
                let first_then_second = match direction {
                    CategoryBarDirection::Vertical => r_across.1 + 2 < b_across.0,
                    // Bottom-up categories, series top-down in their slot.
                    CategoryBarDirection::Horizontal => r_across.1 + 2 < b_across.0,
                };
                assert!(first_then_second, "{case}: {r_across:?} {b_across:?}");
                let thickness = (r_across.1 - r_across.0 + 1) as f32;
                let column = across_of(region, r_across, direction);
                let green = bbox(&img, column, is_green).expect("the median");
                assert!(
                    (value_centre(green, direction) - px(median)).abs() <= 1.5 * scale,
                    "{case}: median at {} for {}",
                    value_centre(green, direction),
                    px(median)
                );
                assert!(across_span(green, direction).0.abs_diff(r_across.0) <= 1);
                assert!(across_span(green, direction).1.abs_diff(r_across.1) <= 1);
                let yellow = bbox(&img, column, is_yellow).expect("the whiskers");
                let whisker = 1.0 * scale;
                assert!(
                    near(
                        value_span(yellow, direction),
                        (px(low), px(high)),
                        whisker + slack
                    ),
                    "{case}: whiskers {:?} for {}..{}",
                    value_span(yellow, direction),
                    px(low),
                    px(high)
                );
                let caps = across_span(yellow, direction);
                let cap = (caps.1 - caps.0 + 1) as f32;
                assert!(
                    (cap - thickness * 0.5).abs() <= 2.0,
                    "{case}: cap {cap} for a box of {thickness}"
                );
                match bbox(&img, column, is_cyan) {
                    Some(cyan) => assert!(
                        (value_centre(cyan, direction) - px(mean)).abs() <= 1.5 * scale,
                        "{case}: mean"
                    ),
                    None => assert!(mean.is_nan(), "{case}: the mean is drawn"),
                }
                // The outliers: orange where the first series has some.
                let orange = bbox(&img, column, is_orange);
                match ONE_OUTLIERS[k] {
                    [] => assert_eq!(orange, None, "{case}"),
                    values => {
                        let orange = orange.expect("the outliers");
                        let lo = px(values[0]);
                        let hi = px(*values.last().unwrap());
                        let r = 4.0 * scale + slack;
                        let (a, b) = value_span(orange, direction);
                        assert!(
                            (a - lo).abs() <= r && (b - hi).abs() <= r,
                            "{case}: outliers {a}..{b} for {lo}..{hi}"
                        );
                    }
                }
                // The second series has no outliers and no means.
                let other = across_of(region, b_across, direction);
                assert_eq!(bbox(&img, other, is_orange), None, "{case}");
                assert_eq!(bbox(&img, other, is_cyan), None, "{case}");
            }
        }
    }
}

/// A notched box narrows to the notch depth at its median and keeps its
/// full thickness at q1 and q3; a box without a confidence interval stays
/// plain.
#[test]
fn notched_boxes_narrow_at_the_median() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let mut one = boxes("one", RED);
    style(&mut one).notched = true;
    style(&mut one).notch_depth = 0.3;
    style(&mut one).median_width = 0.0;
    style(&mut one).show_mean = false;
    for scale in [1.0f32, 2.0] {
        let img = r.export_panel_rgba(&chart, &[one.clone()], scale).unwrap();
        let width_at = |region: Region, v: f64| {
            let y = value_px(&chart, direction, v, scale) as u32;
            (region.0..region.2)
                .filter(|&x| is_red(pixel(&img, x, y)))
                .count() as f32
        };
        let region = slot(&chart, direction, 0, scale);
        let [_, q1, median, ..] = ONE[0];
        let full = width_at(region, q1 + 2.0);
        let notch = width_at(region, median);
        assert!(
            (full - notch - 2.0 * 0.3 * full).abs() <= 3.0,
            "{scale}x: {full} wide, {notch} at the notch"
        );
        // Category c has no interval: a plain box.
        let region = slot(&chart, direction, 2, scale);
        let [_, q1, median, ..] = ONE[2];
        assert!((width_at(region, q1 + 2.0) - width_at(region, median)).abs() <= 1.0);
    }
}

/// A missing box (a row of NaN) draws nothing, and statistics the record
/// does not order are clamped where they are drawn: q1 above q3 draws the
/// same body, a median outside the box sits on its edge, and a whisker end
/// inside the box draws no whisker.
#[test]
fn missing_boxes_draw_nothing_and_unordered_statistics_are_clamped() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let ordered = [
        [10.0, 20.0, 30.0, 40.0, 50.0, NAN, NAN, NAN],
        [NAN; 8],
        [10.0, 20.0, 30.0, 40.0, 50.0, NAN, NAN, NAN],
    ];
    // a: q1 and q3 swapped; c: the median above q3 and the low whisker end
    // inside the box.
    let unordered = [
        [10.0, 40.0, 30.0, 20.0, 50.0, NAN, NAN, NAN],
        [NAN; 8],
        [30.0, 20.0, 60.0, 40.0, 50.0, NAN, NAN, NAN],
    ];
    upload_record(&mut r, "ordered.record", &ordered);
    upload_record(&mut r, "unordered.record", &unordered);
    let mut a = boxes("ordered", RED);
    let mut b = boxes("unordered", RED);
    for s in [&mut a, &mut b] {
        style(s).show_mean = false;
    }
    for scale in [1.0f32, 2.0] {
        let first = r.export_panel_rgba(&chart, &[a.clone()], scale).unwrap();
        let second = r.export_panel_rgba(&chart, &[b.clone()], scale).unwrap();
        let px = |v: f64| value_px(&chart, direction, v, scale);
        let slot = |k| slot(&chart, direction, k, scale);
        assert_eq!(bbox(&first, slot(1), is_red), None);
        assert_eq!(bbox(&first, slot(1), is_yellow), None);
        assert_eq!(bbox(&first, slot(1), is_green), None);
        // q1 > q3 draws the ordered box.
        assert_eq!(
            bbox(&first, slot(0), is_red),
            bbox(&second, slot(0), is_red),
            "{scale}x"
        );
        // The median clamps to q3; the low whisker is gone, the high one
        // stays.
        let green = bbox(&second, slot(2), is_green).unwrap();
        assert!((value_centre(green, direction) - px(40.0)).abs() <= 1.5 * scale);
        let yellow = bbox(&second, slot(2), is_yellow).unwrap();
        let (low, high) = value_span(yellow, direction);
        assert!(
            low <= px(20.0) + 2.0 * scale && (high - px(50.0)).abs() <= 3.0 * scale,
            "{scale}x: whiskers {low}..{high}"
        );
        assert!(
            low >= px(20.0) - 3.0 * scale,
            "{scale}x: no low whisker: {low}"
        );
    }
}

/// A per-box override recolours and restyles its own box only: a colour
/// override keeps the box's geometry, a style override changes its parts
/// (here the median colour and the material), and a series material
/// changes the shading inside the boxes but not their extents.
#[test]
fn overrides_and_materials_restyle_only_their_box() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let plain = [boxes("one", RED), boxes("two", BLUE)];
    let mut overridden = plain.clone();
    let mut o = BoxPlotOverride::new(BoxPlotTarget::new("b", "one"));
    o.color = Some(GREEN);
    overrides(&mut overridden[0]).push(o);
    for scale in [1.0f32, 2.0] {
        let before = r.export_panel_rgba(&chart, &plain, scale).unwrap();
        let after = r.export_panel_rgba(&chart, &overridden, scale).unwrap();
        let region = slot(&chart, direction, 1, scale);
        let red = bbox(&before, region, is_red).unwrap();
        let column = across_of(region, across_span(red, direction), direction);
        let green_body = bbox(&after, column, is_green).expect("the recoloured box");
        assert_eq!(green_body, red, "{scale}x: the override keeps the geometry");
        assert_eq!(bbox(&after, region, is_red), None);
        for k in [0, 2] {
            let region = slot(&chart, direction, k, scale);
            assert_eq!(
                bbox(&after, region, is_red),
                bbox(&before, region, is_red),
                "{scale}x category {k}"
            );
        }
    }

    // A style override: its median is magenta and its body Matte.
    let mut styled = plain.clone();
    let mut o = BoxPlotOverride::new(BoxPlotTarget::new("a", "two"));
    o.style = Some(BoxPlotStyle {
        median_color: MAGENTA,
        caps: false,
        ..flat_style()
    });
    overrides(&mut styled[1]).push(o);
    let before = r.export_panel_rgba(&chart, &plain, 1.0).unwrap();
    let after = r.export_panel_rgba(&chart, &styled, 1.0).unwrap();
    let region = slot(&chart, direction, 0, 1.0);
    let blue = bbox(&before, region, is_blue).unwrap();
    let column = across_of(region, across_span(blue, direction), direction);
    assert!(bbox(&after, column, is_magenta).is_some());
    assert_eq!(bbox(&before, column, is_magenta), None);
    // Caps off: the whisker is a stem only, narrower than the caps were.
    let caps = across_span(bbox(&before, column, is_yellow).unwrap(), direction);
    let stem = across_span(bbox(&after, column, is_yellow).unwrap(), direction);
    assert!(stem.1 - stem.0 + 4 < caps.1 - caps.0, "{caps:?} {stem:?}");
    for k in [1, 2] {
        let region = slot(&chart, direction, k, 1.0);
        assert_eq!(
            bbox(&after, region, is_magenta),
            None,
            "category {k} keeps its style"
        );
    }

    // A grained material shades the boxes' insides; the extents stay.
    let mut matte = plain.clone();
    for s in &mut matte {
        style(s).material = Material::Matte;
        style(s).texture_strength = 1.0;
    }
    let flat = r.export_panel_rgba(&chart, &plain, 1.0).unwrap();
    let grained = r.export_panel_rgba(&chart, &matte, 1.0).unwrap();
    assert_ne!(flat.rgba, grained.rgba, "the material changes the shading");
    let near_red = |p: &[u8]| p[3] > 200 && p[0] > 150 && p[1] < 110 && p[2] < 110;
    for k in 0..3 {
        let region = slot(&chart, direction, k, 1.0);
        assert_eq!(
            bbox(&flat, region, is_red),
            bbox(&grained, region, near_red)
        );
    }
}

/// The selected box's outline traces its body as drawn, overrides
/// included: a box whose override rounds its corners is outlined inside
/// it, and no other box is touched.
#[test]
fn the_selection_outline_traces_an_overridden_box() {
    let mut r = renderer();
    for direction in DIRECTIONS {
        let mut chart = chart(direction);
        let mut series = [boxes("one", RED), boxes("two", BLUE)];
        // No green medians: green is the overridden body's colour here.
        for s in &mut series {
            style(s).median_width = 0.0;
        }
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("b", "two"));
        o.color = Some(GREEN);
        o.style = Some(BoxPlotStyle {
            corner_radius: 3.0,
            median_width: 0.0,
            show_mean: false,
            ..flat_style()
        });
        overrides(&mut series[1]).push(o);
        for scale in [1.0f32, 2.0] {
            let case = format!("{direction:?} {scale}x");
            chart.config_mut().picked_data = None;
            let painted = r.export_panel_rgba(&chart, &series, scale).unwrap();
            let region = slot(&chart, direction, 1, scale);
            let body = bbox(&painted, region, is_green).expect("the overridden box paints");
            chart.config_mut().picked_data = Some(DataSelectionsConfig {
                visible: true,
                refs: vec![PickedDataRef::HistogramBin {
                    source_id: None,
                    series_id: "two".into(),
                    bin_index: 1,
                }],
                highlight_color: MAGENTA,
                outline_width_px: 3.0,
                point_radius_extra_px: 0.0,
                contour_width_extra_px: 0.0,
            });
            let outlined = r.export_panel_rgba(&chart, &series, scale).unwrap();
            let full = (0, 0, outlined.width, outlined.height);
            let ring = bbox(&outlined, full, is_magenta).expect("the selected box is outlined");
            let edges = [
                (ring.0, body.0),
                (ring.1, body.1),
                (ring.2, body.2),
                (ring.3, body.3),
            ];
            assert!(
                edges.iter().all(|(a, b)| a.abs_diff(*b) <= 1),
                "{case}: the outline {ring:?} does not trace the painted box {body:?}"
            );
            // Rounded corners: the outer corner pixel is not ring.
            assert!(!is_magenta(pixel(&outlined, ring.0, ring.1)), "{case}");
            for k in [0, 2] {
                let other = slot(&chart, direction, k, scale);
                assert_eq!(
                    bbox(&outlined, other, is_magenta),
                    None,
                    "{case} category {k}"
                );
            }
        }
    }
}

/// The value-axis fit covers every statistic the records and the outlier
/// column hold, from the pool's statistics: the whisker ends, the means
/// and intervals and the outliers, hidden or not.
#[test]
fn the_value_axis_fits_the_record_and_outlier_statistics() {
    let r = renderer();
    for direction in DIRECTIONS {
        let mut chart = chart(direction);
        let mut series = [boxes("one", RED), boxes("two", BLUE)];
        // Hidden outliers still count, as they did.
        style(&mut series[0]).show_outliers = false;
        chart
            .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
            .unwrap();
        let config = chart.config();
        let (min, max) = match direction {
            CategoryBarDirection::Vertical => (config.left_y.min, config.left_y.max),
            CategoryBarDirection::Horizontal => (config.bottom_x.min, config.bottom_x.max),
        };
        assert!(min <= 5.0 && min > 0.0, "{direction:?}: {min}");
        assert!(max >= 90.0 && max < 95.0, "{direction:?}: {max}");
        // The category axis keeps its category range.
        let category = match direction {
            CategoryBarDirection::Vertical => &config.bottom_x,
            CategoryBarDirection::Horizontal => &config.left_y,
        };
        assert_eq!(category.data_range(), (-0.5, 2.5));
        // Without the outliers the second series' whiskers decide.
        let mut chart = self::chart(direction);
        chart
            .auto_fit_layer_union(r.pool(), LayerRef::Base, &series[1..], 0.0)
            .unwrap();
        let config = chart.config();
        let (min, max) = match direction {
            CategoryBarDirection::Vertical => (config.left_y.min, config.left_y.max),
            CategoryBarDirection::Horizontal => (config.bottom_x.min, config.bottom_x.max),
        };
        assert_eq!((min, max), (10.0, 75.0), "{direction:?}");
    }
}

/// Replacing a record redraws its boxes (the record's member columns are
/// part of the panel's column fingerprint), and a record or outlier column
/// that does not hold one row or group per category is an explicit error.
#[test]
fn a_replaced_record_redraws_and_mismatched_sources_are_errors() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let series = [boxes("one", RED), boxes("two", BLUE)];
    let before = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let mut moved = ONE;
    for row in &mut moved {
        for v in row.iter_mut() {
            *v += 5.0;
        }
    }
    upload_record(&mut r, "one.record", &moved);
    let after = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let region = slot(&chart, direction, 0, 1.0);
    let (a, b) = (
        bbox(&before, region, is_red).unwrap(),
        bbox(&after, region, is_red).unwrap(),
    );
    let shift = value_px(&chart, direction, 0.0, 1.0) - value_px(&chart, direction, 5.0, 1.0);
    assert!(
        ((a.1 as f32 - b.1 as f32) - shift).abs() <= 1.0,
        "the box moved by {} for {shift}",
        a.1 as f32 - b.1 as f32
    );
    // Two rows for three categories.
    upload_record(&mut r, "short.record", &ONE[..2]);
    let mut short = boxes("short", RED);
    let DataRenderType::BoxPlot { outliers, .. } = &mut short.render_type else {
        unreachable!()
    };
    *outliers = None;
    let error = r
        .export_panel_rgba(&chart, &[short], 1.0)
        .err()
        .expect("an error");
    assert!(error.to_string().contains("one per category"), "{error}");
    // Two groups for three categories.
    upload_outliers(&mut r, "one.outliers", &ONE_OUTLIERS[..2]);
    let error = r
        .export_panel_rgba(&chart, &series, 1.0)
        .err()
        .expect("an error");
    assert!(error.to_string().contains("groups"), "{error}");
    upload_outliers(&mut r, "one.outliers", &ONE_OUTLIERS);
    // A record that is not uploaded.
    let missing = SeriesConfig {
        y_column: "nowhere".into(),
        ..boxes("two", BLUE)
    };
    let error = r
        .export_panel_rgba(&chart, &[missing], 1.0)
        .err()
        .expect("an error");
    assert!(error.to_string().contains("nowhere"), "{error}");
}

/// Box plot series sit on overlays and subplots with their own box groups,
/// and the typed pick finds them on their active layers.
#[test]
fn overlays_and_subplots_draw_their_own_box_groups() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut parent = chart(direction);
    let mut overlay = renderer::OverlayLayer::default();
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 100.0;
    overlay.y_axis.major_spacing = 20.0;
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
    let mut below = boxes("below", GREEN);
    below.y_column = "one.record".into();
    style(&mut below).median_color = MAGENTA;
    let mut series = [boxes("one", RED), boxes("two", BLUE), below];
    // Green is the subplot's body colour here.
    for s in &mut series {
        style(s).median_color = MAGENTA;
    }
    for scale in [1.0f32, 2.0] {
        let img = r.export_document_rgba(&parent, &series, scale).unwrap();
        let (w, h) = (img.width, img.height);
        let top = (0, 0, w, h / 2);
        let bottom = (0, h / 2, w, h);
        let first = slot(&parent, direction, 0, scale);
        let red = bbox(&img, first, is_red).expect("the parent's boxes");
        let blue = bbox(&img, first, is_blue).expect("the overlay's boxes");
        // The overlay is a box group of its own: both layers hold one
        // series, so each box takes the whole group width in the same place.
        assert!(
            blue.0.abs_diff(red.0) <= 1 && blue.2.abs_diff(red.2) <= 1,
            "{scale}x: {red:?} {blue:?}"
        );
        assert_eq!(bbox(&img, bottom, is_red), None);
        let green =
            bbox(&img, (first.0, h / 2, first.2, h), is_green).expect("the subplot's boxes");
        assert_eq!(bbox(&img, top, is_green), None);

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
        // A blue pixel of the overlay's first box.
        let blue_first = bbox(&img, (first.0, 0, first.2, h / 2), is_blue).unwrap();
        let (x, y) = (blue_first.0 + 1, blue_first.3 - 2);
        assert!(is_blue(pixel(&img, x, y)));
        r.set_active_layer(id, LayerRef::Overlay(0)).unwrap();
        assert_eq!(pick(&mut r, x, y), Some(("two".into(), 0)), "{scale}x");
        r.set_active_layer(id, LayerRef::Subplot(0)).unwrap();
        let (x, y) = (green.0 + 1, green.3 - 2);
        assert!(is_green(pixel(&img, x, y)));
        assert_eq!(pick(&mut r, x, y).map(|p| p.0).as_deref(), Some("below"));
        r.remove_chart(id).unwrap();
    }
}

/// A released box plot document read by `Config::from_boxplot` draws
/// through this path: its record and outlier uploads borrow the document's
/// summaries, its override colour is drawn, and its boxes paint at 1x and
/// 2x.
#[cfg(feature = "serde")]
#[test]
fn a_box_plot_document_draws_through_the_reader() {
    use renderer::UploadValues;
    let doc: renderer::BoxPlotChart = serde_json::from_str(include_str!(
        "fixtures/released/model-0.12.1-boxplot-edited.json"
    ))
    .unwrap();
    let (mut config, series, uploads) = renderer::config::Config::from_boxplot(&doc);
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    });
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();
    for upload in &uploads {
        match &upload.values {
            UploadValues::BoxRecord(_) => {
                let fields = upload.values.box_fields().unwrap();
                let pairs: [(&str, &dyn ColumnSource); 9] = std::array::from_fn(|k| {
                    (BOX_PLOT_RECORD[k].0, &fields[k] as &dyn ColumnSource)
                });
                r.upsert_record(
                    upload.column.clone(),
                    &RecordSource {
                        schema: BOX_PLOT_RECORD,
                        fields: &pairs,
                    },
                )
                .unwrap();
            }
            UploadValues::BoxOutliers(_) => {
                let (values, offsets) = upload.values.box_outliers().unwrap();
                r.upsert_ragged(
                    upload.column.clone(),
                    &RaggedSource {
                        values: &values,
                        offsets: &offsets,
                    },
                )
                .unwrap();
            }
            values => {
                r.add_column(upload.column.clone(), values).unwrap();
            }
        }
    }
    let chart = Chart::new(config);
    for scale in [1.0f32, 2.0] {
        let with = r.export_panel_rgba(&chart, &series, scale).unwrap();
        let without = r.export_panel_rgba(&chart, &[], scale).unwrap();
        // The document's green override (c2, b) is drawn.
        let green = |p: &[u8]| {
            p[3] > 200 && (30..60).contains(&p[0]) && (140..175).contains(&p[1]) && p[2] < 70
        };
        let full = (0, 0, with.width, with.height);
        assert!(
            bbox(&with, full, green).is_some(),
            "{scale}x: the override paints"
        );
        let changed = with
            .rgba
            .chunks(4)
            .zip(without.rgba.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > (5000.0 * scale * scale) as usize,
            "{scale}x: {changed}"
        );
    }
}

/// The annotation layer of a chart of box plots maps its category axis by
/// category index and its value axis by value, in both directions at 1x
/// and 2x: a span over the first category's slot (`-0.5..0.5`) fills that
/// slot only and lies under its boxes, and a reference line at a value
/// crosses the other slots at that value.
#[test]
fn the_annotation_layer_maps_categories_and_values() {
    use renderer::{Annotation, AnnotationAxis, AnnotationKind, AnnotationLineStyle};
    let mut r = renderer();
    for direction in DIRECTIONS {
        let mut chart = chart(direction);
        let (category_axis, value_axis) = match direction {
            CategoryBarDirection::Vertical => (AnnotationAxis::X, AnnotationAxis::Y),
            CategoryBarDirection::Horizontal => (AnnotationAxis::Y, AnnotationAxis::X),
        };
        chart.config_mut().annotations.items = vec![
            Annotation::new(AnnotationKind::Span {
                axis: category_axis,
                from: -0.5,
                to: 0.5,
                fill: MAGENTA,
            }),
            Annotation::new(AnnotationKind::ReferenceLine {
                axis: value_axis,
                value: 90.0,
                style: AnnotationLineStyle {
                    color: Color::from_rgb8(0, 0, 0),
                    width: 2.0,
                    ..AnnotationLineStyle::default()
                },
                label: None,
            }),
        ];
        let series = [boxes("one", RED), boxes("two", BLUE)];
        for scale in [1.0f32, 2.0] {
            let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
            let case = format!("{direction:?} {scale}x");
            // The export pixel at value `v` in the middle of slot `k`.
            let at = |k: usize, v: f64| {
                let region = slot(&chart, direction, k, scale);
                let along = value_px(&chart, direction, v, scale).round() as u32;
                match direction {
                    CategoryBarDirection::Vertical => ((region.0 + region.2) / 2, along),
                    CategoryBarDirection::Horizontal => (along, (region.1 + region.3) / 2),
                }
            };
            // Below every whisker: the span fills the first slot only.
            let (x, y) = at(0, 5.0);
            assert!(is_magenta(pixel(&img, x, y)), "{case}: span in slot 0");
            for k in [1, 2] {
                let (x, y) = at(k, 5.0);
                assert!(!is_magenta(pixel(&img, x, y)), "{case}: span in slot {k}");
            }
            // The boxes of the first slot draw over the span.
            let red = bbox(&img, slot(&chart, direction, 0, scale), is_red)
                .unwrap_or_else(|| panic!("{case}: the first box is under the span"));
            // Between q1 (20) and the median (30), off the median line.
            let body = value_px(&chart, direction, 25.0, scale).round() as u32;
            let inside = match direction {
                CategoryBarDirection::Vertical => ((red.0 + red.2) / 2, body),
                CategoryBarDirection::Horizontal => (body, (red.1 + red.3) / 2),
            };
            assert!(
                is_red(pixel(&img, inside.0, inside.1)),
                "{case}: body over the span"
            );
            // The reference line crosses the last slot at 90.
            let (x, y) = at(2, 90.0);
            let dark = (-2i32..=2).any(|d| {
                let (x, y) = match direction {
                    CategoryBarDirection::Vertical => (x, (y as i32 + d) as u32),
                    CategoryBarDirection::Horizontal => ((x as i32 + d) as u32, y),
                };
                let p = pixel(&img, x, y);
                p[3] > 200 && p[0].max(p[1]).max(p[2]) < 60
            });
            assert!(dark, "{case}: no reference line at 90");
        }
    }
}

/// A 2x export is the 1x layout doubled: every box body at 2x is twice its
/// 1x rect in both directions, so the gap between the boxes of a group
/// (logical pixels in `bar_groups.bar_gap`) doubles, as the box thickness
/// does.
#[test]
fn a_2x_export_is_the_1x_layout_doubled() {
    let mut r = renderer();
    for direction in DIRECTIONS {
        let chart = chart(direction);
        let series = [boxes("one", RED), boxes("two", BLUE)];
        let one = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let two = r.export_panel_rgba(&chart, &series, 2.0).unwrap();
        for k in 0..3 {
            for (name, pred) in [("red", is_red as fn(&[u8]) -> bool), ("blue", is_blue)] {
                let case = format!("{direction:?} category {k} {name}");
                let a = bbox(&one, slot(&chart, direction, k, 1.0), pred)
                    .unwrap_or_else(|| panic!("{case}: no box at 1x"));
                let b = bbox(&two, slot(&chart, direction, k, 2.0), pred)
                    .unwrap_or_else(|| panic!("{case}: no box at 2x"));
                let doubled = (2 * a.0, 2 * a.1, 2 * a.2 + 1, 2 * a.3 + 1);
                let near = [
                    (b.0, doubled.0),
                    (b.1, doubled.1),
                    (b.2, doubled.2),
                    (b.3, doubled.3),
                ]
                .iter()
                .all(|&(x, y)| x.abs_diff(y) <= 2);
                assert!(near, "{case}: 2x {b:?} for 1x {a:?} doubled {doubled:?}");
            }
        }
    }
}

/// A box group has no box cap (#107): four series over 150 categories are
/// 600 boxes, past the old 512 (and the categories past the old 64). Every
/// sampled box paints at 1x and 2x, the last box picks as its own series
/// and row, and its picked highlight outlines it.
#[test]
fn six_hundred_boxes_draw_pick_and_highlight() {
    const N: usize = 150;
    const M: usize = 4;
    const WIDE: u32 = 3000;
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("pos", &col((0..N).map(|k| k as f64).collect()))
        .unwrap();
    let stats: Vec<Stats> = vec![[10.0, 20.0, 50.0, 80.0, 90.0, NAN, NAN, NAN]; N];
    let mut series = Vec::new();
    for s in 0..M {
        let id = format!("s{s}");
        upload_record(&mut r, &format!("{id}.record"), &stats);
        let mut item = boxes(&id, if s % 2 == 0 { RED } else { BLUE });
        style(&mut item).show_mean = false;
        series.push(item);
    }
    let mut chart = chart(CategoryBarDirection::Vertical);
    {
        let config = chart.config_mut();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: WIDE,
            height: HEIGHT,
        });
        config.bottom_x.categories = Some(
            (0..N)
                .map(|k| Category::new(format!("c{k}"), format!("{k}")))
                .collect(),
        );
        config.bar_groups.as_mut().unwrap().bar_gap = 0.0;
    }
    let region = |chart: &Chart, k: usize, scale: f32| -> Region {
        let da = chart.config().data_area().expect("data area");
        let w = da.width as f32 / N as f32;
        let x0 = da.x as f32 + w * k as f32;
        let s = |v: f32| (v * scale).round() as u32;
        (
            s(x0),
            s(da.y as f32),
            s(x0 + w),
            s((da.y + da.height) as f32),
        )
    };
    for scale in [1.0f32, 2.0] {
        let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
        for k in [0, 63, 64, 100, N - 1] {
            let at = region(&chart, k, scale);
            assert!(bbox(&img, at, is_red).is_some(), "{scale}x red box {k}");
            assert!(bbox(&img, at, is_blue).is_some(), "{scale}x blue box {k}");
        }
    }
    // The picked highlight outlines the last box and no other.
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: format!("s{}", M - 1),
            bin_index: N - 1,
        }],
        highlight_color: MAGENTA,
        outline_width_px: 1.0,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 0.0,
    });
    let outlined = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert!(bbox(&outlined, region(&chart, N - 1, 1.0), is_magenta).is_some());
    assert_eq!(bbox(&outlined, region(&chart, 0, 1.0), is_magenta), None);
    chart.config_mut().picked_data = None;
    // A pick inside the last box's body names its series and row.
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let body = bbox(&img, region(&chart, N - 1, 1.0), is_blue).unwrap();
    let da = chart.config().data_area().expect("data area");
    // The rightmost blue box of the slot is the last series'.
    let x = body.2 - 1;
    let y = (da.y as f32 + da.height as f32 * 0.7).round() as u32;
    assert!(is_blue(pixel(&img, x, y)), "{body:?}");
    let request = renderer::GpuPickRequest {
        canvas_position_px: [x as f32 + 0.5, y as f32 + 0.5],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: WIDE,
            height: HEIGHT,
        },
        display_scale: 1.0,
        max_distance_px: 0.0,
    };
    let hit = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    assert!(
        matches!(
            &hit,
            Some(renderer::PickedData::HistogramBin { series_id, bin_index, .. })
                if *series_id == format!("s{}", M - 1) && *bin_index == N - 1
        ),
        "{hit:?}"
    );
}

/// Box overrides are not sized by a category table (#107 R5): a style and
/// a colour override on every one of 100 boxes are 300 override rows, past
/// the old 64-category table, and every box past the old 64th still takes
/// its override at 1x and 2x.
#[test]
fn overrides_on_a_hundred_boxes_all_apply() {
    const N: usize = 100;
    const WIDE: u32 = 2000;
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("pos", &col((0..N).map(|k| k as f64).collect()))
        .unwrap();
    let stats: Vec<Stats> = vec![[10.0, 20.0, 50.0, 80.0, 90.0, NAN, NAN, NAN]; N];
    upload_record(&mut r, "s.record", &stats);
    let mut series = boxes("s", RED);
    style(&mut series).show_mean = false;
    let restyled = BoxPlotStyle {
        median_color: YELLOW,
        show_mean: false,
        ..flat_style()
    };
    *overrides(&mut series) = (0..N)
        .map(|k| BoxPlotOverride {
            color: Some(BLUE),
            style: Some(restyled.clone()),
            ..BoxPlotOverride::new(BoxPlotTarget::new(format!("c{k}"), "s"))
        })
        .collect();
    let mut chart = chart(CategoryBarDirection::Vertical);
    {
        let config = chart.config_mut();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: WIDE,
            height: HEIGHT,
        });
        config.bottom_x.categories = Some(
            (0..N)
                .map(|k| Category::new(format!("c{k}"), format!("{k}")))
                .collect(),
        );
    }
    let series = [series];
    for scale in [1.0f32, 2.0] {
        let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
        let da = chart.config().data_area().expect("data area");
        let w = da.width as f32 / N as f32;
        let s = |v: f32| (v * scale).round() as u32;
        for k in [0, 63, 64, 99] {
            let x0 = da.x as f32 + w * k as f32;
            let at = (
                s(x0),
                s(da.y as f32),
                s(x0 + w),
                s((da.y + da.height) as f32),
            );
            assert!(bbox(&img, at, is_blue).is_some(), "{scale}x box {k}");
            assert_eq!(bbox(&img, at, is_red), None, "{scale}x box {k}");
            // The override's style restyles the median too.
            assert!(bbox(&img, at, is_yellow).is_some(), "{scale}x median {k}");
            assert_eq!(bbox(&img, at, is_green), None, "{scale}x median {k}");
        }
    }
}
