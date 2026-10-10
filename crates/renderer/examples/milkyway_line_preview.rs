//! README Milkyway preview: line-generated stars and nebula only.
//! cargo run --locked -p figgy-renderer --example milkyway_line_preview -- target/milkyway-preview.png
//! Do not replace Line with ScatterLine/LineScatterErrorbar: those add planets/jets.
use renderer::config::{DrawStyle, LegendCorner, LegendEntryKind, MilkywayOptions};
use renderer::data::Column;
use renderer::data_config::{DataLineStyleConfig, DataRenderType};
use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{Chart, Color, Renderer, RendererDevice, SeriesConfig, encode_png};
use std::{error::Error, path::PathBuf, sync::Arc};

fn column(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or("target/milkyway-preview.png".into()),
    );
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        8 * 1024 * 1024,
    )?;
    let mut cfg = renderer::default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 1280,
        height: 1024,
    });
    cfg.chart_title.top_margin = 60.0;
    cfg.left_y.out_margin = 200.0;
    cfg.bottom_x.out_margin = 145.0;
    cfg.top_x.out_margin = 32.0;
    cfg.right_y.out_margin = 64.0;
    cfg.draw_style = DrawStyle::Milkyway(MilkywayOptions {
        star_density: 70.0,
        ribbon_width_px: 24.0,
        ribbon_intensity: 0.08,
        star_scale: 1.0,
        star_brightness: 0.32,
        spread_px: 8.0,
        structure_scale: 1.0,
        faint_bias: 4.5,
        glow: 0.35,
        nebula: 0.6,
        dust: 0.7,
        planet_rim: 0.0,
        seed: 0,
    });
    let chrome = Color::from_rgb8(198, 207, 223);
    for axis in [
        &mut cfg.bottom_x,
        &mut cfg.top_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.line_color = chrome;
        axis.label_style.color = chrome;
        axis.label_style.font_size = 25.0;
        axis.title_option.text.color = chrome;
        axis.title_option.text.font_size = 32.0;
    }
    cfg.grid.show_major_x = false;
    cfg.grid.show_major_y = false;
    cfg.grid.show_minor_x = false;
    cfg.grid.show_minor_y = false;
    let bg = Color::from_rgb8(11, 15, 23);
    cfg.legend.corner = LegendCorner::TopLeft;
    cfg.legend.bg_color = bg;
    cfg.legend.border_color = chrome;
    cfg.legend.content.color = Color::from_rgb8(238, 241, 248);
    cfg.legend.content.font_size = 23.0;
    cfg.legend.padding = 9.0;
    let mut chart = Chart::new(cfg)
        .with_x_title("Time [min]")
        .with_y_title("Response mean");
    chart.set_x_range(-6.0, 126.0);
    chart.set_y_range(-0.05, 1.08);
    let ts: Vec<f64> = (0..=480).map(|i| i as f64 * 0.25).collect();
    r.add_column("time", &column(ts.clone()))?;
    let mut series = Vec::new();
    for (id, label, color, ceiling, tau) in [
        (
            "control",
            "Control mean",
            Color::from_rgb8(225, 161, 206),
            0.87,
            40.0,
        ),
        (
            "treated",
            "Treated mean",
            Color::from_rgb8(165, 215, 229),
            1.04,
            19.0,
        ),
    ] {
        let values = ts
            .iter()
            .map(|t| 0.06 + (ceiling - 0.06) * (1.0 - (-t / tau).exp()))
            .collect();
        r.add_column(id, &column(values))?;
        // Line is intentional: stars are generated along arc length by the
        // Milkyway line pipeline; no scatter planets or errorbar jets exist.
        series.push(SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "time".into(),
            y_column: id.into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_color: color,
                    line_width: 2.0,
                    line_style: LineStylePreset::Solid,
                },
            },
        });
        chart = chart.with_legend_entry(label, color, 2.0, LegendEntryKind::Line);
    }
    let img = pollster::block_on(r.export_panel_rgba_with_clear_async(&chart, &series, 1.0, bg))?;
    let empty = pollster::block_on(r.export_panel_rgba_with_clear_async(&chart, &[], 1.0, bg))?;
    let changed = img
        .rgba
        .chunks_exact(4)
        .zip(empty.rgba.chunks_exact(4))
        .enumerate()
        .filter(|(i, (a, b))| {
            let x = *i % 1280;
            let y = *i / 1280;
            x > 210
                && x < 1200
                && y > 180
                && y < 869
                && a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 8)
        })
        .count();
    assert!(
        changed > 1000,
        "missing line-generated star field: {changed}"
    );
    assert!(img.rgba.chunks_exact(4).all(|p| p[3] == 255));
    std::fs::write(&output, encode_png(&img)?)?;
    println!(
        "{}: 1280x1024, {changed} data-dependent pixels; two Line series",
        output.display()
    );
    r.wait_idle();
    Ok(())
}
