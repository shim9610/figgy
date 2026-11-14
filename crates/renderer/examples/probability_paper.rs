//! Probability paper (#209): probability axes, where a sample of the paper's
//! distribution plotted against its plotting positions falls on a line.
//! - `probability_paper` writes `gallery-probability.png`: tensile strengths
//!   on normal paper and, beside them, failure times on Weibull paper over a
//!   logarithmic time axis, where the line's slope is the shape parameter.
//!
//! The host computes what a reader takes as data: the Hazen plotting
//! positions `(i - 0.5) / n` of each sorted sample and the fitted lines. The
//! renderer places them on the axes (CLAUDE.md rule 1).
//!
//! cargo run --locked -p figgy-renderer --example probability_paper -- target/probability
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::{
    Chart, Config, Renderer, SeriesConfig, Subplot,
    config::{AxisScale, LogTickPolicy, ProbabilityScale},
    data_config::ScatterShape,
    format::LabelFormat,
    theme::ChartKind,
};
use std::path::Path;
use styled::{background, save, theme_named, themed};

/// A deterministic value in `-1..1`, so the sample is the same every run.
fn jitter(seed: u64, i: usize) -> f64 {
    let mut x = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x as f64 / u64::MAX as f64) * 2.0 - 1.0
}

fn probability_paper(r: &mut Renderer, out: &Path) -> Result {
    let mut theme = theme_named(ChartKind::Probability, "warm-neutral")?;
    // Each sample and its fitted line share a colour: the strengths take
    // the first, the failure times the second.
    let [first, second, ..] = theme.series.palette[..] else {
        return Err("the warm-neutral theme has two colours".into());
    };
    theme.series.palette = vec![first, first, second, second];
    let bg = background(&theme);
    let normal = ProbabilityScale::Normal;
    let weibull = ProbabilityScale::Weibull;

    let n = 40;
    let hazen: Vec<f64> = (0..n).map(|i| (i as f64 + 0.5) / n as f64).collect();
    // Tensile strengths: normal, mean 420 MPa, sd 18, scattered about their
    // quantiles and sorted as a probability plot reads them.
    let (mean, sd) = (420.0, 18.0);
    let mut strength = Vec::with_capacity(n);
    for (i, p) in hazen.iter().enumerate() {
        let z = normal
            .forward(*p)
            .ok_or("a Hazen position is inside (0, 1)")?;
        strength.push(mean + sd * z + 4.0 * jitter(209, i));
    }
    strength.sort_by(f64::total_cmp);
    // Failure times: Weibull, shape 1.8 and scale 1000 h.
    let (shape, scale) = (1.8_f64, 1000.0_f64);
    let weibull_time = |p: f64| scale * (-(1.0 - p).ln()).powf(1.0 / shape);
    let mut hours: Vec<f64> = hazen
        .iter()
        .enumerate()
        .map(|(i, p)| weibull_time(*p) * (1.0 + 0.08 * jitter(211, i)))
        .collect();
    hours.sort_by(f64::total_cmp);
    // The fitted lines, from end to end of the axes.
    let fit_p = vec![0.001, 0.01, 0.1, 0.3, 0.5, 0.7, 0.9, 0.99, 0.999];
    let mut fit_strength = Vec::with_capacity(fit_p.len());
    for p in &fit_p {
        fit_strength.push(mean + sd * normal.forward(*p).ok_or("inside (0, 1)")?);
    }
    let fit_hours: Vec<f64> = fit_p.iter().map(|p| weibull_time(*p)).collect();
    for (id, data) in [
        ("p", hazen),
        ("strength", strength),
        ("hours", hours),
        ("fit_p", fit_p),
        ("fit_strength", fit_strength),
        ("fit_hours", fit_hours),
    ] {
        r.add_column(id, &data)?;
    }

    let cell = |x: u32, title: &str, x_title: &str, y_title: &str| -> Config {
        Chart::placed(x, 0, 600, 800)
            .with_title(title)
            .with_x_title(x_title)
            .with_y_title(y_title)
            .config()
            .clone()
    };
    let mut config = cell(
        0,
        "Tensile strength",
        "Strength [MPa]",
        "Cumulative probability",
    );
    for axis in [&mut config.left_y, &mut config.right_y] {
        axis.scale = AxisScale::Probability(normal);
    }
    let mut times = Subplot {
        config: cell(600, "Time to failure", "Time [h]", "Unreliability"),
        series: vec!["failures".into(), "weibull_fit".into()],
        ..Default::default()
    };
    for axis in [&mut times.config.left_y, &mut times.config.right_y] {
        axis.scale = AxisScale::Probability(weibull);
    }
    for axis in [&mut times.config.bottom_x, &mut times.config.top_x] {
        axis.scale = AxisScale::Logarithmic;
        axis.log_ticks = LogTickPolicy::Adaptive;
    }
    config.subplots = vec![times];

    let mut specs = [
        SeriesConfig::scatter("strength", "strength", "p"),
        SeriesConfig::line("normal_fit", "fit_strength", "fit_p"),
        // Squares tell the second paper's samples apart; the theme sizes
        // them.
        SeriesConfig::scatter("failures", "hours", "p")
            .marker(ScatterShape::SquareFilled, theme.series.lines.marker_size),
        SeriesConfig::line("weibull_fit", "fit_hours", "fit_p"),
    ];
    let mut chart = Chart::new(config);
    chart.auto_fit_x(r.pool(), "strength", 0.08)?;
    chart.set_y_range(0.001, 0.999);
    chart.set_subplot_x_range(0, 30., 5000.);
    chart.set_subplot_y_range(0, 0.001, 0.999);
    // Hours read as plain numbers (50, 100, 200 …) rather than powers.
    chart.with_axis_range_change(|c| {
        c.subplots[0].config.bottom_x.label_style.format = LabelFormat::Decimal;
    });
    themed(&mut chart, &theme, &mut specs)?;
    let image = r.export_rgba_blocking(
        renderer::ExportSource::chart(&chart, &specs),
        renderer::ExportOptions::document(1.).with_clear(bg),
    )?;
    save(&image, bg, &out.join("gallery-probability.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/probability")?;
    gallery::with_renderer(|r| probability_paper(r, &out))
}
