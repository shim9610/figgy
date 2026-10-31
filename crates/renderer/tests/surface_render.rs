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
    DataLineStyleConfig, DataScatterStyleConfig, FieldFillConfig, FillMode, GridLayout,
    LineInterpolation, MatrixOrientation, MatrixRef, ScatterShape, Shading,
};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::view3d::{SurfaceStyle, SurfaceWireframe, View3d};
use renderer::{
    Chart, Color, ColorMap, Config, DataRenderType, DataSelectionsConfig, PickedDataRef,
    RasterImage, Renderer, RendererDevice, SeriesConfig,
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
        z_column: None,
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

/// Whether `p` is drawn in Viridis' top colour (yellow).
fn is_top_color(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 100 && p[1] > 100 && p[2] < 90
}

/// The 2x export draws what the 1x export draws, at twice the pixels: the
/// peak lands where the scaled camera puts it.
#[test]
fn the_2x_export_draws_the_1x_surface() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    let config = chart_3d();
    let series = [surface_series(21, Shading::Interpolated, None)];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        assert_eq!(img.width, (WIDTH as f32 * scale) as u32);
        let peak = config.scaled(scale).project_3d([0.5, 0.4, 1.0]).unwrap();
        let p = pixel(&img, (peak[0], peak[1] + 3.0 * f64::from(scale)));
        assert!(is_top_color(p), "{scale}x peak at {peak:?}: {p:?}");
    }
}

/// A perspective camera draws the surface where `project_3d` places it.
#[test]
fn a_perspective_camera_draws_the_surface() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    let mut config = chart_3d();
    config.view_3d.as_mut().unwrap().projection = renderer::view3d::Projection3d::Perspective;
    let series = [surface_series(21, Shading::Interpolated, None)];
    let img = export(&mut r, &config, &series, 1.0);
    save("surface-perspective", &img);
    let peak = config.project_3d([0.5, 0.4, 1.0]).unwrap();
    let ortho = chart_3d().project_3d([0.5, 0.4, 1.0]).unwrap();
    assert!(
        (peak[0] - ortho[0]).abs() + (peak[1] - ortho[1]).abs() > 1.0,
        "the perspective camera moves the peak"
    );
    let p = pixel(&img, (peak[0], peak[1] + 3.0));
    assert!(is_top_color(p), "peak at {peak:?}: {p:?}");
}

/// A logarithmic z axis places the heights by their logarithm: the peak
/// of `1 + 99·ridge` reaches the box's top at 100.
#[test]
fn a_logarithmic_z_axis_places_the_heights_by_their_logarithm() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, |x, y| 1.0 + 99.0 * ridge(x, y));
    let mut config = chart_3d();
    config.right_y.scale = renderer::config::AxisScale::Logarithmic;
    config.right_y.min = 1.0;
    config.right_y.max = 100.0;
    let bar = config.colorbar.as_mut().unwrap();
    bar.axis.min = 1.0;
    bar.axis.max = 100.0;
    let series = [surface_series(21, Shading::Interpolated, None)];
    let img = export(&mut r, &config, &series, 1.0);
    save("surface-log-z", &img);
    let peak = config.project_3d([0.5, 0.4, 1.0]).unwrap();
    let p = pixel(&img, (peak[0], peak[1] + 3.0));
    assert!(is_top_color(p), "peak at {peak:?}: {p:?}");
}

/// A NaN sample drops the quads around it, as a NaN breaks a line: the
/// surface has a hole there and stays whole elsewhere.
#[test]
fn a_nan_sample_drops_the_quads_around_it() {
    let mut r = renderer();
    upload_grid(&mut r, 11, 11, |x, y| {
        if (x - 0.5).abs() < 1e-9 && (y - 0.5).abs() < 1e-9 {
            f64::NAN
        } else {
            0.5
        }
    });
    let config = chart_3d();
    let series = [surface_series(11, Shading::Interpolated, None)];
    let img = export(&mut r, &config, &series, 1.0);
    save("surface-nan-hole", &img);
    // Viridis at 0.5 is teal.
    let teal = |p: [u8; 4]| p[3] > 200 && p[1] > 110 && p[2] > 110 && p[0] < 80;
    let hole = config.project_3d([0.5, 0.5, 0.5]).unwrap();
    assert!(!teal(pixel(&img, (hole[0], hole[1]))), "the hole is open");
    let whole = config.project_3d([0.2, 0.2, 0.5]).unwrap();
    assert!(teal(pixel(&img, (whole[0], whole[1]))), "the rest is drawn");
}

