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
//! - `timeline` writes `timeline.png` and `timeline@2x.png`: the default
//!   timeline look (#357) without a theme, `SeriesConfig::timeline_bars` on
//!   `default::default_calendar_timeline_config` (a month header over Monday
//!   weeks), with the recipes a web Gantt chart
//!   reads by: a progress column, milestones as filled diamonds, a today
//!   line and finish-to-start dependency links (`SeriesConfig::range_links`).
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
use renderer::theme::ThemeFamily;
use renderer::{
    Category, Chart, Color, Renderer, SeriesConfig,
    config::LegendCorner,
    data_config::{
        BarGroups, CategoryBarDirection, CategoryBarLabels, DataRenderType, RangeBarSeries,
        RangeColor, RangeConnector, RangeProgress, RangeStateStyle, RangeTotal, RangeWhiskers,
    },
    format::{LabelFormat, TimestampLabelFormat, TimestampLabelMode},
    theme::ChartKind,
};
use std::path::Path;
use styled::{background, export, save, scene_chart, themed};

/// A colour the theme sets, for the legend swatch that names it.
fn role(color: Option<Color>, name: &str) -> Result<Color> {
    Ok(color.ok_or_else(|| format!("the theme has no {name} colour"))?)
}

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
    let theme = ThemeFamily::WarmPaper.theme(ChartKind::RangeBars);
    let bg = background(&theme);
    let mut chart = scene_chart(
        "Profit bridge / Q1 to Q2",
        "",
        "Operating profit (EUR million)",
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
    // Bars by direction, totals flagged by a column, joined step to step;
    // the theme colours them and draws the connectors.
    let mut bars = SeriesConfig::range_bars("bridge", "wf-pos", "wf-start", "wf-end");
    if let DataRenderType::RangeBars(RangeBarSeries {
        color_by,
        connector,
        labels,
        ..
    }) = &mut bars.render_type
    {
        *color_by = RangeColor::Direction {
            up: Color::BLACK,
            down: Color::BLACK,
            total: Some(RangeTotal {
                column: "wf-total".into(),
                color: Color::BLACK,
            }),
        };
        *connector = Some(RangeConnector {
            color: Color::BLACK,
            width: 1.,
        });
        labels.placement = CategoryBarLabels::Outside;
        labels.text = text;
    }
    let mut specs = [bars];
    chart.set_y_range(0., 80.);
    chart.config_mut().left_y.major_spacing = 20.;
    chart.config_mut().left_y.auto_significant_digits();
    themed(&mut chart, &theme, &mut specs)?;
    let range = &theme.series.range_bars;
    chart = chart
        .with_legend_swatch("Increase", role(range.up, "up")?)
        .with_legend_swatch("Decrease", role(range.down, "down")?)
        .with_legend_swatch("Total", role(range.total, "total")?);
    chart.config_mut().legend.corner = LegendCorner::TopRight;
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-waterfall.png"))
}

// Candlesticks: open to close bodies and low to high wicks on a time axis,
// with the host's moving average of the closes drawn over them.
fn candlestick(r: &mut Renderer, out: &Path) -> Result {
    let theme = ThemeFamily::Midnight.theme(ChartKind::RangeBars);
    let bg = background(&theme);
    let mut chart = scene_chart("Daily prices / synthetic index", "Date (2026)", "Price");
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
    // Bodies by direction over low-to-high wicks; the theme colours them.
    let mut candles = SeriesConfig::range_bars("price", "cd-t", "cd-open", "cd-close");
    if let DataRenderType::RangeBars(RangeBarSeries {
        whiskers,
        color_by,
        slot_width,
        ..
    }) = &mut candles.render_type
    {
        *whiskers = Some(RangeWhiskers {
            low: "cd-low".into(),
            high: "cd-high".into(),
            width: 1.,
            color: None,
        });
        *color_by = RangeColor::Direction {
            up: Color::BLACK,
            down: Color::BLACK,
            total: None,
        };
        // One row is one day on the numeric time axis.
        *slot_width = Some(DAY);
    }
    let trend = SeriesConfig::line("sma", "cd-t", "cd-sma").label("20-day moving average");
    let config = chart.config_mut();
    config.bar_groups = Some(BarGroups {
        group_width: 0.7,
        ..BarGroups::default()
    });
    config.bottom_x.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat {
        label: TimestampLabelMode::Pattern("%m/%d".into()),
        ..TimestampLabelFormat::default()
    });
    let mut specs = [candles, trend];
    chart.set_x_range(T0 - DAY, T0 + n as f64 * DAY);
    chart.set_y_range(95., 125.);
    chart.config_mut().left_y.major_spacing = 5.;
    chart.config_mut().left_y.auto_significant_digits();
    themed(&mut chart, &theme, &mut specs)?;
    let range = &theme.series.range_bars;
    chart = chart
        .with_legend_swatch("Close above open", role(range.up, "up")?)
        .with_legend_swatch("Close below open", role(range.down, "down")?)
        .with_legend_from(&specs);
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-candlestick.png"))
}

