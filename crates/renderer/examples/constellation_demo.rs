//! Milkyway demo — line element as a star chain over a nebula ribbon.
//! Exports parameter-sweep PNGs for visual review.
//!
//! Run: `cargo run -p figgy-renderer --example constellation_demo`
//! Output: `target/constellation_demo/*.png`

use renderer::color::Color;
use renderer::config::{DrawStyle, MilkywayOptions};
use renderer::data_config::{ScatterShape, SeriesConfig};
use renderer::theme::SubplotScope;
#[path = "support/looks.rs"]
mod looks;
use renderer::{Chart, ExportOptions, ExportSource, Renderer};

/// The deep-space backdrop the exports clear to.
const SPACE: Color = Color::from_rgb8(11, 15, 23);

/// Fit the chart's margins to its labels, render it on the backdrop and
/// write it as a PNG.
fn export(r: &mut Renderer, chart: &mut Chart, series: &[SeriesConfig], path: &str) {
    chart.fit_layout();
    let img = r
        .export_rgba_blocking(
            ExportSource::chart(chart, series),
            ExportOptions::panel(1.0).with_clear(SPACE),
        )
        .expect("export");
    let ink = img
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] > 30 || p[1] > 35 || p[2] > 45)
        .count();
    img.save_png(path).expect("write");
    println!("wrote {path} (bright px: {ink})");
}

fn line_series(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
    SeriesConfig::line(id, x, y).color(color).width(2.0)
}

fn build_chart(style: DrawStyle) -> Chart {
    // The night-sky chrome over the backdrop the export clears to, with no
    // legend. The demo sweeps the draw style, which it writes into the
    // theme; the series keep the colours each case picks.
    let mut theme = looks::milkyway();
    theme.chrome.draw_style = style;
    let mut chart = Chart::sized(960, 620);
    theme
        .apply_chrome(chart.config_mut(), &[], SubplotScope::All)
        .expect("a chart without series takes any theme");
    chart.config_mut().legend.visible = false;

    let mut chart = chart
        .with_title("Milkyway — line as star chain")
        .with_x_title("t")
        .with_y_title("value");
    chart.set_x_range(-0.03, 1.03);
    chart.set_y_range(0.0, 100.0);
    chart
}

