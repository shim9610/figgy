//! Independent rich titles on pie and radar charts.
//! cargo run --locked -p figgy-renderer --example bounded_titles -- target/bounded-titles
use renderer::{
    BoundedTitleOptions, Category, Color, RadarChart, RadarRenderer, RadarSeries, RadialChart,
    RadialRenderer, RadialSlice, RendererDevice, TextAlign,
    data_render::{create_instance, request_adapter, request_device},
    encode_png,
};
use std::{path::PathBuf, sync::Arc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/bounded-titles"));
    std::fs::create_dir_all(&output)?;
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
    let color = Color::from_rgb8(70, 130, 180);
    let mut title =
        BoundedTitleOptions::from_legacy("Independent title\nH2O", "sans-serif", 16., Color::BLACK);
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
        // The radial chart holds the shared chart title; the other three
        // charts still hold the bounded one.
        chart_title: Some(renderer::config::ChartTitleOptions {
            text: title.text.clone(),
            visible: title.visible,
            ..renderer::config::ChartTitleOptions::from_legacy_plain("", "", 16., Color::BLACK)
        }),
        font_size: 12.,
        slices: vec![
            RadialSlice::new("A", 30., color),
            RadialSlice::new("B", 70., Color::from_rgb8(180, 100, 90)),
        ],
        ..Default::default()
    };
    pie.format_labels();
    let radar = RadarChart {
        chart_title: Some(title),
        font_size: 12.,
        categories: vec![
            Category::new("a", "A"),
            Category::new("b", "B"),
            Category::new("c", "C"),
        ],
        series: vec![RadarSeries::new("s", "Values", vec![3., 7., 5.], color)],
        ..Default::default()
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut radial = RadialRenderer::new(gpu.clone(), format)?;
    let mut radar_renderer = RadarRenderer::new(gpu, format)?;
    for (name, image) in [
        ("pie", radial.export_rgba(&pie, (800, 520), 2.)?),
        ("radar", radar_renderer.export_rgba(&radar, (800, 520), 2.)?),
    ] {
        std::fs::write(output.join(format!("{name}.png")), encode_png(&image)?)?;
    }
    Ok(())
}
