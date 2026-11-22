//! Parallel coordinates (`Config.parallel`, `DataRenderType::Parallel`,
//! #141) through the public renderer API: the headless export, the row
//! pick, the picked row's highlight, `step_selection`, the colour column,
//! the fit from column statistics, the axes' tick labels (hit and dragged)
//! and the registration rules.
//!
//! House pattern (mirrors `ternary_render.rs`): one process-wide device, a
//! renderer per test, and assertions that name the pixel a value lands on
//! through the axis's own range (`ParallelAxis::fraction`) and the
//! categorical axis's positions, so a wrong axis, a wrong normalization or
//! a row drawn through the wrong column fails rather than passes.

use test_gpu::BlockOn as _;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;
use harness::{
    BLUE, DARK_GREEN as GREEN, RED, col, pixel_at as pixel, renderer_with, save_png as save, to8,
};

use renderer::config::{AxisScale, Category, DataSelectionsConfig, PickedDataRef, TickVisibility};
use renderer::layout::{ChartArea, Element, NudgeResult, Rect, Side};
use renderer::parallel::{ParallelAxis, ParallelOptions};
use renderer::{
    Chart, Color, ColorMap, Config, CpuTextMeasure, DataLineStyleConfig, DataRenderType,
    FiggyError, GpuPickRequest, HitMap, ParallelSeries, PickedData, RasterImage, Renderer,
    SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;

/// The four axes, in order, and their ranges.
const AXES: usize = 4;
const RANGES: [(f64, f64); AXES] = [(0.0, 10.0), (0.0, 100.0), (1.0, 1000.0), (-5.0, 5.0)];

/// Three rows: one value per axis each.
const ROWS: [[f64; AXES]; 3] = [
    [2.0, 20.0, 10.0, -2.5],
    [5.0, 75.0, 100.0, 0.0],
    [8.0, 40.0, 500.0, 4.0],
];

const COLOURS: [Color; 3] = [RED, GREEN, BLUE];

/// The highlight is orange, a colour no row has.
const HIGHLIGHT: Color = Color {
    r: 1.0,
    g: 0.5,
    b: 0.0,
    a: 1.0,
};

fn renderer() -> Renderer {
    renderer_with(64 * 1024 * 1024)
}

/// The column ids of row set `prefix`, one per axis.
fn ids(prefix: &str) -> Vec<String> {
    (0..AXES).map(|i| format!("{prefix}_{i}")).collect()
}

/// Uploads `rows` as one column per axis under `prefix`.
fn upload(r: &mut Renderer, prefix: &str, rows: &[[f64; AXES]]) -> Vec<String> {
    let ids = ids(prefix);
    for (i, id) in ids.iter().enumerate() {
        if r.handle_for(id).is_ok() {
            continue;
        }
        r.add_column(id, &col(rows.iter().map(|row| row[i]).collect()))
            .expect("upload");
    }
    ids
}

fn line(color: Color, width: f32) -> DataLineStyleConfig {
    let mut line = DataLineStyleConfig::default();
    line.line_color = color;
    line.line_width = width;
    line
}

/// The parallel series `id` over `columns`.
fn series(id: &str, columns: &[String], color: Color, color_column: Option<&str>) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        z_column: None,
        label: None,
        x_column: columns[0].clone(),
        y_column: columns[1].clone(),
        render_type: DataRenderType::Parallel(ParallelSeries {
            columns: columns.to_vec(),
            line: line(color, 2.0),
            color_column: color_column.map(str::to_owned),
        }),
    }
}

/// A parallel chart over the four axes with no grid, legend, axis lines or
/// labels, so the only ink is the rows.
fn bare_config() -> Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.colorbar = None;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.label_style.visible = false;
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.title_option.visible = false;
    }
    config.bottom_x.categories = Some(
        ["a", "b", "c", "d"]
            .iter()
            .map(|id| Category::new(*id, id.to_uppercase()))
            .collect(),
    );
    config.left_y.min = 0.0;
    config.left_y.max = 1.0;
    config.left_y.major_spacing = 0.25;
    config.set_parallel(Some(ParallelOptions {
        names_on_top: false,
        label_backdrop: None,
        axes: RANGES
            .iter()
            .map(|(min, max)| ParallelAxis::new(*min, *max))
            .collect(),
    }));
    config
}

/// `bare_config` with each parallel axis's line, ticks and labels drawn.
fn dressed_config() -> Config {
    let mut config = bare_config();
    config.left_y.label_style.visible = true;
    config.left_y.label_style.label_visible = true;
    config.left_y.line_visible = true;
    config.left_y.tick = TickVisibility::Outside;
    config
}

/// Where axis `i` at `value` lands in the export at `scale`: the category's
/// position along x, the value's fraction of the axis's range along y.
fn point_px(config: &Config, i: usize, value: f64, scale: f64) -> (f64, f64) {
    let da = config.data_area().expect("data area");
    let axis = &config.parallel().expect("parallel").axes[i];
    let tx = (i as f64 + 0.5) / AXES as f64;
    let ty = axis.fraction(value).expect("a placeable value");
    (
        (f64::from(da.x) + tx * f64::from(da.width)) * scale,
        (f64::from(da.y + da.height) - ty * f64::from(da.height)) * scale,
    )
}

