//! 3D surfaces (`Config.view_3d`, a heatmap drawn as a surface, and a
//! triangle mesh drawn as one, #210) through the public renderer API: the
//! headless export, the depth test, the colour ramp, the wireframe, the lit
//! material, the camera, the ID pick and the highlight.
//!
//! House pattern (mirrors `polar_render.rs`): one process-wide device, a
//! renderer per test, and assertions that name the pixel a grid sample lands
//! on through `Config::project_3d`, so a wrong axis, camera or depth order
//! fails rather than passes.

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::data::Column;
use renderer::data_config::{
    DataLineStyleConfig, DataScatterStyleConfig, FieldFillConfig, FillMode, GridLayout,
    LineInterpolation, MatrixOrientation, MatrixRef, ScatterShape, Shading,
};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::tri_mesh::{MeshValueLocation, MeshValues, TriMeshSeries};
use renderer::view3d::{SurfaceStyle, SurfaceWireframe, View3d};
use renderer::{
    Chart, Color, ColorMap, Config, DataRenderType, DataSelectionsConfig, PickedDataRef,
    RasterImage, Renderer, RendererDevice, SeriesConfig, TriangleSource,
};

const WIDTH: u32 = 520;
const HEIGHT: u32 = 440;

fn col(data: Vec<f64>) -> Column<f64> {
    Column { data }
}

fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(|| {
        let (device, queue) = test_gpu::device();
        (device, queue)
    });
    (device.clone(), queue.clone())
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
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
        render_type: DataRenderType::Field(renderer::data_config::FieldSeries {
            matrix: matrix(nx),
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading,
                opacity: 1.0,
            }),
            contour: None,
            surface,
        }),
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
    config.set_view_3d(Some(View3d::default()));
    config
}

fn ridge(x: f64, y: f64) -> f64 {
    (-((x - 0.5).powi(2) + (y - 0.4).powi(2)) / 0.05).exp()
}

fn export(r: &mut Renderer, config: &Config, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_rgba(
        renderer::ExportSource::chart(&Chart::new(config.clone()), series),
        renderer::ExportOptions::panel(scale),
    )
    .block_on()
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
            test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap()
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
        let miss = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
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
        render_type: DataRenderType::Field(renderer::data_config::FieldSeries {
            matrix: matrix(21),
            fill: None,
            contour: Some(contour_config(vec![0.5])),
            surface: None,
        }),
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
    match test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap() {
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

/// A floor isoline is `line_width` wide on screen whichever way it runs
/// (#259): where the circle runs across the view the floor is foreshortened,
/// and a stroke measured in floor-plane pixels came out much thinner there
/// than where it runs up the screen.
#[test]
fn floor_contours_keep_their_screen_width_in_every_direction() {
    let mut r = renderer();
    upload_grid(&mut r, 41, 33, ridge);
    let config = chart_3d();
    let mut contour = contour_config(vec![0.5]);
    contour.line.line_width = 2.0;
    let series = vec![SeriesConfig {
        series_id: "floor".into(),
        render_type: DataRenderType::Field(renderer::data_config::FieldSeries {
            matrix: matrix(41),
            fill: None,
            contour: Some(contour),
            surface: None,
        }),
        ..surface_series(41, Shading::Interpolated, None)
    }];
    let img = export(&mut r, &config, &series, 1.0);
    save("surface-floor-contour-width", &img);
    let radius = (0.05f64 * std::f64::consts::LN_2).sqrt();
    let on_screen = |a: f64| {
        let p = config
            .project_3d([0.5 + radius * a.cos(), 0.4 + radius * a.sin(), 0.0])
            .unwrap();
        (p[0], p[1])
    };
    // The stroke's width across its screen tangent at angle `a` of the
    // circle: the red coverage summed along the screen normal.
    let width_at = |a: f64| {
        let p = on_screen(a);
        let (q0, q1) = (on_screen(a - 1e-3), on_screen(a + 1e-3));
        let (tx, ty) = (q1.0 - q0.0, q1.1 - q0.1);
        let len = tx.hypot(ty);
        let (nx, ny) = (-ty / len, tx / len);
        let mut sum = 0.0;
        let step = 0.25;
        let mut d = -8.0;
        while d <= 8.0 {
            let px = pixel(&img, (p.0 + nx * d, p.1 + ny * d));
            let cover = (f64::from(px[0]) - f64::from(px[1])).max(0.0) / 255.0;
            sum += cover * f64::from(px[3]) / 255.0 * step;
            d += step;
        }
        sum
    };
    let widths: Vec<(f64, f64)> = (0..16)
        .map(|k| {
            let a = f64::from(k) / 16.0 * std::f64::consts::TAU;
            (a, width_at(a))
        })
        .collect();
    let (min, max) = widths
        .iter()
        .fold((f64::INFINITY, 0.0f64), |(lo, hi), (_, w)| {
            (lo.min(*w), hi.max(*w))
        });
    assert!(
        min > 1.2,
        "every stroke is about 2 px wide on screen: {widths:?}"
    );
    // Pixel sampling spreads a 2 px stroke over 1.6 to 2.5; measured in
    // floor-plane pixels the same circle ran from 0 (broken) to 1.3.
    assert!(
        max / min < 1.75,
        "the widths agree across directions: {widths:?}"
    );
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
            render_type: DataRenderType::Field(renderer::data_config::FieldSeries {
                matrix: matrix(21),
                fill: Some(FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: Shading::Interpolated,
                    opacity,
                }),
                contour: Some(contour_config(vec![0.5])),
                surface: None,
            }),
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
    let mut view = r.create_chart_view(&chart, rect).unwrap();
    let styles: Vec<_> = series
        .iter()
        .map(|cfg| r.create_style_for_series(cfg).unwrap())
        .collect();
    let drawn: Vec<_> = series
        .iter()
        .zip(&styles)
        .map(|(config, style)| renderer::Series { config, style })
        .collect();
    let mut items = [renderer::ChartDrawItem {
        view: &mut view,
        chart: &chart,
        series: &drawn,
    }];
    let prepared = r.prepare(&mut items).unwrap();
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
    config.view_3d_mut().unwrap().projection = renderer::view3d::Projection3d::Perspective;
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
        RendererDevice::new(device, queue).expect("figgy poll thread"),
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
        DataRenderType::Xy(renderer::data_config::XySeries {
            scatter: Some(red_points(5.0)),
            line: Some(red_line(3.0, LineStylePreset::Solid)),
            errors: None,
            stem: None,
        }),
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
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(red_points(6.0)),
                line: None,
                errors: None,
                stem: None,
            }),
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
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(red_line(3.0, LineStylePreset::Solid)),
                errors: None,
                stem: None,
            }),
        ),
        point_series(
            "dots",
            "tx",
            "ty",
            "tz",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(red_points(5.0)),
                line: None,
                errors: None,
                stem: None,
            }),
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
            test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap()
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
        DataRenderType::Xy(renderer::data_config::XySeries {
            scatter: Some(red_points(5.0)),
            line: None,
            errors: None,
            stem: None,
        }),
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
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(red_line(3.0, style)),
                errors: None,
                stem: None,
            }),
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
        DataRenderType::Xy(renderer::data_config::XySeries {
            scatter: Some(red_points(5.0)),
            line: Some(red_line(3.0, LineStylePreset::Solid)),
            errors: None,
            stem: None,
        }),
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
        RendererDevice::new(device, queue).expect("figgy poll thread"),
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
        DataRenderType::Xy(renderer::data_config::XySeries {
            scatter: Some(red_points(3.0)),
            line: Some(red_line(2.0, LineStylePreset::Solid)),
            errors: None,
            stem: None,
        }),
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
    let picked = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
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