/// A lit material shades the surface: the lit image differs from the
/// unlit one while the colours stay the ramp's.
#[test]
fn a_lit_material_shades_the_surface() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    let config = chart_3d();
    let flat = export(
        &mut r,
        &config,
        &[surface_series(21, Shading::Interpolated, None)],
        1.0,
    );
    let lit_style = SurfaceStyle {
        material: renderer::Material::Matte,
        ..SurfaceStyle::default()
    };
    let lit = export(
        &mut r,
        &config,
        &[surface_series(21, Shading::Interpolated, Some(lit_style))],
        1.0,
    );
    save("surface-lit", &lit);
    let changed = flat
        .rgba
        .chunks_exact(4)
        .zip(lit.rgba.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 12))
        .count();
    assert!(changed > 1000, "lighting changed {changed} pixels");
}

/// The z fit reads the height columns' statistics: `right_y` spans the
/// heights of the grid.
#[test]
fn the_z_fit_spans_the_heights() {
    let mut r = renderer();
    upload_grid(&mut r, 11, 9, |x, y| 3.0 + 4.0 * ridge(x, y));
    let mut chart = Chart::new(chart_3d());
    let series = [surface_series(11, Shading::Interpolated, None)];
    chart
        .auto_fit_view_3d_z(r.pool(), renderer::LayerRef::Base, &series, 0.0)
        .unwrap();
    let z = &chart.config().right_y;
    let lo = (0..11)
        .flat_map(|i| (0..9).map(move |j| (i as f64 / 10.0, j as f64 / 8.0)))
        .map(|(x, y)| 3.0 + 4.0 * ridge(x, y));
    let (min, max) = lo.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
        (a.min(v), b.max(v))
    });
    assert!(
        (z.min - min).abs() < 1e-9 && (z.max - max).abs() < 1e-9,
        "{} {}",
        z.min,
        z.max
    );
}

/// Rule 10: a grid past `1 << 22` quads is drawn whole; the renderer caps
/// no quad count.
#[test]
fn a_grid_past_four_million_quads_draws() {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Bgra8Unorm,
        512 * 1024 * 1024,
    )
    .expect("renderer init");
    let (nx, ny) = (2050, 2049);
    assert!((nx - 1) * (ny - 1) > 1 << 22);
    upload_grid(&mut r, nx, ny, ridge);
    let config = chart_3d();
    let series = [surface_series(nx, Shading::Interpolated, None)];
    let img = export(&mut r, &config, &series, 1.0);
    let peak = config.project_3d([0.5, 0.4, 1.0]).unwrap();
    let p = pixel(&img, (peak[0], peak[1] + 3.0));
    assert!(is_top_color(p), "peak at {peak:?}: {p:?}");
    // The far corner's quads are drawn too.
    let corner = config
        .project_3d([0.999, 0.999, ridge(0.999, 0.999)])
        .unwrap();
    assert!(
        pixel(&img, (corner[0], corner[1]))[3] > 200,
        "the far corner"
    );
}

/// The 2x camera is the 1x camera doubled: every box point lands at twice
/// its 1x pixel position.
#[test]
fn the_2x_camera_is_the_1x_camera_doubled() {
    let mut config = chart_3d();
    config.legend.visible = true;
    config.right_y.label_style.label_visible = true;
    let doubled = config.scaled(2.0);
    let (a, b) = (
        config.data_area().unwrap().0,
        doubled.data_area().unwrap().0,
    );
    eprintln!("data area 1x {a:?} 2x {b:?}");
    for t in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.5, 0.2, 0.7]] {
        let p = config.project_3d(t).unwrap();
        let q = doubled.project_3d(t).unwrap();
        assert!(
            (q[0] - 2.0 * p[0]).abs() < 0.6 && (q[1] - 2.0 * p[1]).abs() < 0.6,
            "{t:?}: 1x {p:?}, 2x {q:?}"
        );
    }
}

