//! Shared native device setup, presentation defaults and PNG checks for gallery examples.
//! Chart data and series definitions live in each example, not in this module.
use renderer::{
    Chart, Color, Renderer, SeriesConfig, encode_png,
    theme::{ChartKind, SubplotScope, Theme},
};
use std::{error::Error, path::Path};

pub type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

/// The gallery look: the light theme with a navy title, a slate frame, a
/// pale major grid and smaller text.
pub fn theme() -> Theme {
    let mut theme = Theme::named(ChartKind::Line, "default").expect("every kind has a default");
    let chrome = &mut theme.chrome;
    chrome.ink = Color::from_rgb8(24, 39, 63);
    chrome.frame_ink = Color::from_rgb8(88, 102, 123);
    chrome.title_size = 26.0;
    chrome.axis_title_size = 18.0;
    chrome.tick_label_size = 16.0;
    chrome.grid.major_x.color = Color::from_rgb8(230, 235, 242);
    chrome.grid.major_y.color = chrome.grid.major_x.color;
    theme
}

/// A gallery chart of `size` in the gallery look, with its titles. The
/// margins are fitted to the labels when the chart is saved.
pub fn chart(title: &str, x_title: &str, y_title: &str, size: (u32, u32)) -> Chart {
    let mut chart = Chart::sized(size.0, size.1);
    theme()
        .apply_chrome(chart.config_mut(), &[], SubplotScope::All)
        .expect("a chart without series takes any theme");
    chart
        .with_title(title)
        .with_x_title(x_title)
        .with_y_title(y_title)
}

/// A chart of `size` with its titles, for a built-in theme applied with
/// its series (`Chart::apply_theme`).
pub fn titled(title: &str, x_title: &str, y_title: &str, size: (u32, u32)) -> Chart {
    Chart::sized(size.0, size.1)
        .with_title(title)
        .with_x_title(x_title)
        .with_y_title(y_title)
}

/// The built-in theme `name` of `kind` (#291).
pub fn theme_named(kind: ChartKind, name: &str) -> Result<Theme> {
    Ok(Theme::named(kind, name).ok_or_else(|| format!("no {kind} theme {name:?}"))?)
}

/// Apply `theme` to the chart and its series (`Chart::apply_theme`).
pub fn themed(chart: &mut Chart, theme: &Theme, series: &mut [SeriesConfig]) -> Result {
    chart.apply_theme(theme, series, SubplotScope::All)?;
    Ok(())
}

/// Fit the chart's margins to its labels, render it on white and write it
/// as a PNG after checking that it drew.
pub fn save(r: &mut Renderer, chart: &mut Chart, series: &[SeriesConfig], path: &Path) -> Result {
    chart.fit_layout();
    let img = r.export_rgba_blocking(
        renderer::ExportSource::chart(chart, series),
        renderer::ExportOptions::panel(1.0).with_clear(Color::WHITE),
    )?;
    if img.rgba.chunks_exact(4).any(|px| px[3] != 255) {
        return Err(format!("{}: white-background export is not opaque", path.display()).into());
    }
    let colored = img
        .rgba
        .chunks_exact(4)
        .filter(|px| {
            let hi = px[0].max(px[1]).max(px[2]);
            let lo = px[0].min(px[1]).min(px[2]);
            hi.saturating_sub(lo) > 40
        })
        .count();
    if colored < 500 {
        return Err(format!(
            "{}: missing colored chart pixels ({colored})",
            path.display()
        )
        .into());
    }
    std::fs::write(path, encode_png(&img)?)?;
    println!(
        "{}: {}x{}, {colored} colored pixels",
        path.display(),
        img.width,
        img.height
    );
    Ok(())
}

pub fn with_renderer(render: impl FnOnce(&mut Renderer) -> Result) -> Result {
    // Software GPU selection belongs to the environment, not the example.
    let mut renderer = Renderer::headless_blocking()?;
    let result = render(&mut renderer);
    renderer.wait_idle()?;
    result
}

pub fn output_directory(default: &str) -> Result<std::path::PathBuf> {
    let directory = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| default.into());
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}
