//! End-to-end pixel verification of the triangle mesh draw (#210), through
//! the public renderer API and the headless `export_rgba`.
//!
//! The colourmap is an explicit red → green ramp, so every fill assertion
//! names the colour `ColorBarOptions::color_for_z` computes on the CPU for
//! the value the host's linear field has at that point: the mesh shades the
//! barycentric blend of the given vertex values, which for a field linear
//! over the whole square is that field itself.

use test_gpu::BlockOn as _;
#[path = "support/edge_band.rs"]
mod edge_band;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::config::AxisScale;
use renderer::data::Column;
use renderer::data_config::{
    ContourConfig, DataLineStyleConfig, FieldFillConfig, FillMode, LineInterpolation, Shading,
};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::tri_mesh::{MeshValueLocation, MeshValues, TriMeshSeries};
use renderer::{
    Chart, Color, ColorBarOptions, ColorMap, DataRenderType, FiggyError, RasterImage, Renderer,
    RendererDevice, SeriesConfig, TriangleSource, encode_png,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;

const LOW: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const HIGH: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
const NAN_PAINT: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

fn shared_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    static DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let (device, queue) = test_gpu::device();
            Some((device, queue))
        })
        .as_ref()
        .map(|(device, queue)| (device.clone(), queue.clone()))
}

fn try_renderer() -> Option<Renderer> {
    let (device, queue) = shared_device()?;
    Some(
        Renderer::try_new(
            RendererDevice::new(device, queue).expect("figgy poll thread"),
            wgpu::TextureFormat::Bgra8Unorm,
            64 * 1024 * 1024,
        )
        .expect("renderer init"),
    )
}

fn colorbar(z_min: f64, z_max: f64) -> ColorBarOptions {
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Custom {
        stops: vec![LOW, HIGH],
    };
    bar.nan_color = NAN_PAINT;
    bar.axis.min = z_min;
    bar.axis.max = z_max;
    bar.axis.major_spacing = (z_max - z_min) / 5.0;
    bar
}

/// Grid and legend off over `0..2 × 0..2`: the only non-mesh ink is the
/// black deco frame.
fn chart(bar: Option<ColorBarOptions>) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    config.colorbar = bar;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 2.0);
    chart.set_y_range(0.0, 2.0);
    chart
}

fn line(color: Color, width: f32) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: LineStylePreset::Solid,
        line_color: color,
        line_width: width,
        fill: None,
        fill_color: None,
        interpolation: LineInterpolation::Linear,
    }
}

fn fill(shading: Shading, mode: FillMode) -> FieldFillConfig {
    FieldFillConfig {
        mode,
        shading,
        opacity: 1.0,
    }
}

fn mesh_series(
    at: MeshValueLocation,
    fill: Option<FieldFillConfig>,
    contour: Option<ContourConfig>,
) -> SeriesConfig {
    SeriesConfig {
        series_id: "mesh".into(),
        source_id: None,
        label: None,
        x_column: "mx".into(),
        y_column: "my".into(),
        z_column: None,
        render_type: DataRenderType::TriMesh(TriMeshSeries {
            triangles: "tris".into(),
            values: MeshValues {
                column: "mv".into(),
                at,
            },
            fill,
            contour,
            surface: None,
        }),
    }
}

/// The square `0..2 × 0..2` as two triangles meeting on the diagonal from
/// `(0, 0)` to `(2, 2)`: triangle 0 below it, triangle 1 above.
const SQUARE_X: [f64; 4] = [0.0, 2.0, 2.0, 0.0];
const SQUARE_Y: [f64; 4] = [0.0, 0.0, 2.0, 2.0];
const SQUARE_TRIANGLES: [u32; 6] = [0, 1, 2, 0, 2, 3];

/// The host's field `x + 2y`, linear over the whole square, so the
/// barycentric blend of its vertex values is the field itself.
fn field(x: f64, y: f64) -> f64 {
    x + 2.0 * y
}

fn square(renderer: &mut Renderer, xs: [f64; 4], values: Vec<f64>) {
    renderer
        .upsert_column("mx", &Column { data: xs.to_vec() })
        .unwrap();
    renderer
        .upsert_column(
            "my",
            &Column {
                data: SQUARE_Y.to_vec(),
            },
        )
        .unwrap();
    renderer
        .upsert_column("mv", &Column { data: values })
        .unwrap();
    renderer
        .upsert_triangles(
            "tris",
            &TriangleSource {
                indices: &Column {
                    data: SQUARE_TRIANGLES.to_vec(),
                },
            },
        )
        .unwrap();
}

fn vertex_field() -> Vec<f64> {
    (0..4).map(|i| field(SQUARE_X[i], SQUARE_Y[i])).collect()
}

fn export(
    renderer: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    scale: f32,
) -> RasterImage {
    renderer
        .export_rgba(
            renderer::ExportSource::chart(chart, series),
            renderer::ExportOptions::panel(scale),
        )
        .block_on()
        .unwrap()
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> [i32; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    let p = &img.rgba[i..i + 4];
    [p[0] as i32, p[1] as i32, p[2] as i32, p[3] as i32]
}

/// Data `(x, y)` → pixel at `scale`, through the data area the renderer
/// draws with.
fn data_to_px(chart: &Chart, x: f64, y: f64, scale: f64) -> (u32, u32) {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let tx = (x - cfg.bottom_x.min) / (cfg.bottom_x.max - cfg.bottom_x.min);
    let ty = (y - cfg.left_y.min) / (cfg.left_y.max - cfg.left_y.min);
    (
        ((da.x as f64 + tx * da.width as f64) * scale) as u32,
        (((da.y + da.height) as f64 - ty * da.height as f64) * scale) as u32,
    )
}

fn to8(v: f32) -> i32 {
    (v.clamp(0.0, 1.0) * 255.0).round() as i32
}

fn assert_colour(img: &RasterImage, px: (u32, u32), want: Color, tol: i32, what: &str) {
    let got = pixel(img, px.0, px.1);
    let expected = [to8(want.r), to8(want.g), to8(want.b), to8(want.a)];
    assert!(
        got.iter().zip(expected).all(|(g, e)| (*g - e).abs() <= tol),
        "{what} at px {px:?}: got {got:?}, expected {expected:?} (tol {tol})"
    );
}

fn probe_dir() -> String {
    let dir = std::env::var("FIGGY_PROBE_DIR")
        .unwrap_or_else(|_| env!("CARGO_TARGET_TMPDIR").to_string());
    std::fs::create_dir_all(&dir).expect("create probe directory");
    dir
}

fn save_probe(name: &str, img: &RasterImage) {
    let png = encode_png(img).expect("encode png");
    std::fs::write(format!("{}/probe_tri_mesh_{name}.png", probe_dir()), png).expect("write png");
}

/// Interior points of both triangles paint the colourbar colour of the
/// field's value there, at 1x and at 2x.
#[test]
fn vertex_values_shade_the_barycentric_blend_at_1x_and_2x() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let series = [mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut renderer, &chart, &series, scale);
        save_probe(&format!("vertices-{scale}x"), &img);
        for (x, y) in [(1.5, 0.3), (1.7, 1.2), (0.3, 1.5), (0.8, 1.7), (1.0, 0.5)] {
            let px = data_to_px(&chart, x, y, f64::from(scale));
            assert_colour(
                &img,
                px,
                bar.color_for_z(field(x, y)),
                3,
                &format!("{scale}x ({x}, {y})"),
            );
        }
    }
}

