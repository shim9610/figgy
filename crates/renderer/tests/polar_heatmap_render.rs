//! End-to-end pixel verification of a heatmap and its contours on a polar
//! chart (#208), through the public renderer API and the headless
//! `export_rgba`: annular cells at 1x and 2x, the angle wrap, the
//! interpolated seam, the hole, the sector, log and inverted radius axes, a
//! categorical angle axis, the pick, `step_selection`, the picked cell's
//! outline, ring and spoke contours with their labels, the contour pick and
//! highlight, a subplot, annotations, the refusals and the rule-10 grids.
//!
//! The colour ramp runs red to green, so a cell's colour names its value
//! (`ColorBarOptions::color_for_z`). Expected positions come from the
//! chart's own `polar_geometry` and `polar_angle_map`, the circle and the
//! angle mapping the renderer draws with.

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::f64::consts::{PI, TAU};
use std::sync::OnceLock;

use renderer::config::{AxisScale, Category, TickVisibility};
use renderer::data::Column;
use renderer::data_config::{
    ContourConfig, ContourLabelConfig, DataLineStyleConfig, FieldFillConfig, FieldSeries, FillMode,
    GridLayout, LineInterpolation, MatrixOrientation, MatrixRef, Shading,
};
use renderer::format::LabelFormat;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::polar::{GridShape, PolarOptions};
use renderer::{
    Anchor, Annotation, AnnotationKind, AnnotationSkip, BoxStyle, Chart, Color, ColorBarOptions,
    ColorMap, ColumnSource, DataRenderType, DataSelectionsConfig, PickedData, PickedDataRef,
    RasterImage, RenderWarning, Renderer, RendererDevice, SeriesConfig, Subplot,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 480;
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
const LINE: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};
const MAGENTA: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};
const LABEL_BG: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(test_gpu::device);
    (device.clone(), queue.clone())
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init")
}

fn col(data: Vec<f64>) -> Column<f64> {
    Column { data }
}

/// Uploads the angle coordinates `ga`, the radius coordinates `gr` and
/// `columns` constituent columns `z{i}` of `rows` values, `z(i, j)`.
/// Returns the constituent ids.
fn upload(
    r: &mut Renderer,
    ga: &[f64],
    gr: &[f64],
    (columns, rows): (usize, usize),
    z: impl Fn(usize, usize) -> f64,
) -> Vec<String> {
    let ga = col(ga.to_vec());
    let gr = col(gr.to_vec());
    let ids: Vec<String> = (0..columns).map(|i| format!("z{i}")).collect();
    let values: Vec<Column<f64>> = (0..columns)
        .map(|i| col((0..rows).map(|j| z(i, j)).collect()))
        .collect();
    let mut batch: Vec<(&str, &dyn ColumnSource)> = vec![("ga", &ga), ("gr", &gr)];
    batch.extend(
        ids.iter()
            .zip(&values)
            .map(|(id, c)| (id.as_str(), c as &dyn ColumnSource)),
    );
    r.add_columns(&batch).unwrap();
    ids
}

fn colorbar(z_min: f64, z_max: f64) -> ColorBarOptions {
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Custom {
        stops: vec![LOW, HIGH],
    };
    bar.visible = false;
    bar.axis.min = z_min;
    bar.axis.max = z_max;
    bar.axis.major_spacing = (z_max - z_min) / 5.0;
    bar
}

/// A polar chart clockwise from north, the radius over `radius`, no grid,
/// chrome or legend: the only ink is the field.
fn chart(radius: (f64, f64), bar: ColorBarOptions) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.chart_title.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.label_style.visible = false;
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.title_option.visible = false;
    }
    config.colorbar = Some(bar);
    config.set_polar(Some(PolarOptions {
        zero_angle_degrees: 90.0,
        clockwise: true,
        ..PolarOptions::default()
    }));
    let mut chart = Chart::new(config);
    chart.set_y_range(radius.0, radius.1);
    chart
}

fn polar_mut(chart: &mut Chart) -> &mut PolarOptions {
    chart.config_mut().polar_mut().expect("polar")
}

fn field(
    ids: &[String],
    layout: GridLayout,
    shading: Shading,
    contour: Option<ContourConfig>,
    fill: bool,
) -> SeriesConfig {
    SeriesConfig {
        series_id: "field".into(),
        source_id: None,
        label: None,
        x_column: "ga".into(),
        y_column: "gr".into(),
        z_column: None,
        render_type: DataRenderType::Field(FieldSeries {
            matrix: MatrixRef {
                columns: ids.to_vec(),
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: layout,
            },
            fill: fill.then_some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading,
                opacity: 1.0,
            }),
            contour,
            surface: None,
        }),
    }
}

fn contour(levels: Vec<f64>, width: f32, labels: Option<ContourLabelConfig>) -> ContourConfig {
    ContourConfig {
        levels,
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: LINE,
            line_width: width,
            fill: None,
            fill_color: None,
            interpolation: LineInterpolation::Linear,
        },
        per_level_color: None,
        labels,
    }
}

/// The ring's centre, radius and inner radius in export pixels.
fn geometry(chart: &Chart, scale: f32) -> ((f64, f64), f64, f64) {
    let g = chart
        .config()
        .scaled(scale)
        .polar_geometry()
        .expect("geometry");
    (
        (f64::from(g.center[0]), f64::from(g.center[1])),
        f64::from(g.radius),
        f64::from(g.inner_radius),
    )
}

