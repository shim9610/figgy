//! Radar documents drawn as polar charts of radar series: each `RadarChart`
//! is opened with `Config::from_radar`, its category indices and values
//! uploaded from the borrowed document, and exported by the Cartesian
//! renderer at 960 × 640 without post-render scaling. Writes five PNGs into
//! the directory given as the first argument (default
//! `target/radar-gallery`), and fails when an image has fewer than 1000
//! coloured pixels.
//! - `01-spider-web.png`: two series on the default polygon grid.
//! - `02-circle-values.png`: a circular grid with a value label outside
//!   every vertex.
//! - `03-fixed-axis.png`: a fixed 0 to 10 axis with a ring every 2.5 and
//!   square markers.
//! - `04-four-series.png`: four series counter-clockwise from the right,
//!   including an unfilled baseline without markers.
//! - `05-selection.png`: a selected vertex, highlighted by the picked ring.
//!
//! cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery
use renderer::{
    Category, Chart, Color, LayerRef, Renderer, RendererDevice, config::Config, data_render,
    encode_png, radar::*,
};
use std::{path::Path, path::PathBuf, sync::Arc};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// `document` as a polar chart of radar series, exported to `name.png`.
fn save(r: &mut Renderer, document: &RadarChart, directory: &Path, name: &str) -> Result {
    document.validate()?;
    let (mut config, series, uploads) = Config::from_radar(document);
    config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
        x: 0,
        y: 0,
        width: 960,
        height: 640,
    });
    for upload in &uploads {
        r.remove_column(&upload.column)?;
        r.add_column(upload.column.clone(), &upload.values)?;
    }
    // The fixed `value_range` of a document is the radius axis; a document
    // without one fits the values' statistics from zero, as the former
    // chart did, and keeps its ring spacing.
    let mut chart = Chart::new(config);
    if document.value_range.is_none() {
        // The fit chooses a label format too; the document's ring labels
        // keep the reader's.
        let format = chart.config().left_y.label_style.format.clone();
        chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
        let mut config = chart.config().clone();
        let radius = &mut config.left_y;
        let min = radius.min.min(0.0);
        let max = radius.max;
        radius
            .auto_ticks(min, max, 5)
            .map_err(|e| format!("radius axis ticks: {e:?}"))?;
        radius.label_style.format = format;
        if let Some(spacing) = document.ring_spacing {
            radius.major_spacing = spacing;
        }
        chart = Chart::new(config);
    }
    let image = r.export_panel_rgba(&chart, &series, 1.0)?;
    let colored = image
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 40)
        .count();
    if colored < 1000 {
        return Err("missing radar pixels".into());
    }
    std::fs::write(directory.join(format!("{name}.png")), encode_png(&image)?)?;
    println!(
        "{name}: {}x{}, {colored} colored pixels",
        image.width, image.height
    );
    Ok(())
}

fn main() -> Result {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/radar-gallery".into()),
    );
    std::fs::create_dir_all(&dir)?;
    let instance = data_render::create_instance();
    let adapter = data_render::request_adapter(&instance)?;
    eprintln!("Radar adapter: {:?}", adapter.get_info());
    let (d, q) = data_render::request_device(&adapter)?;
    let gpu = RendererDevice::new(Arc::new(d), Arc::new(q));
    let mut r = Renderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm, 8 << 20)?;
    let mut c = RadarChart {
        title: "Build quality".into(),
        categories: ["Speed", "Safety", "Size", "Docs", "Tests", "Ergonomics"]
            .into_iter()
            .map(|s| Category::new(s.to_lowercase(), s))
            .collect(),
        series: vec![
            RadarSeries::new(
                "v1",
                "Release 1",
                vec![6., 8., 4., 5., 7., 6.5],
                Color::from_rgb8(65, 144, 208),
            ),
            RadarSeries::new(
                "v2",
                "Release 2",
                vec![8., 7., 6., 8., 9., 7.],
                Color::from_rgb8(230, 120, 64),
            ),
        ],
        font_size: 18.,
        ..Default::default()
    };
    save(&mut r, &c, &dir, "01-spider-web")?;
    c.title = "Circular grid, value labels".into();
    c.style.grid_shape = RadarGridShape::Circle;
    c.labels = RadarLabels::Values;
    c.value_suffix = " pt".into();
    c.label_decimals = 0;
    save(&mut r, &c, &dir, "02-circle-values")?;
    c.title = "Fixed 0 to 10 axis, square markers".into();
    c.style.grid_shape = RadarGridShape::Polygon;
    c.labels = RadarLabels::None;
    c.value_range = Some([0., 10.]);
    c.ring_spacing = Some(2.5);
    c.style.marker = RadarMarker::Square;
    c.style.fill_opacity = 0.15;
    save(&mut r, &c, &dir, "03-fixed-axis")?;
    c.title = "Four teams, counter-clockwise from the right".into();
    c.style.start_angle_degrees = 0.;
    c.style.clockwise = false;
    c.style.marker = RadarMarker::None;
    c.style.line_width_px = 3.;
    c.value_range = None;
    c.ring_spacing = None;
    c.series.push(RadarSeries::new(
        "v3",
        "Release 3",
        vec![9., 8.5, 8., 9., 9.5, 8.],
        Color::from_rgb8(42, 168, 120),
    ));
    let mut muted = RadarSeries::new(
        "baseline",
        "Baseline",
        vec![5., 5., 5., 5., 5., 5.],
        Color::from_rgb8(120, 120, 130),
    );
    muted.style = Some(RadarSeriesStyle {
        line_width_px: Some(1.5),
        fill: Some(Color::new(0., 0., 0., 0.)),
        marker: Some(RadarMarker::None),
        ..Default::default()
    });
    c.series.push(muted);
    save(&mut r, &c, &dir, "04-four-series")?;
    c.title = "Selected vertex".into();
    c.style.start_angle_degrees = -90.;
    c.style.clockwise = true;
    c.style.marker = RadarMarker::Circle;
    c.series.truncate(2);
    c.labels = RadarLabels::Emphasized;
    c.selected = Some(RadarTarget::vertex("v2", "docs"));
    c.hovered = Some(RadarTarget::series("v1"));
    save(&mut r, &c, &dir, "05-selection")?;
    Ok(())
}