/// Face values paint each triangle one flat colour.
#[test]
fn face_values_paint_each_triangle_flat() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vec![1.0, 5.0]);
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let series = [mesh_series(
        MeshValueLocation::Faces,
        Some(fill(Shading::Flat, FillMode::Continuous)),
        None,
    )];
    let img = export(&mut renderer, &chart, &series, 1.0);
    save_probe("faces", &img);
    for (x, y) in [(1.5, 0.3), (1.8, 1.5), (1.0, 0.2)] {
        assert_colour(
            &img,
            data_to_px(&chart, x, y, 1.0),
            bar.color_for_z(1.0),
            2,
            "triangle 0",
        );
    }
    for (x, y) in [(0.3, 1.5), (0.2, 1.8), (1.0, 1.8)] {
        assert_colour(
            &img,
            data_to_px(&chart, x, y, 1.0),
            bar.color_for_z(5.0),
            2,
            "triangle 1",
        );
    }
}

/// A banded fill paints the band colour of the levels the value reaches.
#[test]
fn bands_follow_the_levels() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let levels = vec![2.0, 4.0];
    let series = [mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Bands)),
        Some(ContourConfig {
            levels: levels.clone(),
            line: line(Color::new(0.0, 0.0, 0.0, 0.0), 0.0),
            per_level_color: None,
            labels: None,
        }),
    )];
    let img = export(&mut renderer, &chart, &series, 1.0);
    save_probe("bands", &img);
    // Three bands, each the ramp at its own midpoint: (k + 0.5) / 3.
    let band = |k: f64| bar.color_for_z(6.0 * (k + 0.5) / 3.0);
    assert_colour(
        &img,
        data_to_px(&chart, 0.5, 0.25, 1.0),
        band(0.0),
        2,
        "below 2",
    );
    assert_colour(
        &img,
        data_to_px(&chart, 1.5, 0.75, 1.0),
        band(1.0),
        2,
        "between 2 and 4",
    );
    assert_colour(
        &img,
        data_to_px(&chart, 1.5, 1.75, 1.0),
        band(2.0),
        2,
        "above 4",
    );
}

/// A non-finite vertex value paints its triangles in `nan_color`; the other
/// triangle is shaded as usual.
#[test]
fn a_nan_vertex_value_paints_the_nan_colour() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let mut values = vertex_field();
    values[3] = f64::NAN;
    square(&mut renderer, SQUARE_X, values);
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let series = [mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    let img = export(&mut renderer, &chart, &series, 1.0);
    assert_colour(
        &img,
        data_to_px(&chart, 0.4, 1.5, 1.0),
        NAN_PAINT,
        1,
        "triangle 1",
    );
    assert_colour(
        &img,
        data_to_px(&chart, 1.5, 0.4, 1.0),
        bar.color_for_z(field(1.5, 0.4)),
        3,
        "triangle 0",
    );
}

/// A vertex the axes cannot place drops only the triangles that name it,
/// as a non-finite sample breaks a line.
#[test]
fn a_nan_coordinate_drops_only_its_triangles() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let mut xs = SQUARE_X;
    xs[3] = f64::NAN;
    square(&mut renderer, xs, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let series = [mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    let img = export(&mut renderer, &chart, &series, 1.0);
    let dropped = pixel(
        &img,
        data_to_px(&chart, 0.4, 1.5, 1.0).0,
        data_to_px(&chart, 0.4, 1.5, 1.0).1,
    );
    assert_eq!(dropped[3], 0, "triangle 1 is not drawn: {dropped:?}");
    assert_colour(
        &img,
        data_to_px(&chart, 1.5, 0.4, 1.0),
        bar.color_for_z(field(1.5, 0.4)),
        3,
        "triangle 0",
    );
}

/// The two triangles share the diagonal: every pixel along it is fully
/// painted (no background through the seam under MSAA), in the field's
/// colour.
#[test]
fn an_interior_shared_edge_shows_no_background() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vec![3.0; 4]);
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let series = [mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut renderer, &chart, &series, scale);
        let want = bar.color_for_z(3.0);
        for k in 1..40 {
            let t = 0.05 + 1.9 * f64::from(k) / 40.0;
            let px = data_to_px(&chart, t, t, f64::from(scale));
            assert_colour(&img, px, want, 1, &format!("{scale}x diagonal at {t}"));
        }
    }
}

/// A contour of the linear field `x + 2y` at level 2 is the straight
/// segment from `(2, 0)` to `(0, 1)`: its stroke covers the analytic
/// crossing points and leaves the fill a few pixels away, at 1x and 2x.
#[test]
fn a_contour_stroke_passes_through_the_analytic_crossings() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let black = Color::new(0.0, 0.0, 0.0, 1.0);
    let series = [mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        Some(ContourConfig {
            levels: vec![2.0],
            line: line(black, 3.0),
            per_level_color: None,
            labels: None,
        }),
    )];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut renderer, &chart, &series, scale);
        save_probe(&format!("contour-{scale}x"), &img);
        // Crossings in both triangles (the segment crosses the diagonal at
        // (2/3, 2/3)).
        for x in [0.3, 0.6, 1.0, 1.4] {
            let y = (2.0 - x) / 2.0;
            let px = data_to_px(&chart, x, y, f64::from(scale));
            assert_colour(&img, px, black, 40, &format!("{scale}x stroke at x = {x}"));
        }
        // Eight output pixels up the y axis is off the stroke.
        let (px, py) = data_to_px(&chart, 1.4, 0.3, f64::from(scale));
        let off = (px, py - (8.0 * scale) as u32);
        let got = pixel(&img, off.0, off.1);
        assert!(got[1] > 40, "{scale}x fill beside the stroke: {got:?}");
    }
}