/// The midpoint of row `row`'s segment between axes `i` and `i + 1`.
fn midpoint_px(config: &Config, row: &[f64; AXES], i: usize, scale: f64) -> (f64, f64) {
    let a = point_px(config, i, row[i], scale);
    let b = point_px(config, i + 1, row[i + 1], scale);
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// The pixel is `want`'s colour within the rounding steps, and the stroke
/// covers it: a steep 2 px stroke covers the pixel its centre line crosses
/// at least by a third, so the alpha is well above the background's.
fn assert_colour_at(img: &RasterImage, at: (f64, f64), want: Color, what: &str) {
    let got = pixel(img, at).map(i32::from);
    let expected = [to8(want.r), to8(want.g), to8(want.b)];
    assert!(
        got.iter().zip(expected).all(|(g, e)| (*g - e).abs() <= 8) && got[3] >= 100,
        "{what} at px {at:?}: got {got:?}, expected {expected:?} with alpha >= 100"
    );
}

fn is_background(p: [u8; 4], background: [u8; 4]) -> bool {
    p == background
}

fn request(at: (f64, f64)) -> GpuPickRequest {
    GpuPickRequest {
        canvas_position_px: [at.0 as f32, at.1 as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT,
        },
        display_scale: 1.0,
        max_distance_px: 3.0,
    }
}

fn row_of(picked: Option<PickedData>) -> Option<(String, usize)> {
    match picked {
        Some(PickedData::Point {
            series_id,
            point_index,
            ..
        }) => Some((series_id, point_index)),
        None => None,
        other => panic!("picked {other:?}"),
    }
}

/// Three rows as three one-row series: each row passes through its value
/// on every axis and through each segment's midpoint in its own colour, at
/// 1x and 2x, and nothing is painted between the rows.
#[test]
fn each_row_passes_through_its_axis_points_and_midpoints_at_1x_and_2x() {
    let mut r = renderer();
    let config = bare_config();
    let series: Vec<SeriesConfig> = ROWS
        .iter()
        .enumerate()
        .map(|(k, row)| {
            let columns = upload(&mut r, &format!("row{k}"), std::slice::from_ref(row));
            series(&format!("row{k}"), &columns, COLOURS[k], None)
        })
        .collect();
    let chart = Chart::new(config.clone());
    for scale in [1.0f64, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale as f32),
            )
            .block_on()
            .unwrap();
        save(&format!("parallel-rows-{scale}x"), &img);
        for (k, row) in ROWS.iter().enumerate() {
            for i in 0..AXES {
                // Two percent along the segment leaving the axis (arriving
                // at the last), so a steep segment's stroke still covers
                // the probe.
                let (x, y) = point_px(&config, i, row[i], scale);
                let (nx, ny) = if i + 1 < AXES {
                    point_px(&config, i + 1, row[i + 1], scale)
                } else {
                    point_px(&config, i - 1, row[i - 1], scale)
                };
                let near = (x + (nx - x) * 0.02, y + (ny - y) * 0.02);
                assert_colour_at(
                    &img,
                    near,
                    COLOURS[k],
                    &format!("row {k} at axis {i} at {scale}x"),
                );
            }
            for i in 0..AXES - 1 {
                assert_colour_at(
                    &img,
                    midpoint_px(&config, row, i, scale),
                    COLOURS[k],
                    &format!("row {k} between axes {i} and {} at {scale}x", i + 1),
                );
            }
        }
        // Between the first and second rows, a quarter of the way from the
        // first axis: no row passes there.
        let background = pixel(&img, (2.0 * scale, 2.0 * scale));
        let (x0, y0) = point_px(&config, 0, 3.5, scale);
        let (x1, y1) = point_px(&config, 1, 47.0, scale);
        let between = (x0 + (x1 - x0) * 0.25, y0 + (y1 - y0) * 0.25);
        assert!(
            is_background(pixel(&img, between), background),
            "between the rows at {scale}x: {:?}",
            pixel(&img, between)
        );
    }
}

/// An inverted axis places its values top down and a logarithmic axis by
/// decades: the test's third axis is `1..1000` in `log10`, so 10 sits a
/// third of the way up, and inverting the second axis puts 75 a quarter
/// of the way up.
#[test]
fn inverted_and_logarithmic_axes_place_their_values() {
    let mut r = renderer();
    let mut config = bare_config();
    {
        let axes = &mut config.parallel_mut().unwrap().axes;
        axes[1].inverted = true;
        axes[2].scale = AxisScale::Logarithmic;
    }
    let columns = upload(&mut r, "rows", &ROWS);
    let series = [series("rows", &columns, RED, None)];
    let chart = Chart::new(config.clone());
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    save("parallel-inverted-log", &img);
    let da = config.data_area().unwrap();
    let frac = |(_, y): (f64, f64)| (f64::from(da.y + da.height) - y) / f64::from(da.height);
    // The model's fraction and the pixel the renderer paints agree.
    let p75 = point_px(&config, 1, 75.0, 1.0);
    assert!(
        (frac(p75) - 0.25).abs() < 1e-9,
        "inverted 75: {}",
        frac(p75)
    );
    let p10 = point_px(&config, 2, 10.0, 1.0);
    assert!(
        (frac(p10) - 1.0 / 3.0).abs() < 1e-9,
        "log 10: {}",
        frac(p10)
    );
    for row in &ROWS {
        for i in 0..AXES - 1 {
            assert_colour_at(
                &img,
                midpoint_px(&config, row, i, 1.0),
                RED,
                &format!("row {row:?} between axes {i} and {}", i + 1),
            );
        }
    }
    // Where the un-inverted, linear placement would have put the second
    // row's second and third values, nothing is drawn.
    let linear = bare_config();
    let background = pixel(&img, (2.0, 2.0));
    for i in [1usize, 2] {
        let (x, y) = point_px(&linear, i, ROWS[1][i], 1.0);
        let at = (if i == 1 { x - 1.0 } else { x + 1.0 }, y);
        assert!(
            is_background(pixel(&img, at), background),
            "the linear placement at axis {i}: {:?}",
            pixel(&img, at)
        );
    }
}

