//! Point value maps: a value column colours the points through the chart's
//! colourbar or sizes their markers.
//! - `colormap_scatter` writes `gallery-colormap.png`: 1400 particles along two
//!   spiral arms coloured by an age column (`point_color_map`).
//! - `bubbles` writes `gallery-bubbles.png`: three regions of cities whose
//!   marker area follows a population column over one explicit domain
//!   (`point_size_map` with `fit_domain`).
//!
//! cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, column, scatter, series};
use renderer::{
    Color, ColorMap, Renderer,
    config::{LegendCorner, LegendEntryKind},
    data_config::{DataRenderType, PointColorMap, PointSizeMap, SizeScale},
    text::RichText,
};
use std::path::Path;
use styled::{cartesian, colorbar, export, rgb, save};

// A colour-mapped scatter: samples along two spiral arms, coloured by a third
// column through the colourbar.
fn colormap_scatter(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(250, 250, 252);
    let ink = rgb(52, 57, 85);
    let mut chart = cartesian(
        "Particle ages / two arms",
        "x (kpc)",
        "y (kpc)",
        bg,
        ink,
        rgb(232, 234, 242),
    );
    chart.set_x_range(-10., 10.);
    chart.set_y_range(-7., 7.);
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_x = true;
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
        r.add_column(id, &column(data))?;
    }
    let mut points = scatter(ink, 5.5);
    points.point_color_map = Some(PointColorMap {
        column: "sage".into(),
    });
    let specs = [series(
        "stars",
        "sx",
        "sy",
        DataRenderType::Scatter { scatter: points },
    )];
    colorbar(
        chart.config_mut(),
        ColorMap::Custom {
            stops: vec![
                rgb(244, 190, 72),
                rgb(238, 110, 92),
                rgb(170, 70, 160),
                rgb(70, 60, 160),
                rgb(24, 32, 96),
            ],
        },
        0.,
        100.,
        25.,
        ink,
    );
    chart
        .config_mut()
        .colorbar
        .as_mut()
        .unwrap()
        .axis
        .title_option
        .visible = true;
    chart
        .config_mut()
        .colorbar
        .as_mut()
        .unwrap()
        .axis
        .title_option
        .text = RichText::plain("Age (Myr)", ink, 19., "");
    chart.config_mut().bottom_x.major_spacing = 5.;
    chart.config_mut().left_y.major_spacing = 3.5;
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-colormap.png"))
}

// A bubble chart: three regions, marker area by population over one shared
// domain, translucent fills so overlaps stay readable.
fn bubbles(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(252, 249, 244);
    let ink = rgb(70, 60, 56);
    let mut chart = cartesian(
        "Income and life expectancy / 54 cities",
        "Income per person (USD thousands)",
        "Life expectancy (years)",
        bg,
        ink,
        rgb(235, 228, 216),
    );
    chart.set_x_range(0., 70.);
    chart.set_y_range(58., 86.);
    chart.config_mut().legend.corner = LegendCorner::BottomRight;
    chart.config_mut().bottom_x.major_spacing = 10.;
    chart.config_mut().left_y.major_spacing = 4.;
    let mut specs = Vec::new();
    for (k, (id, label, color, x0, slope, pop)) in [
        ("east", "East", rgb(222, 101, 83), 8., 0.42, 0.55),
        ("north", "North", rgb(45, 140, 135), 28., 0.24, 0.35),
        ("south", "South", rgb(118, 92, 196), 16., 0.33, 0.8),
    ]
    .into_iter()
    .enumerate()
    {
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
            r.add_column(format!("{id}-{suffix}"), &column(data))?;
        }
        let mut marker = scatter(Color::from_rgba(color.r, color.g, color.b, 0.62), 8.);
        let mut size = PointSizeMap {
            column: format!("{id}-pop"),
            domain_min: 0.,
            domain_max: 1.,
            min_px: 5.,
            max_px: 34.,
            scale: SizeScale::Area,
        };
        size.fit_domain(0.4, 10.);
        marker.point_size_map = Some(size);
        specs.push(series(
            id,
            &format!("{id}-x"),
            &format!("{id}-y"),
            DataRenderType::Scatter { scatter: marker },
        ));
        chart = chart.with_legend_entry(label, color, 2., LegendEntryKind::Scatter);
    }
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-bubbles.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/value-maps")?;
    gallery::with_renderer(|r| {
        colormap_scatter(r, &out)?;
        bubbles(r, &out)
    })
}
