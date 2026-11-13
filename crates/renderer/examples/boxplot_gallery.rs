//! Box plot documents drawn as Cartesian box plot series: each
//! `BoxPlotChart` is opened with `Config::from_boxplot`, its positions,
//! records and outliers uploaded from the borrowed summaries, and exported by
//! the Cartesian renderer at 960 x 640 without post-render scaling, in the
//! `document` theme. Every case labels each box with its median
//! (`BoxPlotLabels::Median`, written into `labels.text` by the reader); the
//! cases that show a finish vary the theme's box part before applying it.
//!
//! Two designed scenes build their box plot series directly, also at
//! 960 x 640, in the `cool-slate` and `warm-paper` themes:
//! - `violins` writes `gallery-violin.png`: split violins, two conditions per
//!   category as the two halves of one series, each half the host's density
//!   curve uploaded as a ragged column.
//! - `raincloud` writes `gallery-raincloud.png`: horizontal rainclouds, a half
//!   violin above a narrow box and the raw samples jittered below it.
//!
//! The example computes the samples, the quartiles and whiskers and the
//! Gaussian kernel density estimates; the renderer draws the columns it is
//! given.
//!
//! cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::{
    BOX_PLOT_RECORD, Category, Chart, Color, ColumnSource, LayerRef, Material, RaggedSource,
    RecordSource, Renderer, UploadValues, boxplot::*, config::Config,
};
use renderer::{
    SeriesConfig,
    config::LegendCorner,
    data_config::{
        BarGroups, BoxPoints, BoxPointsStyle, CategoryBarDirection, Violin, ViolinBody, ViolinHalf,
        ViolinStyle,
    },
    theme::{ChartKind, SubplotScope, Theme},
};
use std::path::Path;
use styled::{background, theme_named, themed};

fn summary(low: f64, q1: f64, med: f64, q3: f64, high: f64, outlier: f64) -> Option<BoxSummary> {
    let mut v = BoxSummary::new(low, q1, med, q3, high);
    v.outliers = vec![outlier];
    v.sample_count = Some(80);
    v.median_ci = Some([med - 5., med + 5.]);
    v.mean = Some(med + 2.);
    Some(v)
}

/// `document` as a Cartesian chart of box plot series in `theme`, exported
/// to `name.png`.
fn save(
    r: &mut Renderer,
    document: &BoxPlotChart,
    theme: &Theme,
    directory: &Path,
    name: &str,
) -> Result {
    document.validate()?;
    let (config, mut series, uploads) = Config::from_boxplot(document);
    for upload in &uploads {
        match &upload.values {
            UploadValues::BoxRecord(_) => {
                let fields = upload.values.box_fields().ok_or("record fields")?;
                let pairs: [(&str, &dyn ColumnSource); 9] = std::array::from_fn(|k| {
                    (BOX_PLOT_RECORD[k].0, &fields[k] as &dyn ColumnSource)
                });
                r.upsert_record(
                    upload.column.clone(),
                    &RecordSource {
                        schema: BOX_PLOT_RECORD,
                        fields: &pairs,
                    },
                )?;
            }
            UploadValues::BoxOutliers(_) => {
                let (values, offsets) = upload.values.box_outliers().ok_or("outliers")?;
                r.upsert_ragged(
                    upload.column.clone(),
                    &RaggedSource {
                        values: &values,
                        offsets: &offsets,
                    },
                )?;
            }
            values => {
                r.remove_column(&upload.column)?;
                r.add_column(upload.column.clone(), values)?;
            }
        }
    }
    // The fixed `value_range` of the documents is the value axis; a
    // document without one fits the records' and outliers' statistics.
    let mut chart = Chart::new(config).with_area(0, 0, 960, 640);
    chart.apply_theme(theme, &mut series, SubplotScope::All)?;
    // The grid marks the value axis, which runs along x for horizontal
    // boxes.
    if document.direction == BoxPlotDirection::Horizontal {
        let grid = &mut chart.config_mut().grid;
        (grid.show_major_x, grid.show_major_y) = (grid.show_major_y, grid.show_major_x);
    }
    if document.value_range.is_none() {
        chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.1)?;
    }
    chart.fit_layout();
    let image = r.export_rgba_blocking(
        renderer::ExportSource::chart(&chart, &series),
        renderer::ExportOptions::panel(1.0),
    )?;
    let colored = image
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 40)
        .count();
    if colored < 1000 {
        return Err("missing box plot pixels".into());
    }
    image.save_png(directory.join(format!("{name}.png")))?;
    println!(
        "{name}: {}x{}, {colored} colored pixels",
        image.width, image.height
    );
    Ok(())
}

