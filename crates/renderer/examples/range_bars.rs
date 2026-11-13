//! Range bars: one bar per row floating from a start value to an end value,
//! with every value supplied by the host as a column.
//! - `waterfall` writes `gallery-waterfall.png`: a profit bridge whose running
//!   totals before and after each step are the start and end columns, the
//!   subtotal and the totals flagged by a column, connectors between the steps
//!   and the host's change labels.
//! - `candlestick` writes `gallery-candlestick.png`: 64 days of synthetic
//!   prices on a time axis, open to close bodies, low to high wicks, the up and
//!   down colours read from the given ends, and the host's 20-day moving average
//!   as a line.
//! - `gantt` writes `gallery-gantt.png`: a release plan as horizontal bars on a
//!   date axis, a state column choosing each task's colour and a progress
//!   column shading the finished part.
//!
//! The example computes the running totals, the prices, the moving average,
//! the dates and the progress; the renderer draws the columns it is given.
//!
//! cargo run --locked -p figgy-renderer --example range_bars -- target/range-bars
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::{
    Category, Renderer, SeriesConfig,
    config::LegendCorner,
    data_config::{
        BarGroups, CategoryBarDirection, CategoryBarLabels, DataRenderType, RangeColor,
        RangeConnector, RangeProgress, RangeStateStyle, RangeTotal, RangeWhiskers,
    },
    format::{LabelFormat, TimestampLabelFormat, TimestampLabelMode},
};
use std::path::Path;
use styled::{cartesian, export, rgb, save};

const DAY: f64 = 86_400.0;

/// Deterministic numbers in [-1, 1] from a seed and an index (no RNG crate).
fn noise(seed: u64, i: usize) -> f64 {
    let mut x = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x as f64 / u64::MAX as f64) * 2.0 - 1.0
}

