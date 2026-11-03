//! End-to-end pixel verification of broken axes (#200) through the public
//! renderer API and the headless `export_rgba`: values on each piece land
//! where the axis chrome places them (`layout::axis_fraction`, the CPU twin
//! of the shader's cut remap), nothing is drawn inside a gap, a line or a
//! fill crossing a gap shows only inside the pieces, a logarithmic axis is
//! cut in decades, cuts outside the range change no pixel, and every check
//! holds at 1x and 2x.
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