// ── Scatter and line series on a 3D chart (#132) ────────────────────────────

const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

fn is_red(p: [u8; 4]) -> bool {
    p[3] > 100 && p[0] > 170 && p[1] < 90 && p[2] < 90
}

/// Whether a red pixel lies within `radius` of `at`.
fn red_near(img: &RasterImage, at: (f64, f64), radius: f64) -> bool {
    let r = radius.ceil() as i64;
    (-r..=r).any(|dy| {
        (-r..=r).any(|dx| {
            f64::from((dx * dx + dy * dy) as i32) <= radius * radius
                && is_red(pixel(img, (at.0 + dx as f64, at.1 + dy as f64)))
        })
    })
}

/// A helix of `n` points through the box, as columns `tx`, `ty`, `tz`.
fn upload_helix(r: &mut Renderer, n: usize) {
    let xs: Vec<f64> = (0..n).map(|i| helix_point(n, i)[0]).collect();
    let ys: Vec<f64> = (0..n).map(|i| helix_point(n, i)[1]).collect();
    let zs: Vec<f64> = (0..n).map(|i| helix_point(n, i)[2]).collect();
    for (id, data) in [("tx", xs), ("ty", ys), ("tz", zs)] {
        let _ = r.remove_column(id);
        r.add_column(id, &col(data)).unwrap();
    }
}

fn helix_point(n: usize, i: usize) -> [f64; 3] {
    let t = i as f64 / (n - 1) as f64;
    [
        0.5 + 0.33 * (4.0 * std::f64::consts::PI * t).cos(),
        0.5 + 0.33 * (4.0 * std::f64::consts::PI * t).sin(),
        0.05 + 0.9 * t,
    ]
}

fn red_points(size: f32) -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: RED,
        point_shape: ScatterShape::CircleFilled,
        point_size: size,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    }
}

fn red_line(width: f32, line_style: LineStylePreset) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style,
        line_color: RED,
        line_width: width,
        fill: None,
        fill_color: None,
        interpolation: LineInterpolation::Linear,
    }
}

fn point_series(id: &str, x: &str, y: &str, z: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: Some(z.into()),
        render_type,
    }
}

/// The markers and the stroke of a scatter-line series land where the
/// camera puts its samples, at 1x and 2x, and nowhere else.
#[test]
fn scatter_and_line_series_draw_on_a_3d_chart() {
    let mut r = renderer();
    let n = 60;
    upload_helix(&mut r, n);
    let config = chart_3d();
    let series = [point_series(
        "helix",
        "tx",
        "ty",
        "tz",
        DataRenderType::ScatterLine {
            scatter: red_points(5.0),
            line: red_line(3.0, LineStylePreset::Solid),
        },
    )];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        save(&format!("scatter_line_3d_{scale}x"), &img);
        let displayed = config.scaled(scale);
        for i in [0, 17, 30, 44, n - 1] {
            let p = displayed.project_3d(helix_point(n, i)).unwrap();
            assert!(
                is_red(pixel(&img, (p[0], p[1]))),
                "{scale}x: no marker at sample {i}, {p:?}"
            );
        }
        // Between two samples: the stroke.
        let (a, b) = (helix_point(n, 30), helix_point(n, 31));
        let mid = displayed
            .project_3d([
                (a[0] + b[0]) / 2.0,
                (a[1] + b[1]) / 2.0,
                (a[2] + b[2]) / 2.0,
            ])
            .unwrap();
        assert!(
            red_near(&img, (mid[0], mid[1]), 2.0 * f64::from(scale)),
            "{scale}x: no stroke between samples 30 and 31 at {mid:?}"
        );
        // The box's centre at z = 0 is well off the helix: nothing red.
        let off = displayed.project_3d([0.5, 0.5, 0.0]).unwrap();
        assert!(
            !red_near(&img, (off[0], off[1]), 6.0 * f64::from(scale)),
            "{scale}x: red ink off the helix at {off:?}"
        );
    }
}