/// The radius in export pixels of value `v` on the radius axis, linear or
/// logarithmic, either direction.
fn radius_px(chart: &Chart, scale: f32, v: f64) -> f64 {
    let (_, outer, inner) = geometry(chart, scale);
    let axis = &chart.config().left_y;
    let t = match axis.scale {
        AxisScale::Logarithmic => {
            (v.log10() - axis.min.log10()) / (axis.max.log10() - axis.min.log10())
        }
        _ => (v - axis.min) / (axis.max - axis.min),
    };
    inner + (outer - inner) * t
}

/// The screen angle (radians, counter-clockwise with y up) of angle value
/// `x`.
fn angle(chart: &Chart, x: f64) -> f64 {
    chart
        .config()
        .polar_angle_map()
        .expect("polar")
        .screen_angle(x)
}

/// The export pixel at screen angle `a` and radius `r` from the centre.
fn at(centre: (f64, f64), a: f64, r: f64) -> (f64, f64) {
    (centre.0 + a.cos() * r, centre.1 - a.sin() * r)
}

/// The export pixel at angle value `x` and radius value `v`.
fn at_value(chart: &Chart, scale: f32, x: f64, v: f64) -> (f64, f64) {
    let (centre, _, _) = geometry(chart, scale);
    at(centre, angle(chart, x), radius_px(chart, scale, v))
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

fn to8(v: f32) -> i32 {
    (v.clamp(0.0, 1.0) * 255.0).round() as i32
}

fn assert_colour(img: &RasterImage, p: (f64, f64), want: Color, what: &str) {
    let got = pixel(img, p);
    let expected = [to8(want.r), to8(want.g), to8(want.b), to8(want.a)];
    assert!(
        got.iter()
            .zip(expected)
            .all(|(g, e)| (i32::from(*g) - e).abs() <= 2),
        "{what} at {p:?}: got {got:?}, expected {expected:?}"
    );
}

fn is_empty(p: [u8; 4]) -> bool {
    p[3] < 16
}
fn is_line(p: [u8; 4]) -> bool {
    p[3] > 200 && p[2] > 200 && p[0] < 60 && p[1] < 60
}
fn is_magenta(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[2] > 200 && p[1] < 60
}

/// Every pixel `pred` accepts, as pixel centres.
fn pixels_where(img: &RasterImage, pred: fn([u8; 4]) -> bool) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for y in 0..img.height {
        for x in 0..img.width {
            let p = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            if pred(pixel(img, p)) {
                out.push(p);
            }
        }
    }
    out
}

/// Whether any pixel within `radius` of `centre` satisfies `pred`.
fn any_within(
    img: &RasterImage,
    centre: (f64, f64),
    radius: f64,
    pred: fn([u8; 4]) -> bool,
) -> bool {
    let n = radius.ceil() as i64;
    (-n..=n)
        .any(|dy| (-n..=n).any(|dx| pred(pixel(img, (centre.0 + dx as f64, centre.1 + dy as f64)))))
}

fn polar_of(centre: (f64, f64), p: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (p.0 - centre.0, centre.1 - p.1);
    ((dx * dx + dy * dy).sqrt(), dy.atan2(dx))
}

/// `a - b` wrapped to (-π, π].
fn angle_diff(a: f64, b: f64) -> f64 {
    (a - b + PI).rem_euclid(TAU) - PI
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_rgba(
        renderer::ExportSource::chart(chart, series),
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

fn pick_request(p: (f64, f64)) -> renderer::GpuPickRequest {
    renderer::GpuPickRequest {
        canvas_position_px: [p.0 as f32, p.1 as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT,
        },
        display_scale: 1.0,
        max_distance_px: 0.0,
    }
}

fn pick(r: &mut Renderer, id: renderer::ChartId, p: (f64, f64)) -> Option<PickedData> {
    test_gpu::block_on(r.pick_chart_data(id, pick_request(p)).unwrap().resolve()).unwrap()
}

fn cell_of(picked: Option<PickedData>) -> Option<(usize, usize)> {
    match picked {
        Some(PickedData::MatrixCell {
            x_index, y_index, ..
        }) => Some((x_index, y_index)),
        _ => None,
    }
}

/// Eight 45° angle cells by four radius cells on edges, every value its own.
const ANGLES: usize = 8;
const RINGS: usize = 4;

fn z_cell(i: usize, j: usize) -> f64 {
    (i * RINGS + j) as f64
}

fn edges_grid(r: &mut Renderer) -> Vec<String> {
    let ga: Vec<f64> = (0..=ANGLES).map(|i| i as f64 * 45.0).collect();
    let gr: Vec<f64> = (0..=RINGS).map(|j| j as f64).collect();
    // Edges: n + 1 boundaries, n cells per constituent.
    upload(r, &ga, &gr, (ANGLES, RINGS), z_cell)
}

/// Each cell paints its own value's colour across its annular sector, at
/// 1x and 2x; past the outer ring nothing is drawn.
#[test]
fn cells_fill_their_annular_sectors_at_1x_and_2x() {
    let mut r = renderer();
    let ids = edges_grid(&mut r);
    let bar = colorbar(0.0, (ANGLES * RINGS - 1) as f64);
    let chart = chart((0.0, RINGS as f64), bar.clone());
    let series = [field(&ids, GridLayout::Edges, Shading::Flat, None, true)];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("polar-heatmap-cells-{scale}x"), &img);
        let (centre, outer, _) = geometry(&chart, scale);
        for i in 0..ANGLES {
            for j in 0..RINGS {
                let want = bar.color_for_z(z_cell(i, j));
                // The middle, and close to each corner, inside the cell.
                for (fa, fr) in [
                    (0.5, 0.5),
                    (0.15, 0.2),
                    (0.85, 0.8),
                    (0.2, 0.85),
                    (0.8, 0.15),
                ] {
                    let a = (i as f64 + fa) * 45.0;
                    let v = j as f64 + fr;
                    // Keep two pixels inside the cell at its narrow end.
                    let p = at_value(&chart, scale, a, v);
                    let (rr, _) = polar_of(centre, p);
                    if rr < 6.0 {
                        continue;
                    }
                    assert_colour(
                        &img,
                        p,
                        want,
                        &format!("{scale}x cell ({i}, {j}) at ({fa}, {fr})"),
                    );
                }
            }
            let past = at(
                centre,
                angle(&chart, (i as f64 + 0.5) * 45.0),
                outer + 3.0 * f64::from(scale),
            );
            assert!(
                is_empty(pixel(&img, past)),
                "{scale}x: past the ring at angle cell {i}"
            );
        }
    }
}