/// A 3D series' height column is one of the columns it reads (#207):
/// removing it removes the series, a streamed one is refused with the
/// explicit error, and the draw info and `step_selection` stop at the
/// shortest of x, y and z on a 3D chart. A flat chart keeps the column
/// unused, so there it bounds nothing.
#[test]
fn a_3d_series_height_column_is_enumerated_with_its_columns() {
    let mut r = renderer();
    upload_helix(&mut r, 20);
    let _ = r.remove_column("tz_short");
    r.add_column("tz_short", &col(vec![0.5; 12])).unwrap();
    let dots = |z: &str| {
        point_series(
            "dots",
            "tx",
            "ty",
            z,
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(red_points(5.0)),
                line: None,
                errors: None,
                stem: None,
            }),
        )
    };
    let config = chart_3d();
    let mut flat = config.clone();
    flat.set_view_3d(None);

    // The draw info counts the height on a 3D chart only.
    let id = r
        .register_chart(config.clone(), vec![dots("tz_short")])
        .unwrap();
    let info = r.series_draw_info(id, "dots").unwrap();
    assert_eq!((info.drawn_count, info.truncated), (12, true), "{info:?}");
    let flat_id = r
        .register_chart(flat.clone(), vec![dots("tz_short")])
        .unwrap();
    let info = r.series_draw_info(flat_id, "dots").unwrap();
    assert_eq!((info.drawn_count, info.truncated), (20, false), "{info:?}");

    // `step_selection` stops at the same last row.
    let series = [dots("tz_short")];
    let len = |id: &str| match id {
        "tx" | "ty" => Some(20),
        "tz_short" => Some(12),
        _ => None,
    };
    for (chart, last) in [(&config, 11), (&flat, 19)] {
        let mut stepped = chart.clone();
        stepped.picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::Point {
                source_id: None,
                series_id: "dots".into(),
                point_index: 5,
            }],
            ..DataSelectionsConfig::default()
        });
        assert_eq!(stepped.step_selection(&series, 100, 0, len), Ok(true));
        assert!(
            matches!(
                stepped.picked_data.as_ref().unwrap().refs[0],
                PickedDataRef::Point { point_index, .. } if point_index == last
            ),
            "{:?}",
            stepped.picked_data
        );
    }

    // Removing the height column removes the series from both charts.
    assert!(r.remove_column("tz_short").unwrap());
    assert!(r.chart_series(id).unwrap().is_empty());
    assert!(r.chart_series(flat_id).unwrap().is_empty());

    // A streamed height column is refused with the explicit error.
    r.register_streamed_columns(vec![renderer::StreamColumn {
        id: "st_z".into(),
        len: 20,
        revision: 1,
        encoding: renderer::StreamEncoding::ScalarF32,
        replay: renderer::StreamReplay::RandomAccess,
        statistics: renderer::StreamStatistics::Unknown,
    }])
    .unwrap();
    let refused = r
        .export_rgba(
            renderer::ExportSource::chart(&Chart::new(config), &[dots("st_z")]),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .err();
    assert!(
        matches!(&refused, Some(renderer::FiggyError::ColumnNotResident { id }) if id == "st_z"),
        "{refused:?}"
    );
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
        DataRenderType::Xy(renderer::data_config::XySeries {
            scatter: Some(red_points(5.0)),
            line: None,
            errors: None,
            stem: None,
        }),
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
        DataRenderType::Xy(renderer::data_config::XySeries {
            scatter: Some(red_points(5.0)),
            line: None,
            errors: None,
            stem: None,
        }),
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

// ── 3D vector fields (#211) ────────────────────────────────────────────────

/// Arrows at `anchors` along `vectors`, as columns `vx`, `vy`, `vz` (the
/// anchor) and `vu`, `vv`, `vw` (the components).
fn upload_arrows(r: &mut Renderer, anchors: &[[f64; 3]], vectors: &[[f64; 3]]) {
    let column = |set: &[[f64; 3]], k: usize| col(set.iter().map(|p| p[k]).collect());
    for (id, data) in [
        ("vx", column(anchors, 0)),
        ("vy", column(anchors, 1)),
        ("vz", column(anchors, 2)),
        ("vu", column(vectors, 0)),
        ("vv", column(vectors, 1)),
        ("vw", column(vectors, 2)),
    ] {
        let _ = r.remove_column(id);
        r.add_column(id, &data).unwrap();
    }
}

fn arrows_3d(
    scale: renderer::data_config::VectorScale,
    pivot: renderer::data_config::VectorPivot,
) -> SeriesConfig {
    point_series(
        "arrows",
        "vx",
        "vy",
        "vz",
        DataRenderType::Vector {
            components: renderer::data_config::VectorComponents::Points {
                u: "vu".into(),
                v: "vv".into(),
                w: Some("vw".into()),
            },
            style: renderer::data_config::VectorStyle {
                color: RED,
                width_px: 2.0,
                scale,
                pivot,
                head_length_px: 10.0,
                head_width_px: 8.0,
                min_length_px: 0.0,
                color_map: None,
            },
        },
    )
}

/// A 3D arrow in `Data` mode runs from its anchor to `anchor + factor · v`
/// in the box: red ink along the projected shaft at 1x and 2x, none past
/// the tip or beside the anchor.
#[test]
fn data_arrows_run_from_their_anchors_along_their_vectors() {
    let mut r = renderer();
    let anchors = [[0.2, 0.3, 0.2], [0.6, 0.6, 0.5], [0.4, 0.2, 0.7]];
    let vectors = [[1.0, 0.0, 0.0], [0.0, -1.0, 1.0], [0.0, 1.0, -1.0]];
    upload_arrows(&mut r, &anchors, &vectors);
    let config = chart_3d();
    let factor = 0.25;
    let series = [arrows_3d(
        renderer::data_config::VectorScale::Data { factor },
        renderer::data_config::VectorPivot::Tail,
    )];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        save(&format!("vector_3d_data_{scale}x"), &img);
        let displayed = config.scaled(scale);
        for (a, v) in anchors.iter().zip(&vectors) {
            let at = |s: f64| {
                displayed
                    .project_3d([
                        a[0] + s * factor * v[0],
                        a[1] + s * factor * v[1],
                        a[2] + s * factor * v[2],
                    ])
                    .unwrap()
            };
            for s in [0.15, 0.4, 0.6] {
                let p = at(s);
                assert!(
                    red_near(&img, (p[0], p[1]), 1.5 * f64::from(scale)),
                    "{scale}x: no shaft at {s} of the arrow from {a:?}, {p:?}"
                );
            }
            // Well past the tip along the same direction: nothing.
            let past = at(1.6);
            assert!(
                !red_near(&img, (past[0], past[1]), 2.0 * f64::from(scale)),
                "{scale}x: ink past the tip of the arrow from {a:?}"
            );
        }
    }
}

