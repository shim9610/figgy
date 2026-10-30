//! 3D surfaces (`Config.view_3d`, a heatmap drawn as a surface) through the
//! public renderer API: the headless export, the depth test, the colour
//! ramp, the wireframe, the lit material, the camera, the ID pick and the
//! highlight.
//!
//! House pattern (mirrors `polar_render.rs`): one process-wide device, a
//! renderer per test, and assertions that name the pixel a grid sample lands
//! on through `Config::project_3d`, so a wrong axis, camera or depth order
//! fails rather than passes.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::data::Column;
use renderer::data_config::{
    FieldFillConfig, FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
};
use renderer::layout::{ChartArea, Rect};
use renderer::view3d::{SurfaceStyle, SurfaceWireframe, View3d};
use renderer::{
    Chart, Color, ColorMap, Config, DataRenderType, RasterImage, Renderer, RendererDevice,
    SeriesConfig,
};

const WIDTH: u32 = 520;
const HEIGHT: u32 = 440;

fn col(data: Vec<f64>) -> Column<f64> {
    let finite = || data.iter().copied().filter(|v| v.is_finite());
    let min = finite().fold(f64::INFINITY, f64::min);
    let max = finite().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

fn shared_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    static DEVICE: OnceLock<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(|| {
        let (device, queue) = test_gpu::device();
        (Arc::new(device), Arc::new(queue))
    });
    (Arc::clone(device), Arc::clone(queue))
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Bgra8Unorm,
        64 * 1024 * 1024,
    )
    .expect("renderer init")
}

/// Upload an `nx × ny` grid of `f(x, y)` over `0..1 × 0..1` sample centres
/// as columns `z0..` (one per x), with `gx` / `gy` the coordinates.
fn upload_grid(r: &mut Renderer, nx: usize, ny: usize, f: impl Fn(f64, f64) -> f64) {
    let xs: Vec<f64> = (0..nx).map(|i| i as f64 / (nx - 1) as f64).collect();
    let ys: Vec<f64> = (0..ny).map(|j| j as f64 / (ny - 1) as f64).collect();
    for id in ["gx", "gy"] {
        let _ = r.remove_column(id);
    }
    r.add_column("gx", &col(xs.clone())).unwrap();
    r.add_column("gy", &col(ys.clone())).unwrap();
    for (i, x) in xs.iter().enumerate() {
        let id = format!("z{i}");
        let _ = r.remove_column(&id);
        r.add_column(&id, &col(ys.iter().map(|y| f(*x, *y)).collect()))
            .unwrap();
    }
}

fn matrix(nx: usize) -> MatrixRef {
    MatrixRef {
        columns: (0..nx).map(|i| format!("z{i}")).collect(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Centers,
    }
}

fn surface_series(nx: usize, shading: Shading, surface: Option<SurfaceStyle>) -> SeriesConfig {
    SeriesConfig {
        series_id: "ridge".into(),
        source_id: None,
        label: None,
        x_column: "gx".into(),
        y_column: "gy".into(),
        render_type: DataRenderType::Heatmap {
            matrix: matrix(nx),
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading,
                opacity: 1.0,
            },
            surface,
        },
    }
}

/// A 3D chart over the unit box with z in `0..1` and a Viridis colourbar
/// over `0..1`.
fn chart_3d() -> Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    for axis in [
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.min = 0.0;
        axis.max = 1.0;
        axis.major_spacing = 0.25;
    }
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Viridis;
    bar.axis.min = 0.0;
    bar.axis.max = 1.0;
    config.colorbar = Some(bar);
    config.view_3d = Some(View3d::default());
    config
}

fn ridge(x: f64, y: f64) -> f64 {
    (-((x - 0.5).powi(2) + (y - 0.4).powi(2)) / 0.05).exp()
}

fn export(r: &mut Renderer, config: &Config, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_panel_rgba(&Chart::new(config.clone()), series, scale)
        .unwrap()
}

/// Writes the image when `FIGGY_INTERACTION_ARTIFACTS` names a directory.
fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            std::path::Path::new(&path).join(format!("{name}.png")),
            renderer::encode_png(image).unwrap(),
        )
        .unwrap();
    }
}

