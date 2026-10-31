//! Pixel verification of area fills and step interpolation on Cartesian line
//! series, through the public `export_panel_rgba` path.
//!
//! House pattern (see `sketch_render.rs`): every test builds its own renderer
//! on a shared device, GPU initialization failures fail the suite, and
//! assertions sample pixels by saturated colour class — the line is red, the
//! fill is an explicit opaque green (or a translucent one where blending is
//! the point) and the background is transparent — so black axis ink never
//! matches a predicate. Expected positions are computed through the chart's
//! own data area and axis range, the same mapping the GPU transform uses.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::AxisScale;
use renderer::data::Column;
use renderer::data_config::{LineFill, LineInterpolation};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, Color, DataLineStyleConfig, DataRenderType, FiggyError, RasterImage, Renderer,
    RendererDevice, SeriesConfig,
};

// ───────────────────────── plumbing / fixtures ─────────────────────────

fn col_f64(data: Vec<f64>) -> Column<f64> {
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

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init")
}

/// Chart with grid, legend and axis margins off: the data area is the whole
/// panel bar the axis lines, so pixel probes map straight through the axis
/// range.
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
const GREEN: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
const BLUE: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

fn line(
    fill: Option<LineFill>,
    fill_color: Option<Color>,
    interpolation: LineInterpolation,
) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: LineStylePreset::Solid,
        line_color: RED,
        line_width: 2.0,
        fill,
        fill_color,
        interpolation,
    }
}

fn series(id: &str, x: &str, y: &str, line: DataLineStyleConfig) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Line { line },
    }
}

fn filled(id: &str, x: &str, y: &str, fill: LineFill, color: Color) -> SeriesConfig {
    series(
        id,
        x,
        y,
        line(Some(fill), Some(color), LineInterpolation::Linear),
    )
}

