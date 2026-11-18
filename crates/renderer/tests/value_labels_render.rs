//! End-to-end verification of the value labels of category bar, box plot
//! and radar series through the public renderer API and the headless
//! `export_rgba`, at 1x and 2x: where each label sits against the
//! bar end, the box's median line or the radar vertex, that the drawn pixels are the SSOT string's own raster, that
//! an empty string draws nothing, that a drag offset moves the label by
//! itself, that the selected label's box is its frame, the labels of a
//! subplot, the typed pick of a label, and how the labels of several radar
//! series make room for each other next to their own vertices (#272).
//!
//! The bars and boxes are pure red and blue, the labels magenta (the value
//! axis' tick label colour) or white inside a dark bar, so a colour
//! predicate attributes every pixel. Expected positions come from the same
//! data area and axis ranges the renderer draws with, and the label sizes
//! from the one text stack (`measure_plain_text`).

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

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

fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(|| {
        let (device, queue) = test_gpu::device();
        (device, queue)
    });
    (device.clone(), queue.clone())
}

fn col(data: Vec<f64>) -> Column<f64> {
    Column { data }
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
/// A second radar series over the same categories (#272): far below the
/// first at the top spoke, a little above it at the right one, equal at the
/// bottom one and within the visual gap of it at the left one.
const RADAR_B: [f64; 4] = [0.3, 0.53, 0.7, 0.405];

/// A renderer holding the bar columns, the box record and outliers and the
/// radar columns, with GPU picking on.
fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
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
    r.add_column("radar.b", &col(RADAR_B.to_vec())).unwrap();
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
        z_column: None,
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
    )
    .unwrap();
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
            let img = r
                .export_rgba(
                    renderer::ExportSource::chart(&chart, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
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
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
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
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
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
        let before = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &still),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let after = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &dragged),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
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
        z_column: None,
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
            violin: None,
            points: None,
        },
    }
}

/// A box's label sits beside its median line (#272), `10` px past the side
/// of the box, never at the box's outlier or whisker end whose value it
/// would seem to name: right of a vertical box with its text centred on
/// the median, above a horizontal box centred on the median, at 1x and 2x.
/// In a group of two the labels go to the group's outer sides.
#[test]
fn box_labels_sit_beside_the_median() {
    let mut r = renderer();
    let medians = BOXES.map(|b| b[2]);
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        // Narrow boxes: every label has room on its own side, in its slot.
        let mut chart = box_chart(direction);
        if let Some(groups) = chart.config_mut().bar_groups.as_mut() {
            groups.group_width = 0.2;
        }
        let series = [boxes(direction, &["30", "45", "25"])];
        for scale in SCALES {
            let img = r
                .export_rgba(
                    renderer::ExportSource::chart(&chart, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
            for (k, median) in medians.iter().enumerate() {
                let case = format!("{direction:?} {scale}x box {k}");
                let region = slot(&chart, direction, k, scale);
                let body = bbox(&img, region, is_red).unwrap_or_else(|| panic!("{case}: no box"));
                let at = value_px(&chart, direction, *median, scale);
                // Past the box's labelled side, around the median: right
                // of a vertical box, above a horizontal one. The top box's
                // label flips under it (`an_edge_box_label_flips_...`), into
                // the slot below, away from the median there.
                let reach = (40.0 * scale) as u32;
                let near = |v: f32| ((v as u32).saturating_sub(reach), v as u32 + reach);
                let side = match direction {
                    CategoryBarDirection::Vertical => {
                        let (y0, y1) = near(at);
                        (body.2 + 1, y0, body.2 + 1 + 2 * reach, y1)
                    }
                    CategoryBarDirection::Horizontal => {
                        let (x0, x1) = near(at);
                        (x0, body.1.saturating_sub(reach), x1, body.1)
                    }
                };
                let ink = bbox(&img, side, is_label).unwrap_or_else(|| panic!("{case}: no label"));
                let gap = 10.0 * scale;
                let slack = 2.0 * scale + 1.0;
                let (cx, cy) = centre(ink);
                match direction {
                    CategoryBarDirection::Vertical => {
                        assert!(
                            (ink.0 as f32 - (body.2 as f32 + 1.0 + gap)).abs() <= slack,
                            "{case}: ink left {} for a box right edge at {}",
                            ink.0,
                            body.2 + 1
                        );
                        assert!(
                            (cy - at).abs() <= slack,
                            "{case}: label y {cy}, median {at}"
                        );
                    }
                    CategoryBarDirection::Horizontal => {
                        assert!(
                            (cx - at).abs() <= slack,
                            "{case}: label x {cx}, median {at}"
                        );
                        assert!(
                            (ink.3 as f32 + 1.0 - (body.1 as f32 - gap)).abs() <= slack,
                            "{case}: ink bottom {} for a box top at {}",
                            ink.3 + 1,
                            body.1
                        );
                    }
                }
            }
        }
        // Two boxes per category: the first one's label goes before it
        // (left, or above), the second one's after it, away from the
        // group's centre. The middle category keeps both off the data
        // area's edges.
        let chart = box_chart(direction);
        let mut first = boxes(direction, &["", "30", ""]);
        first.series_id = "box-a".into();
        let mut second = boxes(direction, &["", "31", ""]);
        second.series_id = "box-b".into();
        if let DataRenderType::BoxPlot {
            color,
            style: _,
            labels: _,
            outliers: _,
            overrides: _,
            violin: _,
            points: _,
        } = &mut second.render_type
        {
            *color = BLUE;
        }
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &[first, second]),
                renderer::ExportOptions::panel(1.0),
            )
            .block_on()
            .unwrap();
        let region = slot(&chart, direction, 1, 1.0);
        let red = bbox(&img, region, is_red).expect("the first box");
        let blue = bbox(&img, region, is_blue).expect("the second box");
        let everywhere = (0, 0, img.width, img.height);
        let ink = bbox(&img, everywhere, is_label).expect("the labels");
        let case = format!("{direction:?} group");
        match direction {
            CategoryBarDirection::Vertical => {
                assert!(red.0 < blue.0, "{case}: series order");
                assert!(ink.0 < red.0, "{case}: a label left of the left box");
                assert!(ink.2 > blue.2, "{case}: a label right of the right box");
            }
            CategoryBarDirection::Horizontal => {
                let (top, bottom) = if red.1 < blue.1 {
                    (red, blue)
                } else {
                    (blue, red)
                };
                assert!(ink.1 < top.1, "{case}: a label above the upper box");
                assert!(ink.3 > bottom.3, "{case}: a label below the lower box");
            }
        }
    }
}

