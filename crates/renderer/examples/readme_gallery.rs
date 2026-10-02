//! Render the README gallery through the native, headless figgy export API.
//!
//! cargo run --locked -p renderer --example readme_gallery -- target/readme-gallery
//! Optional second argument: animation frame count (default 48, at 12 fps).
//! All datasets are synthetic. A missing GPU adapter is an error, not a skip.

use std::{error::Error, f64::consts::TAU, path::Path, sync::Arc};

use renderer::{
    Chart, Color, ColorMap, Renderer, RendererDevice, SeriesConfig,
    config::LegendEntryKind,
    data::Column,
    data_config::{
        BarOrientation, ContourConfig, ContourLabelConfig, DataBarStyleConfig,
        DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterStyleConfig,
        ErrorRef, FieldFillConfig, FillMode, GridLayout, MatrixOrientation, MatrixRef,
        ScatterShape, Shading,
    },
    data_render::{create_instance, request_adapter, request_device},
    default, encode_png,
    format::LabelFormat,
    layout::{ChartArea, Rect},
    line::LineStylePreset,
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn column(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

fn line(color: Color, width: f32) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: LineStylePreset::Solid,
        line_color: color,
        line_width: width,
    }
}

fn scatter(color: Color, size: f32) -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: color,
        point_shape: ScatterShape::CircleFilled,
        point_size: size,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
    }
}

fn series(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type,
    }
}

fn chart(title: &str, x_title: &str, y_title: &str, size: (u32, u32)) -> Chart {
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

fn save(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], path: &Path) -> Result {
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

fn errorbars(r: &mut Renderer, out: &Path) -> Result {
    let blue = Color::from_rgb8(34, 104, 198);
    let orange = Color::from_rgb8(224, 123, 38);
    let x: Vec<f64> = (0..18).map(|i| i as f64 * 0.5).collect();
    r.add_column("response-x", &column(x.clone()))?;
    let mut specs = Vec::new();
    for (name, color, speed) in [("a", blue, 0.65), ("b", orange, 0.32)] {
        let y = x
            .iter()
            .enumerate()
            .map(|(i, t)| 100.0 * (1.0 - (-speed * t).exp()) + 1.6 * (i as f64 * 1.7).sin())
            .collect();
        r.add_column(format!("response-{name}"), &column(y))?;
        r.add_column(
            format!("error-{name}"),
            &column(x.iter().map(|t| 3.0 + 0.4 * t).collect()),
        )?;
        specs.push(series(
            name,
            "response-x",
            &format!("response-{name}"),
            DataRenderType::LineScatterErrorbarY {
                line: line(color, 2.5),
                scatter: scatter(color, 6.0),
                err_y: ErrorRef::Symmetric {
                    column: format!("error-{name}"),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: color,
                    error_bar_width: 1.5,
                    error_bar_cap_size: 7.0,
                    cap_width: 1.5,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                },
            },
        ));
    }
    let mut c = chart(
        "Response curves with uncertainty",
        "Time (s)",
        "Response (%)",
        (960, 600),
    )
    .with_legend_entry("Fast response", blue, 2.5, LegendEntryKind::Line)
    .with_legend_entry("Slow response", orange, 2.5, LegendEntryKind::Line);
    c.set_x_range(-0.3, 8.8);
    c.config_mut().legend.corner = renderer::config::LegendCorner::TopLeft;
    c.set_y_range(-5.0, 115.0);
    save(r, &c, &specs, &out.join("gallery-errorbars.png"))
}

fn histogram(r: &mut Renderer, out: &Path) -> Result {
    let teal = Color::from_rgb8(19, 154, 139);
    let edges: Vec<f64> = (0..41).map(|i| -4.0 + i as f64 * 0.2).collect();
    let counts = edges
        .windows(2)
        .map(|v| {
            let x = (v[0] + v[1]) * 0.5;
            (100.0 * (-0.5 * ((x + 1.2) / 0.65).powi(2)).exp()
                + 68.0 * (-0.5 * ((x - 1.1) / 0.9).powi(2)).exp())
            .round()
        })
        .collect();
    r.add_column("bin-edges", &column(edges))?;
    r.add_column("bin-counts", &column(counts))?;
    let specs = [series(
        "distribution",
        "bin-edges",
        "bin-counts",
        DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: teal,
                border_color: Color::from_rgb8(10, 102, 95),
                border_width: 0.8,
                baseline: 0.0,
                gap_px: 2.0,
                width_ratio: 1.0,
                orientation: BarOrientation::Vertical,
                bar_style_overrides: None,
            },
        },
    )];
    let mut c = chart(
        "A two-population distribution",
        "Measured value",
        "Count",
        (960, 600),
    );
    c.set_x_range(-4.2, 4.2);
    c.set_y_range(0.0, 120.0);
    save(r, &c, &specs, &out.join("gallery-histogram.png"))
}