/// Data x → panel pixel x, through the same data area and axis range the
/// renderer draws with (linear axes).
fn data_x_to_px(chart: &Chart, value: f64) -> f32 {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let t = (value - cfg.bottom_x.min) / (cfg.bottom_x.max - cfg.bottom_x.min);
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
fn is_green(p: &[u8]) -> bool {
    p[3] > 16 && p[1] > 120 && p[0] < 90 && p[2] < 90
}
fn is_blue(p: &[u8]) -> bool {
    p[3] > 16 && p[2] > 120 && p[0] < 90 && p[1] < 90
}
fn is_empty(p: &[u8]) -> bool {
    p[3] <= 16
}

fn count(img: &RasterImage, pred: fn(&[u8]) -> bool) -> usize {
    img.rgba.chunks_exact(4).filter(|p| pred(p)).count()
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

fn two_point_chart(r: &mut Renderer, prefix: &str) -> (Chart, String, String) {
    let x = format!("{prefix}_x");
    let y = format!("{prefix}_y");
    r.add_column(&x, &col_f64(vec![0.0, 1.0])).unwrap();
    r.add_column(&y, &col_f64(vec![0.2, 1.0])).unwrap();
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(-0.2, 1.2);
    chart.set_y_range(-0.2, 1.2);
    (chart, x, y)
}

// ───────────────────────────── area fill ─────────────────────────────

/// The fill occupies exactly the region between the line and the baseline:
/// green below the line, nothing above it, nothing below the baseline — and
/// the line itself stays red because every fill is drawn before every body.
#[test]
fn area_fill_paints_between_the_line_and_its_baseline_under_the_line() {
    let mut r = renderer();
    r.add_column("af_x", &col_f64(vec![0.0, 1.0, 2.0])).unwrap();
    r.add_column("af_y", &col_f64(vec![0.5, 1.0, 0.5])).unwrap();
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(-0.5, 2.5);
    chart.set_y_range(-0.5, 1.5);
    let series = [filled(
        "area",
        "af_x",
        "af_y",
        LineFill::ToBaseline(0.0),
        GREEN,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();

    expect(&img, &chart, 1.0, 0.5, is_green, "under the peak");
    expect(&img, &chart, 0.5, 0.3, is_green, "under the rising edge");
    expect(&img, &chart, 1.5, 0.3, is_green, "under the falling edge");
    expect(&img, &chart, 1.0, 1.25, is_empty, "above the line");
    expect(&img, &chart, 0.5, 0.9, is_empty, "above the rising edge");
    expect(&img, &chart, 1.0, -0.25, is_empty, "below the baseline");
    expect(
        &img,
        &chart,
        1.0,
        1.0,
        is_red,
        "the line is drawn over its fill",
    );
    assert!(
        count(&img, is_green) > 2000,
        "fill ink: {}",
        count(&img, is_green)
    );

    // A nonzero baseline moves the bottom edge; the area is not anchored to 0.
    let series = [filled(
        "area",
        "af_x",
        "af_y",
        LineFill::ToBaseline(0.4),
        GREEN,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    expect(
        &img,
        &chart,
        1.0,
        0.45,
        is_green,
        "just above the raised baseline",
    );
    expect(
        &img,
        &chart,
        1.0,
        0.2,
        is_empty,
        "below the raised baseline",
    );
}

/// The default fill colour is the line colour at a quarter of its alpha, so a
/// series with `fill: Some(..)` and no `fill_color` paints a translucent red
/// area under an opaque red line.
#[test]
fn default_fill_colour_is_the_line_colour_at_quarter_alpha() {
    let mut r = renderer();
    let (chart, x, y) = two_point_chart(&mut r, "dfc");
    let series = [series(
        "area",
        &x,
        &y,
        line(
            Some(LineFill::ToBaseline(0.0)),
            None,
            LineInterpolation::Linear,
        ),
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let under = pixel(&img, data_x_to_px(&chart, 0.5), data_y_to_px(&chart, 0.25));
    assert!(
        under[0] > 200 && under[1] < 40 && under[2] < 40,
        "fill keeps the line hue: {under:?}"
    );
    let expected_alpha = (255.0 * DataLineStyleConfig::DEFAULT_FILL_ALPHA).round() as i32;
    assert!(
        (i32::from(under[3]) - expected_alpha).abs() <= 2,
        "fill alpha {} should be ≈ {expected_alpha}",
        under[3]
    );
    expect(&img, &chart, 0.5, 0.6, is_red, "the opaque line");
}

/// Fill between two series: the area follows the referenced series' own y
/// values index for index, with nothing above the upper line or below the
/// lower one.
#[test]
fn fill_between_two_series_follows_the_referenced_series() {
    let mut r = renderer();
    r.add_column("fb_x", &col_f64(vec![0.0, 1.0, 2.0])).unwrap();
    r.add_column("fb_hi", &col_f64(vec![0.8, 1.0, 0.8]))
        .unwrap();
    r.add_column("fb_lo", &col_f64(vec![0.2, 0.6, 0.2]))
        .unwrap();
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(-0.5, 2.5);
    chart.set_y_range(-0.5, 1.5);
    let series = [
        series(
            "lower",
            "fb_x",
            "fb_lo",
            DataLineStyleConfig {
                line_color: BLUE,
                ..line(None, None, LineInterpolation::Linear)
            },
        ),
        filled(
            "upper",
            "fb_x",
            "fb_hi",
            LineFill::ToSeries("lower".into()),
            GREEN,
        ),
    ];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();

    expect(
        &img,
        &chart,
        1.0,
        0.8,
        is_green,
        "between the lines at the peak",
    );
    expect(
        &img,
        &chart,
        0.5,
        0.6,
        is_green,
        "between the lines on the slope",
    );
    expect(&img, &chart, 1.0, 0.3, is_empty, "below the lower line");
    expect(&img, &chart, 1.0, 1.2, is_empty, "above the upper line");
    expect(
        &img,
        &chart,
        0.0,
        0.0,
        is_empty,
        "the baseline is not involved",
    );
    expect(
        &img,
        &chart,
        1.0,
        1.0,
        is_red,
        "the upper line over the fill",
    );
    expect(
        &img,
        &chart,
        1.0,
        0.6,
        is_blue,
        "the lower line, drawn after the fill, stays visible",
    );
}

/// Draw order is "every fill, then every body": a later series' fill goes
/// over an earlier series' fill but never over its line, and no automatic
/// sorting reorders the series.
#[test]
fn fills_composite_in_series_order_under_every_line() {
    let mut r = renderer();
    r.add_column("fo_x", &col_f64(vec![0.0, 1.0])).unwrap();
    r.add_column("fo_low", &col_f64(vec![0.4, 0.4])).unwrap();
    r.add_column("fo_high", &col_f64(vec![0.9, 0.9])).unwrap();
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(-0.2, 1.2);
    chart.set_y_range(-0.2, 1.2);
    // The low series comes first with a blue fill; the high series' green
    // fill covers the whole low area but not the low line.
    let series = [
        filled("low", "fo_x", "fo_low", LineFill::ToBaseline(0.0), BLUE),
        filled("high", "fo_x", "fo_high", LineFill::ToBaseline(0.0), GREEN),
    ];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    expect(
        &img,
        &chart,
        0.5,
        0.2,
        is_green,
        "the later fill composites over the earlier one",
    );
    expect(
        &img,
        &chart,
        0.5,
        0.4,
        is_red,
        "the earlier series' line is not covered by a later fill",
    );
    expect(&img, &chart, 0.5, 0.65, is_green, "between the two lines");

    // Reversed declaration order reverses the composite: blue wins below.
    let series = [series[1].clone(), series[0].clone()];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    expect(
        &img,
        &chart,
        0.5,
        0.2,
        is_blue,
        "declaration order decides the composite",
    );
}

/// A gap in the line (a non-finite sample) is a gap in the fill, exactly
/// where it is in the line, on both the baseline and the between-series
/// forms.
#[test]
fn fill_breaks_where_the_line_breaks() {
    let mut r = renderer();
    r.add_column("gap_x", &col_f64(vec![0.0, 1.0, 2.0, 3.0]))
        .unwrap();
    r.add_column("gap_y", &col_f64(vec![1.0, f64::NAN, 1.0, 1.0]))
        .unwrap();
    r.add_column("gap_lo", &col_f64(vec![0.2, 0.2, 0.2, 0.2]))
        .unwrap();
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(-0.5, 3.5);
    chart.set_y_range(-0.5, 1.5);
    for fill in [LineFill::ToBaseline(0.0), LineFill::ToSeries("lo".into())] {
        let series = [
            series(
                "lo",
                "gap_x",
                "gap_lo",
                DataLineStyleConfig {
                    line_color: BLUE,
                    ..line(None, None, LineInterpolation::Linear)
                },
            ),
            filled("gap", "gap_x", "gap_y", fill.clone(), GREEN),
        ];
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        expect(
            &img,
            &chart,
            2.5,
            0.6,
            is_green,
            "the intact segment is filled",
        );
        expect(
            &img,
            &chart,
            0.5,
            0.6,
            is_empty,
            "the segment into the gap is not",
        );
        expect(
            &img,
            &chart,
            1.5,
            0.6,
            is_empty,
            "nor the segment out of it",
        );
        expect(&img, &chart, 2.5, 1.0, is_red, "the intact segment's line");
        expect(&img, &chart, 0.5, 1.0, is_empty, "no line into the gap");
    }
}

/// Fills assume an x-monotonic series. Where a series doubles back, the
/// trapezoids overlap and the translucent fill blends twice — a documented
/// constraint, asserted rather than masked.
#[test]
fn non_monotonic_series_blends_overlapping_trapezoids_twice() {
    let mut r = renderer();
    // x runs 0 → 2 → 1: the segment back to 1 overlaps [1, 2].
    r.add_column("nm_x", &col_f64(vec![0.0, 2.0, 1.0])).unwrap();
    r.add_column("nm_y", &col_f64(vec![1.0, 1.0, 1.0])).unwrap();
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(-0.5, 2.5);
    chart.set_y_range(-0.5, 1.5);
    let half_green = Color { a: 0.5, ..GREEN };
    let series = [filled(
        "back",
        "nm_x",
        "nm_y",
        LineFill::ToBaseline(0.0),
        half_green,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let once = pixel(&img, data_x_to_px(&chart, 0.5), data_y_to_px(&chart, 0.5));
    let twice = pixel(&img, data_x_to_px(&chart, 1.5), data_y_to_px(&chart, 0.5));
    assert!(is_green(&once) && is_green(&twice), "{once:?} / {twice:?}");
    assert!(
        (i32::from(once[3]) - 128).abs() <= 3,
        "single coverage keeps the fill alpha: {once:?}"
    );
    assert!(
        (i32::from(twice[3]) - 191).abs() <= 3,
        "double coverage blends twice (1 − 0.5²): {twice:?}"
    );
}

/// On a logarithmic y axis a baseline at or below zero has no position. The
/// line's samples would be masked there; the baseline is instead clamped to
/// the axis minimum so the area still reaches the bottom of the axis.
#[test]
fn log_axis_clamps_a_nonpositive_baseline_to_the_axis_minimum() {
    let mut r = renderer();
    r.add_column("log_x", &col_f64(vec![0.0, 1.0])).unwrap();
    r.add_column("log_y", &col_f64(vec![10.0, 100.0])).unwrap();
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(-0.2, 1.2);
    chart.set_y_range(1.0, 1000.0);
    chart.config_mut().left_y.scale = AxisScale::Logarithmic;
    for baseline in [0.0, -5.0] {
        let series = [filled(
            "log",
            "log_x",
            "log_y",
            LineFill::ToBaseline(baseline),
            GREEN,
        )];
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        expect(
            &img,
            &chart,
            0.5,
            2.0,
            is_green,
            "just above the axis minimum",
        );
        expect(&img, &chart, 0.5, 20.0, is_green, "under the line");
        expect(&img, &chart, 0.5, 300.0, is_empty, "above the line");
        expect(&img, &chart, 0.5, 31.6, is_red, "the line itself");
    }
    // A positive baseline inside the range is placed logarithmically.
    let series = [filled(
        "log",
        "log_x",
        "log_y",
        LineFill::ToBaseline(10.0),
        GREEN,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    expect(
        &img,
        &chart,
        0.5,
        20.0,
        is_green,
        "between the line and the baseline at 10",
    );
    expect(
        &img,
        &chart,
        0.5,
        3.0,
        is_empty,
        "below a positive baseline",
    );
}

/// The fill-between declaration is validated before any GPU work: a different
/// x column, an unknown series and a self reference are explicit errors.
#[test]
fn fill_between_rejects_mismatched_unknown_and_self_references() {
    let mut r = renderer();
    r.add_column("rj_x", &col_f64(vec![0.0, 1.0])).unwrap();
    r.add_column("rj_x2", &col_f64(vec![0.0, 1.0])).unwrap();
    r.add_column("rj_y", &col_f64(vec![0.5, 0.5])).unwrap();
    r.add_column("rj_lo", &col_f64(vec![0.1, 0.1])).unwrap();
    let chart = bare_chart(200, 150);
    let other_x = [
        series(
            "lo",
            "rj_x2",
            "rj_lo",
            line(None, None, LineInterpolation::Linear),
        ),
        filled("hi", "rj_x", "rj_y", LineFill::ToSeries("lo".into()), GREEN),
    ];
    let unknown = [filled(
        "hi",
        "rj_x",
        "rj_y",
        LineFill::ToSeries("missing".into()),
        GREEN,
    )];
    let itself = [filled(
        "hi",
        "rj_x",
        "rj_y",
        LineFill::ToSeries("hi".into()),
        GREEN,
    )];
    for (series, needle) in [
        (&other_x[..], "same x column"),
        (&unknown[..], "not in this chart"),
        (&itself[..], "reference the series itself"),
    ] {
        match r.export_panel_rgba(&chart, series, 1.0) {
            Err(FiggyError::InvalidSeriesConfig { series_id, reason }) => {
                assert_eq!(series_id, "hi");
                assert!(reason.contains(needle), "{reason}");
            }
            other => panic!(
                "expected an explicit rejection, got {:?}",
                other.map(|_| ())
            ),
        }
        match r.register_chart(chart.config().clone(), series.to_vec()) {
            Err(FiggyError::InvalidSeriesConfig { reason, .. }) => {
                assert!(reason.contains(needle), "{reason}");
            }
            other => panic!(
                "register_chart must reject too, got {:?}",
                other.map(|_| ())
            ),
        }
    }
    // The same declarations with a shared x column are fine.
    let ok = [
        series(
            "lo",
            "rj_x",
            "rj_lo",
            line(None, None, LineInterpolation::Linear),
        ),
        filled("hi", "rj_x", "rj_y", LineFill::ToSeries("lo".into()), GREEN),
    ];
    r.export_panel_rgba(&chart, &ok, 1.0).unwrap();
}

/// A fill or a step mode on a streamed (nonresident) series is refused with
/// an explicit error by the streaming capability inspector, before any chunk
/// is requested.
#[test]
fn streamed_series_reject_fill_and_step_interpolation() {
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
    let chart = bare_chart(200, 150);
    for (name, line_cfg) in [
        (
            "fill",
            line(
                Some(LineFill::ToBaseline(0.0)),
                None,
                LineInterpolation::Linear,
            ),
        ),
        ("step", line(None, None, LineInterpolation::StepPost)),
    ] {
        let id = r
            .register_chart(
                chart.config().clone(),
                vec![series("streamed", "st_x", "st_y", line_cfg)],
            )
            .unwrap();
        match r.inspect_streaming_capability(id) {
            Err(FiggyError::InvalidSeriesConfig { series_id, reason }) => {
                assert_eq!(series_id, "streamed", "{name}");
                assert!(reason.contains("GPU-resident"), "{name}: {reason}");
            }
            other => panic!("{name}: expected rejection, got {:?}", other.map(|_| ())),
        }
        r.remove_chart(id).unwrap();
    }
    // A plain streamed line is still accepted.
    let id = r
        .register_chart(
            chart.config().clone(),
            vec![series(
                "streamed",
                "st_x",
                "st_y",
                line(None, None, LineInterpolation::Linear),
            )],
        )
        .unwrap();
    r.inspect_streaming_capability(id).unwrap();
}

// ──────────────────────────── step lines ────────────────────────────

/// Each step mode draws its own axis-aligned path between the two samples
/// (0, 0.2) → (1, 1.0), and never the straight chord.
#[test]
fn step_modes_draw_axis_aligned_pieces_instead_of_the_chord() {
    let mut r = renderer();
    let (chart, x, y) = two_point_chart(&mut r, "st");
    let render = |r: &mut Renderer, mode| {
        let series = [series("step", &x, &y, line(None, None, mode))];
        r.export_panel_rgba(&chart, &series, 1.0).unwrap()
    };

    let linear = render(&mut r, LineInterpolation::Linear);
    expect(&linear, &chart, 0.5, 0.6, is_red, "linear: the chord");
    expect(&linear, &chart, 0.5, 0.2, is_empty, "linear: no run at y_a");
    expect(&linear, &chart, 0.5, 1.0, is_empty, "linear: no run at y_b");

    let post = render(&mut r, LineInterpolation::StepPost);
    expect(&post, &chart, 0.5, 0.2, is_red, "step-post: run at y_a");
    expect(&post, &chart, 1.0, 0.6, is_red, "step-post: riser at x_b");
    expect(&post, &chart, 0.5, 0.6, is_empty, "step-post: no chord");
    expect(
        &post,
        &chart,
        0.5,
        1.0,
        is_empty,
        "step-post: no run at y_b",
    );
    expect(
        &post,
        &chart,
        0.0,
        0.6,
        is_empty,
        "step-post: no riser at x_a",
    );

    let pre = render(&mut r, LineInterpolation::StepPre);
    expect(&pre, &chart, 0.0, 0.6, is_red, "step-pre: riser at x_a");
    expect(&pre, &chart, 0.5, 1.0, is_red, "step-pre: run at y_b");
    expect(&pre, &chart, 0.5, 0.6, is_empty, "step-pre: no chord");
    expect(&pre, &chart, 0.5, 0.2, is_empty, "step-pre: no run at y_a");
    expect(
        &pre,
        &chart,
        1.0,
        0.6,
        is_empty,
        "step-pre: no riser at x_b",
    );

    let mid = render(&mut r, LineInterpolation::StepMid);
    expect(
        &mid,
        &chart,
        0.25,
        0.2,
        is_red,
        "step-mid: run at y_a before the midpoint",
    );
    expect(
        &mid,
        &chart,
        0.5,
        0.6,
        is_red,
        "step-mid: riser at the midpoint",
    );
    expect(
        &mid,
        &chart,
        0.75,
        1.0,
        is_red,
        "step-mid: run at y_b after the midpoint",
    );
    expect(
        &mid,
        &chart,
        0.75,
        0.2,
        is_empty,
        "step-mid: no run at y_a after the midpoint",
    );
    expect(
        &mid,
        &chart,
        0.25,
        1.0,
        is_empty,
        "step-mid: no run at y_b before the midpoint",
    );
    expect(&mid, &chart, 0.25, 0.6, is_empty, "step-mid: no chord");
}

/// A fill follows the stepped line: the area under a step-post run stops at
/// `y_a` across the whole segment, under step-pre it reaches `y_b`, and under
/// step-mid it switches at the data-space midpoint.
#[test]
fn step_fill_follows_the_stepped_line() {
    let mut r = renderer();
    let (chart, x, y) = two_point_chart(&mut r, "sf");
    let render = |r: &mut Renderer, mode| {
        let series = [series(
            "step",
            &x,
            &y,
            line(Some(LineFill::ToBaseline(0.0)), Some(GREEN), mode),
        )];
        r.export_panel_rgba(&chart, &series, 1.0).unwrap()
    };
    let linear = render(&mut r, LineInterpolation::Linear);
    expect(
        &linear,
        &chart,
        0.5,
        0.4,
        is_green,
        "linear: under the chord",
    );
    expect(
        &linear,
        &chart,
        0.5,
        0.8,
        is_empty,
        "linear: above the chord",
    );

    let post = render(&mut r, LineInterpolation::StepPost);
    expect(
        &post,
        &chart,
        0.5,
        0.1,
        is_green,
        "step-post: under the run at y_a",
    );
    expect(
        &post,
        &chart,
        0.5,
        0.4,
        is_empty,
        "step-post: nothing above y_a before x_b",
    );

    let pre = render(&mut r, LineInterpolation::StepPre);
    expect(
        &pre,
        &chart,
        0.5,
        0.8,
        is_green,
        "step-pre: under the run at y_b",
    );
    expect(
        &pre,
        &chart,
        0.5,
        0.4,
        is_green,
        "step-pre: the whole column is filled",
    );

    let mid = render(&mut r, LineInterpolation::StepMid);
    expect(
        &mid,
        &chart,
        0.25,
        0.1,
        is_green,
        "step-mid: under y_a before the midpoint",
    );
    expect(
        &mid,
        &chart,
        0.25,
        0.4,
        is_empty,
        "step-mid: nothing above y_a before the midpoint",
    );
    expect(
        &mid,
        &chart,
        0.75,
        0.8,
        is_green,
        "step-mid: under y_b after the midpoint",
    );
}

/// A dashed step line keeps its pattern through the corners: the dash
/// phase follows the stepped arc, so the ink is reduced by the pattern's
/// duty cycle along the whole path, not shattered or lost at the riser.
#[test]
fn dashed_step_lines_keep_their_pattern_along_the_stepped_path() {
    let mut r = renderer();
    r.add_column("ds_x", &col_f64((0..6).map(f64::from).collect()))
        .unwrap();
    r.add_column("ds_y", &col_f64(vec![0.1, 0.9, 0.3, 0.8, 0.2, 0.7]))
        .unwrap();
    let mut chart = bare_chart(480, 320);
    chart.set_x_range(-0.5, 5.5);
    chart.set_y_range(0.0, 1.0);
    for mode in [
        LineInterpolation::StepPre,
        LineInterpolation::StepPost,
        LineInterpolation::StepMid,
    ] {
        let render = |r: &mut Renderer, style| {
            let series = [series(
                "step",
                "ds_x",
                "ds_y",
                DataLineStyleConfig {
                    line_style: style,
                    ..line(None, None, mode)
                },
            )];
            r.export_panel_rgba(&chart, &series, 1.0).unwrap()
        };
        let solid = count(&render(&mut r, LineStylePreset::Solid), is_red);
        let dashed = count(&render(&mut r, LineStylePreset::Dash), is_red);
        assert!(solid > 500, "{mode:?}: solid ink {solid}");
        // Dash [8, 4]: two thirds on. The risers are part of the arc, so
        // they are dashed too; a phase that restarted or stalled at every
        // corner would leave far more or far less ink.
        assert!(
            dashed * 100 > solid * 30 && dashed * 100 < solid * 85,
            "{mode:?}: dashed ink {dashed} vs solid {solid}"
        );
    }
}

/// The export scale has no upper bound of its own: a 16× export of a
/// 400×300 panel is 6400×4800 px with the fill covering 256 times the pixels,
/// and a scale whose target passes the device's texture limit fails with
/// `GpuResourceLimit` instead of being clamped, for PNG, SVG and PDF alike.
#[test]
fn export_scale_is_bounded_by_the_device_texture_limit_only() {
    let mut r = renderer();
    let (chart, x, y) = two_point_chart(&mut r, "s16");
    let series = [series(
        "step",
        &x,
        &y,
        line(
            Some(LineFill::ToBaseline(0.0)),
            Some(GREEN),
            LineInterpolation::StepPost,
        ),
    )];
    let one = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let sixteen = r.export_panel_rgba(&chart, &series, 16.0).unwrap();
    assert_eq!(
        (sixteen.width, sixteen.height),
        (one.width * 16, one.height * 16)
    );
    let (g1, g16) = (count(&one, is_green), count(&sixteen, is_green));
    assert!(
        (g16 as f64 / g1 as f64 / 256.0 - 1.0).abs() < 0.05,
        "fill ink scales by area: {g1} → {g16}"
    );
    let svg = r.export_document_svg(&chart, &series, 16.0).unwrap();
    assert!(
        svg.contains(r#"viewBox="0 0 6400 4800""#),
        "the SVG is 16× too"
    );
    let too_big = 1.0e4;
    for result in [
        r.export_panel_rgba(&chart, &series, too_big).map(|_| ()),
        r.export_document_svg(&chart, &series, too_big).map(|_| ()),
        r.export_document_pdf(&chart, &series, too_big).map(|_| ()),
    ] {
        assert!(
            matches!(result, Err(FiggyError::GpuResourceLimit { .. })),
            "{result:?}"
        );
    }
}

/// The fill and the step geometry scale with the export: a 2× export covers
/// four times the pixels and the same data probes land on the same classes.
#[test]
fn export_scales_fill_and_steps_consistently_at_1x_and_2x() {
    let mut r = renderer();
    let (chart, x, y) = two_point_chart(&mut r, "sc");
    let series = [series(
        "step",
        &x,
        &y,
        line(
            Some(LineFill::ToBaseline(0.0)),
            Some(GREEN),
            LineInterpolation::StepPost,
        ),
    )];
    let one = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let two = r.export_panel_rgba(&chart, &series, 2.0).unwrap();
    assert_eq!((two.width, two.height), (one.width * 2, one.height * 2));
    let (g1, g2) = (count(&one, is_green), count(&two, is_green));
    assert!(
        (g2 as f64 / g1 as f64 - 4.0).abs() < 0.15,
        "fill ink scales by area: {g1} → {g2}"
    );
    let (r1, r2) = (count(&one, is_red), count(&two, is_red));
    assert!(
        r2 > r1 * 3 && r2 < r1 * 5,
        "line ink scales with width and length: {r1} → {r2}"
    );
    for (img, scale) in [(&one, 1.0f32), (&two, 2.0)] {
        let probe = |xv: f64, yv: f64| {
            pixel(
                img,
                data_x_to_px(&chart, xv) * scale,
                data_y_to_px(&chart, yv) * scale,
            )
        };
        assert!(is_green(&probe(0.5, 0.1)), "{scale}x fill under the run");
        assert!(is_red(&probe(0.5, 0.2)), "{scale}x run");
        assert!(is_red(&probe(1.0, 0.6)), "{scale}x riser");
        assert!(is_empty(&probe(0.5, 0.6)), "{scale}x no chord");
    }
    // PNG export goes through the same path and decodes to the same size.
    let png = r.export_panel_png_bytes(&chart, &series, 2.0).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}

/// The same inputs render byte-identically from a fresh renderer, for the
/// fill and for every step mode — the fixed draw order and the GPU-side
/// geometry leave nothing to allocation or timing.
#[test]
fn fills_and_steps_are_deterministic_across_renderers() {
    let build = |r: &mut Renderer| {
        let (chart, x, y) = two_point_chart(r, "det");
        let series = [
            series(
                "step",
                &x,
                &y,
                line(
                    Some(LineFill::ToBaseline(0.0)),
                    None,
                    LineInterpolation::StepMid,
                ),
            ),
            filled("between", &x, &y, LineFill::ToSeries("step".into()), GREEN),
        ];
        r.export_panel_rgba(&chart, &series, 1.0).unwrap()
    };
    let mut a = renderer();
    let first = build(&mut a);
    let mut b = renderer();
    let second = build(&mut b);
    assert_eq!(first.rgba, second.rgba, "fresh renderer, identical inputs");
}

/// The legend mark of a filled series carries a band in the fill colour
/// inside the fixed-width symbol field; an unfilled series shows the rule
/// alone.
#[test]
fn legend_symbol_shows_a_band_for_a_filled_series() {
    let mut r = renderer();
    r.add_column("lg_x", &col_f64(vec![0.0, 1.0])).unwrap();
    r.add_column("lg_y", &col_f64(vec![0.5, 0.5])).unwrap();
    let mut chart = bare_chart(400, 300);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    chart.config_mut().legend.visible = true;
    let render = |r: &mut Renderer, chart: &mut Chart, fill: Option<LineFill>| {
        let series = [series(
            "s",
            "lg_x",
            "lg_y",
            line(fill, Some(BLUE), LineInterpolation::Linear),
        )];
        // The legend's symbol is rebuilt from the series declaration.
        let legend = &mut chart.config_mut().legend;
        legend.content.segments.clear();
        renderer::config::append_legend_entry(
            &mut legend.content,
            renderer::legend::series_symbol_segments(&series[0], None),
            "filled",
        );
        r.export_panel_rgba(chart, &series, 1.0).unwrap()
    };
    let plain = render(&mut r, &mut chart, None);
    let banded = render(&mut r, &mut chart, Some(LineFill::ToBaseline(0.0)));
    // The data area ink is identical between the two apart from the blue fill
    // under the line; the legend band is blue too, so compare blue ink above
    // the line, where only the legend can put it.
    let line_y = data_y_to_px(&chart, 0.5).round() as u32;
    let blue_above = |img: &RasterImage| {
        (0..line_y.saturating_sub(4))
            .flat_map(|y| (0..img.width).map(move |x| (x, y)))
            .filter(|&(x, y)| is_blue(&pixel(img, x as f32, y as f32)))
            .count()
    };
    assert_eq!(blue_above(&plain), 0, "no band without a fill");
    assert!(
        blue_above(&banded) > 20,
        "the filled series' legend mark has a band"
    );
}

// A line on a logarithmic axis sits at its value's logarithm at every
// magnitude. Drivers that blended the log flag with `mix` lost the logarithm
// past 2^24 and drew 1e8 at the floor; llvmpipe never did, so this pins the
// result and `shader_consistency` pins the early return (SHADER_COMMON §3).
#[test]
fn log_axis_places_lines_at_every_magnitude() {
    for magnitude in [1e-20f64, 1e-6, 1.0, 1e6, 1e12, 1e16, 1e30] {
        let mut r = renderer();
        r.add_column("lx", &col_f64(vec![0.0, 1.0])).unwrap();
        r.add_column("ly", &col_f64(vec![magnitude, magnitude]))
            .unwrap();
        let mut chart = bare_chart(200, 160);
        chart.config_mut().left_y.scale = AxisScale::Logarithmic;
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(magnitude / 100.0, magnitude * 100.0);
        let s = vec![series(
            "log_line",
            "lx",
            "ly",
            line(None, None, LineInterpolation::Linear),
        )];
        let img = r.export_panel_rgba(&chart, &s, 1.0).unwrap();
        let x = data_x_to_px(&chart, 0.5);
        let y = data_y_to_px(&chart, magnitude);
        let p = pixel(&img, x, y);
        assert!(
            p[0] > 200 && p[1] < 80 && p[2] < 80,
            "{magnitude:e}: no line at ({x}, {y}) on the log axis: {p:?}"
        );
    }
}

/// The values a pool column holds at `indices`, read back from the GPU pool
/// the way a host's selection details read them (hi + lo pairs).
fn pool_values(r: &Renderer, column: &str, indices: &[usize]) -> Vec<f64> {
    let handle = r.pool().handle_for(column).expect("resident column");
    let buffer = r.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("selected values"),
        size: indices.len() as u64 * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = r.device().create_command_encoder(&Default::default());
    for (slot, index) in indices.iter().enumerate() {
        encoder.copy_buffer_to_buffer(
            r.pool().buffer(),
            handle.offset + *index as u64 * 8,
            &buffer,
            slot as u64 * 8,
            8,
        );
    }
    r.queue().submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    r.device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let values = slice
        .get_mapped_range()
        .unwrap()
        .chunks_exact(8)
        .map(|pair| {
            f64::from(f32::from_le_bytes(pair[..4].try_into().unwrap()))
                + f64::from(f32::from_le_bytes(pair[4..].try_into().unwrap()))
        })
        .collect();
    buffer.unmap();
    values
}

/// Offset spectra (#50): a member is drawn shifted, but a pick on the
/// shifted trace names the source index, and the values a host reads for it
/// are the column's own, unshifted; the column statistics and an export of
/// the same document without the group are unchanged.
#[test]
fn offset_lines_report_unshifted_values() {
    use renderer::data_config::LineOffsets;
    let mut r = renderer();
    r.enable_gpu_picking().unwrap();
    let xs = vec![1.0, 3.0, 5.0, 7.0, 9.0];
    let ys = vec![2.0, 3.5, 2.5, 4.0, 3.0];
    r.add_column("x", &col_f64(xs.clone())).unwrap();
    r.add_column("y", &col_f64(ys.clone())).unwrap();
    let mut chart = bare_chart(240, 180);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 12.0);
    let plain = chart.config().clone();
    chart.config_mut().line_offsets = Some(LineOffsets {
        series: vec!["low".into(), "high".into()],
        step_y: 5.0,
        step_x: 0.5,
    });
    let declared = [
        series("low", "x", "y", line(None, None, LineInterpolation::Linear)),
        series(
            "high",
            "x",
            "y",
            line(None, None, LineInterpolation::Linear),
        ),
    ];
    let id = r
        .register_chart(chart.config().clone(), declared.to_vec())
        .unwrap();
    let image = r.export_document_rgba(&chart, &declared, 1.0).unwrap();
    for index in 0..xs.len() {
        // `high` is drawn half a unit right and five up.
        let (x, y) = (
            data_x_to_px(&chart, xs[index] + 0.5),
            data_y_to_px(&chart, ys[index] + 5.0),
        );
        assert!(
            is_red(&pixel(&image, x, y)),
            "point {index} is not drawn shifted"
        );
        let request = renderer::GpuPickRequest {
            canvas_position_px: [x, y],
            display_panel_px: chart.config().chart_area.0,
            display_scale: 1.0,
            max_distance_px: 2.0,
        };
        let hit = pollster::block_on(r.pick_chart(id, request).unwrap().resolve())
            .unwrap()
            .expect("the shifted point picks");
        assert_eq!((hit.series_id.as_str(), hit.point_index), ("high", index));
        let values = pool_values(&r, "x", &[index])
            .into_iter()
            .chain(pool_values(&r, "y", &[index]))
            .collect::<Vec<_>>();
        assert_eq!(values, [xs[index], ys[index]], "point {index}");
        // The unshifted place of the point is not `high`'s.
        let unshifted = renderer::GpuPickRequest {
            canvas_position_px: [
                data_x_to_px(&chart, xs[index]),
                data_y_to_px(&chart, ys[index]),
            ],
            ..request
        };
        let low = pollster::block_on(r.pick_chart(id, unshifted).unwrap().resolve())
            .unwrap()
            .expect("the first member is unshifted");
        assert_eq!((low.series_id.as_str(), low.point_index), ("low", index));
    }
    let slot = r.pool().slot("y").expect("resident column");
    assert_eq!((slot.min, slot.max), (2.0, 4.0));
    assert_eq!(chart.config().left_y.max, plain.left_y.max);
}

/// Offset spectra have no member cap (#107): 40 members, past the old 32,
/// each draw shifted at 1x and 2x, and a pick on a member past the old cap
/// names that member and the source index.
#[test]
fn forty_offset_members_draw_and_pick() {
    use renderer::data_config::LineOffsets;
    const MEMBERS: usize = 40;
    const STEP: f64 = 2.0;
    let mut r = renderer();
    r.enable_gpu_picking().unwrap();
    let xs = vec![1.0, 3.0, 5.0, 7.0, 9.0];
    let ys = vec![2.0, 3.5, 2.5, 4.0, 3.0];
    r.add_column("x", &col_f64(xs.clone())).unwrap();
    r.add_column("y", &col_f64(ys.clone())).unwrap();
    let mut chart = bare_chart(240, 640);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 5.0 + STEP * MEMBERS as f64);
    let ids: Vec<String> = (0..MEMBERS).map(|k| format!("m{k}")).collect();
    chart.config_mut().line_offsets = Some(LineOffsets {
        series: ids.clone(),
        step_y: STEP,
        step_x: 0.0,
    });
    let declared: Vec<SeriesConfig> = ids
        .iter()
        .map(|id| series(id, "x", "y", line(None, None, LineInterpolation::Linear)))
        .collect();
    let id = r
        .register_chart(chart.config().clone(), declared.clone())
        .unwrap();
    for scale in [1.0f32, 2.0] {
        let image = r.export_document_rgba(&chart, &declared, scale).unwrap();
        for k in [0, 31, 32, MEMBERS - 1] {
            let shift = STEP * k as f64;
            for index in [0, 3] {
                let (x, y) = (
                    data_x_to_px(&chart, xs[index]) * scale,
                    data_y_to_px(&chart, ys[index] + shift) * scale,
                );
                assert!(
                    is_red(&pixel(&image, x, y)),
                    "{scale}x: member {k} point {index} is not drawn shifted"
                );
            }
        }
    }
    for k in [32, MEMBERS - 1] {
        let shift = STEP * k as f64;
        let index = 3;
        let request = renderer::GpuPickRequest {
            canvas_position_px: [
                data_x_to_px(&chart, xs[index]),
                data_y_to_px(&chart, ys[index] + shift),
            ],
            display_panel_px: chart.config().chart_area.0,
            display_scale: 1.0,
            max_distance_px: 2.0,
        };
        let hit = pollster::block_on(r.pick_chart(id, request).unwrap().resolve())
            .unwrap()
            .expect("the shifted member picks");
        assert_eq!(
            (hit.series_id.as_str(), hit.point_index),
            (ids[k].as_str(), index)
        );
    }
}
