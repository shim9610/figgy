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
use gallery::{Result, column, line, series};
use renderer::{
    Category, Chart, Color, Renderer, SeriesConfig,
    config::{LegendCorner, LegendEntryKind, append_legend_entry, scatter_shape_char},
    data::Column,
    data_config::{
        BarGroups, CategoryBarDirection, CategoryBarLabels, CategoryBarSeriesLabels,
        CategoryBarStyle, DataRenderType, RangeColor, RangeConnector, RangeProgress,
        RangeStateStyle, RangeTotal, RangeWhiskers, ScatterShape, XySeries,
    },
    format::{LabelFormat, TimestampLabelFormat, TimestampLabelMode},
    text::RichSegment,
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

/// A legend row with a filled square swatch, the mark a bar series uses.
fn swatch(chart: &mut Chart, label: &str, color: Color) {
    let legend = &mut chart.config_mut().legend;
    let mark = RichSegment::fielded(
        scatter_shape_char(&ScatterShape::SquareFilled),
        2.0,
        Some(color),
    );
    append_legend_entry(&mut legend.content, vec![mark], label);
    legend.visible = true;
}

/// A range bar series reading `start` and `end` at `positions`, every
/// option off.
fn range_bars(id: &str, positions: &str, start: &str, end: &str, color: Color) -> SeriesConfig {
    series(
        id,
        positions,
        end,
        DataRenderType::RangeBars {
            color,
            style: CategoryBarStyle {
                corner_radius: 3.0,
                ..CategoryBarStyle::default()
            },
            start: start.into(),
            whiskers: None,
            color_by: RangeColor::Single,
            progress: None,
            connector: None,
            slot_width: None,
            labels: CategoryBarSeriesLabels::default(),
        },
    )
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
    r.add_column(
        "wf-pos",
        &column((0..steps.len()).map(|i| i as f64).collect()),
    )?;
    r.add_column("wf-start", &column(start))?;
    r.add_column("wf-end", &column(end))?;
    r.add_column("wf-total", &column(flag))?;
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
    let mut bars = range_bars("bridge", "wf-pos", "wf-start", "wf-end", total);
    if let DataRenderType::RangeBars {
        color_by,
        connector,
        labels,
        ..
    } = &mut bars.render_type
    {
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
    swatch(&mut chart, "Increase", up);
    swatch(&mut chart, "Decrease", down);
    swatch(&mut chart, "Total", total);
    chart.set_y_range(0., 80.);
    chart.config_mut().left_y.major_spacing = 20.;
    chart.config_mut().left_y.auto_significant_digits();
    let image = export(r, &chart, &[bars], bg)?;
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
    r.add_hilo_column("cd-t", &Column { data: t })?;
    r.add_column("cd-open", &column(open))?;
    r.add_column("cd-close", &column(close))?;
    r.add_column("cd-low", &column(low))?;
    r.add_column("cd-high", &column(high))?;
    r.add_column("cd-sma", &column(sma))?;
    let mut candles = range_bars("price", "cd-t", "cd-open", "cd-close", up);
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
    let trend = series(
        "sma",
        "cd-t",
        "cd-sma",
        DataRenderType::Xy(XySeries {
            scatter: None,
            line: Some(line(average, 2.5)),
            errors: None,
            stem: None,
        }),
    );
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
    swatch(&mut chart, "Close above open", up);
    swatch(&mut chart, "Close below open", down);
    chart = chart.with_legend_entry("20-day moving average", average, 2.5, LegendEntryKind::Line);
    chart.set_x_range(T0 - DAY, T0 + n as f64 * DAY);
    chart.set_y_range(95., 125.);
    chart.config_mut().left_y.major_spacing = 5.;
    chart.config_mut().left_y.auto_significant_digits();
    let specs = [candles, trend];
    let image = export(r, &chart, &specs, bg)?;
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
    r.add_column("gt-row", &column(pick(&|t| last - t.0 as f64)))?;
    r.add_hilo_column(
        "gt-start",
        &Column {
            data: pick(&|t| T0 + t.1 * DAY),
        },
    )?;
    r.add_hilo_column(
        "gt-end",
        &Column {
            data: pick(&|t| T0 + (t.1 + t.2) * DAY),
        },
    )?;
    r.add_column("gt-state", &column(pick(&|t| t.3 as f64)))?;
    r.add_column("gt-progress", &column(pick(&|t| t.4)))?;
    let mut bars = range_bars("tasks", "gt-row", "gt-start", "gt-end", states[3].1);
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
        swatch(&mut chart, label, color);
    }
    chart.set_x_range(T0 - 2.0 * DAY, T0 + 66.0 * DAY);
    let image = export(r, &chart, &[bars], bg)?;
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
