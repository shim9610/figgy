//! End-to-end pixel verification of the hand-drawn ("sketch") render mode
//! (design SSoT: docs/SKETCH_DESIGN.md §7 V2) using ONLY the public
//! renderer API and the headless `export_panel_rgba` path.
//!
//! House pattern (mirrors the in-crate GPU tests): every test builds its own
//! instance/adapter/device and early-returns when no usable adapter exists.
//!
//! Attribution strategy: in sketch mode the black deco layer (axes, ticks,
//! titles) wobbles too, so a whole-image diff cannot attribute divergence to
//! the GPU data layer. Each data series therefore gets a saturated primary
//! color (line = red, scatter = green, errorbar = blue) on disjoint y-bands,
//! and assertions filter pixels by color class — black deco ink never
//! matches any class predicate.
//!
//! §7 V2 item 6 (JSON default round-trip) is intentionally NOT duplicated
//! here: it already exists as `sketch_serde_tests` in
//! `crates/model/src/config.rs` (config_without_sketch_key_deserializes_to_none,
//! empty_sketch_object_yields_all_defaults,
//! partial_sketch_object_fills_remaining_defaults,
//! none_sketch_serializes_without_key).

use std::sync::Arc;

use renderer::config::SketchOptions;
use renderer::data::Column;
use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, Color, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
    DataScatterStyleConfig, ErrorRef, RasterImage, Renderer, RendererDevice, ScatterShape,
    SeriesConfig,
};

// ───────────────────────── plumbing / fixtures ─────────────────────────

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

/// Headless renderer, or `None` when this environment has no GPU adapter
/// (the caller early-returns — same skip pattern as the in-crate GPU tests).
fn try_renderer() -> Option<Renderer> {
    let inst = create_instance();
    let Ok(adapter) = request_adapter(&inst) else { return None };
    let Ok((device, queue)) = request_device(&adapter) else { return None };
    Some(
        Renderer::try_new(
            RendererDevice::new(Arc::new(device), Arc::new(queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .expect("renderer init"),
    )
}

/// Chart with grid and legend off: the only non-data ink left is the black
/// axis/tick/title deco, which no color-class predicate matches.
fn bare_chart(width: u32, height: u32) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect { x: 0, y: 0, width, height });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    Chart::new(config)
}

const RED: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
const GREEN: Color = Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
const BLUE: Color = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };

fn line_series(id: &str, x: &str, y: &str, color: Color, style: LineStylePreset) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: style,
                line_color: color,
                line_width: 2.0,
            },
        },
    }
}

fn scatter_series(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: color,
                point_shape: ScatterShape::CircleFilled,
                point_size: 7.0,
            },
        },
    }
}

/// Y-errorbar series whose anchor markers are fully transparent — the bars
/// (and caps) are the only ink, so `color` isolates the errorbar pipeline.
fn errorbar_series(id: &str, x: &str, y: &str, err: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::ScatterErrorbarY {
            scatter: DataScatterStyleConfig {
                point_color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
                point_shape: ScatterShape::CircleFilled,
                point_size: 3.0,
            },
            err_y: ErrorRef::Symmetric { column: err.into() },
            err_style: DataErrorBarStyleConfig {
                error_bar_color: color,
                error_bar_width: 2.0,
                error_bar_cap_size: 4.0,
                cap_width: 2.0,
            },
        },
    }
}