/// An arrow behind the surface's peak is hidden by the depth test; one on
/// the near face shows over it, at the front of its tube.
#[test]
fn arrows_are_depth_tested_against_the_surface() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, ridge);
    let (shown, hidden) = ([0.5, 3.0 / 16.0], [0.5, 0.6]);
    let anchors = [shown, hidden].map(|p| [p[0], p[1], ridge(p[0], p[1])]);
    // Short arrows along x, lying on the surface where they sit.
    upload_arrows(&mut r, &anchors, &[[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
    let config = chart_3d();
    let series = [
        surface_series(21, Shading::Interpolated, None),
        arrows_3d(
            renderer::data_config::VectorScale::Data { factor: 0.12 },
            renderer::data_config::VectorPivot::Middle,
        ),
    ];
    let img = export(&mut r, &config, &series, 1.0);
    save("vector_3d_depth_tested", &img);
    let at = |p: [f64; 3]| {
        let q = config.project_3d(p).unwrap();
        (q[0], q[1])
    };
    assert!(
        red_near(&img, at(anchors[0]), 1.5),
        "the arrow on the near face is drawn"
    );
    assert!(
        !red_near(&img, at(anchors[1]), 3.0),
        "the arrow behind the peak is hidden"
    );
    let alone = export(&mut r, &config, &series[1..], 1.0);
    assert!(
        red_near(&alone, at(anchors[1]), 1.5),
        "without the surface it shows"
    );
}

/// The ID pick answers the arrow under the cursor as its row, on the shaft
/// and on the head, and nothing beside it; an arrow behind the surface's
/// peak never answers.
#[test]
fn the_id_pick_answers_the_arrow_under_the_cursor() {
    let mut r = renderer();
    upload_grid(&mut r, 21, 17, |x, y| 0.4 * ridge(x, y));
    let anchors = [[0.15, 0.5, 0.8], [0.75, 0.3, 0.7], [0.5, 0.75, 0.1]];
    let vectors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    upload_arrows(&mut r, &anchors, &vectors);
    let config = chart_3d();
    let factor = 0.3;
    let series = vec![
        surface_series(21, Shading::Interpolated, None),
        arrows_3d(
            renderer::data_config::VectorScale::Data { factor },
            renderer::data_config::VectorPivot::Tail,
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
            test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap()
        };
        let row = |picked: Option<renderer::PickedData>| match picked {
            Some(renderer::PickedData::Point {
                series_id,
                point_index,
                ..
            }) if series_id == "arrows" => Some(point_index),
            _ => None,
        };
        for (i, (a, v)) in anchors.iter().zip(&vectors).enumerate() {
            let along = |s: f64| {
                [
                    a[0] + s * factor * v[0],
                    a[1] + s * factor * v[1],
                    a[2] + s * factor * v[2],
                ]
            };
            assert_eq!(
                row(pick(&mut r, along(0.3), [0.0, 0.0])),
                Some(i),
                "{scale}x: shaft of arrow {i}"
            );
            assert_eq!(
                row(pick(&mut r, along(0.95), [0.0, 0.0])),
                Some(i),
                "{scale}x: head of arrow {i}"
            );
            // Well past the tip: not this arrow.
            assert_ne!(
                row(pick(&mut r, along(1.8), [0.0, 0.0])),
                Some(i),
                "{scale}x: past arrow {i}"
            );
        }
    }
}

/// `Pixels` arrows are `px_per_unit · |v|` long in the screen plane at the
/// anchor: an arrow along the screen plane's direction measures that length
/// at 1x and 2x (twice at 2x), and a zoom keeps it.
#[test]
fn pixel_arrows_keep_their_screen_length() {
    let mut r = renderer();
    let config = chart_3d();
    // The screen plane at the box centre: a direction perpendicular to the
    // eye, which the 2D projection of the box makes horizontal-ish. Take the
    // box x axis and remove its component along the eye.
    let anchor = [0.5, 0.5, 0.5];
    let px_per_unit = 40.0;
    for scale in [1.0f32, 2.0] {
        let displayed = config.scaled(scale);
        let c = displayed.project_3d(anchor).unwrap();
        // The arrow's tip on screen is `len` px from the anchor at most,
        // and exactly `len` when the vector lies in the screen plane.
        for v in [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]] {
            upload_arrows(&mut r, &[anchor], &[v]);
            let series = [arrows_3d(
                renderer::data_config::VectorScale::Pixels { px_per_unit },
                renderer::data_config::VectorPivot::Tail,
            )];
            let img = export(&mut r, &config, &series, scale);
            save(&format!("vector_3d_pixels_{scale}x"), &img);
            // The farthest red pixel from the anchor.
            let mut reach = 0.0f64;
            for y in 0..img.height {
                for x in 0..img.width {
                    let p = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                    if is_red(pixel(&img, p)) {
                        reach = reach.max(((p.0 - c[0]).powi(2) + (p.1 - c[1]).powi(2)).sqrt());
                    }
                }
            }
            let len = px_per_unit * f64::from(scale);
            assert!(
                reach > 0.3 * len,
                "{scale}x: the arrow {v:?} draws ({reach:.1} px)"
            );
            assert!(
                reach <= len + 2.0 * f64::from(scale),
                "{scale}x: the arrow {v:?} is no longer than {len} px on screen ({reach:.1})"
            );
        }
    }
}

/// A `Data` arrow whose tip leaves the box is not drawn, as a 3D line drops
/// a segment whose end leaves it (D6); an anchor outside the box hides its
/// arrow in either mode.
#[test]
fn an_arrow_leaving_the_box_is_not_drawn() {
    let mut r = renderer();
    let anchors = [[0.9, 0.5, 0.5], [0.5, 0.5, 1.2]];
    let vectors = [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
    upload_arrows(&mut r, &anchors, &vectors);
    let config = chart_3d();
    let img = export(
        &mut r,
        &config,
        &[arrows_3d(
            renderer::data_config::VectorScale::Data { factor: 0.3 },
            renderer::data_config::VectorPivot::Tail,
        )],
        1.0,
    );
    save("vector_3d_outside", &img);
    for (k, a) in anchors.iter().enumerate() {
        let p = config
            .project_3d([a[0].min(1.0), a[1], a[2].min(1.0)])
            .unwrap();
        assert!(!red_near(&img, (p[0], p[1]), 3.0), "arrow {k} is hidden");
    }
}

/// The arrow from `anchor` along `v` with its `Data` factor, sampled at
/// fraction `s` of its length (`s = 0` is the anchor).
fn along(anchor: [f64; 3], v: [f64; 3], factor: f64, s: f64) -> [f64; 3] {
    [0, 1, 2].map(|k| anchor[k] + s * factor * v[k])
}

/// The ID pick at canvas pixel `at` of a registered chart.
fn pick_at(
    r: &mut Renderer,
    id: renderer::ChartId,
    at: [f64; 2],
    scale: f32,
) -> Option<renderer::PickedData> {
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at[0] as f32, at[1] as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: (WIDTH as f32 * scale) as u32,
            height: (HEIGHT as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: 0.0,
    };
    test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap()
}

/// All three pivots put their point of the arrow on the anchor: `Tail`
/// runs from it, `Middle` straddles it and `Tip` ends at it.
#[test]
fn pivots_place_the_tail_middle_or_tip_on_the_anchor() {
    use renderer::data_config::{VectorPivot, VectorScale};
    let mut r = renderer();
    let (anchor, v, factor) = ([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 0.3);
    upload_arrows(&mut r, &[anchor], &[v]);
    let config = chart_3d();
    // (pivot, fractions inked, fractions clear), relative to the anchor.
    for (pivot, inked, clear) in [
        (VectorPivot::Tail, [0.2, 0.5], [-0.5, -0.3]),
        (VectorPivot::Middle, [-0.3, 0.2], [-0.8, 0.8]),
        (VectorPivot::Tip, [-0.8, -0.5], [0.3, 0.5]),
    ] {
        let series = [arrows_3d(VectorScale::Data { factor }, pivot)];
        for scale in [1.0f32, 2.0] {
            let img = export(&mut r, &config, &series, scale);
            save(&format!("vector_3d_pivot_{pivot:?}_{scale}x"), &img);
            let displayed = config.scaled(scale);
            let at = |s: f64| {
                let p = displayed.project_3d(along(anchor, v, factor, s)).unwrap();
                (p[0], p[1])
            };
            for s in inked {
                assert!(
                    red_near(&img, at(s), 1.5 * f64::from(scale)),
                    "{pivot:?} {scale}x: ink at {s}"
                );
            }
            for s in clear {
                assert!(
                    !red_near(&img, at(s), 1.5 * f64::from(scale)),
                    "{pivot:?} {scale}x: no ink at {s}"
                );
            }
        }
    }
}

/// A logarithmic z axis places the anchor and the `Data` tip by their
/// logarithms: the arrow from z = 10 to z = 50 is drawn between those
/// heights in the box, and its screen midpoint is at their geometric mean.
#[test]
fn arrows_follow_a_logarithmic_z_axis() {
    use renderer::data_config::{VectorPivot, VectorScale};
    let mut r = renderer();
    let (anchor, v, factor) = ([0.3, 0.6, 10.0], [0.0, 0.0, 1.0], 40.0);
    upload_arrows(&mut r, &[anchor], &[v]);
    let mut config = chart_3d();
    config.right_y.scale = renderer::config::AxisScale::Logarithmic;
    config.right_y.min = 1.0;
    config.right_y.max = 100.0;
    let series = [arrows_3d(VectorScale::Data { factor }, VectorPivot::Tail)];
    let img = export(&mut r, &config, &series, 1.0);
    save("vector_3d_log_z", &img);
    let box_at = |z: f64| {
        let p = config
            .project_3d([anchor[0], anchor[1], z.log10() / 2.0])
            .unwrap();
        (p[0], p[1])
    };
    for z in [12.0, (10.0f64 * 50.0).sqrt(), 40.0] {
        assert!(red_near(&img, box_at(z), 1.5), "ink at z = {z}");
    }
    for z in [5.0, 80.0] {
        assert!(!red_near(&img, box_at(z), 1.5), "no ink at z = {z}");
    }
}

/// A perspective camera places the arrows where `project_3d` puts their
/// shafts, which is not where the orthographic camera puts them.
#[test]
fn a_perspective_camera_draws_the_arrows() {
    use renderer::data_config::{VectorPivot, VectorScale};
    let mut r = renderer();
    let anchors = [[0.1, 0.2, 0.3], [0.7, 0.8, 0.6]];
    let vectors = [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0]];
    let factor = 0.2;
    upload_arrows(&mut r, &anchors, &vectors);
    let mut config = chart_3d();
    config.view_3d_mut().unwrap().projection = renderer::view3d::Projection3d::Perspective;
    let series = [arrows_3d(VectorScale::Data { factor }, VectorPivot::Tail)];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        save(&format!("vector_3d_perspective_{scale}x"), &img);
        let displayed = config.scaled(scale);
        for (a, v) in anchors.iter().zip(&vectors) {
            for s in [0.2, 0.5] {
                let p = displayed.project_3d(along(*a, *v, factor, s)).unwrap();
                let ortho = chart_3d()
                    .scaled(scale)
                    .project_3d(along(*a, *v, factor, s))
                    .unwrap();
                assert!(
                    (p[0] - ortho[0]).hypot(p[1] - ortho[1]) > 2.0,
                    "the perspective camera moves the shaft"
                );
                assert!(
                    red_near(&img, (p[0], p[1]), 1.5 * f64::from(scale)),
                    "{scale}x: shaft of the arrow from {a:?} at {s}"
                );
            }
        }
    }
}

/// The picked arrow's anchor takes the ring at the glyph radius (half the
/// head width) plus the selection's extra, at 1x and 2x; the other arrow
/// takes none.
#[test]
fn the_picked_ring_surrounds_the_3d_arrow_anchor() {
    use renderer::data_config::{VectorPivot, VectorScale};
    let mut r = renderer();
    let anchors = [[0.3, 0.3, 0.4], [0.7, 0.6, 0.6]];
    let vectors = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    upload_arrows(&mut r, &anchors, &vectors);
    let mut config = chart_3d();
    let series = [arrows_3d(
        VectorScale::Data { factor: 0.15 },
        VectorPivot::Tail,
    )];
    let unpicked = config.clone();
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "arrows".into(),
            point_index: 1,
        }],
        ..DataSelectionsConfig::default()
    });
    let selection = config.picked_data.clone().unwrap();
    for scale in [1.0f32, 2.0] {
        let plain = export(&mut r, &unpicked, &series, scale);
        let img = export(&mut r, &config, &series, scale);
        save(&format!("vector_3d_ring_{scale}x"), &img);
        let displayed = config.scaled(scale);
        // Head width 8: the anchor radius is 4 logical pixels.
        let ring_radius = f64::from((4.0 + selection.point_radius_extra_px) * scale);
        let on_ring = |img: &RasterImage, at: [f64; 3]| {
            let p = displayed.project_3d(at).unwrap();
            (0..16)
                .filter(|k| {
                    let a = f64::from(*k) * std::f64::consts::TAU / 16.0;
                    let q = (p[0] + ring_radius * a.cos(), p[1] + ring_radius * a.sin());
                    let got = pixel(img, q);
                    got[3] > 60 && !is_red(got) && got != pixel(&plain, q)
                })
                .count()
        };
        let picked = on_ring(&img, anchors[1]);
        assert!(picked >= 10, "{scale}x: ring pixels on {picked} of 16");
        let other = on_ring(&img, anchors[0]);
        assert_eq!(other, 0, "{scale}x: the unpicked anchor takes no ring");
    }
}

