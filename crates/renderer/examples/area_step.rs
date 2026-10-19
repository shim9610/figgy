//! Area fills and step lines on line series.
//! - `forecast` writes `gallery-forecast.png`: the history solid and the
//!   forecast dashed, with 80% and 95% bands filled between their edge series
//!   (`LineFill::ToSeries`). Samples outside each part are NaN, which breaks
//!   the line there.
//! - `thermostat` writes `gallery-thermostat.png`: a setpoint schedule as a
//!   `StepPost` line filled to a baseline (`LineFill::ToBaseline`), the room
//!   temperature as a smooth line over it.
//! - `step_modes` writes `steps.png`: `StepPre`, `StepPost` and `StepMid` over
//!   the same eight samples, with the fill under `StepPost` following the
//!   steps.
//!
//! cargo run --locked -p figgy-renderer --example area_step -- target/area-step
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, column, line, series};
use renderer::{
    Color, Renderer,
    config::LegendEntryKind,
    data_config::{DataRenderType, LineFill, LineInterpolation},
    line::LineStylePreset,
};
use std::path::Path;
use styled::{cartesian, export, rgb, save};

// A forecast: the history as a solid line, the forecast dashed, with 80% and
// 95% bands filled between their edge series.
fn forecast(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(247, 248, 253);
    let ink = rgb(52, 57, 85);
    let teal = rgb(37, 150, 160);
    let mut chart = cartesian(
        "Demand forecast / next three weeks",
        "Day",
        "Orders per day",
        bg,
        ink,
        rgb(228, 231, 240),
    );
    chart.set_x_range(0., 49.);
    chart.set_y_range(0., 1600.);
    chart.config_mut().bottom_x.major_spacing = 7.;
    chart.config_mut().left_y.major_spacing = 400.;
    let day: Vec<f64> = (0..=49).map(f64::from).collect();
    let trend = |d: f64| 520. + 11. * d + 160. * ((d - 3.) / 7. * std::f64::consts::TAU).sin();
    let history: Vec<f64> = day
        .iter()
        .map(|d| {
            if *d <= 28. {
                trend(*d) + 35. * (d * 1.9).sin() + 22. * (d * 4.3).cos()
            } else {
                f64::NAN
            }
        })
        .collect();
    let forecast: Vec<f64> = day
        .iter()
        .map(|d| if *d >= 28. { trend(*d) } else { f64::NAN })
        .collect();
    let band = |width: f64| -> (Vec<f64>, Vec<f64>) {
        let upper = day
            .iter()
            .map(|d| {
                if *d >= 28. {
                    trend(*d) + width * (d - 28. + 2.).sqrt()
                } else {
                    f64::NAN
                }
            })
            .collect();
        let lower = day
            .iter()
            .map(|d| {
                if *d >= 28. {
                    trend(*d) - width * (d - 28. + 2.).sqrt()
                } else {
                    f64::NAN
                }
            })
            .collect();
        (upper, lower)
    };
    let (up80, lo80) = band(28.);
    let (up95, lo95) = band(52.);
    for (id, data) in [
        ("day", day),
        ("history", history),
        ("forecast", forecast),
        ("up80", up80),
        ("lo80", lo80),
        ("up95", up95),
        ("lo95", lo95),
    ] {
        r.add_column(id, &column(data))?;
    }
    let edge = |id: &str, to: Option<&str>, alpha: f32| {
        let mut style = line(Color::from_rgba(teal.r, teal.g, teal.b, 0.0), 0.5);
        style.fill = to.map(|s| LineFill::ToSeries(s.into()));
        style.fill_color = Some(Color::from_rgba(teal.r, teal.g, teal.b, alpha));
        series(id, "day", id, DataRenderType::Line { line: style })
    };
    let mut dashed = line(teal, 3.);
    dashed.line_style = LineStylePreset::Dash;
    let specs = [
        edge("up95", Some("lo95"), 0.14),
        edge("lo95", None, 0.),
        edge("up80", Some("lo80"), 0.22),
        edge("lo80", None, 0.),
        series(
            "history",
            "day",
            "history",
            DataRenderType::Line {
                line: line(ink, 3.),
            },
        ),
        series(
            "forecast",
            "day",
            "forecast",
            DataRenderType::Line { line: dashed },
        ),
    ];
    chart = chart
        .with_legend_entry("Actual", ink, 3., LegendEntryKind::Line)
        .with_legend_entry(
            "Forecast, 80% and 95% bands",
            teal,
            3.,
            LegendEntryKind::Line,
        );
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-forecast.png"))
}