/// The designed scenes' size, the size of the document cases.
const SCENE: (u32, u32) = (960, 640);

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

/// An approximately standard normal deviate: the sum of four uniform
/// deviates scaled to unit variance.
fn normal(seed: u64, i: usize) -> f64 {
    (0..4).map(|k| noise(seed, 4 * i + k)).sum::<f64>() * 0.75f64.sqrt()
}

/// Uploads the host's statistics of `groups`, one group per category:
/// the box record `{id}.record` (median, quartiles and the last samples
/// inside the 1.5 IQR fences), a Gaussian kernel density estimate with
/// Silverman's bandwidth on `grid` positions per group (the ragged column
/// `{id}.curve` and the column `{id}.density`) and the raw samples with
/// their category positions (`{id}.cat`, `{id}.value`). Returns the largest
/// density, the value that fills a half slot.
fn upload_groups(r: &mut Renderer, id: &str, groups: &[Vec<f64>], grid: usize) -> Result<f64> {
    let quantile = |sorted: &[f64], q: f64| {
        let at = q * (sorted.len() - 1) as f64;
        let (lo, hi) = (at.floor() as usize, at.ceil() as usize);
        sorted[lo] + (sorted[hi] - sorted[lo]) * (at - lo as f64)
    };
    let mut stats: [Vec<f64>; 5] = Default::default();
    let (mut curve, mut density, mut offsets) = (Vec::new(), Vec::new(), vec![0.0]);
    let (mut cat, mut value) = (Vec::new(), Vec::new());
    let mut peak = 0.0f64;
    for (k, group) in groups.iter().enumerate() {
        let mut sorted = group.clone();
        sorted.sort_by(f64::total_cmp);
        let (q1, median, q3) = (
            quantile(&sorted, 0.25),
            quantile(&sorted, 0.5),
            quantile(&sorted, 0.75),
        );
        let fence = 1.5 * (q3 - q1);
        let low = sorted.iter().copied().find(|v| *v >= q1 - fence);
        let high = sorted.iter().rev().copied().find(|v| *v <= q3 + fence);
        for (field, v) in
            stats
                .iter_mut()
                .zip([median, q1, q3, low.unwrap_or(q1), high.unwrap_or(q3)])
        {
            field.push(v);
        }
        let n = sorted.len() as f64;
        let mean = sorted.iter().sum::<f64>() / n;
        let sd = (sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
        let h = 1.06 * sd * n.powf(-0.2);
        // The curve runs two bandwidths past the extreme samples.
        let (a, b) = (sorted[0] - 2.0 * h, sorted[sorted.len() - 1] + 2.0 * h);
        for i in 0..grid {
            let v = a + (b - a) * i as f64 / (grid - 1) as f64;
            let d = sorted
                .iter()
                .map(|x| (-0.5 * ((v - x) / h).powi(2)).exp())
                .sum::<f64>()
                / (n * h * (2.0 * std::f64::consts::PI).sqrt());
            peak = peak.max(d);
            curve.push(v);
            density.push(d);
        }
        offsets.push(curve.len() as f64);
        cat.extend(std::iter::repeat_n(k as f64, group.len()));
        value.extend_from_slice(group);
    }
    let fields: [(&str, &dyn ColumnSource); 5] =
        std::array::from_fn(|k| (BOX_PLOT_RECORD[k].0, &stats[k] as &dyn ColumnSource));
    r.upsert_record(
        format!("{id}.record"),
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
    )?;
    r.upsert_ragged(
        format!("{id}.curve"),
        &RaggedSource {
            values: &curve,
            offsets: &offsets,
        },
    )?;
    r.add_column(format!("{id}.density"), &density)?;
    r.add_column(format!("{id}.cat"), &cat)?;
    r.add_column(format!("{id}.value"), &value)?;
    Ok(peak)
}

/// A designed scene of [`SCENE`] with its titles and a visible top-left
/// legend, for a built-in theme applied with its series. The margins are
/// fitted to the labels when the scene is exported.
fn designed(title: &str, x: &str, y: &str) -> Chart {
    let mut chart = gallery::titled(title, x, y, SCENE);
    let legend = &mut chart.config_mut().legend;
    legend.visible = true;
    legend.corner = LegendCorner::TopLeft;
    chart
}

/// A box plot series at the positions `{id}.pos` over the host's record
/// `{id}.record` with `violin` behind its boxes.
fn violin_series(id: &str, color: Color, violin: Violin) -> SeriesConfig {
    SeriesConfig::box_plot(id, format!("{id}.pos"), format!("{id}.record"), color).violin(violin)
}

/// A density half over `{id}.curve` and `{id}.density`, its widest point
/// `peak` filling the half slot. A `None` fill takes the series colour or
/// a box override's. The theme draws its outline.
fn half(id: &str, peak: f64, fill: Option<Color>) -> ViolinHalf {
    let mut half = ViolinHalf {
        curve: format!("{id}.curve"),
        density: format!("{id}.density"),
        density_scale: 1.0,
        style: ViolinStyle {
            fill,
            ..ViolinStyle::default()
        },
    };
    half.fit_density_scale(peak);
    half
}

fn save_scene(
    r: &mut Renderer,
    chart: &mut Chart,
    specs: &[SeriesConfig],
    bg: Color,
    path: &Path,
) -> Result {
    let image = styled::export(r, chart, specs, bg)?;
    assert_eq!((image.width, image.height), SCENE);
    assert!(image.rgba.chunks_exact(4).all(|p| p[3] == 255));
    let colored = image
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 40)
        .count();
    if colored < 2000 {
        return Err(format!("{}: missing chart colours ({colored})", path.display()).into());
    }
    image.save_png(path)?;
    println!("{}: {colored} colored pixels", path.display());
    Ok(())
}

