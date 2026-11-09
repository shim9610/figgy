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
    Color, ColorMap, Material, Renderer, SeriesConfig,
    categorical::*,
    config::{DrawStyle, SketchOptions},
    data_config::{MatrixRef, ScatterShape, Shading},
    radial::*,
};
use std::path::Path;
use styled::{SIZE, cartesian, export, rgb, save};

// 1. A dashboard card: restrained chrome and two high-contrast service metrics.
fn observability(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(14, 22, 38);
    let mut chart = cartesian(
        "Traffic / last 24 hours",
        "Hour (UTC)",
        "Requests / s",
        bg,
        rgb(182, 199, 222),
        rgb(36, 49, 70),
    );
    chart.set_x_range(0., 24.);
    chart.set_y_range(0., 1000.);
    chart.config_mut().bottom_x.major_spacing = 4.;
    chart.config_mut().left_y.major_spacing = 200.;
    let x: Vec<f64> = (0..=288).map(|i| i as f64 / 12.).collect();
    r.add_column("hour", &x)?;
    let mut specs = Vec::new();
    for (id, label, color, scale, shift) in [
        ("edge", "Edge", rgb(65, 227, 194), 1., 0.),
        ("origin", "Origin", rgb(139, 154, 255), 0.58, 0.7),
    ] {
        let data: Vec<f64> = x
            .iter()
            .map(|t| {
                let day = 180. + 450. * (-((t - 15. - shift) / 5.5).powi(2)).exp();
                let burst = 200. * (-((t - 19. - shift) / 0.65).powi(2)).exp();
                scale * (day + burst + 28. * (t * 2.4).sin() + 12. * (t * 7.1).cos())
            })
            .collect();
        r.add_column(id, &data)?;
        specs.push(
            SeriesConfig::line(id, "hour", id)
                .label(label)
                .color(color)
                .width(3.5),
        );
    }
    let chart = chart.with_legend_from(&specs);
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("concept-observability.png"))
}

// 2. A warm editorial figure, with deterministic path wobble and sparse markers.
fn editorial(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(250, 245, 232);
    let mut chart = cartesian(
        "Small habits, lasting growth",
        "Week",
        "Weekly active members",
        bg,
        rgb(78, 67, 56),
        rgb(225, 215, 195),
    );
    chart.config_mut().draw_style = DrawStyle::Sketch(SketchOptions {
        amplitude_px: 1.5,
        wavelength_px: 64.,
        seed: 7,
    });
    chart.set_x_range(-0.3, 12.3);
    chart.set_y_range(0., 1200.);
    chart.config_mut().bottom_x.major_spacing = 2.;
    chart.config_mut().left_y.major_spacing = 300.;
    let x: Vec<f64> = (0..=12).map(|i| i as f64).collect();
    r.add_column("week", &x)?;
    let mut specs = Vec::new();
    for (id, label, color, base, gain) in [
        ("community", "Community", rgb(190, 81, 51), 100., 70.),
        ("newsletter", "Newsletter", rgb(49, 121, 107), 80., 39.),
    ] {
        let data: Vec<f64> = x
            .iter()
            .map(|t| base + gain * t + 20. * (t * 1.6).sin())
            .collect();
        r.add_column(id, &data)?;
        specs.push(
            SeriesConfig::scatter_line(id, "week", id)
                .label(label)
                .color(color)
                .width(3.)
                .marker(ScatterShape::CircleFilled, 7.),
        );
    }
    let chart = chart.with_legend_from(&specs);
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("concept-editorial.png"))
}

// 3. A product analytics card: horizontal stacks and rounded ends, a
// categorical document opened as a Cartesian chart of category bars.
fn revenue(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(247, 248, 253);
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
                rgb(98, 86, 225),
            ),
            BarSeries::new(
                "business",
                "Business",
                vec![Some(28.), Some(23.), Some(19.), Some(14.)],
                rgb(157, 141, 242),
            ),
            BarSeries::new(
                "enterprise",
                "Enterprise",
                vec![Some(19.), Some(17.), Some(12.), Some(9.)],
                rgb(37, 181, 167),
            ),
        ],
        direction: CategoryBarDirection::Horizontal,
        mode: CategoryBarMode::Stacked,
        labels: CategoryBarLabels::Inside,
        label_decimals: 0,
        value_title: "Monthly revenue (USD thousands)".into(),
        group_width: 0.55,
        font_size: 20.,
        background: bg,
        text_color: rgb(52, 57, 85),
        grid: false,
        style: CategoryBarStyle {
            material: Material::Matte,
            corner_radius: 12.,
            texture_strength: 0.,
            ..Default::default()
        },
        ..Default::default()
    };
    let (config, specs, uploads) = renderer::config::Config::from_categorical(&chart);
    // The positions, then the value columns as one batch.
    let (positions, values) = uploads.split_first().expect("the positions column");
    r.add_column(positions.column.clone(), &positions.values)?;
    let columns: Vec<(&str, &dyn renderer::ColumnSource)> = values
        .iter()
        .map(|u| (u.column.as_str(), &u.values as &dyn renderer::ColumnSource))
        .collect();
    r.add_columns(&columns)?;
    let mut cartesian = renderer::Chart::new(config).with_area(0, 0, SIZE.0, SIZE.1);
    cartesian.auto_fit_layer_union(r.pool(), renderer::LayerRef::Base, &specs, 0.0)?;
    let image = export(r, &cartesian, &specs, bg)?;
    save(&image, bg, &out.join("concept-revenue.png"))
}