/// Registers all combined-fixture columns on `r` and returns a 640×480 chart
/// plus three series on disjoint y-bands:
///   red sine line   y ∈ [0.5, 1.1]
///   green markers   y ∈ [-0.2, 0.2]
///   blue errorbars  y ∈ [-1.05, -0.35]
/// Bands are ≥ 30 px apart so a ±few-px wobble can never mix color classes.
fn build_combined(r: &mut Renderer) -> (Chart, Vec<SeriesConfig>) {
    let n = 257;
    let lx: Vec<f64> = (0..n).map(|i| i as f64 * 6.28 / (n - 1) as f64).collect();
    let ly: Vec<f64> = lx.iter().map(|x| 0.8 + 0.3 * x.sin()).collect();
    let m = 33;
    let sx: Vec<f64> = (0..m).map(|i| 0.15 + i as f64 * 6.0 / (m - 1) as f64).collect();
    let sy: Vec<f64> = sx.iter().map(|x| 0.2 * (x * 1.3).cos()).collect();
    let ey: Vec<f64> = sx.iter().map(|x| -0.7 + 0.2 * (x * 0.9).sin()).collect();
    let ee: Vec<f64> = vec![0.15; m];
    r.add_column("lx", &col_f64(lx)).unwrap();
    r.add_column("ly", &col_f64(ly)).unwrap();
    r.add_column("sx", &col_f64(sx)).unwrap();
    r.add_column("sy", &col_f64(sy)).unwrap();
    r.add_column("ey", &col_f64(ey)).unwrap();
    r.add_column("ee", &col_f64(ee)).unwrap();
    // Required by every errorbar series: zero-fill padding for the unused
    // error dimension (the web wrapper registers this automatically).
    r.add_column("__zero", &col_f64(vec![0.0; m])).unwrap();

    let mut chart = bare_chart(640, 480);
    chart.set_x_range(0.0, 6.5);
    chart.set_y_range(-1.2, 1.2);

    let series = vec![
        line_series("line", "lx", "ly", RED, LineStylePreset::Solid),
        scatter_series("scatter", "sx", "sy", GREEN),
        errorbar_series("errbar", "sx", "ey", "ee", BLUE),
    ];
    (chart, series)
}

// ───────────────────────── pixel-class helpers ─────────────────────────

fn is_red(p: &[u8]) -> bool {
    p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
}
fn is_green(p: &[u8]) -> bool {
    p[3] > 16 && p[1] > 120 && p[0] < 90 && p[2] < 90
}
fn is_blue(p: &[u8]) -> bool {
    p[3] > 16 && p[2] > 120 && p[0] < 90 && p[1] < 90
}

/// (label, predicate, minimum expected ink pixels) per data-layer primitive.
const CLASSES: [(&str, fn(&[u8]) -> bool, usize); 3] = [
    ("line(red)", is_red, 300),
    ("scatter(green)", is_green, 200),
    ("errorbar(blue)", is_blue, 300),
];

fn count_class(img: &RasterImage, pred: fn(&[u8]) -> bool) -> usize {
    img.rgba.chunks_exact(4).filter(|p| pred(p)).count()
}

/// Pixels that belong to `pred`'s color class in at least one image AND whose
/// RGBA bytes differ between the two — i.e. that class's ink actually moved.
fn class_diff(a: &RasterImage, b: &RasterImage, pred: fn(&[u8]) -> bool) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height), "image dims");
    a.rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(pa, pb)| (pred(pa) || pred(pb)) && pa != pb)
        .count()
}

/// Byte-equality assert that prints a one-line locus instead of two
/// megabyte-sized vectors on failure.
fn assert_bytes_eq(a: &RasterImage, b: &RasterImage, what: &str) {
    assert_eq!((a.width, a.height), (b.width, b.height), "{what}: dims");
    if a.rgba != b.rgba {
        let i = a.rgba.iter().zip(&b.rgba).position(|(x, y)| x != y).unwrap();
        let px = i / 4;
        let (x, y) = (px % a.width as usize, px / a.width as usize);
        panic!(
            "{what}: first byte diff at offset {i} (pixel {x},{y} ch {}): {} vs {}",
            i % 4,
            a.rgba[i],
            b.rgba[i]
        );
    }
}

// ───────────────────────────── the tests ─────────────────────────────