fn main() {
    let mut r = Renderer::headless_blocking().expect("renderer");

    std::fs::create_dir_all("target/constellation_demo").expect("mkdir");

    // Two smooth curves — does the star chain read as a curve?
    let n = 90;
    let ts: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let warm: Vec<f64> = ts.iter().map(|&t| 62.0 + 26.0 * (t * 5.2).sin()).collect();
    let cool: Vec<f64> = ts
        .iter()
        .map(|&t| 14.0 + 30.0 * t + 9.0 * (t * 8.0 + 1.0).cos())
        .collect();
    r.add_column("t", &ts).unwrap();
    r.add_column("warm", &warm).unwrap();
    r.add_column("cool", &cool).unwrap();

    let series = [
        line_series("nebula_warm", "t", "warm", Color::from_rgb8(255, 142, 92)),
        line_series("nebula_cool", "t", "cool", Color::from_rgb8(96, 168, 255)),
    ];

    let cons = |density: f32, width: f32, intensity: f32, seed: u32| {
        DrawStyle::Milkyway(MilkywayOptions {
            star_density: density,
            ribbon_width_px: width,
            ribbon_intensity: intensity,
            seed,
            ..MilkywayOptions::default()
        })
    };

    let cases = [
        ("main", cons(14.0, 14.0, 0.30, 0)),
        ("density_low", cons(6.0, 14.0, 0.30, 0)),
        ("density_high", cons(28.0, 14.0, 0.30, 0)),
        ("ribbon_thin", cons(14.0, 8.0, 0.15, 0)),
        ("ribbon_thick", cons(14.0, 26.0, 0.50, 0)),
        ("seed7", cons(14.0, 14.0, 0.30, 7)),
        // Bisection debug pair: isolate each pass.
        ("dbg_ribbon_only", cons(0.0, 14.0, 0.30, 0)),
        ("dbg_stars_only", cons(14.0, 14.0, 0.0, 0)),
    ];
    let faint_cases = [
        // Luminosity-function slope sweep: more faint dust per anchor.
        ("faint_default", 14.0, 3.0),
        ("faint_heavy", 26.0, 5.5),
        ("faint_extreme", 40.0, 8.0),
    ];
    for (name, style) in cases {
        export(
            &mut r,
            &mut build_chart(style),
            &series,
            &format!("target/constellation_demo/{name}.png"),
        );
    }
    for (name, density, bias) in faint_cases {
        let style = DrawStyle::Milkyway(MilkywayOptions {
            star_density: density,
            faint_bias: bias,
            ..MilkywayOptions::default()
        });
        export(
            &mut r,
            &mut build_chart(style),
            &series,
            &format!("target/constellation_demo/{name}.png"),
        );
    }

    // ── Per-series decorrelation: same-x spectrum stack. Real spectra are
    // scaled copies on one x grid, so before the Style.series_salt fix every
    // series shared one bright-star pattern (vertically aligned anchors).
    // Control arm: all five series share ONE series_id — identical salt
    // reproduces the correlated look. Real arm: distinct ids.
    let sn = 700;
    let sxs: Vec<f64> = (0..sn)
        .map(|i| 1200.0 + 360.0 * i as f64 / (sn - 1) as f64)
        .collect();
    let peak = |x: f64, c: f64, w: f64| (-((x - c) / w) * ((x - c) / w)).exp();
    let base: Vec<f64> = sxs
        .iter()
        .map(|&x| {
            0.012
                + 0.115 * peak(x, 1242.0, 14.0)
                + 0.16 * peak(x, 1345.0, 22.0)
                + 0.05 * peak(x, 1455.0, 40.0)
                + 0.62 * peak(x, 1585.0, 28.0)
        })
        .collect();
    r.add_column("spec_x", &sxs).unwrap();
    let spec_scales = [1.0, 0.55, 0.10, 0.05, 0.02];
    for (i, k) in spec_scales.iter().enumerate() {
        let ys: Vec<f64> = base.iter().map(|b| b * k).collect();
        r.add_column(format!("spec_{i}"), &ys).unwrap();
    }
    let spec_colors = [
        Color::from_rgb8(110, 200, 255),
        Color::from_rgb8(150, 235, 170),
        Color::from_rgb8(235, 215, 130),
        Color::from_rgb8(255, 150, 100),
        Color::from_rgb8(220, 140, 255),
    ];
    let mut spec_correlated = Vec::new();
    let mut spec_distinct = Vec::new();
    for (i, &color) in spec_colors.iter().enumerate() {
        let y = format!("spec_{i}");
        spec_correlated.push(line_series("spec_same", "spec_x", &y, color));
        spec_distinct.push(line_series(&format!("spec_s{i}"), "spec_x", &y, color));
    }
    let spec_chart = || {
        let mut c = build_chart(DrawStyle::Milkyway(MilkywayOptions::default()));
        c.set_x_range(1195.0, 1565.0);
        c.set_y_range(0.0, 0.72);
        c
    };
    export(
        &mut r,
        &mut spec_chart(),
        &spec_correlated,
        "target/constellation_demo/spectrum_correlated.png",
    );
    export(
        &mut r,
        &mut spec_chart(),
        &spec_distinct,
        "target/constellation_demo/spectrum.png",
    );

    // Resolution invariance: the same chart at a 2× export scale must read
    // as the 1× render upscaled — same star count per data span, same clump
    // structure — NOT a denser, blown-out chain (field report). The scaled
    // config divides star_density and multiplies structure_scale.
    let mut x2 = spec_chart();
    x2.fit_layout();
    r.save_blocking(
        ExportSource::chart(&x2, &spec_distinct),
        ExportOptions::panel(2.0).with_clear(SPACE),
        "target/constellation_demo/spectrum_x2.png",
    )
    .expect("export");
    println!("wrote target/constellation_demo/spectrum_x2.png");

    // Sampling invariance: the same curve sampled coarsely (10 pts) vs
    // densely (400 pts) must get the same star treatment. The old
    // per-segment quad budget made sparse polylines star-thin and deaf to
    // the density knob; the arc-driven indirect pass budgets by total arc.
    let curve = |n: usize, lift: f64| -> (Vec<f64>, Vec<f64>) {
        let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|x| lift + 22.0 * (x * 4.6).sin()).collect();
        (xs, ys)
    };
    let (cx, cy) = curve(10, 70.0);
    let (fx, fy) = curve(400, 30.0);
    r.add_column("coarse_x", &cx).unwrap();
    r.add_column("coarse_y", &cy).unwrap();
    r.add_column("fine_x", &fx).unwrap();
    r.add_column("fine_y", &fy).unwrap();
    let sampling = [
        line_series(
            "samp_coarse",
            "coarse_x",
            "coarse_y",
            Color::from_rgb8(120, 190, 255),
        ),
        line_series(
            "samp_fine",
            "fine_x",
            "fine_y",
            Color::from_rgb8(120, 190, 255),
        ),
    ];
    let style = DrawStyle::Milkyway(MilkywayOptions {
        star_density: 30.0,
        ..MilkywayOptions::default()
    });
    export(
        &mut r,
        &mut build_chart(style),
        &sampling,
        "target/constellation_demo/sampling_match.png",
    );

    // ── Step 2: ringed planets (scatter). Ring angle = ScatterShape. ──

    // Three scatter series, distinct ring angles, chart-scale markers.
    let m = 11;
    let pxs: Vec<f64> = (0..m)
        .map(|i| 0.06 + 0.88 * i as f64 / (m - 1) as f64)
        .collect();
    let pa: Vec<f64> = pxs
        .iter()
        .enumerate()
        .map(|(i, _)| 74.0 + 12.0 * ((i as f64) * 1.7).sin())
        .collect();
    let pb: Vec<f64> = pxs
        .iter()
        .enumerate()
        .map(|(i, _)| 48.0 + 11.0 * ((i as f64) * 2.3 + 1.0).cos())
        .collect();
    let pc: Vec<f64> = pxs
        .iter()
        .enumerate()
        .map(|(i, _)| 21.0 + 9.0 * ((i as f64) * 1.3 + 2.0).sin())
        .collect();
    r.add_column("px", &pxs).unwrap();
    r.add_column("pa", &pa).unwrap();
    r.add_column("pb", &pb).unwrap();
    r.add_column("pc", &pc).unwrap();
    let chart_planets = [
        SeriesConfig::scatter("p_a", "px", "pa")
            .color(Color::from_rgb8(255, 142, 92))
            .marker(ScatterShape::Circle, 13.0),
        SeriesConfig::scatter("p_b", "px", "pb")
            .color(Color::from_rgb8(96, 168, 255))
            .marker(ScatterShape::Triangle, 13.0),
        SeriesConfig::scatter("p_c", "px", "pc")
            .color(Color::from_rgb8(120, 220, 150))
            .marker(ScatterShape::DiamondFilled, 13.0),
    ];
    export(
        &mut r,
        &mut build_chart(DrawStyle::Milkyway(MilkywayOptions::default())),
        &chart_planets,
        "target/constellation_demo/planets_chart.png",
    );

    // Close-up showcase: few big planets to inspect the baked surfaces.
    let bx: Vec<f64> = vec![0.14, 0.38, 0.62, 0.86, 0.26, 0.74];
    let by: Vec<f64> = vec![70.0, 76.0, 68.0, 74.0, 30.0, 26.0];
    r.add_column("bx", &bx).unwrap();
    r.add_column("by", &by).unwrap();
    let big = [SeriesConfig::scatter("big", "bx", "by")
        .color(Color::from_rgb8(255, 170, 110))
        .marker(ScatterShape::Square, 44.0)];
    export(
        &mut r,
        &mut build_chart(DrawStyle::Milkyway(MilkywayOptions::default())),
        &big,
        "target/constellation_demo/planets_big.png",
    );

    // Line + scatter combined: planets riding their own star chain.
    let combo = [SeriesConfig::scatter_line("combo", "px", "pb")
        .color(Color::from_rgb8(255, 142, 92))
        .width(2.0)
        .marker(ScatterShape::Circle, 15.0)];
    export(
        &mut r,
        &mut build_chart(DrawStyle::Milkyway(MilkywayOptions::default())),
        &combo,
        "target/constellation_demo/planets_combo.png",
    );

    // Errorbars as bipolar jets: planets with ±σ rendered as glowing jet
    // beams terminating in shock knots at the exact interval bounds.
    let m2 = 11;
    let jerr: Vec<f64> = (0..m2)
        .map(|i| 4.5 + 2.5 * ((i as f64) * 1.1).cos().abs())
        .collect();
    r.add_column("perr", &jerr).unwrap();
    let jets = [SeriesConfig::scatter("jets", "px", "pb")
        .color(Color::from_rgb8(255, 142, 92))
        .marker(ScatterShape::Circle, 13.0)
        .y_errors("perr")
        .error_color(Color::from_rgb8(255, 160, 110))
        .error_bars(2.0, 7.0)];
    export(
        &mut r,
        &mut build_chart(DrawStyle::Milkyway(MilkywayOptions::default())),
        &jets,
        "target/constellation_demo/jets.png",
    );
}
