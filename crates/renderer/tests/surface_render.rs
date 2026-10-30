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