// Split violins: reaction times under four memory loads, rested on the
// left half and after a night without sleep on the right. Both halves are
// the host's density curves on one shared scale, so their widths compare.
fn violins(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::Distribution, "cool-slate")?;
    let bg = background(&theme);
    // The two conditions' legend swatches, in the theme's first two
    // colours as the halves are.
    let (rested, tired) = match theme.series.palette.as_slice() {
        [rested, tired, ..] => (*rested, *tired),
        _ => return Err("the theme has fewer than two colours".into()),
    };
    let loads = ["1-back", "2-back", "3-back", "4-back"];
    let n = 220;
    let rested_samples: Vec<Vec<f64>> = (0..loads.len())
        .map(|k| {
            let (centre, spread) = (390.0 + 38.0 * k as f64, 38.0 + 7.0 * k as f64);
            (0..n)
                .map(|i| centre + spread * normal(11 + k as u64, i))
                .collect()
        })
        .collect();
    // After sleep loss the responses slow down and a growing share of
    // them are lapses, a second mode far above the first.
    let tired_samples: Vec<Vec<f64>> = (0..loads.len())
        .map(|k| {
            let (centre, spread) = (425.0 + 48.0 * k as f64, 46.0 + 9.0 * k as f64);
            let lapses = 0.06 + 0.07 * k as f64;
            (0..n)
                .map(|i| {
                    let lapse = (noise(41 + k as u64, i) + 1.0) * 0.5 < lapses;
                    let z = normal(31 + k as u64, i);
                    if lapse {
                        centre + 220.0 + 35.0 * z
                    } else {
                        centre + spread * z
                    }
                })
                .collect()
        })
        .collect();
    let positions: Vec<f64> = (0..loads.len()).map(|k| k as f64).collect();
    r.add_column("rt.pos", &positions)?;
    let peak = upload_groups(r, "rt-rested", &rested_samples, 160)?.max(upload_groups(
        r,
        "rt-tired",
        &tired_samples,
        160,
    )?);
    let mut chart = designed(
        "Reaction time by memory load",
        "Working-memory task",
        "Reaction time (ms)",
    );
    let config = chart.config_mut();
    config.bottom_x.categories = Some(
        loads
            .iter()
            .map(|label| Category::new(*label, *label))
            .collect(),
    );
    config.bar_groups = Some(BarGroups {
        group_width: 0.9,
        ..BarGroups::default()
    });
    // One series holds both conditions as the halves of its violins and
    // draws no box (no record): a box would summarise one condition only.
    let mut split = violin_series(
        "rt",
        Color::BLACK,
        Violin {
            body: ViolinBody::Split {
                // Each half has a fill of its own, which the theme
                // colours: the series' colour and the one after it.
                low: Some(half("rt-rested", peak, Some(Color::BLACK))),
                high: Some(half("rt-tired", peak, Some(Color::BLACK))),
            },
            box_width: 0.0,
        },
    );
    split.y_column.clear();
    chart.set_y_range(200., 1000.);
    chart.config_mut().left_y.major_spacing = 200.;
    chart.config_mut().left_y.auto_significant_digits();
    let mut specs = [split];
    themed(&mut chart, &theme, &mut specs)?;
    chart = chart
        .with_legend_swatch("Rested", rested)
        .with_legend_swatch("After 24 h awake", tired);
    save_scene(r, &mut chart, &specs, bg, &out.join("gallery-violin.png"))
}