/// A 3D subplot draws its arrows beside a flat panel, which draws the same
/// series flat: `z_column` and `w` are kept and unused there.
#[test]
fn a_3d_subplot_draws_arrows_beside_a_flat_panel() {
    use renderer::data_config::{VectorPivot, VectorScale};
    let mut r = renderer();
    let (anchor, v, factor) = ([0.4, 0.5, 0.5], [1.0, 0.0, 0.0], 0.3);
    upload_arrows(&mut r, &[anchor], &[v]);
    let mut flat = chart_3d();
    flat.set_view_3d(None);
    flat.colorbar = None;
    let mut subplot = chart_3d();
    subplot.chart_area = ChartArea(Rect {
        x: WIDTH,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    let flat_arrows = arrows_3d(VectorScale::Data { factor }, VectorPivot::Tail);
    let mut arrows_3d_sub = flat_arrows.clone();
    arrows_3d_sub.series_id = "arrows_3d".into();
    let mut parent = Chart::new(flat.clone());
    parent.config_mut().subplots.push(renderer::Subplot {
        config: subplot.clone(),
        series: vec!["arrows_3d".into()],
        link_x: None,
        link_y: None,
        marker: None,
        text_scale: Default::default(),
    });
    let series = [flat_arrows, arrows_3d_sub];
    for scale in [1.0f32, 2.0] {
        let document = r
            .export_rgba(
                renderer::ExportSource::chart(&parent, &series),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        save(&format!("vector_3d_subplot_{scale}x"), &document);
        let s = f64::from(scale);
        assert_eq!(
            (document.width, document.height),
            (
                (2.0 * f64::from(WIDTH) * s) as u32,
                (f64::from(HEIGHT) * s) as u32
            )
        );
        // The 3D panel: the shaft where its camera puts it, offset by its area.
        let displayed = subplot.scaled(scale);
        for f in [0.3, 0.6] {
            let p = displayed.project_3d(along(anchor, v, factor, f)).unwrap();
            assert!(
                red_near(&document, (p[0], p[1]), 1.5 * s),
                "{scale}x: 3D shaft at {f}: {p:?}"
            );
        }
        // The flat panel: a flat arrow along x through the data area.
        let red_left = (0..(f64::from(HEIGHT) * s) as u32)
            .flat_map(|y| (0..(f64::from(WIDTH) * s) as u32).map(move |x| (x, y)))
            .filter(|&(x, y)| is_red(pixel(&document, (f64::from(x), f64::from(y)))))
            .count();
        assert!(
            red_left > 20,
            "{scale}x: the flat panel draws its arrow ({red_left})"
        );
    }
}

/// Rule 10: `(1 << 20) + 1` arrows draw whole, the draw info counts them
/// all, the last row picks and `step_selection` reaches it. The renderer
/// caps no row count. All rows but the last cluster in one corner of the
/// box so the last stands alone.
#[test]
fn a_million_arrows_draw_pick_and_step() {
    use renderer::data_config::{VectorPivot, VectorScale};
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Bgra8Unorm,
        256 * 1024 * 1024,
    )
    .expect("renderer init");
    let n = (1 << 20) + 1;
    let corner = |i: usize| {
        let k = i as f64 / n as f64;
        [
            0.05 + 0.1 * k,
            0.05 + 0.1 * (k * 7.0).fract(),
            0.1 + 0.1 * (k * 13.0).fract(),
        ]
    };
    let last = [0.6, 0.6, 0.6];
    let mut anchors: Vec<[f64; 3]> = (0..n - 1).map(corner).collect();
    anchors.push(last);
    let vectors = vec![[1.0, 0.0, 0.0]; n];
    upload_arrows(&mut r, &anchors, &vectors);
    let factor = 0.25;
    let config = chart_3d();
    let series = vec![arrows_3d(VectorScale::Data { factor }, VectorPivot::Tail)];
    let started = std::time::Instant::now();
    let img = export(&mut r, &config, &series, 1.0);
    eprintln!("{n} 3D arrows: export {:?}", started.elapsed());
    save("vector_3d_million", &img);
    let at = |s: f64| {
        let p = config
            .project_3d(along(last, [1.0, 0.0, 0.0], factor, s))
            .unwrap();
        [p[0], p[1]]
    };
    assert!(
        red_near(&img, (at(0.4)[0], at(0.4)[1]), 1.5),
        "the last row draws"
    );
    let first = config
        .project_3d(along(corner(0), [1.0, 0.0, 0.0], factor, 0.5))
        .unwrap();
    assert!(
        red_near(&img, (first[0], first[1]), 1.5),
        "the first row draws"
    );

    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    let info = r.series_draw_info(id, "arrows").unwrap();
    assert_eq!(
        (info.drawn_count, info.truncated),
        (n as u64, false),
        "{info:?}"
    );
    let started = std::time::Instant::now();
    let picked = pick_at(&mut r, id, at(0.4), 1.0);
    eprintln!("{n} 3D arrows: pick {:?}", started.elapsed());
    match picked {
        Some(renderer::PickedData::Point { point_index, .. }) => {
            assert_eq!(point_index, n - 1)
        }
        other => panic!("{other:?}"),
    }
    let mut stepped = config.clone();
    stepped.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "arrows".into(),
            point_index: n - 2,
        }],
        ..DataSelectionsConfig::default()
    });
    let len = |_: &str| Some(n);
    assert_eq!(stepped.step_selection(&series, 5, 0, len), Ok(true));
    assert!(matches!(
        stepped.picked_data.as_ref().unwrap().refs[0],
        PickedDataRef::Point { point_index, .. } if point_index == n - 1
    ));
}

