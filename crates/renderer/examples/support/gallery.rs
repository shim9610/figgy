//! Shared native device setup, presentation defaults and PNG checks for gallery examples.
//! Chart data and series definitions live in each example, not in this module.
use renderer::{
    Chart, Color, Renderer, RendererDevice, SeriesConfig,
    data::Column,
    data_config::{DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ScatterShape},
    data_render::{create_instance, request_adapter, request_device},
    default, encode_png,
    layout::{ChartArea, Rect},
    line::LineStylePreset,
};
use std::{error::Error, path::Path, sync::Arc};

pub type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

pub fn column(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

pub fn line(color: Color, width: f32) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: LineStylePreset::Solid,
        line_color: color,
        line_width: width,
        fill: None,
        fill_color: None,
        interpolation: renderer::data_config::LineInterpolation::Linear,
    }
}

pub fn scatter(color: Color, size: f32) -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: color,
        point_shape: ScatterShape::CircleFilled,
        point_size: size,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    }
}

pub fn series(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type,
    }
}

pub fn chart(title: &str, x_title: &str, y_title: &str, size: (u32, u32)) -> Chart {
    let mut cfg = default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: size.0,
        height: size.1,
    });
    cfg.legend.visible = false;
    cfg.chart_title.text.font_size = 26.0;
    cfg.chart_title.text.color = Color::from_rgb8(24, 39, 63);
    cfg.chart_title.top_margin = 48.0;
    cfg.grid.major_x_color = Color::from_rgb8(230, 235, 242);
    cfg.grid.major_y_color = cfg.grid.major_x_color;
    cfg.grid.show_minor_x = false;
    cfg.grid.show_minor_y = false;
    for axis in [
        &mut cfg.bottom_x,
        &mut cfg.top_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.label_style.font_size = 16.0;
        axis.title_option.text.font_size = 18.0;
        axis.line_color = Color::from_rgb8(88, 102, 123);
    }
    cfg.left_y.out_margin = 90.0;
    cfg.bottom_x.out_margin = 70.0;
    cfg.top_x.out_margin = 16.0;
    cfg.right_y.out_margin = 24.0;
    Chart::new(cfg)
        .with_title(title)
        .with_x_title(x_title)
        .with_y_title(y_title)
}

pub fn save(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], path: &Path) -> Result {
    let img =
        pollster::block_on(r.export_panel_rgba_with_clear_async(chart, series, 1.0, Color::WHITE))?;
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
    // Instance and adapter stay alive until rendering and readback finish.
    // Software GPU selection belongs to the environment, not the example.
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("Gallery adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let mut renderer = Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        16 * 1024 * 1024,
    )?;
    let result = render(&mut renderer);
    renderer.wait_idle();
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