fn contours(r: &mut Renderer, out: &Path) -> Result {
    let n = 81;
    let axis: Vec<f64> = (0..n)
        .map(|i| -3.0 + 6.0 * i as f64 / (n - 1) as f64)
        .collect();
    r.add_column("field-x", &column(axis.clone()))?;
    r.add_column("field-y", &column(axis.clone()))?;
    let mut columns = Vec::new();
    for (i, x) in axis.iter().enumerate() {
        let id = format!("field-z-{i}");
        let values = axis
            .iter()
            .map(|y| {
                1.1 * (-((x + 1.0).powi(2) / 1.3 + (y - 0.7).powi(2) / 0.8)).exp()
                    + 0.85 * (-((x - 1.1).powi(2) / 0.7 + (y + 0.7).powi(2) / 1.8)).exp()
            })
            .collect();
        r.add_column(id.clone(), &column(values))?;
        columns.push(id);
    }
    let contour = ContourConfig {
        levels: (1..=10).map(|i| i as f64 * 0.1).collect(),
        line: line(Color::from_rgba(1.0, 1.0, 1.0, 0.9), 1.3),
        per_level_color: None,
        labels: Some(ContourLabelConfig {
            visible: true,
            font_size: 13.0,
            color: Color::WHITE,
            format: LabelFormat::Decimal,
            significant_digits: 2,
            spacing_px: 140.0,
            anchors: Vec::new(),
            bg_color: None,
            bg_padding_px: 3.0,
        }),
    };
    let specs = [series(
        "potential",
        "field-x",
        "field-y",
        DataRenderType::HeatmapContour {
            matrix: MatrixRef {
                columns,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            },
            contour,
        },
    )];
    let mut c = chart("A smooth field, exact contour levels", "x", "y", (960, 640));
    c.set_x_range(-3.0, 3.0);
    c.set_y_range(-3.0, 3.0);
    let mut bar = default::default_colorbar_options();
    bar.colormap = ColorMap::Viridis;
    bar.axis.min = 0.0;
    bar.axis.max = 1.2;
    bar.axis.major_spacing = 0.2;
    c.config_mut().colorbar = Some(bar);
    c.config_mut().grid.show_major_x = false;
    c.config_mut().grid.show_major_y = false;
    save(r, &c, &specs, &out.join("gallery-contours.png"))
}

