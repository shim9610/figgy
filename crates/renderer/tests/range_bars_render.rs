//! End-to-end pixel verification of range bar series
//! (`DataRenderType::RangeBars`) through the public renderer API and the
//! headless `export_rgba`: bars floating from a start to an end value
//! in both directions at 1x and 2x, the direction, total and state colours,
//! whiskers, connectors and progress, missing rows, a numeric (time)
//! position axis with a slot width, the value-axis and position-axis fit,
//! the picked highlight and the pick of every part, value labels, column
//! replacement, the streaming rejection, overlays and subplots, and a
//! timeline's row colours with its first row at the top.
//!
//! Every part has its own pure colour, all flat with no outline, so a
//! colour predicate attributes every pixel to one part. Expected extents
//! come from the same data area and axis ranges the renderer draws with.

use test_gpu::BlockOn as _;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;
use harness::{
    BLUE, CYAN, DIRECTIONS, GREEN, MAGENTA, ORANGE, RED, Region, YELLOW, across_span, bbox, col,
    flat_style, is_blue, is_cyan, is_green, is_magenta, is_orange, is_red, is_yellow, pixel, slot,
    value_px,
};
use test_gpu::shared_device;

use renderer::config::Category;
use renderer::data_config::{
    BarGroups, CategoryBarDirection, CategoryBarLabels, CategoryBarMode, CategoryBarSeriesLabels,
    RangeColor, RangeConnector, RangeProgress, RangeStateStyle, RangeTotal, RangeWhiskers,
};
use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, Color, DataRenderType, DataSelectionsConfig, PickedDataRef, RangeBarSeries, Renderer,
    RendererDevice, SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const NAN: f64 = f64::NAN;

/// Three rows over the categories a, b, c: a rising bar, a falling bar and
/// a third rising one.
const START: [f64; 3] = [20.0, 70.0, 30.0];
const END: [f64; 3] = [60.0, 40.0, 80.0];
const LOW: [f64; 3] = [10.0, 30.0, 25.0];
const HIGH: [f64; 3] = [70.0, 85.0, 90.0];

/// A renderer holding the positions and every column of the three rows.
fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
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

