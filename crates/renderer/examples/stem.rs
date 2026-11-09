//! Stems from a baseline to each marker (`XySeries::stem`, #206).
//! - `impulse_response` writes `gallery-stem.png`: the 41 taps of a
//!   windowed-sinc low-pass filter as vertical stems from zero, over the
//!   ideal (unwindowed) sinc the taps approximate.
//! - `lollipop` writes `gallery-lollipop.png`: a ranked lollipop chart,
//!   horizontal stems from zero on a categorical axis, gains and losses in
//!   two colours, the zero baseline a reference line annotation.
//!
//! All inputs are synthetic; the host computes every value.
//!
//! cargo run --locked -p figgy-renderer --example stem -- target/stem
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::{
    Annotation, AnnotationAxis, AnnotationKind, AnnotationLineStyle, Chart, Color, Renderer,
    SeriesConfig,
    config::{Category, LegendCorner},
    data_config::{CategoryBarDirection, DataRenderType, ScatterShape, Stem, XySeries},
    format::{LabelFormat, PlainNumberFormat},
    line::LineStylePreset,
};
use std::f64::consts::PI;
use std::path::Path;
use styled::{cartesian, export, rgb, save};

/// `color` with alpha `a`.
fn faded(color: Color, a: f32) -> Color {
    Color::from_rgba(color.r, color.g, color.b, a)
}

/// The stem of a stem series, for the fields no modifier sets.
fn stem_of(series: &mut SeriesConfig) -> &mut Stem {
    match &mut series.render_type {
        DataRenderType::Xy(XySeries {
            stem: Some(stem), ..
        }) => stem,
        _ => unreachable!("a stem series"),
    }
}

/// The normalised sinc, sin(pi x) / (pi x).
fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.
    } else {
        (PI * x).sin() / (PI * x)
    }
}

// A stem plot: the impulse response h[n] of a 41-tap low-pass FIR filter
// (cutoff 0.2 of the sample rate, Hamming window), the ideal sinc behind it.
fn impulse_response(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(248, 249, 252);
    let ink = rgb(44, 52, 78);
    let teal = rgb(22, 128, 140);
    let sand = rgb(214, 148, 72);
    let mut chart = cartesian(
        "Low-pass FIR filter / impulse response",
        "Tap n",
        "h[n]",
        bg,
        ink,
        rgb(228, 232, 240),
    );
    let (half, cutoff) = (20i32, 0.2);
    let taps: Vec<f64> = (-half..=half).map(f64::from).collect();
    let window = |n: f64| 0.54 + 0.46 * (PI * n / f64::from(half)).cos();
    let h: Vec<f64> = taps
        .iter()
        .map(|n| 2. * cutoff * sinc(2. * cutoff * n) * window(*n))
        .collect();
    let fine: Vec<f64> = (0..=840).map(|i| -21. + f64::from(i) * 0.05).collect();
    let ideal: Vec<f64> = fine
        .iter()
        .map(|n| 2. * cutoff * sinc(2. * cutoff * n))
        .collect();
    r.add_column("stem-n", &taps)?;
    r.add_column("stem-h", &h)?;
    r.add_column("stem-fine-n", &fine)?;
    r.add_column("stem-ideal", &ideal)?;
    let ideal = SeriesConfig::line("ideal", "stem-fine-n", "stem-ideal")
        .label("Ideal sinc")
        .color(faded(sand, 0.9))
        .width(2.)
        .dash(LineStylePreset::Dash);
    let mut taps = SeriesConfig::stem("taps", "stem-n", "stem-h")
        .label("Taps h[n], Hamming window")
        .color(teal)
        .width(2.5)
        .marker(ScatterShape::CircleFilled, 10.);
    stem_of(&mut taps).color = faded(teal, 0.75);
    let specs = [ideal, taps];
    chart.set_x_range(-21.5, 21.5);
    chart.set_y_range(-0.1, 0.45);
    chart.config_mut().bottom_x.major_spacing = 5.;
    chart.config_mut().left_y.major_spacing = 0.1;
    plain_labels(&mut chart);
    chart.config_mut().legend.corner = LegendCorner::TopRight;
    // The taps lead the legend but draw over the ideal curve.
    chart = chart
        .with_legend_from(&specs[1..])
        .with_legend_from(&specs[..1]);
    chart.config_mut().annotations.items = vec![baseline(AnnotationAxis::Y, ink)];
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-stem.png"))
}

