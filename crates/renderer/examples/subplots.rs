//! Subplots: temperature in the parent's area, and below it a second area
//! with pressure on its own left Y axis and humidity on a twin right axis.
//! One document, one X range, exported as one PNG.
//! cargo run --locked -p figgy-renderer --example subplots -- target/subplots.png
use renderer::{
    Chart, Color, HitMap, LayerRef, OverlayLayer, Renderer, RendererDevice, SeriesConfig,
    data::Column,
    data_config::{DataLineStyleConfig, DataRenderType},
    data_render::{create_instance, request_adapter, request_device},
    default, encode_png,
    layout::{ChartArea, Rect},
    line::LineStylePreset,
    text::RichText,
};
use std::sync::Arc;

fn line(id: &str, y: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "hour".into(),
        y_column: y.into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: color,
                line_width: 2.5,
                fill: None,
                fill_color: None,
                interpolation: renderer::data_config::LineInterpolation::Linear,
            },
        },
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let mut renderer = Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )?;

    // One day of hourly readings in three units.
    let hour: Vec<f64> = (0..=24).map(f64::from).collect();
    let wave = |phase: f64| {
        hour.iter()
            .map(move |h| (h / 24.0 * std::f64::consts::TAU + phase).sin())
            .collect::<Vec<f64>>()
    };
    let temperature: Vec<f64> = wave(-1.2).iter().map(|v| 18.0 + 8.0 * v).collect();
    let pressure: Vec<f64> = wave(0.0).iter().map(|v| 1008.0 + 6.0 * v).collect();
    let humidity: Vec<f64> = wave(2.0).iter().map(|v| 60.0 + 25.0 * v).collect();
    let extent = |values: &[f64]| {
        values
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(*v), hi.max(*v))
            })
    };
    for (id, data) in [
        ("hour", hour.clone()),
        ("temperature", temperature),
        ("pressure", pressure),
        ("humidity", humidity),
    ] {
        let (min, max) = extent(&data);
        renderer.add_column(id, &Column { data, min, max })?;
    }

    // The parent is the upper area; the subplot area sits below it. The
    // document covers both.
    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 960,
        height: 420,
    });
    config.left_y.out_margin = 110.0;
    config.right_y.out_margin = 110.0;
    config.bottom_x.out_margin = 70.0;
    config.top_x.out_margin = 24.0;
    config.right_y.line_visible = false;
    config.right_y.tick = renderer::config::TickVisibility::None;

    // Both layers of the subplot name the same area: the first draws its
    // axis on the left with the grid and the parent's X axes, the second
    // its own axis on the right.
    let area = Some(ChartArea(Rect {
        x: 0,
        y: 420,
        width: 960,
        height: 420,
    }));
    let pressure_color = Color::from_rgb8(176, 64, 32);
    let humidity_color = Color::from_rgb8(40, 140, 90);
    let mut pressure = OverlayLayer::default();
    pressure.series = vec!["pressure".into()];
    pressure.chart_area = area.clone();
    pressure.y_axis.out_margin = 110.0;
    pressure.y_axis.title_option.visible = true;
    pressure.y_axis.title_option.text = RichText::plain("Pressure (hPa)", pressure_color, 22.0, "");
    pressure.y_axis.label_style.color = pressure_color;
    let mut humidity = OverlayLayer::default();
    humidity.series = vec!["humidity".into()];
    humidity.chart_area = area;
    humidity.y_axis.out_margin = 110.0;
    humidity.y_axis.title_option.visible = true;
    humidity.y_axis.title_option.text = RichText::plain("Humidity (%)", humidity_color, 22.0, "");
    humidity.y_axis.label_style.color = humidity_color;
    config.overlays = vec![pressure, humidity];

    let mut chart = Chart::new(config)
        .with_title("Temperature, pressure and humidity")
        .with_x_title("Hour")
        .with_y_title("Temperature (°C)");
    chart.set_x_range(0.0, 24.0);
    chart.auto_fit_y(renderer.pool(), "temperature", 0.1)?;
    chart.auto_fit_overlay_y_union(renderer.pool(), 0, &["pressure"], 0.1)?;
    chart.auto_fit_overlay_y_union(renderer.pool(), 1, &["humidity"], 0.1)?;

    let series = [
        line("temperature", "temperature", Color::from_rgb8(34, 104, 198)),
        line("pressure", "pressure", pressure_color),
        line("humidity", "humidity", humidity_color),
    ];
    // One merged legend in the parent's area lists every layer's series.
    let mut legend = chart.config().legend.clone();
    legend.visible = true;
    legend.content.font_size = 20.0;
    for item in &series {
        renderer::config::append_legend_entry(
            &mut legend.content,
            renderer::config::series_symbol_segments(item),
            &item.series_id,
        );
    }
    chart.config_mut().legend = legend;

    // The two layers of the subplot share one data area inside their area.
    let document = chart.config();
    assert_eq!(document.document_area().height, 840);
    let shared = document
        .overlay_data_area(0)
        .map_err(|error| format!("subplot layout: {error:?}"))?;
    assert_eq!(document.overlay_data_area(1).ok(), Some(shared));
    // Pointer interaction addresses one layer at a time; the subplot's first
    // layer answers with its data area and its left axis.
    let mut interactive = document.clone();
    interactive.set_active_layer(LayerRef::Overlay(0));
    let map = HitMap::for_active_layer(&interactive);
    assert!(map.find("overlay_0:data_area").is_some());
    assert!(map.find("overlay_0:axis_left").is_some());

    let image = pollster::block_on(renderer.export_document_rgba_with_clear_async(
        &chart,
        &series,
        1.0,
        Color::WHITE,
    ))?;
    let output = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "subplots.png".into());
    if let Some(parent) = std::path::Path::new(&output).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, encode_png(&image)?)?;
    renderer.wait_idle();
    println!(
        "{}: {}x{}",
        std::path::Path::new(&output).display(),
        image.width,
        image.height
    );
    Ok(())
}