fn radar_chart() -> Chart {
    let mut config = bare_config();
    config.bottom_x.categories = Some(categories(4));
    config.set_polar(Some(PolarOptions {
        zero_angle_degrees: 90.0,
        clockwise: true,
        ..PolarOptions::default()
    }));
    let mut chart = Chart::new(config);
    chart.set_y_range(0.0, 1.0);
    chart
}

fn radar(placement: RadarLabels) -> SeriesConfig {
    radar_series("radar", "radar", &RADAR, placement)
}

/// A radar series `id` over the value column `column` holding `values`,
/// an outline 2 px wide with no marker and no fill, labelled with each
/// value as written.
fn radar_series(id: &str, column: &str, values: &[f64], placement: RadarLabels) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "cat".into(),
        y_column: column.into(),
        z_column: None,
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
                text: values.iter().map(|v| format!("{v}")).collect(),
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

/// How far a radar label's content box keeps from its vertex at `scale`
/// (#272): `max(4, 0.25 em)` past the vertex's mark, here half the 2 px
/// outline of [`radar_series`], which draws no marker.
fn radar_clear(chart: &Chart, scale: f32) -> f32 {
    let font = chart.config().left_y.label_style.font_size;
    (1.0 + 4.0f32.max(0.25 * font)) * scale
}