// A Gantt chart: tasks on workstream rows over a date axis, coloured by a
// state column, their finished part shaded by a progress column.
fn gantt(r: &mut Renderer, out: &Path) -> Result {
    let theme = ThemeFamily::CoolSlate.theme(ChartKind::RangeBars);
    let bg = background(&theme);
    let mut chart = scene_chart("Release plan / spring 2026", "", "");
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
    // The states in the order of the theme's state colours.
    let states = ["Done", "In progress", "At risk", "Not started"];
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
    let mut bars = SeriesConfig::range_bars("tasks", "gt-row", "gt-start", "gt-end");
    if let DataRenderType::RangeBars(RangeBarSeries {
        color_by,
        progress,
        labels,
        ..
    }) = &mut bars.render_type
    {
        // One style per state; the theme colours them.
        *color_by = RangeColor::State {
            column: "gt-state".into(),
            styles: states
                .iter()
                .map(|_| RangeStateStyle {
                    color: Color::BLACK,
                })
                .collect(),
        };
        *progress = Some(RangeProgress {
            column: "gt-progress".into(),
            remaining_opacity: 1.,
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
    config.bottom_x.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat {
        label: TimestampLabelMode::Pattern("%m/%d".into()),
        ..TimestampLabelFormat::default()
    });
    chart.set_x_range(T0 - 2.0 * DAY, T0 + 66.0 * DAY);
    let mut specs = [bars];
    themed(&mut chart, &theme, &mut specs)?;
    for (label, color) in states.iter().zip(&theme.series.range_bars.states) {
        chart = chart.with_legend_swatch(label, *color);
    }
    chart.config_mut().legend.corner = LegendCorner::TopRight;
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-gantt.png"))
}

// The default timeline look (#357), no theme: muted row colours, labels
// inside or beside their bars in contrast ink, banded rows, and the host's
// progress, milestones, today line and links. The host computes every
// value: dates, progress, which rows link and where today is.
fn timeline(r: &mut Renderer, out: &Path) -> Result {
    use renderer::annotation::{Annotation, AnnotationAxis, AnnotationKind, AnnotationLineStyle};
    use renderer::{ScatterShape, line::LineStylePreset, text::RichText};
    const T0: f64 = 1_772_409_600.0; // Monday 2026-03-02T00:00:00Z
    // (id, task, start day, days, progress, label); no days is a milestone.
    let tasks = [
        ("interviews", "User interviews", 0.0, 9.0, 1.0, "Done"),
        ("survey", "Survey", 6.0, 8.0, 1.0, "Done"),
        ("brief", "Product brief", 14.0, 5.0, 1.0, "Done"),
        ("design", "Visual design", 19.0, 14.0, 0.8, "80%"),
        ("api", "API", 22.0, 18.0, 0.55, "55%"),
        ("web", "Web app", 35.0, 18.0, 0.2, "20%"),
        ("beta", "Beta", 53.0, 0.0, 0.0, ""),
        ("qa", "Test pass", 53.0, 10.0, 0.0, "Not started"),
        ("docs", "Docs", 56.0, 9.0, 0.0, "Not started"),
        ("release", "Release", 70.0, 0.0, 0.0, ""),
    ];
    // Finish-to-start links, by row: the host's dependencies. Test pass to
    // Docs runs backward (Docs starts first) and takes the S route.
    let links = [(0, 2), (2, 3), (3, 5), (4, 5), (7, 8)];
    let today = T0 + 37.5 * DAY;
    let column = |f: &dyn Fn(&(&str, &str, f64, f64, f64, &str)) -> f64| -> Vec<f64> {
        tasks.iter().map(f).collect()
    };
    let bar = |t: &(&str, &str, f64, f64, f64, &str), v: f64| if t.3 > 0.0 { v } else { f64::NAN };
    let mark = |t: &(&str, &str, f64, f64, f64, &str), v: f64| if t.3 > 0.0 { f64::NAN } else { v };
    r.add_column(
        "tl-row",
        &column(&|t| tasks.iter().position(|u| u.0 == t.0).unwrap() as f64),
    )?;
    r.add_hilo_column("tl-start", &column(&|t| bar(t, T0 + t.2 * DAY)))?;
    r.add_hilo_column("tl-end", &column(&|t| bar(t, T0 + (t.2 + t.3) * DAY)))?;
    r.add_column("tl-done", &column(&|t| t.4))?;
    r.add_hilo_column("tl-mark", &column(&|t| mark(t, T0 + t.2 * DAY)))?;

    // The calendar header: Monday weeks below, the months above.
    let mut config = renderer::default::default_calendar_timeline_config();
    config.left_y.categories = Some(tasks.iter().map(|t| Category::new(t.0, t.1)).collect());
    let ink = Color::from_rgb8(55, 65, 81);
    let line = |color: Color, width: f32| AnnotationLineStyle {
        color,
        width,
        style: LineStylePreset::Solid,
    };
    let font = config.left_y.label_style.label_font.clone();
    let items = &mut config.annotations.items;
    let red = Color::from_rgb8(220, 68, 55);
    items.push(Annotation::new(AnnotationKind::ReferenceLine {
        axis: AnnotationAxis::X,
        value: today,
        style: line(red, 1.5),
        label: Some(RichText::plain("Today", red, 14.0, font)),
    }));

    let mut chart = Chart::sized(1200, 640).with_title("Release plan");
    *chart.config_mut() = renderer::Config {
        chart_area: chart.config().chart_area.clone(),
        ..config
    };
    chart.set_x_range(T0 - 2.0 * DAY, T0 + 74.0 * DAY);
    chart = chart.with_title("Release plan");
    let mut bars = SeriesConfig::timeline_bars("tasks", "tl-row", "tl-start", "tl-end")
        .with_progress("tl-done");
    if let DataRenderType::RangeBars(RangeBarSeries { labels, .. }) = &mut bars.render_type {
        labels.text = tasks.iter().map(|t| t.5.to_string()).collect();
    }
    let milestones = SeriesConfig::scatter("milestones", "tl-mark", "tl-row")
        .marker(ScatterShape::DiamondFilled, 16.0)
        .color(ink);
    r.add_column(
        "tl-link-from",
        &links.iter().map(|l| l.0 as f64).collect::<Vec<_>>(),
    )?;
    r.add_column(
        "tl-link-to",
        &links.iter().map(|l| l.1 as f64).collect::<Vec<_>>(),
    )?;
    // The links route between the bars on every draw (#357).
    let dependencies = SeriesConfig::range_links("links", "tasks", "tl-link-from", "tl-link-to");
    let specs = [bars, dependencies, milestones];
    for (scale, name) in [(1.0, "timeline.png"), (2.0, "timeline@2x.png")] {
        chart.fit_layout();
        let image = r.export_rgba_blocking(
            renderer::ExportSource::chart(&mut chart, &specs),
            renderer::ExportOptions::panel(scale).with_clear(Color::WHITE),
        )?;
        std::fs::write(out.join(name), renderer::encode_png(&image)?)?;
        println!("{}", out.join(name).display());
    }
    Ok(())
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    waterfall(r, out)?;
    candlestick(r, out)?;
    gantt(r, out)?;
    timeline(r, out)
}

fn main() -> Result {
    let out = gallery::output_directory("target/range-bars")?;
    gallery::with_renderer(|r| render(r, &out))
}
