//! Minimal native line chart: upload two columns and write a PNG.
//! cargo run --locked -p figgy-renderer --example line
use renderer::{
    Chart, Color, Renderer, RendererDevice, SeriesConfig,
    data::Column,
    data_config::{DataLineStyleConfig, DataRenderType},
    default, encode_png,
    gpu::{create_instance, request_adapter_blocking, request_device_blocking},
    layout::{ChartArea, Rect},
    line::LineStylePreset,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let instance = create_instance();
    let adapter = request_adapter_blocking(&instance)?;
    eprintln!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device_blocking(&adapter)?;
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue)?,
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024, // initial GPU column pool size in bytes
    )?;

    let x: Vec<f64> = (0..=100).map(|i| i as f64 / 10.0).collect();
    let y: Vec<f64> = x.iter().map(|x| x.sin()).collect();
    renderer.add_column("x", &Column { data: x })?;
    renderer.add_column("y", &Column { data: y })?;

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 960,
        height: 600,
    });
    config.legend.visible = false;
    config.left_y.out_margin = 80.0;
    config.right_y.out_margin = 40.0;
    config.bottom_x.out_margin = 70.0;
    config.top_x.out_margin = 24.0;
    let mut chart = Chart::new(config)
        .with_title("A sine wave")
        .with_x_title("Time (s)")
        .with_y_title("Amplitude");
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-1.2, 1.2);
    let series = [SeriesConfig {
        series_id: "sine".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Xy(renderer::data_config::XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::from_rgb8(34, 104, 198),
                line_width: 2.5,
                fill: None,
                fill_color: None,
                interpolation: renderer::data_config::LineInterpolation::Linear,
            }),
            errors: None,
        }),
    }];
    let image = renderer.export_rgba_blocking(
        renderer::ExportSource::chart(&chart, &series),
        renderer::ExportOptions::panel(1.0).with_clear(Color::WHITE),
    )?;
    let output = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "line.png".into());
    std::fs::write(&output, encode_png(&image)?)?;
    renderer.wait_idle()?;
    println!(
        "{}: {}x{}",
        std::path::Path::new(&output).display(),
        image.width,
        image.height
    );
    Ok(())
}
