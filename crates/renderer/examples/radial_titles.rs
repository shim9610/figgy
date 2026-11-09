//! An independent rich title on a pie chart.
//! cargo run --locked -p figgy-renderer --example radial_titles -- target/radial-titles
use renderer::{
    BoundedTitleOptions, Color, ExportOptions, RadialChart, RadialRenderer, RadialSlice,
    RendererDevice, TextAlign,
    gpu::{create_instance, request_adapter_blocking, request_device_blocking},
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/radial-titles"));
    std::fs::create_dir_all(&output)?;
    let instance = create_instance();
    let adapter = request_adapter_blocking(&instance)?;
    eprintln!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device_blocking(&adapter)?;
    let gpu = RendererDevice::new(device, queue).expect("figgy poll thread");
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
        // The radial chart holds the shared chart title, built here from
        // the bounded title type a legacy document carries.
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
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut radial = RadialRenderer::try_new(gpu, format)?;
    let options = ExportOptions::document(2.);
    radial.save_blocking(&pie, (800, 520), options, output.join("pie.png"))?;
    Ok(())
}