fn pixel(img: &RasterImage, (x, y): (f64, f64)) -> [u8; 4] {
    let x = (x.floor() as i64).clamp(0, img.width as i64 - 1) as u32;
    let y = (y.floor() as i64).clamp(0, img.height as i64 - 1) as u32;
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

#[test]
fn a_heatmap_on_a_3d_chart_draws_as_a_surface() {
    let mut r = renderer();
    upload_grid(&mut r, 40, 32, ridge);
    let config = chart_3d();
    let style = SurfaceStyle {
        wireframe: Some(SurfaceWireframe {
            color: Color::new(0.1, 0.1, 0.1, 0.6),
            width: 1.0,
        }),
        ..SurfaceStyle::default()
    };
    let series = [surface_series(40, Shading::Interpolated, Some(style))];
    let img = export(&mut r, &config, &series, 1.0);
    save("surface-ridge", &img);
    // The peak sample (0.5, 0.4, 1) is drawn in the ramp's top colour.
    let peak = config.project_3d([0.5, 0.4, 1.0]).unwrap();
    let p = pixel(&img, (peak[0], peak[1] + 3.0));
    assert!(
        p[3] > 200 && p[0] > 100 && p[1] > 100 && p[2] < 80,
        "peak at {peak:?}: {p:?}"
    );
}

/// The ID pick answers the grid sample whose patch the cursor's pixel
/// shows, at 1x and 2x, and nothing off the surface.
#[test]
fn the_id_pick_answers_the_sample_under_the_cursor() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    let config = chart_3d();
    let series = vec![surface_series(21, Shading::Interpolated, None)];
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    for scale in [1.0f32, 2.0] {
        let displayed = config.scaled(scale);
        let pick = |r: &mut Renderer, at: [f64; 3]| {
            let p = displayed.project_3d(at).unwrap();
            let request = renderer::GpuPickRequest {
                canvas_position_px: [p[0] as f32, p[1] as f32],
                display_panel_px: Rect {
                    x: 0,
                    y: 0,
                    width: (WIDTH as f32 * scale) as u32,
                    height: (HEIGHT as f32 * scale) as u32,
                },
                display_scale: scale,
                max_distance_px: 0.0,
            };
            pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap()
        };
        let cell = |picked| match picked {
            Some(renderer::PickedData::MatrixCell {
                series_id,
                x_index,
                y_index,
                ..
            }) => {
                assert_eq!(series_id, "ridge");
                (x_index, y_index)
            }
            other => panic!("{scale}x: picked {other:?}"),
        };
        // Sample (10, 3) sits at x = 0.5, y = 0.1875, on the near side of
        // the peak (the default camera looks from −y).
        let (x, y) = (0.5, 3.0 / 16.0);
        assert_eq!(cell(pick(&mut r, [x, y, ridge(x, y)])), (10, 3), "{scale}x");
        // Sample (10, 8) at y = 0.5 is behind the peak at y = 0.4: the
        // nearer surface in front of it answers, by the depth test.
        let hidden = cell(pick(&mut r, [0.5, 0.5, ridge(0.5, 0.5)]));
        assert_ne!(hidden, (10, 8), "{scale}x: a hidden sample never picks");
        assert!(hidden.1 < 8, "{scale}x: {hidden:?} is nearer the eye");
        // Above the box: nothing.
        let above = displayed.project_3d([0.5, 0.5, 1.0]).unwrap();
        let p = [above[0], above[1] - 60.0 * f64::from(scale)];
        let request = renderer::GpuPickRequest {
            canvas_position_px: [p[0] as f32, p[1] as f32],
            display_panel_px: Rect {
                x: 0,
                y: 0,
                width: (WIDTH as f32 * scale) as u32,
                height: (HEIGHT as f32 * scale) as u32,
            },
            display_scale: scale,
            max_distance_px: 0.0,
        };
        let miss = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
        assert_eq!(miss, None, "{scale}x");
    }
}