// A thermostat schedule: the setpoint as a step line with a filled area, the
// measured room temperature as a smooth line over it.
fn thermostat(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(248, 247, 252);
    let ink = rgb(58, 54, 82);
    let violet = rgb(112, 92, 212);
    let coral = rgb(232, 104, 84);
    let mut chart = cartesian(
        "Thermostat / a winter day",
        "Hour",
        "Temperature (°C)",
        bg,
        ink,
        rgb(230, 228, 240),
    );
    chart.set_x_range(0., 24.);
    chart.set_y_range(14., 24.);
    chart.config_mut().bottom_x.major_spacing = 3.;
    chart.config_mut().left_y.major_spacing = 2.;
    let hours: Vec<f64> = (0..=24).map(f64::from).collect();
    let setpoint: Vec<f64> = hours
        .iter()
        .map(|h| match *h as u32 {
            0..=5 => 16.,
            6..=8 => 21.,
            9..=16 => 18.5,
            17..=21 => 21.5,
            _ => 17.,
        })
        .collect();
    let minute: Vec<f64> = (0..=288).map(|i| f64::from(i) / 12.).collect();
    let room: Vec<f64> = minute
        .iter()
        .map(|t| {
            // The room lags the setpoint with a soft first-order response.
            let target = |h: f64| match h as u32 {
                0..=5 => 16.,
                6..=8 => 21.,
                9..=16 => 18.5,
                17..=21 => 21.5,
                _ => 17.,
            };
            let mut temp = 16.4;
            let mut h = 0.;
            while h < *t {
                temp += (target(h) - temp) * 0.045 + 0.02 * (h * 5.).sin();
                h += 1. / 12.;
            }
            temp
        })
        .collect();
    r.add_column("hour", &column(hours))?;
    r.add_column("setpoint", &column(setpoint))?;
    r.add_column("minute", &column(minute))?;
    r.add_column("room", &column(room))?;
    let mut step = line(violet, 2.5);
    step.interpolation = LineInterpolation::StepPost;
    step.fill = Some(LineFill::ToBaseline(14.));
    step.fill_color = Some(Color::from_rgba(violet.r, violet.g, violet.b, 0.16));
    let specs = [
        series(
            "setpoint",
            "hour",
            "setpoint",
            DataRenderType::Line { line: step },
        ),
        series(
            "room",
            "minute",
            "room",
            DataRenderType::Line {
                line: line(coral, 3.5),
            },
        ),
    ];
    chart = chart
        .with_legend_entry("Setpoint schedule", violet, 2.5, LegendEntryKind::Line)
        .with_legend_entry("Room temperature", coral, 3.5, LegendEntryKind::Line);
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-thermostat.png"))
}

// The three step modes over the same sparse samples, for comparing where each
// one places the jump.
fn step_modes(r: &mut Renderer, out: &Path) -> Result {
    let sx: Vec<f64> = (0..8).map(f64::from).collect();
    let sy = vec![1.0, 2.5, 1.5, 3.0, 2.0, 3.5, 2.5, 3.0];
    r.add_column("sx", &column(sx))?;
    r.add_column("sy", &column(sy))?;
    let steps = [
        (
            "pre",
            LineInterpolation::StepPre,
            Color::from_rgb8(198, 68, 34),
        ),
        (
            "post",
            LineInterpolation::StepPost,
            Color::from_rgb8(34, 150, 90),
        ),
        (
            "mid",
            LineInterpolation::StepMid,
            Color::from_rgb8(120, 60, 180),
        ),
    ]
    .map(|(id, interpolation, color)| {
        let mut style = line(color, 2.0);
        style.interpolation = interpolation;
        // A fill under a step line follows the steps.
        style.fill = (id == "post").then_some(LineFill::ToBaseline(0.0));
        series(id, "sx", "sy", DataRenderType::Line { line: style })
    });
    let mut chart = gallery::chart("Step interpolation", "Sample", "Value", (960, 600));
    chart.set_x_range(-0.5, 7.5);
    chart.set_y_range(0.0, 4.0);
    gallery::save(r, &chart, &steps, &out.join("steps.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/area-step")?;
    gallery::with_renderer(|r| {
        forecast(r, &out)?;
        thermostat(r, &out)?;
        step_modes(r, &out)
    })
}