/// A row with a non-finite value breaks at that axis: the two segments that
/// touch it are not drawn, the others are.
#[test]
fn a_non_finite_value_breaks_the_row_at_its_axis() {
    let mut r = renderer();
    let config = bare_config();
    let mut rows = ROWS;
    rows[0][2] = f64::NAN;
    let columns = upload(&mut r, "nan", &rows);
    let series = [series("nan", &columns, RED, None)];
    let chart = Chart::new(config.clone());
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    save("parallel-broken", &img);
    let background = pixel(&img, (2.0, 2.0));
    // The first segment of the broken row is drawn.
    assert_colour_at(
        &img,
        midpoint_px(&config, &ROWS[0], 0, 1.0),
        RED,
        "the broken row's first segment",
    );
    // The segments touching the missing value are not, where they would be.
    for i in [1usize, 2] {
        let at = midpoint_px(&config, &ROWS[0], i, 1.0);
        assert!(
            is_background(pixel(&img, at), background),
            "the broken row's segment {i}: {:?}",
            pixel(&img, at)
        );
    }
    // The other rows are whole.
    for row in &rows[1..] {
        for i in 0..AXES - 1 {
            assert_colour_at(&img, midpoint_px(&config, row, i, 1.0), RED, "a whole row");
        }
    }
}

/// A registered chart whose root is Cartesian and whose subplot is a
/// parallel chart (#325): the series is held to the subplot's axes, not
/// the root's, and the registered chart draws and picks its rows there.
#[test]
fn a_registered_parallel_subplot_beside_a_cartesian_root_draws_and_picks() {
    let mut r = renderer();
    let columns = upload(&mut r, "rows", &ROWS);
    let mut root = renderer::default::default_config();
    root.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT / 2,
    });
    root.legend.visible = false;
    root.colorbar = None;
    let mut cell = bare_config();
    cell.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT / 2,
        width: WIDTH,
        height: HEIGHT / 2,
    });
    root.subplots.push(renderer::Subplot {
        config: cell.clone(),
        series: vec!["rows".into()],
        ..renderer::Subplot::default()
    });
    assert!(root.parallel().is_none(), "the root stays rectangular");
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(root, vec![series("rows", &columns, RED, None)])
        .expect("validated against the parallel subplot");
    let img = r
        .export_rgba(
            renderer::ExportSource::Registered(id),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .unwrap();
    for (k, row) in ROWS.iter().enumerate() {
        let mid = midpoint_px(&cell, row, 0, 1.0);
        assert!(mid.1 > f64::from(HEIGHT / 2), "in the lower cell");
        assert_colour_at(&img, mid, RED, &format!("row {k}"));
        assert_eq!(
            row_of(
                test_gpu::block_on(r.pick_chart_data(id, request(mid)).unwrap().resolve()).unwrap()
            ),
            Some(("rows".to_owned(), k)),
            "row {k}"
        );
    }
    r.remove_chart(id).unwrap();
}

/// The GPU pick answers the row whose segment is under the cursor, within
/// the distance, and nothing between rows; the picked row is redrawn in the
/// highlight colour along every segment, the other rows as they were.
#[test]
fn the_pick_answers_the_row_and_the_highlight_redraws_it() {
    let mut r = renderer();
    let config = bare_config();
    let columns = upload(&mut r, "rows", &ROWS);
    let series = vec![series("rows", &columns, RED, None)];
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    let mut pick = |at: (f64, f64)| {
        row_of(test_gpu::block_on(r.pick_chart_data(id, request(at)).unwrap().resolve()).unwrap())
    };
    for (k, row) in ROWS.iter().enumerate() {
        for i in 0..AXES - 1 {
            assert_eq!(
                pick(midpoint_px(&config, row, i, 1.0)),
                Some(("rows".to_owned(), k)),
                "row {k}, segment {i}"
            );
        }
    }
    let (x0, y0) = point_px(&config, 0, 3.5, 1.0);
    let (x1, y1) = point_px(&config, 1, 47.0, 1.0);
    assert_eq!(
        pick((x0 + (x1 - x0) * 0.25, y0 + (y1 - y0) * 0.25)),
        None,
        "between the rows"
    );
    r.remove_chart(id).unwrap();

    let mut highlighted = config.clone();
    let width_px = 3.0f32;
    highlighted.picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "rows".into(),
            point_index: 1,
        }],
        highlight_color: HIGHLIGHT,
        outline_width_px: width_px,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 0.0,
    });
    let chart = Chart::new(highlighted);
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    save("parallel-selected", &img);
    for i in 0..AXES - 1 {
        assert_colour_at(
            &img,
            midpoint_px(&config, &ROWS[1], i, 1.0),
            HIGHLIGHT,
            &format!("the picked row's segment {i}"),
        );
        assert_colour_at(
            &img,
            midpoint_px(&config, &ROWS[0], i, 1.0),
            RED,
            &format!("the first row's segment {i}"),
        );
        assert_colour_at(
            &img,
            midpoint_px(&config, &ROWS[2], i, 1.0),
            RED,
            &format!("the third row's segment {i}"),
        );
    }
}