/// A triangle whose three values are equal has no gradient: a level at that
/// value draws nothing over it (a plateau), as the heatmap's.
#[test]
fn a_plateau_draws_no_contour() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vec![3.0; 4]);
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let series = [mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        Some(ContourConfig {
            levels: vec![3.0],
            line: line(Color::new(0.0, 0.0, 0.0, 1.0), 3.0),
            per_level_color: None,
            labels: None,
        }),
    )];
    let img = export(&mut renderer, &chart, &series, 1.0);
    for (x, y) in [(1.5, 0.4), (0.4, 1.5), (1.0, 1.0)] {
        assert_colour(
            &img,
            data_to_px(&chart, x, y, 1.0),
            bar.color_for_z(3.0),
            1,
            "plateau",
        );
    }
}

/// The renderer's registration rules: an index past the vertex count, a
/// face value column shorter than the triangles, a missing colourbar, a
/// triangle id that is not a triangle column and a 3D chart are refused.
#[test]
fn registration_refuses_what_the_mesh_cannot_draw() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let interpolated = Some(fill(Shading::Interpolated, FillMode::Continuous));
    let reason = |result: renderer::Result<renderer::ChartId>| match result {
        Err(FiggyError::InvalidSeriesConfig { reason, .. }) => reason,
        other => panic!("expected InvalidSeriesConfig, got {other:?}"),
    };
    // Fine as given.
    renderer
        .register_chart(
            chart(Some(bar.clone())).config().clone(),
            vec![mesh_series(
                MeshValueLocation::Vertices,
                interpolated.clone(),
                None,
            )],
        )
        .unwrap();
    // A shorter value column leaves vertex 3 without a value.
    renderer
        .upsert_column(
            "mv",
            &Column {
                data: vec![0.0, 1.0, 2.0],
            },
        )
        .unwrap();
    let why = reason(renderer.register_chart(
        chart(Some(bar.clone())).config().clone(),
        vec![mesh_series(
            MeshValueLocation::Vertices,
            interpolated.clone(),
            None,
        )],
    ));
    assert!(
        why.contains("triangle index 3") && why.contains("\"mv\""),
        "{why}"
    );
    // One face value for two triangles.
    renderer
        .upsert_column("mv", &Column { data: vec![1.0] })
        .unwrap();
    let why = reason(renderer.register_chart(
        chart(Some(bar.clone())).config().clone(),
        vec![mesh_series(
            MeshValueLocation::Faces,
            Some(fill(Shading::Flat, FillMode::Continuous)),
            None,
        )],
    ));
    assert!(why.contains("1 values for 2 triangles"), "{why}");
    renderer
        .upsert_column(
            "mv",
            &Column {
                data: vertex_field(),
            },
        )
        .unwrap();
    let why = reason(renderer.register_chart(
        chart(None).config().clone(),
        vec![mesh_series(
            MeshValueLocation::Vertices,
            interpolated.clone(),
            None,
        )],
    ));
    assert!(why.contains("Config.colorbar"), "{why}");
    let mut plain = mesh_series(MeshValueLocation::Vertices, interpolated.clone(), None);
    if let DataRenderType::TriMesh(mesh) = &mut plain.render_type {
        mesh.triangles = "mx".into();
    }
    let why =
        reason(renderer.register_chart(chart(Some(bar.clone())).config().clone(), vec![plain]));
    assert!(
        why.contains("not a resident triangle index column"),
        "{why}"
    );
    let mut three_d = chart(Some(bar.clone())).config().clone();
    three_d.set_view_3d(Some(renderer::view3d::View3d::default()));
    let mut lifted = mesh_series(MeshValueLocation::Vertices, interpolated, None);
    lifted.z_column = Some("mv".into());
    let why = reason(renderer.register_chart(three_d, vec![lifted]));
    assert!(why.contains("3D"), "{why}");
}

/// A later upload that shortens a vertex column is not re-checked: the draw
/// drops the triangles that name a vertex past the end, and the draw info
/// reports the truncation.
#[test]
fn a_shortened_vertex_column_truncates_and_reports() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    let id = renderer
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let info = renderer.series_draw_info(id, "mesh").unwrap();
    assert_eq!(info.drawn_count, 2);
    assert!(!info.truncated);
    renderer
        .upsert_column(
            "mx",
            &Column {
                data: vec![0.0, 2.0, 2.0],
            },
        )
        .unwrap();
    let info = renderer.series_draw_info(id, "mesh").unwrap();
    assert!(info.truncated, "vertex 3 has no x any more");
    let img = export(&mut renderer, &chart, &series, 1.0);
    let (x, y) = data_to_px(&chart, 0.4, 1.5, 1.0);
    assert_eq!(pixel(&img, x, y)[3], 0, "triangle 1 is dropped");
    assert_colour(
        &img,
        data_to_px(&chart, 1.5, 0.4, 1.0),
        bar.color_for_z(field(1.5, 0.4)),
        3,
        "triangle 0 still draws",
    );
}

/// On logarithmic axes the vertices sit at their logarithmic places and the
/// blend runs in screen space, as the heatmap's interpolation does: a
/// triangle of equal values paints one colour wherever it lands.
#[test]
fn log_axes_place_the_vertices() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .upsert_column(
            "mx",
            &Column {
                data: vec![1.0, 100.0, 100.0, 1.0],
            },
        )
        .unwrap();
    renderer
        .upsert_column(
            "my",
            &Column {
                data: vec![1.0, 1.0, 100.0, 100.0],
            },
        )
        .unwrap();
    renderer
        .upsert_column(
            "mv",
            &Column {
                data: vec![1.0, 1.0, 1.0, 5.0],
            },
        )
        .unwrap();
    renderer
        .upsert_triangles(
            "tris",
            &TriangleSource {
                indices: &Column {
                    data: SQUARE_TRIANGLES.to_vec(),
                },
            },
        )
        .unwrap();
    let bar = colorbar(0.0, 6.0);
    let mut chart = chart(Some(bar.clone()));
    chart.config_mut().bottom_x.scale = AxisScale::Logarithmic;
    chart.config_mut().left_y.scale = AxisScale::Logarithmic;
    chart.set_x_range(1.0, 100.0);
    chart.set_y_range(1.0, 100.0);
    let series = [mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    let img = export(&mut renderer, &chart, &series, 1.0);
    save_probe("log", &img);
    let da = chart.config().data_area().unwrap();
    // Triangle 0 (all values 1) fills the lower-right half of the data
    // area in screen space: a point a quarter in from the bottom right.
    let px = (da.x + da.width * 3 / 4, da.y + da.height * 3 / 4);
    assert_colour(&img, px, bar.color_for_z(1.0), 2, "lower-right half");
    // Triangle 1 at its screen-space centroid: the blend of 1, 1 and 5 in
    // equal parts.
    let centroid = (da.x + da.width / 3, da.y + da.height / 3);
    assert_colour(
        &img,
        centroid,
        bar.color_for_z(7.0 / 3.0),
        3,
        "upper-left centroid",
    );
}

