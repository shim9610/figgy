//! End-to-end verification of the value labels of category bar, box plot
//! and radar series through the public renderer API and the headless
//! `export_panel_rgba`, at 1x and 2x: where each label sits against the
//! bar end, the top of the box or the radar vertex (the former charts'
//! placement), that the drawn pixels are the SSOT string's own raster, that
//! an empty string draws nothing, that a drag offset moves the label by
//! itself, that the selected label's box is its frame, the labels of a
//! subplot, and the typed pick of a label.
//!
//! The bars and boxes are pure red and blue, the labels magenta (the value
//! axis' tick label colour) or white inside a dark bar, so a colour
//! predicate attributes every pixel. Expected positions come from the same
//! data area and axis ranges the renderer draws with, and the label sizes
//! from the one text stack (`measure_plain_text`).

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::{Category, TickVisibility};
use renderer::data::Column;
use renderer::data_config::{
    BarGroups, BoxPlotLabels, BoxPlotSeriesLabels, BoxPlotStyle, CategoryBarDirection,
    CategoryBarLabels, CategoryBarMode, CategoryBarSeriesLabels, CategoryBarStyle,
};
use renderer::layout::{ChartArea, Rect};
use renderer::polar::PolarOptions;
use renderer::raster::Canvas;
use renderer::select::{SELECTION_PADDING, SELECTION_STROKE_WIDTH};
use renderer::text_render::{FontPolicy, draw_plain_text, measure_plain_text};
use renderer::{
    BOX_PLOT_RECORD, Chart, Color, ColumnSource, DataRenderType, DataSelectionsConfig,
    GpuPickRequest, Material, PickedData, PickedDataRef, RadarLabels, RadarMarker,
    RadarSeriesLabels, RadarSeriesStyle, RaggedSource, RasterImage, RecordSource, Renderer,
    RendererDevice, SeriesConfig, Subplot,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const RED: Color = Color::from_rgb8(255, 0, 0);
const BLUE: Color = Color::from_rgb8(0, 0, 255);
const MAGENTA: Color = Color::from_rgb8(255, 0, 255);
/// The bar values over the categories a, b, c.
const ONE: [f64; 3] = [10.0, 20.0, 5.0];
const TWO: [f64; 3] = [20.0, 10.0, 15.0];
const SCALES: [f32; 2] = [1.0, 2.0];

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

/// One box: whisker low, q1, median, q3, whisker high.
type Stats = [f64; 5];
const BOXES: [Stats; 3] = [
    [10.0, 20.0, 30.0, 40.0, 50.0],
    [20.0, 30.0, 45.0, 60.0, 70.0],
    [5.0, 15.0, 25.0, 35.0, 45.0],
];
/// The outliers of the boxes: the first box's top is its outlier at 85.
const OUTLIERS: [&[f64]; 3] = [&[85.0], &[], &[]];
/// The radar values over four categories, on a 0..1 radius.
const RADAR: [f64; 4] = [0.6, 0.5, 0.7, 0.4];

/// A renderer holding the bar columns, the box record and outliers and the
/// radar columns, with GPU picking on.
fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("pos", &col(vec![0.0, 1.0, 2.0])).unwrap();
    r.add_columns(&[
        ("one", &col(ONE.to_vec()) as &dyn ColumnSource),
        ("two", &col(TWO.to_vec()) as &dyn ColumnSource),
    ])
    .unwrap();
    let field = |k: usize| col(BOXES.iter().map(|s| s[k]).collect());
    let columns = [field(2), field(1), field(3), field(0), field(4)];
    let names = ["median", "q1", "q3", "whisker_low", "whisker_high"];
    let fields: Vec<(&str, &dyn ColumnSource)> = names
        .iter()
        .zip(&columns)
        .map(|(name, column)| (*name, column as &dyn ColumnSource))
        .collect();
    r.upsert_record(
        "box",
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
    )
    .unwrap();
    let values = col(OUTLIERS.iter().flat_map(|g| g.iter().copied()).collect());
    let mut offsets = vec![0.0];
    for g in OUTLIERS {
        offsets.push(offsets.last().unwrap() + g.len() as f64);
    }
    r.upsert_ragged(
        "box.outliers",
        &RaggedSource {
            values: &values,
            offsets: &col(offsets),
        },
    )
    .unwrap();
    r.add_column("cat", &col(vec![0.0, 1.0, 2.0, 3.0])).unwrap();
    r.add_column("radar", &col(RADAR.to_vec())).unwrap();
    r.enable_gpu_picking().unwrap();
    r
}

fn categories(n: usize) -> Vec<Category> {
    (0..n)
        .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
        .collect()
}