/// `step_selection` walks the rows of a parallel series, wrapping at the
/// shortest column.
#[test]
fn step_selection_walks_the_rows() {
    let mut config = bare_config();
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "rows".into(),
            point_index: 1,
        }],
        ..DataSelectionsConfig::default()
    });
    let columns = ids("rows");
    let series = [series("rows", &columns, RED, None)];
    let len = |id: &str| (id == "rows_2").then_some(3).or(Some(5));
    assert_eq!(config.step_selection(&series, 1, 0, len), Ok(true));
    let row = |config: &Config| match &config.picked_data.as_ref().unwrap().refs[0] {
        PickedDataRef::Point { point_index, .. } => *point_index,
        other => panic!("{other:?}"),
    };
    assert_eq!(row(&config), 2);
    assert_eq!(
        config.step_selection(&series, 1, 0, len),
        Ok(false),
        "stops at the shortest column's last row"
    );
    assert_eq!(row(&config), 2);
    assert_eq!(config.step_selection(&series, -1, 0, len), Ok(true));
    assert_eq!(row(&config), 1);
}

/// With a colour column, each row takes its value's colour through the
/// colourbar: the rows at the range's ends take the colormap's end stops,
/// the middle row a colour of its own.
#[test]
fn a_colour_column_colours_each_row_through_the_colorbar() {
    let mut r = renderer();
    let mut config = bare_config();
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Viridis;
    bar.axis.min = 0.0;
    bar.axis.max = 2.0;
    bar.visible = false;
    config.colorbar = Some(bar);
    let columns = upload(&mut r, "rows", &ROWS);
    r.add_column("rows_class", &col(vec![0.0, 1.0, 2.0]))
        .unwrap();
    let series = [series("rows", &columns, RED, Some("rows_class"))];
    let chart = Chart::new(config.clone());
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    save("parallel-coloured", &img);
    let stops = ColorMap::Viridis.stops();
    let first = stops[0];
    let last = stops[stops.len() - 1];
    for i in 0..AXES - 1 {
        assert_colour_at(
            &img,
            midpoint_px(&config, &ROWS[0], i, 1.0),
            first,
            "the row at the colourbar's minimum",
        );
        assert_colour_at(
            &img,
            midpoint_px(&config, &ROWS[2], i, 1.0),
            last,
            "the row at the colourbar's maximum",
        );
    }
    let middle = pixel(&img, midpoint_px(&config, &ROWS[1], 0, 1.0));
    let distance = |c: Color| {
        (i32::from(middle[0]) - to8(c.r)).abs()
            + (i32::from(middle[1]) - to8(c.g)).abs()
            + (i32::from(middle[2]) - to8(c.b)).abs()
    };
    assert!(
        distance(first) > 60 && distance(last) > 60 && distance(RED) > 60,
        "the middle row has the ramp's middle colour: {middle:?}"
    );
}

/// `Chart::auto_fit_parallel_axes` writes each axis's range from the
/// columns' statistics, padded; a logarithmic axis from the smallest
/// positive value.
#[test]
fn the_fit_takes_each_axis_range_from_its_column_statistics() {
    let mut r = renderer();
    let mut config = bare_config();
    config.parallel_mut().unwrap().axes[2].scale = AxisScale::Logarithmic;
    let columns = upload(&mut r, "rows", &ROWS);
    let series = [series("rows", &columns, RED, None)];
    let mut chart = Chart::new(config);
    chart
        .auto_fit_parallel_axes(r.pool(), &series, 0.0)
        .unwrap();
    let axes = &chart.config().parallel().unwrap().axes;
    assert_eq!((axes[0].min, axes[0].max), (2.0, 8.0));
    assert_eq!((axes[1].min, axes[1].max), (20.0, 75.0));
    assert!(
        (axes[2].min - 10.0).abs() < 1e-9 && (axes[2].max - 500.0).abs() < 1e-9,
        "the log axis: {:?}",
        (axes[2].min, axes[2].max)
    );
    assert_eq!((axes[3].min, axes[3].max), (-2.5, 4.0));
    // The chart's own axes are left alone.
    assert_eq!(
        (chart.config().left_y.min, chart.config().left_y.max),
        (0.0, 1.0)
    );
    let mut padded = Chart::new(bare_config());
    padded
        .auto_fit_parallel_axes(r.pool(), &series, 0.1)
        .unwrap();
    let axis = &padded.config().parallel().unwrap().axes[0];
    assert!((axis.min - 1.4).abs() < 1e-9 && (axis.max - 8.6).abs() < 1e-9);
    // The layer fit of a parallel chart fits its axes too.
    let mut layered = Chart::new(bare_config());
    layered
        .auto_fit_layer_union(r.pool(), renderer::LayerRef::Base, &series, 0.0)
        .unwrap();
    assert_eq!(layered.config().parallel().unwrap().axes[3].max, 4.0);
    assert_eq!(
        layered.config().bottom_x.categories.as_ref().map(Vec::len),
        Some(AXES)
    );
}

