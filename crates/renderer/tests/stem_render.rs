//! Stem and lollipop plots (#206): pixel checks of the stems an xy series
//! draws from a baseline to each point, through the public `export_rgba`
//! path, plus their pick, fit, streaming refusal and size scaling.
//!
//! House pattern (see `line_area_step.rs`): each test builds its own
//! renderer on a shared device; stems are red, markers blue, the background
//! transparent, so black axis ink never matches a predicate. Expected
//! positions go through the chart's own data area and axis range.

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::config::AxisScale;
use renderer::data::Column;
use renderer::data_config::{
    CategoryBarDirection, DataScatterStyleConfig, ScatterShape, Stem, XySeries,
};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{Chart, Color, DataRenderType, RasterImage, Renderer, RendererDevice, SeriesConfig};

fn col_f64(data: Vec<f64>) -> Column<f64> {
    Column { data }
}

fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(test_gpu::device);
    (device.clone(), queue.clone())
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Bgra8Unorm,
        64 * 1024 * 1024,
    )
    .expect("renderer init")
}

/// A chart with grid and legend off, so probes see only the series.
fn bare_chart(width: u32, height: u32) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width,
        height,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    Chart::new(config)
}

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

fn blue_points(size: f32) -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: BLUE,
        point_shape: ScatterShape::CircleFilled,
        point_size: size,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    }
}

fn red_stem(baseline: f64, direction: CategoryBarDirection, width: f32) -> Stem {
    Stem {
        baseline,
        direction,
        color: RED,
        width,
        line_style: LineStylePreset::Solid,
    }
}

fn stem_series(id: &str, x: &str, y: &str, stem: Stem) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Xy(XySeries {
            scatter: Some(blue_points(3.0)),
            line: None,
            errors: None,
            stem: Some(stem),
        }),
    }
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_rgba(
        renderer::ExportSource::chart(chart, series),
        renderer::ExportOptions::panel(scale),
    )
    .block_on()
    .unwrap()
}

/// The whole document: overlays, subplots and line offset members are
/// drawn too.
fn export_document(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig]) -> RasterImage {
    r.export_rgba(
        renderer::ExportSource::chart(chart, series),
        renderer::ExportOptions::document(1.0),
    )
    .block_on()
    .unwrap()
}

fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            std::path::Path::new(&path).join(format!("{name}.png")),
            renderer::encode_png(image).unwrap(),
        )
        .unwrap();
    }
}

/// Data x → panel pixel x (linear or logarithmic), times the export scale.
fn data_x_to_px(chart: &Chart, value: f64) -> f32 {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let t = match cfg.bottom_x.scale {
        AxisScale::Linear => (value - cfg.bottom_x.min) / (cfg.bottom_x.max - cfg.bottom_x.min),
        AxisScale::Logarithmic => {
            (value.log10() - cfg.bottom_x.min.log10())
                / (cfg.bottom_x.max.log10() - cfg.bottom_x.min.log10())
        }
        AxisScale::Probability(_) => unreachable!("these charts are linear or logarithmic"),
    };
    da.x as f32 + t as f32 * da.width as f32
}

/// Data y → panel pixel y (screen y grows downward), linear or logarithmic.
fn data_y_to_px(chart: &Chart, value: f64) -> f32 {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let t = match cfg.left_y.scale {
        AxisScale::Linear => (value - cfg.left_y.min) / (cfg.left_y.max - cfg.left_y.min),
        AxisScale::Logarithmic => {
            (value.log10() - cfg.left_y.min.log10())
                / (cfg.left_y.max.log10() - cfg.left_y.min.log10())
        }
        AxisScale::Probability(_) => unreachable!("these charts are linear or logarithmic"),
    };
    (da.y + da.height) as f32 - t as f32 * da.height as f32
}

fn pixel(img: &RasterImage, x: f32, y: f32) -> [u8; 4] {
    let (x, y) = (x.round() as u32, y.round() as u32);
    assert!(
        x < img.width && y < img.height,
        "probe ({x}, {y}) outside the image"
    );
    let i = ((y * img.width + x) * 4) as usize;
    img.rgba[i..i + 4].try_into().unwrap()
}

