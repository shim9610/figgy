//! End-to-end pixel verification of broken axes (#200) through the public
//! renderer API and the headless `export_rgba`: values on each piece land
//! where the axis chrome places them (`layout::axis_fraction`, the CPU twin
//! of the shader's cut remap), nothing is drawn inside a gap, a line or a
//! fill crossing a gap shows only inside the pieces, a logarithmic axis is
//! cut in decades, cuts outside the range change no pixel, a `Panels` axis
//! draws each piece at its own linear map, and every check holds at 1x and
//! 2x.
//!
//! Grid, legend, title and axis lines are off, and every mark has its own
//! pure colour, so a colour predicate attributes every pixel to one mark.

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::config::{AxisBreak, AxisBreakStyle, AxisBreaks, AxisScale, BreakMark};
use renderer::data::Column;
use renderer::data_config::{
    DataLineStyleConfig, DataScatterStyleConfig, LineFill, LineInterpolation, ScatterShape,
    XySeries,
};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{Chart, Color, DataRenderType, RasterImage, Renderer, RendererDevice, SeriesConfig};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const RED: Color = Color::from_rgb8(255, 0, 0);
const BLUE: Color = Color::from_rgb8(0, 0, 255);
const GREEN: Color = Color::from_rgb8(0, 255, 0);

fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(test_gpu::device);
    (device.clone(), queue.clone())
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init")
}

fn upload(r: &mut Renderer, id: &str, data: Vec<f64>) {
    r.add_column(id, &Column { data }).unwrap();
}

/// A bare chart: x over 0..10, y over `y_min..y_max` cut by `cuts`.
fn config(y_min: f64, y_max: f64, cuts: &[(f64, f64)]) -> renderer::Config {
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
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 10.0;
    config.left_y.min = y_min;
    config.left_y.max = y_max;
    config.left_y.breaks = Some(AxisBreaks {
        ranges: cuts.iter().map(|&(a, b)| AxisBreak::new(a, b)).collect(),
        style: AxisBreakStyle::Continuous,
        gap: 0.1,
        mark: BreakMark::None,
        frame_each_panel: false,
    });
    config
}

fn markers(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
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
        }),
    }
}

fn line(id: &str, x: &str, y: &str, color: Color, fill: Option<LineFill>) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Xy(XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: color,
                line_width: 2.0,
                fill_color: fill.as_ref().map(|_| GREEN),
                fill,
                interpolation: LineInterpolation::Linear,
            }),
            errors: None,
        }),
    }
}