/// A bare chart: no grid, legend or title, axis lines off, the value axes'
/// tick labels magenta (the value labels' colour) and every axis' tick
/// labels hidden.
fn bare_config() -> renderer::Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.chart_title.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    for axis in [
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.top_x,
        &mut config.right_y,
    ] {
        axis.line_visible = false;
        axis.label_style.visible = false;
        axis.label_style.color = MAGENTA;
        axis.tick = TickVisibility::None;
        axis.title_option.visible = false;
    }
    config
}

fn bar_chart(mode: CategoryBarMode, direction: CategoryBarDirection, value: (f64, f64)) -> Chart {
    let mut config = bare_config();
    let cats = Some(categories(3));
    match direction {
        CategoryBarDirection::Vertical => config.bottom_x.categories = cats,
        CategoryBarDirection::Horizontal => config.left_y.categories = cats,
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

fn bars(
    id: &str,
    color: Color,
    direction: CategoryBarDirection,
    placement: CategoryBarLabels,
    text: &[&str],
) -> SeriesConfig {
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
            style: CategoryBarStyle {
                material: Material::Flat,
                corner_radius: 0.0,
                outline: false,
                ..CategoryBarStyle::default()
            },
            labels: CategoryBarSeriesLabels {
                placement,
                text: text.iter().map(|t| t.to_string()).collect(),
                ..CategoryBarSeriesLabels::default()
            },
            label_format: Default::default(),
            overrides: None,
        },
    }
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

fn is_red(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 60 && p[2] < 60
}
fn is_blue(p: [u8; 4]) -> bool {
    p[3] > 200 && p[2] > 200 && p[0] < 60 && p[1] < 60
}
/// Label ink over a transparent background: magenta at any coverage.
fn is_label(p: [u8; 4]) -> bool {
    p[3] > 60 && p[1] < 40 && p[0] > p[3] / 2 && p[2] > p[3] / 2
}
/// White label ink over a red or blue bar.
fn is_white(p: [u8; 4]) -> bool {
    p[0] > 150 && p[1] > 150 && p[2] > 150
}

/// The inclusive bounding box `(x0, y0, x1, y1)` of the pixels `pred`
/// accepts within `region` (export pixels, end exclusive).
fn bbox(
    img: &RasterImage,
    region: (u32, u32, u32, u32),
    pred: fn([u8; 4]) -> bool,
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

fn centre(b: (u32, u32, u32, u32)) -> (f32, f32) {
    (
        (b.0 + b.2) as f32 * 0.5 + 0.5,
        (b.1 + b.3) as f32 * 0.5 + 0.5,
    )
}

/// The export region of category `k`'s slot along the category axis, the
/// whole panel across it.
fn slot(
    chart: &Chart,
    direction: CategoryBarDirection,
    k: usize,
    scale: f32,
) -> (u32, u32, u32, u32) {
    let da = chart.config().data_area().expect("data area");
    let s = |v: f32| (v * scale).round().max(0.0) as u32;
    match direction {
        CategoryBarDirection::Vertical => {
            let w = da.width as f32 / 3.0;
            let x0 = da.x as f32 + w * k as f32;
            (s(x0), 0, s(x0 + w), s(HEIGHT as f32))
        }
        CategoryBarDirection::Horizontal => {
            let h = da.height as f32 / 3.0;
            let y1 = (da.y + da.height) as f32 - h * k as f32;
            (0, s(y1 - h), s(WIDTH as f32), s(y1))
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

/// The value labels' face: the value axis' tick label face at `scale`.
fn face(chart: &Chart, direction: CategoryBarDirection) -> (String, f32) {
    let config = chart.config();
    let axis = match direction {
        CategoryBarDirection::Vertical => &config.left_y,
        CategoryBarDirection::Horizontal => &config.bottom_x,
    };
    (
        axis.label_style.label_font.clone(),
        axis.label_style.font_size,
    )
}

/// `text` rasterized the way the label atlas bakes it: in `color` with the
/// baseline at the ascent, at `size`, on a transparent canvas of its
/// measured size, in straight alpha as an export is.
fn raster(text: &str, family: &str, size: f32, color: Color) -> RasterImage {
    let policy = FontPolicy::Standard;
    let m = measure_plain_text(text, family, size, false, false, policy);
    let (w, h) = (
        m.width.ceil().max(1.0) as u32,
        (m.ascent + m.descent).ceil().max(1.0) as u32,
    );
    let mut canvas = Canvas::new(w, h).unwrap();
    draw_plain_text(
        &mut canvas,
        text,
        (0.0, m.ascent),
        color,
        family,
        size,
        false,
        false,
        policy,
    );
    // The canvas is premultiplied; the export is straight alpha.
    let mut rgba = canvas.into_rgba();
    for p in rgba.chunks_exact_mut(4) {
        if p[3] > 0 {
            for c in 0..3 {
                p[c] = ((u32::from(p[c]) * 255 + u32::from(p[3]) / 2) / u32::from(p[3])).min(255)
                    as u8;
            }
        }
    }
    RasterImage {
        width: w,
        height: h,
        rgba,
    }
}

/// Every bar series' labels sit where the former categorical chart put
/// them, at 1x and 2x: an outside label's baseline `6` px past a rising
/// bar end, centred on the bar; past a horizontal bar's end, `6` px from it
/// and centred across it; a stacked segment's `Auto` label inside it,
/// centred, in white over the dark bar.
#[test]
fn bar_labels_sit_at_the_bar_ends_as_the_former_chart_placed_them() {
    let mut r = renderer();
    let text = |values: &[f64; 3]| values.map(|v| format!("{v}"));
    let one_text = text(&ONE);
    let two_text = text(&TWO);
    let one: Vec<&str> = one_text.iter().map(String::as_str).collect();
    let two: Vec<&str> = two_text.iter().map(String::as_str).collect();
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        let chart = bar_chart(CategoryBarMode::Grouped, direction, (0.0, 25.0));
        let series = [
            bars("one", RED, direction, CategoryBarLabels::Outside, &one),
            bars("two", BLUE, direction, CategoryBarLabels::Outside, &two),
        ];
        for scale in SCALES {
            let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
            for k in 0..3 {
                let case = format!("{direction:?} {scale}x category {k}");
                let region = slot(&chart, direction, k, scale);
                for (pred, value) in [(is_red as fn([u8; 4]) -> bool, ONE[k]), (is_blue, TWO[k])] {
                    let bar = bbox(&img, region, pred).unwrap_or_else(|| panic!("{case}: no bar"));
                    let end = value_px(&chart, direction, value, scale);
                    let slack = 2.0 * scale + 1.0;
                    match direction {
                        CategoryBarDirection::Vertical => {
                            // The ink right above the bar, across its width.
                            let above = (bar.0.saturating_sub(4), 0, bar.2 + 5, bar.1);
                            let ink = bbox(&img, above, is_label)
                                .unwrap_or_else(|| panic!("{case}: no label above {bar:?}"));
                            let (cx, _) = centre(ink);
                            let (bx, _) = centre(bar);
                            assert!(
                                (cx - bx).abs() <= slack,
                                "{case}: label at x {cx}, bar {bx}"
                            );
                            // Digits sit on the baseline, `6` px above the end.
                            let baseline = end - 6.0 * scale;
                            assert!(
                                (ink.3 as f32 + 1.0 - baseline).abs() <= slack,
                                "{case}: ink bottom {} for a baseline at {baseline}",
                                ink.3 + 1
                            );
                        }
                        CategoryBarDirection::Horizontal => {
                            let right = (bar.2 + 1, bar.1, img.width, bar.3 + 1);
                            let ink = bbox(&img, right, is_label)
                                .unwrap_or_else(|| panic!("{case}: no label right of {bar:?}"));
                            let (_, cy) = centre(ink);
                            let (_, by) = centre(bar);
                            assert!(
                                (cy - by).abs() <= slack + scale,
                                "{case}: label at y {cy}, bar {by}"
                            );
                            assert!(
                                (ink.0 as f32 - (end + 6.0 * scale)).abs() <= slack,
                                "{case}: ink left {} for an end at {end}",
                                ink.0
                            );
                        }
                    }
                }
            }
        }
    }
    // A stack's segments that fit their labels show them inside, in white.
    let direction = CategoryBarDirection::Vertical;
    let chart = bar_chart(CategoryBarMode::Stacked, direction, (0.0, 50.0));
    let series = [
        bars("one", RED, direction, CategoryBarLabels::Auto, &one),
        bars("two", BLUE, direction, CategoryBarLabels::Auto, &two),
    ];
    for scale in SCALES {
        let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
        for k in [0usize, 1] {
            let case = format!("stacked {scale}x category {k}");
            let region = slot(&chart, direction, k, scale);
            for (pred, from, to) in [
                (is_red as fn([u8; 4]) -> bool, 0.0, ONE[k]),
                (is_blue, ONE[k], ONE[k] + TWO[k]),
            ] {
                let bar = bbox(&img, region, pred).unwrap_or_else(|| panic!("{case}: no bar"));
                let (y0, y1) = (
                    value_px(&chart, direction, to, scale),
                    value_px(&chart, direction, from, scale),
                );
                let inside = (bar.0, y0.ceil() as u32, bar.2 + 1, y1.floor() as u32);
                let ink = bbox(&img, inside, is_white)
                    .unwrap_or_else(|| panic!("{case}: no white label in {inside:?}"));
                let (cx, cy) = centre(ink);
                let (bx, _) = centre(bar);
                let slack = 2.0 * scale + 1.0;
                assert!((cx - bx).abs() <= slack, "{case}: label x {cx}, bar {bx}");
                assert!(
                    (cy - (y0 + y1) * 0.5).abs() <= slack + 2.0 * scale,
                    "{case}: label y {cy}, segment {y0}..{y1}"
                );
            }
        }
        // The third category's lower segment (5 of 50) is too low for its
        // label: `Auto` leaves it out.
        let region = slot(&chart, direction, 2, scale);
        let red = bbox(&img, region, is_red).unwrap();
        assert_eq!(
            bbox(&img, (red.0, red.1, red.2 + 1, red.3 + 1), is_white),
            None
        );
    }
}

/// The drawn label is the SSOT string's own raster, texel for texel, at 1x
/// and 2x: a custom string draws as it is, and an empty string draws
/// nothing.
#[test]
fn a_label_draws_its_own_string_and_an_empty_one_draws_nothing() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = bar_chart(CategoryBarMode::Grouped, direction, (0.0, 25.0));
    let series = [bars(
        "two",
        BLUE,
        direction,
        CategoryBarLabels::Outside,
        &["Ab 7%", "", "x"],
    )];
    let (family, size) = face(&chart, direction);
    for scale in SCALES {
        let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
        let region = slot(&chart, direction, 0, scale);
        let ink = bbox(&img, region, is_label).expect("a label in category 0");
        let want = raster("Ab 7%", &family, size * scale, MAGENTA);
        let want_ink = bbox(&want, (0, 0, want.width, want.height), is_label).unwrap();
        assert_eq!(
            (ink.2 - ink.0, ink.3 - ink.1),
            (want_ink.2 - want_ink.0, want_ink.3 - want_ink.1),
            "{scale}x: the drawn ink has the string's size"
        );
        // Every pixel of the string's raster, aligned on its ink: the
        // coverage everywhere, the colour where the coverage is enough to
        // carry it through straight alpha.
        let mut worst = 0u8;
        for y in 0..want.height {
            for x in 0..want.width {
                let (ex, ey) = (ink.0 + x - want_ink.0, ink.1 + y - want_ink.1);
                if x < want_ink.0 || y < want_ink.1 || ex >= img.width || ey >= img.height {
                    continue;
                }
                let (a, b) = (pixel(&img, ex, ey), pixel(&want, x, y));
                let colour = if b[3] > 32 { 0..4 } else { 3..4 };
                for c in colour {
                    worst = worst.max(a[c].abs_diff(b[c]));
                }
            }
        }
        assert!(
            worst <= 3,
            "{scale}x: the label differs from the string by {worst}"
        );
        // The empty string of category 1 draws nothing.
        assert_eq!(
            bbox(&img, slot(&chart, direction, 1, scale), is_label),
            None
        );
        assert!(bbox(&img, slot(&chart, direction, 2, scale), is_label).is_some());
    }
}

/// A dragged label moves by its offset and nothing else, at 1x and 2x: the
/// drag writes `labels.offsets` (SSOT) and the next frame draws the label
/// there.
#[test]
fn a_dragged_label_moves_by_its_offset_at_1x_and_2x() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = bar_chart(CategoryBarMode::Grouped, direction, (0.0, 25.0));
    let still = [bars(
        "two",
        BLUE,
        direction,
        CategoryBarLabels::Outside,
        &["20", "10", "15"],
    )];
    let mut dragged = still.clone();
    assert_eq!(
        dragged[0].drag_value_label(1, 3, 12.0, -8.0),
        renderer::layout::NudgeResult::Moved
    );
    assert_eq!(dragged[0].value_label_offset(1), [12.0, -8.0]);
    for scale in SCALES {
        let before = r.export_panel_rgba(&chart, &still, scale).unwrap();
        let after = r.export_panel_rgba(&chart, &dragged, scale).unwrap();
        let everywhere = (0, 0, before.width, before.height);
        let region = |k| slot(&chart, direction, k, scale);
        let moved = |img: &RasterImage| {
            // Category 1's label, wherever the slot band finds it.
            let (x0, _, x1, _) = region(1);
            bbox(
                img,
                (x0, 0, x1 + (16.0 * scale) as u32, img.height),
                is_label,
            )
            .unwrap()
        };
        let (a, b) = (moved(&before), moved(&after));
        assert_eq!(
            (b.0 as i64 - a.0 as i64, b.1 as i64 - a.1 as i64),
            ((12.0 * scale) as i64, (-8.0 * scale) as i64),
            "{scale}x: the label moves by its offset"
        );
        assert_eq!((b.2 - b.0, b.3 - b.1), (a.2 - a.0, a.3 - a.1));
        // The other labels and every bar stay where they were.
        for k in [0, 2] {
            assert_eq!(
                bbox(&before, region(k), is_label),
                bbox(&after, region(k), is_label)
            );
        }
        assert_eq!(
            bbox(&before, everywhere, is_blue),
            bbox(&after, everywhere, is_blue)
        );
    }
}

fn box_chart(direction: CategoryBarDirection) -> Chart {
    let mut config = bare_config();
    let cats = Some(categories(3));
    match direction {
        CategoryBarDirection::Vertical => config.bottom_x.categories = cats,
        CategoryBarDirection::Horizontal => config.left_y.categories = cats,
    }
    config.bar_groups = Some(BarGroups {
        mode: CategoryBarMode::Grouped,
        direction,
        group_width: 0.6,
        bar_gap: 4.0,
    });
    let mut chart = Chart::new(config);
    match direction {
        CategoryBarDirection::Vertical => chart.set_y_range(0.0, 100.0),
        CategoryBarDirection::Horizontal => chart.set_x_range(0.0, 100.0),
    }
    chart
}

fn boxes(direction: CategoryBarDirection, text: &[&str]) -> SeriesConfig {
    let style = BoxPlotStyle {
        material: Material::Flat,
        outline: false,
        ..BoxPlotStyle::default()
    };
    let _ = direction;
    SeriesConfig {
        series_id: "box".into(),
        source_id: None,
        label: None,
        x_column: "pos".into(),
        y_column: "box".into(),
        render_type: DataRenderType::BoxPlot {
            color: RED,
            style,
            labels: BoxPlotSeriesLabels {
                placement: BoxPlotLabels::Median,
                text: text.iter().map(|t| t.to_string()).collect(),
                ..BoxPlotSeriesLabels::default()
            },
            outliers: Some("box.outliers".into()),
            overrides: None,
        },
    }
}

/// A box's label sits `10` px past the top of the box, its high whisker or
/// its highest outlier, centred across the box, at 1x and 2x, as the former
/// box plot placed it.
#[test]
fn box_labels_sit_past_the_top_of_the_box() {
    let mut r = renderer();
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        let chart = box_chart(direction);
        let series = [boxes(direction, &["30", "45", "25"])];
        for scale in SCALES {
            let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
            for (k, top) in [(0usize, 85.0), (1, 70.0), (2, 45.0)] {
                let case = format!("{direction:?} {scale}x box {k}");
                let region = slot(&chart, direction, k, scale);
                let body = bbox(&img, region, is_red).unwrap_or_else(|| panic!("{case}: no box"));
                let ink =
                    bbox(&img, region, is_label).unwrap_or_else(|| panic!("{case}: no label"));
                let end = value_px(&chart, direction, top, scale);
                let slack = 2.0 * scale + 1.0;
                let (cx, cy) = centre(ink);
                let (bx, by) = centre(body);
                match direction {
                    CategoryBarDirection::Vertical => {
                        assert!((cx - bx).abs() <= slack, "{case}: label x {cx}, box {bx}");
                        let baseline = end - 10.0 * scale;
                        assert!(
                            (ink.3 as f32 + 1.0 - baseline).abs() <= slack,
                            "{case}: ink bottom {} for a baseline at {baseline}",
                            ink.3 + 1
                        );
                    }
                    CategoryBarDirection::Horizontal => {
                        assert!(
                            (cy - by).abs() <= slack + scale,
                            "{case}: label y {cy}, box {by}"
                        );
                        assert!(
                            (ink.0 as f32 - (end + 10.0 * scale)).abs() <= slack,
                            "{case}: ink left {} for a top at {end}",
                            ink.0
                        );
                    }
                }
            }
        }
    }
}

fn radar_chart() -> Chart {
    let mut config = bare_config();
    config.bottom_x.categories = Some(categories(4));
    config.polar = Some(PolarOptions {
        zero_angle_degrees: 90.0,
        clockwise: true,
        ..PolarOptions::default()
    });
    let mut chart = Chart::new(config);
    chart.set_y_range(0.0, 1.0);
    chart
}

fn radar(placement: RadarLabels) -> SeriesConfig {
    SeriesConfig {
        series_id: "radar".into(),
        source_id: None,
        label: None,
        x_column: "cat".into(),
        y_column: "radar".into(),
        render_type: DataRenderType::Radar {
            color: RED,
            style: RadarSeriesStyle {
                line_width_px: Some(2.0),
                fill: Some(Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }),
                marker: Some(RadarMarker::None),
                ..Default::default()
            },
            labels: RadarSeriesLabels {
                placement,
                text: RADAR.iter().map(|v| format!("{v}")).collect(),
                ..RadarSeriesLabels::default()
            },
        },
    }
}