fn is_red(p: &[u8]) -> bool {
    p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
}
fn is_blue(p: &[u8]) -> bool {
    p[3] > 16 && p[2] > 120 && p[0] < 90 && p[1] < 90
}
fn is_empty(p: &[u8]) -> bool {
    p[3] <= 16
}

/// A probe at data coordinates, asserting one colour class with a readable
/// failure. A 2 px stroke can straddle the rounded pixel row, so a probe
/// that expects line ink (red or blue) accepts the 3×3 neighbourhood; fill
/// and emptiness probes are exact.
fn expect(img: &RasterImage, chart: &Chart, x: f64, y: f64, pred: fn(&[u8]) -> bool, what: &str) {
    let (px, py) = (data_x_to_px(chart, x), data_y_to_px(chart, y));
    let p = pixel(img, px, py);
    if pred(&p) {
        return;
    }
    let line_ink = std::ptr::fn_addr_eq(pred, is_red as fn(&[u8]) -> bool)
        || std::ptr::fn_addr_eq(pred, is_blue as fn(&[u8]) -> bool);
    if line_ink {
        let hit =
            (-1..=1).any(|dy| (-1..=1).any(|dx| pred(&pixel(img, px + dx as f32, py + dy as f32))));
        assert!(
            hit,
            "{what}: no line ink around data ({x}, {y}), centre is {p:?}"
        );
        return;
    }
    panic!("{what}: pixel at data ({x}, {y}) is {p:?}");
}

fn upload(r: &mut Renderer, prefix: &str, xs: &[f64], ys: &[f64]) -> (String, String) {
    let (x, y) = (format!("{prefix}_x"), format!("{prefix}_y"));
    for (id, data) in [(&x, xs), (&y, ys)] {
        let _ = r.remove_column(id);
        r.add_column(id, &col_f64(data.to_vec())).unwrap();
    }
    (x, y)
}

const XS: [f64; 5] = [1.0, 2.0, 3.0, 4.0, 5.0];
const YS: [f64; 5] = [3.0, -2.0, 4.0, -1.0, 2.0];

/// Vertical stems run from the baseline to each point, up or down, at 1x
/// and 2x, and nowhere beside or past their point; the marker sits on top.
#[test]
fn vertical_stems_run_from_the_baseline_to_each_point() {
    let mut r = renderer();
    let (x, y) = upload(&mut r, "v", &XS, &YS);
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(0.0, 6.0);
    chart.set_y_range(-3.0, 5.0);
    let series = [stem_series(
        "stems",
        &x,
        &y,
        red_stem(0.0, CategoryBarDirection::Vertical, 3.0),
    )];
    for scale in [1.0_f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("vertical_stems_{scale}x"), &img);
        let at = |x: f64, y: f64| {
            pixel(
                &img,
                data_x_to_px(&chart, x) * scale,
                data_y_to_px(&chart, y) * scale,
            )
        };
        for (&px, &py) in XS.iter().zip(&YS) {
            for f in [0.1, 0.5, 0.8] {
                assert!(
                    is_red(&at(px, py * f)),
                    "{scale}x: stem {px} at {f}: {:?}",
                    at(px, py * f)
                );
            }
            assert!(
                is_blue(&at(px, py)),
                "{scale}x: marker {px}: {:?}",
                at(px, py)
            );
            let past = py + py.signum() * 0.4;
            assert!(is_empty(&at(px, past)), "{scale}x: past point {px}");
            assert!(
                is_empty(&at(px + 0.25, py * 0.5)),
                "{scale}x: beside stem {px}"
            );
        }
    }
}