fn export(
    r: &mut Renderer,
    config: &renderer::Config,
    series: &[SeriesConfig],
    scale: f32,
) -> RasterImage {
    let chart = Chart::new(config.clone());
    r.export_rgba(
        renderer::ExportSource::chart(&chart, series),
        renderer::ExportOptions::panel(scale),
    )
    .block_on()
    .unwrap()
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

fn is(color: Color) -> impl Fn([u8; 4]) -> bool {
    let [r, g, b] = [color.r, color.g, color.b].map(|c| (c * 255.0).round() as i32);
    move |p| {
        (p[0] as i32 - r).abs() < 40 && (p[1] as i32 - g).abs() < 40 && (p[2] as i32 - b).abs() < 40
    }
}

/// The data area at `scale`, in image pixels: the export lays out the
/// chart scaled by `scale` (`Config::scaled`), so its rounding is that of
/// the scaled layout, not the 1x area doubled.
fn data_area(config: &renderer::Config, scale: f32) -> (f32, f32, f32, f32) {
    let da = config.scaled(scale).data_area().unwrap().0;
    (da.x as f32, da.y as f32, da.width as f32, da.height as f32)
}

/// The image row of y value `v`, where the axis chrome places it.
fn row_of(config: &renderer::Config, v: f64, scale: f32) -> f32 {
    let (_, y, _, h) = data_area(config, scale);
    y + h * (1.0 - renderer::layout::axis_fraction(v, &config.left_y))
}

/// The image column of x value `v`.
fn col_of(config: &renderer::Config, v: f64, scale: f32) -> f32 {
    let (x, _, w, _) = data_area(config, scale);
    x + w * renderer::layout::axis_fraction(v, &config.bottom_x)
}

/// The rows of the gap of the first cut, inset by `inset` pixels.
fn gap_rows(config: &renderer::Config, scale: f32, inset: f32) -> std::ops::Range<u32> {
    let map = renderer::layout::BreakMap::of(&config.left_y);
    let knot = map.knots()[0];
    let (_, y, _, h) = data_area(config, scale);
    let top = y + h * (1.0 - knot.end as f32);
    let bottom = y + h * (1.0 - knot.start as f32);
    (top + inset).ceil() as u32..(bottom - inset).floor() as u32
}

fn count_in_rows(
    img: &RasterImage,
    rows: std::ops::Range<u32>,
    pred: impl Fn([u8; 4]) -> bool,
) -> usize {
    rows.flat_map(|y| (0..img.width).map(move |x| (x, y)))
        .filter(|&(x, y)| pred(pixel(img, x, y)))
        .count()
}

/// The centroid of the pixels matching `pred` within `radius` of `(cx, cy)`.
fn centroid(
    img: &RasterImage,
    cx: f32,
    cy: f32,
    radius: f32,
    pred: impl Fn([u8; 4]) -> bool,
) -> Option<(f32, f32)> {
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    let lo = |c: f32| (c - radius).max(0.0) as u32;
    for y in lo(cy)..((cy + radius) as u32).min(img.height) {
        for x in lo(cx)..((cx + radius) as u32).min(img.width) {
            if pred(pixel(img, x, y)) {
                sx += x as f32 + 0.5;
                sy += y as f32 + 0.5;
                n += 1.0;
            }
        }
    }
    (n > 0.0).then(|| (sx / n, sy / n))
}

#[test]
fn markers_on_each_piece_land_where_the_chrome_places_them() {
    let mut r = renderer();
    let ys = [2.0, 5.0, 9.0, 91.0, 95.0, 99.0];
    upload(&mut r, "x", vec![1.0, 2.5, 4.0, 5.5, 7.0, 8.5]);
    upload(&mut r, "y", ys.to_vec());
    let config = config(0.0, 100.0, &[(10.0, 90.0)]);
    let series = [markers("m", "x", "y", RED)];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        for (i, &v) in ys.iter().enumerate() {
            let x = [1.0, 2.5, 4.0, 5.5, 7.0, 8.5][i];
            let (cx, cy) = (col_of(&config, x, scale), row_of(&config, v, scale));
            let (mx, my) = centroid(&img, cx, cy, 10.0 * scale, is(RED))
                .unwrap_or_else(|| panic!("{scale}x: no marker for y = {v}"));
            assert!(
                (mx - cx).abs() < 1.0 && (my - cy).abs() < 1.0,
                "{scale}x y = {v}: marker at ({mx}, {my}), chrome at ({cx}, {cy})"
            );
        }
        assert_eq!(
            count_in_rows(&img, gap_rows(&config, scale, 0.0), is(RED)),
            0
        );
    }
}