/// Where radar category `i` at value `v` lands in an export at `scale`.
fn vertex(chart: &Chart, scale: f32, i: f64, v: f64) -> (f32, f32) {
    let config = chart.config().scaled(scale);
    let map = config.polar_angle_map().expect("polar");
    let geometry = config.polar_geometry().expect("geometry");
    let inner = f64::from(geometry.inner_radius);
    let t = (v - config.left_y.min) / (config.left_y.max - config.left_y.min);
    let radius = inner + (f64::from(geometry.radius) - inner) * t;
    let a = map.screen_angle(i);
    (
        (f64::from(geometry.center[0]) + a.cos() * radius) as f32,
        (f64::from(geometry.center[1]) - a.sin() * radius) as f32,
    )
}

/// A radar vertex's label sits outside the polygon along its spoke, `9` px
/// from the vertex, as the former radar chart placed it: above the top
/// vertex, right of the right one, below the bottom one and left of the
/// left one, at 1x and 2x. `Emphasized` labels show only for a series with
/// a selection.
#[test]
fn radar_labels_sit_outside_the_vertex_along_the_spoke() {
    let mut r = renderer();
    let chart = radar_chart();
    let series = [radar(RadarLabels::Values)];
    for scale in SCALES {
        let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
        let slack = 2.0 * scale + 1.0;
        let gap = 9.0 * scale;
        let font = chart.config().left_y.label_style.font_size;
        let quarter = |i: usize| {
            let (vx, vy) = vertex(&chart, scale, i as f64, RADAR[i]);
            let (w, h) = (img.width as f32, img.height as f32);
            // The quarter of the panel beyond the vertex along its spoke.
            let region = match i {
                0 => (0.0, 0.0, w, vy),
                1 => (vx, 0.0, w, h),
                2 => (0.0, vy, w, h),
                _ => (0.0, 0.0, vx, h),
            };
            let region = (
                region.0 as u32,
                region.1 as u32,
                region.2.ceil() as u32,
                region.3.ceil() as u32,
            );
            (vx, vy, bbox(&img, region, is_label))
        };
        let (vx, vy, top) = quarter(0);
        let top = top.expect("a label above the top vertex");
        assert!(
            (centre(top).0 - vx).abs() <= slack,
            "{scale}x top: {top:?} at {vx}"
        );
        // The former radar's baseline: `gap` along the spoke, then
        // `0.35` of the font size down, so the digits centre on the spoke.
        let drop = 0.35 * font * scale;
        let baseline = vy - gap + drop;
        assert!(
            (top.3 as f32 + 1.0 - baseline).abs() <= slack,
            "{scale}x top: {top:?} for a baseline at {baseline}"
        );
        let (vx, vy, right) = quarter(1);
        let right = right.expect("a label right of the right vertex");
        assert!(
            (right.0 as f32 - (vx + gap)).abs() <= slack,
            "{scale}x right: {right:?} at {vx}"
        );
        assert!(
            (centre(right).1 - vy).abs() <= slack + scale,
            "{scale}x right: {right:?} at {vy}"
        );
        let (vx, vy, bottom) = quarter(2);
        let bottom = bottom.expect("a label below the bottom vertex");
        assert!(
            (centre(bottom).0 - vx).abs() <= slack,
            "{scale}x bottom: {bottom:?} at {vx}"
        );
        let baseline = vy + gap + drop;
        assert!(
            (bottom.3 as f32 + 1.0 - baseline).abs() <= slack,
            "{scale}x bottom: {bottom:?} for a baseline at {baseline}"
        );
        let (vx, _, left) = quarter(3);
        let left = left.expect("a label left of the left vertex");
        assert!(
            (left.2 as f32 + 1.0 - (vx - gap)).abs() <= slack,
            "{scale}x left: {left:?} at {vx}"
        );
    }
    // `Emphasized`: nothing until the series has a selection.
    let mut emphasized = [radar(RadarLabels::Emphasized)];
    let all = (0, 0, WIDTH, HEIGHT);
    let img = r.export_panel_rgba(&chart, &emphasized, 1.0).unwrap();
    assert_eq!(bbox(&img, all, is_label), None);
    let mut selected = Chart::new(chart.config().clone());
    selected.config_mut().picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "radar".into(),
            point_index: 1,
        }],
        ..DataSelectionsConfig::default()
    });
    let img = r.export_panel_rgba(&selected, &emphasized, 1.0).unwrap();
    assert!(bbox(&img, all, is_label).is_some());
    emphasized[0].series_id = "other".into();
    let img = r.export_panel_rgba(&selected, &emphasized, 1.0).unwrap();
    assert_eq!(bbox(&img, all, is_label), None);
}