/// Horizontal stems (a lollipop) run along x from the baseline, the column
/// roles unchanged.
#[test]
fn horizontal_stems_run_along_x() {
    let mut r = renderer();
    let values = [2.0, 4.0, 1.0];
    let rows = [0.0, 1.0, 2.0];
    let (x, y) = upload(&mut r, "h", &values, &rows);
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(-1.0, 5.0);
    chart.set_y_range(-1.0, 3.0);
    let series = [stem_series(
        "lollipop",
        &x,
        &y,
        red_stem(0.0, CategoryBarDirection::Horizontal, 3.0),
    )];
    let img = export(&mut r, &chart, &series, 1.0);
    save("horizontal_stems", &img);
    for (&value, &row) in values.iter().zip(&rows) {
        expect(&img, &chart, value * 0.5, row, is_red, "stem body");
        expect(&img, &chart, value, row, is_blue, "marker");
        expect(&img, &chart, -0.5, row, is_empty, "before the baseline");
        expect(
            &img,
            &chart,
            value * 0.5,
            row + 0.4,
            is_empty,
            "beside the stem",
        );
    }
}

/// On a logarithmic value axis a baseline at or below zero has no
/// position: the stem starts at the axis minimum instead of vanishing.
/// A non-positive point draws no stem, as it draws no marker.
#[test]
fn a_log_axis_clamps_a_nonpositive_baseline_to_its_minimum() {
    let mut r = renderer();
    let (x, y) = upload(&mut r, "log", &[1.0, 2.0, 3.0], &[10.0, 100.0, -5.0]);
    let mut chart = bare_chart(400, 300);
    chart.config_mut().left_y.scale = AxisScale::Logarithmic;
    chart.set_x_range(0.0, 4.0);
    chart.set_y_range(1.0, 1000.0);
    let series = [stem_series(
        "log",
        &x,
        &y,
        red_stem(0.0, CategoryBarDirection::Vertical, 3.0),
    )];
    let img = export(&mut r, &chart, &series, 1.0);
    save("log_stems", &img);
    expect(&img, &chart, 1.0, 1.5, is_red, "near the axis minimum");
    expect(&img, &chart, 1.0, 5.0, is_red, "stem body");
    expect(&img, &chart, 2.0, 50.0, is_red, "stem body");
    expect(&img, &chart, 1.0, 30.0, is_empty, "past the point");
    expect(
        &img,
        &chart,
        3.0,
        2.0,
        is_empty,
        "a non-positive point has no stem",
    );
}

/// A dashed stem starts its dash at the baseline: the first dash is ink
/// from the baseline end, and gaps appear along the stem.
#[test]
fn a_dashed_stem_starts_its_dash_at_the_baseline() {
    let mut r = renderer();
    let (x, y) = upload(&mut r, "dash", &[1.0], &[10.0]);
    let mut chart = bare_chart(200, 400);
    chart.set_x_range(0.0, 2.0);
    chart.set_y_range(0.0, 10.0);
    let mut stem = red_stem(0.0, CategoryBarDirection::Vertical, 3.0);
    stem.line_style = LineStylePreset::Dash;
    let series = [stem_series("dashed", &x, &y, stem)];
    let img = export(&mut r, &chart, &series, 1.0);
    save("dashed_stem", &img);
    let px = data_x_to_px(&chart, 1.0);
    let base = data_y_to_px(&chart, 0.0);
    let top = data_y_to_px(&chart, 9.0);
    let column: Vec<bool> = (top as i32..base as i32)
        .map(|py| is_red(&pixel(&img, px, py as f32)))
        .collect();
    let ink = column.iter().filter(|&&on| on).count();
    assert!(
        ink > column.len() / 4 && ink < column.len() * 9 / 10,
        "{ink} of {}",
        column.len()
    );
    assert!(
        is_red(&pixel(&img, px, base - 2.0)),
        "the first dash starts at the baseline"
    );
}

fn pick_at(
    r: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    at: (f32, f32),
) -> Option<renderer::PickedData> {
    let _ = r.enable_gpu_picking();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let rect = chart.config().chart_area.0;
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0, at.1],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: rect.width,
            height: rect.height,
        },
        display_scale: 1.0,
        max_distance_px: 0.0,
    };
    let picked = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    picked
}

fn point_index(picked: &Option<renderer::PickedData>) -> Option<usize> {
    match picked {
        Some(renderer::PickedData::Point { point_index, .. }) => Some(*point_index),
        _ => None,
    }
}

