//! The restrained chart chrome shared by the designed gallery scenes: no axis
//! lines or ticks, a horizontal grid, a quiet legend, an opaque background,
//! 1200 x 800 pixels.
use crate::gallery::{self, Result};
use renderer::{
    Chart, Color, ColorMap, Config, RasterImage, Renderer,
    config::{AxisScale, LegendCorner, TickVisibility},
    encode_png,
    layout::Side,
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

/// The shared chrome: no axis lines or ticks, horizontal grid only, a quiet
/// legend in the background colour.
pub fn chrome(config: &mut Config, bg: Color, ink: Color, grid: Color) {
    config.chart_title.text.font_size = 34.;
    config.chart_title.text.color = ink;
    config.chart_title.top_margin = 85.;
    config.left_y.out_margin = 110.;
    config.right_y.out_margin = 65.;
    config.bottom_x.out_margin = 95.;
    config.top_x.out_margin = 45.;
    config.grid.major_y_color = grid;
    config.grid.major_x_color = grid;
    config.grid.show_major_x = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    for a in [
        &mut config.left_y,
        &mut config.right_y,
        &mut config.bottom_x,
        &mut config.top_x,
    ] {
        a.line_visible = false;
        a.tick = TickVisibility::None;
        a.label_style.color = ink;
        a.label_style.font_size = 21.;
        a.title_option.text.color = ink;
        a.title_option.text.font_size = 23.;
    }
    config.legend.visible = true;
    config.legend.corner = LegendCorner::TopLeft;
    config.legend.bg_color = bg;
    config.legend.border_color = bg;
    config.legend.content.color = ink;
    config.legend.content.font_size = 22.;
    config.legend.padding = 15.;
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

pub fn export(
    r: &mut Renderer,
    chart: &Chart,
    specs: &[renderer::SeriesConfig],
    bg: Color,
) -> Result<RasterImage> {
    Ok(pollster::block_on(
        r.export_panel_rgba_with_clear_async(chart, specs, 1., bg),
    )?)
}
