//! Headless sketch-mode demo — exports PNGs comparing precise vs sketch.
//!
//! Run: `cargo run -p renderer --example sketch_demo`
//! Output: `target/sketch_demo/{precise,sketch,sketch_tour}.png`

use std::sync::Arc;

use renderer::color::Color;
use renderer::config::{DrawStyle, LegendEntryKind, SketchOptions};
use renderer::data::Column;
use renderer::data_config::{
    DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ErrorRef,
    ScatterShape, SeriesConfig,
};
use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::default;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{Chart, RasterImage, Renderer, RendererDevice, encode_png};

fn col(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

/// Export clears to transparent; composite over white for viewing.
fn over_white(img: &mut RasterImage) {
    for px in img.rgba.chunks_exact_mut(4) {
        let a = px[3] as f32 / 255.0;
        for c in &mut px[..3] {
            *c = (*c as f32 * a + 255.0 * (1.0 - a)).round() as u8;
        }
        px[3] = 255;
    }
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], path: &str) {
    let mut img = r.export_panel_rgba(chart, series, 1.0).expect("export");
    over_white(&mut img);
    std::fs::write(path, encode_png(&img).expect("png")).expect("write");
    println!("wrote {path}");
}

fn main() {
    let inst = create_instance();
    let adapter = request_adapter(&inst).expect("adapter");
    let (device, queue) = request_device(&adapter).expect("device");
    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        8 * 1024 * 1024,
    )
    .expect("renderer");

    std::fs::create_dir_all("target/sketch_demo").expect("mkdir");

    // ── Star-history-like S-curve (sparse points → visible markers) ──
    let n = 44;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let stars: Vec<f64> = xs
        .iter()
        .map(|&x| {
            3.0 + 40.0 / (1.0 + (-26.0 * (x - 0.27)).exp())
                + 34.0 / (1.0 + (-8.5 * (x - 0.62)).exp())
        })
        .collect();
    r.add_column("x", &col(xs.clone())).unwrap();
    r.add_column("stars", &col(stars)).unwrap();

    let accent = Color::from_rgb8(214, 86, 54);
    let star_series = [SeriesConfig {
        series_id: "stars".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "stars".into(),
        render_type: DataRenderType::ScatterLine {
            scatter: DataScatterStyleConfig {
                point_color: accent,
                point_shape: ScatterShape::CircleFilled,
                point_size: 6.5,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
            },
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: accent,
                line_width: 3.0,
            },
        },
    }];

    let star_chart = |style: DrawStyle| {
        let mut config = default::default_config();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 920,
            height: 600,
        });
        config.draw_style = style;
        let mut chart = Chart::new(config)
            .with_title("Star history")
            .with_x_title("Date")
            .with_y_title("GitHub Stars")
            .with_legend_entry("figgy/sketch", accent, 3.0, LegendEntryKind::LineScatter);
        chart.set_x_range(-0.02, 1.02);
        chart.set_y_range(0.0, 85.0);
        chart
    };

    export(
        &mut r,
        &star_chart(DrawStyle::Precise),
        &star_series,
        "target/sketch_demo/precise.png",
    );
    export(
        &mut r,
        &star_chart(DrawStyle::Sketch(SketchOptions {
            amplitude_px: 2.2,
            wavelength_px: 55.0,
            seed: 1,
        })),
        &star_series,
        "target/sketch_demo/sketch.png",
    );

    // ── Tour: solid+markers / dashed line / errorbar series + dot grid ──
    let m = 36;
    let txs: Vec<f64> = (0..m).map(|i| i as f64 / (m - 1) as f64).collect();
    let ya: Vec<f64> = txs.iter().map(|&x| 60.0 + 24.0 * (x * 6.0).sin()).collect();
    let yb: Vec<f64> = txs
        .iter()
        .map(|&x| 36.0 + 22.0 * x - 8.0 * (x * 9.0).cos())
        .collect();
    let yc: Vec<f64> = txs
        .iter()
        .map(|&x| 12.0 + 9.0 * (x * 4.5 + 1.2).sin())
        .collect();
    let err: Vec<f64> = txs
        .iter()
        .map(|&x| 2.2 + 1.3 * (x * 7.0).cos().abs())
        .collect();
    r.add_column("t", &col(txs)).unwrap();
    r.add_column("ya", &col(ya)).unwrap();
    r.add_column("yb", &col(yb)).unwrap();
    r.add_column("yc", &col(yc)).unwrap();
    r.add_column("err", &col(err)).unwrap();
    r.add_column("__zero", &col(vec![0.0; m])).unwrap();

    let blue = Color::from_rgb8(38, 104, 214);
    let green = Color::from_rgb8(44, 140, 80);
    let tour_series = [
        SeriesConfig {
            series_id: "wave".into(),
            source_id: None,
            label: None,
            x_column: "t".into(),
            y_column: "ya".into(),
            render_type: DataRenderType::ScatterLine {
                scatter: DataScatterStyleConfig {
                    point_color: accent,
                    point_shape: ScatterShape::CircleFilled,
                    point_size: 5.5,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                },
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: accent,
                    line_width: 2.5,
                },
            },
        },
        SeriesConfig {
            series_id: "trend".into(),
            source_id: None,
            label: None,
            x_column: "t".into(),
            y_column: "yb".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Dash,
                    line_color: blue,
                    line_width: 2.5,
                },
            },
        },
        SeriesConfig {
            series_id: "meas".into(),
            source_id: None,
            label: None,
            x_column: "t".into(),
            y_column: "yc".into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: DataScatterStyleConfig {
                    point_color: green,
                    point_shape: ScatterShape::Diamond,
                    point_size: 5.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                },
                err_y: ErrorRef::Symmetric {
                    column: "err".into(),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: green,
                    error_bar_width: 1.6,
                    error_bar_cap_size: 6.0,
                    cap_width: 1.6,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                },
            },
        },
    ];

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 920,
        height: 600,
    });
    config.grid.show_major_x = true;
    config.grid.show_major_y = true;
    config.grid.show_minor_x = true;
    config.grid.show_minor_y = true;
    config.grid.minor_x_style = LineStylePreset::Dot;
    config.grid.minor_y_style = LineStylePreset::Dot;
    config.draw_style = DrawStyle::Sketch(SketchOptions {
        amplitude_px: 2.5,
        wavelength_px: 45.0,
        seed: 3,
    });
    let mut chart = Chart::new(config)
        .with_title("figgy draw_style tour — sketch")
        .with_x_title("t")
        .with_y_title("value")
        .with_legend_entry("solid + markers", accent, 2.5, LegendEntryKind::LineScatter)
        .with_legend_entry("dashed line", blue, 2.5, LegendEntryKind::Line)
        .with_legend_entry("scatter + errorbar", green, 2.5, LegendEntryKind::Scatter);
    chart.set_x_range(-0.03, 1.03);
    chart.set_y_range(0.0, 95.0);

    export(
        &mut r,
        &chart,
        &tour_series,
        "target/sketch_demo/sketch_tour.png",
    );
}