/// A marker behind the surface is hidden by the depth test and one on the
/// surface shows over it (the bias towards the camera).
#[test]
fn markers_are_depth_tested_against_the_surface() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    // On the near face of the ridge, exactly on the surface; and on the far
    // slope, on the surface the peak hides (see the ID pick test).
    let (shown, hidden) = ([0.5, 3.0 / 16.0], [0.5, 0.6]);
    let points = [shown, hidden];
    for (id, k) in [("px", 0), ("py", 1)] {
        let _ = r.remove_column(id);
        r.add_column(id, &col(points.iter().map(|p| p[k]).collect()))
            .unwrap();
    }
    let _ = r.remove_column("pz");
    r.add_column(
        "pz",
        &col(points.iter().map(|p| ridge(p[0], p[1])).collect()),
    )
    .unwrap();
    let config = chart_3d();
    let series = [
        surface_series(21, Shading::Interpolated, None),
        point_series(
            "dots",
            "px",
            "py",
            "pz",
            DataRenderType::Scatter {
                scatter: red_points(6.0),
            },
        ),
    ];
    let img = export(&mut r, &config, &series, 1.0);
    save("markers_depth_tested", &img);
    let at = |p: [f64; 2]| {
        let q = config.project_3d([p[0], p[1], ridge(p[0], p[1])]).unwrap();
        (q[0], q[1])
    };
    assert!(
        is_red(pixel(&img, at(shown))),
        "the marker on the near face is drawn over the surface"
    );
    assert!(
        !red_near(&img, at(hidden), 2.0),
        "the marker behind the peak is hidden by the surface"
    );
    // Without the surface the hidden one shows: it is the depth test, not
    // the placement, that hides it.
    let alone = export(&mut r, &config, &series[1..], 1.0);
    assert!(is_red(pixel(&alone, at(hidden))));
}

/// The ID pick answers the marker under the cursor and, on the stroke, the
/// nearer end of the segment; beside the stroke nothing is a point.
#[test]
fn the_id_pick_answers_markers_and_segment_ends() {
    let mut r = renderer();
    let n = 40;
    upload_helix(&mut r, n);
    upload_grid(&mut r, 21, 17, |x, y| 0.5 * ridge(x, y));
    let config = chart_3d();
    let series = vec![
        surface_series(21, Shading::Interpolated, None),
        point_series(
            "trail",
            "tx",
            "ty",
            "tz",
            DataRenderType::Line {
                line: red_line(3.0, LineStylePreset::Solid),
            },
        ),
        point_series(
            "dots",
            "tx",
            "ty",
            "tz",
            DataRenderType::Scatter {
                scatter: red_points(5.0),
            },
        ),
    ];
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    for scale in [1.0f32, 2.0] {
        let displayed = config.scaled(scale);
        let pick = |r: &mut Renderer, at: [f64; 3], offset: [f64; 2]| {
            let p = displayed.project_3d(at).unwrap();
            let request = renderer::GpuPickRequest {
                canvas_position_px: [(p[0] + offset[0]) as f32, (p[1] + offset[1]) as f32],
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
        let point = |picked: Option<renderer::PickedData>| match picked {
            Some(renderer::PickedData::Point {
                series_id,
                point_index,
                ..
            }) => (series_id, point_index),
            other => panic!("{scale}x: picked {other:?}"),
        };
        // A marker, drawn over the stroke: the dots answer.
        assert_eq!(
            point(pick(&mut r, helix_point(n, 25), [0.0, 0.0])),
            ("dots".to_string(), 25),
            "{scale}x"
        );
        // On the stroke between samples 25 and 26 (a 16 px chord), past
        // both markers (radius 5) and nearer 26: the trail's end 26 answers.
        let (a, b) = (helix_point(n, 25), helix_point(n, 26));
        let near_b = [
            a[0] + 0.6 * (b[0] - a[0]),
            a[1] + 0.6 * (b[1] - a[1]),
            a[2] + 0.6 * (b[2] - a[2]),
        ];
        assert_eq!(
            point(pick(&mut r, near_b, [0.0, 0.0])),
            ("trail".to_string(), 26),
            "{scale}x"
        );
        // The mirror point nearer 25: its start answers. The pick's 1x1
        // target clips the stroke around the cursor, so the nearer end is
        // decided on the whole chord, not the clipped piece.
        let near_a = [
            a[0] + 0.4 * (b[0] - a[0]),
            a[1] + 0.4 * (b[1] - a[1]),
            a[2] + 0.4 * (b[2] - a[2]),
        ];
        assert_eq!(
            point(pick(&mut r, near_a, [0.0, 0.0])),
            ("trail".to_string(), 25),
            "{scale}x"
        );
        // Beside the stroke: never a point.
        let beside = pick(&mut r, helix_point(n, 25), [0.0, -14.0 * f64::from(scale)]);
        assert!(
            !matches!(beside, Some(renderer::PickedData::Point { .. })),
            "{scale}x: {beside:?}"
        );
    }
    // `step_selection` walks the picked series' samples.
    let mut stepped = config.clone();
    stepped.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "dots".into(),
            point_index: 25,
        }],
        ..DataSelectionsConfig::default()
    });
    let len = |id: &str| (id == "tx" || id == "ty" || id == "tz").then_some(n);
    assert_eq!(stepped.step_selection(&series, 1, 0, len), Ok(true));
    assert!(matches!(
        stepped.picked_data.as_ref().unwrap().refs[0],
        PickedDataRef::Point {
            point_index: 26,
            ..
        }
    ));
}