fn animations(r: &mut Renderer, out: &Path, frames: usize) -> Result {
    let blue = Color::from_rgb8(34, 104, 198);
    let coral = Color::from_rgb8(225, 81, 79);
    let x: Vec<f64> = (0..401).map(|i| i as f64 / 400.0 * TAU * 2.0).collect();
    r.add_column("wave-x", &column(x.clone()))?;
    r.add_column("wave-a", &column(vec![0.0; x.len()]))?;
    r.add_column("wave-b", &column(vec![0.0; x.len()]))?;
    let specs = [
        series(
            "primary",
            "wave-x",
            "wave-a",
            DataRenderType::Line {
                line: line(blue, 3.0),
            },
        ),
        series(
            "harmonic",
            "wave-x",
            "wave-b",
            DataRenderType::Line {
                line: line(coral, 2.5),
            },
        ),
    ];
    let mut wave = chart("Waves in motion", "Position", "Amplitude", (720, 460));
    wave.set_x_range(0.0, TAU * 2.0);
    wave.set_y_range(-1.6, 1.6);

    let curve: Vec<f64> = (0..501).map(|i| TAU * i as f64 / 500.0).collect();
    r.add_column(
        "orbit-x",
        &column(curve.iter().map(|t| (2.0 * t).sin()).collect()),
    )?;
    r.add_column(
        "orbit-y",
        &column(curve.iter().map(|t| (3.0 * t + 0.4).sin()).collect()),
    )?;
    for id in ["trail-x", "trail-y"] {
        r.add_column(id, &column(vec![0.0; 101]))?;
    }
    for id in ["head-x", "head-y"] {
        r.add_column(id, &column(vec![0.0]))?;
    }
    let orbit_specs = [
        series(
            "path",
            "orbit-x",
            "orbit-y",
            DataRenderType::Line {
                line: line(Color::from_rgb8(186, 204, 226), 1.5),
            },
        ),
        series(
            "trail",
            "trail-x",
            "trail-y",
            DataRenderType::Line {
                line: line(blue, 3.5),
            },
        ),
        series(
            "head",
            "head-x",
            "head-y",
            DataRenderType::Scatter {
                scatter: scatter(coral, 11.0),
            },
        ),
    ];
    let mut orbit = chart(
        "A 2:3 phase portrait",
        "sin(2t)",
        "sin(3t + 0.4)",
        (720, 460),
    );
    orbit.set_x_range(-1.2, 1.2);
    orbit.set_y_range(-1.2, 1.2);
    for name in ["wave", "orbit"] {
        std::fs::create_dir_all(out.join(name))?;
    }
    for i in 0..frames {
        let phase = TAU * i as f64 / frames as f64;
        r.upsert_column(
            "wave-a",
            &column(x.iter().map(|x| (x - phase).sin()).collect()),
        )?;
        r.upsert_column(
            "wave-b",
            &column(x.iter().map(|x| 0.55 * (2.0 * x + phase).sin()).collect()),
        )?;
        save(r, &wave, &specs, &out.join(format!("wave/{i:03}.png")))?;
        let trail: Vec<f64> = (0..101).map(|j| phase - 0.8 + j as f64 * 0.008).collect();
        r.upsert_column(
            "trail-x",
            &column(trail.iter().map(|t| (2.0 * t).sin()).collect()),
        )?;
        r.upsert_column(
            "trail-y",
            &column(trail.iter().map(|t| (3.0 * t + 0.4).sin()).collect()),
        )?;
        r.upsert_column("head-x", &column(vec![(2.0 * phase).sin()]))?;
        r.upsert_column("head-y", &column(vec![(3.0 * phase + 0.4).sin()]))?;
        save(
            r,
            &orbit,
            &orbit_specs,
            &out.join(format!("orbit/{i:03}.png")),
        )?;
    }
    Ok(())
}

fn main() -> Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() > 2 {
        return Err("usage: readme_gallery [output-directory] [frames]".into());
    }
    let out = std::path::PathBuf::from(
        args.first()
            .map(String::as_str)
            .unwrap_or("target/readme-gallery"),
    );
    let frames = args
        .get(1)
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(48);
    if !(2..=240).contains(&frames) {
        return Err("frames must be between 2 and 240".into());
    }
    std::fs::create_dir_all(&out)?;
    // Keep the instance and adapter alive through all exports; use the normal
    // native adapter selection. Software Vulkan is an environment choice.
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("Gallery adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let mut renderer = Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        16 * 1024 * 1024,
    )?;
    errorbars(&mut renderer, &out)?;
    histogram(&mut renderer, &out)?;
    contours(&mut renderer, &out)?;
    animations(&mut renderer, &out, frames)?;
    renderer.wait_idle();
    Ok(())
}
