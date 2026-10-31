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

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, column, line, scatter, series};
use renderer::{
    Color, ColorMap, Renderer, RendererDevice,
    categorical::*,
    config::{DrawStyle, LegendEntryKind, SketchOptions},
    data_config::{
        ContourConfig, DataRenderType, FieldFillConfig, FillMode, GridLayout, MatrixOrientation,
        MatrixRef, Shading,
    },
    data_render::{create_instance, request_adapter, request_device},
    radial::*,
};
use std::{path::Path, sync::Arc};
use styled::{SIZE, cartesian, rgb, save};

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
    r.add_column("hour", &column(x.clone()))?;
    let mut specs = Vec::new();
    for (id, label, color, scale, shift) in [
        ("edge", "Edge", rgb(65, 227, 194), 1., 0.),
        ("origin", "Origin", rgb(139, 154, 255), 0.58, 0.7),
    ] {
        let data = x
            .iter()
            .map(|t| {
                let day = 180. + 450. * (-((t - 15. - shift) / 5.5).powi(2)).exp();
                let burst = 200. * (-((t - 19. - shift) / 0.65).powi(2)).exp();
                scale * (day + burst + 28. * (t * 2.4).sin() + 12. * (t * 7.1).cos())
            })
            .collect();
        r.add_column(id, &column(data))?;
        specs.push(series(
            id,
            "hour",
            id,
            DataRenderType::Line {
                line: line(color, 3.5),
            },
        ));
        chart = chart.with_legend_entry(label, color, 3.5, LegendEntryKind::Line);
    }
    let image = pollster::block_on(r.export_panel_rgba_with_clear_async(&chart, &specs, 1., bg))?;
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
    r.add_column("week", &column(x.clone()))?;
    let mut specs = Vec::new();
    for (id, label, color, base, gain) in [
        ("community", "Community", rgb(190, 81, 51), 100., 70.),
        ("newsletter", "Newsletter", rgb(49, 121, 107), 80., 39.),
    ] {
        r.add_column(
            id,
            &column(
                x.iter()
                    .map(|t| base + gain * t + 20. * (t * 1.6).sin())
                    .collect(),
            ),
        )?;
        specs.push(series(
            id,
            "week",
            id,
            DataRenderType::ScatterLine {
                line: line(color, 3.),
                scatter: scatter(color, 7.),
            },
        ));
        chart = chart.with_legend_entry(label, color, 3., LegendEntryKind::Line);
    }
    let image = pollster::block_on(r.export_panel_rgba_with_clear_async(&chart, &specs, 1., bg))?;
    save(&image, bg, &out.join("concept-editorial.png"))
}

// 3. A product analytics card: horizontal stacks, restrained labels, rounded ends.
fn revenue(gpu: &RendererDevice, out: &Path) -> Result {
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
            material: CategoryBarMaterial::Matte,
            corner_radius: 12.,
            texture_strength: 0.,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut r = CategoricalRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)?;
    let image = r.export_rgba(&chart, SIZE, 1.)?;
    save(&image, bg, &out.join("concept-revenue.png"))?;
    r.end_frame();
    Ok(())
}

// 4–5. The same named-value contract, with two very different material choices.
fn radial_concepts(gpu: &RendererDevice, out: &Path) -> Result {
    let mut r = RadialRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)?;
    let mut chart = RadialChart {
        title: "Studio time / a creative week".into(),
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
            material: RadialMaterial::Toon,
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
    };
    let image = r.export_rgba(&chart, SIZE, 1.)?;
    save(&image, chart.background, &out.join("concept-pop-art.png"))?;
    r.end_frame();

    chart.title = "Portfolio / a different perspective".into();
    chart.kind = RadialKind::Donut { inner_radius: 0.57 };
    chart.background = rgb(19, 22, 39);
    chart.label_color = rgb(218, 224, 241);
    chart.slices = vec![
        RadialSlice::new("Equities", 42., rgb(145, 159, 244)),
        RadialSlice::new("Bonds", 28., rgb(99, 204, 192)),
        RadialSlice::new("Real assets", 18., rgb(236, 166, 187)),
        RadialSlice::new("Cash", 12., rgb(218, 200, 146)),
    ];
    chart.style = RadialStyle {
        material: RadialMaterial::Pearl,
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
    let image = r.export_rgba(&chart, SIZE, 1.)?;
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
    r.add_column("east", &column(x.clone()))?;
    r.add_column("north", &column(y.clone()))?;
    let mut columns = Vec::new();
    for (i, x) in x.iter().enumerate() {
        let id = format!("terrain-{i}");
        let values = y
            .iter()
            .map(|y| {
                let a = 1.5 * (-((x - 3.).powi(2) / 3. + (y - 3.9).powi(2) / 1.7)).exp();
                let b = 1.1 * (-((x - 7.).powi(2) / 5. + (y - 2.).powi(2) / 2.)).exp();
                a + b + 0.11 * (x * 2. + y).sin() * (y * 1.7 - x * 0.5).cos() + 0.12
            })
            .collect();
        r.add_column(id.clone(), &column(values))?;
        columns.push(id);
    }
    let specs = [series(
        "terrain",
        "east",
        "north",
        DataRenderType::HeatmapContour {
            matrix: MatrixRef {
                columns,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.,
            },
            contour: ContourConfig {
                levels: (1..=17).map(|i| i as f64 * 0.1).collect(),
                line: line(Color::from_rgba(0.92, 0.98, 0.8, 0.65), 1.1),
                per_level_color: None,
                labels: None,
            },
        },
    )];
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
    let image = pollster::block_on(r.export_panel_rgba_with_clear_async(&chart, &specs, 1., bg))?;
    save(&image, bg, &out.join("concept-terrain.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/concepts")?;
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
    let mut r = Renderer::try_new(
        gpu.clone(),
        wgpu::TextureFormat::Rgba8Unorm,
        16 * 1024 * 1024,
    )?;
    observability(&mut r, &out)?;
    editorial(&mut r, &out)?;
    revenue(&gpu, &out)?;
    radial_concepts(&gpu, &out)?;
    terrain(&mut r, &out)?;
    r.wait_idle();
    Ok(())
}
