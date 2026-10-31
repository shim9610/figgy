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

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::{AxisScale, Category, DataSelectionsConfig, PickedDataRef, TickVisibility};
use renderer::data::Column;
use renderer::layout::{ChartArea, Element, NudgeResult, Rect, Side};
use renderer::parallel::{ParallelAxis, ParallelOptions};
use renderer::{
    Chart, Color, ColorMap, Config, CpuTextMeasure, DataLineStyleConfig, DataRenderType,
    FiggyError, GpuPickRequest, HitMap, PickedData, RasterImage, Renderer, RendererDevice,
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

const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const GREEN: Color = Color {
    r: 0.0,
    g: 0.6,
    b: 0.0,
    a: 1.0,
};
const BLUE: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};
const COLOURS: [Color; 3] = [RED, GREEN, BLUE];

/// The highlight is orange, a colour no row has.
const HIGHLIGHT: Color = Color {
    r: 1.0,
    g: 0.5,
    b: 0.0,
    a: 1.0,
};

fn col(data: Vec<f64>) -> Column<f64> {
    let finite = || data.iter().copied().filter(|v| v.is_finite());
    let min = finite().fold(f64::INFINITY, f64::min);
    let max = finite().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

fn shared_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    static DEVICE: OnceLock<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(|| {
        let (device, queue) = test_gpu::device();
        (Arc::new(device), Arc::new(queue))
    });
    (Arc::clone(device), Arc::clone(queue))
}

fn renderer_with(budget: u64) -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Bgra8Unorm,
        budget,
    )
    .expect("renderer init")
}

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
        render_type: DataRenderType::Parallel {
            columns: columns.to_vec(),
            line: line(color, 2.0),
            color_column: color_column.map(str::to_owned),
        },
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
    config.parallel = Some(ParallelOptions {
        axes: RANGES
            .iter()
            .map(|(min, max)| ParallelAxis::new(*min, *max))
            .collect(),
    });
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
    let axis = &config.parallel.as_ref().expect("parallel").axes[i];
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

/// Writes the image when `FIGGY_INTERACTION_ARTIFACTS` names a directory.
fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        let png = renderer::encode_png(image).unwrap();
        std::fs::write(format!("{path}/{name}.png"), png).unwrap();
    }
}