/// The distance from `p` to the box `b`, 0 inside it.
fn box_distance(b: (f32, f32, f32, f32), p: (f32, f32)) -> f32 {
    let dx = (b.0 - p.0).max(p.0 - b.2).max(0.0);
    let dy = (b.1 - p.1).max(p.1 - b.3).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// A radar label's content box keeps its gap from its vertex and is never
/// farther from it than one em plus the marker size (#272; `marker` in
/// logical pixels, the outline width without a marker).
fn assert_next_to_vertex(
    chart: &Chart,
    content: (f32, f32, f32, f32),
    vertex: (f32, f32),
    marker: f32,
    scale: f32,
    case: &str,
) {
    let font = chart.config().left_y.label_style.font_size;
    let slack = 2.0 * scale + 1.0;
    let d = box_distance(content, vertex);
    assert!(
        d <= (font + marker) * scale + slack,
        "{case}: {content:?} lies {d} px from its vertex {vertex:?}"
    );
    assert!(
        d >= radar_clear(chart, scale) - slack,
        "{case}: {content:?} lies {d} px from its vertex {vertex:?}"
    );
}

/// The radar label of `text` whose ink lies within `40` px (at `scale`) of
/// `at` in `img`: its content box.
fn radar_label_near(
    chart: &Chart,
    img: &RasterImage,
    at: (f32, f32),
    text: &str,
    scale: f32,
    case: &str,
) -> ((u32, u32, u32, u32), (f32, f32, f32, f32)) {
    let reach = 40.0 * scale;
    let region = (
        (at.0 - reach).max(0.0) as u32,
        (at.1 - reach).max(0.0) as u32,
        (at.0 + reach).ceil() as u32,
        (at.1 + reach).ceil() as u32,
    );
    let ink = bbox(img, region, is_label).unwrap_or_else(|| panic!("{case}: no label near {at:?}"));
    let axis = &chart.config().left_y.label_style;
    let content = content_box(text, &axis.label_font, axis.font_size * scale, ink);
    (ink, content)
}

/// A radar vertex's label sits next to its vertex, outside it along its
/// spoke (#272): its content box `max(4, 0.25 em)` past the vertex's mark,
/// above the top vertex, right of the right one, below the bottom one and
/// left of the left one, centred on the spoke, at 1x and 2x. `Emphasized`
/// labels show only for a series with a selection.
#[test]
fn radar_labels_sit_outside_the_vertex_along_the_spoke() {
    let mut r = renderer();
    let chart = radar_chart();
    let series = [radar(RadarLabels::Values)];
    for scale in SCALES {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let slack = 2.0 * scale + 1.0;
        let clear = radar_clear(&chart, scale);
        for (i, v) in RADAR.iter().enumerate() {
            let case = format!("{scale}x vertex {i}");
            let (vx, vy) = vertex(&chart, scale, i as f64, *v);
            let (_, b) = radar_label_near(&chart, &img, (vx, vy), &format!("{v}"), scale, &case);
            let (cx, cy) = ((b.0 + b.2) * 0.5, (b.1 + b.3) * 0.5);
            let (near, across) = match i {
                0 => (vy - b.3, cx - vx),
                1 => (b.0 - vx, cy - vy),
                2 => (b.1 - vy, cx - vx),
                _ => (vx - b.2, cy - vy),
            };
            assert!((near - clear).abs() <= slack, "{case}: {b:?} {near} px out");
            assert!(
                across.abs() <= slack,
                "{case}: {b:?} {across} px off the spoke"
            );
            assert_next_to_vertex(&chart, b, (vx, vy), 2.0, scale, &case);
        }
    }
    // `Emphasized`: nothing until the series has a selection.
    let mut emphasized = [radar(RadarLabels::Emphasized)];
    let all = (0, 0, WIDTH, HEIGHT);
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &emphasized),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
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
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&selected, &emphasized),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    assert!(bbox(&img, all, is_label).is_some());
    emphasized[0].series_id = "other".into();
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&selected, &emphasized),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    assert_eq!(bbox(&img, all, is_label), None);
}

/// The two radar series of `RADAR` and `RADAR_B`, labelled.
fn two_radars() -> Vec<SeriesConfig> {
    vec![
        radar(RadarLabels::Values),
        radar_series("radar.b", "radar.b", &RADAR_B, RadarLabels::Values),
    ]
}

/// `series` with only the label of category `i` of series `k` on the
/// chart: every other label is dragged far off it, which leaves each label
/// where it is placed (the drag applies after the placement).
fn only_label(series: &[SeriesConfig], k: usize, i: usize) -> Vec<SeriesConfig> {
    let mut out = series.to_vec();
    for (j, s) in out.iter_mut().enumerate() {
        if let DataRenderType::Radar { labels, .. } = &mut s.render_type {
            labels.offsets = (0..labels.text.len())
                .map(|c| {
                    if j == k && c == i {
                        [0.0; 2]
                    } else {
                        [4000.0; 2]
                    }
                })
                .collect();
        }
    }
    out
}

/// Where a radar label sits against its vertex (#272).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Spot {
    /// Outside the vertex along the spoke.
    Outside,
    /// Inside it, toward the centre.
    Inside,
    /// Beside it at its radius, counter-clockwise of the spoke on screen.
    Ccw,
    /// Beside it, clockwise of the spoke.
    Cw,
}