fn ranges(id: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "pos".into(),
        y_column: "end".into(),
        z_column: None,
        render_type: DataRenderType::RangeBars(RangeBarSeries {
            color,
            style: flat_style(),
            start: "start".into(),
            whiskers: None,
            color_by: RangeColor::Single,
            progress: None,
            connector: None,
            slot_width: None,
            labels: CategoryBarSeriesLabels::default(),
        }),
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
    let DataRenderType::RangeBars(RangeBarSeries {
        whiskers,
        color_by,
        progress,
        connector,
        labels,
        ..
    }) = &mut series.render_type
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

/// The value-axis span `(smaller value end, larger value end)` of a box in
/// export pixels.
fn value_span(b: Region, direction: CategoryBarDirection) -> (f32, f32) {
    match direction {
        CategoryBarDirection::Vertical => (b.3 as f32 + 1.0, b.1 as f32),
        CategoryBarDirection::Horizontal => (b.0 as f32, b.2 as f32 + 1.0),
    }
}

fn selection(series_id: &str, row: usize) -> DataSelectionsConfig {
    DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: series_id.into(),
            bin_index: row,
            half: None,
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
            let img = r
                .export_rgba(
                    renderer::ExportSource::chart(&chart, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
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
    let hit = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
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
            .export_rgba(
                renderer::ExportSource::chart(&chart, std::slice::from_ref(&series)),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
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
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &[states]),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    // States 1, 0 and 7: the third has no style.
    assert!(bbox(&img, slot(&chart, direction, 0, 1.0), is_blue).is_some());
    assert!(bbox(&img, slot(&chart, direction, 1, 1.0), is_green).is_some());
    assert!(bbox(&img, slot(&chart, direction, 2, 1.0), is_red).is_some());
}

/// A timeline's row colours (#357): row `i` takes colour `i % count` with
/// the first row at the top of an inverted axis, at 1x and 2x; no colours
/// draw the series colour; the highlight traces the picked row's body and
/// the pick answers its row; the vector exports embed the same data; and
/// a serde round trip draws the same pixels.
#[test]
fn rows_take_their_colours_and_keep_every_interaction() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Horizontal;
    let mut chart = chart(direction);
    chart.config_mut().left_y.inverted = true;
    let mut series = ranges("one", RED);
    *fields(&mut series).1 = RangeColor::ByRow {
        colors: vec![GREEN, BLUE],
    };
    let series = [series];
    // The axis is inverted: row `k` sits in the bottom-up slot `2 - k`.
    let row = |chart: &Chart, k: usize, scale: f32| slot(chart, direction, 2 - k, scale);
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        assert!(
            bbox(&img, row(&chart, 0, scale), is_green).is_some(),
            "{scale}x row 0"
        );
        assert!(
            bbox(&img, row(&chart, 1, scale), is_blue).is_some(),
            "{scale}x row 1"
        );
        assert!(
            bbox(&img, row(&chart, 2, scale), is_green).is_some(),
            "{scale}x row 2"
        );
        assert_eq!(bbox(&img, row(&chart, 1, scale), is_green), None);
        assert_eq!(bbox(&img, (0, 0, img.width, img.height), is_red), None);
        // The bars stand where their values are, whatever their colour.
        let body = bbox(&img, row(&chart, 1, scale), is_blue).unwrap();
        let (lo, hi) = value_span(body, direction);
        let (a, b) = (
            value_px(&chart, direction, END[1], scale),
            value_px(&chart, direction, START[1], scale),
        );
        assert!(
            (lo - a).abs() <= 1.5 && (hi - b).abs() <= 1.5,
            "{scale}x {body:?}"
        );

        // The picked row's highlight traces its body; the pick answers it.
        chart.config_mut().picked_data = Some(selection("one", 1));
        let outlined = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let ring = bbox(
            &outlined,
            (0, 0, outlined.width, outlined.height),
            is_magenta,
        )
        .expect("the selected bar is outlined");
        assert!(
            [
                (ring.0, body.0),
                (ring.1, body.1),
                (ring.2, body.2),
                (ring.3, body.3)
            ]
            .iter()
            .all(|(a, b)| a.abs_diff(*b) <= 1),
            "{scale}x: the outline {ring:?} does not trace the body {body:?}"
        );
        chart.config_mut().picked_data = None;
        let centre = (
            (body.0 + body.2) as f32 / 2.0,
            (body.1 + body.3) as f32 / 2.0,
        );
        assert_eq!(
            registered_pick(&mut r, &chart, &series, scale, centre, 0.0),
            Some(("one".into(), 1)),
            "{scale}x"
        );
    }

    // No row colours: every bar in the series colour.
    let mut plain = series[0].clone();
    *fields(&mut plain).1 = RangeColor::ByRow { colors: Vec::new() };
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, std::slice::from_ref(&plain)),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    for k in 0..3 {
        assert!(bbox(&img, row(&chart, k, 1.0), is_red).is_some(), "row {k}");
    }

    // The SVG and the PDF embed the drawn data image.
    let svg = r
        .export_svg_blocking(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .unwrap();
    assert!(svg.contains("<image"), "the SVG embeds the data image");
    let pdf = r
        .export_pdf_blocking(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .unwrap();
    assert!(pdf.starts_with(b"%PDF"));

    // A document saved and read back draws the same pixels.
    #[cfg(feature = "serde")]
    {
        let draw = |r: &mut Renderer, chart: &mut Chart, series: &[SeriesConfig]| {
            r.export_rgba(
                renderer::ExportSource::chart(chart, series),
                renderer::ExportOptions::panel(1.0),
            )
            .block_on()
            .unwrap()
            .rgba
        };
        let before = draw(&mut r, &mut chart, &series);
        let json = serde_json::to_string(&(chart.config(), &series)).unwrap();
        let (config, again): (renderer::config::Config, Vec<SeriesConfig>) =
            serde_json::from_str(&json).unwrap();
        assert_eq!(again.as_slice(), &series);
        let mut again_chart = Chart::new(config);
        assert_eq!(draw(&mut r, &mut again_chart, &again), before);
    }
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
            let img = r
                .export_rgba(
                    renderer::ExportSource::chart(&chart, &series),
                    renderer::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
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
            .export_rgba(
                renderer::ExportSource::chart(&chart, std::slice::from_ref(&series)),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
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
    let DataRenderType::RangeBars(RangeBarSeries {
        start, color_by, ..
    }) = &mut series.render_type
    else {
        unreachable!()
    };
    *start = "gappy.start".into();
    *color_by = RangeColor::Single;
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &[series]),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
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
    if let DataRenderType::RangeBars(RangeBarSeries { style, .. }) = &mut series.render_type {
        style.corner_radius = 3.0;
    }
    let series = [series];
    for scale in [1.0f32, 2.0] {
        chart.config_mut().picked_data = None;
        let painted = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let body = bbox(&painted, slot(&chart, direction, 1, scale), is_blue).expect("body");
        chart.config_mut().picked_data = Some(selection("one", 1));
        let outlined = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
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
        .export_rgba(
            renderer::ExportSource::chart(&chart, std::slice::from_ref(&series)),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
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
        .export_rgba(
            renderer::ExportSource::chart(&chart, std::slice::from_ref(&series)),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
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
    if let DataRenderType::RangeBars(RangeBarSeries { start, .. }) = &mut gantt.render_type {
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
        .export_rgba(
            renderer::ExportSource::chart(&gantt_chart, std::slice::from_ref(&gantt)),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
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
    if let DataRenderType::RangeBars(RangeBarSeries { slot_width, .. }) = &mut bars.render_type {
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
        .export_rgba(
            renderer::ExportSource::chart(&candles, std::slice::from_ref(&bars)),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
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
        .export_rgba(
            renderer::ExportSource::chart(&chart, &[ranges("one", RED)]),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
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
    let before = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    r.remove_column("start").unwrap();
    r.add_column("start", &col(START.iter().map(|v| v - 10.0).collect()))
        .unwrap();
    let after = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
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
    if let DataRenderType::RangeBars(RangeBarSeries { start, .. }) = &mut short.render_type {
        *start = "short".into();
    }
    let error = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &[short]),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .err()
        .expect("an error");
    assert!(error.to_string().contains("\"short\""), "{error}");
    let mut missing = ranges("one", RED);
    if let DataRenderType::RangeBars(RangeBarSeries { start, .. }) = &mut missing.render_type {
        *start = "nowhere".into();
    }
    let error = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &[missing]),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
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
        marker: None,
        text_scale: Default::default(),
    });
    // The overlay and the subplot colour their bars by row (#357), each
    // over a series colour of its own that must not show.
    let by_row = |id: &str, color: Color| {
        let mut series = ranges(id, YELLOW);
        *fields(&mut series).1 = RangeColor::ByRow {
            colors: vec![color],
        };
        series
    };
    let series = [
        ranges("one", RED),
        by_row("two", BLUE),
        by_row("below", GREEN),
    ];
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&parent, &series),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
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
        assert_eq!(bbox(&img, (0, 0, w, h), is_yellow), None);
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
            match test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap() {
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
        let one = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(1.0),
            )
            .block_on()
            .unwrap();
        let two = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(2.0),
            )
            .block_on()
            .unwrap();
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
    // Seven row colours cycle over every row: no row is left uncoloured.
    let mut bars = ranges("one", BLUE);
    *fields(&mut bars).1 = RangeColor::ByRow {
        colors: vec![RED; 7],
    };
    bars.x_column = "n.pos".into();
    bars.y_column = "n.end".into();
    if let DataRenderType::RangeBars(RangeBarSeries {
        start, slot_width, ..
    }) = &mut bars.render_type
    {
        *start = "n.start".into();
        *slot_width = Some(1.0);
    }
    let series = [bars];
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
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
    let hit = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    eprintln!("range bar pick over {N} rows: {:?}", started.elapsed());
    assert!(
        matches!(&hit, Some(renderer::PickedData::HistogramBin { bin_index, .. }) if *bin_index == N - 1),
        "{hit:?}"
    );
    r.remove_chart(id).unwrap();
    chart.config_mut().picked_data = Some(selection("one", N - 1));
    let outlined = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    let x = x_of(N - 1) as u32;
    assert!(bbox(&outlined, (x - 3, 0, x + 4, HEIGHT), is_magenta).is_some());
    assert_eq!(bbox(&outlined, (0, 0, x - 10, HEIGHT), is_magenta), None);

    // The cost grows with the rows; it is measured, never capped.
    const BIG: usize = 100_000;
    for id in ["n.pos", "n.start", "n.end"] {
        r.remove_column(id).unwrap();
    }
    r.add_column("n.pos", &col((0..BIG).map(|k| k as f64).collect()))
        .unwrap();
    r.add_column(
        "n.start",
        &col((0..BIG).map(|k| 10.0 + (k % 7) as f64).collect()),
    )
    .unwrap();
    r.add_column(
        "n.end",
        &col((0..BIG).map(|k| 60.0 + (k % 11) as f64).collect()),
    )
    .unwrap();
    chart.config_mut().picked_data = None;
    chart.config_mut().bottom_x.max = BIG as f64 - 0.5;
    let _ = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    let started = std::time::Instant::now();
    let _ = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    let draw = started.elapsed();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let started = std::time::Instant::now();
    let hit = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    let pick = started.elapsed();
    assert!(hit.is_some(), "a bar under the pointer of {BIG} rows");
    eprintln!("range bars over {BIG} rows: export {draw:?}, pick {pick:?}");
    r.remove_chart(id).unwrap();
}

/// A dark navy, a pale cream and the dark label ink of the contrast tests.
const NAVY: Color = Color::from_rgb8(20, 30, 90);
const CREAM: Color = Color::from_rgb8(250, 240, 200);
const INK: Color = Color::from_rgb8(20, 20, 20);

fn is_navy(p: &[u8]) -> bool {
    p[3] > 200 && p[0] < 40 && p[1] < 50 && (70..110).contains(&p[2])
}

fn is_cream(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 240 && (230..250).contains(&p[1]) && (185..215).contains(&p[2])
}

/// Label ink in white, over any fill.
fn is_light_ink(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 235 && p[1] > 235 && p[2] > 235
}

/// Label ink in the dark ink, over any fill.
fn is_dark_ink(p: &[u8]) -> bool {
    p[3] > 200 && p[0] < 45 && p[1] < 45 && p[2] < 45
}

/// Three rows labelled inside or beside their bar in contrast ink (#357):
/// row 0 dark navy, rows 1 and 2 pale cream, row 1's label too long for
/// its bar.
fn contrast_series() -> SeriesConfig {
    let mut series = ranges("one", RED);
    let (_, color_by, _, _, labels) = fields(&mut series);
    *color_by = RangeColor::ByRow {
        colors: vec![NAVY, CREAM, CREAM],
    };
    labels.placement = CategoryBarLabels::InsideOrBeside;
    labels.contrast_ink = Some([INK, Color::WHITE]);
    labels.text = vec!["Alpha".into(), "Beside its bar".into(), "Gamma".into()];
    series
}

/// Inside-or-beside labels with contrast ink (#357): a label that fits sits
/// inside its bar in the ink that contrasts more with the bar (white on
/// navy, dark on cream), one that does not sits beside the bar's end in the
/// ink that contrasts with the plot; the ink is chosen per pixel, so a
/// label across a progress split is white over the finished part and dark
/// over the light unfinished part. The labels pick, outline and drag where
/// they are drawn at 1x and 2x, the vector exports embed them, and a saved
/// document draws the same.
#[test]
fn inside_or_beside_labels_take_contrast_ink_and_keep_every_interaction() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Horizontal;
    let mut chart = chart(direction);
    let series = [contrast_series()];
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&mut chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let row = |k: usize| slot(&chart, direction, k, scale);
        // Row 0: white ink inside the navy bar.
        let navy = bbox(&img, row(0), is_navy).expect("row 0's bar");
        let inside = (navy.0, navy.1, navy.2 + 1, navy.3 + 1);
        let white = bbox(&img, inside, is_light_ink).expect("white ink on navy");
        assert_eq!(bbox(&img, inside, is_dark_ink), None, "{scale}x row 0");
        // Row 2: dark ink inside its cream bar.
        let cream = bbox(&img, row(2), is_cream).expect("row 2's bar");
        let cream_box = (cream.0, cream.1, cream.2 + 1, cream.3 + 1);
        let dark = bbox(&img, cream_box, is_dark_ink).expect("dark ink on cream");
        assert_eq!(bbox(&img, cream_box, is_light_ink), None, "{scale}x row 2");
        // Row 1: the long label sits beside the falling bar's end (its
        // smaller value, on the left), dark on the plot, none inside.
        let short = bbox(&img, row(1), is_cream).expect("row 1's bar");
        let short_box = (short.0, short.1, short.2 + 1, short.3 + 1);
        assert_eq!(bbox(&img, short_box, is_dark_ink), None, "{scale}x row 1");
        assert_eq!(bbox(&img, short_box, is_light_ink), None, "{scale}x row 1");
        let r1 = row(1);
        let left = (r1.0, r1.1, short.0, r1.3);
        let beside = bbox(&img, left, is_dark_ink).expect("the long label beside its bar");
        assert!(beside.2 < short.0, "{scale}x {beside:?} {short:?}");

        // Each label picks where it is drawn, inside or beside.
        let centre = |b: Region| ((b.0 + b.2) as f32 / 2.0, (b.1 + b.3) as f32 / 2.0);
        r.enable_gpu_picking().unwrap();
        let id = r
            .register_chart(chart.config().clone(), series.to_vec())
            .unwrap();
        for (k, ink) in [(0, white), (1, beside), (2, dark)] {
            let request = renderer::GpuPickRequest {
                canvas_position_px: [centre(ink).0, centre(ink).1],
                display_panel_px: Rect {
                    x: 0,
                    y: 0,
                    width: img.width,
                    height: img.height,
                },
                display_scale: scale,
                max_distance_px: 0.0,
            };
            match test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap() {
                Some(renderer::PickedData::ValueLabel {
                    series_id,
                    category_index,
                    ..
                }) => assert_eq!((series_id.as_str(), category_index), ("one", k)),
                other => panic!("{scale}x label {k} picked {other:?}"),
            }
        }
        r.remove_chart(id).unwrap();

        // The selected label is outlined around where it is drawn.
        chart.config_mut().picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::ValueLabel {
                source_id: None,
                series_id: "one".into(),
                category_index: 1,
                part: None,
            }],
            ..DataSelectionsConfig::default()
        });
        let selected = r
            .export_rgba(
                renderer::ExportSource::chart(&mut chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        chart.config_mut().picked_data = None;
        let changed = (r1.1..r1.3.min(img.height))
            .flat_map(|y| (r1.0..short.0).map(move |x| (x, y)))
            .filter(|&(x, y)| pixel(&img, x, y) != pixel(&selected, x, y))
            .count();
        assert!(changed > 0, "{scale}x: the selected label is outlined");

        // A dragged label moves by its offset.
        let mut dragged = series.clone();
        fields(&mut dragged[0]).4.offsets = vec![[0.0, 0.0], [0.0, -12.0], [0.0, 0.0]];
        let moved = r
            .export_rgba(
                renderer::ExportSource::chart(&mut chart, &dragged),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let near = (
            beside.0 - 2,
            beside.1 - (20.0 * scale) as u32,
            short.0,
            beside.3 + 1,
        );
        let after = bbox(&moved, near, is_dark_ink).expect("the dragged label");
        assert!(
            (beside.1 as f32 - after.1 as f32 - 12.0 * scale).abs() <= 1.5,
            "{scale}x {beside:?} -> {after:?}"
        );
        // A label dragged off its navy bar reads the plot under it: dark,
        // never white on white.
        let mut off_bar = series.clone();
        fields(&mut off_bar[0]).4.offsets = vec![[0.0, 60.0], [0.0, 0.0], [0.0, 0.0]];
        let off = r
            .export_rgba(
                renderer::ExportSource::chart(&mut chart, &off_bar),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let below = (navy.0, navy.3 + 1, navy.2 + 1, img.height);
        assert!(
            bbox(&off, below, is_dark_ink).is_some(),
            "{scale}x off its bar"
        );
        assert_eq!(
            bbox(&off, below, is_light_ink),
            None,
            "{scale}x off its bar"
        );
    }

    // Across a progress split: white over the finished navy part, dark over
    // its light unfinished part.
    let mut split = series.clone();
    *fields(&mut split[0]).2 = Some(RangeProgress {
        column: "progress".into(),
        remaining_opacity: 0.2,
    });
    fields(&mut split[0]).4.text[0] = "Alpha beta gamma".into();
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&mut chart, &split),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    let at = value_px(&chart, direction, 0.5 * (START[0] + END[0]), 1.0) as u32;
    let row0 = slot(&chart, direction, 0, 1.0);
    // From the bar's start: the tick marks left of the plot are dark too.
    let row0 = (
        value_px(&chart, direction, START[0], 1.0) as u32 + 1,
        row0.1,
        row0.2,
        row0.3,
    );
    assert!(
        bbox(&img, (row0.0, row0.1, at, row0.3), is_light_ink).is_some(),
        "white ink before the split"
    );
    assert!(
        bbox(&img, (at + 1, row0.1, row0.2, row0.3), is_dark_ink).is_some(),
        "dark ink past the split"
    );
    assert_eq!(
        bbox(
            &img,
            (row0.0, row0.1, at.saturating_sub(1), row0.3),
            is_dark_ink
        ),
        None,
        "no dark ink before the split"
    );

    // The SVG and the PDF embed the drawn data image, labels included.
    let svg = r
        .export_svg_blocking(
            renderer::ExportSource::chart(&mut chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .unwrap();
    assert!(svg.contains("<image"));
    let pdf = r
        .export_pdf_blocking(
            renderer::ExportSource::chart(&mut chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .unwrap();
    assert!(pdf.starts_with(b"%PDF"));

    // A document saved and read back draws the same pixels.
    #[cfg(feature = "serde")]
    {
        let before = r
            .export_rgba(
                renderer::ExportSource::chart(&mut chart, &series),
                renderer::ExportOptions::panel(1.0),
            )
            .block_on()
            .unwrap()
            .rgba;
        let json = serde_json::to_string(&(chart.config(), &series)).unwrap();
        assert!(json.contains("contrast_ink") && json.contains("InsideOrBeside"));
        let (config, again): (renderer::config::Config, Vec<SeriesConfig>) =
            serde_json::from_str(&json).unwrap();
        assert_eq!(again.as_slice(), &series);
        let after = r
            .export_rgba(
                renderer::ExportSource::chart(&mut Chart::new(config), &again),
                renderer::ExportOptions::panel(1.0),
            )
            .block_on()
            .unwrap()
            .rgba;
        assert_eq!(after, before);
    }
}

/// Rule 10: inside-or-beside labels in contrast ink on 3000 rows all draw,
/// with no cap on rows or labels.
#[test]
fn contrast_labels_draw_on_every_row() {
    let mut r = renderer();
    let n = 3000usize;
    r.add_column("many-pos", &col((0..n).map(|i| i as f64).collect()))
        .unwrap();
    r.add_column("many-start", &col(vec![0.0; n])).unwrap();
    r.add_column("many-end", &col(vec![100.0; n])).unwrap();
    let mut series = contrast_series();
    series.x_column = "many-pos".into();
    series.y_column = "many-end".into();
    if let DataRenderType::RangeBars(bars) = &mut series.render_type {
        bars.start = "many-start".into();
        bars.slot_width = Some(1.0);
        bars.labels.text = (0..n).map(|i| format!("{i}")).collect();
        bars.color_by = RangeColor::ByRow { colors: vec![NAVY] };
    }
    let mut chart = chart(CategoryBarDirection::Horizontal);
    let config = chart.config_mut();
    config.left_y.categories = None;
    config.left_y.min = n as f64 - 3.5;
    config.left_y.max = n as f64 - 0.5;
    config.left_y.major_spacing = 1.0;
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&mut chart, std::slice::from_ref(&series)),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    // The last rows, past any former cap, draw their bars and white labels.
    let whole = (0, 0, img.width, img.height);
    let navy = bbox(&img, whole, is_navy).expect("the last rows draw");
    let inside = (navy.0, navy.1, navy.2 + 1, navy.3 + 1);
    assert!(bbox(&img, inside, is_light_ink).is_some(), "labels draw");
}

/// The timeline row band colour of `default_timeline_config` (#357).
fn is_row_band(p: &[u8]) -> bool {
    p[3] == 255 && (245..=249).contains(&p[0]) && (246..=250).contains(&p[1]) && p[2] >= 248
}

/// The timeline look (#357): `timeline_bars` on `default_timeline_config`
/// draws a faint band behind every other row, under the bars, at 1x and
/// 2x; the first row (at the top) stays clear; a bar over a band still
/// picks; a subplot of the same look draws its own bands; and the saved
/// document draws the same.
#[test]
fn the_timeline_look_bands_every_other_row_under_the_bars() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Horizontal;
    let timeline = |y: u32| {
        let mut config = renderer::default::default_timeline_config();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y,
            width: WIDTH,
            height: HEIGHT,
        });
        config.legend.visible = false;
        config.chart_title.visible = false;
        config.left_y.categories = Some(vec![
            Category::new("a", "A"),
            Category::new("b", "B"),
            Category::new("c", "C"),
        ]);
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 100.0);
        chart.fit_layout();
        chart
    };
    let mut chart = timeline(0);
    let series = [SeriesConfig::timeline_bars("one", "pos", "start", "end")];
    // The axis is inverted: row `k` sits in the bottom-up slot `2 - k`.
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&mut chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let row = |k: usize| slot(&chart, direction, 2 - k, scale);
        assert_eq!(bbox(&img, row(0), is_row_band), None, "{scale}x row 0");
        let band = bbox(&img, row(1), is_row_band).expect("row 1's band");
        let r1 = row(1);
        assert!(
            band.0.abs_diff(r1.0) <= 1 && band.2.abs_diff(r1.2 - 1) <= 1,
            "{scale}x: the band spans the plot: {band:?} {r1:?}"
        );
        assert_eq!(bbox(&img, row(2), is_row_band), None, "{scale}x row 2");
        // The bar over the band picks.
        let x = value_px(&chart, direction, 0.5 * (START[1] + END[1]), scale);
        let y = (r1.1 + r1.3) as f32 / 2.0;
        assert_eq!(
            registered_pick(&mut r, &chart, &series, scale, (x, y), 0.0),
            Some(("one".into(), 1)),
            "{scale}x"
        );
    }

    // A subplot of the same look draws its own bands.
    let mut parent = timeline(0);
    let sub = timeline(HEIGHT).config().clone();
    parent.config_mut().subplots.push(renderer::Subplot {
        config: sub,
        series: vec!["below".into()],
        link_x: None,
        link_y: None,
        marker: None,
        text_scale: Default::default(),
    });
    let both = [
        SeriesConfig::timeline_bars("one", "pos", "start", "end"),
        SeriesConfig::timeline_bars("below", "pos", "start", "end"),
    ];
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&mut parent, &both),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .unwrap();
    let (w, h) = (img.width, img.height);
    assert!(
        bbox(&img, (0, 0, w, h / 2), is_row_band).is_some(),
        "the parent's"
    );
    assert!(
        bbox(&img, (0, h / 2, w, h), is_row_band).is_some(),
        "the subplot's"
    );

    #[cfg(feature = "serde")]
    {
        let draw = |r: &mut Renderer, chart: &mut Chart| {
            r.export_rgba(
                renderer::ExportSource::chart(chart, &series),
                renderer::ExportOptions::panel(2.0),
            )
            .block_on()
            .unwrap()
            .rgba
        };
        let before = draw(&mut r, &mut chart);
        let json = serde_json::to_string(chart.config()).unwrap();
        let config: renderer::config::Config = serde_json::from_str(&json).unwrap();
        assert_eq!(&config, chart.config());
        assert_eq!(draw(&mut r, &mut Chart::new(config)), before);
    }
}