/// A cursor on a stem picks its point, on the stem body, at its baseline
/// end and on the marker; beside the stem it picks nothing. A cursor in a
/// dash gap misses with a zero reach.
#[test]
fn a_stem_picks_as_its_point() {
    let mut r = renderer();
    let (x, y) = upload(&mut r, "pick", &XS, &YS);
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(0.0, 6.0);
    chart.set_y_range(-3.0, 5.0);
    let series = [stem_series(
        "stems",
        &x,
        &y,
        red_stem(0.0, CategoryBarDirection::Vertical, 3.0),
    )];
    let px = |x: f64, y: f64| (data_x_to_px(&chart, x), data_y_to_px(&chart, y));
    for (i, (&sx, &sy)) in XS.iter().zip(&YS).enumerate() {
        for f in [0.05, 0.5, 0.9] {
            let picked = pick_at(&mut r, &chart, &series, px(sx, sy * f));
            assert_eq!(point_index(&picked), Some(i), "stem {i} at {f}: {picked:?}");
        }
        let beside = pick_at(&mut r, &chart, &series, px(sx + 0.3, sy * 0.5));
        assert!(beside.is_none(), "beside stem {i}: {beside:?}");
    }
    // A dashed stem misses inside a gap, which a solid one would hit.
    let (x, y) = upload(&mut r, "gap", &[1.0], &[4.0]);
    let mut dashed = red_stem(0.0, CategoryBarDirection::Vertical, 3.0);
    dashed.line_style = LineStylePreset::Dash;
    let series = [stem_series("dashed", &x, &y, dashed)];
    let base = px(1.0, 0.0);
    // `Dash` is 8 on, 4 off: 10 px up from the baseline is in the first gap.
    let gap = pick_at(&mut r, &chart, &series, (base.0, base.1 - 10.0));
    assert!(gap.is_none(), "in a dash gap: {gap:?}");
    let on = pick_at(&mut r, &chart, &series, (base.0, base.1 - 4.0));
    assert_eq!(point_index(&on), Some(0), "on the first dash");
}

/// The baseline joins the value axis' auto-fit, like a histogram's: stems
/// above a baseline of -10 fit an axis reaching it.
#[test]
fn the_baseline_joins_the_auto_fit() {
    let mut r = renderer();
    let (x, y) = upload(&mut r, "fit", &[1.0, 2.0], &[5.0, 8.0]);
    let series = [stem_series(
        "fit",
        &x,
        &y,
        red_stem(-10.0, CategoryBarDirection::Vertical, 2.0),
    )];
    let mut chart = bare_chart(400, 300);
    chart
        .auto_fit_layer_union(r.pool(), renderer::LayerRef::Base, &series, 0.0)
        .unwrap();
    assert!(
        chart.config().left_y.min <= -10.0,
        "{}",
        chart.config().left_y.min
    );
    assert!(chart.config().left_y.max >= 8.0);
    // Horizontal stems fit x instead.
    let horizontal = [stem_series(
        "fit",
        &x,
        &y,
        red_stem(-10.0, CategoryBarDirection::Horizontal, 2.0),
    )];
    let mut chart = bare_chart(400, 300);
    chart
        .auto_fit_layer_union(r.pool(), renderer::LayerRef::Base, &horizontal, 0.0)
        .unwrap();
    assert!(
        chart.config().bottom_x.min <= -10.0,
        "{}",
        chart.config().bottom_x.min
    );
    assert!(
        chart.config().left_y.min >= 5.0 - 1e-9,
        "{}",
        chart.config().left_y.min
    );
}

/// Streaming is not extended to stems (rule 8): an export of a stem series
/// over streamed columns is refused with an explicit error. The stream
/// pass's own refusal (`STREAMED_STEM_REASON`) is checked in the renderer's
/// unit tests.
#[test]
fn a_streamed_stem_series_is_refused() {
    let mut r = renderer();
    let streamed = |id: &str| renderer::StreamColumn {
        id: id.into(),
        len: 3,
        revision: 1,
        encoding: renderer::StreamEncoding::ScalarF32,
        replay: renderer::StreamReplay::RandomAccess,
        statistics: renderer::StreamStatistics::Unknown,
    };
    r.register_streamed_columns(vec![streamed("st_x"), streamed("st_y")])
        .unwrap();
    let mut chart = bare_chart(200, 200);
    chart.set_x_range(0.0, 3.0);
    chart.set_y_range(0.0, 3.0);
    let series = [stem_series(
        "streamed",
        "st_x",
        "st_y",
        red_stem(0.0, CategoryBarDirection::Vertical, 2.0),
    )];
    let refused = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .err()
        .map(|error| error.to_string());
    assert!(
        refused
            .as_deref()
            .is_some_and(|e| e.contains("not GPU resident")),
        "{refused:?}"
    );
}