/// The picked sample's patch is drawn over the surface around the sample's
/// projected place, at 1x and 2x, and nowhere else.
#[test]
fn the_picked_sample_patch_is_highlighted() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    let mut config = chart_3d();
    config.picked_data = Some(renderer::DataSelectionsConfig {
        refs: vec![renderer::PickedDataRef::MatrixCell {
            source_id: None,
            series_id: "ridge".into(),
            x_index: 10,
            y_index: 3,
        }],
        highlight_color: Color::new(1.0, 0.0, 1.0, 1.0),
        ..renderer::DataSelectionsConfig::default()
    });
    let series = [surface_series(21, Shading::Interpolated, None)];
    let (x, y) = (0.5, 3.0 / 16.0);
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        save(&format!("surface-picked-{scale}x"), &img);
        let at = config
            .scaled(scale)
            .project_3d([x, y, ridge(x, y)])
            .unwrap();
        let (mut near, mut far) = (0, 0);
        for (i, p) in img.rgba.chunks_exact(4).enumerate() {
            if !(p[3] > 180 && p[0] > 180 && p[1] < 80 && p[2] > 180) {
                continue;
            }
            let (px, py) = ((i as u32 % img.width) as f64, (i as u32 / img.width) as f64);
            if (px - at[0]).hypot(py - at[1]) < 30.0 * f64::from(scale) {
                near += 1;
            } else {
                far += 1;
            }
        }
        assert!(near > 10, "{scale}x: {near} outline pixels at the sample");
        assert_eq!(far, 0, "{scale}x: outline pixels away from the sample");
    }
}

fn contour_config(levels: Vec<f64>) -> renderer::data_config::ContourConfig {
    renderer::data_config::ContourConfig {
        levels,
        line: renderer::DataLineStyleConfig {
            line_style: renderer::line::LineStylePreset::Solid,
            line_color: Color::new(1.0, 0.0, 0.0, 1.0),
            line_width: 2.5,
            fill: None,
            fill_color: None,
            interpolation: renderer::data_config::LineInterpolation::Linear,
        },
        per_level_color: None,
        labels: None,
    }
}

/// A contour on a 3D chart draws its isolines on the box floor, where its
/// level crosses the grid, and the ID pick names the level and its cell.
#[test]
fn contours_draw_and_pick_on_the_floor() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    let config = chart_3d();
    let series = vec![SeriesConfig {
        series_id: "floor".into(),
        render_type: DataRenderType::Contour {
            matrix: matrix(21),
            contour: contour_config(vec![0.5]),
        },
        ..surface_series(21, Shading::Interpolated, None)
    }];
    let img = export(&mut r, &config, &series, 1.0);
    save("surface-floor-contour", &img);
    // The 0.5 isoline of the ridge is the circle of radius sqrt(0.05 ln 2)
    // around (0.5, 0.4) on the floor.
    let radius = (0.05f64 * std::f64::consts::LN_2).sqrt();
    let mut hits = 0;
    for k in 0..16 {
        let a = f64::from(k) / 16.0 * std::f64::consts::TAU;
        let (x, y) = (0.5 + radius * a.cos(), 0.4 + radius * a.sin());
        let p = config.project_3d([x, y, 0.0]).unwrap();
        let px = pixel(&img, (p[0], p[1]));
        if px[3] > 120 && px[0] > 150 && px[1] < 100 {
            hits += 1;
        }
    }
    assert!(
        hits >= 12,
        "{hits} of 16 points of the floor isoline are red"
    );
    // Off the isoline, at the circle's centre: no stroke.
    let centre = config.project_3d([0.5, 0.4, 0.0]).unwrap();
    let c = pixel(&img, (centre[0], centre[1]));
    assert!(
        !(c[0] > 150 && c[1] < 100),
        "no stroke at the centre: {c:?}"
    );

    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    let on = config.project_3d([0.5 + radius, 0.4, 0.0]).unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [on[0] as f32, on[1] as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT,
        },
        display_scale: 1.0,
        max_distance_px: 0.0,
    };
    match pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap() {
        Some(renderer::PickedData::ContourLevel {
            series_id,
            level_index,
            ..
        }) => {
            assert_eq!(series_id, "floor");
            assert_eq!(level_index, 0);
        }
        other => panic!("picked {other:?}"),
    }
}