/// §7 V2-1 — divergence. Same data, sketch `None` vs `Some(default)`:
/// the whole image differs AND each data primitive's own color class
/// differs (so line, scatter and errorbar wobble are each individually
/// proven, not masked by the deco-layer wobble). A vanish guard pins the
/// sketch ink quantity to the same order of magnitude as precise ink —
/// "differs because the series disappeared" cannot pass. Then the same
/// precise-vs-sketch comparison is repeated on single-series charts for
/// unambiguous per-pipeline attribution.
#[test]
fn sketch_diverges_from_precise() {
    let Some(mut r) = try_renderer() else { return };
    let (mut chart, series) = build_combined(&mut r);

    let img_p = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    chart.config_mut().sketch = Some(SketchOptions::default());
    let img_s = r.export_panel_rgba(&chart, &series, 1.0).unwrap();

    assert!(
        img_p.rgba != img_s.rgba,
        "sketch(default) export is pixel-identical to precise export"
    );
    for (name, pred, min_ink) in CLASSES {
        let (cp, cs) = (count_class(&img_p, pred), count_class(&img_s, pred));
        assert!(cp > min_ink, "{name}: precise ink missing ({cp} px)");
        assert!(cs > min_ink, "{name}: sketch ink missing ({cs} px)");
        assert!(
            cs * 2 >= cp && cp * 2 >= cs,
            "{name}: sketch ink quantity implausible (precise {cp} px vs sketch {cs} px)"
        );
        let d = class_diff(&img_p, &img_s, pred);
        assert!(d > 0, "{name}: ink identical under sketch — wobble not applied");
    }

    // Single-series isolation: one chart per primitive, fresh precise/sketch
    // pair each, diff restricted to that primitive's color class.
    let singles: [(SeriesConfig, fn(&[u8]) -> bool, &str, usize); 3] = [
        (line_series("only_l", "lx", "ly", RED, LineStylePreset::Solid), is_red, "line-only", 300),
        (scatter_series("only_s", "sx", "sy", GREEN), is_green, "scatter-only", 200),
        (errorbar_series("only_e", "sx", "ey", "ee", BLUE), is_blue, "errorbar-only", 300),
    ];
    for (cfg, pred, name, min_ink) in singles {
        let mut chart = bare_chart(640, 480);
        chart.set_x_range(0.0, 6.5);
        chart.set_y_range(-1.2, 1.2);
        let one = [cfg];
        let p = r.export_panel_rgba(&chart, &one, 1.0).unwrap();
        chart.config_mut().sketch = Some(SketchOptions::default());
        let s = r.export_panel_rgba(&chart, &one, 1.0).unwrap();
        let (cp, cs) = (count_class(&p, pred), count_class(&s, pred));
        assert!(cp > min_ink, "{name}: precise ink missing ({cp} px)");
        assert!(cs > min_ink, "{name}: sketch ink missing ({cs} px)");
        let d = class_diff(&p, &s, pred);
        assert!(d > 0, "{name}: ink identical under sketch — wobble not applied");
    }
}

/// §7 V2-2 — determinism. Identical config + data must export
/// byte-identical pixels: twice from the same renderer (no per-frame
/// randomness / time dependence) and once from a freshly built renderer
/// with the same column insertion order (no hidden instance state).
#[test]
fn sketch_is_deterministic() {
    let Some(mut r) = try_renderer() else { return };
    let (mut chart, series) = build_combined(&mut r);
    chart.config_mut().sketch = Some(SketchOptions::default());

    let a = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let b = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert_bytes_eq(&a, &b, "same renderer, repeated sketch export");

    let Some(mut r2) = try_renderer() else { return };
    let (mut chart2, series2) = build_combined(&mut r2);
    chart2.config_mut().sketch = Some(SketchOptions::default());
    let c = r2.export_panel_rgba(&chart2, &series2, 1.0).unwrap();
    assert_bytes_eq(&a, &c, "fresh renderer instance, identical inputs");
}