/// The labels of two radar series move only where they would touch, and
/// stay next to their own vertices (#272), at 1x and 2x: far-apart values
/// at the top spoke keep both labels outside their vertices; close ones at
/// the right spoke put the larger value's outside and the smaller value's
/// inside its own vertex; equal values at the bottom spoke, and values
/// within the visual gap of each other at the left one, split one to each
/// side of the spoke at the vertices' radius, the earlier of the scan (the
/// larger value, or the lower rank) counter-clockwise. Every label keeps
/// its gap and lies within one em plus the mark of its vertex, no two
/// labels touch, and each picks as its own series and category where it
/// is drawn.
#[test]
fn radar_labels_of_two_series_move_only_where_they_touch() {
    let mut r = renderer();
    let chart = radar_chart();
    let values = [RADAR, RADAR_B];
    let ids = ["radar", "radar.b"];
    // Per series and spoke: top, right, bottom, left.
    let spots = [
        [Spot::Outside, Spot::Inside, Spot::Ccw, Spot::Cw],
        [Spot::Outside, Spot::Outside, Spot::Cw, Spot::Ccw],
    ];
    for scale in SCALES {
        let slack = 2.0 * scale + 1.0;
        let clear = radar_clear(&chart, scale);
        let mut placed = Vec::new();
        for k in 0..2 {
            for i in 0..4 {
                let series = only_label(&two_radars(), k, i);
                let img = r
                    .export_rgba(
                        renderer::ExportSource::chart(&chart, &series),
                        renderer::ExportOptions::panel(scale),
                    )
                    .block_on()
                    .unwrap();
                let case = format!("{scale}x {} vertex {i}", ids[k]);
                let v = values[k][i];
                let (vx, vy) = vertex(&chart, scale, i as f64, v);
                let (ink, b) =
                    radar_label_near(&chart, &img, (vx, vy), &format!("{v}"), scale, &case);
                // The spoke's outward direction and its counter-clockwise
                // side on screen (y down), as unit axes.
                let (u, n) = match i {
                    0 => ((0.0, -1.0), (-1.0, 0.0)),
                    1 => ((1.0, 0.0), (0.0, -1.0)),
                    2 => ((0.0, 1.0), (1.0, 0.0)),
                    _ => ((-1.0, 0.0), (0.0, 1.0)),
                };
                let (d, c) = match spots[k][i] {
                    Spot::Outside => (u, n),
                    Spot::Inside => ((-u.0, -u.1), n),
                    Spot::Ccw => (n, u),
                    Spot::Cw => ((-n.0, -n.1), u),
                };
                // The near edge `clear` past the vertex along `d`, the box
                // centred on the vertex along the other axis `c`.
                let (cx, cy) = ((b.0 + b.2) * 0.5, (b.1 + b.3) * 0.5);
                let near = if d.0 > 0.5 {
                    b.0 - vx
                } else if d.0 < -0.5 {
                    vx - b.2
                } else if d.1 > 0.5 {
                    b.1 - vy
                } else {
                    vy - b.3
                };
                let across = (cx - vx) * c.0 + (cy - vy) * c.1;
                assert!(
                    (near - clear).abs() <= slack,
                    "{case}: {:?} expected, {b:?} {near} px from {vx},{vy}",
                    spots[k][i]
                );
                assert!(
                    across.abs() <= slack,
                    "{case}: {:?} expected, {b:?} {across} px off",
                    spots[k][i]
                );
                assert_next_to_vertex(&chart, b, (vx, vy), 2.0, scale, &case);
                assert_picks(
                    pick_at(&mut r, &chart, &series, &img, scale, centre(ink)),
                    ids[k],
                    i,
                    &case,
                );
                placed.push((case, ink));
            }
        }
        // No two labels touch: their inks keep at least a pixel apart.
        for (a, (case_a, ia)) in placed.iter().enumerate() {
            for (case_b, ib) in placed.iter().skip(a + 1) {
                let apart =
                    ia.2 + 1 < ib.0 || ib.2 + 1 < ia.0 || ia.3 + 1 < ib.1 || ib.3 + 1 < ia.1;
                assert!(apart, "{case_a} {ia:?} touches {case_b} {ib:?}");
            }
        }
    }
}

/// The selection box of a moved radar label is its frame where it is drawn
/// (#272): the inside label of the first series at the right spoke and the
/// split one of the second at the left spoke, at 1x and 2x.
#[test]
fn a_moved_radar_label_is_outlined_where_it_is_drawn() {
    let mut r = renderer();
    let mut chart = radar_chart();
    for (k, id, i, v) in [
        (0usize, "radar", 1usize, RADAR[1]),
        (1, "radar.b", 3, RADAR_B[3]),
    ] {
        let series = only_label(&two_radars(), k, i);
        for scale in SCALES {
            let case = format!("{scale}x {id} vertex {i}");
            chart.config_mut().picked_data = None;
            let plain = r
                .export_rgba(
                    renderer::ExportSource::chart(&chart, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
            chart.config_mut().picked_data = Some(DataSelectionsConfig {
                refs: vec![PickedDataRef::ValueLabel {
                    source_id: None,
                    series_id: id.into(),
                    category_index: i,
                    part: None,
                }],
                ..DataSelectionsConfig::default()
            });
            let selected = r
                .export_rgba(
                    renderer::ExportSource::chart(&chart, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
            let at = vertex(&chart, scale, i as f64, v);
            let (_, content) = radar_label_near(&chart, &plain, at, &format!("{v}"), scale, &case);
            assert_selection_frame(&plain, &selected, content, scale, &case);
        }
    }
}

/// A radar label group has no member cap (rule 10): twenty labelled radar
/// series, past the old 16, place their labels together at every spoke.
/// Crowded as they are, the labels of the innermost, a middle and the
/// outermost series each stay next to their own vertices at 1x and 2x.
#[test]
fn twenty_labelled_radar_series_keep_their_labels_next_to_their_vertices() {
    const M: usize = 20;
    let mut r = renderer();
    let value = |k: usize| (8 + 4 * k) as f64 / 100.0;
    let mut series = Vec::new();
    for k in 0..M {
        let id = format!("radar.{k}");
        r.add_column(&id, &col(vec![value(k); 4])).unwrap();
        series.push(radar_series(&id, &id, &[value(k); 4], RadarLabels::Values));
    }
    let chart = radar_chart();
    for scale in SCALES {
        for k in [0, 10, M - 1] {
            for i in 0..4 {
                let only = only_label(&series, k, i);
                let img = r
                    .export_rgba(
                        renderer::ExportSource::chart(&chart, &only),
                        renderer::ExportOptions::panel(scale),
                    )
                    .block_on()
                    .unwrap();
                let case = format!("{scale}x series {k} vertex {i}");
                let at = vertex(&chart, scale, i as f64, value(k));
                let (ink, content) =
                    radar_label_near(&chart, &img, at, &format!("{}", value(k)), scale, &case);
                assert_next_to_vertex(&chart, content, at, 2.0, scale, &case);
                assert_picks(
                    pick_at(&mut r, &chart, &only, &img, scale, centre(ink)),
                    &format!("radar.{k}"),
                    i,
                    &case,
                );
            }
        }
    }
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
        let plain = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        chart.config_mut().picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::ValueLabel {
                source_id: None,
                series_id: "two".into(),
                category_index: 0,
                part: None,
            }],
            ..DataSelectionsConfig::default()
        });
        let selected = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let ink = bbox(&plain, slot(&chart, direction, 0, scale), is_label).unwrap();
        let content = content_box("20", &family, size * scale, ink);
        assert_selection_frame(&plain, &selected, content, scale, &format!("{scale}x"));
    }
}

