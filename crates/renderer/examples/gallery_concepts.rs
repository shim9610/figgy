//! Six visual recipes built from the public chart APIs, not presets. All data
//! is synthetic; each PNG is 1200 x 800 without post-processing.
//! - `observability` writes `concept-observability.png`: a dark service dashboard.
//! - `editorial` writes `concept-editorial.png`: a warm sketch-style figure.
//! - `revenue` writes `concept-revenue.png`: rounded horizontal stacked bars.
//! - `radial_concepts` writes `concept-pop-art.png` (a toon pie) and
//!   `concept-portfolio.png` (a pearl donut) from one slice model.
//! - `terrain` writes `concept-terrain.png`: a custom colour ramp under isolines.
//!
//! cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts
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
    Color, Renderer, SeriesConfig,
    categorical::*,
    data_config::{MatrixRef, Shading},
    radial::*,
    theme::{ChartKind, RadialTheme},
};
use std::path::Path;
use styled::{SIZE, background, export, save, scene_chart, theme_named, themed};

// 1. A dashboard card: restrained chrome and two high-contrast service metrics.
fn observability(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::Line, "midnight")?;
    let bg = background(&theme);
    let mut chart = scene_chart("Traffic / last 24 hours", "Hour (UTC)", "Requests / s");
    chart.set_x_range(0., 24.);
    chart.set_y_range(0., 1000.);
    chart.config_mut().bottom_x.major_spacing = 4.;
    chart.config_mut().left_y.major_spacing = 200.;
    let x: Vec<f64> = (0..=288).map(|i| i as f64 / 12.).collect();
    r.add_column("hour", &x)?;
    let mut specs = Vec::new();
    for (id, label, scale, shift) in [("edge", "Edge", 1., 0.), ("origin", "Origin", 0.58, 0.7)] {
        let data: Vec<f64> = x
            .iter()
            .map(|t| {
                let day = 180. + 450. * (-((t - 15. - shift) / 5.5).powi(2)).exp();
                let burst = 200. * (-((t - 19. - shift) / 0.65).powi(2)).exp();
                scale * (day + burst + 28. * (t * 2.4).sin() + 12. * (t * 7.1).cos())
            })
            .collect();
        r.add_column(id, &data)?;
        specs.push(SeriesConfig::line(id, "hour", id).label(label));
    }
    themed(&mut chart, &theme, &mut specs)?;
    let mut chart = chart.with_legend_from(&specs);
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("concept-observability.png"))
}

// 2. A warm editorial figure, with deterministic path wobble and sparse markers.
fn editorial(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::Line, "editorial")?;
    let bg = background(&theme);
    let mut chart = scene_chart(
        "Small habits, lasting growth",
        "Week",
        "Weekly active members",
    );
    chart.set_x_range(-0.3, 12.3);
    chart.set_y_range(0., 1200.);
    chart.config_mut().bottom_x.major_spacing = 2.;
    chart.config_mut().left_y.major_spacing = 300.;
    let x: Vec<f64> = (0..=12).map(|i| i as f64).collect();
    r.add_column("week", &x)?;
    let mut specs = Vec::new();
    for (id, label, base, gain) in [
        ("community", "Community", 100., 70.),
        ("newsletter", "Newsletter", 80., 39.),
    ] {
        let data: Vec<f64> = x
            .iter()
            .map(|t| base + gain * t + 20. * (t * 1.6).sin())
            .collect();
        r.add_column(id, &data)?;
        specs.push(SeriesConfig::scatter_line(id, "week", id).label(label));
    }
    themed(&mut chart, &theme, &mut specs)?;
    let mut chart = chart.with_legend_from(&specs);
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("concept-editorial.png"))
}

// 3. A product analytics card: horizontal stacks and rounded ends, a
// categorical document opened as a Cartesian chart of category bars.
fn revenue(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::CategoryBars, "revenue")?;
    let bg = background(&theme);
    let chart = CategoricalChart {
        title: "Revenue mix / by region".into(),
        categories: ["North America", "Europe", "Asia Pacific", "Latin America"]
            .iter()
            .enumerate()
            .map(|(i, name)| Category::new(format!("region-{i}"), *name))
            .collect(),
        series: vec![
            BarSeries::new(
                "team",
                "Team",
                vec![Some(42.), Some(36.), Some(28.), Some(16.)],
                Color::BLACK,
            ),
            BarSeries::new(
                "business",
                "Business",
                vec![Some(28.), Some(23.), Some(19.), Some(14.)],
                Color::BLACK,
            ),
            BarSeries::new(
                "enterprise",
                "Enterprise",
                vec![Some(19.), Some(17.), Some(12.), Some(9.)],
                Color::BLACK,
            ),
        ],
        direction: CategoryBarDirection::Horizontal,
        mode: CategoryBarMode::Stacked,
        labels: CategoryBarLabels::Inside,
        label_decimals: 0,
        value_title: "Monthly revenue (USD thousands)".into(),
        group_width: 0.55,
        ..Default::default()
    };
    let (config, mut specs, uploads) = renderer::config::Config::from_categorical(&chart);
    // The positions, then the value columns as one batch.
    let (positions, values) = uploads.split_first().expect("the positions column");
    r.add_column(positions.column.clone(), &positions.values)?;
    let columns: Vec<(&str, &dyn renderer::ColumnSource)> = values
        .iter()
        .map(|u| (u.column.as_str(), &u.values as &dyn renderer::ColumnSource))
        .collect();
    r.add_columns(&columns)?;
    let mut cartesian = renderer::Chart::new(config).with_area(0, 0, SIZE.0, SIZE.1);
    themed(&mut cartesian, &theme, &mut specs)?;
    // The value axis runs 0 to 100 (the largest stack is 89) in steps of 20,
    // printed in the document's integer format, which a range change resets.
    let format = cartesian.config().bottom_x.label_style.format.clone();
    cartesian.set_x_range(0., 100.);
    let value_axis = &mut cartesian.config_mut().bottom_x;
    value_axis.major_spacing = 20.;
    value_axis.label_style.format = format;
    // The legend as the design draws it: a filled square for each part of
    // the stacks, the entries close together in one row centred under the
    // x title. The squares are the only fielded segments.
    let legend = &mut cartesian.config_mut().legend;
    let size = legend.content.font_size;
    for mark in legend
        .content
        .segments
        .iter_mut()
        .filter(|s| s.field_em.is_some())
    {
        mark.field_em = Some(0.7);
        mark.font_size = Some(size * 1.3);
    }
    styled::legend_row_under_x_title(&mut cartesian, 0.4);
    let image = export(r, &mut cartesian, &specs, bg)?;
    save(&image, bg, &out.join("concept-revenue.png"))
}

