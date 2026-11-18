//! An independent rich title on a pie chart.
//! cargo run --locked -p figgy-renderer --example radial_titles -- target/radial-titles
use renderer::{
    Color, ExportOptions, RadialChart, RadialRenderer, RadialSlice, TextAlign,
    config::ChartTitleOptions,
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/radial-titles"));
    std::fs::create_dir_all(&output)?;
    let color = Color::from_rgb8(70, 130, 180);
    // A rich title of its own font and size, apart from the chart's labels.
    let mut title = ChartTitleOptions::from_legacy_plain(
        "Independent title\nH2O",
        "sans-serif",
        16.,
        Color::BLACK,
    );
    title.text.align = TextAlign::Center;
    title.text.segments[0].bold = true;
    title
        .text
        .segments
        .iter_mut()
        .find(|s| s.text == '2')
        .unwrap()
        .subscript = true;
    let mut pie = RadialChart {
        chart_title: Some(title),
        font_size: 12.,
        slices: vec![
            RadialSlice::new("A", 30., color),
            RadialSlice::new("B", 70., Color::from_rgb8(180, 100, 90)),
        ],
        ..Default::default()
    };
    pie.format_labels();
    let mut radial = RadialRenderer::headless_blocking()?;
    let options = ExportOptions::document(2.);
    radial.save_blocking(&pie, (800, 520), options, output.join("pie.png"))?;
    Ok(())
}
