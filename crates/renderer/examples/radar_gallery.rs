//! Radar documents drawn as polar charts of radar series: each `RadarChart`
//! is opened with `Config::from_radar`, its category indices and values
//! uploaded from the borrowed document, and exported by the Cartesian
//! renderer at 960 × 640 without post-render scaling, in the `document`
//! radar theme; the cases that show another outline or fill vary the
//! theme's radar part before applying it. Writes five PNGs into
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
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::Result;
use renderer::{
    Category, Chart, Color, LayerRef, Renderer, SeriesConfig,
    config::Config,
    radar::*,
    theme::{ChartKind, SubplotScope, Theme},
};
use std::path::Path;

/// `document` as a polar chart of radar series in `theme`, exported to
/// `name.png`. `restyle` then styles single series, for a case that draws
/// one of them apart.
fn save(
    r: &mut Renderer,
    document: &RadarChart,
    theme: &Theme,
    restyle: impl FnOnce(&mut [SeriesConfig]),
    directory: &Path,
    name: &str,
) -> Result {
    document.validate()?;
    let (config, mut series, uploads) = Config::from_radar(document);
    for upload in &uploads {
        r.remove_column(&upload.column)?;
        r.add_column(upload.column.clone(), &upload.values)?;
    }
    // The fixed `value_range` of a document is the radius axis; a document
    // without one fits the values' statistics from zero, as the former
    // chart did, and keeps its ring spacing.
    let mut chart = Chart::new(config).with_area(0, 0, 960, 640);
    chart.apply_theme(theme, &mut series, SubplotScope::All)?;
    restyle(&mut series);
    if document.value_range.is_none() {
        // The fit chooses a label format too; the document's ring labels
        // keep the reader's.
        let format = chart.config().left_y.label_style.format.clone();
        chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
        let radius = &mut chart.config_mut().left_y;
        let min = radius.min.min(0.0);
        let max = radius.max;
        radius
            .auto_ticks(min, max, 5)
            .map_err(|e| format!("radius axis ticks: {e:?}"))?;
        radius.label_style.format = format;
        if let Some(spacing) = document.ring_spacing {
            radius.major_spacing = spacing;
        }
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
        return Err("missing radar pixels".into());
    }
    image.save_png(directory.join(format!("{name}.png")))?;
    println!(
        "{name}: {}x{}, {colored} colored pixels",
        image.width, image.height
    );
    Ok(())
}

fn main() -> Result {
    let dir = gallery::output_directory("target/radar-gallery")?;
    gallery::with_renderer(|r| render(r, &dir))
}

fn render(r: &mut Renderer, dir: &Path) -> Result {
    let mut theme = gallery::theme_named(ChartKind::Radar, "document")?;
    let keep = |_: &mut [SeriesConfig]| {};
    // The theme colours the series: series `k` takes its colour `k`.
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
                Color::BLACK,
            ),
            RadarSeries::new(
                "v2",
                "Release 2",
                vec![8., 7., 6., 8., 9., 7.],
                Color::BLACK,
            ),
        ],
        ..Default::default()
    };
    save(r, &c, &theme, keep, dir, "01-spider-web")?;
    c.title = "Circular grid, value labels".into();
    c.style.grid_shape = RadarGridShape::Circle;
    c.labels = RadarLabels::Values;
    c.value_suffix = " pt".into();
    c.label_decimals = 0;
    save(r, &c, &theme, keep, dir, "02-circle-values")?;
    c.title = "Fixed 0 to 10 axis, square markers".into();
    c.style.grid_shape = RadarGridShape::Polygon;
    c.labels = RadarLabels::None;
    c.value_range = Some([0., 10.]);
    c.ring_spacing = Some(2.5);
    c.style.marker = RadarMarker::Square;
    // A lighter fill from here on.
    theme.series.radar.fill_opacity = Some(0.15);
    save(r, &c, &theme, keep, dir, "03-fixed-axis")?;
    c.title = "Four teams, counter-clockwise from the right".into();
    c.style.start_angle_degrees = 0.;
    c.style.clockwise = false;
    c.style.marker = RadarMarker::None;
    // Bolder outlines from here on.
    theme.series.radar.line_width_px = Some(3.);
    c.value_range = None;
    c.ring_spacing = None;
    c.series.push(RadarSeries::new(
        "v3",
        "Release 3",
        vec![9., 8.5, 8., 9., 9.5, 8.],
        Color::BLACK,
    ));
    let mut muted = RadarSeries::new(
        "baseline",
        "Baseline",
        vec![5., 5., 5., 5., 5., 5.],
        Color::BLACK,
    );
    // The baseline is an outline alone, in the theme's fourth colour.
    muted.style = Some(RadarSeriesStyle {
        fill: Some(Color::new(0., 0., 0., 0.)),
        marker: Some(RadarMarker::None),
        ..Default::default()
    });
    c.series.push(muted);
    // It is drawn thinner than the releases: the theme with a hairline
    // outline, applied to that series alone.
    let mut thin = theme.clone();
    thin.series.radar.line_width_px = Some(1.5);
    let baseline = |series: &mut [SeriesConfig]| thin.style_series(&mut series[3], 3);
    save(r, &c, &theme, baseline, dir, "04-four-series")?;
    c.title = "Selected vertex".into();
    c.style.start_angle_degrees = -90.;
    c.style.clockwise = true;
    c.style.marker = RadarMarker::Circle;
    c.series.truncate(2);
    c.labels = RadarLabels::Emphasized;
    c.selected = Some(RadarTarget::vertex("v2", "docs"));
    c.hovered = Some(RadarTarget::series("v1"));
    save(r, &c, &theme, keep, dir, "05-selection")?;
    Ok(())
}
