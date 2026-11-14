//! README Milkyway preview: line-generated stars and nebula only.
//! cargo run --locked -p figgy-renderer --example milkyway_line_preview -- target/milkyway-preview.png
//! Do not replace Line with ScatterLine/LineScatterErrorbar: those add planets/jets.
use renderer::config::LegendCorner;
use renderer::theme::{ChartKind, Theme};
use renderer::{Chart, Color, ExportOptions, ExportSource, Renderer, SeriesConfig};
use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or("target/milkyway-preview.png".into()),
    );
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut r = Renderer::headless_blocking()?;
    // The night-sky Milkyway look: pale ink on deep space at the style
    // row's text sizes, no grid, pastel lines drawn as star fields.
    let theme = Theme::named(ChartKind::Line, "milkyway").ok_or("no milkyway theme")?;
    let bg = theme.chrome.background.unwrap_or(Color::BLACK);
    let mut chart = Chart::sized(1280, 1024)
        .with_x_title("Time [min]")
        .with_y_title("Response mean");
    chart.set_x_range(-6.0, 126.0);
    chart.set_y_range(-0.05, 1.08);
    let ts: Vec<f64> = (0..=480).map(|i| i as f64 * 0.25).collect();
    r.add_column("time", &ts)?;
    let mut series = Vec::new();
    for (id, label, ceiling, tau) in [
        ("control", "Control mean", 0.87, 40.0),
        ("treated", "Treated mean", 1.04, 19.0),
    ] {
        let values: Vec<f64> = ts
            .iter()
            .map(|t| 0.06 + (ceiling - 0.06) * (1.0 - (-t / tau).exp()))
            .collect();
        r.add_column(id, &values)?;
        // Line is intentional: stars are generated along arc length by the
        // Milkyway line pipeline; no scatter planets or errorbar jets exist.
        series.push(SeriesConfig::line(id, "time", id).label(label));
    }
    chart.apply_custom_theme(&theme, &mut series)?;
    let mut chart = chart.with_legend_from(&series);
    chart.config_mut().legend.corner = LegendCorner::TopLeft;
    // Fit the margins to the labels, titles and legend before export.
    chart.fit_layout();
    let img = r.export_rgba_blocking(
        ExportSource::chart(&chart, &series),
        ExportOptions::panel(1.0).with_clear(bg),
    )?;
    let empty = r.export_rgba_blocking(
        ExportSource::chart(&chart, &[]),
        ExportOptions::panel(1.0).with_clear(bg),
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
    assert!(
        changed > 1000,
        "missing line-generated star field: {changed}"
    );
    assert!(img.rgba.chunks_exact(4).all(|p| p[3] == 255));
    img.save_png(&output)?;
    println!(
        "{}: 1280x1024, {changed} data-dependent pixels; two Line series",
        output.display()
    );
    r.wait_idle()?;
    Ok(())
}