// Rainclouds: door-to-door commute times by mode, each a half violin of the
// host's density above a narrow box of its quartiles, with every sample
// jittered below.
fn raincloud(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::Distribution, "warm-paper")?;
    let bg = background(&theme);
    // (id, label, sample generator), listed top-down: the first category
    // of a horizontal axis is the bottom row, so the axis and the positions
    // take them in reverse. The theme colours each mode, top-down.
    let modes: [(&str, &str, &dyn Fn(usize) -> f64); 5] = [
        ("bike", "Bike", &|i| 19.0 + 3.5 * normal(72, i)),
        ("car", "Car", &|i| {
            // Congestion stretches the right tail.
            let u = ((noise(79, i) + 1.0) * 0.5).max(1e-3);
            18.0 + 3.0 * normal(78, i) - 9.0 * u.ln()
        }),
        ("train", "Train", &|i| {
            27.0 + 4.0 * normal(76, i) + 9.0 * ((noise(77, i) + 1.0) * 0.5).powi(4)
        }),
        ("walk", "Walk", &|i| 31.0 + 6.5 * normal(71, i)),
        ("bus", "Bus", &|i| {
            // One route, or two with a transfer.
            if noise(73, i) > -0.2 {
                34.0 + 5.0 * normal(74, i)
            } else {
                52.0 + 6.0 * normal(75, i)
            }
        }),
    ];
    let n = 140;
    let samples: Vec<Vec<f64>> = modes
        .iter()
        .rev()
        .map(|m| (0..n).map(m.2).collect())
        .collect();
    let positions: Vec<f64> = (0..modes.len()).map(|k| k as f64).collect();
    r.add_column("commute.pos", &positions)?;
    let peak = upload_groups(r, "commute", &samples, 160)?;
    let mut chart = designed("Commute time by mode", "Door-to-door time (min)", "");
    let config = chart.config_mut();
    config.left_y.categories = Some(
        modes
            .iter()
            .rev()
            .map(|&(id, label, _)| Category::new(id, label))
            .collect(),
    );
    config.bar_groups = Some(BarGroups {
        direction: CategoryBarDirection::Horizontal,
        group_width: 0.92,
        ..BarGroups::default()
    });
    config.legend.visible = false;
    let cloud = violin_series(
        "commute",
        Color::BLACK,
        Violin {
            body: ViolinBody::Split {
                low: None,
                // No fill of its own: each category takes its box colour.
                high: Some(half("commute", peak, None)),
            },
            box_width: 0.1,
        },
    )
    .style_box_plot(|parts| {
        parts.style.show_outliers = false;
        // The samples jittered in a band below each box.
        *parts.points = Some(BoxPoints {
            positions: "commute.cat".into(),
            values: "commute.value".into(),
            style: BoxPointsStyle {
                jitter: 0.4,
                offset: -0.45,
                ..BoxPointsStyle::default()
            },
        });
    });
    chart.set_x_range(0., 80.);
    chart.config_mut().bottom_x.major_spacing = 10.;
    chart.config_mut().bottom_x.auto_significant_digits();
    let mut specs = [cloud];
    themed(&mut chart, &theme, &mut specs)?;
    save_scene(
        r,
        &mut chart,
        &specs,
        bg,
        &out.join("gallery-raincloud.png"),
    )
}

