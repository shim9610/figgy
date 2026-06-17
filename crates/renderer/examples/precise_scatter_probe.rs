//! Minimal precise-style scatter render probe — regression check for the
//! "plain scatter renders a black panel" report.
//!
//! Run: `cargo run -p renderer --example precise_scatter_probe`

use std::sync::Arc;

use renderer::color::Color;
use renderer::config::DrawStyle;
use renderer::data::Column;
use renderer::data_config::{DataRenderType, DataScatterStyleConfig, ScatterShape, SeriesConfig};
use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::default;
use renderer::layout::{ChartArea, Rect};
use renderer::{Chart, Renderer, RendererDevice, encode_png};

fn col(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

fn main() {
    let inst = create_instance();
    let adapter = request_adapter(&inst).expect("adapter");
    let (device, queue) = request_device(&adapter).expect("device");
    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer");

    let n = 40;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let ys: Vec<f64> = xs
        .iter()
        .map(|x| 20.0 + 60.0 * (x * 6.0).sin().abs())
        .collect();
    r.add_column("x", &col(xs)).unwrap();
    r.add_column("y", &col(ys)).unwrap();

    let series = [SeriesConfig {
        series_id: "pts".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: Color::from_rgb8(220, 60, 40),
                point_shape: ScatterShape::CircleFilled,
                point_size: 6.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
            },
        },
    }];

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 800,
        height: 520,
    });
    config.draw_style = DrawStyle::Precise;
    let mut chart = Chart::new(config);
    chart.set_x_range(-0.05, 1.05);
    chart.set_y_range(0.0, 100.0);

    let img = r.export_panel_rgba(&chart, &series, 1.0).expect("export");
    let mut red = 0usize;
    let mut opaque = 0usize;
    for px in img.rgba.chunks_exact(4) {
        if px[3] > 16 {
            opaque += 1;
            if px[0] > 120 && px[1] < 90 && px[2] < 90 {
                red += 1;
            }
        }
    }
    std::fs::create_dir_all("target/probe").unwrap();
    std::fs::write(
        "target/probe/precise_scatter.png",
        encode_png(&img).unwrap(),
    )
    .unwrap();
    println!("wrote target/probe/precise_scatter.png  opaque px: {opaque}  red marker px: {red}");
}