/// The height and the z component are columns the 3D arrow reads (#211):
/// the draw info and `step_selection` stop at the shortest of them on a 3D
/// chart and ignore them on a flat one, removing `w` removes the series,
/// an unregistered `w` is refused at registration on a 3D chart only, and a
/// streamed `w` is refused with the explicit error.
#[test]
fn a_3d_arrow_reads_its_height_and_z_component() {
    use renderer::data_config::{VectorComponents, VectorPivot, VectorScale};
    let mut r = renderer();
    let anchors: Vec<[f64; 3]> = (0..20).map(|i| [0.04 * i as f64, 0.5, 0.5]).collect();
    upload_arrows(&mut r, &anchors, &vec![[0.0, 0.0, 1.0]; 20]);
    let _ = r.remove_column("vw_short");
    r.add_column("vw_short", &col(vec![1.0; 12])).unwrap();
    let with_w = |w: &str| {
        let mut series = arrows_3d(VectorScale::Data { factor: 0.1 }, VectorPivot::Tail);
        if let DataRenderType::Vector {
            components: VectorComponents::Points { w: slot, .. },
            ..
        } = &mut series.render_type
        {
            *slot = Some(w.into());
        }
        series
    };
    let config = chart_3d();
    let mut flat = config.clone();
    flat.set_view_3d(None);
    flat.colorbar = None;

    let id = r
        .register_chart(config.clone(), vec![with_w("vw_short")])
        .unwrap();
    let info = r.series_draw_info(id, "arrows").unwrap();
    assert_eq!((info.drawn_count, info.truncated), (12, true), "{info:?}");
    let flat_id = r
        .register_chart(flat.clone(), vec![with_w("vw_short")])
        .unwrap();
    let info = r.series_draw_info(flat_id, "arrows").unwrap();
    assert_eq!((info.drawn_count, info.truncated), (20, false), "{info:?}");

    let series = [with_w("vw_short")];
    let len = |id: &str| match id {
        "vw_short" => Some(12),
        "vx" | "vy" | "vz" | "vu" | "vv" | "vw" => Some(20),
        _ => None,
    };
    for (chart, last) in [(&config, 11), (&flat, 19)] {
        let mut stepped = chart.clone();
        stepped.picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::Point {
                source_id: None,
                series_id: "arrows".into(),
                point_index: 5,
            }],
            ..DataSelectionsConfig::default()
        });
        assert_eq!(stepped.step_selection(&series, 100, 0, len), Ok(true));
        assert!(
            matches!(
                stepped.picked_data.as_ref().unwrap().refs[0],
                PickedDataRef::Point { point_index, .. } if point_index == last
            ),
            "{:?}",
            stepped.picked_data
        );
    }

    // Removing w removes the series from both charts.
    assert!(r.remove_column("vw_short").unwrap());
    assert!(r.chart_series(id).unwrap().is_empty());
    assert!(r.chart_series(flat_id).unwrap().is_empty());

    // An unregistered w: refused on the 3D chart, kept unused on the flat.
    let err = r
        .register_chart(config.clone(), vec![with_w("vw_missing")])
        .err()
        .expect("a 3D arrow reads its w");
    assert!(err.to_string().contains("vw_missing"), "{err}");
    r.register_chart(flat.clone(), vec![with_w("vw_missing")])
        .expect("a flat arrow does not read w");

    // A streamed w is refused with the explicit error.
    r.register_streamed_columns(vec![renderer::StreamColumn {
        id: "st_w".into(),
        len: 20,
        revision: 1,
        encoding: renderer::StreamEncoding::ScalarF32,
        replay: renderer::StreamReplay::RandomAccess,
        statistics: renderer::StreamStatistics::Unknown,
    }])
    .unwrap();
    let refused = r
        .export_rgba(
            renderer::ExportSource::chart(&Chart::new(config), &[with_w("st_w")]),
            renderer::ExportOptions::panel(1.0),
        )
        .block_on()
        .err();
    assert!(
        matches!(&refused, Some(renderer::FiggyError::ColumnNotResident { id }) if id == "st_w"),
        "{refused:?}"
    );
}

