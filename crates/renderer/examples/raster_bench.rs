//! CPU raster cost probe — times the per-refresh layer rasters that run on
//! every `refresh_axis` (= every set_config / pan / zoom commit), per style.
//! No GPU needed.
//!
//! Run: `cargo run --release -p renderer --example raster_bench`

use std::time::Instant;

use renderer::axis_render::{try_raster_chart_layer_to_rgba, AxisLayerKind};
use renderer::color::Color;
use renderer::config::{DrawStyle, MilkywayOptions};
use renderer::default;
use renderer::layout::{ChartArea, Rect};

fn build_config(w: u32, h: u32, style: DrawStyle) -> renderer::config::Config {
    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect { x: 0, y: 0, width: w, height: h });
    config.draw_style = style;
    let chrome = Color::from_rgb8(186, 194, 210);
    for axis in [
        &mut config.top_x, &mut config.bottom_x, &mut config.left_y, &mut config.right_y,
    ] {
        axis.line_color = chrome;
        axis.label_style.color = chrome;
        axis.title_option.text.color = chrome;
    }
    config
}

fn time_layer(config: &renderer::config::Config, layer: AxisLayerKind) -> f64 {
    // Warm-up (font db init etc.), then median of 5.
    let _ = try_raster_chart_layer_to_rgba(config, layer).unwrap();
    let mut samples: Vec<f64> = (0..5)
        .map(|_| {
            let t = Instant::now();
            let _ = try_raster_chart_layer_to_rgba(config, layer).unwrap();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[2]
}

fn main() {
    for (w, h) in [(960u32, 620u32), (2000, 1600)] {
        for (name, style) in [
            ("precise", DrawStyle::Precise),
            (
                "milkyway",
                DrawStyle::Milkyway(MilkywayOptions::default()),
            ),
        ] {
            let config = build_config(w, h, style);
            let grid = time_layer(&config, AxisLayerKind::Grid);
            let deco = time_layer(&config, AxisLayerKind::Decoration);
            println!(
                "{w}x{h} {name:>13}: grid {grid:7.2} ms | deco(+glow) {deco:7.2} ms | refresh total ~{:7.2} ms",
                grid + deco
            );
        }
    }
}