/// The selected label's box is its frame: the pixels the selection adds lie
/// on the label's content box grown by `SELECTION_PADDING`, stroked
/// `SELECTION_STROKE_WIDTH` wide in `SELECTION_COLOR`, and every edge is
/// drawn, at 1x and 2x; the label itself is unchanged.
#[test]
fn the_selected_label_is_outlined_by_its_frame() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut chart = bar_chart(CategoryBarMode::Grouped, direction, (0.0, 25.0));
    let series = [bars(
        "two",
        BLUE,
        direction,
        CategoryBarLabels::Outside,
        &["20", "10", "15"],
    )];
    let (family, size) = face(&chart, direction);
    for scale in SCALES {
        chart.config_mut().picked_data = None;
        let plain = r.export_panel_rgba(&chart, &series, scale).unwrap();
        chart.config_mut().picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::ValueLabel {
                source_id: None,
                series_id: "two".into(),
                category_index: 0,
            }],
            ..DataSelectionsConfig::default()
        });
        let selected = r.export_panel_rgba(&chart, &series, scale).unwrap();
        // The content box: the string's measured size around its ink.
        let ink = bbox(&plain, slot(&chart, direction, 0, scale), is_label).unwrap();
        let want = raster("20", &family, size * scale, MAGENTA);
        let want_ink = bbox(&want, (0, 0, want.width, want.height), is_label).unwrap();
        let content = (
            ink.0 as f32 - want_ink.0 as f32,
            ink.1 as f32 - want_ink.1 as f32,
        );
        let m = measure_plain_text(
            "20",
            &family,
            size * scale,
            false,
            false,
            FontPolicy::Standard,
        );
        let pad = SELECTION_PADDING * scale;
        let frame = (
            content.0 - pad,
            content.1 - pad,
            content.0 + m.width + pad,
            content.1 + (m.ascent + m.descent).ceil() + pad,
        );
        let half = SELECTION_STROKE_WIDTH * scale * 0.5 + 1.0;
        let mut changed = 0;
        let mut edges = [false; 4];
        for y in 0..plain.height {
            for x in 0..plain.width {
                let (a, b) = (pixel(&plain, x, y), pixel(&selected, x, y));
                if a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 2) {
                    continue;
                }
                changed += 1;
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let on = [
                    (px - frame.0).abs() <= half && py >= frame.1 - half && py <= frame.3 + half,
                    (px - frame.2).abs() <= half && py >= frame.1 - half && py <= frame.3 + half,
                    (py - frame.1).abs() <= half && px >= frame.0 - half && px <= frame.2 + half,
                    (py - frame.3).abs() <= half && px >= frame.0 - half && px <= frame.2 + half,
                ];
                assert!(
                    on.iter().any(|e| *e),
                    "{scale}x: ({x}, {y}) changed off the frame {frame:?}"
                );
                for (edge, hit) in edges.iter_mut().zip(on) {
                    *edge |= hit;
                }
                // The box is drawn in the selection colour.
                assert!(b[2] > b[0] && b[2] > b[1], "{scale}x: ({x}, {y}) is {b:?}");
            }
        }
        assert!(
            changed > 0 && edges.iter().all(|e| *e),
            "{scale}x: {changed} pixels, edges {edges:?}"
        );
    }
}