/// Dependency links (#357, item 14 D) over the three test bars: link 0 from
/// row 0 to row 2, link 1 from row 0 to row 1, link 2 a NaN row (no link),
/// stroked in orange 2 px wide with an 8 px chevron, 10 px in from the
/// predecessor's start.
fn links_series(r: &mut Renderer) -> SeriesConfig {
    r.add_column("link-from", &col(vec![0.0, 0.0, NAN]))
        .unwrap();
    r.add_column("link-to", &col(vec![2.0, 1.0, 1.0])).unwrap();
    let mut links = SeriesConfig::range_links("links", "bars", "link-from", "link-to");
    if let DataRenderType::RangeLinks(l) = &mut links.render_type {
        l.color = ORANGE;
        l.width = 2.0;
        l.head = 8.0;
        l.inset = 10.0;
    }
    links
}

/// Links route between the bars they join on every draw, at 1x and 2x: ink
/// leaves the predecessor's end and reaches the successor's start; a NaN
/// row draws nothing. A link picks as a histogram bin of its row, its
/// highlight traces it, `step_selection` walks the links, the vector
/// exports embed it, a saved document draws the same, and 3000 links draw
/// with no cap (rule 10).
#[test]
fn range_links_route_between_their_bars_and_keep_every_interaction() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Horizontal;
    let mut chart = chart(direction);
    let bars = ranges("bars", GREEN);
    let links = links_series(&mut r);
    let series = [bars, links];
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        let whole = (0, 0, img.width, img.height);
        let ink = bbox(&img, whole, is_orange).expect("the links draw");
        // Both links leave row 0's edge near its start (20), the one to
        // row 2 (start 30) moved back to leave room for its head, and reach
        // row 1's start (70, a bar running back) and row 2's: the ink starts
        // within a head and an inset of 20 and never runs to row 0's end.
        let (x20, x70) = (
            value_px(&chart, direction, 20.0, scale),
            value_px(&chart, direction, 70.0, scale),
        );
        assert!(
            (ink.0 as f32) <= x20 + 10.0 * scale
                && (ink.0 as f32) >= x20 - 20.0 * scale
                && (ink.2 as f32) >= x70 - 1.0,
            "{scale}x {ink:?}"
        );
        // Ink in the successors' slots and in the predecessor's.
        for k in 0..3 {
            assert!(
                bbox(&img, slot(&chart, direction, k, scale), is_orange).is_some(),
                "{scale}x slot {k}"
            );
        }
        // The arrowhead of link 1 sits at row 1's start, inside the gap
        // left of nothing: orange within its head length of x = 70.
        let r1 = slot(&chart, direction, 1, scale);
        let head = bbox(
            &img,
            (
                (x70 - 12.0 * scale).max(0.0) as u32,
                r1.1,
                (x70 + 14.0 * scale) as u32,
                r1.3,
            ),
            is_orange,
        );
        assert!(head.is_some(), "{scale}x: link 1 reaches row 1's start");

        // A link pixel picks its link; the highlight is drawn over it.
        r.enable_gpu_picking().unwrap();
        let id = r
            .register_chart(chart.config().clone(), series.to_vec())
            .unwrap();
        let mut picked = None;
        'scan: for y in ink.1..=ink.3 {
            for x in ink.0..=ink.2 {
                if is_orange(pixel(&img, x, y)) {
                    let request = renderer::GpuPickRequest {
                        canvas_position_px: [x as f32 + 0.5, y as f32 + 0.5],
                        display_panel_px: Rect {
                            x: 0,
                            y: 0,
                            width: img.width,
                            height: img.height,
                        },
                        display_scale: scale,
                        max_distance_px: 0.0,
                    };
                    if let Some(renderer::PickedData::HistogramBin {
                        series_id,
                        bin_index,
                        ..
                    }) = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve())
                        .unwrap()
                    {
                        if series_id == "links" {
                            picked = Some(bin_index);
                            break 'scan;
                        }
                    }
                }
            }
        }
        r.remove_chart(id).unwrap();
        let picked = picked.expect("a link pixel picks its link");
        assert!(picked < 2, "{scale}x picked {picked}");
        chart.config_mut().picked_data = Some(selection("links", picked));
        let outlined = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        chart.config_mut().picked_data = None;
        assert!(
            bbox(&outlined, whole, is_magenta).is_some(),
            "{scale}x: the picked link is highlighted"
        );
    }

    // step_selection walks the link rows, bounded by the shorter column.
    let mut config = chart.config().clone();
    config.picked_data = Some(selection("links", 0));
    let len = |c: &str| match c {
        "link-from" | "link-to" => Some(3),
        _ => Some(3),
    };
    assert_eq!(config.step_selection(&series, 1, 0, len), Ok(true));
    match &config.picked_data.as_ref().unwrap().refs[0] {
        PickedDataRef::HistogramBin {
            series_id,
            bin_index,
            ..
        } => {
            assert_eq!((series_id.as_str(), *bin_index), ("links", 1));
        }
        other => panic!("{other:?}"),
    }

    // The vector exports embed the data image; a saved document draws the same.
    let svg = r
        .export_svg_blocking(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .unwrap();
    assert!(svg.contains("<image"));
    let pdf = r
        .export_pdf_blocking(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    #[cfg(feature = "serde")]
    {
        let draw = |r: &mut Renderer, chart: &Chart, series: &[SeriesConfig]| {
            r.export_rgba(
                renderer::ExportSource::chart(chart, series),
                renderer::ExportOptions::panel(1.0),
            )
            .block_on()
            .unwrap()
            .rgba
        };
        let before = draw(&mut r, &chart, &series);
        let json = serde_json::to_string(&series).unwrap();
        assert!(json.contains("RangeLinks"));
        let again: Vec<SeriesConfig> = serde_json::from_str(&json).unwrap();
        assert_eq!(again.as_slice(), &series);
        assert_eq!(
            draw(&mut r, &Chart::new(chart.config().clone()), &again),
            before
        );
    }
}

/// Links draw next to their bars in an overlay and in a subplot, at 1x
/// and 2x, and pick as their own series there (vertical bars: an overlay
/// takes no horizontal range bars).
#[test]
fn range_links_draw_on_overlays_and_subplots() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut parent = chart(direction);
    let mut overlay = renderer::OverlayLayer::default();
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 100.0;
    overlay.y_axis.major_spacing = 20.0;
    overlay.series = vec!["over".into(), "over-links".into()];
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
        series: vec!["below".into(), "below-links".into()],
        link_x: None,
        link_y: None,
        marker: None,
        text_scale: Default::default(),
    });
    let links = links_series(&mut r);
    let named = |id: &str, bars: &str| {
        let mut l = links.clone();
        l.series_id = id.into();
        if let DataRenderType::RangeLinks(fields) = &mut l.render_type {
            fields.bars = bars.into();
        }
        l
    };
    let series = [
        ranges("over", GREEN),
        named("over-links", "over"),
        ranges("below", GREEN),
        named("below-links", "below"),
    ];
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&parent, &series),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        let (w, h) = (img.width, img.height);
        // Each layer routes over its own bars (the subplot's plot area is
        // its own, so the routes need not line up).
        assert!(
            bbox(&img, (0, 0, w, h / 2), is_orange).is_some(),
            "{scale}x: the overlay's links"
        );
        assert!(
            bbox(&img, (0, h / 2, w, h), is_orange).is_some(),
            "{scale}x: the subplot's links"
        );
        r.enable_gpu_picking().unwrap();
        let id = r
            .register_chart(parent.config().clone(), series.to_vec())
            .unwrap();
        let mut found = Vec::new();
        for (region, layer, want) in [
            ((0, h / 2), renderer::LayerRef::Overlay(0), "over-links"),
            ((h / 2, h), renderer::LayerRef::Subplot(0), "below-links"),
        ] {
            r.set_active_layer(id, layer).unwrap();
            'scan: for y in region.0..region.1 {
                for x in 0..w {
                    if !is_orange(pixel(&img, x, y)) {
                        continue;
                    }
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
                    if let Some(renderer::PickedData::HistogramBin { series_id, .. }) =
                        test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve())
                            .unwrap()
                    {
                        if series_id == want {
                            found.push(want);
                            break 'scan;
                        }
                    }
                }
            }
        }
        r.remove_chart(id).unwrap();
        assert_eq!(found, ["over-links", "below-links"], "{scale}x");
    }
}