/// Rule 10: a mesh of a million triangles registers, draws and paints its
/// last triangle, and the pick answers that triangle; nothing caps the
/// count. The times are printed for the host's budget.
#[test]
fn a_million_triangles_draw_to_the_last() {
    let Some(mut renderer) = try_picking_renderer() else {
        return;
    };
    // 708 × 708 vertices: 707² × 2 = 999,698 triangles … one more row of
    // quads to pass a million.
    let (nx, ny) = (708u32, 709u32);
    let vertex = |i: u32, j: u32| j * nx + i;
    let mut xs = Vec::with_capacity((nx * ny) as usize);
    let mut ys = Vec::with_capacity((nx * ny) as usize);
    let mut vs = Vec::with_capacity((nx * ny) as usize);
    for j in 0..ny {
        for i in 0..nx {
            let x = 2.0 * f64::from(i) / f64::from(nx - 1);
            let y = 2.0 * f64::from(j) / f64::from(ny - 1);
            xs.push(x);
            ys.push(y);
            vs.push(field(x, y));
        }
    }
    let mut indices = Vec::with_capacity(((nx - 1) * (ny - 1) * 6) as usize);
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            let (a, b, c, d) = (
                vertex(i, j),
                vertex(i + 1, j),
                vertex(i + 1, j + 1),
                vertex(i, j + 1),
            );
            indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    let triangles = indices.len() / 3;
    assert!(triangles > 1_000_000, "{triangles}");
    renderer.upsert_column("mx", &Column { data: xs }).unwrap();
    renderer.upsert_column("my", &Column { data: ys }).unwrap();
    renderer.upsert_column("mv", &Column { data: vs }).unwrap();
    renderer
        .upsert_triangles(
            "tris",
            &TriangleSource {
                indices: &Column { data: indices },
            },
        )
        .unwrap();
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    let id = renderer
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    assert_eq!(
        renderer.series_draw_info(id, "mesh").unwrap().drawn_count,
        triangles as u64
    );
    let start = std::time::Instant::now();
    let img = export(&mut renderer, &chart, &series, 1.0);
    println!(
        "{triangles} triangles: export at 1x in {:.1} ms",
        start.elapsed().as_secs_f64() * 1e3
    );
    // The last triangle is the top-left one of the last quad, at the top
    // right corner of the square.
    let (x, y) = data_to_px(&chart, 1.999, 1.999, 1.0);
    assert_colour(
        &img,
        (x - 1, y + 1),
        bar.color_for_z(field(1.998, 1.998)),
        3,
        "last triangle",
    );
    renderer.remove_chart(id).unwrap();

    // Zoomed onto the last quad, the pick at the centroid of its upper-left
    // triangle answers the last triangle; the pick walks all of them.
    let mut zoomed = chart;
    let (x0, y0) = (2.0 * 706.0 / 707.0, 2.0 * 707.0 / 708.0);
    zoomed.set_x_range(x0 - 0.001, 2.001);
    zoomed.set_y_range(y0 - 0.001, 2.001);
    let id = renderer
        .register_chart(zoomed.config().clone(), series)
        .unwrap();
    let centroid = ((x0 + 2.0 + x0) / 3.0, (y0 + 2.0 + 2.0) / 3.0);
    let start = std::time::Instant::now();
    let hit = pick_data_point(&mut renderer, id, &zoomed, centroid);
    println!(
        "{triangles} triangles: pick in {:.1} ms",
        start.elapsed().as_secs_f64() * 1e3
    );
    match hit {
        Some(renderer::PickedData::MeshTriangle { triangle_index, .. }) => {
            assert_eq!(triangle_index, triangles - 1)
        }
        other => panic!("expected the last triangle, got {other:?}"),
    }
    renderer.remove_chart(id).unwrap();
}

/// Image pixel of data `(x, y)` where the axis chrome places it, through
/// `axis_fraction` (cuts and scales included) and the scaled layout.
fn chrome_px(config: &renderer::Config, x: f64, y: f64, scale: f32) -> (u32, u32) {
    let da = config.scaled(scale).data_area().unwrap().0;
    let fx = renderer::layout::axis_fraction(x, &config.bottom_x);
    let fy = renderer::layout::axis_fraction(y, &config.left_y);
    (
        (da.x as f64 + da.width as f64 * f64::from(fx)) as u32,
        (da.y as f64 + da.height as f64 * (1.0 - f64::from(fy))) as u32,
    )
}

/// On a broken x axis (#200), both styles: every piece shades the field
/// where the chrome places it, and nothing is drawn in the gap. The mesh's
/// vertices sit on the cut's edges, so each piece holds whole triangles.
#[test]
fn a_broken_axis_shades_each_piece_and_leaves_the_gap_empty() {
    use renderer::config::{AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark};
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let xs = [0.0, 0.8, 1.2, 2.0];
    let mut mx = Vec::new();
    let mut my = Vec::new();
    let mut mv = Vec::new();
    for y in [0.0, 2.0] {
        for x in xs {
            mx.push(x);
            my.push(y);
            mv.push(field(x, y));
        }
    }
    let mut indices = Vec::new();
    for i in 0..3u32 {
        let (a, b, c, d) = (i, i + 1, i + 5, i + 4);
        indices.extend_from_slice(&[a, b, c, a, c, d]);
    }
    renderer.upsert_column("mx", &Column { data: mx }).unwrap();
    renderer.upsert_column("my", &Column { data: my }).unwrap();
    renderer.upsert_column("mv", &Column { data: mv }).unwrap();
    renderer
        .upsert_triangles(
            "tris",
            &TriangleSource {
                indices: &Column { data: indices },
            },
        )
        .unwrap();
    let bar = colorbar(0.0, 6.0);
    let series = [mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    for style in [AxisBreakStyle::Continuous, AxisBreakStyle::Panels] {
        let mut chart = chart(Some(bar.clone()));
        chart.config_mut().bottom_x.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(0.8, 1.2)],
            style,
            gap: 0.1,
            mark: BreakMark::None,
            frame_each_panel: false,
        });
        let config = chart.config().clone();
        for scale in [1.0f32, 2.0] {
            let img = export(&mut renderer, &chart, &series, scale);
            save_probe(&format!("break-{style:?}-{scale}x"), &img);
            for (x, y) in [(0.3, 0.5), (0.6, 1.6), (1.5, 0.4), (1.8, 1.7)] {
                assert_colour(
                    &img,
                    chrome_px(&config, x, y, scale),
                    bar.color_for_z(field(x, y)),
                    4,
                    &format!("{style:?} {scale}x ({x}, {y})"),
                );
            }
            // The middle of the gap: no fill.
            let gap_left = chrome_px(&config, 0.8, 1.0, scale).0;
            let gap_right = chrome_px(&config, 1.2, 1.0, scale).0;
            let middle = (gap_left + gap_right) / 2;
            let row = chrome_px(&config, 1.0, 1.0, scale).1;
            let got = pixel(&img, middle, row);
            assert!(
                got[0] + got[1] < 80 || got[3] < 40,
                "{style:?} {scale}x: the gap at column {middle} is painted: {got:?}"
            );
        }
    }
}