/// The picked ring draws around the marker's projected place at 1x and 2x.
#[test]
fn the_picked_ring_surrounds_the_3d_marker() {
    let mut r = renderer();
    let n = 40;
    upload_helix(&mut r, n);
    let mut config = chart_3d();
    let series = [point_series(
        "dots",
        "tx",
        "ty",
        "tz",
        DataRenderType::Scatter {
            scatter: red_points(5.0),
        },
    )];
    let plain = export(&mut r, &config, &series, 1.0);
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "dots".into(),
            point_index: 12,
        }],
        ..DataSelectionsConfig::default()
    });
    let selection = config.picked_data.clone().unwrap();
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        save(&format!("picked_ring_3d_{scale}x"), &img);
        let p = config.scaled(scale).project_3d(helix_point(n, 12)).unwrap();
        let extra = f64::from(selection.point_radius_extra_px * scale);
        let ring_radius = 5.0 * f64::from(scale) + extra;
        // Inked, not the marker, and (at 1x) not there without the
        // selection: the ring, on the circle around the marker.
        let mut on_ring = 0;
        for k in 0..16 {
            let a = k as f64 * std::f64::consts::TAU / 16.0;
            let q = (p[0] + ring_radius * a.cos(), p[1] + ring_radius * a.sin());
            let got = pixel(&img, q);
            let changed = scale != 1.0 || got != pixel(&plain, q);
            if got[3] > 60 && !is_red(got) && changed {
                on_ring += 1;
            }
        }
        assert!(
            on_ring >= 10,
            "{scale}x: ring pixels on {on_ring} of 16 samples"
        );
    }
}

/// A dashed 3D line has gaps along its projected chord; a solid one has
/// none: the dash phase walks the chord the stroke draws.
#[test]
fn a_dashed_3d_line_dashes_along_the_projected_chord() {
    let mut r = renderer();
    for (id, data) in [
        ("lx", vec![0.1, 0.9]),
        ("ly", vec![0.1, 0.9]),
        ("lz", vec![0.1, 0.9]),
    ] {
        let _ = r.remove_column(id);
        r.add_column(id, &col(data)).unwrap();
    }
    let config = chart_3d();
    let runs = |style: LineStylePreset, scale: f32, r: &mut Renderer| {
        let series = [point_series(
            "chord",
            "lx",
            "ly",
            "lz",
            DataRenderType::Line {
                line: red_line(3.0, style),
            },
        )];
        let img = export(r, &config, &series, scale);
        let displayed = config.scaled(scale);
        let a = displayed.project_3d([0.1, 0.1, 0.1]).unwrap();
        let b = displayed.project_3d([0.9, 0.9, 0.9]).unwrap();
        let steps = 400;
        let mut runs = 0;
        let mut inside = false;
        for k in 40..=360 {
            let t = k as f64 / steps as f64;
            let red = red_near(
                &img,
                (a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])),
                1.0,
            );
            if red && !inside {
                runs += 1;
            }
            inside = red;
        }
        runs
    };
    for scale in [1.0f32, 2.0] {
        assert_eq!(
            runs(LineStylePreset::Solid, scale, &mut r),
            1,
            "{scale}x solid"
        );
        let dashed = runs(LineStylePreset::Dash, scale, &mut r);
        assert!(dashed >= 4, "{scale}x dashed: {dashed} runs");
    }
}