/// Rule 10: 3000 links over 3000 rows all route and draw; the last one,
/// past any former cap, picks.
#[test]
fn three_thousand_links_draw_and_pick() {
    let mut r = renderer();
    let n = 3000usize;
    r.add_column("lk-pos", &col((0..n).map(|i| i as f64).collect()))
        .unwrap();
    r.add_column(
        "lk-start",
        &col((0..n).map(|i| (i % 10) as f64 * 9.0).collect()),
    )
    .unwrap();
    r.add_column(
        "lk-end",
        &col((0..n).map(|i| (i % 10) as f64 * 9.0 + 6.0).collect()),
    )
    .unwrap();
    r.add_column("lk-from", &col((0..n - 1).map(|i| i as f64).collect()))
        .unwrap();
    r.add_column("lk-to", &col((1..n).map(|i| i as f64).collect()))
        .unwrap();
    let mut bars = ranges("lk-bars", GREEN);
    bars.x_column = "lk-pos".into();
    bars.y_column = "lk-end".into();
    if let DataRenderType::RangeBars(b) = &mut bars.render_type {
        b.start = "lk-start".into();
        b.slot_width = Some(1.0);
    }
    let mut links = SeriesConfig::range_links("lk", "lk-bars", "lk-from", "lk-to");
    if let DataRenderType::RangeLinks(l) = &mut links.render_type {
        l.color = ORANGE;
        l.width = 3.0;
    }
    let mut chart = chart(CategoryBarDirection::Horizontal);
    let config = chart.config_mut();
    config.left_y.categories = None;
    config.left_y.min = n as f64 - 4.5;
    config.left_y.max = n as f64 - 0.5;
    config.left_y.major_spacing = 1.0;
    let series = [bars, links];
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    assert_eq!(
        r.series_draw_info(id, "lk").unwrap().drawn_count,
        (n - 1) as u64
    );
    r.remove_chart(id).unwrap();
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    assert!(
        bbox(&img, (0, 0, img.width, img.height), is_orange).is_some(),
        "the last links draw"
    );
}