/// The content box `(x0, y0, x1, y1)` of the label `text` in the face
/// `family` at `size` export pixels drawn as `ink`: the string's measured
/// size around its ink, as the atlas bakes it.
fn content_box(
    text: &str,
    family: &str,
    size: f32,
    ink: (u32, u32, u32, u32),
) -> (f32, f32, f32, f32) {
    let want = raster(text, family, size, MAGENTA);
    let want_ink = bbox(&want, (0, 0, want.width, want.height), is_label).unwrap();
    let m = measure_plain_text(text, family, size, false, false, FontPolicy::Standard);
    let (x0, y0) = (
        ink.0 as f32 - want_ink.0 as f32,
        ink.1 as f32 - want_ink.1 as f32,
    );
    (x0, y0, x0 + m.width, y0 + (m.ascent + m.descent).ceil())
}

/// Every pixel `selected` changes over `plain` lies on the selection frame
/// around `content` (a label's content box grown by `SELECTION_PADDING`,
/// stroked `SELECTION_STROKE_WIDTH` wide, at `scale`) in the selection
/// colour, and every edge of the frame is drawn.
fn assert_selection_frame(
    plain: &RasterImage,
    selected: &RasterImage,
    content: (f32, f32, f32, f32),
    scale: f32,
    case: &str,
) {
    let pad = SELECTION_PADDING * scale;
    let frame = (
        content.0 - pad,
        content.1 - pad,
        content.2 + pad,
        content.3 + pad,
    );
    let half = SELECTION_STROKE_WIDTH * scale * 0.5 + 1.0;
    let mut changed = 0;
    let mut edges = [false; 4];
    for y in 0..plain.height {
        for x in 0..plain.width {
            let (a, b) = (pixel(plain, x, y), pixel(selected, x, y));
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
                "{case}: ({x}, {y}) changed off the frame {frame:?}"
            );
            for (edge, hit) in edges.iter_mut().zip(on) {
                *edge |= hit;
            }
            // The box is drawn in the selection colour, a blue: over a
            // transparent background it is that colour, over a drawn
            // outline it turns the pixel toward it.
            assert!(
                (b[2] > b[0] && b[2] > b[1]) || b[2] > a[2],
                "{case}: ({x}, {y}) is {b:?} over {a:?}"
            );
        }
    }
    assert!(
        changed > 0 && edges.iter().all(|e| *e),
        "{case}: {changed} pixels, edges {edges:?}"
    );
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
        marker: None,
        text_scale: Default::default(),
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
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&parent, &series),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
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
            test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap()
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
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
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
            test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap()
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

/// Fully covered label ink, over a transparent background or a dark bar
/// alike: the same predicate measures a label wherever it is drawn.
fn is_ink_core(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 40 && p[2] > 200
}

/// The typed pick of `chart` at the export pixel `at`, at `scale`.
fn pick_at(
    r: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    img: &RasterImage,
    scale: f32,
    at: (f32, f32),
) -> Option<PickedData> {
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let request = GpuPickRequest {
        canvas_position_px: [at.0, at.1],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: img.width,
            height: img.height,
        },
        display_scale: scale,
        max_distance_px: 0.0,
    };
    let picked = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    picked
}

/// The data area in export pixels, `(x0, y0, x1, y1)`.
fn data_area_px(chart: &Chart, scale: f32) -> (f32, f32, f32, f32) {
    let da = chart.config().data_area().expect("data area");
    (
        da.x as f32 * scale,
        da.y as f32 * scale,
        (da.x + da.width) as f32 * scale,
        (da.y + da.height) as f32 * scale,
    )
}

fn assert_inside(ink: (u32, u32, u32, u32), area: (f32, f32, f32, f32), case: &str) {
    assert!(
        ink.0 as f32 >= area.0
            && ink.1 as f32 >= area.1
            && ink.2 as f32 + 1.0 <= area.2
            && ink.3 as f32 + 1.0 <= area.3,
        "{case}: ink {ink:?} leaves the data area {area:?}"
    );
}