/// §7 V2-3 — seed separation. seed 0 vs seed 1 must change the pixels,
/// and must change them within EVERY data primitive's color class (each
/// GPU sketch entry consumes the global seed).
#[test]
fn sketch_seed_separates() {
    let Some(mut r) = try_renderer() else { return };
    let (mut chart, series) = build_combined(&mut r);

    chart.config_mut().sketch = Some(SketchOptions { seed: 0, ..SketchOptions::default() });
    let s0 = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    chart.config_mut().sketch = Some(SketchOptions { seed: 1, ..SketchOptions::default() });
    let s1 = r.export_panel_rgba(&chart, &series, 1.0).unwrap();

    assert!(s0.rgba != s1.rgba, "seed 0 and seed 1 produced identical exports");
    for (name, pred, _) in CLASSES {
        let d = class_diff(&s0, &s1, pred);
        assert!(d > 0, "{name}: seed change did not affect this primitive's ink");
    }
}

/// §7 V2-4 — amplitude bound. A horizontal line (the wobble displacement is
/// perpendicular to the path, i.e. purely vertical here) drawn with
/// amplitude A = 3 px must keep its ink rows within the PRECISE export's ink
/// row span ± (A + 2 px AA slack). Scanned only across data-area-interior
/// columns so axis-adjacent end caps and deco ink stay out of the window
/// (deco is black and color-filtered anyway).
#[test]
fn sketch_amplitude_is_bounded() {
    let Some(mut r) = try_renderer() else { return };
    let n = 129;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let ys: Vec<f64> = vec![0.5; n];
    r.add_column("hx", &col_f64(xs)).unwrap();
    r.add_column("hy", &col_f64(ys)).unwrap();

    let mut chart = bare_chart(640, 480);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    let series = [line_series("h", "hx", "hy", RED, LineStylePreset::Solid)];

    let img_p = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let da = chart.config().data_area().unwrap().0;
    let (x0, x1) = ((da.x + 8) as usize, (da.x + da.width - 8) as usize);

    let red_rows = |img: &RasterImage| -> Option<(usize, usize)> {
        let w = img.width as usize;
        let mut span: Option<(usize, usize)> = None;
        for y in 0..img.height as usize {
            for x in x0..x1 {
                if is_red(&img.rgba[(y * w + x) * 4..(y * w + x) * 4 + 4]) {
                    span = Some(match span {
                        Some((lo, hi)) => (lo.min(y), hi.max(y)),
                        None => (y, y),
                    });
                }
            }
        }
        span
    };

    let (pmin, pmax) = red_rows(&img_p).expect("precise horizontal line drew no ink");

    const AMP: f32 = 3.0;
    chart.config_mut().sketch =
        Some(SketchOptions { amplitude_px: AMP, ..SketchOptions::default() });
    let img_s = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let (smin, smax) = red_rows(&img_s).expect("sketch horizontal line drew no ink");

    // The sketch pass must still wobble (guards a vacuous bound pass)…
    assert!(
        class_diff(&img_p, &img_s, is_red) > 0,
        "sketch line ink identical to precise — amplitude test is vacuous"
    );
    // …but never beyond amplitude + AA slack.
    let slack = (AMP + 2.0).ceil() as i64; // = 5 px
    assert!(
        smin as i64 >= pmin as i64 - slack && smax as i64 <= pmax as i64 + slack,
        "wobble exceeded amplitude bound: precise rows {pmin}..={pmax}, \
         sketch rows {smin}..={smax}, allowed slack ±{slack} px (A={AMP} + 2 AA)"
    );
}