// 4–5. The same named-value contract, with two very different material choices.
fn radial_concepts(out: &Path) -> Result {
    // The radial renderer takes a device of its own making.
    let mut r = RadialRenderer::headless_blocking()?;
    let slice = |label: &str, value: f64| RadialSlice::new(label, value, Color::BLACK);
    let theme = RadialTheme::named("pop-art").ok_or("no pop-art radial theme")?;
    let mut chart = RadialChart {
        slices: vec![
            slice("Design", 38.),
            slice("Build", 32.),
            slice("Explore", 18.),
            slice("Share", 12.),
        ],
        label_decimals: 0,
        ..Default::default()
    }
    .with_title("Studio time / a creative week");
    theme.apply(&mut chart);
    chart.format_labels();
    let image = r.export_rgba_blocking(&chart, SIZE, renderer::ExportOptions::document(1.))?;
    save(&image, chart.background, &out.join("concept-pop-art.png"))?;
    r.end_frame();

    let theme = RadialTheme::named("portfolio").ok_or("no portfolio radial theme")?;
    chart.kind = RadialKind::Donut { inner_radius: 0.57 };
    chart.set_title_text("Portfolio / a different perspective");
    chart.slices = vec![
        slice("Equities", 42.),
        slice("Bonds", 28.),
        slice("Real assets", 18.),
        slice("Cash", 12.),
    ];
    theme.apply(&mut chart);
    chart.format_labels();
    let image = r.export_rgba_blocking(&chart, SIZE, renderer::ExportOptions::document(1.))?;
    save(&image, chart.background, &out.join("concept-portfolio.png"))?;
    r.end_frame();
    Ok(())
}

// 6. A topographic poster: the `topographic` field theme's ramp and isolines
// over a synthetic terrain.
fn terrain(r: &mut Renderer, out: &Path) -> Result {
    let theme = theme_named(ChartKind::Field, "topographic")?;
    let bg = background(&theme);
    let mut chart = scene_chart(
        "Contours of an imagined landscape",
        "East (km)",
        "North (km)",
    );
    chart.set_x_range(0., 10.);
    chart.set_y_range(0., 6.);
    chart.config_mut().legend.visible = false;
    let x: Vec<f64> = (0..=120).map(|i| i as f64 / 12.).collect();
    let y: Vec<f64> = (0..=72).map(|i| i as f64 / 12.).collect();
    r.add_column("east", &x)?;
    r.add_column("north", &y)?;
    let mut columns = Vec::new();
    for (i, x) in x.iter().enumerate() {
        let id = format!("terrain-{i}");
        let values: Vec<f64> = y
            .iter()
            .map(|y| {
                let a = 1.5 * (-((x - 3.).powi(2) / 3. + (y - 3.9).powi(2) / 1.7)).exp();
                let b = 1.1 * (-((x - 7.).powi(2) / 5. + (y - 2.).powi(2) / 2.)).exp();
                a + b + 0.11 * (x * 2. + y).sin() * (y * 1.7 - x * 0.5).cos() + 0.12
            })
            .collect();
        r.add_column(id.clone(), &values)?;
        columns.push(id);
    }
    let levels = (1..=17).map(|i| i as f64 * 0.1).collect();
    let terrain = SeriesConfig::contours(
        "terrain",
        "east",
        "north",
        MatrixRef::columns_are_x(columns),
        levels,
    )
    // The theme's ramp under the isolines, interpolated between the cells.
    .filled()
    .shading(Shading::Interpolated);
    let mut specs = [terrain];
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.min = 0.;
    bar.axis.max = 1.8;
    bar.axis.major_spacing = 0.6;
    chart.config_mut().colorbar = Some(bar);
    themed(&mut chart, &theme, &mut specs)?;
    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("concept-terrain.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/concepts")?;
    gallery::with_renderer(|r| {
        observability(r, &out)?;
        editorial(r, &out)?;
        revenue(r, &out)?;
        terrain(r, &out)
    })?;
    radial_concepts(&out)
}
