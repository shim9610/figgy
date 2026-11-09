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
use gallery::Result;
use renderer::{
    Color, Renderer, SeriesConfig,
    data_config::{DataLineStyleConfig, DataRenderType, LineFill, LineInterpolation, XySeries},
    line::LineStylePreset,
};
use std::path::Path;
use styled::{cartesian, export, rgb, save};

/// The line style of a line series, for the fields no modifier sets.
fn line_of(series: &mut SeriesConfig) -> &mut DataLineStyleConfig {
    match &mut series.render_type {
        DataRenderType::Xy(XySeries {
            line: Some(line), ..
        }) => line,
        _ => unreachable!("a line series"),
    }
}

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
        r.add_column(id, &data)?;
    }
    let edge = |id: &str, to: Option<&str>, alpha: f32| {
        let mut series = SeriesConfig::line(id, "day", id)
            .color(Color::from_rgba(teal.r, teal.g, teal.b, 0.0))
            .width(0.5);
        let style = line_of(&mut series);
        style.fill = to.map(|s| LineFill::ToSeries(s.into()));
        style.fill_color = Some(Color::from_rgba(teal.r, teal.g, teal.b, alpha));
        series
    };
    let specs = [
        edge("up95", Some("lo95"), 0.14),
        edge("lo95", None, 0.),
        edge("up80", Some("lo80"), 0.22),
        edge("lo80", None, 0.),
        SeriesConfig::line("history", "day", "history")
            .label("Actual")
            .color(ink)
            .width(3.),
        SeriesConfig::line("forecast", "day", "forecast")
            .label("Forecast, 80% and 95% bands")
            .color(teal)
            .width(3.)
            .dash(LineStylePreset::Dash),
    ];
    chart = chart.with_legend_from(&specs);
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
    r.add_column("hour", &hours)?;
    r.add_column("setpoint", &setpoint)?;
    r.add_column("minute", &minute)?;
    r.add_column("room", &room)?;
    let mut schedule = SeriesConfig::line("setpoint", "hour", "setpoint")
        .label("Setpoint schedule")
        .color(violet)
        .width(2.5)
        .fill_to(14.);
    let step = line_of(&mut schedule);
    step.interpolation = LineInterpolation::StepPost;
    step.fill_color = Some(Color::from_rgba(violet.r, violet.g, violet.b, 0.16));
    let measured = SeriesConfig::line("room", "minute", "room")
        .label("Room temperature")
        .color(coral)
        .width(3.5);
    let specs = [schedule, measured];
    chart = chart.with_legend_from(&specs);
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-thermostat.png"))
}

// The three step modes over the same sparse samples, for comparing where each
// one places the jump.
fn step_modes(r: &mut Renderer, out: &Path) -> Result {
    let sx: Vec<f64> = (0..8).map(f64::from).collect();
    let sy = vec![1.0, 2.5, 1.5, 3.0, 2.0, 3.5, 2.5, 3.0];
    r.add_column("sx", &sx)?;
    r.add_column("sy", &sy)?;
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
        let mut series = SeriesConfig::line(id, "sx", "sy").color(color).width(2.0);
        let style = line_of(&mut series);
        style.interpolation = interpolation;
        // A fill under a step line follows the steps.
        style.fill = (id == "post").then_some(LineFill::ToBaseline(0.0));
        series
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