/// The hole inside `inner_radius_fraction` and the angles outside a sector
/// draw nothing; the cells keep their place between them.
#[test]
fn the_hole_and_the_sector_clip_the_cells() {
    let mut r = renderer();
    let ids = edges_grid(&mut r);
    let bar = colorbar(0.0, (ANGLES * RINGS - 1) as f64);
    let mut chart = chart((0.0, RINGS as f64), bar.clone());
    polar_mut(&mut chart).inner_radius_fraction = 0.3;
    polar_mut(&mut chart).sector = Some([0.0, 180.0]);
    let series = [field(&ids, GridLayout::Edges, Shading::Flat, None, true)];
    let img = export(&mut r, &chart, &series, 1.0);
    save("polar-heatmap-hole-sector", &img);
    let (centre, _, inner) = geometry(&chart, 1.0);
    assert!(inner > 20.0);
    for a in [10.0, 100.0, 170.0, 200.0, 300.0] {
        let hole = at(centre, angle(&chart, a), inner * 0.5);
        assert!(is_empty(pixel(&img, hole)), "the hole at {a}°");
    }
    for i in 0..ANGLES {
        let a = (i as f64 + 0.5) * 45.0;
        for j in 0..RINGS {
            let p = at_value(&chart, 1.0, a, j as f64 + 0.5);
            if a < 180.0 {
                assert_colour(
                    &img,
                    p,
                    bar.color_for_z(z_cell(i, j)),
                    &format!("cell ({i}, {j})"),
                );
            } else {
                assert!(
                    is_empty(pixel(&img, p)),
                    "outside the sector: cell ({i}, {j})"
                );
            }
        }
    }
}

/// A grid that starts past a turn draws each cell where a marker at its
/// angle would sit: the fragment's angle is read in the turn starting at
/// the grid's low boundary.
#[test]
fn the_angle_wraps_into_the_turn_from_the_grid_start() {
    let mut r = renderer();
    let ga: Vec<f64> = (0..=ANGLES).map(|i| 180.0 + i as f64 * 45.0).collect();
    let ids = upload(&mut r, &ga, &[0.0, 1.0], (ANGLES, 1), |i, _| i as f64);
    let bar = colorbar(0.0, (ANGLES - 1) as f64);
    let chart = chart((0.0, 1.0), bar.clone());
    let series = [field(&ids, GridLayout::Edges, Shading::Flat, None, true)];
    let img = export(&mut r, &chart, &series, 1.0);
    save("polar-heatmap-wrap", &img);
    for i in 0..ANGLES {
        let value = 180.0 + (i as f64 + 0.5) * 45.0;
        // The same screen place reads either value of the turn.
        for shown in [value, value - 360.0] {
            assert_colour(
                &img,
                at_value(&chart, 1.0, shown, 0.6),
                bar.color_for_z(i as f64),
                &format!("cell {i} at {shown}°"),
            );
        }
    }
}

/// An interpolated full circle leaves a wedge between its last and first
/// sample angles; naming the first constituent again at `first + turn`
/// closes it with the colours in between (D7). No column is copied.
#[test]
fn repeating_the_first_column_closes_the_interpolated_seam() {
    let mut r = renderer();
    let z = [0.0, 4.0, 8.0, 2.0];
    let ga = col(vec![0.0, 90.0, 180.0, 270.0]);
    let closed = col(vec![0.0, 90.0, 180.0, 270.0, 360.0]);
    let gr = col(vec![0.0, 1.0]);
    let zs: Vec<Column<f64>> = z.iter().map(|v| col(vec![*v, *v])).collect();
    r.add_columns(&[
        ("ga", &ga as &dyn ColumnSource),
        ("ga_closed", &closed),
        ("gr", &gr),
        ("z0", &zs[0]),
        ("z1", &zs[1]),
        ("z2", &zs[2]),
        ("z3", &zs[3]),
    ])
    .unwrap();
    let bar = colorbar(0.0, 8.0);
    let chart = chart((0.0, 1.0), bar.clone());
    let ids: Vec<String> = (0..4).map(|i| format!("z{i}")).collect();
    let open = field(&ids, GridLayout::Centers, Shading::Interpolated, None, true);
    let img = export(&mut r, &chart, &[open], 1.0);
    let seam = at_value(&chart, 1.0, 315.0, 0.5);
    assert!(is_empty(pixel(&img, seam)), "an open grid leaves the seam");
    // Between two samples the colour blends in angle.
    assert_colour(
        &img,
        at_value(&chart, 1.0, 45.0, 0.5),
        bar.color_for_z(2.0),
        "between 0° and 90°",
    );
    let mut ids_closed = ids.clone();
    ids_closed.push("z0".into());
    let mut closed_series = field(
        &ids_closed,
        GridLayout::Centers,
        Shading::Interpolated,
        None,
        true,
    );
    closed_series.x_column = "ga_closed".into();
    let img = export(&mut r, &chart, &[closed_series], 1.0);
    save("polar-heatmap-seam", &img);
    assert_colour(&img, seam, bar.color_for_z(1.0), "the closed seam");
}