/// Rule 10: three hundred mesh series draw, past the field scratch cache's
/// 256 entries (an eviction bound, not a cap): every series paints.
#[test]
fn three_hundred_mesh_series_all_draw() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vec![3.0; 4]);
    let bar = colorbar(0.0, 6.0);
    let chart = chart(Some(bar.clone()));
    let series: Vec<SeriesConfig> = (0..300)
        .map(|k| {
            let mut s = mesh_series(
                MeshValueLocation::Vertices,
                Some(fill(Shading::Interpolated, FillMode::Continuous)),
                None,
            );
            s.series_id = format!("mesh{k}");
            s
        })
        .collect();
    let id = renderer
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    for k in [0, 255, 256, 299] {
        assert_eq!(
            renderer
                .series_draw_info(id, &format!("mesh{k}"))
                .unwrap()
                .drawn_count,
            2
        );
    }
    let img = export(&mut renderer, &chart, &series, 1.0);
    assert_colour(
        &img,
        data_to_px(&chart, 1.0, 1.3, 1.0),
        bar.color_for_z(3.0),
        1,
        "300 series",
    );
}

/// Streaming is not extended to triangle meshes (rule 8): registration
/// admits streamed vertex columns, and the streaming inspector refuses the
/// mesh with its explicit reason.
#[test]
fn a_streamed_vertex_column_is_refused() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let streamed = |id: &str| renderer::StreamColumn {
        id: id.into(),
        len: 4,
        revision: 1,
        encoding: renderer::StreamEncoding::ScalarF32,
        replay: renderer::StreamReplay::RandomAccess,
        statistics: renderer::StreamStatistics::Unknown,
    };
    renderer
        .register_streamed_columns(vec![streamed("st_x"), streamed("st_y")])
        .unwrap();
    let mut series = mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    );
    series.x_column = "st_x".into();
    series.y_column = "st_y".into();
    let id = renderer
        .register_chart(
            chart(Some(colorbar(0.0, 6.0))).config().clone(),
            vec![series],
        )
        .expect("registration admits the columns; the streaming inspector names the rule");
    match renderer.inspect_streaming_capability(id) {
        Err(FiggyError::InvalidSeriesConfig { series_id, reason }) => {
            assert_eq!(series_id, "mesh");
            assert_eq!(
                reason,
                "triangle mesh series require GPU-resident columns; streamed sources are not \
                 supported"
            );
        }
        other => panic!("expected the mesh refusal, got {:?}", other.map(|_| ())),
    }
}

// ───── Picking and the picked highlight (#210 PR 4) ─────────────────────────

/// The chart over `-1..3 × -1..3`, so the square leaves clear space round it.
fn wide_chart(bar: Option<ColorBarOptions>) -> Chart {
    let mut chart = chart(bar);
    chart.set_x_range(-1.0, 3.0);
    chart.set_y_range(-1.0, 3.0);
    chart
}

/// A renderer with the GPU picker enabled.
fn try_picking_renderer() -> Option<Renderer> {
    let mut renderer = try_renderer()?;
    renderer.enable_gpu_picking().expect("GPU picking");
    Some(renderer)
}

fn pick_request(x: f32, y: f32, scale: f32, max: f32) -> renderer::GpuPickRequest {
    renderer::GpuPickRequest {
        canvas_position_px: [x, y],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: (WIDTH as f32 * scale).round() as u32,
            height: (HEIGHT as f32 * scale).round() as u32,
        },
        display_scale: scale,
        max_distance_px: max,
    }
}

/// The typed data pick at canvas position `(x, y)` of chart `id` displayed
/// at `scale`.
fn pick_at(
    renderer: &mut Renderer,
    id: renderer::ChartId,
    (x, y): (f32, f32),
    scale: f32,
    max: f32,
) -> Option<renderer::PickedData> {
    renderer
        .pick_chart_data(id, pick_request(x, y, scale, max))
        .unwrap()
        .resolve()
        .block_on()
        .unwrap()
}

/// The pick at the centre of the pixel of data `(x, y)`.
fn pick_data_point(
    renderer: &mut Renderer,
    id: renderer::ChartId,
    chart: &Chart,
    (x, y): (f64, f64),
) -> Option<renderer::PickedData> {
    let (px, py) = data_to_px(chart, x, y, 1.0);
    pick_at(renderer, id, (px as f32 + 0.5, py as f32 + 0.5), 1.0, 4.0)
}

fn contour(levels: Vec<f64>, width: f32) -> ContourConfig {
    ContourConfig {
        levels,
        line: line(Color::BLACK, width),
        per_level_color: None,
        labels: None,
    }
}