/// Each axis's tick labels are drawn beside its line, hit as their own
/// element and dragged into the axis's `label_offset`; the chart's left
/// axis draws nothing.
#[test]
fn each_axis_has_its_own_tick_labels_hit_and_dragged() {
    let mut r = renderer();
    let config = dressed_config();
    let columns = upload(&mut r, "rows", &ROWS);
    let series = [series("rows", &columns, RED, None)];
    let chart = Chart::new(config.clone());
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    save("parallel-dressed", &img);
    let measure = CpuTextMeasure::for_style(&config.draw_style);
    let map = HitMap::for_active_layer(&config);
    let hit = |x: f32, y: f32| {
        map.hit_test(&config, &measure, x, y)
            .and_then(|id| map.get(id))
            .map(|e| e.element_id())
    };
    let background = pixel(&img, (2.0, 2.0));
    let da = config.data_area().unwrap();
    for i in 0..AXES {
        // Some tick label of the axis sits left of its line.
        let (min, max) = RANGES[i];
        let (x, y) = point_px(&config, i, (min + max) * 0.5, 1.0);
        let found = (0..da.height)
            .step_by(3)
            .flat_map(|dy| (2..60).map(move |d| (d, dy)))
            .map(|(d, dy)| (x - f64::from(d), f64::from(da.y + dy) + 0.5))
            .find(|at| {
                hit(at.0 as f32, at.1 as f32).as_deref()
                    == Some(&format!("parallel_axis_labels_{i}"))
            });
        let at = found.unwrap_or_else(|| panic!("axis {i}: no label hit left of its line"));
        // Ink of the label near the hit.
        let inked = (0..14).any(|dx| {
            (0..10).any(|dy| {
                !is_background(
                    pixel(&img, (at.0 - f64::from(dx), at.1 - 5.0 + f64::from(dy))),
                    background,
                )
            })
        });
        assert!(inked, "axis {i}: its label is drawn");
        // The axis line itself.
        assert!(
            !is_background(pixel(&img, (x, y - 20.0)), background),
            "axis {i}: its line is drawn"
        );
    }
    // The chart's own left axis draws no labels: left of the data area is
    // background.
    for dy in 0..da.height {
        let at = (f64::from(da.x) - 6.0, f64::from(da.y + dy) + 0.5);
        assert!(
            is_background(pixel(&img, at), background),
            "the left axis band is empty at {at:?}: {:?}",
            pixel(&img, at)
        );
    }
    // A drag writes the axis's label offset and nothing else.
    let mut moved = config.clone();
    assert_eq!(
        moved.nudge(Element::ParallelAxisLabel(2), &measure, -4.0, 6.0),
        NudgeResult::Moved
    );
    let axes = &moved.parallel().unwrap().axes;
    assert_eq!(axes[2].label_offset, [-4.0, 6.0]);
    assert_eq!(axes[1].label_offset, [0.0, 0.0]);
    assert_eq!(moved.left_y.label_style.label_offset_x, 0.0);
    // The moved labels are hit where they moved to, and no longer where
    // they were: the hit map's answers shift by the drag.
    let map_moved = HitMap::for_active_layer(&moved);
    let hit_moved = |x: f32, y: f32| {
        map_moved
            .hit_test(&moved, &measure, x, y)
            .and_then(|id| map_moved.get(id))
            .map(|e| e.element_id())
    };
    let (x, _) = point_px(&config, 2, 500.0, 1.0);
    let before: Vec<(f64, f64)> = (0..da.height)
        .step_by(3)
        .flat_map(|dy| (2..60).map(move |d| (d, dy)))
        .map(|(d, dy)| (x - f64::from(d), f64::from(da.y + dy) + 0.5))
        .filter(|at| hit(at.0 as f32, at.1 as f32).as_deref() == Some("parallel_axis_labels_2"))
        .collect();
    assert!(!before.is_empty());
    let shifted = before
        .iter()
        .filter(|at| {
            hit_moved((at.0 - 4.0) as f32, (at.1 + 6.0) as f32).as_deref()
                == Some("parallel_axis_labels_2")
        })
        .count();
    assert!(
        shifted * 10 >= before.len() * 9,
        "the moved labels are hit at their new place: {shifted} of {}",
        before.len()
    );
    // The categorical bottom axis keeps its names.
    let mut named = config.clone();
    named.bottom_x.label_style.visible = true;
    named.bottom_x.label_style.label_visible = true;
    let names = Chart::new(named.clone());
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&names, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    let (x, _) = point_px(&named, 1, 50.0, 1.0);
    let below = (0..30).any(|dy| {
        !is_background(
            pixel(&img, (x, f64::from(da.y + da.height) + 4.0 + f64::from(dy))),
            background,
        )
    });
    assert!(below, "the category name is drawn under its axis");
    let _ = Side::Left;
}