/// A logarithmic radius axis places the ring boundaries by decade, and an
/// inverted one puts the first ring at the outside.
#[test]
fn log_and_inverted_radius_axes_place_the_rings() {
    let mut r = renderer();
    let ga = col(vec![0.0, 180.0, 360.0]);
    let decades = col(vec![1.0, 10.0, 100.0, 1000.0]);
    let linear = col(vec![0.0, 1.0, 2.0, 3.0]);
    let zs: Vec<Column<f64>> = (0..2)
        .map(|i| col((0..3).map(|j| (i * 3 + j) as f64).collect()))
        .collect();
    r.add_columns(&[
        ("ga", &ga as &dyn ColumnSource),
        ("gr", &decades),
        ("gr_linear", &linear),
        ("z0", &zs[0]),
        ("z1", &zs[1]),
    ])
    .unwrap();
    let ids: Vec<String> = vec!["z0".into(), "z1".into()];
    let bar = colorbar(0.0, 5.0);
    let mut log = chart((1.0, 1000.0), bar.clone());
    log.config_mut().left_y.scale = AxisScale::Logarithmic;
    let series = [field(&ids, GridLayout::Edges, Shading::Flat, None, true)];
    let img = export(&mut r, &log, &series, 1.0);
    save("polar-heatmap-log", &img);
    for i in 0..2 {
        for j in 0..3 {
            let v = 10f64.powf(j as f64 + 0.5);
            let a = (i as f64 + 0.5) * 180.0;
            assert_colour(
                &img,
                at_value(&log, 1.0, a, v),
                bar.color_for_z((i * 3 + j) as f64),
                &format!("log cell ({i}, {j})"),
            );
        }
    }
    let inverted = chart((3.0, 0.0), bar.clone());
    let mut linear_series = field(&ids, GridLayout::Edges, Shading::Flat, None, true);
    linear_series.y_column = "gr_linear".into();
    let img = export(&mut r, &inverted, &[linear_series], 1.0);
    save("polar-heatmap-inverted", &img);
    let (centre, outer, _) = geometry(&inverted, 1.0);
    for i in 0..2 {
        let a = angle(&inverted, (i as f64 + 0.5) * 180.0);
        // The first ring (0..1) is the outermost.
        assert_colour(
            &img,
            at(centre, a, outer * 5.0 / 6.0),
            bar.color_for_z((i * 3) as f64),
            "inverted first ring",
        );
        assert_colour(
            &img,
            at(centre, a, outer / 6.0),
            bar.color_for_z((i * 3 + 2) as f64),
            "inverted last ring",
        );
    }
}

/// On a categorical angle axis a centred grid over `0..n` gives one cell
/// per category slot.
#[test]
fn a_categorical_angle_axis_gives_each_slot_its_cell() {
    let mut r = renderer();
    let n = 5;
    let ga: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let ids = upload(&mut r, &ga, &[0.25, 0.75], (n, 2), |i, _| i as f64);
    let bar = colorbar(0.0, (n - 1) as f64);
    let mut chart = chart((0.0, 1.0), bar.clone());
    chart.config_mut().bottom_x.categories = Some(
        (0..n)
            .map(|i| Category::new(format!("c{i}"), format!("Cat {i}")))
            .collect(),
    );
    let series = [field(&ids, GridLayout::Centers, Shading::Flat, None, true)];
    let img = export(&mut r, &chart, &series, 1.0);
    save("polar-heatmap-categories", &img);
    for i in 0..n {
        assert_colour(
            &img,
            at_value(&chart, 1.0, i as f64, 0.5),
            bar.color_for_z(i as f64),
            &format!("category {i}"),
        );
    }
}

/// The pick names the cell under the cursor in (angle, radius) indices,
/// nothing in the hole, past the ring or outside the sector, and
/// `step_selection` moves along the angle and the radius.
#[test]
fn the_pick_answers_the_annular_cell_and_the_selection_steps() {
    let mut r = renderer();
    let ids = edges_grid(&mut r);
    let mut chart = chart((0.0, RINGS as f64), colorbar(0.0, 31.0));
    polar_mut(&mut chart).inner_radius_fraction = 0.2;
    polar_mut(&mut chart).sector = Some([0.0, 270.0]);
    r.enable_gpu_picking().unwrap();
    let series = vec![field(&ids, GridLayout::Edges, Shading::Flat, None, true)];
    let id = r
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let (centre, outer, inner) = geometry(&chart, 1.0);
    for i in 0..ANGLES {
        let a = (i as f64 + 0.5) * 45.0;
        for j in 0..RINGS {
            let got = cell_of(pick(&mut r, id, at_value(&chart, 1.0, a, j as f64 + 0.5)));
            if a < 270.0 {
                assert_eq!(got, Some((i, j)), "cell ({i}, {j})");
            } else {
                assert_eq!(got, None, "outside the sector: ({i}, {j})");
            }
        }
    }
    assert_eq!(
        cell_of(pick(
            &mut r,
            id,
            at(centre, angle(&chart, 30.0), inner * 0.5)
        )),
        None,
        "the hole"
    );
    assert_eq!(
        cell_of(pick(
            &mut r,
            id,
            at(centre, angle(&chart, 30.0), outer + 4.0)
        )),
        None,
        "past the ring"
    );
    let mut stepped = chart.config().clone();
    stepped.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::MatrixCell {
            source_id: None,
            series_id: "field".into(),
            x_index: 0,
            y_index: 0,
        }],
        ..DataSelectionsConfig::default()
    });
    let len = |id: &str| match id {
        "ga" => Some(ANGLES + 1),
        "gr" => Some(RINGS + 1),
        _ => Some(RINGS),
    };
    assert_eq!(stepped.step_selection(&series, 1, 2, len), Ok(true));
    assert!(matches!(
        stepped.picked_data.as_ref().unwrap().refs[0],
        PickedDataRef::MatrixCell {
            x_index: 1,
            y_index: 2,
            ..
        }
    ));
    // The last angle cell: the step clamps, as on a rectangular grid (D6).
    assert_eq!(stepped.step_selection(&series, 100, 0, len), Ok(true));
    assert!(matches!(
        stepped.picked_data.as_ref().unwrap().refs[0],
        PickedDataRef::MatrixCell {
            x_index: 7,
            y_index: 2,
            ..
        }
    ));
}