fn assert_picks(picked: Option<PickedData>, series: &str, category: usize, case: &str) {
    match picked {
        Some(PickedData::ValueLabel {
            series_id,
            category_index,
            ..
        }) => assert_eq!(
            (series_id.as_str(), category_index),
            (series, category),
            "{case}"
        ),
        other => panic!("{case}: the flipped label picked {other:?}"),
    }
}

/// An outside bar label that would cross the data area's edge goes inside
/// its bar's end instead, as the bars' `Auto` rule puts a label inside:
/// fully drawn, the size the same string has where it does not flip, inside
/// the data area and the bar, and picked there, at 1x and 2x, vertical and
/// horizontal. A label with room keeps its outside place.
#[test]
fn an_edge_bar_label_flips_inside_its_end() {
    let mut r = renderer();
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        let roomy = bar_chart(CategoryBarMode::Grouped, direction, (0.0, 40.0));
        // `20` ends a few pixels short of the edge; `10` and `15` have room.
        let edge = bar_chart(CategoryBarMode::Grouped, direction, (0.0, 21.0));
        let series = [bars(
            "two",
            BLUE,
            direction,
            CategoryBarLabels::Outside,
            &["20", "10", "15"],
        )];
        for scale in SCALES {
            let case = format!("{direction:?} {scale}x");
            let roomy_img = r
                .export_rgba(
                    renderer::ExportSource::chart(&roomy, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
            let want = bbox(&roomy_img, slot(&roomy, direction, 0, scale), is_ink_core)
                .expect("the label with room");
            let img = r
                .export_rgba(
                    renderer::ExportSource::chart(&edge, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
            let bar = bbox(&img, slot(&edge, direction, 0, scale), is_blue).unwrap();
            let inside = (bar.0, bar.1, bar.2 + 1, bar.3 + 1);
            let ink = bbox(&img, inside, is_ink_core)
                .unwrap_or_else(|| panic!("{case}: no label inside the bar {bar:?}"));
            assert_eq!(
                (ink.2 - ink.0, ink.3 - ink.1),
                (want.2 - want.0, want.3 - want.1),
                "{case}: the flipped label is drawn whole"
            );
            assert_inside(ink, data_area_px(&edge, scale), &case);
            let end = value_px(&edge, direction, 20.0, scale);
            let gap = 6.0 * scale;
            match direction {
                CategoryBarDirection::Vertical => assert!(
                    ink.1 as f32 >= end + gap - (2.0 * scale + 1.0),
                    "{case}: ink top {} for an end at {end}",
                    ink.1
                ),
                CategoryBarDirection::Horizontal => assert!(
                    (ink.2 as f32 + 1.0 - (end - gap)).abs() <= 2.0 * scale + 1.0,
                    "{case}: ink right {} for an end at {end}",
                    ink.2 + 1
                ),
            }
            assert_picks(
                pick_at(&mut r, &edge, &series, &img, scale, centre(ink)),
                "two",
                0,
                &case,
            );
            // The `15` bar has room: its label stays past its end.
            let bar = bbox(&img, slot(&edge, direction, 2, scale), is_blue).unwrap();
            assert_eq!(
                bbox(&img, (bar.0, bar.1, bar.2 + 1, bar.3 + 1), is_ink_core),
                None,
                "{case}: a label with room flipped"
            );
        }
    }
}

/// A box label that would cross the data area's edge beside its box goes
/// to the other side of the box (left of a vertical box, under a
/// horizontal one): fully drawn, inside the data area, `10` px from the
/// box, still on the median, picked there, at 1x and 2x (#272). A box with
/// room keeps its label on its own side.
#[test]
fn an_edge_box_label_flips_to_the_other_side() {
    let mut r = renderer();
    let median = BOXES[2][2];
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        let mut roomy = box_chart(direction);
        if let Some(groups) = roomy.config_mut().bar_groups.as_mut() {
            groups.group_width = 0.2;
        }
        // The last box (the rightmost, or the top one) nearly fills its
        // slot: its side is a few pixels short of the data area's edge.
        let mut edge = box_chart(direction);
        if let Some(groups) = edge.config_mut().bar_groups.as_mut() {
            groups.group_width = 0.95;
        }
        let series = [boxes(direction, &["", "", "25"])];
        for scale in SCALES {
            let case = format!("{direction:?} {scale}x");
            let everywhere = |img: &RasterImage| (0, 0, img.width, img.height);
            let roomy_img = r
                .export_rgba(
                    renderer::ExportSource::chart(&roomy, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
            let want =
                bbox(&roomy_img, everywhere(&roomy_img), is_ink_core).expect("the label with room");
            let img = r
                .export_rgba(
                    renderer::ExportSource::chart(&edge, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
            let ink = bbox(&img, everywhere(&img), is_ink_core)
                .unwrap_or_else(|| panic!("{case}: no label"));
            assert_eq!(
                (ink.2 - ink.0, ink.3 - ink.1),
                (want.2 - want.0, want.3 - want.1),
                "{case}: the flipped label is drawn whole"
            );
            assert_inside(ink, data_area_px(&edge, scale), &case);
            let body = bbox(&img, slot(&edge, direction, 2, scale), is_red).unwrap();
            let at = value_px(&edge, direction, median, scale);
            let gap = 10.0 * scale;
            let slack = 2.0 * scale + 1.0;
            let (cx, cy) = centre(ink);
            match direction {
                CategoryBarDirection::Vertical => {
                    assert!(
                        (ink.2 as f32 + 1.0 - (body.0 as f32 - gap)).abs() <= slack,
                        "{case}: ink right {} for a box left edge at {}",
                        ink.2 + 1,
                        body.0
                    );
                    assert!(
                        (cy - at).abs() <= slack,
                        "{case}: label y {cy}, median {at}"
                    );
                }
                CategoryBarDirection::Horizontal => {
                    assert!(
                        ink.1 as f32 >= body.3 as f32 + 1.0 + gap - slack,
                        "{case}: ink top {} for a box bottom at {}",
                        ink.1,
                        body.3 + 1
                    );
                    assert!(
                        (cx - at).abs() <= slack,
                        "{case}: label x {cx}, median {at}"
                    );
                }
            }
            assert_picks(
                pick_at(&mut r, &edge, &series, &img, scale, centre(ink)),
                "box",
                2,
                &case,
            );
        }
    }
}

/// A radar label that would cross the data area's edge or the outer ring
/// (where the category labels sit, #272) outward along its spoke goes
/// inward along it instead: fully drawn, inside the data area, its content
/// box its gap past the vertex on the centre's side and next to it, picked
/// there, at 1x and 2x. The top and bottom vertices flip; the right one,
/// with room inside the ring, does not.
#[test]
fn an_edge_radar_label_flips_inward() {
    let mut r = renderer();
    // The top and bottom vertices on the outer ring of a 0 to 0.7 axis; the
    // right one at its middle, its label well inside the ring.
    const EDGE: [f64; 4] = [0.7, 0.35, 0.7, 0.4];
    r.add_column("radar.edge", &col(EDGE.to_vec())).unwrap();
    let roomy = radar_chart();
    let mut edge = radar_chart();
    edge.set_y_range(0.0, 0.7);
    let series = [radar(RadarLabels::Values)];
    let mut edge_series = series.clone();
    edge_series[0].y_column = "radar.edge".into();
    for scale in SCALES {
        let roomy_img = r
            .export_rgba(
                renderer::ExportSource::chart(&roomy, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&edge, &edge_series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let area = data_area_px(&edge, scale);
        let slack = 2.0 * scale + 1.0;
        let clear = radar_clear(&edge, scale);
        for i in [0usize, 2] {
            let case = format!("{scale}x vertex {i}");
            let (_, ry) = vertex(&roomy, scale, i as f64, RADAR[i]);
            let (vx, vy) = vertex(&edge, scale, i as f64, EDGE[i]);
            let h = img.height as f32;
            // The label with room: beyond the roomy vertex; the flipped one:
            // between the edge vertex and the centre.
            let (rx, _) = vertex(&roomy, scale, i as f64, RADAR[i]);
            let (bx0, bx1) = ((rx - 40.0 * scale) as u32, (rx + 40.0 * scale) as u32);
            let beyond = |y: f32| match i {
                0 => (bx0, 0, bx1, y as u32),
                _ => (bx0, y.ceil() as u32, bx1, h as u32),
            };
            let want = bbox(&roomy_img, beyond(ry), is_ink_core).expect("the label with room");
            // Around the spoke, between the vertex and the centre.
            let (x0, x1) = ((vx - 40.0 * scale) as u32, (vx + 40.0 * scale) as u32);
            let inward = match i {
                0 => (x0, vy.ceil() as u32, x1, (vy + 40.0 * scale) as u32),
                _ => (x0, (vy - 40.0 * scale) as u32, x1, vy as u32),
            };
            let ink = bbox(&img, inward, is_ink_core)
                .unwrap_or_else(|| panic!("{case}: no label inward of {vy}"));
            assert_eq!(
                (ink.2 - ink.0, ink.3 - ink.1),
                (want.2 - want.0, want.3 - want.1),
                "{case}: the flipped label is drawn whole"
            );
            assert_inside(ink, area, &case);
            assert!(
                (centre(ink).0 - vx).abs() <= slack,
                "{case}: {ink:?} at {vx}"
            );
            let b = content_box(
                &format!("{}", EDGE[i]),
                &edge.config().left_y.label_style.label_font,
                edge.config().left_y.label_style.font_size * scale,
                ink,
            );
            // Below the top vertex, above the bottom one.
            let near = if i == 0 { b.1 - vy } else { vy - b.3 };
            assert!(
                (near - clear).abs() <= slack,
                "{case}: {b:?} {near} px inward of {vy}"
            );
            assert_next_to_vertex(&edge, b, (vx, vy), 2.0, scale, &case);
            assert_picks(
                pick_at(&mut r, &edge, &edge_series, &img, scale, centre(ink)),
                "radar",
                i,
                &case,
            );
        }
        // The right vertex has room: its label stays outside.
        let (vx, vy) = vertex(&edge, scale, 1.0, EDGE[1]);
        let right = bbox(
            &img,
            (
                vx.ceil() as u32,
                (vy - 20.0 * scale) as u32,
                img.width,
                (vy + 20.0 * scale) as u32,
            ),
            is_ink_core,
        )
        .expect("the right vertex's label outside");
        assert!(
            right.0 as f32 >= vx + clear - slack,
            "{scale}x: {right:?} at {vx}"
        );
    }
}

/// Value labels have no count cap (#107 R11): a bar series over 300
/// categories, past the old 64 cells of the label uniform, labels every
/// category. The cells are a storage array sized by the labels, so every
/// sampled label (the first, the 64th and 65th, the last) draws above its
/// bar at 1x and 2x, picks as its own category, and a drag of the last
/// label moves it by its offset and nothing else.
#[test]
fn three_hundred_value_labels_draw_pick_and_drag() {
    const N: usize = 300;
    // 8000 export pixels at 2x: the device's texture limit.
    const WIDE: u32 = 4000;
    let mut r = renderer();
    r.add_column("pos300", &col((0..N).map(|k| k as f64).collect()))
        .unwrap();
    r.add_column("v300", &col(vec![10.0; N])).unwrap();
    let direction = CategoryBarDirection::Vertical;
    let mut chart = bar_chart(CategoryBarMode::Grouped, direction, (0.0, 25.0));
    {
        let config = chart.config_mut();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: WIDE,
            height: HEIGHT,
        });
        config.bottom_x.categories = Some(categories(N));
    }
    // One digit per label, so a label fits its 13 px slot.
    let text: Vec<String> = (0..N).map(|k| format!("{}", k % 10)).collect();
    let text: Vec<&str> = text.iter().map(String::as_str).collect();
    let mut series = bars("v300", RED, direction, CategoryBarLabels::Outside, &text);
    series.x_column = "pos300".into();
    let still = [series];
    // Category `k`'s slot, the whole panel across it.
    let region = |k: usize, scale: f32| {
        let da = chart.config().data_area().expect("data area");
        let w = da.width as f32 / N as f32;
        let x0 = da.x as f32 + w * k as f32;
        let s = |v: f32| (v * scale).round() as u32;
        (s(x0), 0, s(x0 + w), s(HEIGHT as f32))
    };
    let sampled = [0, 63, 64, 150, N - 1];
    for scale in SCALES {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &still),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        for k in sampled {
            let bar = bbox(&img, region(k, scale), is_red)
                .unwrap_or_else(|| panic!("{scale}x: no bar in category {k}"));
            let ink = bbox(
                &img,
                (region(k, scale).0, 0, region(k, scale).2, bar.1),
                is_label,
            )
            .unwrap_or_else(|| panic!("{scale}x: no label above bar {k}"));
            // Centred across the bar, above its value end.
            let (cx, _) = centre(ink);
            let (bx, _) = centre(bar);
            assert!(
                (cx - bx).abs() <= 2.0 * scale,
                "{scale}x label {k}: {cx} over {bx}"
            );
            assert!(ink.3 < bar.1, "{scale}x label {k} sits above its bar");
        }
        // Each sampled label picks as its own category.
        for k in [64, N - 1] {
            let bar = bbox(&img, region(k, scale), is_red).unwrap();
            let ink = bbox(
                &img,
                (region(k, scale).0, 0, region(k, scale).2, bar.1),
                is_label,
            )
            .unwrap();
            assert_picks(
                pick_at(&mut r, &chart, &still, &img, scale, centre(ink)),
                "v300",
                k,
                &format!("{scale}x label {k}"),
            );
        }
    }
    // The last label drags up by its offset; the label of category 0
    // stays.
    let mut dragged = still.clone();
    assert_eq!(
        dragged[0].drag_value_label(N - 1, N, 0.0, -30.0),
        renderer::layout::NudgeResult::Moved
    );
    for scale in SCALES {
        let before = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &still),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let after = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &dragged),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let band = |k: usize| region(k, scale);
        let (a, b) = (
            bbox(&before, band(N - 1), is_label).unwrap(),
            bbox(&after, band(N - 1), is_label).unwrap(),
        );
        assert_eq!(
            (b.0 as i64 - a.0 as i64, b.1 as i64 - a.1 as i64),
            (0, (-30.0 * scale) as i64),
            "{scale}x: the last label moves by its offset"
        );
        assert_eq!(
            bbox(&before, band(0), is_label),
            bbox(&after, band(0), is_label)
        );
    }
}