/// The axis names above the plot (`names_on_top`) and the tick labels'
/// backdrop boxes (`label_backdrop`, #357) draw at 1x and 2x, and are hit
/// and dragged where they are drawn: the names as `bottom_x`'s labels, each
/// box with its axis's labels.
#[test]
fn names_on_top_and_label_backdrops_draw_hit_and_drag() {
    let mut r = renderer();
    let mut config = dressed_config();
    config.bottom_x.label_style.visible = true;
    config.bottom_x.label_style.label_visible = true;
    // A backdrop no row or label is drawn in.
    let backdrop = Color::from_rgb8(250, 230, 120);
    let options = config.parallel_mut().unwrap();
    options.names_on_top = true;
    options.label_backdrop = Some(backdrop);
    // The names need a top band: the chart is created with its layout
    // fitted to them (#357 item 7), as for a chart made from the preset.
    config.layout_state = renderer::LayoutState::Fitted;
    let columns = upload(&mut r, "rows", &ROWS);
    let series = [series("rows", &columns, RED, None)];
    let chart = Chart::new(config);
    let want = [250u8, 230, 120];
    let mut boxed = [0usize; 2];
    let mut fitted: Option<Config> = None;
    for (k, scale) in [1.0f32, 2.0].into_iter().enumerate() {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        save(&format!("parallel-names-on-top-{scale}x"), &img);
        // The chart holds its fit; neither export changes it.
        let config = chart.config().clone();
        assert_eq!(config.layout_state, renderer::LayoutState::Fitted);
        if let Some(first) = &fitted {
            assert_eq!(first, &config, "the 2x export does not refit");
        }
        fitted = Some(config.clone());
        let da = config.data_area().unwrap();
        boxed[k] = img.rgba.chunks_exact(4).filter(|p| p[..3] == want).count();
        let background = pixel(&img, (2.0, 2.0));
        // A name is inked above the data area over axis 1, none below it.
        let (x, _) = point_px(&config, 1, 50.0, f64::from(scale));
        let s = f64::from(scale);
        let top = f64::from(da.y) * s;
        let bottom = f64::from(da.y + da.height) * s;
        let above = (2..(30.0 * s) as i32)
            .any(|dy| !is_background(pixel(&img, (x, top - f64::from(dy))), background));
        assert!(above, "{scale}x: the name is drawn above its axis");
        let below = (4..(30.0 * s) as i32)
            .any(|dy| !is_background(pixel(&img, (x, bottom + f64::from(dy))), background));
        assert!(!below, "{scale}x: nothing is drawn under the axes");
    }
    assert!(boxed[0] > 100, "the backdrops are drawn: {boxed:?}");
    // The 2x boxes cover about four times the 1x pixels.
    let ratio = boxed[1] as f64 / boxed[0] as f64;
    assert!((3.0..5.0).contains(&ratio), "{boxed:?}");

    let config = fitted.unwrap();
    let da = config.data_area().unwrap();
    let measure = CpuTextMeasure::for_style(&config.draw_style);
    let map = HitMap::for_active_layer(&config);
    let hit = |config: &Config, map: &HitMap, x: f32, y: f32| {
        map.hit_test(config, &measure, x, y)
            .and_then(|id| map.get(id))
            .map(|e| e.element_id())
    };
    // The names are hit above the data area, as the bottom axis's labels.
    let (x, _) = point_px(&config, 1, 50.0, 1.0);
    let name_at = (1..30)
        .map(|dy| (x as f32, da.y as f32 - dy as f32))
        .find(|at| hit(&config, &map, at.0, at.1).as_deref() == Some("tick_labels_bottom"))
        .expect("a name is hit above the plot");
    // A drag moves them by the stored offset, and they are hit there.
    let mut moved = config.clone();
    assert_eq!(
        moved.nudge(Element::AxisLabel(Side::Bottom), &measure, 0.0, -5.0),
        NudgeResult::Moved
    );
    assert_eq!(moved.bottom_x.label_style.label_offset_y, -5.0);
    let moved_map = HitMap::for_active_layer(&moved);
    assert_eq!(
        hit(&moved, &moved_map, name_at.0, name_at.1 - 5.0).as_deref(),
        Some("tick_labels_bottom")
    );
    // A backdrop box is hit as its axis's labels: some point just outside
    // a label's text, inside its box, answers the axis.
    let (x, _) = point_px(&config, 2, 500.0, 1.0);
    let top = da.y;
    let found = (0..da.height)
        .flat_map(|dy| (2..60).map(move |d| (x - f64::from(d), f64::from(top + dy) + 0.5)))
        .any(|at| {
            hit(&config, &map, at.0 as f32, at.1 as f32).as_deref()
                == Some("parallel_axis_labels_2")
        });
    assert!(found, "axis 2's labels are hit");
}

