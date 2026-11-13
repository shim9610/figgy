//! Point value maps: a value column colours the points through the chart's
//! colourbar or sizes their markers.
//! - `colormap_scatter` writes `gallery-colormap.png`: 1400 particles along two
//!   spiral arms coloured by an age column (`point_color_map` through
//!   `color_by`).
//! - `bubbles` writes `gallery-bubbles.png`: three regions of cities whose
//!   marker area follows a population column over one explicit domain
//!   (`point_size_map` through `size_by`).
//!
//! cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::{
    Renderer, SeriesConfig,
    config::{LegendCorner, LegendEntryKind},
    theme::ChartKind,
};
use std::path::Path;
use styled::{background, export, save, scene_chart, theme_named, themed};

// A colour-mapped scatter: samples along two spiral arms, coloured by a third
// column through the colourbar.
fn colormap_scatter(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::Scatter, "cool-slate")?;
    let bg = background(&theme);
    let mut chart = scene_chart("Particle ages / two arms", "x (kpc)", "y (kpc)");
    chart.set_x_range(-10., 10.);
    chart.set_y_range(-7., 7.);
    chart.config_mut().legend.visible = false;
    let (mut x, mut y, mut age) = (Vec::new(), Vec::new(), Vec::new());
    let n = 1400u32;
    for i in 0..n {
        let t = f64::from(i) / f64::from(n - 1);
        let h = f64::from(i) * 0.618;
        let j1 = (h.sin() * 43758.5453).fract() - 0.5;
        let j2 = (h.cos() * 43758.5453).fract() - 0.5;
        let arm = if i % 2 == 0 { 0. } else { std::f64::consts::PI };
        let theta = arm + 3.6 * t;
        let radius = 0.6 + 8.6 * t;
        let spread = 0.25 + 0.9 * t;
        x.push(radius * theta.cos() + spread * j1 * 2.);
        y.push(0.7 * radius * theta.sin() + spread * j2 * 1.4);
        age.push(100. * (1. - t) + 14. * j1);
    }
    for (id, data) in [("sx", x), ("sy", y), ("sage", age)] {
        r.add_column(id, &data)?;
    }
    let mut specs = [SeriesConfig::scatter("stars", "sx", "sy").color_by("sage")];
    // The age scale; the theme draws the bar and its ramp.
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.min = 0.;
    bar.axis.max = 100.;
    bar.axis.major_spacing = 25.;
    chart.config_mut().colorbar = Some(bar);
    let mut chart = chart.with_colorbar_title("Age (Myr)");
    chart.config_mut().bottom_x.major_spacing = 5.;
    chart.config_mut().left_y.major_spacing = 3.5;
    themed(&mut chart, &theme, &mut specs)?;
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-colormap.png"))
}

// A bubble chart: three regions, marker area by population over one shared
// domain, translucent fills so overlaps stay readable.
fn bubbles(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::Scatter, "warm-paper")?;
    let bg = background(&theme);
    let mut chart = scene_chart(
        "Income and life expectancy / 54 cities",
        "Income per person (USD thousands)",
        "Life expectancy (years)",
    );
    chart.set_x_range(0., 70.);
    chart.set_y_range(58., 86.);
    chart.config_mut().bottom_x.major_spacing = 10.;
    chart.config_mut().left_y.major_spacing = 4.;
    let regions = [
        ("east", "East", 8., 0.42, 0.55),
        ("north", "North", 28., 0.24, 0.35),
        ("south", "South", 16., 0.33, 0.8),
    ];
    let mut specs = Vec::new();
    for (k, (id, _, x0, slope, pop)) in regions.into_iter().enumerate() {
        let (mut x, mut y, mut p) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..18u32 {
            let h = f64::from(i * 31 + k as u32 * 97) * 0.618;
            let j1 = (h.sin() * 43758.5453).fract() - 0.5;
            let j2 = (h.cos() * 43758.5453).fract() - 0.5;
            let j3 = ((h * 1.7).sin() * 43758.5453).fract();
            let income = x0 + 30. * (f64::from(i) / 17.) + 7. * j1;
            x.push(income);
            y.push(60. + slope * income + 20. * (1. - (-income / 25.).exp()) * 0.4 + 2.5 * j2);
            p.push(0.4 + 9.6 * j3 * j3 * pop);
        }
        for (suffix, data) in [("x", x), ("y", y), ("pop", p)] {
            r.add_column(format!("{id}-{suffix}"), &data)?;
        }
        specs.push(
            SeriesConfig::scatter(id, format!("{id}-x"), format!("{id}-y")).size_by(
                format!("{id}-pop"),
                [0.4, 10.],
                [5., 34.],
            ),
        );
    }
    themed(&mut chart, &theme, &mut specs)?;
    // The legend shows each region's colour opaque, not the translucent
    // marker fill.
    for ((_, label, ..), color) in regions.iter().zip(&theme.series.palette) {
        chart = chart.with_legend_entry(label, *color, 2., LegendEntryKind::Scatter);
    }
    chart.config_mut().legend.corner = LegendCorner::BottomRight;
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-bubbles.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/value-maps")?;
    gallery::with_renderer(|r| {
        colormap_scatter(r, &out)?;
        bubbles(r, &out)
    })
}