fn main() -> Result {
    let dir = gallery::output_directory("target/boxplot-gallery")?;
    gallery::with_renderer(|r| render(r, &dir))
}

fn render(r: &mut Renderer, dir: &Path) -> Result {
    let document = theme_named(ChartKind::Distribution, "document")?;
    let mut c = BoxPlotChart {
        title: "Classic matte".into(),
        categories: ["Control", "Low", "High"]
            .into_iter()
            .map(|s| Category::new(s, s))
            .collect(),
        series: vec![BoxPlotSeries::new(
            "response",
            "Response",
            vec![
                summary(12., 30., 42., 55., 78., 90.),
                summary(20., 42., 55., 68., 85., 96.),
                summary(8., 24., 36., 49., 67., 82.),
            ],
            Color::BLACK,
        )],
        value_range: Some([0., 100.]),
        value_title: "Response".into(),
        legend: false,
        // The median above every box, in every case below.
        labels: BoxPlotLabels::Median,
        ..Default::default()
    };
    // One series: the theme colours each category.
    save(r, &c, &document, dir, "01-classic-matte")?;
    c.title = "Satin finish".into();
    let mut satin = document.clone();
    satin.series.boxes.material = Material::SatinMetal;
    satin.series.boxes.gloss = 0.6;
    save(r, &c, &satin, dir, "02-satin")?;
    c.title = "Horizontal comparison".into();
    c.direction = BoxPlotDirection::Horizontal;
    save(r, &c, &document, dir, "03-horizontal")?;
    c.title = "Grouped + notch".into();
    c.direction = BoxPlotDirection::Vertical;
    c.style.notched = true;
    c.legend = true;
    c.categories = ["A", "B", "C"]
        .into_iter()
        .map(|s| Category::new(s, s))
        .collect();
    c.series[0].id = "before".into();
    c.series[0].label = "Before".into();
    c.series.push(BoxPlotSeries::new(
        "after",
        "After",
        vec![
            summary(18., 37., 50., 62., 82., 96.),
            summary(12., 30., 44., 58., 74., 88.),
            summary(9., 27., 40., 52., 69., 80.),
        ],
        Color::BLACK,
    ));
    save(r, &c, &document, dir, "04-grouped-notch")?;
    c.title = "Mean and individual styling".into();
    c.style.show_mean = true;
    c.selected = Some(BoxPlotTarget::new("B", "after"));
    let mut unoutlined = document.clone();
    unoutlined.series.boxes.outline = false;
    save(r, &c, &unoutlined, dir, "05-mean-selection")?;
    violins(r, dir)?;
    raincloud(r, dir)?;
    Ok(())
}