/// Hover answers the triangle under the cursor with its vertex nearest the
/// cursor, a contour stroke over it as the level with the triangle in
/// `x_index`, the later of two overlapping triangles, and nothing off the
/// mesh.
#[test]
fn hover_answers_the_triangle_its_nearest_vertex_and_the_stroke_over_it() {
    let Some(mut renderer) = try_picking_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let chart = wide_chart(Some(colorbar(0.0, 6.0)));
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        Some(contour(vec![3.0], 2.0)),
    )];
    let id = renderer
        .register_chart(chart.config().clone(), series)
        .unwrap();
    let triangle = |hit: Option<renderer::PickedData>| match hit {
        Some(renderer::PickedData::MeshTriangle {
            series_id,
            triangle_index,
            vertex_index,
            distance_px,
            ..
        }) => {
            assert_eq!(series_id, "mesh");
            assert_eq!(distance_px, 0.0);
            (triangle_index, vertex_index)
        }
        other => panic!("expected a triangle, got {other:?}"),
    };
    // Below the diagonal, nearest (2, 0); above it, nearest (0, 2).
    assert_eq!(
        triangle(pick_data_point(&mut renderer, id, &chart, (1.5, 0.25))),
        (0, 1)
    );
    assert_eq!(
        triangle(pick_data_point(&mut renderer, id, &chart, (0.25, 1.5))),
        (1, 3)
    );
    // On the level x + 2y = 3 inside triangle 0.
    match pick_data_point(&mut renderer, id, &chart, (1.4, 0.8)) {
        Some(renderer::PickedData::ContourLevel {
            level_index,
            x_index,
            y_index,
            ..
        }) => assert_eq!((level_index, x_index, y_index), (0, 0, 0)),
        other => panic!("expected the stroke, got {other:?}"),
    }
    // Off the mesh.
    assert_eq!(
        pick_data_point(&mut renderer, id, &chart, (-0.5, 2.5)),
        None
    );
    renderer.remove_chart(id).unwrap();

    // A third triangle over triangle 0: the later one is drawn on top and
    // answers.
    renderer
        .upsert_triangles(
            "tris",
            &TriangleSource {
                indices: &Column {
                    data: vec![0u32, 1, 2, 0, 2, 3, 0, 1, 2],
                },
            },
        )
        .unwrap();
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    let id = renderer
        .register_chart(chart.config().clone(), series)
        .unwrap();
    assert_eq!(
        triangle(pick_data_point(&mut renderer, id, &chart, (1.5, 0.25))),
        (2, 1)
    );
    renderer.remove_chart(id).unwrap();
}

/// What a pixel of `with` adds over `without`: the largest channel change.
fn ink(with: &RasterImage, without: &RasterImage, x: u32, y: u32) -> i32 {
    let (a, b) = (pixel(with, x, y), pixel(without, x, y));
    (0..4).map(|c| (a[c] - b[c]).abs()).max().unwrap_or(0)
}

/// Every pixel round the mesh picks as it paints, at 1x and 2x: a painted
/// pixel picks the series (within the one-pixel fringe of a triangle edge
/// or the pick distance of a stroke), and a pixel left clear never picks at
/// distance zero. The cases cover interpolated vertex values, flat face
/// values with a transparent NaN triangle (painted nowhere, so picked
/// nowhere) and contour strokes alone.
#[test]
fn triangles_and_strokes_pick_where_they_paint() {
    let Some(mut renderer) = try_picking_renderer() else {
        return;
    };
    // The square, and a third triangle off to the right.
    let xs = vec![0.0, 2.0, 2.0, 0.0, 2.3, 2.9, 2.6];
    let ys = vec![0.0, 0.0, 2.0, 2.0, 0.2, 0.2, 1.7];
    renderer
        .upsert_column("mx", &Column { data: xs.clone() })
        .unwrap();
    renderer
        .upsert_column("my", &Column { data: ys.clone() })
        .unwrap();
    let values: Vec<f64> = xs.iter().zip(&ys).map(|(x, y)| field(*x, *y)).collect();
    renderer
        .upsert_column("mv", &Column { data: values })
        .unwrap();
    renderer
        .upsert_column(
            "mf",
            &Column {
                data: vec![1.0, 4.0, f64::NAN],
            },
        )
        .unwrap();
    renderer
        .upsert_triangles(
            "tris",
            &TriangleSource {
                indices: &Column {
                    data: vec![0u32, 1, 2, 0, 2, 3, 4, 5, 6],
                },
            },
        )
        .unwrap();
    let mut bar = colorbar(0.0, 8.0);
    bar.nan_color = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 0.0,
    };
    let chart = wide_chart(Some(bar));
    let mut faces = mesh_series(
        MeshValueLocation::Faces,
        Some(fill(Shading::Flat, FillMode::Continuous)),
        None,
    );
    if let DataRenderType::TriMesh(mesh) = &mut faces.render_type {
        mesh.values.column = "mf".into();
    }
    let cases = [
        (
            "vertex values",
            mesh_series(
                MeshValueLocation::Vertices,
                Some(fill(Shading::Interpolated, FillMode::Continuous)),
                None,
            ),
        ),
        ("face values", faces),
        (
            "strokes",
            mesh_series(
                MeshValueLocation::Vertices,
                None,
                Some(contour(vec![1.0, 2.5, 4.0, 5.5], 2.0)),
            ),
        ),
    ];
    for (what, series) in cases {
        for scale in [1.0f32, 2.0] {
            let what = format!("{what} at {scale}x");
            let series = vec![series.clone()];
            let with = export(&mut renderer, &chart, &series, scale);
            let without = export(&mut renderer, &chart, &[], scale);
            let (x0, y0) = data_to_px(&chart, -0.2, 2.2, f64::from(scale));
            let (x1, y1) = data_to_px(&chart, 3.1, -0.2, f64::from(scale));
            let id = renderer
                .register_chart(chart.config().clone(), series.clone())
                .unwrap();
            let (mut painted, mut clear, mut failures) = (0usize, 0usize, Vec::new());
            // Every pixel near a change of coverage, a sparse grid elsewhere.
            let probes =
                edge_band::edge_band((x0, y0, x1, y1), |x, y| ink(&with, &without, x, y) >= 24);
            for (px, py) in probes {
                let ink = ink(&with, &without, px, py);
                // The pick distance is the stroke's antialiased fringe.
                let hit = pick_at(
                    &mut renderer,
                    id,
                    (px as f32 + 0.5, py as f32 + 0.5),
                    scale,
                    1.0,
                );
                if ink >= 24 {
                    painted += 1;
                    if hit.is_none() {
                        failures.push(format!("({px}, {py}) ink {ink} picked nothing"));
                    }
                } else if ink == 0 {
                    clear += 1;
                    if hit.as_ref().is_some_and(|hit| hit.distance_px() == 0.0) {
                        failures.push(format!("({px}, {py}) clear but picked {hit:?}"));
                    }
                }
            }
            assert!(
                failures.is_empty(),
                "{what}: {} disagreements, first {:?}",
                failures.len(),
                &failures[..failures.len().min(12)]
            );
            assert!(painted > 200 && clear > 200, "{what}: {painted} / {clear}");
            renderer.remove_chart(id).unwrap();
        }
    }
}