/// The signed distance in pixels from `p` to the annular sector between
/// `r0` and `r1` over the screen angles `(a, b)`.
fn sector_distance(p: (f64, f64), centre: (f64, f64), r0: f64, r1: f64, (a, b): (f64, f64)) -> f64 {
    let (r, phi) = polar_of(centre, p);
    let mid = (a + b) * 0.5;
    let rel = angle_diff(phi, mid);
    let half = (b - a).abs() * 0.5;
    let radial = (r0 - r).max(r - r1);
    if rel.abs() <= half {
        return radial.max(-(half - rel.abs()).sin() * r);
    }
    let edge = if rel > 0.0 { a.max(b) } else { a.min(b) };
    let (dx, dy) = (p.0 - centre.0, centre.1 - p.1);
    let (ex, ey) = (edge.cos(), edge.sin());
    let along = (dx * ex + dy * ey).clamp(r0, r1);
    ((dx - ex * along).powi(2) + (dy - ey * along).powi(2)).sqrt()
}

/// The picked cell's outline runs inside its annular sector along both arcs
/// and both spokes, at 1x and 2x.
#[test]
fn the_picked_highlight_outlines_the_annular_cell() {
    let mut r = renderer();
    let ids = edges_grid(&mut r);
    let mut chart = chart((0.0, RINGS as f64), colorbar(0.0, 31.0));
    let series = [field(&ids, GridLayout::Edges, Shading::Flat, None, true)];
    let (ci, cj) = (2usize, 1usize);
    for scale in [1.0f32, 2.0] {
        chart.config_mut().picked_data = Some(DataSelectionsConfig {
            visible: true,
            refs: vec![PickedDataRef::MatrixCell {
                source_id: None,
                series_id: "field".into(),
                x_index: ci,
                y_index: cj,
            }],
            highlight_color: MAGENTA,
            outline_width_px: 3.0,
            point_radius_extra_px: 0.0,
            contour_width_extra_px: 0.0,
        });
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("polar-heatmap-highlight-{scale}x"), &img);
        let (centre, _, _) = geometry(&chart, scale);
        let r0 = radius_px(&chart, scale, cj as f64);
        let r1 = radius_px(&chart, scale, cj as f64 + 1.0);
        let angles = (
            angle(&chart, ci as f64 * 45.0),
            angle(&chart, (ci + 1) as f64 * 45.0),
        );
        let band = 3.0 * f64::from(scale);
        let drawn = pixels_where(&img, is_magenta);
        assert!(drawn.len() > 50, "{scale}x: the cell is outlined");
        let mut sides = [false; 4];
        for p in drawn {
            let d = sector_distance(p, centre, r0, r1, angles);
            assert!(
                d <= 1.0 && d >= -(band + 1.0),
                "{scale}x: outline pixel {p:?} is {d:.2} px from the cell boundary"
            );
            let (rr, phi) = polar_of(centre, p);
            sides[0] |= (rr - r0).abs() <= band + 1.0;
            sides[1] |= (r1 - rr).abs() <= band + 1.0;
            sides[2] |= angle_diff(phi, angles.0).abs() * rr <= band + 1.0;
            sides[3] |= angle_diff(phi, angles.1).abs() * rr <= band + 1.0;
        }
        assert_eq!(
            sides, [true; 4],
            "{scale}x: inner arc, outer arc, both spokes"
        );
    }
}

/// A field whose value is its radius: every level is a circle. Centred
/// samples on 30° steps with the first column repeated at 360° so the ring
/// closes.
fn ring_field(r: &mut Renderer) -> Vec<String> {
    let ga = col((0..=12).map(|i| i as f64 * 30.0).collect());
    let gr = col((0..=4).map(|j| j as f64).collect());
    let zs: Vec<Column<f64>> = (0..12)
        .map(|_| col((0..=4).map(|j| j as f64).collect()))
        .collect();
    let ids: Vec<String> = (0..12).map(|i| format!("z{i}")).collect();
    let mut batch: Vec<(&str, &dyn ColumnSource)> = vec![("ga", &ga), ("gr", &gr)];
    batch.extend(
        ids.iter()
            .zip(&zs)
            .map(|(id, c)| (id.as_str(), c as &dyn ColumnSource)),
    );
    r.add_columns(&batch).unwrap();
    let mut closed = ids;
    closed.push("z0".into());
    closed
}