#[test]
fn a_line_and_a_fill_crossing_a_gap_show_only_inside_the_pieces() {
    let mut r = renderer();
    upload(&mut r, "x", vec![1.0, 9.0]);
    upload(&mut r, "y", vec![5.0, 95.0]);
    let config = config(0.0, 100.0, &[(10.0, 90.0)]);
    let series = [line("l", "x", "y", BLUE, Some(LineFill::ToBaseline(0.0)))];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        // Inset by a pixel: the export's layout rounds the piece edges, and
        // the check is that nothing is drawn inside the gap.
        let gap = gap_rows(&config, scale, 1.0);
        assert_eq!(
            count_in_rows(&img, gap.clone(), is(BLUE)),
            0,
            "{scale}x line in the gap"
        );
        assert_eq!(
            count_in_rows(&img, gap.clone(), is(GREEN)),
            0,
            "{scale}x fill in the gap"
        );
        // The fill reaches both pieces, next to the gap on either side.
        let below = gap.end + 2..gap.end + 4;
        let above = gap.start.saturating_sub(4)..gap.start.saturating_sub(2);
        assert!(
            count_in_rows(&img, below, is(GREEN)) > 0,
            "{scale}x fill below the gap"
        );
        assert!(
            count_in_rows(&img, above, is(GREEN)) > 0,
            "{scale}x fill above the gap"
        );
        // The line is straight across the gap: its two drawn ends sit on
        // the straight segment between (1, 5) and (9, 95) as remapped.
        let (x0, y0) = (col_of(&config, 1.0, scale), row_of(&config, 5.0, scale));
        let (x1, y1) = (col_of(&config, 9.0, scale), row_of(&config, 95.0, scale));
        for &y in &[gap.end + 3, gap.start.saturating_sub(3)] {
            let x = x0 + (x1 - x0) * (y as f32 + 0.5 - y0) / (y1 - y0);
            let found = (0..img.width)
                .filter(|&c| is(BLUE)(pixel(&img, c, y)))
                .collect::<Vec<_>>();
            assert!(
                found
                    .iter()
                    .any(|&c| (c as f32 + 0.5 - x).abs() < 2.5 * scale),
                "{scale}x row {y}: line at {found:?}, straight segment at {x}"
            );
        }
    }
}

#[test]
fn a_logarithmic_axis_is_cut_in_decades() {
    let mut r = renderer();
    let ys = [3.0, 30.0, 3e4, 3e5];
    upload(&mut r, "x", vec![2.0, 4.0, 6.0, 8.0]);
    upload(&mut r, "y", ys.to_vec());
    let mut config = config(1.0, 1e6, &[(100.0, 1e4)]);
    config.left_y.scale = AxisScale::Logarithmic;
    let series = [markers("m", "x", "y", RED)];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        for (i, &v) in ys.iter().enumerate() {
            let x = [2.0, 4.0, 6.0, 8.0][i];
            let (cx, cy) = (col_of(&config, x, scale), row_of(&config, v, scale));
            let (mx, my) = centroid(&img, cx, cy, 10.0 * scale, is(RED))
                .unwrap_or_else(|| panic!("{scale}x: no marker for y = {v}"));
            assert!(
                (mx - cx).abs() < 1.0 && (my - cy).abs() < 1.0,
                "{scale}x y = {v}: ({mx}, {my}) vs ({cx}, {cy})"
            );
        }
    }
}

#[test]
fn cuts_outside_the_range_and_no_breaks_draw_the_same_pixels() {
    let mut r = renderer();
    upload(&mut r, "x", vec![1.0, 5.0, 9.0]);
    upload(&mut r, "y", vec![10.0, 60.0, 30.0]);
    let series = [
        line("l", "x", "y", BLUE, Some(LineFill::ToBaseline(0.0))),
        markers("m", "x", "y", RED),
    ];
    let outside = config(0.0, 100.0, &[(200.0, 300.0)]);
    let mut none = outside.clone();
    none.left_y.breaks = None;
    for scale in [1.0f32, 2.0] {
        let a = export(&mut r, &outside, &series, scale);
        let b = export(&mut r, &none, &series, scale);
        assert!(
            a.rgba == b.rgba,
            "{scale}x: an out-of-range cut changed pixels"
        );
    }
}

/// What a data pick at `at` (image pixels) on the chart at `scale` answers,
/// as `(series_id, point index)`.
fn pick_at(
    r: &mut Renderer,
    config: &renderer::Config,
    series: &[SeriesConfig],
    at: (f32, f32),
    scale: f32,
) -> Option<(String, usize)> {
    let id = r.register_chart(config.clone(), series.to_vec()).unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0, at.1],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: (WIDTH as f32 * scale) as u32,
            height: (HEIGHT as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: 6.0 * scale,
    };
    let hit = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    match hit {
        Some(renderer::PickedData::Point {
            series_id,
            point_index,
            ..
        }) => Some((series_id, point_index)),
        Some(other) => panic!("unexpected pick {other:?}"),
        None => None,
    }
}