/// The picked triangle draws an outline inside its edges in the highlight
/// colour, and a picked level its stroke widened, at 1x and 2x; the rest of
/// the mesh is unchanged.
#[test]
fn the_highlight_outlines_the_triangle_and_widens_the_level() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let highlight = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        Some(contour(vec![3.0], 2.0)),
    )];
    for scale in [1.0f32, 2.0] {
        let s = f64::from(scale);
        let plain = wide_chart(Some(colorbar(0.0, 6.0)));
        let selected = |refs| {
            let mut chart = wide_chart(Some(colorbar(0.0, 6.0)));
            chart.config_mut().picked_data = Some(renderer::DataSelectionsConfig {
                refs: vec![refs],
                highlight_color: highlight,
                outline_width_px: 3.0,
                contour_width_extra_px: 6.0,
                ..renderer::DataSelectionsConfig::default()
            });
            chart
        };
        let before = export(&mut renderer, &plain, &series, scale);

        let chart = selected(renderer::PickedDataRef::MeshTriangle {
            source_id: None,
            series_id: "mesh".into(),
            triangle_index: 0,
        });
        let img = export(&mut renderer, &chart, &series, scale);
        // One pixel inside triangle 0's bottom edge (y = 0): outlined.
        let (x, y) = data_to_px(&chart, 1.5, 0.0, s);
        assert_colour(
            &img,
            (x, y - (2.0 * s) as u32),
            highlight,
            8,
            &format!("triangle outline at {scale}x"),
        );
        // Its centroid and triangle 1's are unchanged.
        for (cx, cy) in [(4.0 / 3.0, 2.0 / 3.0), (2.0 / 3.0, 4.0 / 3.0)] {
            let (x, y) = data_to_px(&chart, cx, cy, s);
            assert_eq!(
                pixel(&img, x, y),
                pixel(&before, x, y),
                "centroid ({cx}, {cy}) at {scale}x"
            );
        }

        let chart = selected(renderer::PickedDataRef::ContourLevel {
            source_id: None,
            series_id: "mesh".into(),
            level_index: 0,
            x_index: 0,
            y_index: 0,
        });
        let img = export(&mut renderer, &chart, &series, scale);
        // On the level, x + 2y = 3, in both triangles.
        for (lx, ly) in [(1.4, 0.8), (0.6, 1.2)] {
            let (x, y) = data_to_px(&chart, lx, ly, s);
            assert_colour(
                &img,
                (x, y),
                highlight,
                8,
                &format!("level at ({lx}, {ly}) at {scale}x"),
            );
        }
        // Far from it, unchanged.
        let (x, y) = data_to_px(&chart, 0.3, 0.2, s);
        assert_eq!(pixel(&img, x, y), pixel(&before, x, y), "far at {scale}x");
    }
}

// ───── Interactions over a mesh (#210 PR 4, rule 6) ─────────────────────────

/// Image pixel of data `(x, y)` on `config`'s x axis and `y_axis`.
fn axis_px(
    config: &renderer::Config,
    y_axis: &renderer::config::AxisOptions,
    x: f64,
    y: f64,
    scale: f32,
) -> (u32, u32) {
    let da = config.scaled(scale).data_area().unwrap().0;
    let fx = renderer::layout::axis_fraction(x, &config.bottom_x);
    let fy = renderer::layout::axis_fraction(y, y_axis);
    (
        (da.x as f64 + da.width as f64 * f64::from(fx)) as u32,
        (da.y as f64 + da.height as f64 * (1.0 - f64::from(fy))) as u32,
    )
}

fn expect_triangle(hit: Option<renderer::PickedData>, what: &str) -> usize {
    match hit {
        Some(renderer::PickedData::MeshTriangle { triangle_index, .. }) => triangle_index,
        other => panic!("{what}: expected a triangle, got {other:?}"),
    }
}

/// A moved and resized plot area draws the mesh where its chrome puts it and
/// picks it there, at 1x and 2x.
#[test]
fn a_moved_and_resized_plot_area_draws_and_picks_the_mesh() {
    use renderer::layout::{Element, NudgeResult, Side};
    let Some(mut renderer) = try_picking_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let mut chart = chart(Some(bar.clone()));
    let measure = renderer::CpuTextMeasure::for_style(&chart.config().draw_style);
    let config = chart.config_mut();
    assert_eq!(
        config.nudge(Element::DataArea, &measure, 12.0, -8.0),
        NudgeResult::Moved
    );
    assert_eq!(
        config.nudge(Element::DataAreaEdge(Side::Right), &measure, -40.0, 0.0),
        NudgeResult::Moved
    );
    assert_eq!(
        config.nudge(Element::DataAreaEdge(Side::Top), &measure, 0.0, 30.0),
        NudgeResult::Moved
    );
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    let id = renderer
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    for scale in [1.0f32, 2.0] {
        let img = export(&mut renderer, &chart, &series, scale);
        for ((x, y), want) in [((1.5, 0.25), 0usize), ((0.25, 1.5), 1)] {
            let at = chrome_px(chart.config(), x, y, scale);
            assert_colour(
                &img,
                at,
                bar.color_for_z(field(x, y)),
                3,
                &format!("({x}, {y}) at {scale}x"),
            );
            let hit = pick_at(
                &mut renderer,
                id,
                (at.0 as f32 + 0.5, at.1 as f32 + 0.5),
                scale,
                1.0,
            );
            assert_eq!(expect_triangle(hit, "resized"), want);
        }
    }
    renderer.remove_chart(id).unwrap();
}