/// The z fit spans a 3D arrow's anchor heights and, in `Data` mode, the
/// bound `[z_min + f·w_min, z_max + f·w_max]` of its tips (D7), ordered
/// for a negative factor; `Pixels` arrows fit their anchors only.
#[test]
fn the_z_fit_spans_the_arrow_tips() {
    use renderer::data_config::{VectorPivot, VectorScale};
    let mut r = renderer();
    let anchors = [[0.5, 0.5, 0.2], [0.5, 0.5, 0.6], [0.5, 0.5, 0.4]];
    let vectors = [[0.0, 0.0, -1.0], [0.0, 0.0, 2.0], [0.0, 0.0, 0.5]];
    upload_arrows(&mut r, &anchors, &vectors);
    for (scale, want) in [
        (VectorScale::Data { factor: 0.125 }, (0.075, 0.85)),
        (VectorScale::Data { factor: -0.125 }, (-0.05, 0.725)),
        (VectorScale::Pixels { px_per_unit: 40.0 }, (0.2, 0.6)),
    ] {
        let mut chart = Chart::new(chart_3d());
        let series = [arrows_3d(scale.clone(), VectorPivot::Tail)];
        chart
            .auto_fit_view_3d_z(r.pool(), renderer::LayerRef::Base, &series, 0.0)
            .unwrap();
        let z = &chart.config().right_y;
        assert!(
            (z.min - want.0).abs() < 1e-9 && (z.max - want.1).abs() < 1e-9,
            "{scale:?}: {} {}",
            z.min,
            z.max
        );
    }
}

// ───── Triangle meshes as surfaces (#210) ───────────────────────────────────

/// The vertex lattice of most test meshes: `MESH_N × MESH_N` vertices over
/// the unit square, two triangles per cell. 41 puts vertices at 0.5 and
/// 0.4, the ridge's peak.
const MESH_N: usize = 41;

/// Upload `f` over the unit square as a triangle mesh of an `n × n` vertex
/// lattice: the vertex columns `mx` and `my`, the heights `mz` (the values
/// the mesh is coloured by too) and the triangle index column `mtri`: cell
/// `(i, j)`'s lower triangle `(a, b, d)` then its upper `(a, d, c)`, with
/// `a = j·n + i`, `b = a + 1`, `c = a + n`, `d = c + 1`.
fn upload_mesh(r: &mut Renderer, n: usize, f: impl Fn(f64, f64) -> f64) {
    let at = |k: usize| k as f64 / (n - 1) as f64;
    let (mut xs, mut ys, mut zs) = (Vec::new(), Vec::new(), Vec::new());
    for j in 0..n {
        for i in 0..n {
            xs.push(at(i));
            ys.push(at(j));
            zs.push(f(at(i), at(j)));
        }
    }
    let mut tris: Vec<u32> = Vec::new();
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = (j * n + i) as u32;
            let (b, c) = (a + 1, a + n as u32);
            let d = c + 1;
            tris.extend([a, b, d, a, d, c]);
        }
    }
    r.upsert_column("mx", &col(xs)).unwrap();
    r.upsert_column("my", &col(ys)).unwrap();
    r.upsert_column("mz", &col(zs)).unwrap();
    r.upsert_triangles(
        "mtri",
        &TriangleSource {
            indices: &Column { data: tris },
        },
    )
    .unwrap();
}

/// The lower triangle `(a, b, d)` of cell `(i, j)` of an `n × n` lattice.
fn lower_triangle(i: usize, j: usize, n: usize) -> usize {
    2 * (j * (n - 1) + i)
}

/// Vertex `(i, j)` of an `n × n` lattice.
fn mesh_vertex(i: usize, j: usize, n: usize) -> usize {
    j * n + i
}

/// The centroid of the lower triangle of cell `(i, j)` of an `n × n`
/// lattice and the height of the surface there: the mean of its three
/// vertex heights, which the surface interpolates linearly.
fn lower_centroid(i: usize, j: usize, n: usize, f: impl Fn(f64, f64) -> f64) -> [f64; 3] {
    let at = |k: usize| k as f64 / (n - 1) as f64;
    let corners = [(at(i), at(j)), (at(i + 1), at(j)), (at(i + 1), at(j + 1))];
    let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
    for (cx, cy) in corners {
        x += cx / 3.0;
        y += cy / 3.0;
        z += f(cx, cy) / 3.0;
    }
    [x, y, z]
}

/// A triangle mesh series over `mx`, `my`, `mtri` with the heights `mz` as
/// its values and its `z_column`: a fill when `fill`, the contours given,
/// the surface look given.
fn mesh_surface_series(
    surface: Option<SurfaceStyle>,
    fill: bool,
    contour: Option<renderer::data_config::ContourConfig>,
) -> SeriesConfig {
    SeriesConfig {
        series_id: "mesh".into(),
        source_id: None,
        label: None,
        x_column: "mx".into(),
        y_column: "my".into(),
        z_column: Some("mz".into()),
        render_type: DataRenderType::TriMesh(TriMeshSeries {
            triangles: "mtri".into(),
            values: MeshValues {
                column: "mz".into(),
                at: MeshValueLocation::Vertices,
            },
            fill: fill.then_some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            }),
            contour,
            surface,
        }),
    }
}

