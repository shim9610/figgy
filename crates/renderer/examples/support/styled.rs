//! The restrained chart chrome shared by the designed gallery scenes: no axis
//! lines or ticks, a horizontal grid, a quiet legend, an opaque background,
//! 1200 x 800 pixels.
use crate::gallery::{self, Result};
use renderer::{
    AxisTitleElement, Chart, Color, ColorMap, Config, CpuTextMeasure, LegendElement, RasterImage,
    Renderer, SelectionGeometry,
    config::{AxisScale, LegendCorner, TickVisibility},
    encode_png,
    layout::Side,
    text::RichSegment,
    theme::{ChartKind, SubplotScope, Theme},
};
use std::path::Path;

pub const SIZE: (u32, u32) = (1200, 800);
pub fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}
pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba(
        f32::from(r) / 255.,
        f32::from(g) / 255.,
        f32::from(b) / 255.,
        a,
    )
}
pub fn extent(values: &[f64]) -> (f64, f64) {
    values
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        })
}

/// Clear color is part of each scene, including the area behind axes and labels.
pub fn save(image: &RasterImage, background: Color, path: &Path) -> Result {
    assert_eq!((image.width, image.height), SIZE);
    assert!(image.rgba.chunks_exact(4).all(|p| p[3] == 255));
    let bg = [background.r, background.g, background.b].map(|c| (c * 255.).round() as u8);
    let colored = image
        .rgba
        .chunks_exact(4)
        .filter(|p| {
            let distance: u16 = (0..3).map(|i| p[i].abs_diff(bg[i]) as u16).sum();
            distance > 80 && p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 40
        })
        .count();
    assert!(colored > 2000, "missing chart colors: {colored}");
    std::fs::write(path, encode_png(image)?)?;
    println!("{}: {colored} colored foreground pixels", path.display());
    Ok(())
}

/// The designed scenes' look: the dashboard theme in the scene's background,
/// ink and grid colours.
pub fn theme(bg: Color, ink: Color, grid: Color) -> Theme {
    let mut theme = Theme::named(ChartKind::Line, "default").expect("every kind has a default");
    let chrome = &mut theme.chrome;
    chrome.background = Some(bg);
    chrome.axis_preset = renderer::AxisPreset::Minimal;
    chrome.ink = ink;
    chrome.frame_ink = ink;
    chrome.label_ink = ink;
    chrome.title_size = 34.0;
    chrome.axis_title_size = 23.0;
    chrome.tick_label_size = 21.0;
    chrome.legend_size = 22.0;
    chrome.grid.major_x.show = false;
    chrome.grid.major_x.color = grid;
    chrome.grid.major_y.show = true;
    chrome.grid.major_y.color = grid;
    chrome.grid.major_y.width = 1.0;
    chrome.grid.major_y.style = renderer::line::LineStylePreset::Solid;
    chrome.grid.minor_x.show = false;
    chrome.grid.minor_y.show = false;
    chrome.legend_fill = bg;
    chrome.legend_border = bg;
    chrome.colorbar.border = Color::from_rgba(36. / 255., 41. / 255., 51. / 255., 0.25);
    theme
}

/// The shared chrome: the scene's theme and a top-left legend. The margins
/// are fitted to the labels when the chart is exported.
pub fn chrome(config: &mut Config, bg: Color, ink: Color, grid: Color) {
    theme(bg, ink, grid)
        .apply_chrome(config, &[], SubplotScope::All)
        .expect("a chart without series takes any theme");
    config.legend.visible = true;
    config.legend.corner = LegendCorner::TopLeft;
    config.legend.padding = 15.;
}

/// A scene chart of [`SIZE`] with its titles and a top-left legend, for a
/// built-in theme applied with its series ([`themed`]).
pub fn scene_chart(title: &str, x: &str, y: &str) -> Chart {
    let mut chart = Chart::sized(SIZE.0, SIZE.1)
        .with_title(title)
        .with_x_title(x)
        .with_y_title(y);
    let legend = &mut chart.config_mut().legend;
    legend.visible = true;
    legend.corner = LegendCorner::TopLeft;
    chart
}

// Not every scene uses them.
#[allow(unused_imports)]
pub use crate::gallery::{theme_named, themed};

/// The scene colour of `theme`: its chart-area fill.
pub fn background(theme: &Theme) -> Color {
    theme.chrome.background.unwrap_or(Color::WHITE)
}

pub fn cartesian(title: &str, x: &str, y: &str, bg: Color, ink: Color, grid: Color) -> Chart {
    let mut chart = gallery::chart(title, x, y, SIZE);
    chrome(chart.config_mut(), bg, ink, grid);
    chart
}

pub fn colorbar(config: &mut Config, ramp: ColorMap, min: f64, max: f64, spacing: f64, ink: Color) {
    let mut bar = renderer::default::default_colorbar_options();
    // The gallery images were rendered with this gap.
    bar.gap_px = 24.0;
    bar.side = Side::Right;
    bar.colormap = ramp;
    bar.axis.scale = AxisScale::Linear;
    bar.axis.min = min;
    bar.axis.max = max;
    bar.axis.major_spacing = spacing;
    bar.axis.label_style.color = ink;
    bar.axis.label_style.font_size = 18.;
    bar.axis.tick = TickVisibility::None;
    bar.border_color = Color::from_rgba(ink.r, ink.g, ink.b, 0.25);
    bar.border_width = 1.;
    bar.thickness_px = 16.;
    bar.length_frac = 0.6;
    config.colorbar = Some(bar);
}

/// The legend in one row under the x axis title, centred on the plot: the
/// line breaks between entries become `gap_em` of space, the chart is laid
/// out, the legend is moved below the title, and the chart is laid out
/// again. The legend is the host's placement past the plot: the layout
/// keeps it there and grows the bottom band to hold it (#270).
pub fn legend_row_under_x_title(chart: &mut Chart, gap_em: f32) {
    let legend = &mut chart.config_mut().legend;
    legend.corner = LegendCorner::BottomLeft;
    legend.offset_x = 0.;
    legend.offset_y = 0.;
    for segment in &mut legend.content.segments {
        if segment.text == '\n' {
            *segment = RichSegment::fielded(' ', gap_em, None);
        }
    }
    chart.fit_layout();
    let measure = CpuTextMeasure::for_style(&chart.config().draw_style);
    let cfg = chart.config();
    let title = AxisTitleElement { side: Side::Bottom }.bounds(cfg, &measure);
    let boxed = LegendElement.bounds(cfg, &measure);
    if let (Ok(plot), Some(title), Some(boxed)) = (cfg.data_area(), title, boxed) {
        let offset_x = plot.x as f32 + (plot.width as f32 - boxed.width) * 0.5 - boxed.x;
        let offset_y = title.y + title.height + 10. - boxed.y;
        let legend = &mut chart.config_mut().legend;
        legend.offset_x = offset_x.round();
        legend.offset_y = offset_y.round();
        // Laid out again around the legend in its place.
        chart.fit_layout();
    }
}

/// Fit the chart's margins to its labels and render it on `bg`.
pub fn export(
    r: &mut Renderer,
    chart: &mut Chart,
    specs: &[renderer::SeriesConfig],
    bg: Color,
) -> Result<RasterImage> {
    chart.fit_layout();
    Ok(r.export_rgba_blocking(
        renderer::ExportSource::chart(chart, specs),
        renderer::ExportOptions::panel(1.).with_clear(bg),
    )?)
}