/// Every row draws a stem (rule 10): a million and one rows draw, and the
/// last one picks. The time is printed, never capped.
#[test]
fn a_million_stems_draw_and_the_last_picks() {
    let mut r = Renderer::try_new(
        RendererDevice::new(shared_device().0, shared_device().1).expect("figgy poll thread"),
        wgpu::TextureFormat::Bgra8Unorm,
        256 * 1024 * 1024,
    )
    .expect("renderer init");
    let n = 1_000_001;
    let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let ys: Vec<f64> = (0..n).map(|i| 1.0 + (i % 7) as f64).collect();
    let (x, y) = upload(&mut r, "big", &xs, &ys);
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(n as f64 - 11.0, n as f64);
    chart.set_y_range(0.0, 9.0);
    let series = [stem_series(
        "big",
        &x,
        &y,
        red_stem(0.0, CategoryBarDirection::Vertical, 3.0),
    )];
    let start = std::time::Instant::now();
    let img = export(&mut r, &chart, &series, 1.0);
    eprintln!("1,000,001 stems drawn in {:?}", start.elapsed());
    let last = (n - 1) as f64;
    let ly = ys[n - 1];
    assert!(
        is_red(&pixel(
            &img,
            data_x_to_px(&chart, last - 0.0) - 1.0,
            data_y_to_px(&chart, ly * 0.5)
        )) || is_red(&pixel(
            &img,
            data_x_to_px(&chart, last),
            data_y_to_px(&chart, ly * 0.5)
        )),
        "the last stem draws"
    );
    let start = std::time::Instant::now();
    let picked = pick_at(
        &mut r,
        &chart,
        &series,
        (data_x_to_px(&chart, last), data_y_to_px(&chart, ly * 0.5)),
    );
    eprintln!("1,000,001 stems picked in {:?}", start.elapsed());
    assert_eq!(point_index(&picked), Some(n - 1));
}