/// A bar series on an overlay or a subplot carries its labels: they draw
/// in its own panel, over its own bars, at 1x and 2x, and pick there on
/// its layer.
#[test]
fn overlay_and_subplot_bar_series_draw_their_labels_in_their_panels() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut parent = bar_chart(CategoryBarMode::Grouped, direction, (0.0, 25.0));
    let mut overlay = renderer::OverlayLayer::default();
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 50.0;
    overlay.y_axis.major_spacing = 10.0;
    overlay.series = vec!["two".into()];
    // The overlay's labels take its value axis' face and colour.
    overlay.y_axis.label_style.color = MAGENTA;
    let mut sub = bar_chart(CategoryBarMode::Grouped, direction, (0.0, 25.0))
        .config()
        .clone();
    sub.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT,
        width: WIDTH,
        height: HEIGHT,
    });
    let config = parent.config_mut();
    config.overlays.push(overlay);
    config.subplots.push(Subplot {
        config: sub,
        series: vec!["sub".into()],
        link_x: None,
        link_y: None,
    });
    let mut sub_bars = bars(
        "sub",
        RED,
        direction,
        CategoryBarLabels::Outside,
        &["s0", "s1", "s2"],
    );
    sub_bars.y_column = "one".into();
    let series = [
        bars(
            "one",
            RED,
            direction,
            CategoryBarLabels::None,
            &["10", "20", "5"],
        ),
        bars(
            "two",
            BLUE,
            direction,
            CategoryBarLabels::Outside,
            &["20", "10", "15"],
        ),
        sub_bars,
    ];
    for scale in SCALES {
        let img = r.export_document_rgba(&parent, &series, scale).unwrap();
        let (w, h) = (img.width, img.height);
        let top = (0, 0, w, h / 2);
        let bottom = (0, h / 2, w, h);
        // The overlay's labels above its blue bars, the subplot's above its
        // red ones; the parent's own series shows none.
        assert!(
            bbox(&img, top, is_blue).is_some(),
            "{scale}x: the overlay's bars"
        );
        let first_slot = slot(&parent, direction, 0, scale);
        let first_bar = bbox(&img, (first_slot.0, 0, first_slot.2, h / 2), is_blue).unwrap();
        let first = bbox(&img, (first_slot.0, 0, first_slot.2, first_bar.1), is_label)
            .expect("the overlay's label above its first bar");
        let red = bbox(&img, bottom, is_red).expect("the subplot's bars");
        let sub_ink = bbox(&img, bottom, is_label).expect("the subplot's labels");
        assert!(
            sub_ink.1 >= h / 2 && sub_ink.3 <= red.3,
            "{scale}x: {sub_ink:?}"
        );

        let id = r
            .register_chart(parent.config().clone(), series.to_vec())
            .unwrap();
        let pick = |r: &mut Renderer, (x, y): (f32, f32)| {
            let request = GpuPickRequest {
                canvas_position_px: [x, y],
                display_panel_px: Rect {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                },
                display_scale: scale,
                max_distance_px: 0.0,
            };
            pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap()
        };
        // The overlay's label picks on the overlay's layer.
        let mut config = parent.config().clone();
        config.set_active_layer(renderer::LayerRef::Overlay(0));
        r.set_chart_config(id, config).unwrap();
        match pick(&mut r, centre(first)) {
            Some(PickedData::ValueLabel { series_id, .. }) => assert_eq!(series_id, "two"),
            other => panic!("{scale}x: the overlay's label picked {other:?}"),
        }
        // The subplot's label picks in the subplot's panel.
        r.set_active_layer(id, renderer::LayerRef::Subplot(0))
            .unwrap();
        let sub_first = bbox(&img, (first_slot.0, h / 2, first_slot.2, h), is_label)
            .expect("the subplot's first label");
        match pick(&mut r, centre(sub_first)) {
            Some(PickedData::ValueLabel {
                series_id,
                category_index,
                ..
            }) => assert_eq!((series_id.as_str(), category_index), ("sub", 0)),
            other => panic!("{scale}x: the subplot's label picked {other:?}"),
        }
        r.remove_chart(id).unwrap();
    }
}

