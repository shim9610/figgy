//! Constellation Step-1 demo — line element as a star chain over a nebula
//! ribbon. Exports parameter-sweep PNGs for visual review.
//!
//! Run: `cargo run -p renderer --example constellation_demo`
//! Output: `target/constellation_demo/*.png`

use std::sync::Arc;

use renderer::color::Color;
use renderer::config::{ConstellationOptions, DrawStyle};
use renderer::data::Column;
use renderer::data_config::{DataLineStyleConfig, DataRenderType, SeriesConfig};
use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::default;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{encode_png, Chart, RasterImage, Renderer, RendererDevice};

fn col(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

/// Composite the transparent export over a deep-space backdrop.
fn over_space(img: &mut RasterImage) {
    const BG: [f32; 3] = [11.0, 15.0, 23.0];
    for px in img.rgba.chunks_exact_mut(4) {
        let a = px[3] as f32 / 255.0;
        for (c, bg) in px[..3].iter_mut().zip(BG) {
            *c = (*c as f32 * a + bg * (1.0 - a)).round().min(255.0) as u8;
        }
        px[3] = 255;
    }
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], path: &str) {
    let mut img = r.export_panel_rgba(chart, series, 1.0).expect("export");
    over_space(&mut img);
    let ink = img.rgba.chunks_exact(4).filter(|p| p[0] > 30 || p[1] > 35 || p[2] > 45).count();
    std::fs::write(path, encode_png(&img).expect("png")).expect("write");
    println!("wrote {path} (bright px: {ink})");
}

fn line_series(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: color,
                line_width: 2.0,
            },
        },
    }
}

fn build_chart(style: DrawStyle) -> Chart {
    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect { x: 0, y: 0, width: 960, height: 620 });
    config.draw_style = style;
    // Dark-backdrop chrome: light axis/label/title colors via plain SSoT
    // fields; no grid; no legend box (its white fill fights the backdrop).
    let chrome = Color::from_rgb8(186, 194, 210);
    for axis in [
        &mut config.top_x, &mut config.bottom_x, &mut config.left_y, &mut config.right_y,
    ] {
        axis.line_color = chrome;
        axis.label_style.color = chrome;
        axis.title_option.text.color = chrome;
    }
    config.chart_title.text.color = Color::from_rgb8(214, 220, 232);
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    config.legend.visible = false;

    let mut chart = Chart::new(config)
        .with_title("Constellation — line as star chain")
        .with_x_title("t")
        .with_y_title("value");
    chart.set_x_range(-0.03, 1.03);
    chart.set_y_range(0.0, 100.0);
    chart
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

    std::fs::create_dir_all("target/constellation_demo").expect("mkdir");

    // Two smooth curves — does the star chain read as a curve?
    let n = 90;
    let ts: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let warm: Vec<f64> = ts.iter().map(|&t| 62.0 + 26.0 * (t * 5.2).sin()).collect();
    let cool: Vec<f64> = ts
        .iter()
        .map(|&t| 14.0 + 30.0 * t + 9.0 * (t * 8.0 + 1.0).cos())
        .collect();
    r.add_column("t", &col(ts)).unwrap();
    r.add_column("warm", &col(warm)).unwrap();
    r.add_column("cool", &col(cool)).unwrap();

    let series = [
        line_series("nebula_warm", "t", "warm", Color::from_rgb8(255, 142, 92)),
        line_series("nebula_cool", "t", "cool", Color::from_rgb8(96, 168, 255)),
    ];

    let cons = |density: f32, width: f32, intensity: f32, seed: u32| {
        DrawStyle::Constellation(ConstellationOptions {
            star_density: density,
            ribbon_width_px: width,
            ribbon_intensity: intensity,
            seed,
            ..ConstellationOptions::default()
        })
    };

    let cases = [
        ("main", cons(14.0, 14.0, 0.30, 0)),
        ("density_low", cons(6.0, 14.0, 0.30, 0)),
        ("density_high", cons(28.0, 14.0, 0.30, 0)),
        ("ribbon_thin", cons(14.0, 8.0, 0.15, 0)),
        ("ribbon_thick", cons(14.0, 26.0, 0.50, 0)),
        ("seed7", cons(14.0, 14.0, 0.30, 7)),
        // Bisection debug pair: isolate each pass.
        ("dbg_ribbon_only", cons(0.0, 14.0, 0.30, 0)),
        ("dbg_stars_only", cons(14.0, 14.0, 0.0, 0)),
    ];
    let faint_cases = [
        // Luminosity-function slope sweep: more faint dust per anchor.
        ("faint_default", 14.0, 3.0),
        ("faint_heavy", 26.0, 5.5),
        ("faint_extreme", 40.0, 8.0),
    ];
    for (name, style) in cases {
        export(
            &mut r,
            &build_chart(style),
            &series,
            &format!("target/constellation_demo/{name}.png"),
        );
    }
    for (name, density, bias) in faint_cases {
        let style = DrawStyle::Constellation(ConstellationOptions {
            star_density: density,
            faint_bias: bias,
            ..ConstellationOptions::default()
        });
        export(
            &mut r,
            &build_chart(style),
            &series,
            &format!("target/constellation_demo/{name}.png"),
        );
    }
}