// 4–5. The same named-value contract, with two very different material choices.
fn radial_concepts(out: &Path) -> Result {
    // The radial renderer takes a device of its own making.
    let mut r = RadialRenderer::headless_blocking()?;
    let mut chart = RadialChart {
        slices: vec![
            RadialSlice::new("Design", 38., rgb(245, 103, 87)),
            RadialSlice::new("Build", 32., rgb(111, 126, 241)),
            RadialSlice::new("Explore", 18., rgb(247, 191, 65)),
            RadialSlice::new("Share", 12., rgb(95, 196, 170)),
        ],
        font_size: 23.,
        label_decimals: 0,
        label_color: rgb(49, 43, 59),
        background: rgb(255, 248, 237),
        style: RadialStyle {
            material: Material::Toon,
            tilt_degrees: 45.,
            depth: 0.14,
            bevel: 0.,
            gap_degrees: 2.,
            inner_corner: 0.,
            outer_corner: 0.02,
            shadow: false,
            texture_strength: 0.,
            outline: RadialOutline {
                rim: true,
                separators: true,
                width: 2.,
                color: rgb(49, 43, 59),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    }
    .with_title("Studio time / a creative week");
    chart.format_labels();
    let image = r.export_rgba_blocking(&chart, SIZE, renderer::ExportOptions::document(1.))?;
    save(&image, chart.background, &out.join("concept-pop-art.png"))?;
    r.end_frame();

    chart.kind = RadialKind::Donut { inner_radius: 0.57 };
    chart.background = rgb(19, 22, 39);
    chart.label_color = rgb(218, 224, 241);
    chart.set_title_text("Portfolio / a different perspective");
    chart.slices = vec![
        RadialSlice::new("Equities", 42., rgb(145, 159, 244)),
        RadialSlice::new("Bonds", 28., rgb(99, 204, 192)),
        RadialSlice::new("Real assets", 18., rgb(236, 166, 187)),
        RadialSlice::new("Cash", 12., rgb(218, 200, 146)),
    ];
    chart.style = RadialStyle {
        material: Material::Pearl,
        tilt_degrees: 55.,
        depth: 0.19,
        bevel: 0.025,
        gap_degrees: 2.,
        inner_corner: 0.06,
        outer_corner: 0.045,
        texture_strength: 0.45,
        texture_scale: 1.,
        roughness: 0.3,
        gloss: 0.65,
        shadow: true,
        ..Default::default()
    };
    chart.format_labels();
    let image = r.export_rgba_blocking(&chart, SIZE, renderer::ExportOptions::document(1.))?;
    save(&image, chart.background, &out.join("concept-portfolio.png"))?;
    r.end_frame();
    Ok(())
}

// 6. A topographic poster: a custom ramp and isolines over a synthetic terrain.
fn terrain(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(12, 40, 43);
    let mut chart = cartesian(
        "Contours of an imagined landscape",
        "East (km)",
        "North (km)",
        bg,
        rgb(222, 230, 197),
        bg,
    );
    chart.set_x_range(0., 10.);
    chart.set_y_range(0., 6.);
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_y = false;
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
    .color(Color::from_rgba(0.92, 0.98, 0.8, 0.65))
    .width(1.1)
    // The custom ramp under the isolines, interpolated between the cells.
    .filled()
    .shading(Shading::Interpolated);
    let specs = [terrain];
    let mut bar = renderer::default::default_colorbar_options();
    // The gallery image was rendered with this gap.
    bar.gap_px = 24.0;
    bar.colormap = ColorMap::Custom {
        stops: vec![
            bg,
            rgb(29, 89, 85),
            rgb(73, 132, 112),
            rgb(164, 185, 120),
            rgb(239, 221, 150),
        ],
    };
    bar.axis.min = 0.;
    bar.axis.max = 1.8;
    bar.axis.major_spacing = 0.6;
    bar.axis.label_style.color = rgb(222, 230, 197);
    bar.axis.label_style.font_size = 18.;
    bar.axis.line_color = rgb(164, 185, 120);
    chart.config_mut().colorbar = Some(bar);
    let image = export(r, &chart, &specs, bg)?;
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