fn pick_request(p: [f64; 2], scale: f32) -> renderer::GpuPickRequest {
    renderer::GpuPickRequest {
        canvas_position_px: [p[0] as f32, p[1] as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: (WIDTH as f32 * scale) as u32,
            height: (HEIGHT as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: 0.0,
    }
}

/// A triangle mesh on a 3D chart draws as a surface over its triangles,
/// coloured by its values: the peak vertex lands where `project_3d` places
/// it and a low triangle paints the ramp's bottom, at 1x and 2x (the 2x
/// export is the 1x surface at twice the pixels).
#[test]
fn a_triangle_mesh_on_a_3d_chart_draws_as_a_surface() {
    let mut r = renderer();
    upload_mesh(&mut r, MESH_N, ridge);
    let config = chart_3d();
    let series = [mesh_surface_series(None, true, None)];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        save(&format!("trisurf-ridge-{scale}x"), &img);
        assert_eq!(img.width, (WIDTH as f32 * scale) as u32);
        let displayed = config.scaled(scale);
        // Vertex (20, 16) is (0.5, 0.4), the peak at height 1.
        let peak = displayed.project_3d([0.5, 0.4, 1.0]).unwrap();
        let p = pixel(&img, (peak[0], peak[1] + 3.0 * f64::from(scale)));
        assert!(is_top_color(p), "{scale}x peak at {peak:?}: {p:?}");
        // The lower triangle of the near corner cell (36, 4) is low: Viridis
        // near 0 is dark purple.
        let foot = displayed
            .project_3d(lower_centroid(36, 4, MESH_N, ridge))
            .unwrap();
        let q = pixel(&img, (foot[0], foot[1]));
        assert!(
            q[3] > 200 && q[2] > q[1] && q[0] < 120,
            "{scale}x foot at {foot:?}: {q:?}"
        );
    }
}

/// The ID pick answers the triangle whose pixel the cursor is over and its
/// vertex nearest the fragment, at 1x and 2x; a triangle hidden behind the
/// ridge never answers, and nothing off the surface does.
#[test]
fn the_id_pick_answers_the_triangle_under_the_cursor() {
    let mut r = renderer();
    upload_mesh(&mut r, MESH_N, ridge);
    let config = chart_3d();
    let series = vec![mesh_surface_series(None, true, None)];
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    for scale in [1.0f32, 2.0] {
        let displayed = config.scaled(scale);
        let pick = |r: &mut Renderer, at: [f64; 3]| {
            let p = displayed.project_3d(at).unwrap();
            test_gpu::block_on(
                r.pick_chart_data(id, pick_request([p[0], p[1]], scale))
                    .unwrap()
                    .resolve(),
            )
            .unwrap()
        };
        let triangle = |picked| match picked {
            Some(renderer::PickedData::MeshTriangle {
                series_id,
                triangle_index,
                vertex_index,
                ..
            }) => {
                assert_eq!(series_id, "mesh");
                (triangle_index, vertex_index)
            }
            other => panic!("{scale}x: picked {other:?}"),
        };
        // The lower triangle of cell (20, 3), on the near side of the peak
        // (the default camera looks from -y): the cursor at its centroid.
        let (tri, vertex) = triangle(pick(&mut r, lower_centroid(20, 3, MESH_N, ridge)));
        assert_eq!(tri, lower_triangle(20, 3, MESH_N), "{scale}x");
        assert!(
            [
                mesh_vertex(20, 3, MESH_N),
                mesh_vertex(21, 3, MESH_N),
                mesh_vertex(21, 4, MESH_N)
            ]
            .contains(&vertex),
            "{scale}x: vertex {vertex} is a corner of the triangle"
        );
        // Cell (20, 20) at y = 0.5 is behind the peak at y = 0.4: the nearer
        // surface in front of it answers, by the depth test.
        let (hidden, _) = triangle(pick(&mut r, lower_centroid(20, 20, MESH_N, ridge)));
        assert_ne!(
            hidden,
            lower_triangle(20, 20, MESH_N),
            "{scale}x: a hidden triangle never picks"
        );
        assert!(
            hidden / 2 / (MESH_N - 1) < 20,
            "{scale}x: triangle {hidden} is nearer the eye"
        );
        // Above the box: nothing.
        let above = displayed.project_3d([0.5, 0.5, 1.0]).unwrap();
        let p = [above[0], above[1] - 60.0 * f64::from(scale)];
        let miss = test_gpu::block_on(
            r.pick_chart_data(id, pick_request(p, scale))
                .unwrap()
                .resolve(),
        )
        .unwrap();
        assert_eq!(miss, None, "{scale}x");
    }
}

/// The picked triangle is outlined over the surface where it is projected,
/// at 1x and 2x, and nowhere else. A coarse mesh, so the triangle spans
/// pixels enough to count its outline.
#[test]
fn the_picked_triangle_is_highlighted() {
    const N: usize = 11;
    let mut r = renderer();
    upload_mesh(&mut r, N, ridge);
    let mut config = chart_3d();
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::MeshTriangle {
            source_id: None,
            series_id: "mesh".into(),
            triangle_index: lower_triangle(5, 1, N),
        }],
        highlight_color: Color::new(1.0, 0.0, 1.0, 1.0),
        ..DataSelectionsConfig::default()
    });
    let series = [mesh_surface_series(None, true, None)];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        save(&format!("trisurf-picked-{scale}x"), &img);
        let at = config
            .scaled(scale)
            .project_3d(lower_centroid(5, 1, N, ridge))
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
        assert!(near > 10, "{scale}x: {near} outline pixels at the triangle");
        assert_eq!(far, 0, "{scale}x: outline pixels away from the triangle");
    }
}

/// A mesh's contours on a 3D chart draw on the box floor, where its level
/// crosses the triangles, and the ID pick names the level and the triangle;
/// an opaque surface covers them and a translucent one shows them through.
#[test]
fn mesh_contours_draw_and_pick_on_the_floor() {
    let mut r = renderer();
    upload_mesh(&mut r, MESH_N, ridge);
    let config = chart_3d();
    let series = vec![mesh_surface_series(
        None,
        false,
        Some(contour_config(vec![0.5])),
    )];
    let img = export(&mut r, &config, &series, 1.0);
    save("trisurf-floor-contour", &img);
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
    let centre = config.project_3d([0.5, 0.4, 0.0]).unwrap();
    let c = pixel(&img, (centre[0], centre[1]));
    assert!(
        !(c[0] > 150 && c[1] < 100),
        "no stroke at the centre: {c:?}"
    );

    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    let on = config.project_3d([0.5 + radius, 0.4, 0.0]).unwrap();
    match test_gpu::block_on(
        r.pick_chart_data(id, pick_request([on[0], on[1]], 1.0))
            .unwrap()
            .resolve(),
    )
    .unwrap()
    {
        Some(renderer::PickedData::ContourLevel {
            series_id,
            level_index,
            x_index,
            y_index,
            ..
        }) => {
            assert_eq!(series_id, "mesh");
            assert_eq!(level_index, 0);
            // The triangle the stroke lies in: in the column of cells the
            // point (0.5 + r, 0.4) falls in.
            let cell_x = ((0.5 + radius) * (MESH_N - 1) as f64).floor() as usize;
            let triangle_cell_x = x_index / 2 % (MESH_N - 1);
            assert!(
                triangle_cell_x.abs_diff(cell_x) <= 1,
                "triangle {x_index} lies in cell column {triangle_cell_x}, the point in {cell_x}"
            );
            assert_eq!(y_index, 0);
        }
        other => panic!("picked {other:?}"),
    }

    // With a fill: the opaque surface covers the floor, a translucent one
    // shows it through.
    let filled = |opacity: f32| {
        let mut series = mesh_surface_series(None, true, Some(contour_config(vec![0.5])));
        if let DataRenderType::TriMesh(mesh) = &mut series.render_type {
            mesh.fill.as_mut().unwrap().opacity = opacity;
        }
        vec![series]
    };
    let reds = |img: &RasterImage| {
        img.rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 120 && p[0] > 150 && p[1] < 90 && p[2] < 90)
            .count()
    };
    let opaque = export(&mut r, &config, &filled(1.0), 1.0);
    let clear = export(&mut r, &config, &filled(0.3), 1.0);
    save("trisurf-floor-opaque", &opaque);
    save("trisurf-floor-translucent", &clear);
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