/// The typed pick names a label by its series and category where it is
/// drawn, and a label wins over the bar it is drawn over; away from the
/// label the bar picks. At 1x and 2x.
#[test]
fn a_label_picks_with_its_series_and_category_over_the_data_under_it() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = bar_chart(CategoryBarMode::Stacked, direction, (0.0, 40.0));
    let one = ["10", "20", "5"];
    let series = [
        bars("one", RED, direction, CategoryBarLabels::Inside, &one),
        bars(
            "two",
            BLUE,
            direction,
            CategoryBarLabels::Outside,
            &["20", "10", "15"],
        ),
    ];
    for scale in SCALES {
        let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
        let id = r
            .register_chart(chart.config().clone(), series.to_vec())
            .unwrap();
        let pick = |r: &mut Renderer, (x, y): (f32, f32)| {
            let request = GpuPickRequest {
                canvas_position_px: [x, y],
                display_panel_px: Rect {
                    x: 0,
                    y: 0,
                    width: img.width,
                    height: img.height,
                },
                display_scale: scale,
                max_distance_px: 0.0,
            };
            pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap()
        };
        for k in 0..3 {
            let region = slot(&chart, direction, k, scale);
            // The inside label of series one, over its red bar.
            let red = bbox(&img, region, is_red).unwrap();
            let inside = bbox(&img, (red.0, red.1, red.2 + 1, red.3 + 1), is_white);
            if let Some(ink) = inside {
                match pick(&mut r, centre(ink)) {
                    Some(PickedData::ValueLabel {
                        series_id,
                        category_index,
                        ..
                    }) => assert_eq!((series_id.as_str(), category_index), ("one", k)),
                    other => panic!("{scale}x category {k}: the inside label picked {other:?}"),
                }
            }
            // The outside label of series two.
            let blue = bbox(&img, region, is_blue).unwrap();
            let ink = bbox(&img, (region.0, 0, region.2, blue.1), is_label).unwrap();
            match pick(&mut r, centre(ink)) {
                Some(PickedData::ValueLabel {
                    series_id,
                    category_index,
                    ..
                }) => assert_eq!((series_id.as_str(), category_index), ("two", k)),
                other => panic!("{scale}x category {k}: the outside label picked {other:?}"),
            }
            // Low in the red bar, away from its label, the bar picks.
            let low = (centre(red).0, red.3 as f32 - 1.0);
            match pick(&mut r, low) {
                Some(PickedData::HistogramBin {
                    series_id,
                    bin_index,
                    ..
                }) => {
                    assert_eq!((series_id.as_str(), bin_index), ("one", k))
                }
                other => panic!("{scale}x category {k}: the bar picked {other:?}"),
            }
        }
        r.remove_chart(id).unwrap();
    }
}