/// Ring contours draw on their circles, all the way round, with the stroke
/// width measured in pixels.
#[test]
fn ring_contours_lie_on_their_circles() {
    let mut r = renderer();
    let ids = ring_field(&mut r);
    let chart = chart((0.0, 4.0), colorbar(0.0, 4.0));
    let series = [field(
        &ids,
        GridLayout::Centers,
        Shading::Interpolated,
        Some(contour(vec![1.5, 2.5], 2.0, None)),
        false,
    )];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("polar-heatmap-rings-{scale}x"), &img);
        let (centre, _, _) = geometry(&chart, scale);
        let rings = [radius_px(&chart, scale, 1.5), radius_px(&chart, scale, 2.5)];
        let half = f64::from(scale) + 1.5;
        let drawn = pixels_where(&img, is_line);
        assert!(drawn.len() > 200, "{scale}x: the rings draw");
        for p in &drawn {
            let (rr, _) = polar_of(centre, *p);
            assert!(
                rings.iter().any(|ring| (rr - ring).abs() <= half),
                "{scale}x: a contour pixel at {p:?} (r {rr:.2}) is off the rings {rings:?}"
            );
        }
        for k in 0..72 {
            let a = k as f64 * 5.0_f64.to_radians();
            for ring in rings {
                assert!(
                    any_within(&img, at(centre, a, ring), 1.0, is_line),
                    "{scale}x: ring {ring:.1} at {}°",
                    k * 5
                );
            }
        }
    }
}

/// A field whose value is its angle: a level is a spoke. Its stroke keeps
/// one width in pixels from near the centre to the ring, which is the
/// polar Jacobian at work (a constant `t` width would fan out).
#[test]
fn spoke_contours_keep_their_pixel_width() {
    let mut r = renderer();
    let ga = col((0..12).map(|i| i as f64 * 30.0).collect());
    let gr = col(vec![0.0, 4.0]);
    let zs: Vec<Column<f64>> = (0..12).map(|i| col(vec![i as f64 * 30.0; 2])).collect();
    let ids: Vec<String> = (0..12).map(|i| format!("z{i}")).collect();
    let mut batch: Vec<(&str, &dyn ColumnSource)> = vec![("ga", &ga), ("gr", &gr)];
    batch.extend(
        ids.iter()
            .zip(&zs)
            .map(|(id, c)| (id.as_str(), c as &dyn ColumnSource)),
    );
    r.add_columns(&batch).unwrap();
    let chart = chart((0.0, 4.0), colorbar(0.0, 330.0));
    let series = [field(
        &ids,
        GridLayout::Centers,
        Shading::Interpolated,
        Some(contour(vec![100.0], 3.0, None)),
        false,
    )];
    let img = export(&mut r, &chart, &series, 1.0);
    save("polar-heatmap-spoke", &img);
    let (centre, outer, _) = geometry(&chart, 1.0);
    let spoke = angle(&chart, 100.0);
    let drawn = pixels_where(&img, is_line);
    assert!(drawn.len() > 100, "the spoke draws");
    for p in &drawn {
        let (rr, phi) = polar_of(centre, *p);
        let off = angle_diff(phi, spoke).sin() * rr;
        assert!(
            off.abs() <= 2.5 && angle_diff(phi, spoke).abs() < 0.5 || rr < 3.0,
            "a spoke pixel at {p:?} is {off:.2} px off the spoke"
        );
    }
    // The width across the spoke, in pixels, near the centre and near the ring.
    let width_at = |rr: f64| {
        (-40..=40)
            .filter(|k| {
                let s = f64::from(*k) * 0.25;
                let (nx, ny) = (-spoke.sin(), spoke.cos());
                let base = at(centre, spoke, rr);
                is_line(pixel(&img, (base.0 + nx * s, base.1 - ny * s)))
            })
            .count() as f64
            * 0.25
    };
    let near = width_at(outer * 0.2);
    let far = width_at(outer * 0.9);
    assert!(
        near > 1.5 && far > 1.5,
        "the spoke has width: {near} and {far}"
    );
    assert!(
        (near - far).abs() <= 1.0,
        "the stroke keeps its pixel width: {near} near the centre, {far} near the ring"
    );
}

/// Contour labels sit on their rings: each label box's pixels lie within
/// its size of the level's circle, and the line leaves a gap under it.
#[test]
fn ring_contour_labels_sit_on_their_circles() {
    let mut r = renderer();
    let ids = ring_field(&mut r);
    let chart = chart((0.0, 4.0), colorbar(0.0, 4.0));
    let labels = ContourLabelConfig {
        visible: true,
        font_size: 14.0,
        color: Color::WHITE,
        format: LabelFormat::Decimal,
        significant_digits: 2,
        spacing_px: 90.0,
        anchors: Vec::new(),
        bg_color: Some(LABEL_BG),
        bg_padding_px: 2.0,
    };
    let series = [field(
        &ids,
        GridLayout::Centers,
        Shading::Interpolated,
        Some(contour(vec![1.5, 2.5], 2.0, Some(labels))),
        false,
    )];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("polar-heatmap-labels-{scale}x"), &img);
        let (centre, _, _) = geometry(&chart, scale);
        let rings = [radius_px(&chart, scale, 1.5), radius_px(&chart, scale, 2.5)];
        let boxes = pixels_where(&img, is_magenta);
        assert!(boxes.len() > 40, "{scale}x: labels are drawn");
        // Half the box's diagonal bounds how far a box pixel can sit from
        // the anchor on the circle.
        let reach = f64::from(scale) * 30.0;
        let mut near = [false; 2];
        for p in &boxes {
            let (rr, _) = polar_of(centre, *p);
            let nearest = rings
                .iter()
                .enumerate()
                .min_by(|a, b| (rr - a.1).abs().total_cmp(&(rr - b.1).abs()))
                .unwrap();
            assert!(
                (rr - nearest.1).abs() <= reach,
                "{scale}x: a label pixel at {p:?} (r {rr:.1}) is off the rings"
            );
            near[nearest.0] = true;
        }
        assert_eq!(near, [true, true], "{scale}x: both levels are labelled");
    }
}