#[test]
fn a_pick_hits_what_each_piece_draws_and_nothing_in_a_gap() {
    let mut r = renderer();
    r.enable_gpu_picking().unwrap();
    upload(&mut r, "mx", vec![2.0, 8.0]);
    upload(&mut r, "my", vec![5.0, 95.0]);
    upload(&mut r, "lx", vec![1.0, 9.0]);
    upload(&mut r, "ly", vec![5.0, 95.0]);
    let config = config(0.0, 100.0, &[(10.0, 90.0)]);
    let series = [
        markers("m", "mx", "my", RED),
        line("l", "lx", "ly", BLUE, None),
    ];
    for scale in [1.0f32, 2.0] {
        // Each marker, on its own piece.
        for (index, (x, y)) in [(2.0, 5.0), (8.0, 95.0)].into_iter().enumerate() {
            let at = (col_of(&config, x, scale), row_of(&config, y, scale));
            assert_eq!(
                pick_at(&mut r, &config, &series, at, scale),
                Some(("m".into(), index)),
                "{scale}x marker {index}"
            );
        }
        // The line where it is drawn, just below the gap, and where it
        // crosses the gap, which draws nothing.
        let (x0, y0) = (col_of(&config, 1.0, scale), row_of(&config, 5.0, scale));
        let (x1, y1) = (col_of(&config, 9.0, scale), row_of(&config, 95.0, scale));
        let on_line = |y: f32| (x0 + (x1 - x0) * (y - y0) / (y1 - y0), y);
        let gap = gap_rows(&config, scale, 1.0);
        let below = on_line(gap.end as f32 + 4.0 * scale);
        assert!(
            matches!(pick_at(&mut r, &config, &series, below, scale), Some((ref id, _)) if id == "l"),
            "{scale}x the line below the gap"
        );
        let middle = on_line((gap.start + gap.end) as f32 / 2.0);
        assert_eq!(
            pick_at(&mut r, &config, &series, middle, scale),
            None,
            "{scale}x the line inside the gap"
        );
    }
}

/// `config` with every broken axis drawn as `Panels`.
fn panels(mut config: renderer::Config) -> renderer::Config {
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        if let Some(breaks) = axis.breaks.as_mut() {
            breaks.style = AxisBreakStyle::Panels;
        }
    }
    config
}

/// `config` with `bottom_x` cut by `cuts` as well.
fn with_x_cuts(mut config: renderer::Config, cuts: &[(f64, f64)]) -> renderer::Config {
    config.bottom_x.breaks = config.left_y.breaks.clone().map(|breaks| AxisBreaks {
        ranges: cuts.iter().map(|&(a, b)| AxisBreak::new(a, b)).collect(),
        ..breaks
    });
    config
}

#[test]
fn panels_markers_on_every_piece_land_where_the_chrome_places_them() {
    let mut r = renderer();
    // A marker in each of the four pieces of a chart broken on both axes,
    // and one at each edge of the x cut.
    let points = [
        (1.0, 5.0),
        (8.5, 5.0),
        (1.0, 95.0),
        (8.5, 95.0),
        (3.5, 50.0),
        (7.5, 92.0),
    ];
    upload(&mut r, "x", points.iter().map(|p| p.0).collect());
    upload(&mut r, "y", points.iter().map(|p| p.1).collect());
    let y_only = panels(config(0.0, 100.0, &[(10.0, 90.0)]));
    let both = panels(with_x_cuts(
        config(0.0, 100.0, &[(10.0, 90.0)]),
        &[(4.0, 7.0)],
    ));
    let mut x_inverted = both.clone();
    x_inverted.bottom_x.inverted = true;
    let mut y_inverted = both.clone();
    y_inverted.left_y.inverted = true;
    let series = [markers("m", "x", "y", RED)];
    for config in [&y_only, &both, &x_inverted, &y_inverted] {
        for scale in [1.0f32, 2.0] {
            let img = export(&mut r, config, &series, scale);
            for &(x, y) in &points {
                if renderer::layout::value_in_cut(&config.bottom_x, x)
                    || renderer::layout::value_in_cut(&config.left_y, y)
                {
                    continue;
                }
                let (cx, cy) = (col_of(config, x, scale), row_of(config, y, scale));
                let (mx, my) = centroid(&img, cx, cy, 10.0 * scale, is(RED))
                    .unwrap_or_else(|| panic!("{scale}x: no marker for ({x}, {y})"));
                assert!(
                    (mx - cx).abs() < 1.0 && (my - cy).abs() < 1.0,
                    "{scale}x ({x}, {y}): marker at ({mx}, {my}), chrome at ({cx}, {cy}), \
                     inverted x {} y {}",
                    config.bottom_x.inverted,
                    config.left_y.inverted,
                );
            }
        }
    }
}