/// Rule 6: stems draw on an overlay against its own y axis, in a subplot
/// against the subplot's axes, and across a broken value axis with the
/// part inside the gap hidden.
#[test]
fn stems_draw_on_overlays_subplots_and_broken_axes() {
    use renderer::config::{AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark};
    let mut r = renderer();
    let (x, y) = upload(&mut r, "ov", &[2.0], &[80.0]);
    // Overlay: the base y axis is 0..10, the overlay's 0..100, so the stem
    // reaches 80 on the overlay scale (80 % of the data area).
    let mut chart = bare_chart(300, 300);
    chart.set_x_range(0.0, 4.0);
    chart.set_y_range(0.0, 10.0);
    let mut overlay = renderer::OverlayLayer::default();
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 100.0;
    overlay.series = vec!["ov".into()];
    chart.config_mut().overlays.push(overlay);
    let series = [stem_series(
        "ov",
        &x,
        &y,
        red_stem(0.0, CategoryBarDirection::Vertical, 3.0),
    )];
    let img = export_document(&mut r, &chart, &series);
    save("overlay_stems", &img);
    let da = chart.config().data_area().unwrap();
    let px = data_x_to_px(&chart, 2.0);
    let at = |share: f32| (da.y + da.height) as f32 - share * da.height as f32;
    assert!(is_red(&pixel(&img, px, at(0.6))), "overlay stem body");
    assert!(
        is_empty(&pixel(&img, px, at(0.9))),
        "past the overlay point"
    );

    // Subplot: the stem draws in the subplot's own data area.
    let mut doc = bare_chart(600, 300);
    doc.set_x_range(0.0, 4.0);
    doc.set_y_range(0.0, 10.0);
    let mut sub = doc.config().clone();
    sub.chart_area = ChartArea(Rect {
        x: 300,
        y: 0,
        width: 300,
        height: 300,
    });
    sub.left_y.min = 0.0;
    sub.left_y.max = 100.0;
    doc.config_mut().chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 300,
        height: 300,
    });
    doc.config_mut().subplots.push(renderer::Subplot {
        config: sub.clone(),
        series: vec!["ov".into()],
        link_x: None,
        link_y: None,
        marker: None,
    });
    let img = export_document(&mut r, &doc, &series);
    save("subplot_stems", &img);
    let sda = sub.data_area().unwrap();
    let sx = sda.x as f32 + 0.5 * sda.width as f32;
    let sy = |share: f32| (sda.y + sda.height) as f32 - share * sda.height as f32;
    assert!(is_red(&pixel(&img, sx, sy(0.6))), "subplot stem body");
    assert!(
        is_empty(&pixel(&img, sx, sy(0.9))),
        "past the subplot point"
    );

    // A Continuous cut of 40..60 hides the stem inside the gap: the column
    // under the stem runs red, then empty, then red again.
    let mut broken = bare_chart(300, 300);
    broken.set_x_range(0.0, 4.0);
    broken.set_y_range(0.0, 100.0);
    broken.config_mut().left_y.breaks = Some(AxisBreaks {
        ranges: vec![AxisBreak::new(40.0, 60.0)],
        style: AxisBreakStyle::Continuous,
        gap_px: 24.0,
        mark: BreakMark::None,
        frame_each_panel: false,
        sizing: Default::default(),
    });
    let img = export(&mut r, &broken, &series, 1.0);
    save("broken_stems", &img);
    let da = broken.config().data_area().unwrap();
    let px = data_x_to_px(&broken, 2.0);
    let column: Vec<bool> = (da.y + 2..da.y + da.height - 2)
        .map(|py| is_red(&pixel(&img, px, py as f32)))
        .collect();
    let runs = column.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(
        runs >= 3,
        "a red run, the gap, a red run: {runs} transitions"
    );
}

/// Rule 6: a line offset member's stems and baseline shift with it, so each
/// member keeps its own zero. Member 1, shifted by 5, draws its stem from 5
/// to its shifted point, not from 0.
#[test]
fn a_line_offset_member_shifts_its_stems_and_baseline() {
    let mut r = renderer();
    let (x0, y0) = upload(&mut r, "lo0", &[1.0, 1.5], &[2.0, 2.0]);
    let (x1, y1) = upload(&mut r, "lo1", &[3.0, 3.5], &[2.0, 2.0]);
    let member = |id: &str, x: &str, y: &str| {
        let mut series = stem_series(id, x, y, red_stem(0.0, CategoryBarDirection::Vertical, 3.0));
        if let DataRenderType::Xy(xy) = &mut series.render_type {
            xy.line = Some(renderer::DataLineStyleConfig {
                line_color: BLUE,
                line_width: 1.0,
                ..renderer::DataLineStyleConfig::default()
            });
        }
        series
    };
    let series = [member("lo0", &x0, &y0), member("lo1", &x1, &y1)];
    let mut chart = bare_chart(300, 300);
    chart.set_x_range(0.0, 4.0);
    chart.set_y_range(0.0, 10.0);
    chart.config_mut().line_offsets = Some(renderer::data_config::LineOffsets {
        series: vec!["lo0".into(), "lo1".into()],
        step_y: 5.0,
        step_x: 0.0,
    });
    let img = export_document(&mut r, &chart, &series);
    save("line_offset_stems", &img);
    expect(&img, &chart, 1.0, 1.0, is_red, "member 0's stem from 0");
    expect(
        &img,
        &chart,
        3.0,
        6.0,
        is_red,
        "member 1's stem from its shifted baseline",
    );
    expect(
        &img,
        &chart,
        3.0,
        3.0,
        is_empty,
        "member 1 draws nothing below its baseline",
    );
}