fn pixel(img: &RasterImage, (x, y): (f64, f64)) -> [u8; 4] {
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

fn to8(v: f32) -> i32 {
    (v.clamp(0.0, 1.0) * 255.0).round() as i32
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
        let img = r.export_panel_rgba(&chart, &series, scale as f32).unwrap();
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
        let axes = &mut config.parallel.as_mut().unwrap().axes;
        axes[1].inverted = true;
        axes[2].scale = AxisScale::Logarithmic;
    }
    let columns = upload(&mut r, "rows", &ROWS);
    let series = [series("rows", &columns, RED, None)];
    let chart = Chart::new(config.clone());
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
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
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
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
        row_of(pollster::block_on(r.pick_chart_data(id, request(at)).unwrap().resolve()).unwrap())
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
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
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
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
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
    config.parallel.as_mut().unwrap().axes[2].scale = AxisScale::Logarithmic;
    let columns = upload(&mut r, "rows", &ROWS);
    let series = [series("rows", &columns, RED, None)];
    let mut chart = Chart::new(config);
    chart
        .auto_fit_parallel_axes(r.pool(), &series, 0.0)
        .unwrap();
    let axes = &chart.config().parallel.as_ref().unwrap().axes;
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
    let axis = &padded.config().parallel.as_ref().unwrap().axes[0];
    assert!((axis.min - 1.4).abs() < 1e-9 && (axis.max - 8.6).abs() < 1e-9);
    // The layer fit of a parallel chart fits its axes too.
    let mut layered = Chart::new(bare_config());
    layered
        .auto_fit_layer_union(r.pool(), renderer::LayerRef::Base, &series, 0.0)
        .unwrap();
    assert_eq!(layered.config().parallel.as_ref().unwrap().axes[3].max, 4.0);
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
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
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
    let axes = &moved.parallel.as_ref().unwrap().axes;
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
    let img = r.export_panel_rgba(&names, &series, 1.0).unwrap();
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
        let what = format!("{:?} / {:?}", config.parallel, series);
        match r.register_chart(config.clone(), series) {
            Err(FiggyError::InvalidSeriesConfig { reason, .. }) => reason,
            Err(FiggyError::InvalidConfig { reason, .. }) => reason.to_owned(),
            other => panic!("registered {what}: {other:?}"),
        }
    };
    let mut short = series("rows", &columns, RED, None);
    if let DataRenderType::Parallel { columns, .. } = &mut short.render_type {
        columns.pop();
    }
    assert_eq!(
        reason(&mut r, &config, vec![short]),
        "a parallel series names one column per axis"
    );
    let mut plain_line = series("rows", &columns, RED, None);
    plain_line.render_type = DataRenderType::Line {
        line: line(RED, 1.0),
    };
    assert_eq!(
        reason(&mut r, &config, vec![plain_line]),
        "a parallel chart draws only parallel series"
    );
    let mut plain = bare_config();
    plain.parallel = None;
    assert_eq!(
        reason(&mut r, &plain, vec![series("rows", &columns, RED, None)]),
        "a parallel series is drawn on a parallel chart (Config.parallel)"
    );
    r.add_column("rows_class", &col(vec![0.0, 1.0, 2.0]))
        .unwrap();
    assert!(
        reason(
            &mut r,
            &config,
            vec![series("rows", &columns, RED, Some("rows_class"))]
        )
        .starts_with("a parallel series coloured by a column needs Config.colorbar")
    );
    let mut dashed = series("rows", &columns, RED, None);
    if let DataRenderType::Parallel { line, .. } = &mut dashed.render_type {
        line.line_style = renderer::line::LineStylePreset::Dash;
    }
    assert_eq!(
        reason(&mut r, &config, vec![dashed]),
        "a parallel series' line is solid"
    );
    let mut polar = bare_config();
    polar.polar = Some(renderer::polar::PolarOptions::default());
    assert_eq!(
        reason(&mut r, &polar, vec![series("rows", &columns, RED, None)]),
        "parallel series are drawn on parallel charts, not polar axes"
    );
    // The chart's own rules are checked where the chart is drawn (as the
    // polar and ternary systems' are): the export names them.
    let config_reason = |r: &mut Renderer, config: &Config, series: Vec<SeriesConfig>| {
        let what = format!("{:?}", config.parallel);
        match r.export_panel_rgba(&Chart::new(config.clone()), &series, 1.0) {
            Err(FiggyError::InvalidConfig { field, reason }) => {
                assert_eq!(field, "parallel", "{what}");
                reason.to_owned()
            }
            Err(FiggyError::InvalidSeriesConfig { reason, .. }) => reason,
            other => panic!("drew {what}: {:?}", other.map(|_| ())),
        }
    };
    let mut few = bare_config();
    few.parallel.as_mut().unwrap().axes.pop();
    assert_eq!(
        config_reason(&mut r, &few, vec![series("rows", &columns[..3], RED, None)]),
        "a parallel chart's bottom_x has one category per axis"
    );
    let mut band = bare_config();
    band.left_y.max = 2.0;
    assert_eq!(
        config_reason(&mut r, &band, vec![series("rows", &columns, RED, None)]),
        "a parallel chart's left_y spans the normalized band: linear 0..1, not inverted"
    );
    let mut sketched = bare_config();
    sketched.draw_style = renderer::config::DrawStyle::Sketch(Default::default());
    assert_eq!(
        config_reason(&mut r, &sketched, vec![series("rows", &columns, RED, None)]),
        "parallel charts are drawn in the Precise style only"
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
    if let DataRenderType::Parallel { columns, .. } = &mut over_stream.render_type {
        columns[0] = "st_0".into();
        columns[1] = "st_1".into();
    }
    let id = r
        .register_chart(config.clone(), vec![over_stream])
        .expect("registration admits the columns; the streaming inspector names the rule");
    match r.inspect_streaming_capability(id) {
        Err(FiggyError::InvalidSeriesConfig { series_id, reason }) => {
            assert_eq!(series_id, "rows");
            assert_eq!(
                reason,
                "parallel coordinates series require GPU-resident columns; streamed sources are \
                 not supported"
            );
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
    config.parallel = Some(ParallelOptions {
        axes: (0..axes).map(|_| ParallelAxis::new(0.0, 100.0)).collect(),
    });
    let series = SeriesConfig {
        series_id: "big".into(),
        source_id: None,
        z_column: None,
        label: None,
        x_column: columns[0].clone(),
        y_column: columns[1].clone(),
        render_type: DataRenderType::Parallel {
            columns: columns.clone(),
            line: line(RED, 1.0),
            color_column: None,
        },
    };
    let chart = Chart::new(config.clone());
    let started = std::time::Instant::now();
    let img = r.export_panel_rgba(&chart, &[series.clone()], 1.0).unwrap();
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
        row_of(pollster::block_on(r.pick_chart_data(id, request(at)).unwrap().resolve()).unwrap());
    eprintln!("picked in {:?}", started.elapsed());
    assert!(picked.is_some(), "a row under the cursor");
}
