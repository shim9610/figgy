//! End-to-end pixel verification of category bar series
//! (`DataRenderType::CategoryBars`) through the public renderer API and the
//! headless `export_panel_rgba`: grouped, stacked and percent bars in both
//! directions at 1x and 2x, per-bar overrides and materials, the selected
//! bar's outline, and the value-axis fit over the stacked extent.
//!
//! The first series is pure red, the second pure blue, both flat with no
//! outline and square corners unless a test says otherwise, so a colour
//! predicate attributes every pixel to one series. Expected bar extents come
//! from the same data area and axis ranges the renderer draws with.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::Category;
use renderer::data::Column;
use renderer::data_config::{
    BarGroups, CategoryBarDirection, CategoryBarMode, CategoryBarOverride, CategoryBarStyle,
    CategoryBarTarget,
};
use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, Color, ColumnSource, DataRenderType, DataSelectionsConfig, LayerRef, Material,
    PickedDataRef, RasterImage, Renderer, RendererDevice, SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
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
const GREEN: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
const MAGENTA: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};
/// Values of the two series over the categories a, b, c.
const ONE: [f64; 3] = [10.0, 20.0, 5.0];
const TWO: [f64; 3] = [20.0, 10.0, 15.0];

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

/// A renderer holding the position column and the two value columns, the
/// values uploaded as one batch so the pool measures their stack.
fn renderer(one: &[f64], two: &[f64]) -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("pos", &col((0..one.len()).map(|i| i as f64).collect()))
        .unwrap();
    r.add_columns(&[
        ("one", &col(one.to_vec()) as &dyn ColumnSource),
        ("two", &col(two.to_vec()) as &dyn ColumnSource),
    ])
    .unwrap();
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

fn bars(id: &str, color: Color, direction: CategoryBarDirection) -> SeriesConfig {
    let (x, y) = match direction {
        CategoryBarDirection::Vertical => ("pos", id),
        CategoryBarDirection::Horizontal => (id, "pos"),
    };
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::CategoryBars {
            color,
            style: flat_style(),
            labels: Default::default(),
            label_format: Default::default(),
            overrides: None,
        },
    }
}