/// The contour pick names the level under the cursor on a ring, and the
/// selected level's highlight runs along that circle only.
#[test]
fn a_ring_contour_picks_and_highlights_its_level() {
    let mut r = renderer();
    let ids = ring_field(&mut r);
    let mut chart = chart((0.0, 4.0), colorbar(0.0, 4.0));
    r.enable_gpu_picking().unwrap();
    let series = vec![field(
        &ids,
        GridLayout::Centers,
        Shading::Interpolated,
        Some(contour(vec![1.5, 2.5], 3.0, None)),
        false,
    )];
    let id = r
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let (centre, _, _) = geometry(&chart, 1.0);
    for (level, v) in [(0usize, 1.5), (1, 2.5)] {
        for a in [20.0, 135.0, 250.0] {
            let p = at(centre, angle(&chart, a), radius_px(&chart, 1.0, v));
            assert!(
                matches!(pick(&mut r, id, p), Some(PickedData::ContourLevel { level_index, .. }) if level_index == level),
                "level {level} at {a}°"
            );
        }
    }
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::ContourLevel {
            source_id: None,
            series_id: "field".into(),
            level_index: 1,
            x_index: 0,
            y_index: 0,
        }],
        highlight_color: MAGENTA,
        outline_width_px: 2.0,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 2.0,
    });
    let img = export(&mut r, &chart, &series, 1.0);
    save("polar-heatmap-contour-highlight", &img);
    let ring = radius_px(&chart, 1.0, 2.5);
    let drawn = pixels_where(&img, is_magenta);
    assert!(drawn.len() > 200, "the level is highlighted");
    for p in drawn {
        let (rr, _) = polar_of(centre, p);
        assert!(
            (rr - ring).abs() <= 4.5,
            "a highlight pixel at {p:?} (r {rr:.1}) is off the selected ring {ring:.1}"
        );
    }
}

/// A polar heatmap subplot draws its cells beside a rectangular parent.
#[test]
fn a_polar_heatmap_subplot_draws_its_cells() {
    let mut r = renderer();
    let ids = edges_grid(&mut r);
    let bar = colorbar(0.0, 31.0);
    let mut parent = renderer::default::default_config();
    parent.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    parent.legend.visible = false;
    let mut cell = chart((0.0, RINGS as f64), bar.clone()).config().clone();
    cell.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT,
        width: WIDTH,
        height: HEIGHT,
    });
    let sub_chart = Chart::new(cell.clone());
    parent.subplots.push(Subplot {
        config: cell,
        series: vec!["field".into()],
        link_x: None,
        link_y: None,
    });
    let doc = Chart::new(parent);
    let series = [field(&ids, GridLayout::Edges, Shading::Flat, None, true)];
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&doc, &series),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        save(&format!("polar-heatmap-subplot-{scale}x"), &img);
        for i in [0usize, 3, 6] {
            for j in [0usize, 3] {
                let p = at_value(&sub_chart, scale, (i as f64 + 0.5) * 45.0, j as f64 + 0.5);
                assert_colour(
                    &img,
                    p,
                    bar.color_for_z(z_cell(i, j)),
                    &format!("{scale}x subplot cell ({i}, {j})"),
                );
            }
        }
    }
}