/// An opaque surface covers the floor contours under it; a translucent one
/// shows them through.
#[test]
fn a_translucent_surface_shows_its_floor_contours() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    let config = chart_3d();
    let series = |opacity: f32| {
        vec![SeriesConfig {
            series_id: "both".into(),
            render_type: DataRenderType::HeatmapContour {
                matrix: matrix(21),
                fill: FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: Shading::Interpolated,
                    opacity,
                },
                contour: contour_config(vec![0.5]),
                surface: None,
            },
            ..surface_series(21, Shading::Interpolated, None)
        }]
    };
    let reds = |img: &RasterImage| {
        img.rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 120 && p[0] > 150 && p[1] < 90 && p[2] < 90)
            .count()
    };
    let opaque = export(&mut r, &config, &series(1.0), 1.0);
    let clear = export(&mut r, &config, &series(0.3), 1.0);
    save("surface-floor-opaque", &opaque);
    save("surface-floor-translucent", &clear);
    assert!(
        reds(&opaque) < 20,
        "the opaque surface covers the floor: {}",
        reds(&opaque)
    );
    assert!(
        reds(&clear) > 100,
        "the translucent surface shows it: {}",
        reds(&clear)
    );
}

/// Paint `config` with `series` on the screen path (prepare, then paint
/// into a target, through the resident panel composite) and read it back.
fn paint_screen(r: &mut Renderer, config: &Config, series: &[SeriesConfig]) -> RasterImage {
    let chart = Chart::new(config.clone());
    let rect = config.chart_area.0;
    let view = r.create_chart_view(&chart, rect).unwrap();
    let styles: Vec<_> = series
        .iter()
        .map(|cfg| r.create_style_for_series(cfg).unwrap())
        .collect();
    let drawn: Vec<_> = series
        .iter()
        .zip(&styles)
        .map(|(config, style)| renderer::Series { config, style })
        .collect();
    let items = [renderer::ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &drawn,
    }];
    let prepared = r.prepare(&items).unwrap();
    let (w, h) = (rect.width, rect.height);
    let device = r.device().clone();
    let queue = r.queue().clone();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface screen target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target = texture.create_view(&Default::default());
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("surface screen readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("surface screen paint"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        r.paint_prepared(&mut pass, (w, h), &prepared).unwrap();
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    drop(prepared);
    r.end_gpu_frame();
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        // BGRA to RGBA.
        for px in data[start..start + (w * 4) as usize].chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    drop(data);
    buffer.unmap();
    RasterImage {
        width: w,
        height: h,
        rgba,
    }
}

/// The screen draws the surface through the resident panel composite with
/// its depth-stencil attachment as the export does, and a camera edit
/// draws it again (the composite's key holds the camera).
#[test]
fn the_screen_draws_the_surface_and_redraws_it_when_the_camera_turns() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    let mut config = chart_3d();
    let series = [surface_series(21, Shading::Interpolated, None)];
    let screen = paint_screen(&mut r, &config, &series);
    save("surface-screen", &screen);
    let peak = config.project_3d([0.5, 0.4, 1.0]).unwrap();
    let p = pixel(&screen, (peak[0], peak[1] + 4.0));
    assert!(
        p[0] > 100 && p[1] > 100 && p[2] < 90,
        "the peak on screen: {p:?}"
    );
    // Turn the camera: the peak moves and the composite follows.
    config.orbit_view(60.0, 0.0);
    let turned = paint_screen(&mut r, &config, &series);
    save("surface-screen-turned", &turned);
    let moved = config.project_3d([0.5, 0.4, 1.0]).unwrap();
    assert!(
        (moved[0] - peak[0]).abs() > 3.0,
        "the camera turned the box"
    );
    let p = pixel(&turned, (moved[0], moved[1] + 4.0));
    assert!(
        p[0] > 100 && p[1] > 100 && p[2] < 90,
        "the peak after the turn: {p:?}"
    );
}
