//! Constellation Step-1 demo — line element as a star chain over a nebula
//! ribbon. Exports parameter-sweep PNGs for visual review.
//!
//! Run: `cargo run -p renderer --example constellation_demo`
//! Output: `target/constellation_demo/*.png`

use std::sync::Arc;

use renderer::color::Color;
use renderer::config::{ConstellationOptions, DrawStyle};
use renderer::data::Column;
use renderer::data_config::{
    DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterStyleConfig,
    ErrorRef, ScatterShape, SeriesConfig,
};
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

    // ── Step 2: ringed planets (scatter). Ring angle = ScatterShape. ──
    let planet_series = |id: &str, x: &str, y: &str, shape: ScatterShape, size: f32, color: Color| SeriesConfig {
        series_id: id.into(),
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: color,
                point_shape: shape,
                point_size: size,
            },
        },
    };

    // Three scatter series, distinct ring angles, chart-scale markers.
    let m = 11;
    let pxs: Vec<f64> = (0..m).map(|i| 0.06 + 0.88 * i as f64 / (m - 1) as f64).collect();
    let pa: Vec<f64> = pxs.iter().enumerate().map(|(i, _)| 74.0 + 12.0 * ((i as f64) * 1.7).sin()).collect();
    let pb: Vec<f64> = pxs.iter().enumerate().map(|(i, _)| 48.0 + 11.0 * ((i as f64) * 2.3 + 1.0).cos()).collect();
    let pc: Vec<f64> = pxs.iter().enumerate().map(|(i, _)| 21.0 + 9.0 * ((i as f64) * 1.3 + 2.0).sin()).collect();
    r.add_column("px", &col(pxs)).unwrap();
    r.add_column("pa", &col(pa)).unwrap();
    r.add_column("pb", &col(pb)).unwrap();
    r.add_column("pc", &col(pc)).unwrap();
    let chart_planets = [
        planet_series("p_a", "px", "pa", ScatterShape::Circle, 13.0, Color::from_rgb8(255, 142, 92)),
        planet_series("p_b", "px", "pb", ScatterShape::Triangle, 13.0, Color::from_rgb8(96, 168, 255)),
        planet_series("p_c", "px", "pc", ScatterShape::DiamondFilled, 13.0, Color::from_rgb8(120, 220, 150)),
    ];
    export(
        &mut r,
        &build_chart(DrawStyle::Constellation(ConstellationOptions::default())),
        &chart_planets,
        "target/constellation_demo/planets_chart.png",
    );

    // Close-up showcase: few big planets to inspect the baked surfaces.
    let bx: Vec<f64> = vec![0.14, 0.38, 0.62, 0.86, 0.26, 0.74];
    let by: Vec<f64> = vec![70.0, 76.0, 68.0, 74.0, 30.0, 26.0];
    r.add_column("bx", &col(bx)).unwrap();
    r.add_column("by", &col(by)).unwrap();
    let big = [planet_series(
        "big", "bx", "by", ScatterShape::Square, 44.0, Color::from_rgb8(255, 170, 110),
    )];
    export(
        &mut r,
        &build_chart(DrawStyle::Constellation(ConstellationOptions::default())),
        &big,
        "target/constellation_demo/planets_big.png",
    );

    // Line + scatter combined: planets riding their own star chain.
    let combo = [SeriesConfig {
        series_id: "combo".into(),
        label: None,
        x_column: "px".into(),
        y_column: "pb".into(),
        render_type: DataRenderType::ScatterLine {
            scatter: DataScatterStyleConfig {
                point_color: Color::from_rgb8(255, 142, 92),
                point_shape: ScatterShape::Circle,
                point_size: 15.0,
            },
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::from_rgb8(255, 142, 92),
                line_width: 2.0,
            },
        },
    }];
    export(
        &mut r,
        &build_chart(DrawStyle::Constellation(ConstellationOptions::default())),
        &combo,
        "target/constellation_demo/planets_combo.png",
    );

    // Errorbars as bipolar jets: planets with ±σ rendered as glowing jet
    // beams terminating in shock knots at the exact interval bounds.
    let m2 = 11;
    let jerr: Vec<f64> = (0..m2).map(|i| 4.5 + 2.5 * ((i as f64) * 1.1).cos().abs()).collect();
    r.add_column("perr", &col(jerr)).unwrap();
    r.add_column("__zero", &col(vec![0.0; m2])).unwrap();
    let jets = [SeriesConfig {
        series_id: "jets".into(),
        label: None,
        x_column: "px".into(),
        y_column: "pb".into(),
        render_type: DataRenderType::ScatterErrorbarY {
            scatter: DataScatterStyleConfig {
                point_color: Color::from_rgb8(255, 142, 92),
                point_shape: ScatterShape::Circle,
                point_size: 13.0,
            },
            err_y: ErrorRef::Symmetric { column: "perr".into() },
            err_style: DataErrorBarStyleConfig {
                error_bar_color: Color::from_rgb8(255, 160, 110),
                error_bar_width: 2.0,
                error_bar_cap_size: 7.0,
                cap_width: 2.0,
            },
        },
    }];
    export(
        &mut r,
        &build_chart(DrawStyle::Constellation(ConstellationOptions::default())),
        &jets,
        "target/constellation_demo/jets.png",
    );
}