// A profit bridge: each step floats between the running total before and
// after it; the opening, gross and closing bars stand on zero.
fn waterfall(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(251, 250, 247);
    let ink = rgb(46, 52, 64);
    let up = rgb(42, 157, 122);
    let down = rgb(221, 96, 78);
    let total = rgb(56, 86, 140);
    let mut chart = cartesian(
        "Profit bridge / Q1 to Q2",
        "",
        "Operating profit (EUR million)",
        bg,
        ink,
        rgb(232, 230, 224),
    );
    // (id, label, change, is a total)
    let steps = [
        ("q1", "Q1", 48.0, true),
        ("price", "Price", 16.0, false),
        ("volume", "Volume", 12.0, false),
        ("mix", "Mix", -4.0, false),
        ("gross", "Gross", 0.0, true),
        ("materials", "Materials", -9.0, false),
        ("freight", "Freight", -3.0, false),
        ("payroll", "Payroll", -6.0, false),
        ("fx", "FX", 2.0, false),
        ("q2", "Q2", 0.0, true),
    ];
    // The running totals are host data: a total bar spans zero to the
    // running total, a step bar the total before it to the total after it.
    let (mut running, mut start, mut end, mut flag, mut text) =
        (0.0f64, Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for &(_, _, change, is_total) in &steps {
        if is_total {
            running += change;
            start.push(0.0);
            end.push(running);
            text.push(format!("{running:.0}"));
        } else {
            start.push(running);
            running += change;
            end.push(running);
            let sign = if change < 0.0 { '\u{2212}' } else { '+' };
            text.push(format!("{sign}{:.0}", change.abs()));
        }
        flag.push(if is_total { 1.0 } else { 0.0 });
    }
    let positions: Vec<f64> = (0..steps.len()).map(|i| i as f64).collect();
    r.add_column("wf-pos", &positions)?;
    r.add_column("wf-start", &start)?;
    r.add_column("wf-end", &end)?;
    r.add_column("wf-total", &flag)?;
    let config = chart.config_mut();
    config.bottom_x.categories = Some(
        steps
            .iter()
            .map(|&(id, label, _, _)| Category::new(id, label))
            .collect(),
    );
    config.bar_groups = Some(BarGroups {
        group_width: 0.66,
        ..BarGroups::default()
    });
    config.legend.corner = LegendCorner::TopRight;
    let mut bars = SeriesConfig::range_bars("bridge", "wf-pos", "wf-start", "wf-end", total);
    if let DataRenderType::RangeBars {
        style,
        color_by,
        connector,
        labels,
        ..
    } = &mut bars.render_type
    {
        style.corner_radius = 3.0;
        *color_by = RangeColor::Direction {
            up,
            down,
            total: Some(RangeTotal {
                column: "wf-total".into(),
                color: total,
            }),
        };
        *connector = Some(RangeConnector {
            color: rgb(150, 150, 146),
            width: 1.5,
        });
        labels.placement = CategoryBarLabels::Outside;
        labels.text = text;
    }
    chart = chart
        .with_legend_swatch("Increase", up)
        .with_legend_swatch("Decrease", down)
        .with_legend_swatch("Total", total);
    chart.set_y_range(0., 80.);
    chart.config_mut().left_y.major_spacing = 20.;
    chart.config_mut().left_y.auto_significant_digits();
    let image = export(r, &mut chart, &[bars], bg)?;
    save(&image, bg, &out.join("gallery-waterfall.png"))
}

// Candlesticks: open to close bodies and low to high wicks on a time axis,
// with the host's moving average of the closes drawn over them.
fn candlestick(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(16, 21, 31);
    let ink = rgb(176, 186, 204);
    let up = rgb(38, 184, 150);
    let down = rgb(238, 86, 82);
    let average = rgb(242, 186, 78);
    let mut chart = cartesian(
        "Daily prices / synthetic index",
        "Date (2026)",
        "Price",
        bg,
        ink,
        rgb(34, 42, 57),
    );
    const T0: f64 = 1_767_225_600.0; // 2026-01-01T00:00:00Z
    let n = 64;
    let (mut t, mut open, mut close, mut low, mut high) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut price = 104.0f64;
    for i in 0..n {
        // A slow swing under a random walk, so the chart has a dip and a
        // rally rather than pure noise.
        let drift = 0.006 * (i as f64 / 14.0).sin();
        let o = price;
        let c = o * (1.0 + drift + 0.018 * noise(31, i));
        let wick = 0.008 * o * (0.4 + noise(32, i).abs());
        t.push(T0 + i as f64 * DAY);
        open.push(o);
        close.push(c);
        low.push(o.min(c) - wick * (0.5 + 0.5 * noise(33, i).abs()));
        high.push(o.max(c) + wick * (0.5 + 0.5 * noise(34, i).abs()));
        // The next day opens near the close.
        price = c * (1.0 + 0.003 * noise(35, i));
    }
    // The host's 20-day simple moving average of the closes; NaN until
    // twenty closes exist.
    let window = 20;
    let sma: Vec<f64> = (0..n)
        .map(|i| {
            if i + 1 < window {
                f64::NAN
            } else {
                close[i + 1 - window..=i].iter().sum::<f64>() / window as f64
            }
        })
        .collect();
    // Seconds since the epoch keep their full precision as hi/lo pairs.
    r.add_hilo_column("cd-t", &t)?;
    r.add_column("cd-open", &open)?;
    r.add_column("cd-close", &close)?;
    r.add_column("cd-low", &low)?;
    r.add_column("cd-high", &high)?;
    r.add_column("cd-sma", &sma)?;
    let mut candles = SeriesConfig::range_bars("price", "cd-t", "cd-open", "cd-close", up);
    if let DataRenderType::RangeBars {
        style,
        whiskers,
        color_by,
        slot_width,
        ..
    } = &mut candles.render_type
    {
        style.corner_radius = 0.0;
        *whiskers = Some(RangeWhiskers {
            low: "cd-low".into(),
            high: "cd-high".into(),
            width: 1.5,
            color: None,
        });
        *color_by = RangeColor::Direction {
            up,
            down,
            total: None,
        };
        // One row is one day on the numeric time axis.
        *slot_width = Some(DAY);
    }
    let trend = SeriesConfig::line("sma", "cd-t", "cd-sma")
        .color(average)
        .width(2.5)
        .label("20-day moving average");
    let config = chart.config_mut();
    config.bar_groups = Some(BarGroups {
        group_width: 0.7,
        ..BarGroups::default()
    });
    config.grid.show_major_x = true;
    config.bottom_x.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat {
        label: TimestampLabelMode::Pattern("%m/%d".into()),
        ..TimestampLabelFormat::default()
    });
    let specs = [candles, trend];
    chart = chart
        .with_legend_swatch("Close above open", up)
        .with_legend_swatch("Close below open", down)
        .with_legend_from(&specs);
    chart.set_x_range(T0 - DAY, T0 + n as f64 * DAY);
    chart.set_y_range(95., 125.);
    chart.config_mut().left_y.major_spacing = 5.;
    chart.config_mut().left_y.auto_significant_digits();
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-candlestick.png"))
}