/// Registration names each rule: the axis count, a series that is not
/// parallel on a parallel chart, a parallel series on a plain chart, a
/// colour column without a colourbar, a polar chart, a 3D chart and a
/// streamed source.
#[test]
fn registration_names_what_a_parallel_chart_cannot_read() {
    let mut r = renderer();
    let config = bare_config();
    let columns = upload(&mut r, "rows", &ROWS);
    let reason = |r: &mut Renderer, config: &Config, series: Vec<SeriesConfig>| {
        let what = format!("{:?} / {:?}", config.parallel(), series);
        match r.register_chart(config.clone(), series) {
            Err(FiggyError::InvalidSeriesConfig { kind: reason, .. }) => reason,
            Err(FiggyError::InvalidConfig { kind: reason, .. }) => reason.to_owned(),
            other => panic!("registered {what}: {other:?}"),
        }
    };
    let mut short = series("rows", &columns, RED, None);
    if let DataRenderType::Parallel(ParallelSeries { columns, .. }) = &mut short.render_type {
        columns.pop();
    }
    assert_eq!(
        reason(&mut r, &config, vec![short]),
        renderer::RuleKind::ParallelSeriesColumnCount
    );
    let mut plain_line = series("rows", &columns, RED, None);
    plain_line.render_type = DataRenderType::Xy(renderer::data_config::XySeries {
        scatter: None,
        line: Some(line(RED, 1.0)),
        errors: None,
        stem: None,
    });
    assert_eq!(
        reason(&mut r, &config, vec![plain_line]),
        renderer::RuleKind::ParallelChartSeriesKind
    );
    let mut plain = bare_config();
    plain.set_parallel(None);
    assert_eq!(
        reason(&mut r, &plain, vec![series("rows", &columns, RED, None)]),
        renderer::RuleKind::ParallelSeriesNotParallel
    );
    r.add_column("rows_class", &col(vec![0.0, 1.0, 2.0]))
        .unwrap();
    assert!(matches!(
        reason(
            &mut r,
            &config,
            vec![series("rows", &columns, RED, Some("rows_class"))]
        ),
        renderer::RuleKind::ParallelSeriesColorbar
    ));
    let mut dashed = series("rows", &columns, RED, None);
    if let DataRenderType::Parallel(ParallelSeries { line, .. }) = &mut dashed.render_type {
        line.line_style = renderer::line::LineStylePreset::Dash;
    }
    assert_eq!(
        reason(&mut r, &config, vec![dashed]),
        renderer::RuleKind::ParallelSeriesDashed
    );
    // A chart has one coordinate system (#160): a polar chart is not a
    // parallel one, so a parallel series is refused there.
    let mut polar = bare_config();
    polar.set_polar(Some(renderer::polar::PolarOptions::default()));
    assert_eq!(
        reason(&mut r, &polar, vec![series("rows", &columns, RED, None)]),
        renderer::RuleKind::ParallelSeriesNotParallel
    );
    // The chart's own rules are checked where the chart is drawn (as the
    // polar and ternary systems' are): the export names them.
    let config_reason = |r: &mut Renderer, config: &Config, series: Vec<SeriesConfig>| {
        let what = format!("{:?}", config.parallel());
        match r
            .export_rgba(
                renderer::ExportSource::chart(&Chart::new(config.clone()), &series),
                renderer::ExportOptions::panel(1.0),
            )
            .block_on()
        {
            Err(FiggyError::InvalidConfig {
                field,
                kind: reason,
            }) => {
                assert_eq!(field, "parallel", "{what}");
                reason.to_owned()
            }
            Err(FiggyError::InvalidSeriesConfig { kind: reason, .. }) => reason,
            other => panic!("drew {what}: {:?}", other.map(|_| ())),
        }
    };
    let mut few = bare_config();
    few.parallel_mut().unwrap().axes.pop();
    assert_eq!(
        config_reason(&mut r, &few, vec![series("rows", &columns[..3], RED, None)]),
        renderer::RuleKind::ParallelBottomXCategories
    );
    let mut band = bare_config();
    band.left_y.max = 2.0;
    assert_eq!(
        config_reason(&mut r, &band, vec![series("rows", &columns, RED, None)]),
        renderer::RuleKind::ParallelLeftY
    );
    let mut sketched = bare_config();
    sketched.draw_style = renderer::config::DrawStyle::Sketch(Default::default());
    assert_eq!(
        config_reason(&mut r, &sketched, vec![series("rows", &columns, RED, None)]),
        renderer::RuleKind::ParallelDrawStyle
    );
    // A streamed source is refused before any chunk is requested (rule 8).
    let streamed = |id: &str| renderer::StreamColumn {
        id: id.into(),
        len: 3,
        revision: 1,
        encoding: renderer::StreamEncoding::ScalarF32,
        replay: renderer::StreamReplay::RandomAccess,
        statistics: renderer::StreamStatistics::Unknown,
    };
    r.register_streamed_columns(vec![streamed("st_0"), streamed("st_1")])
        .unwrap();
    let mut over_stream = series("rows", &columns, RED, None);
    over_stream.x_column = "st_0".into();
    over_stream.y_column = "st_1".into();
    if let DataRenderType::Parallel(ParallelSeries { columns, .. }) = &mut over_stream.render_type {
        columns[0] = "st_0".into();
        columns[1] = "st_1".into();
    }
    let id = r
        .register_chart(config.clone(), vec![over_stream])
        .expect("registration admits the columns; the streaming inspector names the rule");
    match r.inspect_streaming_capability(id) {
        Err(FiggyError::InvalidSeriesConfig {
            series_id,
            kind: reason,
        }) => {
            assert_eq!(series_id, "rows");
            assert_eq!(reason, renderer::RuleKind::StreamedParallel);
        }
        other => panic!(
            "expected the streamed rejection, got {:?}",
            other.map(|_| ())
        ),
    }
    r.remove_chart(id).unwrap();
    // The well-formed chart registers.
    let id = r
        .register_chart(config, vec![series("rows", &columns, RED, None)])
        .unwrap();
    r.remove_chart(id).unwrap();
}

/// A parallel chart with fewer than two axes is refused when it is set
/// (registration, `set_chart_config`, `set_chart_state`), not first when it
/// is drawn, and the series checked against it never indexes a missing
/// column (#152): each call returns an error and the stored chart is
/// unchanged.
#[test]
fn a_chart_with_fewer_than_two_axes_is_refused_when_it_is_set() {
    let mut r = renderer();
    let config = bare_config();
    let columns = upload(&mut r, "rows", &ROWS);
    let id = r
        .register_chart(config.clone(), vec![series("rows", &columns, RED, None)])
        .unwrap();
    for count in [0usize, 1] {
        let mut short = bare_config();
        short.parallel_mut().unwrap().axes.truncate(count);
        let mut item = series("rows", &columns, RED, None);
        if let DataRenderType::Parallel(ParallelSeries { columns, .. }) = &mut item.render_type {
            columns.truncate(count);
        }
        let refused = |result: renderer::Result<()>, what: &str| match result {
            Err(FiggyError::InvalidConfig {
                field,
                kind: reason,
            }) => {
                assert_eq!(field, "parallel", "{what}, {count} axes");
                assert_eq!(reason, renderer::RuleKind::ParallelAxisCount);
            }
            other => panic!("{what} with {count} axes: {other:?}"),
        };
        refused(
            r.set_chart_state(id, short.clone(), vec![item.clone()]),
            "set_chart_state",
        );
        refused(r.set_chart_config(id, short.clone()), "set_chart_config");
        refused(
            r.register_chart(short.clone(), vec![item.clone()])
                .map(|_| ()),
            "register_chart",
        );
        // The stored chart keeps its four axes, so the series checked
        // against it is refused by its column count.
        match r.set_chart_series(id, vec![item]) {
            Err(FiggyError::InvalidSeriesConfig { kind: reason, .. }) => {
                assert_eq!(reason, renderer::RuleKind::ParallelSeriesColumnCount);
            }
            other => panic!("set_chart_series with {count} columns: {other:?}"),
        }
        assert_eq!(r.chart_config(id).unwrap().parallel(), config.parallel());
    }
    r.remove_chart(id).unwrap();
}