/// Where the straight line through `(1, 5)` and `(9, 95)` is at `y`.
fn true_line_x(y: f64) -> f64 {
    1.0 + (y - 5.0) * 8.0 / 90.0
}

#[test]
fn a_panels_line_keeps_its_own_slope_in_each_piece() {
    let mut r = renderer();
    upload(&mut r, "x", vec![1.0, 9.0]);
    upload(&mut r, "y", vec![5.0, 95.0]);
    let config = panels(config(0.0, 100.0, &[(10.0, 90.0)]));
    let series = [line("l", "x", "y", BLUE, Some(LineFill::ToBaseline(0.0)))];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        let gap = gap_rows(&config, scale, 1.0);
        assert_eq!(
            count_in_rows(&img, gap.clone(), is(BLUE)),
            0,
            "{scale}x line in the gap"
        );
        assert_eq!(
            count_in_rows(&img, gap, is(GREEN)),
            0,
            "{scale}x fill in the gap"
        );
        // Each piece draws the true line at its own map, so near each edge
        // of the gap the line is where the data puts it, not on a straight
        // segment between the two pieces (a `Continuous` axis would put it
        // 50 px and more away).
        for y in [6.0, 8.5, 91.5, 94.0] {
            let row = row_of(&config, y, scale);
            let x = col_of(&config, true_line_x(y), scale);
            let found = (0..img.width)
                .filter(|&c| is(BLUE)(pixel(&img, c, row as u32)))
                .collect::<Vec<_>>();
            assert!(
                found
                    .iter()
                    .any(|&c| (c as f32 + 0.5 - x).abs() < 2.5 * scale),
                "{scale}x y = {y}: line at {found:?}, data at {x}"
            );
        }
    }
}

#[test]
fn a_panels_pick_hits_what_each_piece_draws() {
    let mut r = renderer();
    r.enable_gpu_picking().unwrap();
    upload(&mut r, "mx", vec![2.0, 8.0]);
    upload(&mut r, "my", vec![5.0, 95.0]);
    upload(&mut r, "lx", vec![1.0, 9.0]);
    upload(&mut r, "ly", vec![5.0, 95.0]);
    let config = panels(with_x_cuts(
        config(0.0, 100.0, &[(10.0, 90.0)]),
        &[(4.0, 6.0)],
    ));
    let series = [
        markers("m", "mx", "my", RED),
        line("l", "lx", "ly", BLUE, None),
    ];
    for scale in [1.0f32, 2.0] {
        for (index, (x, y)) in [(2.0, 5.0), (8.0, 95.0)].into_iter().enumerate() {
            let at = (col_of(&config, x, scale), row_of(&config, y, scale));
            assert_eq!(
                pick_at(&mut r, &config, &series, at, scale),
                Some(("m".into(), index)),
                "{scale}x marker {index}"
            );
        }
        // The line where the top right piece draws it, and nothing where
        // a straight segment across the gap would be.
        let y = 93.0;
        let drawn = (
            col_of(&config, true_line_x(y), scale),
            row_of(&config, y, scale),
        );
        assert!(
            matches!(pick_at(&mut r, &config, &series, drawn, scale), Some((ref id, _)) if id == "l"),
            "{scale}x the line in the top right piece"
        );
        let gap = gap_rows(&config, scale, 1.0);
        let middle = (
            col_of(&config, true_line_x(50.0), scale),
            (gap.start + gap.end) as f32 / 2.0,
        );
        assert_eq!(
            pick_at(&mut r, &config, &series, middle, scale),
            None,
            "{scale}x the line inside the gap"
        );
    }
}