// A Gantt chart: tasks on workstream rows over a date axis, coloured by a
// state column, their finished part shaded by a progress column.
fn gantt(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(248, 249, 252);
    let ink = rgb(40, 48, 66);
    let mut chart = cartesian(
        "Release plan / spring 2026",
        "",
        "",
        bg,
        ink,
        rgb(228, 232, 240),
    );
    const T0: f64 = 1_772_409_600.0; // Monday 2026-03-02T00:00:00Z
    // Listed top-down; the first category of a horizontal axis is the
    // bottom row, so the axis takes them in reverse.
    let rows = [
        ("research", "Research"),
        ("design", "Design"),
        ("backend", "Backend"),
        ("frontend", "Frontend"),
        ("qa", "QA"),
        ("launch", "Launch"),
    ];
    let states = [
        ("Done", rgb(96, 186, 150)),
        ("In progress", rgb(112, 156, 236)),
        ("At risk", rgb(243, 170, 88)),
        ("Not started", rgb(132, 144, 168)),
    ];
    // (row, start day, days, state, progress, name)
    let tasks = [
        (0, 0.0, 9.0, 0, 1.0, "Interviews"),
        (0, 10.0, 8.0, 0, 1.0, "Survey"),
        (1, 7.0, 12.0, 0, 1.0, "Wireframes"),
        (1, 21.0, 14.0, 1, 0.55, "Visual design"),
        (2, 12.0, 16.0, 1, 0.7, "API"),
        (2, 30.0, 12.0, 2, 0.15, "Billing"),
        (3, 24.0, 18.0, 1, 0.35, "Web app"),
        (3, 44.0, 10.0, 3, 0.0, "Polish"),
        (4, 38.0, 16.0, 3, 0.0, "Test pass"),
        (5, 56.0, 7.0, 3, 0.0, "Rollout"),
    ];
    let pick = |f: &dyn Fn(&(usize, f64, f64, usize, f64, &str)) -> f64| -> Vec<f64> {
        tasks.iter().map(f).collect()
    };
    let last = (rows.len() - 1) as f64;
    r.add_column("gt-row", &pick(&|t| last - t.0 as f64))?;
    r.add_hilo_column("gt-start", &pick(&|t| T0 + t.1 * DAY))?;
    r.add_hilo_column("gt-end", &pick(&|t| T0 + (t.1 + t.2) * DAY))?;
    r.add_column("gt-state", &pick(&|t| t.3 as f64))?;
    r.add_column("gt-progress", &pick(&|t| t.4))?;
    let mut bars = SeriesConfig::range_bars("tasks", "gt-row", "gt-start", "gt-end", states[3].1);
    if let DataRenderType::RangeBars {
        color_by,
        progress,
        labels,
        style,
        ..
    } = &mut bars.render_type
    {
        style.corner_radius = 5.0;
        *color_by = RangeColor::State {
            column: "gt-state".into(),
            styles: states
                .iter()
                .map(|&(_, color)| RangeStateStyle { color })
                .collect(),
        };
        *progress = Some(RangeProgress {
            column: "gt-progress".into(),
            remaining_opacity: 0.35,
        });
        labels.placement = CategoryBarLabels::Inside;
        labels.text = tasks.iter().map(|t| t.5.to_string()).collect();
    }
    let config = chart.config_mut();
    config.left_y.categories = Some(
        rows.iter()
            .rev()
            .map(|&(id, label)| Category::new(id, label))
            .collect(),
    );
    config.bar_groups = Some(BarGroups {
        direction: CategoryBarDirection::Horizontal,
        group_width: 0.62,
        ..BarGroups::default()
    });
    config.grid.show_major_x = true;
    config.grid.show_major_y = false;
    config.legend.corner = LegendCorner::TopRight;
    config.bottom_x.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat {
        label: TimestampLabelMode::Pattern("%m/%d".into()),
        ..TimestampLabelFormat::default()
    });
    for (label, color) in states {
        chart = chart.with_legend_swatch(label, color);
    }
    chart.set_x_range(T0 - 2.0 * DAY, T0 + 66.0 * DAY);
    let image = export(r, &mut chart, &[bars], bg)?;
    save(&image, bg, &out.join("gallery-gantt.png"))
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    waterfall(r, out)?;
    candlestick(r, out)?;
    gantt(r, out)
}

fn main() -> Result {
    let out = gallery::output_directory("target/range-bars")?;
    gallery::with_renderer(|r| render(r, &out))
}