/// A million rows over eight axes draw and pick: no count limit (rule 10);
/// the times are printed, not asserted.
#[test]
fn a_million_rows_over_eight_axes_draw_and_pick() {
    let mut r = renderer_with(512 * 1024 * 1024);
    let (rows, axes) = (1_000_000usize, 8usize);
    let columns: Vec<String> = (0..axes).map(|i| format!("big_{i}")).collect();
    for (i, id) in columns.iter().enumerate() {
        // Smooth waves per axis, so the rows fan out.
        let data: Vec<f64> = (0..rows)
            .map(|k| {
                let t = k as f64 / rows as f64;
                50.0 + 45.0 * ((t * 6.283 * (i as f64 + 1.0)) + i as f64).sin()
            })
            .collect();
        r.add_column(id, &col(data))
            .expect("eight million values upload");
    }
    let mut config = bare_config();
    config.bottom_x.categories = Some(
        (0..axes)
            .map(|i| Category::new(format!("x{i}"), format!("x{i}")))
            .collect(),
    );
    config.set_parallel(Some(ParallelOptions {
        names_on_top: false,
        label_backdrop: None,
        axes: (0..axes).map(|_| ParallelAxis::new(0.0, 100.0)).collect(),
    }));
    let series = SeriesConfig {
        series_id: "big".into(),
        source_id: None,
        z_column: None,
        label: None,
        x_column: columns[0].clone(),
        y_column: columns[1].clone(),
        render_type: DataRenderType::Parallel(ParallelSeries {
            columns: columns.clone(),
            line: line(RED, 1.0),
            color_column: None,
        }),
    };
    let chart = Chart::new(config.clone());
    let started = std::time::Instant::now();
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &[series.clone()]),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    eprintln!(
        "{rows} rows x {axes} axes exported in {:?}",
        started.elapsed()
    );
    // Row 0 passes through 50 + 45 sin(i) on axis i.
    let da = config.data_area().unwrap();
    let at = (
        f64::from(da.x) + 0.5 / axes as f64 * f64::from(da.width) + 1.0,
        f64::from(da.y + da.height) - 0.5 * f64::from(da.height),
    );
    assert_colour_at(&img, at, RED, "the first axis at 50");
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config, vec![series]).unwrap();
    let started = std::time::Instant::now();
    let picked =
        row_of(test_gpu::block_on(r.pick_chart_data(id, request(at)).unwrap().resolve()).unwrap());
    eprintln!("picked in {:?}", started.elapsed());
    assert!(picked.is_some(), "a row under the cursor");
}

/// A colour-mapped parallel series in a subplot reads the subplot's own
/// colourbar (#401): beside a Cartesian root without one it registers,
/// colours its rows through the subplot's ramp and picks them; a root
/// colourbar alone does not satisfy it (the model's per-chart rule, #325,
/// refuses it before the renderer's).
#[test]
fn a_colour_mapped_parallel_subplot_reads_its_own_colorbar() {
    let mut r = renderer();
    let columns = upload(&mut r, "rows", &ROWS);
    if r.handle_for("rows_class").is_err() {
        r.add_column("rows_class", &col(vec![0.0, 1.0, 2.0]))
            .unwrap();
    }
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Viridis;
    bar.axis.min = 0.0;
    bar.axis.max = 2.0;
    bar.visible = false;
    use renderer::config::ColorBarOptions;
    let document = |root_bar: Option<ColorBarOptions>, cell_bar: Option<ColorBarOptions>| {
        let mut root = renderer::default::default_config();
        root.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT / 2,
        });
        root.legend.visible = false;
        root.colorbar = root_bar;
        let mut cell = bare_config();
        cell.chart_area = ChartArea(Rect {
            x: 0,
            y: HEIGHT / 2,
            width: WIDTH,
            height: HEIGHT / 2,
        });
        cell.colorbar = cell_bar;
        root.subplots.push(renderer::Subplot {
            config: cell.clone(),
            series: vec!["rows".into()],
            ..renderer::Subplot::default()
        });
        (root, cell)
    };
    let coloured = || vec![series("rows", &columns, RED, Some("rows_class"))];

    let (root, cell) = document(None, Some(bar.clone()));
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(root, coloured())
        .expect("the subplot's colourbar colours its rows");
    let img = r
        .export_rgba(
            renderer::ExportSource::Registered(id),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .unwrap();
    let stops = ColorMap::Viridis.stops();
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    let low = midpoint_px(&cell, &ROWS[0], 0, 1.0);
    assert!(low.1 > f64::from(HEIGHT / 2), "in the lower cell");
    assert_colour_at(&img, low, first, "the row at the colourbar's minimum");
    assert_colour_at(
        &img,
        midpoint_px(&cell, &ROWS[2], 0, 1.0),
        last,
        "the row at the colourbar's maximum",
    );
    assert_eq!(
        row_of(test_gpu::block_on(r.pick_chart_data(id, request(low)).unwrap().resolve()).unwrap()),
        Some(("rows".to_owned(), 0))
    );
    r.remove_chart(id).unwrap();

    let (root, _) = document(Some(bar), None);
    let error = r.register_chart(root, coloured()).unwrap_err();
    assert!(
        matches!(
            &error,
            renderer::FiggyError::InvalidSeriesConfig {
                kind: renderer::RuleKind::ParallelSeriesColorbar,
                ..
            }
        ),
        "{error}"
    );
}