/// The screen path (the resident composite) draws the marks too.
#[test]
fn the_screen_draws_the_marks() {
    let mut r = renderer();
    let n = 30;
    upload_helix(&mut r, n);
    let config = chart_3d();
    let series = vec![point_series(
        "helix",
        "tx",
        "ty",
        "tz",
        DataRenderType::ScatterLine {
            scatter: red_points(5.0),
            line: red_line(3.0, LineStylePreset::Solid),
        },
    )];
    let img = paint_screen(&mut r, &config, &series);
    let p = config.project_3d(helix_point(n, 10)).unwrap();
    assert!(is_red(pixel(&img, (p[0], p[1]))), "{p:?}");
}

/// Rule 10: a trajectory past `1 << 20` rows draws whole and picks; the
/// renderer caps no row count.
#[test]
fn a_million_row_trajectory_draws_and_picks() {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Bgra8Unorm,
        256 * 1024 * 1024,
    )
    .expect("renderer init");
    let n = (1 << 20) + 1;
    upload_helix(&mut r, n);
    let config = chart_3d();
    let series = vec![point_series(
        "helix",
        "tx",
        "ty",
        "tz",
        DataRenderType::ScatterLine {
            scatter: red_points(3.0),
            line: red_line(2.0, LineStylePreset::Solid),
        },
    )];
    let img = export(&mut r, &config, &series, 1.0);
    save("million_rows", &img);
    let p = config.project_3d(helix_point(n, n - 1)).unwrap();
    assert!(
        red_near(&img, (p[0], p[1]), 2.0),
        "the last row draws: {:?} at {p:?}",
        pixel(&img, (p[0], p[1]))
    );
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [p[0] as f32, p[1] as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT,
        },
        display_scale: 1.0,
        max_distance_px: 0.0,
    };
    let picked = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    // The nearest mark under the cursor answers: a row whose marker covers
    // the last row's pixel and lies no deeper than it (the end of the helix
    // is where its strands cross on screen).
    match picked {
        Some(renderer::PickedData::Point { point_index, .. }) => {
            let q = config.project_3d(helix_point(n, point_index)).unwrap();
            assert!(
                (q[0] - p[0]).hypot(q[1] - p[1]) <= 3.5 && q[2] <= p[2] + 1e-6,
                "picked row {point_index} at {q:?}, the last row at {p:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// The renderer rejects a scatter series without a height column on a 3D
/// chart, and the z fit spans a trajectory's heights.
#[test]
fn a_3d_point_series_needs_its_height_and_fits_the_z_axis() {
    let mut r = renderer();
    upload_helix(&mut r, 20);
    let config = chart_3d();
    let mut flat = point_series(
        "dots",
        "tx",
        "ty",
        "tz",
        DataRenderType::Scatter {
            scatter: red_points(5.0),
        },
    );
    flat.z_column = None;
    let err = r
        .register_chart(config.clone(), vec![flat])
        .err()
        .expect("a 3D scatter names its z column");
    assert!(err.to_string().contains("z_column"), "{err}");

    let mut chart = Chart::new(config);
    let series = [point_series(
        "dots",
        "tx",
        "ty",
        "tz",
        DataRenderType::Scatter {
            scatter: red_points(5.0),
        },
    )];
    chart
        .auto_fit_view_3d_z(r.pool(), renderer::LayerRef::Base, &series, 0.0)
        .unwrap();
    let z = &chart.config().right_y;
    assert!(
        (z.min - 0.05).abs() < 1e-9 && (z.max - 0.95).abs() < 1e-9,
        "{} {}",
        z.min,
        z.max
    );
}
