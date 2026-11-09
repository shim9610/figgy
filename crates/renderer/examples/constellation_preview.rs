//! README Constellation preview: stars at sparse source points, joined by lines.
//! cargo run --locked -p figgy-renderer --example constellation_preview -- target/constellation-preview.png
//! Constellation uses ScatterLine; Milkyway uses Line in its separate preview.
use renderer::config::{ConstellationOptions, DrawStyle, LegendCorner, LegendEntryKind};
use renderer::theme::{SubplotScope, Theme};
use renderer::{Chart, Color, Renderer, SeriesConfig, encode_png};
use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or("target/constellation-preview.png".into()),
    );
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut r = Renderer::headless_blocking()?;
    // Light ink on the deep-space backdrop the export clears to, at the
    // style row's text sizes, without grid lines.
    let bg = Color::from_rgb8(11, 15, 23);
    let mut theme = Theme::dark();
    theme.chrome.background = None;
    theme.chrome.legend_fill = bg;
    theme.chrome.tick_label_size = 25.0;
    theme.chrome.axis_title_size = 32.0;
    theme.chrome.legend_size = 23.0;
    let grid = &mut theme.chrome.grid;
    for line in [
        &mut grid.major_x,
        &mut grid.major_y,
        &mut grid.minor_x,
        &mut grid.minor_y,
    ] {
        line.show = false;
    }
    let mut chart = Chart::sized(1280, 1024);
    theme.apply_chrome(chart.config_mut(), SubplotScope::All);
    let cfg = chart.config_mut();
    // The four style previews share these margins, so their plot areas
    // line up in the row whatever face a draw style uses.
    cfg.chart_title.top_margin = 60.0;
    cfg.left_y.out_margin = 200.0;
    cfg.bottom_x.out_margin = 145.0;
    cfg.top_x.out_margin = 32.0;
    cfg.right_y.out_margin = 64.0;
    // After the theme, which writes the draw style too.
    cfg.draw_style = DrawStyle::Constellation(ConstellationOptions::default());
    cfg.legend.corner = LegendCorner::TopLeft;
    cfg.legend.padding = 9.0;
    let mut chart = chart
        .with_x_title("Time [min]")
        .with_y_title("Response mean");
    chart.set_x_range(-6.0, 126.0);
    chart.set_y_range(-0.05, 1.08);
    let ts: Vec<f64> = vec![0.0, 5.0, 10.0, 15.0, 20.0, 30.0, 45.0, 60.0, 90.0, 120.0];
    r.add_column("time", &ts)?;
    let mut series = Vec::new();
    for (id, label, color, ceiling, tau) in [
        (
            "control",
            "Control mean",
            Color::from_rgb8(225, 161, 206),
            0.87,
            40.0,
        ),
        (
            "treated",
            "Treated mean",
            Color::from_rgb8(165, 215, 229),
            1.04,
            19.0,
        ),
    ] {
        let values: Vec<f64> = ts
            .iter()
            .map(|t| 0.06 + (ceiling - 0.06) * (1.0 - (-t / tau).exp()))
            .collect();
        r.add_column(id, &values)?;
        // Each original sample becomes a star; the line connects those samples.
        // Do not densely resample this series to create additional stars.
        series.push(
            SeriesConfig::scatter_line(id, "time", id)
                .color(color)
                .width(2.0),
        );
        chart = chart.with_legend_entry(label, color, 2.0, LegendEntryKind::Line);
    }
    let img = r.export_rgba_blocking(
        renderer::ExportSource::chart(&chart, &series),
        renderer::ExportOptions::panel(1.0).with_clear(bg),
    )?;
    let empty = r.export_rgba_blocking(
        renderer::ExportSource::chart(&chart, &[]),
        renderer::ExportOptions::panel(1.0).with_clear(bg),
    )?;
    let changed = img
        .rgba
        .chunks_exact(4)
        .zip(empty.rgba.chunks_exact(4))
        .enumerate()
        .filter(|(i, (a, b))| {
            let x = *i % 1280;
            let y = *i / 1280;
            x > 210
                && x < 1200
                && y > 180
                && y < 869
                && a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 8)
        })
        .count();
    assert!(changed > 1000, "missing constellation data: {changed}");
    assert!(img.rgba.chunks_exact(4).all(|p| p[3] == 255));
    std::fs::write(&output, encode_png(&img)?)?;
    println!(
        "{}: 1280x1024, {changed} data-dependent pixels; two ScatterLine series",
        output.display()
    );
    r.wait_idle()?;
    Ok(())
}