/// Tick labels as plain numbers (`-20`, `0.4`), without padded decimals.
fn plain_labels(chart: &mut Chart) {
    let config = chart.config_mut();
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
    }
}

/// The zero line every stem starts from, drawn as a reference line.
fn baseline(axis: AnnotationAxis, ink: Color) -> Annotation {
    Annotation::new(AnnotationKind::ReferenceLine {
        axis,
        value: 0.,
        style: AnnotationLineStyle {
            color: faded(ink, 0.7),
            width: 1.5,
            style: LineStylePreset::Solid,
        },
        label: None,
    })
}

// A ranked lollipop chart: the change in tree canopy of twelve districts,
// sorted, gains and losses as two series of one categorical axis.
fn lollipop(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(251, 249, 245);
    let ink = rgb(62, 56, 52);
    let green = rgb(46, 140, 96);
    let rust = rgb(204, 88, 64);
    let grid = rgb(234, 228, 218);
    let mut chart = cartesian(
        "Tree canopy change / 2015 to 2025",
        "Change in canopy cover (percentage points)",
        "",
        bg,
        ink,
        grid,
    );
    // Ranked from the largest loss (row 0, at the bottom) to the largest
    // gain (at the top).
    let districts = [
        ("rail", "Rail Yards", -4.1),
        ("port", "Port District", -2.9),
        ("airport", "Airport Fringe", -1.8),
        ("market", "Market Square", -0.9),
        ("old", "Old Town", 0.4),
        ("uni", "University", 1.3),
        ("river", "Riverside", 2.2),
        ("hill", "North Hill", 2.8),
        ("garden", "Garden Suburb", 3.6),
        ("mill", "Millbrook", 4.5),
        ("lake", "Lakeside", 5.4),
        ("park", "Parkway", 6.7),
    ];
    let categories: Vec<Category> = districts
        .iter()
        .map(|(id, label, _)| Category::new(*id, *label))
        .collect();
    chart.config_mut().left_y.categories = Some(categories);
    chart.config_mut().left_y.out_margin = 200.;
    chart.config_mut().grid.show_major_y = false;
    chart.config_mut().grid.show_major_x = true;
    chart.config_mut().legend.corner = LegendCorner::TopLeft;
    let mut specs = Vec::new();
    for (id, label, color, gains) in [
        ("gains", "Gain", green, true),
        ("losses", "Loss", rust, false),
    ] {
        let (mut rows, mut change) = (Vec::new(), Vec::new());
        for (row, (_, _, value)) in districts.iter().enumerate() {
            if (*value > 0.) == gains {
                rows.push(row as f64);
                change.push(*value);
            }
        }
        r.add_column(format!("lollipop-{id}-row"), &rows)?;
        r.add_column(format!("lollipop-{id}-change"), &change)?;
        let mut lollipops = SeriesConfig::stem(
            id,
            format!("lollipop-{id}-change"),
            format!("lollipop-{id}-row"),
        )
        .label(label)
        .color(color)
        .width(4.)
        .marker(ScatterShape::CircleFilled, 18.);
        stem_of(&mut lollipops).direction = CategoryBarDirection::Horizontal;
        specs.push(lollipops);
    }
    chart = chart.with_legend_from(&specs);
    chart.set_x_range(-5., 8.);
    chart.config_mut().bottom_x.major_spacing = 1.;
    plain_labels(&mut chart);
    chart.config_mut().annotations.items = vec![baseline(AnnotationAxis::X, ink)];
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-lollipop.png"))
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    impulse_response(r, out)?;
    lollipop(r, out)
}

fn main() -> Result {
    let out = gallery::output_directory("target/stem")?;
    gallery::with_renderer(|r| render(r, &out))
}
