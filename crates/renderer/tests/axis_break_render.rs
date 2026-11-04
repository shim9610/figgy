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
        sizing: Default::default(),
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

// #205: pieces sized by weight draw each piece at its own scale, in the
// shader as the chrome places it, and a pick finds what each piece draws.
#[test]
fn weighted_pieces_draw_and_pick_where_the_chrome_places_them() {
    let mut r = renderer();
    r.enable_gpu_picking().unwrap();
    let xs = [1.0, 2.5, 4.0, 5.5, 7.0, 8.5];
    // Clear of the pieces' edges, where a marker is cut by its piece.
    let ys = [2.0, 5.0, 8.0, 93.0, 95.0, 97.0];
    upload(&mut r, "x", xs.to_vec());
    upload(&mut r, "y", ys.to_vec());
    for sizing in [
        renderer::config::PieceSizing::Weights(vec![3.0, 1.0]),
        renderer::config::PieceSizing::Equal,
    ] {
        let mut config = config(0.0, 100.0, &[(10.0, 90.0)]);
        config.left_y.breaks.as_mut().unwrap().sizing = sizing.clone();
        config.validate().unwrap();
        assert!(
            renderer::layout::BreakMap::of(&config.left_y).per_piece()
                || sizing == renderer::config::PieceSizing::Equal
        );
        let series = [markers("m", "x", "y", RED)];
        for scale in [1.0f32, 2.0] {
            let img = export(&mut r, &config, &series, scale);
            for (i, (&x, &v)) in xs.iter().zip(&ys).enumerate() {
                let (cx, cy) = (col_of(&config, x, scale), row_of(&config, v, scale));
                let (mx, my) = centroid(&img, cx, cy, 10.0 * scale, is(RED))
                    .unwrap_or_else(|| panic!("{sizing:?} {scale}x: no marker for y = {v}"));
                assert!(
                    (mx - cx).abs() < 1.0 && (my - cy).abs() < 1.0,
                    "{sizing:?} {scale}x y = {v}: marker at ({mx}, {my}), chrome at ({cx}, {cy})"
                );
                assert_eq!(
                    pick_at(&mut r, &config, &series, (cx, cy), scale),
                    Some(("m".into(), i)),
                    "{sizing:?} {scale}x pick y = {v}"
                );
            }
            assert_eq!(
                count_in_rows(&img, gap_rows(&config, scale, 0.0), is(RED)),
                0
            );
        }
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

#[test]
fn a_streamed_source_on_a_broken_axis_is_refused() {
    let mut r = renderer();
    let streamed = |id: &str| renderer::StreamColumn {
        id: id.into(),
        len: 4,
        revision: 1,
        encoding: renderer::StreamEncoding::ScalarF32,
        replay: renderer::StreamReplay::RandomAccess,
        statistics: renderer::StreamStatistics::Unknown,
    };
    r.register_streamed_columns(vec![streamed("st_x"), streamed("st_y")])
        .unwrap();
    let series = vec![markers("m", "st_x", "st_y", RED)];
    let refused = |config: renderer::Config, r: &mut Renderer| {
        let id = r
            .register_chart(config, series.clone())
            .expect("registration admits the columns; the streaming inspector names the rule");
        let result = r.inspect_streaming_capability(id);
        r.remove_chart(id).unwrap();
        match result {
            Err(renderer::FiggyError::InvalidSeriesConfig { series_id, reason }) => {
                assert_eq!(series_id, "m");
                assert_eq!(
                    reason,
                    "broken axes require GPU-resident columns; streamed sources are not supported"
                );
            }
            other => panic!(
                "expected the streamed rejection, got {:?}",
                other.map(|_| ())
            ),
        }
    };
    for style in [AxisBreakStyle::Continuous, AxisBreakStyle::Panels] {
        let mut y = config(0.0, 100.0, &[(10.0, 90.0)]);
        if let Some(breaks) = y.left_y.breaks.as_mut() {
            breaks.style = style;
        }
        let x = with_x_cuts(y.clone(), &[(4.0, 6.0)]);
        let mut x_only = x.clone();
        x_only.left_y.breaks = None;
        refused(y, &mut r);
        refused(x_only, &mut r);
    }
    // The same chart without breaks streams.
    let mut plain = config(0.0, 100.0, &[]);
    plain.left_y.breaks = None;
    let id = r.register_chart(plain, series.clone()).unwrap();
    r.inspect_streaming_capability(id).unwrap();
    r.remove_chart(id).unwrap();
}

const MAGENTA: Color = Color::from_rgb8(255, 0, 255);
const CYAN: Color = Color::from_rgb8(0, 255, 255);
const BLACK: Color = Color::from_rgb8(0, 0, 0);

/// A line in black with `color` markers on its points: the kind a line
/// offset member must be.
fn line_markers(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
    let mut series = line(id, x, y, BLACK, None);
    let marker = markers(id, x, y, color);
    if let (DataRenderType::Xy(xy), DataRenderType::Xy(m)) =
        (&mut series.render_type, marker.render_type)
    {
        xy.scatter = m.scatter;
    }
    series
}

/// Where the panel drawing `series_id` places `(x, y)` in a document export
/// at `scale`: its own data area and axes (`Config::expand`).
fn panel_px(config: &renderer::Config, series_id: &str, x: f64, y: f64, scale: f32) -> (f32, f32) {
    let scaled = config.scaled(scale);
    let expansion = scaled.expand().unwrap();
    let panel = &expansion.panels[scaled.panel_of_series(series_id)].config;
    let da = panel.data_area().unwrap().0;
    (
        da.x as f32 + da.width as f32 * renderer::layout::axis_fraction(x, &panel.bottom_x),
        da.y as f32 + da.height as f32 * (1.0 - renderer::layout::axis_fraction(y, &panel.left_y)),
    )
}

/// An overlay cut on its own `y_axis`, a subplot cut on its own axes and
/// line offset members of a broken axis each draw their marks on their
/// panel's pieces, in both styles, at 1x and 2x.
#[test]
fn overlays_subplots_and_line_offsets_draw_their_pieces() {
    let mut r = renderer();
    let column = |r: &mut Renderer, id: &str, v: &[f64]| upload(r, id, v.to_vec());
    column(&mut r, "bx", &[2.0, 8.0]);
    column(&mut r, "by", &[5.0, 95.0]);
    column(&mut r, "oy", &[15.0, 185.0]);
    column(&mut r, "ly", &[5.0, 92.0]);
    column(&mut r, "mx", &[5.0, 5.0]);
    column(&mut r, "my", &[2.0, 98.0]);
    column(&mut r, "lx", &[1.5, 8.5]);
    // Apart on screen, clear of the data area's edges and off the
    // members' lines, which cross x = 5 in the gap: no mark covers another.
    let points = |id: &str| -> (Vec<f64>, Vec<f64>) {
        match id {
            "m" => (vec![5.0, 5.0], vec![2.0, 98.0]),
            "ov" => (vec![5.0, 5.0], vec![15.0, 185.0]),
            "lo0" | "lo1" => (vec![1.5, 8.5], vec![5.0, 92.0]),
            _ => (vec![2.0, 8.0], vec![5.0, 95.0]),
        }
    };
    for style in [AxisBreakStyle::Continuous, AxisBreakStyle::Panels] {
        let mut base = config(0.0, 100.0, &[(10.0, 90.0)]);
        if let Some(breaks) = base.left_y.breaks.as_mut() {
            breaks.style = style;
        }
        let mut sub = with_x_cuts(base.clone(), &[(4.0, 6.0)]);
        sub.chart_area.0.y = HEIGHT;
        let mut overlay = renderer::OverlayLayer::default();
        overlay.y_axis.min = 0.0;
        overlay.y_axis.max = 200.0;
        overlay.y_axis.breaks = base.left_y.breaks.clone().map(|breaks| AxisBreaks {
            ranges: vec![AxisBreak::new(20.0, 180.0)],
            ..breaks
        });
        overlay.series = vec!["ov".into()];
        let mut document = base.clone();
        document.overlays.push(overlay);
        document.subplots.push(renderer::Subplot {
            config: sub,
            series: vec!["sub".into()],
            link_x: None,
            link_y: None,
        });
        document.line_offsets = Some(renderer::data_config::LineOffsets {
            series: vec!["lo0".into(), "lo1".into()],
            // Far enough apart that no member's line covers the other's
            // markers.
            step_y: 3.0,
            step_x: -0.6,
        });
        let series = [
            markers("m", "mx", "my", RED),
            markers("ov", "mx", "oy", BLUE),
            markers("sub", "bx", "by", GREEN),
            line_markers("lo0", "lx", "ly", MAGENTA),
            line_markers("lo1", "lx", "ly", CYAN),
        ];
        let chart = Chart::new(document.clone());
        for scale in [1.0f32, 2.0] {
            let img = r
                .export_rgba(
                    renderer::ExportSource::chart(&chart, &series),
                    renderer::ExportOptions::document(scale),
                )
                .block_on()
                .unwrap();
            if let Ok(dir) = std::env::var("FIGGY_BREAK_DUMP") {
                std::fs::write(
                    format!(
                        "{dir}/layered-{style:?}-{scale}x-{}x{}.rgba",
                        img.width, img.height
                    ),
                    &img.rgba,
                )
                .unwrap();
            }
            for (id, color) in [
                ("m", RED),
                ("ov", BLUE),
                ("sub", GREEN),
                ("lo0", MAGENTA),
                ("lo1", CYAN),
            ] {
                let (xs, ys) = points(id);
                for (&x, &y) in xs.iter().zip(&ys) {
                    let (cx, cy) = panel_px(&document, id, x, y, scale);
                    let (mx, my) =
                        centroid(&img, cx, cy, 10.0 * scale, is(color)).unwrap_or_else(|| {
                            panic!("{style:?} {scale}x {id}: no marker at ({x}, {y})")
                        });
                    assert!(
                        (mx - cx).abs() < 1.0 && (my - cy).abs() < 1.0,
                        "{style:?} {scale}x {id} ({x}, {y}): marker at ({mx}, {my}), panel at ({cx}, {cy})"
                    );
                }
            }
            // The members' lines cross the gap: their own gaps fall on
            // the base's, so nothing of them shows inside it. The axes'
            // break marks reach a few pixels in from the edges.
            let (x0, _, w, _) = data_area(&document, scale);
            let inset = (8.0 * scale) as u32;
            let inked = gap_rows(&document, scale, 1.0)
                .flat_map(|y| (x0 as u32 + inset..(x0 + w) as u32 - inset).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let p = pixel(&img, x, y);
                    p[3] > 0 && p[..3].iter().any(|&c| c < 200)
                })
                .count();
            assert_eq!(inked, 0, "{style:?} {scale}x: ink inside the base's gap");
        }
    }
}

/// The picked highlight is drawn with its mark, at the mark's piece and its
/// shift, on the screen and in the export, and never in a gap.
#[test]
fn the_picked_highlight_rings_the_mark_on_its_piece() {
    let mut r = renderer();
    let points = [(1.0, 5.0), (8.5, 95.0)];
    upload(&mut r, "x", points.iter().map(|p| p.0).collect());
    upload(&mut r, "y", points.iter().map(|p| p.1).collect());
    let series = [markers("m", "x", "y", RED)];
    const GOLD: Color = Color::from_rgb8(255, 215, 0);
    for style in [AxisBreakStyle::Continuous, AxisBreakStyle::Panels] {
        let mut plain = with_x_cuts(config(0.0, 100.0, &[(10.0, 90.0)]), &[(4.0, 7.0)]);
        for axis in [&mut plain.bottom_x, &mut plain.left_y] {
            if let Some(breaks) = axis.breaks.as_mut() {
                breaks.style = style;
            }
        }
        for (index, &(x, y)) in points.iter().enumerate() {
            let mut picked = plain.clone();
            picked.picked_data = Some(renderer::DataSelectionsConfig {
                refs: vec![renderer::PickedDataRef::Point {
                    source_id: None,
                    series_id: "m".into(),
                    point_index: index,
                }],
                ..Default::default()
            });
            let mut images = vec![(1.0, screen(&mut r, &picked, &series))];
            for scale in [1.0f32, 2.0] {
                images.push((scale, export(&mut r, &picked, &series, scale)));
            }
            for (scale, img) in images {
                let (cx, cy) = (col_of(&picked, x, scale), row_of(&picked, y, scale));
                let ring = centroid(&img, cx, cy, 14.0 * scale, is(GOLD)).unwrap_or_else(|| {
                    panic!("{style:?} {scale}x point {index}: no highlight at ({cx}, {cy})")
                });
                assert!(
                    (ring.0 - cx).abs() < 1.5 && (ring.1 - cy).abs() < 1.5,
                    "{style:?} {scale}x point {index}: ring at {ring:?}, mark at ({cx}, {cy})"
                );
                assert_eq!(
                    count_in_rows(&img, gap_rows(&picked, scale, 1.0), is(GOLD)),
                    0,
                    "{style:?} {scale}x: highlight in the gap"
                );
            }
        }
    }
}

/// The annotation layer follows the pieces: a span and a reference line
/// across the gap are drawn on both sides of it and not inside it, in both
/// styles, at 1x and 2x.
#[test]
fn annotations_across_a_gap_are_drawn_per_piece() {
    use renderer::{Annotation, AnnotationAxis, AnnotationKind};
    let mut r = renderer();
    upload(&mut r, "x", vec![1.0]);
    upload(&mut r, "y", vec![5.0]);
    let series = [markers("m", "x", "y", RED)];
    for style in [AxisBreakStyle::Continuous, AxisBreakStyle::Panels] {
        let mut config = config(0.0, 100.0, &[(10.0, 90.0)]);
        if let Some(breaks) = config.left_y.breaks.as_mut() {
            breaks.style = style;
        }
        config.annotations.items = vec![
            Annotation::new(AnnotationKind::Span {
                axis: AnnotationAxis::Y,
                from: 5.0,
                to: 95.0,
                fill: GREEN,
            }),
            Annotation::new(AnnotationKind::ReferenceLine {
                axis: AnnotationAxis::X,
                value: 7.0,
                style: renderer::AnnotationLineStyle {
                    color: BLUE,
                    width: 3.0,
                    ..Default::default()
                },
                label: None,
            }),
        ];
        for scale in [1.0f32, 2.0] {
            let img = export(&mut r, &config, &series, scale);
            let gap = gap_rows(&config, scale, 1.0);
            for (name, color) in [("span", GREEN), ("line", BLUE)] {
                assert_eq!(
                    count_in_rows(&img, gap.clone(), is(color)),
                    0,
                    "{style:?} {scale}x: the {name} inside the gap"
                );
                let below = gap.end + 2..gap.end + 4;
                let above = gap.start.saturating_sub(4)..gap.start.saturating_sub(2);
                assert!(
                    count_in_rows(&img, below, is(color)) > 0
                        && count_in_rows(&img, above, is(color)) > 0,
                    "{style:?} {scale}x: the {name} on both sides of the gap"
                );
            }
        }
    }
}

/// The cost of the pieces (rule 10: measured, never capped). A million
/// markers on a chart cut three times on each axis, 4 × 4 `Panels` pieces,
/// against the same chart unbroken: every piece draws the panel's series
/// once, so the draw cost grows with the piece count, and a pick runs once
/// whatever the pieces. The times are printed, not asserted; every marker
/// is drawn on its piece (one checked).
#[test]
fn sixteen_pieces_over_a_million_points_draw_and_pick() {
    use std::time::Instant;
    const N: usize = 1_000_000;
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        64 * 1024 * 1024,
    )
    .expect("renderer init");
    r.enable_gpu_picking().unwrap();
    // A deterministic spread over 0..100 on both axes.
    let spread = |seed: u64| -> Vec<f64> {
        (0..N as u64)
            .map(|i| {
                let mut x = i
                    .wrapping_add(seed.wrapping_mul(0xD1B5_4A32_D192_ED03))
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15);
                x ^= x >> 33;
                x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
                x ^= x >> 33;
                (x as f64 / u64::MAX as f64) * 100.0
            })
            .collect()
    };
    upload(&mut r, "x", spread(1));
    upload(&mut r, "y", spread(2));
    let cuts = [(20.0, 30.0), (45.0, 55.0), (70.0, 80.0)];
    let mut broken = config(0.0, 100.0, &cuts);
    broken.bottom_x.max = 100.0;
    let broken = panels(with_x_cuts(broken, &cuts));
    let mut whole = broken.clone();
    whole.bottom_x.breaks = None;
    whole.left_y.breaks = None;
    let series = [markers("m", "x", "y", RED)];
    let mut median = |config: &renderer::Config| {
        let mut times: Vec<f64> = (0..3)
            .map(|_| {
                let start = Instant::now();
                let _ = export(&mut r, config, &series, 1.0);
                start.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times[1]
    };
    let (whole_ms, broken_ms) = (median(&whole), median(&broken));
    let mut pick_ms = |config: &renderer::Config| {
        let at = (col_of(config, 10.0, 1.0), row_of(config, 10.0, 1.0));
        let start = Instant::now();
        let hit = pick_at(&mut r, config, &series, at, 1.0);
        (start.elapsed().as_secs_f64() * 1e3, hit)
    };
    // The first pick builds the pick pipelines; it is not timed.
    let _ = pick_ms(&whole);
    let (whole_pick, _) = pick_ms(&whole);
    let (broken_pick, hit) = pick_ms(&broken);
    eprintln!(
        "{N} markers, export 1x median of 3: unbroken {whole_ms:.1} ms, 4 x 4 pieces \
         {broken_ms:.1} ms; pick (with registration): unbroken {whole_pick:.1} ms, \
         4 x 4 pieces {broken_pick:.1} ms"
    );
    assert!(
        hit.is_some(),
        "a dense spread has a marker under the cursor"
    );
    let img = export(&mut r, &broken, &series, 1.0);
    let (cx, cy) = (col_of(&broken, 90.0, 1.0), row_of(&broken, 90.0, 1.0));
    assert!(
        centroid(&img, cx, cy, 4.0, is(RED)).is_some(),
        "the last piece draws its markers"
    );
}

/// Bar charts, where a cut is used most: category bars in both directions
/// and both styles, a tall bar crossing the gap and short ones below it.
/// The tall bar is drawn on both pieces and its end lands where the value
/// axis places it; no bar is drawn inside the gap; the short ones end on
/// the first piece (#200).
#[test]
fn category_bars_cross_a_value_axis_cut() {
    use renderer::config::Category;
    use renderer::data_config::{
        BarGroups, CategoryBarDirection, CategoryBarMode, CategoryBarStyle,
    };
    let mut r = renderer();
    upload(&mut r, "pos", vec![0.0, 1.0, 2.0]);
    let values = [12.0, 96.0, 25.0];
    upload(&mut r, "val", values.to_vec());
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        for style in [AxisBreakStyle::Continuous, AxisBreakStyle::Panels] {
            let mut config = config(0.0, 100.0, &[]);
            config.left_y.breaks = None;
            let categories = Some(vec![
                Category::new("a", "A"),
                Category::new("b", "B"),
                Category::new("c", "C"),
            ]);
            let cut = Some(AxisBreaks {
                ranges: vec![AxisBreak::new(30.0, 85.0)],
                style,
                gap: 0.1,
                mark: BreakMark::None,
                frame_each_panel: false,
                sizing: Default::default(),
            });
            let (x, y) = match direction {
                CategoryBarDirection::Vertical => {
                    config.bottom_x.categories = categories;
                    config.left_y.breaks = cut;
                    ("pos", "val")
                }
                CategoryBarDirection::Horizontal => {
                    config.left_y.categories = categories;
                    config.bottom_x.min = 0.0;
                    config.bottom_x.max = 100.0;
                    config.bottom_x.breaks = cut;
                    ("val", "pos")
                }
            };
            config.bar_groups = Some(BarGroups {
                mode: CategoryBarMode::Grouped,
                direction,
                group_width: 0.8,
                bar_gap: 4.0,
            });
            let series = [SeriesConfig {
                series_id: "bars".into(),
                source_id: None,
                label: None,
                x_column: x.into(),
                y_column: y.into(),
                z_column: None,
                render_type: DataRenderType::CategoryBars {
                    color: RED,
                    style: CategoryBarStyle {
                        material: renderer::Material::Flat,
                        corner_radius: 0.0,
                        outline: false,
                        ..CategoryBarStyle::default()
                    },
                    labels: Default::default(),
                    label_format: Default::default(),
                    overrides: None,
                },
            }];
            for scale in [1.0f32, 2.0] {
                let img = export(&mut r, &config, &series, scale);
                if let Ok(dir) = std::env::var("FIGGY_BREAK_DUMP") {
                    std::fs::write(
                        format!(
                            "{dir}/bars-{direction:?}-{style:?}-{scale}x-{}x{}.rgba",
                            img.width, img.height
                        ),
                        &img.rgba,
                    )
                    .unwrap();
                }
                let vertical = direction == CategoryBarDirection::Vertical;
                let axis = if vertical {
                    &config.left_y
                } else {
                    &config.bottom_x
                };
                let (dx, dy, dw, dh) = data_area(&config, scale);
                // The value axis' pixel of `v` and the category centre `k`.
                let value_px = |v: f64| {
                    let f = renderer::layout::axis_fraction(v, axis);
                    if vertical {
                        dy + dh * (1.0 - f)
                    } else {
                        dx + dw * f
                    }
                };
                let centre = |k: usize| {
                    let f = (k as f32 + 0.5) / 3.0;
                    if vertical {
                        dx + dw * f
                    } else {
                        dy + dh * (1.0 - f)
                    }
                };
                // The red run along the value axis through category `k`.
                let run = |k: usize| -> Vec<f32> {
                    let c = centre(k) as u32;
                    let len = if vertical { img.height } else { img.width };
                    (0..len)
                        .filter(|&i| {
                            let (px, py) = if vertical { (c, i) } else { (i, c) };
                            is(RED)(pixel(&img, px, py))
                        })
                        .map(|i| i as f32 + 0.5)
                        .collect()
                };
                let gap = {
                    let map = renderer::layout::BreakMap::of(axis);
                    let k = map.knots()[0];
                    let (lo, hi) = (k.start, k.end);
                    let to_px = |t: f64| {
                        if vertical {
                            dy + dh * (1.0 - t as f32)
                        } else {
                            dx + dw * t as f32
                        }
                    };
                    let (p, q) = (to_px(lo), to_px(hi));
                    (p.min(q) + 1.0, p.max(q) - 1.0)
                };
                for (k, &v) in values.iter().enumerate() {
                    let px = run(k);
                    let tag = format!("{direction:?} {style:?} {scale}x bar {k} ({v})");
                    assert!(!px.is_empty(), "{tag}: not drawn");
                    assert!(
                        px.iter().all(|&p| p < gap.0 || p > gap.1),
                        "{tag}: drawn inside the gap {gap:?}"
                    );
                    // The value end: the far end from the baseline at 0.
                    let end = if vertical {
                        px.iter().copied().fold(f32::INFINITY, f32::min)
                    } else {
                        px.iter().copied().fold(f32::NEG_INFINITY, f32::max)
                    };
                    let expected = value_px(v);
                    assert!(
                        (end - expected).abs() <= 1.5,
                        "{tag}: ends at {end}, the axis puts {v} at {expected}"
                    );
                    // The tall bar shows on both sides of the gap.
                    if v > 85.0 {
                        assert!(
                            px.iter().any(|&p| p < gap.0) && px.iter().any(|&p| p > gap.1),
                            "{tag}: not on both pieces"
                        );
                    }
                }
            }
        }
    }
}