/// On a polar heatmap a ratio-anchored annotation draws over the cells and
/// a data-anchored one is skipped with `NoValueAxis`, as on every polar
/// chart.
#[test]
fn annotations_on_a_polar_heatmap() {
    let mut r = renderer();
    let ids = edges_grid(&mut r);
    let mut chart = chart((0.0, RINGS as f64), colorbar(0.0, 31.0));
    let note = |anchor| {
        Annotation::new(AnnotationKind::Text {
            anchor,
            text: renderer::text::RichText::plain("note", Color::BLACK, 14.0, ""),
            box_style: Some(BoxStyle {
                fill: Color::WHITE,
                border_color: Color::BLACK,
                border_width: 1.0,
                padding: 6.0,
            }),
        })
    };
    chart.config_mut().annotations.items = vec![
        note(Anchor::Data { x: 10.0, y: 1.0 }),
        note(Anchor::Ratio { x: 0.5, y: 0.5 }),
    ];
    let series = [field(&ids, GridLayout::Edges, Shading::Flat, None, true)];
    // The panel export draws the base layer only; the document carries the
    // annotation layer.
    let img = r
        .export_rgba(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .unwrap();
    save("polar-heatmap-annotations", &img);
    let view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let map = view.annotation_map();
    assert_eq!(map.find(0), None);
    assert!(map.find(1).is_some());
    assert!(map.warnings().iter().any(|w| matches!(
        w,
        RenderWarning::AnnotationSkipped {
            index: 0,
            reason: AnnotationSkip::NoValueAxis
        }
    )));
    let (centre, _, _) = geometry(&chart, 1.0);
    assert!(
        any_within(&img, centre, 24.0, |p| p[3] > 200
            && p[0] > 240
            && p[1] > 240
            && p[2] > 240),
        "the note's white box near the centre"
    );
}

/// The rules that keep a polar field honest, each with its reason: a
/// hexagonal lattice, a polygon grid, an image, and contours drawing now.
#[test]
fn polar_field_refusals_name_their_reason() {
    let mut r = renderer();
    let ids = edges_grid(&mut r);
    let chart = chart((0.0, RINGS as f64), colorbar(0.0, 31.0));
    let reason = |r: &mut Renderer, config: &renderer::Config, s: SeriesConfig| match r
        .register_chart(config.clone(), vec![s])
    {
        Err(renderer::FiggyError::InvalidSeriesConfig { reason, .. }) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    };
    let hex = field(&ids, GridLayout::Hexagonal, Shading::Flat, None, true);
    assert_eq!(
        reason(&mut r, chart.config(), hex),
        "a hexagonal grid is drawn on a rectangular chart"
    );
    let mut polygon = chart.config().clone();
    polygon.bottom_x.categories = Some(
        (0..8)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect(),
    );
    polygon.polar_mut().unwrap().grid_shape = Some(GridShape::Polygon);
    assert_eq!(
        reason(
            &mut r,
            &polygon,
            field(&ids, GridLayout::Edges, Shading::Flat, None, true)
        ),
        "a polar heatmap's cells follow circular rings: not on a polygon grid"
    );
    let fill_and_contour = field(
        &ids,
        GridLayout::Edges,
        Shading::Flat,
        Some(contour(vec![3.0], 1.0, None)),
        true,
    );
    assert!(
        r.register_chart(chart.config().clone(), vec![fill_and_contour])
            .is_ok()
    );
}

/// Rule 10: 65,537 angle cells by 64 radius cells draw, and the pick
/// answers the last angle cell. The first 65,536 cells share the first
/// half turn; the last one spans the second, so a pixel lands in it.
#[test]
fn sixty_five_thousand_angle_cells_draw_and_pick_the_last() {
    let mut r = renderer();
    let n = 65_537usize;
    let rings = 64usize;
    let mut ga: Vec<f64> = (0..n).map(|i| i as f64 * 180.0 / (n - 1) as f64).collect();
    ga.push(360.0);
    let gr: Vec<f64> = (0..=rings).map(|j| j as f64).collect();
    let ga_col = col(ga);
    let gr_col = col(gr);
    r.add_columns(&[("ga", &ga_col as &dyn ColumnSource), ("gr", &gr_col)])
        .unwrap();
    let zs: Vec<Column<f64>> = (0..n).map(|i| col(vec![(i % 7) as f64; rings])).collect();
    let ids: Vec<String> = (0..n).map(|i| format!("z{i}")).collect();
    let batch: Vec<(&str, &dyn ColumnSource)> = ids
        .iter()
        .zip(&zs)
        .map(|(id, c)| (id.as_str(), c as &dyn ColumnSource))
        .collect();
    r.add_columns(&batch).unwrap();
    let bar = colorbar(0.0, 6.0);
    let chart = chart((0.0, rings as f64), bar.clone());
    let series = vec![field(&ids, GridLayout::Edges, Shading::Flat, None, true)];
    let started = std::time::Instant::now();
    let img = export(&mut r, &chart, &series, 1.0);
    eprintln!("65,537 x 64 polar cells drawn in {:?}", started.elapsed());
    save("polar-heatmap-65537", &img);
    let last = at_value(&chart, 1.0, 270.0, 32.5);
    assert_colour(
        &img,
        last,
        bar.color_for_z(((n - 1) % 7) as f64),
        "the last angle cell",
    );
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(chart.config().clone(), series).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(cell_of(pick(&mut r, id, last)), Some((n - 1, 32)));
    eprintln!("the last angle cell picked in {:?}", started.elapsed());
}

/// Rule 10: a 2,048 by 2,048 polar grid (4.2M cells) draws.
#[test]
fn a_four_million_cell_polar_grid_draws() {
    let mut r = renderer();
    let n = 2048usize;
    let ga = col((0..=n).map(|i| i as f64 * 360.0 / n as f64).collect());
    let gr = col((0..=n).map(|j| j as f64).collect());
    r.add_columns(&[("ga", &ga as &dyn ColumnSource), ("gr", &gr)])
        .unwrap();
    let zs: Vec<Column<f64>> = (0..n)
        .map(|i| col((0..n).map(|j| ((i + j) % 2) as f64).collect()))
        .collect();
    let ids: Vec<String> = (0..n).map(|i| format!("z{i}")).collect();
    let batch: Vec<(&str, &dyn ColumnSource)> = ids
        .iter()
        .zip(&zs)
        .map(|(id, c)| (id.as_str(), c as &dyn ColumnSource))
        .collect();
    r.add_columns(&batch).unwrap();
    let chart = chart((0.0, n as f64), colorbar(0.0, 1.0));
    let started = std::time::Instant::now();
    let img = export(
        &mut r,
        &chart,
        &[field(&ids, GridLayout::Edges, Shading::Flat, None, true)],
        1.0,
    );
    eprintln!("2048 x 2048 polar cells drawn in {:?}", started.elapsed());
    let (centre, outer, _) = geometry(&chart, 1.0);
    let painted = pixels_where(&img, |p| p[3] > 200).len() as f64;
    let disc = PI * outer * outer;
    assert!(
        painted > disc * 0.95 && painted < disc * 1.05,
        "the disc is filled: {painted} of {disc:.0}"
    );
    assert!(!is_empty(pixel(&img, at(centre, 1.0, outer * 0.5))));
}