/// A mesh on an overlay layer draws on that layer's y axis and, with the
/// overlay active, picks there.
#[test]
fn a_mesh_on_an_overlay_draws_and_picks_by_its_axis() {
    let Some(mut renderer) = try_picking_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let mut chart = chart(Some(bar.clone()));
    let mut overlay = renderer::OverlayLayer {
        series: vec!["mesh".into()],
        ..Default::default()
    };
    overlay.y_axis.min = -2.0;
    overlay.y_axis.max = 2.0;
    overlay.y_axis.out_margin = 60.0;
    chart.config_mut().overlays.push(overlay);
    let overlay_axis = chart.config().overlays[0].y_axis.clone();
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    let mut active = chart.config().clone();
    active.set_active_layer(renderer::LayerRef::Overlay(0));
    let id = renderer.register_chart(active, series.clone()).unwrap();
    for scale in [1.0f32, 2.0] {
        // The whole document: the overlay layer draws too.
        let img = renderer
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        for ((x, y), want) in [((1.5, 0.25), 0usize), ((0.25, 1.5), 1)] {
            let at = axis_px(chart.config(), &overlay_axis, x, y, scale);
            assert_colour(
                &img,
                at,
                bar.color_for_z(field(x, y)),
                3,
                &format!("overlay ({x}, {y}) at {scale}x"),
            );
            let hit = pick_at(
                &mut renderer,
                id,
                (at.0 as f32 + 0.5, at.1 as f32 + 0.5),
                scale,
                1.0,
            );
            assert_eq!(expect_triangle(hit, "overlay"), want);
        }
        // Below the overlay's 0, where the base axis alone would put it:
        // nothing of the mesh.
        let below = axis_px(chart.config(), &overlay_axis, 1.0, -1.0, scale);
        let hit = pick_at(
            &mut renderer,
            id,
            (below.0 as f32 + 0.5, below.1 as f32 + 0.5),
            scale,
            1.0,
        );
        assert_eq!(hit, None, "overlay below its axis' 0 at {scale}x");
    }
    renderer.remove_chart(id).unwrap();
}

/// A mesh in a subplot cell draws in that cell's data area, and the
/// document picks it there.
#[test]
fn a_mesh_subplot_draws_and_picks_its_triangles() {
    let Some(mut renderer) = try_picking_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let parent = chart(Some(bar.clone()));
    let mut cell = chart(Some(bar.clone())).config().clone();
    cell.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT,
        width: WIDTH,
        height: HEIGHT,
    });
    let mut document = parent.config().clone();
    document.subplots.push(renderer::Subplot {
        config: cell.clone(),
        series: vec!["mesh".into()],
        link_x: None,
        link_y: None,
    });
    let doc = Chart::new(document.clone());
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    // A pick answers for the active layer's series, as a host's pointer
    // does once the user has made the cell active.
    document.set_active_layer(renderer::LayerRef::Subplot(0));
    let id = renderer.register_chart(document, series.clone()).unwrap();
    for scale in [1.0f32, 2.0] {
        let img = renderer
            .export_rgba(
                renderer::ExportSource::chart(&doc, &series),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        for ((x, y), want) in [((1.5, 0.25), 0usize), ((0.25, 1.5), 1)] {
            let at = chrome_px(&cell, x, y, scale);
            assert_colour(
                &img,
                at,
                bar.color_for_z(field(x, y)),
                3,
                &format!("subplot ({x}, {y}) at {scale}x"),
            );
            let request = renderer::GpuPickRequest {
                canvas_position_px: [at.0 as f32 + 0.5, at.1 as f32 + 0.5],
                display_panel_px: Rect {
                    x: 0,
                    y: 0,
                    width: (WIDTH as f32 * scale).round() as u32,
                    height: (2.0 * HEIGHT as f32 * scale).round() as u32,
                },
                display_scale: scale,
                max_distance_px: 1.0,
            };
            let hit = renderer
                .pick_chart_data(id, request)
                .unwrap()
                .resolve()
                .block_on()
                .unwrap();
            assert_eq!(expect_triangle(hit, "subplot"), want);
        }
    }
    renderer.remove_chart(id).unwrap();
}

/// A ratio-anchored annotation draws over the mesh in the document export.
#[test]
fn an_annotation_draws_over_the_mesh() {
    use renderer::{Anchor, Annotation, AnnotationKind, BoxStyle};
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let mut chart = chart(Some(colorbar(0.0, 6.0)));
    chart.config_mut().annotations.items = vec![Annotation::new(AnnotationKind::Text {
        anchor: Anchor::Data { x: 1.5, y: 0.5 },
        text: renderer::text::RichText::plain("note", Color::BLACK, 14.0, ""),
        box_style: Some(BoxStyle {
            fill: Color::WHITE,
            border_color: Color::BLACK,
            border_width: 1.0,
            padding: 6.0,
        }),
    })];
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    for scale in [1.0f32, 2.0] {
        let img = renderer
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        let (cx, cy) = chrome_px(chart.config(), 1.5, 0.5, scale);
        let reach = (20.0 * scale) as i64;
        let white = (-reach..=reach).any(|dy| {
            (-reach..=reach).any(|dx| {
                let (x, y) = ((cx as i64 + dx) as u32, (cy as i64 + dy) as u32);
                pixel(&img, x, y).iter().all(|c| *c > 240)
            })
        });
        assert!(white, "the note's white box over the mesh at {scale}x");
    }
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    assert!(view.annotation_map().find(0).is_some());
}

/// Dragging the colourbar, its labels and the tick labels moves them and
/// leaves the mesh as it was drawn.
#[test]
fn decoration_drags_leave_the_mesh_in_place() {
    use renderer::layout::{Element, NudgeResult, Side};
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    square(&mut renderer, SQUARE_X, vertex_field());
    let bar = colorbar(0.0, 6.0);
    let plain = chart(Some(bar.clone()));
    let mut dragged = chart(Some(bar));
    let measure = renderer::CpuTextMeasure::for_style(&dragged.config().draw_style);
    let config = dragged.config_mut();
    for (element, dx, dy) in [
        (Element::ColorBar, -12.0, 10.0),
        (Element::ColorBarLabel, 6.0, 0.0),
        (Element::AxisLabel(Side::Left), -6.0, 0.0),
        (Element::AxisLabel(Side::Bottom), 0.0, 6.0),
    ] {
        assert_eq!(
            config.nudge(element.clone(), &measure, dx, dy),
            NudgeResult::Moved,
            "{element:?}"
        );
    }
    let series = vec![mesh_series(
        MeshValueLocation::Vertices,
        Some(fill(Shading::Interpolated, FillMode::Continuous)),
        None,
    )];
    for scale in [1.0f32, 2.0] {
        let before = export(&mut renderer, &plain, &series, scale);
        let after = export(&mut renderer, &dragged, &series, scale);
        assert_ne!(
            before.rgba, after.rgba,
            "the drags moved something at {scale}x"
        );
        for (x, y) in [(1.5, 0.25), (0.25, 1.5), (1.0, 1.0)] {
            let at = chrome_px(plain.config(), x, y, scale);
            assert_eq!(
                chrome_px(dragged.config(), x, y, scale),
                at,
                "the data area stayed"
            );
            assert_eq!(
                pixel(&after, at.0, at.1),
                pixel(&before, at.0, at.1),
                "mesh at ({x}, {y}) at {scale}x"
            );
        }
    }
}