/// A lit material shades the mesh surface with its per-triangle normals,
/// and a wireframe draws its edges: each changes the image.
#[test]
fn a_lit_wireframed_mesh_surface_shades_and_outlines_its_triangles() {
    let mut r = renderer();
    upload_mesh(&mut r, MESH_N, ridge);
    let config = chart_3d();
    let flat = export(
        &mut r,
        &config,
        &[mesh_surface_series(None, true, None)],
        1.0,
    );
    let lit = export(
        &mut r,
        &config,
        &[mesh_surface_series(
            Some(SurfaceStyle {
                material: renderer::Material::Matte,
                ..SurfaceStyle::default()
            }),
            true,
            None,
        )],
        1.0,
    );
    save("trisurf-lit", &lit);
    let changed = flat
        .rgba
        .chunks_exact(4)
        .zip(lit.rgba.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 12))
        .count();
    assert!(changed > 1000, "lighting changed {changed} pixels");
    let wired = export(
        &mut r,
        &config,
        &[mesh_surface_series(
            Some(SurfaceStyle {
                wireframe: Some(SurfaceWireframe {
                    color: Color::new(0.0, 0.0, 0.0, 1.0),
                    width: 1.0,
                }),
                ..SurfaceStyle::default()
            }),
            true,
            None,
        )],
        1.0,
    );
    save("trisurf-wireframe", &wired);
    let dark = |img: &RasterImage| {
        img.rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 200 && p[0] < 40 && p[1] < 40 && p[2] < 40)
            .count()
    };
    assert!(
        dark(&wired) > dark(&flat) + 2000,
        "the wireframe adds dark edge pixels: {} over {}",
        dark(&wired),
        dark(&flat)
    );
}

/// The z fit reads the height column's statistics: `right_y` spans the
/// mesh's heights.
#[test]
fn the_z_fit_spans_the_mesh_heights() {
    let mut r = renderer();
    upload_mesh(&mut r, MESH_N, |x, y| 3.0 + 4.0 * ridge(x, y));
    let mut chart = Chart::new(chart_3d());
    let series = [mesh_surface_series(None, true, None)];
    chart
        .auto_fit_view_3d_z(r.pool(), renderer::LayerRef::Base, &series, 0.0)
        .unwrap();
    let z = &chart.config().right_y;
    let at = |k: usize| k as f64 / (MESH_N - 1) as f64;
    let (min, max) = (0..MESH_N)
        .flat_map(|i| (0..MESH_N).map(move |j| 3.0 + 4.0 * ridge(at(i), at(j))))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
            (a.min(v), b.max(v))
        });
    assert!(
        (z.min - min).abs() < 1e-9 && (z.max - max).abs() < 1e-9,
        "{} {}",
        z.min,
        z.max
    );
}

/// A new upload of the height column redraws the surface where the new
/// heights are (SSOT: the mesh's group follows its columns' epochs).
#[test]
fn a_height_upload_redraws_the_mesh_surface() {
    let mut r = renderer();
    upload_mesh(&mut r, MESH_N, ridge);
    let config = chart_3d();
    let series = [mesh_surface_series(None, true, None)];
    let before = export(&mut r, &config, &series, 1.0);
    let peak = config.project_3d([0.5, 0.4, 1.0]).unwrap();
    assert!(is_top_color(pixel(&before, (peak[0], peak[1] + 3.0))));
    // Halve the heights (and so the values): the peak is at 0.5 now, and
    // the old peak pixel shows nothing of the surface.
    let at = |k: usize| k as f64 / (MESH_N - 1) as f64;
    let halved: Vec<f64> = (0..MESH_N)
        .flat_map(|j| (0..MESH_N).map(move |i| 0.5 * ridge(at(i), at(j))))
        .collect();
    r.upsert_column("mz", &col(halved)).unwrap();
    let after = export(&mut r, &config, &series, 1.0);
    save("trisurf-halved", &after);
    let p = pixel(&after, (peak[0], peak[1] + 3.0));
    assert!(!is_top_color(p), "the old peak is gone: {p:?}");
    let lower = config.project_3d([0.5, 0.4, 0.5]).unwrap();
    let q = pixel(&after, (lower[0], lower[1] + 3.0));
    // Viridis at 0.5 is teal.
    assert!(
        q[3] > 200 && q[1] > 110 && q[2] > 110 && q[0] < 80,
        "the halved peak at {lower:?}: {q:?}"
    );
}

/// A mesh in a 3D subplot draws as a surface beside the flat panel's mesh,
/// each where its own panel places it, at 1x and 2x.
#[test]
fn a_3d_subplot_draws_a_mesh_surface_beside_a_flat_panel() {
    let mut r = renderer();
    upload_mesh(&mut r, MESH_N, ridge);
    let mut flat = chart_3d();
    flat.set_view_3d(None);
    let mut subplot = chart_3d();
    subplot.chart_area = ChartArea(Rect {
        x: WIDTH,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    let flat_mesh = mesh_surface_series(None, true, None);
    let mut mesh_3d = flat_mesh.clone();
    mesh_3d.series_id = "mesh_3d".into();
    let mut parent = Chart::new(flat.clone());
    parent.config_mut().subplots.push(renderer::Subplot {
        config: subplot.clone(),
        series: vec!["mesh_3d".into()],
        link_x: None,
        link_y: None,
        marker: None,
        text_scale: Default::default(),
    });
    let series = [flat_mesh, mesh_3d];
    for scale in [1.0f32, 2.0] {
        let document = r
            .export_rgba(
                renderer::ExportSource::chart(&parent, &series),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        save(&format!("trisurf-subplot-{scale}x"), &document);
        let s = f64::from(scale);
        assert_eq!(
            (document.width, document.height),
            (
                (2.0 * f64::from(WIDTH) * s) as u32,
                (f64::from(HEIGHT) * s) as u32
            )
        );
        // The 3D panel: the peak where its camera puts it, offset by its
        // area.
        let peak = subplot.scaled(scale).project_3d([0.5, 0.4, 1.0]).unwrap();
        let p = pixel(&document, (peak[0], peak[1] + 3.0 * s));
        assert!(is_top_color(p), "{scale}x: 3D peak at {peak:?}: {p:?}");
        // The flat panel: the mesh's fill at the peak's data position.
        let da = parent.config().data_area().expect("data area");
        let x = (f64::from(da.x) + 0.5 * f64::from(da.width)) * s;
        let y = (f64::from(da.y + da.height) - 0.4 * f64::from(da.height)) * s;
        let q = pixel(&document, (x, y));
        assert!(is_top_color(q), "{scale}x: flat peak at ({x}, {y}): {q:?}");
    }
}