/// Grid, legend and title off; three categories on the direction's
/// category axis; the value axis over `value`.
fn chart(mode: CategoryBarMode, direction: CategoryBarDirection, value: (f64, f64)) -> Chart {
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
        mode,
        direction,
        group_width: 0.8,
        bar_gap: 4.0,
    });
    let mut chart = Chart::new(config);
    match direction {
        CategoryBarDirection::Vertical => chart.set_y_range(value.0, value.1),
        CategoryBarDirection::Horizontal => chart.set_x_range(value.0, value.1),
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
fn is_magenta(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[2] > 200 && p[1] < 60
}

/// Bounding box `(x0, y0, x1, y1)` (inclusive) of the pixels `pred` accepts
/// within `region` (export pixels, end exclusive).
fn bbox(
    img: &RasterImage,
    region: (u32, u32, u32, u32),
    pred: fn(&[u8]) -> bool,
) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
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
fn slot(
    chart: &Chart,
    direction: CategoryBarDirection,
    k: usize,
    scale: f32,
) -> (u32, u32, u32, u32) {
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
        // Category 0 at the bottom: the axis is not inverted.
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

/// The value-axis span `(start, end)` in export pixels of a bar's box.
fn value_span(b: (u32, u32, u32, u32), direction: CategoryBarDirection) -> (f32, f32) {
    match direction {
        // y grows downward: the start (bottom) is the larger coordinate.
        CategoryBarDirection::Vertical => (b.3 as f32 + 1.0, b.1 as f32),
        CategoryBarDirection::Horizontal => (b.0 as f32, b.2 as f32 + 1.0),
    }
}

/// The category-axis span `(low, high)` of a bar's box.
fn across_span(b: (u32, u32, u32, u32), direction: CategoryBarDirection) -> (u32, u32) {
    match direction {
        CategoryBarDirection::Vertical => (b.0, b.2),
        CategoryBarDirection::Horizontal => (b.1, b.3),
    }
}

/// Each bar spans the values it stands for, in its own category's slot, in
/// every mode and direction at 1x and 2x: grouped bars from zero side by
/// side, stacked bars on the earlier series, percent bars as shares of the
/// category total.
#[test]
fn bars_span_their_values_in_every_mode_direction_and_scale() {
    let mut r = renderer(&ONE, &TWO);
    for mode in [
        CategoryBarMode::Grouped,
        CategoryBarMode::Stacked,
        CategoryBarMode::PercentStacked,
    ] {
        for direction in [
            CategoryBarDirection::Vertical,
            CategoryBarDirection::Horizontal,
        ] {
            let range = match mode {
                CategoryBarMode::Grouped => (0.0, 25.0),
                CategoryBarMode::Stacked => (0.0, 40.0),
                CategoryBarMode::PercentStacked => (0.0, 100.0),
            };
            let chart = chart(mode, direction, range);
            let series = [bars("one", RED, direction), bars("two", BLUE, direction)];
            for scale in [1.0f32, 2.0] {
                let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
                assert_eq!(
                    (img.width, img.height),
                    (
                        (WIDTH as f32 * scale) as u32,
                        (HEIGHT as f32 * scale) as u32
                    )
                );
                for k in 0..3 {
                    let case = format!("{mode:?} {direction:?} {scale}x category {k}");
                    let region = slot(&chart, direction, k, scale);
                    let red =
                        bbox(&img, region, is_red).unwrap_or_else(|| panic!("{case}: no red"));
                    let blue =
                        bbox(&img, region, is_blue).unwrap_or_else(|| panic!("{case}: no blue"));
                    let (one, two) = (ONE[k], TWO[k]);
                    let (red_span, blue_span) = match mode {
                        CategoryBarMode::Grouped => ((0.0, one), (0.0, two)),
                        CategoryBarMode::Stacked => ((0.0, one), (one, one + two)),
                        CategoryBarMode::PercentStacked => {
                            let share = one / (one + two) * 100.0;
                            ((0.0, share), (share, 100.0))
                        }
                    };
                    let slack = 2.0 * scale;
                    for (name, b, (from, to)) in [("one", red, red_span), ("two", blue, blue_span)]
                    {
                        let (start, end) = value_span(b, direction);
                        let (want_start, want_end) = (
                            value_px(&chart, direction, from, scale),
                            value_px(&chart, direction, to, scale),
                        );
                        assert!(
                            (start - want_start).abs() <= slack && (end - want_end).abs() <= slack,
                            "{case} {name}: spans {start}..{end}, expected {want_start}..{want_end}"
                        );
                    }
                    let (red_across, blue_across) =
                        (across_span(red, direction), across_span(blue, direction));
                    match mode {
                        // Side by side in series order, the gap between them.
                        // (Series order runs left to right, or top-down.)
                        CategoryBarMode::Grouped => {
                            let (first, second) = (red_across, blue_across);
                            assert!(
                                second.0 > first.1 + (2.0 * scale) as u32,
                                "{case}: grouped bars overlap or touch: {first:?} {second:?}"
                            );
                            let w0 = first.1 - first.0;
                            let w1 = second.1 - second.0;
                            assert!(w0.abs_diff(w1) <= 1, "{case}: widths {w0} and {w1}");
                        }
                        // One column per category.
                        _ => assert!(
                            red_across.0.abs_diff(blue_across.0) <= 1
                                && red_across.1.abs_diff(blue_across.1) <= 1,
                            "{case}: stacked bars not aligned: {red_across:?} {blue_across:?}"
                        ),
                    }
                }
            }
        }
    }
}

/// Stacked negative values stack downward from zero on their own, and a
/// missing (NaN) value draws no bar and leaves its stack position to the
/// next series.
#[test]
fn signed_stacks_and_missing_values_follow_the_stack_rules() {
    let one = [10.0, f64::NAN, -10.0];
    let two = [-5.0, 10.0, -5.0];
    let mut r = renderer(&one, &two);
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(CategoryBarMode::Stacked, direction, (-20.0, 20.0));
    let series = [bars("one", RED, direction), bars("two", BLUE, direction)];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let span = |k: usize, pred: fn(&[u8]) -> bool| {
        bbox(&img, slot(&chart, direction, k, 1.0), pred).map(|b| value_span(b, direction))
    };
    let px = |v| value_px(&chart, direction, v, 1.0);
    // The box's span along the value axis, whichever way the bar grows.
    let near = |(a, b): (f32, f32), (c, d): (f32, f32)| {
        (a.min(b) - c.min(d)).abs() <= 2.0 && (a.max(b) - c.max(d)).abs() <= 2.0
    };
    // a: +10 up from zero, -5 down from zero.
    assert!(near(span(0, is_red).unwrap(), (px(0.0), px(10.0))));
    assert!(near(span(0, is_blue).unwrap(), (px(0.0), px(-5.0))));
    // b: one is missing, two starts at zero.
    assert_eq!(span(1, is_red), None);
    assert!(near(span(1, is_blue).unwrap(), (px(0.0), px(10.0))));
    // c: both negative, two below one.
    assert!(near(span(2, is_red).unwrap(), (px(0.0), px(-10.0))));
    assert!(near(span(2, is_blue).unwrap(), (px(-10.0), px(-15.0))));
}

fn with_overrides(mut s: SeriesConfig, overrides: Vec<CategoryBarOverride>) -> SeriesConfig {
    let DataRenderType::CategoryBars { overrides: o, .. } = &mut s.render_type else {
        unreachable!()
    };
    *o = Some(overrides);
    s
}

/// A per-bar override recolours and restyles its own bar only, and a
/// series material changes the shading inside the bars but not their
/// extents.
#[test]
fn overrides_and_materials_restyle_only_their_bars() {
    let mut r = renderer(&ONE, &TWO);
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(CategoryBarMode::Grouped, direction, (0.0, 25.0));
    let plain = [bars("one", RED, direction), bars("two", BLUE, direction)];
    let mut overridden = plain.clone();
    overridden[0] = with_overrides(
        plain[0].clone(),
        vec![CategoryBarOverride {
            target: CategoryBarTarget::new("b", "one"),
            color: Some(GREEN),
            style: None,
            labels: None,
            label_format: None,
        }],
    );
    for scale in [1.0f32, 2.0] {
        let before = r.export_panel_rgba(&chart, &plain, scale).unwrap();
        let after = r.export_panel_rgba(&chart, &overridden, scale).unwrap();
        let green = bbox(&after, slot(&chart, direction, 1, scale), is_green).expect("green bar");
        let red_before = bbox(&before, slot(&chart, direction, 1, scale), is_red).unwrap();
        assert_eq!(
            green, red_before,
            "{scale}x: the override keeps the bar's geometry"
        );
        assert_eq!(
            bbox(&after, slot(&chart, direction, 1, scale), is_red),
            None
        );
        for k in [0, 2] {
            let region = slot(&chart, direction, k, scale);
            assert_eq!(
                bbox(&after, region, is_green),
                None,
                "{scale}x category {k}"
            );
            assert_eq!(
                bbox(&after, region, is_red),
                bbox(&before, region, is_red),
                "{scale}x category {k}"
            );
        }
        let changed = before
            .rgba
            .chunks(4)
            .zip(after.rgba.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        let bar = (green.2 - green.0 + 1) * (green.3 - green.1 + 1);
        assert!(
            changed as u32 <= bar + 4 * (green.2 - green.0 + green.3 - green.1 + 2),
            "{scale}x: {changed} pixels changed for one bar of {bar}"
        );
    }

    // A grained material shades the bar's inside; the extents stay.
    let mut matte = plain.clone();
    for s in &mut matte {
        let DataRenderType::CategoryBars { style, .. } = &mut s.render_type else {
            unreachable!()
        };
        style.material = Material::Matte;
        style.texture_strength = 1.0;
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

    // A per-bar style override sets that bar's material and corners.
    let mut styled = plain.clone();
    styled[1] = with_overrides(
        plain[1].clone(),
        vec![CategoryBarOverride {
            target: CategoryBarTarget::new("a", "two"),
            color: None,
            style: Some(CategoryBarStyle {
                material: Material::Paper,
                corner_radius: 8.0,
                ..flat_style()
            }),
            labels: None,
            label_format: None,
        }],
    );
    let restyled = r.export_panel_rgba(&chart, &styled, 1.0).unwrap();
    let a = slot(&chart, direction, 0, 1.0);
    let b = bbox(&flat, a, is_blue).unwrap();
    // The rounded top corners leave the bar's corner pixel clear.
    assert!(is_blue(pixel(&flat, b.0, b.1)));
    assert!(!is_blue(pixel(&restyled, b.0, b.1)));
    for k in [1, 2] {
        let region = slot(&chart, direction, k, 1.0);
        assert_eq!(
            bbox(&flat, region, is_blue),
            bbox(&restyled, region, is_blue)
        );
    }
}

/// The selected bar's outline traces the bar as drawn, overrides included:
/// a bar whose override rounds its corners is outlined with those corners,
/// the outline stays inside the bar, and no other bar is touched (#77).
#[test]
fn the_selection_outline_traces_an_overridden_bar() {
    let mut r = renderer(&ONE, &TWO);
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        let mut chart = chart(CategoryBarMode::Stacked, direction, (0.0, 40.0));
        let mut series = [bars("one", RED, direction), bars("two", BLUE, direction)];
        series[1] = with_overrides(
            series[1].clone(),
            vec![CategoryBarOverride {
                target: CategoryBarTarget::new("b", "two"),
                color: Some(GREEN),
                style: Some(CategoryBarStyle {
                    corner_radius: 10.0,
                    ..flat_style()
                }),
                labels: None,
                label_format: None,
            }],
        );
        for scale in [1.0f32, 2.0] {
            let case = format!("{direction:?} {scale}x");
            chart.config_mut().picked_data = None;
            let painted = r.export_panel_rgba(&chart, &series, scale).unwrap();
            let region = slot(&chart, direction, 1, scale);
            let bar = bbox(&painted, region, is_green).expect("the overridden bar paints");
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
            let ring = bbox(&outlined, full, is_magenta).expect("the selected bar is outlined");
            let edges = [
                (ring.0, bar.0),
                (ring.1, bar.1),
                (ring.2, bar.2),
                (ring.3, bar.3),
            ];
            assert!(
                edges.iter().all(|(a, b)| a.abs_diff(*b) <= 1),
                "{case}: the outline {ring:?} does not trace the painted bar {bar:?}"
            );
            // The exposed end is rounded: its outer corner pixel is not ring.
            let corner = match direction {
                CategoryBarDirection::Vertical => (ring.0, ring.1),
                CategoryBarDirection::Horizontal => (ring.2, ring.1),
            };
            assert!(!is_magenta(pixel(&outlined, corner.0, corner.1)), "{case}");
            // Nothing outside the selected bar changed.
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

/// The value-axis fit covers what the bars draw: the grouped values and
/// zero, the stacked extent the pool measured at upload (positive and
/// negative stacks apart), or 0..100 for percent bars.
#[test]
fn the_value_axis_fits_the_drawn_extent() {
    let one = [10.0, 25.0, -5.0];
    let two = [20.0, -10.0, -5.0];
    let r = renderer(&one, &two);
    let fit = |mode, direction| {
        let mut chart = chart(mode, direction, (0.0, 1.0));
        let series = [bars("one", RED, direction), bars("two", BLUE, direction)];
        chart
            .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
            .unwrap();
        let config = chart.config();
        match direction {
            CategoryBarDirection::Vertical => (config.left_y.min, config.left_y.max),
            CategoryBarDirection::Horizontal => (config.bottom_x.min, config.bottom_x.max),
        }
    };
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        let (min, max) = fit(CategoryBarMode::Grouped, direction);
        assert!(
            min <= -10.0 && max >= 25.0,
            "grouped {direction:?}: {min}..{max}"
        );
        // Stacks: a = 30, b = 25 and -10, c = -10.
        let (min, max) = fit(CategoryBarMode::Stacked, direction);
        assert!(
            min <= -10.0 && max >= 30.0,
            "stacked {direction:?}: {min}..{max}"
        );
        assert!(
            max < 40.0,
            "stacked {direction:?}: the fit sums by category: {max}"
        );
        let (min, max) = fit(CategoryBarMode::PercentStacked, direction);
        assert!(
            min <= 0.0 && max >= 100.0,
            "percent {direction:?}: {min}..{max}"
        );
    }
    // The category axis keeps its category range.
    let mut chart = chart(
        CategoryBarMode::Grouped,
        CategoryBarDirection::Vertical,
        (0.0, 1.0),
    );
    let series = [
        bars("one", RED, CategoryBarDirection::Vertical),
        bars("two", BLUE, CategoryBarDirection::Vertical),
    ];
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
        .unwrap();
    assert_eq!(chart.config().bottom_x.data_range(), (-0.5, 2.5));
}

/// A stacked group whose value columns were not uploaded together has no
/// measured stack: the fit says so instead of guessing.
#[test]
fn a_stack_uploaded_column_by_column_is_not_fitted() {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .unwrap();
    r.add_column("pos", &col(vec![0.0, 1.0, 2.0])).unwrap();
    r.add_column("one", &col(ONE.to_vec())).unwrap();
    r.add_column("two", &col(TWO.to_vec())).unwrap();
    let direction = CategoryBarDirection::Vertical;
    let mut chart = chart(CategoryBarMode::Stacked, direction, (0.0, 1.0));
    let series = [bars("one", RED, direction), bars("two", BLUE, direction)];
    let error = chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
        .unwrap_err();
    assert!(error.to_string().contains("one batch"), "{error}");
    // Grouped bars fit from each column's own statistics.
    let mut chart = self::chart(CategoryBarMode::Grouped, direction, (0.0, 1.0));
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
        .unwrap();
}

/// A categorical document read by `Config::from_categorical` draws through
/// this path: its uploads borrow the document's values, and every bar of
/// the document paints.
#[cfg(feature = "serde")]
#[test]
fn a_categorical_document_draws_as_category_bars() {
    let doc: renderer::CategoricalChart = serde_json::from_str(include_str!(
        "fixtures/released/model-0.12.1-categorical-edited.json"
    ))
    .unwrap();
    let (mut config, series, uploads) = renderer::config::Config::from_categorical(&doc);
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
    let (positions, values) = uploads.split_first().unwrap();
    r.add_column(&positions.column, &positions.values).unwrap();
    let columns: Vec<(&str, &dyn ColumnSource)> = values
        .iter()
        .map(|u| (u.column.as_str(), &u.values as &dyn ColumnSource))
        .collect();
    r.add_columns(&columns).unwrap();
    let mut chart = Chart::new(config);
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
        .unwrap();
    for scale in [1.0f32, 2.0] {
        let with = r.export_panel_rgba(&chart, &series, scale).unwrap();
        let without = r.export_panel_rgba(&chart, &[], scale).unwrap();
        // The document's override colour is drawn.
        let red = |p: &[u8]| p[3] > 200 && p[0] > 180 && p[1] < 70 && p[2] < 70;
        let full = (0, 0, with.width, with.height);
        assert!(
            bbox(&with, full, red).is_some(),
            "{scale}x: the q2 override paints"
        );
        let changed = with
            .rgba
            .chunks(4)
            .zip(without.rgba.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > (2000.0 * scale * scale) as usize,
            "{scale}x: {changed}"
        );
    }
}

/// `cross_at` moves the axis line to the value of the perpendicular axis:
/// the category axis line of a chart with negative bars runs along the
/// value axis' zero, between the bars above and below it.
#[test]
fn the_category_axis_line_crosses_the_value_axis_at_zero() {
    let one = [10.0, -10.0, 5.0];
    let two = [5.0, -5.0, -5.0];
    let mut r = renderer(&one, &two);
    let direction = CategoryBarDirection::Vertical;
    let mut chart = chart(CategoryBarMode::Stacked, direction, (-20.0, 20.0));
    let series = [bars("one", RED, direction), bars("two", BLUE, direction)];
    let dark = |p: &[u8]| p[3] > 200 && p[0] < 80 && p[1] < 80 && p[2] < 80;
    // The darkest full row inside the data area's horizontal span.
    let line_row = |img: &RasterImage, chart: &Chart| {
        let da = chart.config().data_area().unwrap();
        (da.y..da.y + da.height + 1)
            .max_by_key(|&y| {
                (da.x + 2..da.x + da.width - 2)
                    .filter(|&x| dark(pixel(img, x, y)))
                    .count()
            })
            .unwrap()
    };
    chart.config_mut().bottom_x.cross_at = None;
    let edge = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let da = chart.config().data_area().unwrap();
    assert!(line_row(&edge, &chart).abs_diff(da.y + da.height) <= 1);
    chart.config_mut().bottom_x.cross_at = Some(0.0);
    let crossed = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let zero = value_px(&chart, direction, 0.0, 1.0);
    let row = line_row(&crossed, &chart);
    assert!(
        (row as f32 - zero).abs() <= 1.5,
        "the axis line is at row {row}, zero at {zero}"
    );
    // Only the line moved: the bars are where they were, the line drawn
    // over their zero end at most.
    for k in 0..3 {
        let region = slot(&chart, direction, k, 1.0);
        let (a, b) = (
            bbox(&edge, region, is_red).unwrap(),
            bbox(&crossed, region, is_red).unwrap(),
        );
        assert!(
            [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)]
                .iter()
                .all(|(p, q)| p.abs_diff(*q) <= 1),
            "category {k}: {a:?} vs {b:?}"
        );
    }
}
