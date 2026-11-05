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
use gallery::{Result, column, line, scatter, series};
use renderer::{
    Chart, Color, Config, Renderer, Subplot,
    config::{AxisScale, LogTickPolicy, ProbabilityScale},
    data_config::{DataRenderType, ScatterShape, XySeries},
    format::LabelFormat,
    layout::{ChartArea, Rect},
};
use std::path::Path;
use styled::{chrome, rgb, save};

/// A deterministic value in `-1..1`, so the sample is the same every run.
fn jitter(seed: u64, i: usize) -> f64 {
    let mut x = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x as f64 / u64::MAX as f64) * 2.0 - 1.0
}

fn probability_paper(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(250, 249, 246);
    let ink = rgb(48, 52, 70);
    let grid = rgb(226, 228, 236);
    let blue = rgb(44, 110, 190);
    let red = rgb(204, 72, 52);
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
        r.add_column(id, &column(data))?;
    }

    let cell = |x: u32, title: &str, x_title: &str, y_title: &str| -> Config {
        let mut config = renderer::default::default_config();
        config.chart_area = ChartArea(Rect {
            x,
            y: 0,
            width: 600,
            height: 800,
        });
        chrome(&mut config, bg, ink, grid);
        config.chart_title.text.font_size = 28.;
        config.left_y.out_margin = 120.;
        config.right_y.out_margin = 30.;
        config.grid.show_major_x = true;
        config.legend.visible = false;
        for axis in [&mut config.left_y, &mut config.right_y] {
            axis.label_style.font_size = 18.;
        }
        Chart::new(config)
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
        link_x: None,
        link_y: None,
    };
    for axis in [&mut times.config.left_y, &mut times.config.right_y] {
        axis.scale = AxisScale::Probability(weibull);
    }
    for axis in [&mut times.config.bottom_x, &mut times.config.top_x] {
        axis.scale = AxisScale::Logarithmic;
        axis.log_ticks = LogTickPolicy::Adaptive;
    }
    config.subplots = vec![times];

    let points = |id: &str, x: &str, color: Color, shape: ScatterShape| {
        let mut style = scatter(color, 11.);
        style.point_shape = shape;
        series(
            id,
            x,
            "p",
            DataRenderType::Xy(XySeries {
                scatter: Some(style),
                line: None,
                errors: None,
                stem: None,
            }),
        )
    };
    let fit = |id: &str, x: &str, color: Color| {
        series(
            id,
            x,
            "fit_p",
            DataRenderType::Xy(XySeries {
                scatter: None,
                line: Some(line(color, 3.)),
                errors: None,
                stem: None,
            }),
        )
    };
    let specs = [
        points("strength", "strength", blue, ScatterShape::CircleFilled),
        fit("normal_fit", "fit_strength", blue),
        points("failures", "hours", red, ScatterShape::SquareFilled),
        fit("weibull_fit", "fit_hours", red),
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
