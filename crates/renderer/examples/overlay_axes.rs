//! Twin Y axes: a temperature series on the base layer and a pressure series
//! on an overlay layer with its own right-hand Y axis, exported as one PNG.
//! cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes.png
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

    // One day of hourly readings in two units that cannot share one Y range.
    let hour: Vec<f64> = (0..=24).map(f64::from).collect();
    let temperature: Vec<f64> = hour
        .iter()
        .map(|h| 18.0 + 8.0 * ((h - 14.0) / 24.0 * std::f64::consts::TAU).cos())
        .collect();
    let pressure: Vec<f64> = hour
        .iter()
        .map(|h| 1008.0 + 6.0 * (h / 24.0 * std::f64::consts::TAU).sin() - 0.2 * h)
        .collect();
    let extent = |values: &[f64]| {
        values
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(*v), hi.max(*v))
            })
    };
    for (id, data) in [
        ("hour", hour),
        ("temperature", temperature),
        ("pressure", pressure),
    ] {
        let (min, max) = extent(&data);
        renderer.add_column(id, &Column { data, min, max })?;
    }

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 960,
        height: 600,
    });
    config.left_y.out_margin = 80.0;
    config.bottom_x.out_margin = 70.0;
    config.top_x.out_margin = 24.0;
    // The parent's own right axis is decoration only; hide its line and ticks
    // so the overlay axis has the right band to itself.
    config.right_y.line_visible = false;
    config.right_y.tick = renderer::config::TickVisibility::None;

    // The overlay owns the pressure series and a right-hand Y axis. Its
    // band widens the shared right margin; the data area is computed once
    // for both layers.
    let mut overlay = OverlayLayer::default();
    overlay.series = vec!["pressure".into()];
    overlay.y_axis.out_margin = 90.0;
    overlay.y_axis.title_option.visible = true;
    overlay.y_axis.title_option.text =
        RichText::plain("Pressure (hPa)", Color::from_rgb8(176, 64, 32), 22.0, "");
    overlay.y_axis.label_style.color = Color::from_rgb8(176, 64, 32);
    overlay.y_axis.line_color = Color::from_rgb8(176, 64, 32);
    config.overlays.push(overlay);

    let mut chart = Chart::new(config)
        .with_title("Temperature and pressure")
        .with_x_title("Hour")
        .with_y_title("Temperature (°C)");
    chart.set_x_range(0.0, 24.0);
    chart.auto_fit_y(renderer.pool(), "temperature", 0.1)?;
    chart.auto_fit_overlay_y_union(renderer.pool(), 0, &["pressure"], 0.1)?;

    let series = [
        line("temperature", "temperature", Color::from_rgb8(34, 104, 198)),
        line("pressure", "pressure", Color::from_rgb8(176, 64, 32)),
    ];
    // The merged legend lists both layers' series; `OverlayLayer::legend`
    // would give the overlay a legend of its own instead.
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

    // Every internal panel shares the one data area the document reports.
    let expansion = chart
        .config()
        .expand()
        .map_err(|error| format!("expansion: {error:?}"))?;
    for panel in &expansion.panels {
        let area = panel
            .config
            .data_area()
            .map_err(|error| format!("panel layout: {error:?}"))?;
        assert_eq!(area, expansion.data_area);
    }
    // Pointer interaction addresses one layer at a time; the overlay's axis
    // answers as `overlay_0:axis_right` while it is active.
    let mut interactive = chart.config().clone();
    interactive.set_active_layer(LayerRef::Overlay(0));
    let map = HitMap::for_active_layer(&interactive);
    assert!(map.find("overlay_0:axis_right").is_some());

    let image = pollster::block_on(renderer.export_document_rgba_with_clear_async(
        &chart,
        &series,
        1.0,
        Color::WHITE,
    ))?;
    let output = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "overlay_axes.png".into());
    if let Some(parent) = std::path::Path::new(&output).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, encode_png(&image)?)?;
    renderer.wait_idle();
    println!(
        "{}: {}x{}, {} layers",
        std::path::Path::new(&output).display(),
        image.width,
        image.height,
        expansion.panels.len()
    );
    Ok(())
}
