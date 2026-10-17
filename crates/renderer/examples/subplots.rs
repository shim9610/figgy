//! Subplots: a 2x2 grid of charts with their own axes in one document.
//! The upper-right chart links its X axis to the upper-left one; every other
//! axis is independent, and the lower-right chart has a twin Y axis of its
//! own. Exported as one PNG.
//! cargo run --locked -p figgy-renderer --example subplots -- target/subplots.png
use renderer::{
    CellRef, Chart, Color, Config, HitMap, LayerRef, OverlayLayer, Renderer, RendererDevice,
    SeriesConfig, Subplot,
    data::Column,
    data_config::{DataLineStyleConfig, DataRenderType},
    data_render::{create_instance, request_adapter, request_device},
    default, encode_png,
    layout::{ChartArea, Rect},
    line::LineStylePreset,
    text::RichText,
};
use std::sync::Arc;

fn line(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
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

    // One 480x420 cell: a chart with its own axes, titled.
    let cell = |x: u32, y: u32, title: &str, x_title: &str, y_title: &str| -> Config {
        let mut config = default::default_config();
        config.chart_area = ChartArea(Rect {
            x,
            y,
            width: 480,
            height: 420,
        });
        config.left_y.out_margin = 100.0;
        config.right_y.out_margin = 40.0;
        config.bottom_x.out_margin = 70.0;
        config.top_x.out_margin = 16.0;
        Chart::new(config)
            .with_title(title)
            .with_x_title(x_title)
            .with_y_title(y_title)
            .config()
            .clone()
    };
    let subplot = |config: Config, series: &[&str]| Subplot {
        config,
        series: series.iter().map(|id| id.to_string()).collect(),
        link_x: None,
        link_y: None,
    };

    // The document's own chart is the upper-left cell; three subplots fill
    // the grid. Only the upper-right one links an axis (X, to the parent).
    let mut config = cell(0, 0, "Temperature", "Hour", "°C");
    let mut pressure_by_hour = subplot(cell(480, 0, "Pressure", "Hour", "hPa"), &["pressure"]);
    pressure_by_hour.link_x = Some(CellRef::Parent);
    let humidity_by_temperature = subplot(
        cell(0, 420, "Humidity against temperature", "°C", "%"),
        &["humidity_t"],
    );
    let pressure_color = Color::from_rgb8(176, 64, 32);
    let humidity_color = Color::from_rgb8(40, 140, 90);
    let mut daytime = subplot(cell(480, 420, "Daytime", "Hour", "hPa"), &["daytime"]);
    daytime.config.right_y.out_margin = 100.0;
    let mut twin = OverlayLayer::default();
    twin.series = vec!["daytime_humidity".into()];
    twin.y_axis.out_margin = 100.0;
    twin.y_axis.title_option.visible = true;
    twin.y_axis.title_option.text = RichText::plain("%", humidity_color, 22.0, "");
    twin.y_axis.label_style.color = humidity_color;
    daytime.config.overlays.push(twin);
    config.subplots = vec![pressure_by_hour, humidity_by_temperature, daytime];

    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 24.0);
    chart.auto_fit_y(renderer.pool(), "temperature", 0.1)?;
    // Subplot 0's X follows the parent; its Y is its own.
    chart.auto_fit_subplot_y_union(renderer.pool(), 0, &["pressure"], 0.1)?;
    chart.auto_fit_subplot_x_union(renderer.pool(), 1, &["temperature"], 0.05)?;
    chart.auto_fit_subplot_y_union(renderer.pool(), 1, &["humidity"], 0.1)?;
    chart.set_subplot_x_range(2, 6.0, 18.0);
    chart.auto_fit_subplot_y_union(renderer.pool(), 2, &["pressure"], 0.1)?;
    chart.with_axis_range_change(|config| {
        let overlay = &mut config.subplots[2].config.overlays[0].y_axis;
        overlay.min = 30.0;
        overlay.max = 90.0;
        overlay.major_spacing = 10.0;
    });

    let series = [
        line(
            "temperature",
            "hour",
            "temperature",
            Color::from_rgb8(34, 104, 198),
        ),
        line("pressure", "hour", "pressure", pressure_color),
        line("humidity_t", "temperature", "humidity", humidity_color),
        line("daytime", "hour", "pressure", pressure_color),
        line("daytime_humidity", "hour", "humidity", humidity_color),
    ];

    // Every cell is a chart of its own with its own data area.
    let document = chart.config();
    assert_eq!(document.document_area().width, 960);
    assert_eq!(document.document_area().height, 840);
    let linked = document.subplot_config(0).ok_or("subplot 0 is missing")?;
    assert_eq!(linked.bottom_x.max, document.bottom_x.max);
    // Pointer interaction addresses one chart at a time; a subplot answers
    // with its own elements under `subplot_<n>:` ids.
    let mut interactive = document.clone();
    interactive.set_active_layer(LayerRef::Subplot(2));
    let map = HitMap::for_active_layer(&interactive);
    assert!(map.find("subplot_2:data_area").is_some());
    interactive.set_active_layer(LayerRef::SubplotOverlay(2, 0));
    let map = HitMap::for_active_layer(&interactive);
    assert!(map.find("subplot_2:overlay_0:axis_right").is_some());

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