/// The chart as the screen path draws it at 1x: `prepare` and
/// `paint_prepared` into a target of the chart's size, through the resident
/// composite and the live picked slot.
fn screen(r: &mut Renderer, config: &renderer::Config, series: &[SeriesConfig]) -> RasterImage {
    let chart = Chart::new(config.clone());
    let rect = config.chart_area.0;
    let view = r.create_chart_view(&chart, rect).unwrap();
    let styles: Vec<_> = series
        .iter()
        .map(|cfg| r.create_style_for_series(cfg).unwrap())
        .collect();
    let drawn: Vec<_> = series
        .iter()
        .zip(&styles)
        .map(|(config, style)| renderer::Series { config, style })
        .collect();
    let items = [renderer::ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &drawn,
    }];
    let prepared = r.prepare(&items).unwrap();
    let (w, h) = (rect.width, rect.height);
    let device = r.device().clone();
    let queue = r.queue().clone();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("axis break screen target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target = texture.create_view(&Default::default());
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("axis break screen readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("axis break screen paint"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        r.paint_prepared(&mut pass, (w, h), &prepared).unwrap();
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    drop(prepared);
    r.end_gpu_frame();
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        rgba.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    drop(data);
    buffer.unmap();
    RasterImage {
        width: w,
        height: h,
        rgba,
    }
}

#[test]
fn the_screen_draws_each_panels_piece_where_the_export_does() {
    let mut r = renderer();
    let points = [(1.0, 5.0), (8.5, 5.0), (1.0, 95.0), (8.5, 95.0)];
    upload(&mut r, "x", points.iter().map(|p| p.0).collect());
    upload(&mut r, "y", points.iter().map(|p| p.1).collect());
    upload(&mut r, "lx", vec![1.0, 9.0]);
    upload(&mut r, "ly", vec![5.0, 95.0]);
    let config = panels(with_x_cuts(
        config(0.0, 100.0, &[(10.0, 90.0)]),
        &[(4.0, 7.0)],
    ));
    let series = [
        line("l", "lx", "ly", BLUE, None),
        markers("m", "x", "y", RED),
    ];
    let img = screen(&mut r, &config, &series);
    for &(x, y) in &points {
        let (cx, cy) = (col_of(&config, x, 1.0), row_of(&config, y, 1.0));
        let (mx, my) = centroid(&img, cx, cy, 10.0, is(RED))
            .unwrap_or_else(|| panic!("screen: no marker for ({x}, {y})"));
        assert!(
            (mx - cx).abs() < 1.0 && (my - cy).abs() < 1.0,
            "screen ({x}, {y}): marker at ({mx}, {my}), chrome at ({cx}, {cy})"
        );
    }
    let y = 93.0;
    let (row, x) = (
        row_of(&config, y, 1.0) as u32,
        col_of(&config, true_line_x(y), 1.0),
    );
    let found = (0..img.width)
        .filter(|&c| is(BLUE)(pixel(&img, c, row)))
        .collect::<Vec<_>>();
    assert!(
        found.iter().any(|&c| (c as f32 + 0.5 - x).abs() < 2.5),
        "screen y = {y}: line at {found:?}, data at {x}"
    );
    assert_eq!(
        count_in_rows(&img, gap_rows(&config, 1.0, 1.0), |p| is(RED)(p)
            || is(BLUE)(p)),
        0,
        "screen: data in the gap"
    );
}