/// §7 V2-5 — NaN gap preservation. A horizontal line with a NaN band over
/// x ∈ [0.40, 0.45) exported in sketch mode must leave the gap's interior
/// columns ink-free (caps may narrow the gap by ~line-width, so the probe
/// stays 5 px inside each edge), while both sides still draw.
#[test]
fn sketch_preserves_nan_gaps() {
    let Some(mut r) = try_renderer() else { return };
    let n = 4001;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let ys: Vec<f64> = xs
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            if (0.40..0.45).contains(&x) {
                f64::NAN
            } else {
                // Tiny ripple keeps the column non-degenerate (min < max).
                0.5 + if i % 2 == 0 { 0.0008 } else { -0.0008 }
            }
        })
        .collect();
    r.add_column("gx", &col_f64(xs)).unwrap();
    r.add_column("gy", &col_f64(ys)).unwrap();

    let mut chart = bare_chart(800, 400);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(-0.1, 1.1);
    chart.config_mut().sketch = Some(SketchOptions::default());
    let series = [line_series("gap", "gx", "gy", RED, LineStylePreset::Solid)];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();

    let (w, h) = (img.width as usize, img.height as usize);
    let red_cols: Vec<usize> = (0..w)
        .filter(|&x| (0..h).any(|y| is_red(&img.rgba[(y * w + x) * 4..(y * w + x) * 4 + 4])))
        .collect();
    assert!(!red_cols.is_empty(), "sketch line drew no ink at all");

    let da = chart.config().data_area().unwrap().0;
    let to_px = |v: f64| da.x as f64 + v * da.width as f64;
    let (gap_a, gap_b) = (to_px(0.40), to_px(0.45));
    let (probe_a, probe_b) = ((gap_a + 5.0) as usize, (gap_b - 5.0) as usize);
    let leaked: Vec<&usize> =
        red_cols.iter().filter(|&&x| x >= probe_a && x <= probe_b).collect();
    assert!(
        leaked.is_empty(),
        "sketch bridged the NaN gap: line ink at columns {leaked:?} inside probe \
         ({probe_a}..{probe_b})"
    );
    assert!(
        red_cols.iter().any(|&x| (x as f64) < gap_a - 2.0),
        "left of NaN gap missing"
    );
    assert!(
        red_cols.iter().any(|&x| (x as f64) > gap_b + 2.0),
        "right of NaN gap missing"
    );
}

/// §7 V2 extension — dash composition. The same sine line exported in
/// sketch mode as Solid vs Dash([8,4]): the dashed variant must draw
/// strictly less ink (gaps exist → < 85 % of solid) but still most of the
/// curve (> 25 % of solid — the wobbled dash phase didn't erase the line).
#[test]
fn sketch_composes_with_dash() {
    let Some(mut r) = try_renderer() else { return };
    let n = 512;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 * 6.28 / (n - 1) as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
    r.add_column("dx", &col_f64(xs)).unwrap();
    r.add_column("dy", &col_f64(ys)).unwrap();

    let mut chart = bare_chart(640, 480);
    chart.set_x_range(0.0, 6.5);
    chart.set_y_range(-1.2, 1.2);
    chart.config_mut().sketch = Some(SketchOptions::default());

    let solid = [line_series("ds", "dx", "dy", RED, LineStylePreset::Solid)];
    let img_solid = r.export_panel_rgba(&chart, &solid, 1.0).unwrap();
    let dashed = [line_series("dd", "dx", "dy", RED, LineStylePreset::Dash)];
    let img_dash = r.export_panel_rgba(&chart, &dashed, 1.0).unwrap();

    let (cs, cd) = (count_class(&img_solid, is_red), count_class(&img_dash, is_red));
    assert!(cs > 300, "sketch solid line missing ({cs} px)");
    assert!(cd > 0, "sketch+dash drew nothing — dash erased the line");
    assert!(
        cd * 20 < cs * 17,
        "sketch+dash shows no gaps: dashed {cd} px vs solid {cs} px (expected < 85 %)"
    );
    assert!(
        cd * 4 > cs,
        "sketch+dash nearly empty: dashed {cd} px vs solid {cs} px (expected > 25 %)"
    );
}
