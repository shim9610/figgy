//! A hexagonal grid (`GridLayout::Hexagonal`, #134) through the public
//! renderer API: the headless export, the cell pick, the selection outline,
//! the fit and the registration rules.
//!
//! House pattern (mirrors `heatmap_render.rs`): each test builds its own
//! renderer on the shared device, and every assertion names the pixel a cell
//! lands on through the lattice the model documents (`x[c]`, `y[r]` raised by
//! half the row pitch in the odd columns), so a wrong centre, a wrong
//! neighbour or a cell painted past its Voronoi edge fails rather than passes.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::AxisScale;
use renderer::data::{Column, ColumnSource};
use renderer::data_config::{
    ContourConfig, DataLineStyleConfig, FieldFillConfig, FillMode, GridLayout, MatrixOrientation,
    MatrixRef, Shading,
};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::view3d::View3d;
use renderer::{
    Chart, Color, ColorBarOptions, ColorMap, DataRenderType, DataSelectionsConfig, GpuPickRequest,
    PickedData, PickedDataRef, RasterImage, Renderer, RendererDevice, SeriesConfig,
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

/// The lattice: five columns by four rows, column pitch 1, row pitch 2/√3,
/// so the flat-topped hexagons are regular with circumradius 2/3.
const COLS: usize = 5;
const ROWS: usize = 4;
const PX: f64 = 1.0;

fn py() -> f64 {
    2.0 / 3f64.sqrt()
}

fn xs() -> Vec<f64> {
    (0..COLS).map(|c| 1.0 + c as f64 * PX).collect()
}

fn ys() -> Vec<f64> {
    (0..ROWS).map(|r| 1.0 + r as f64 * py()).collect()
}

/// A distinct value per cell, so a cell painted in the wrong place shows the
/// wrong colour rather than passing.
fn z(c: usize, r: usize) -> f64 {
    (c * ROWS + r) as f64
}

/// The centre of column `c`, row `r`: the odd columns are raised by half
/// the row pitch.
fn center(c: i64, r: i64) -> (f64, f64) {
    (
        1.0 + c as f64 * PX,
        1.0 + r as f64 * py() + (c & 1) as f64 * py() / 2.0,
    )
}

/// The six neighbours of column `c`, row `r` on the flat-topped lattice.
fn neighbors(c: i64, r: i64) -> [(i64, i64); 6] {
    let up = c & 1;
    [
        (c, r - 1),
        (c, r + 1),
        (c - 1, r - 1 + up),
        (c - 1, r + up),
        (c + 1, r - 1 + up),
        (c + 1, r + up),
    ]
}

fn col_f64(data: Vec<f64>) -> Column<f64> {
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

fn renderer_with(budget: u64) -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Bgra8Unorm,
        budget,
    )
    .expect("renderer init")
}

fn renderer() -> Renderer {
    renderer_with(16 * 1024 * 1024)
}

/// The coordinate columns `hx`, `hy` and the grid `hz*`: one constituent
/// column per lattice column under `ColumnsAreX`, per row under
/// `ColumnsAreY`.
fn upload(r: &mut Renderer, orientation: &MatrixOrientation) -> Vec<String> {
    let x = col_f64(xs());
    let y = col_f64(ys());
    let (count, depth, columns_are_y) = match orientation {
        MatrixOrientation::ColumnsAreX => (COLS, ROWS, false),
        MatrixOrientation::ColumnsAreY => (ROWS, COLS, true),
    };
    let grid: Vec<Column<f64>> = (0..count)
        .map(|k| {
            col_f64(
                (0..depth)
                    .map(|i| if columns_are_y { z(i, k) } else { z(k, i) })
                    .collect(),
            )
        })
        .collect();
    let ids: Vec<String> = (0..count).map(|k| format!("hz{k}")).collect();
    let mut batch: Vec<(&str, &dyn ColumnSource)> = vec![("hx", &x), ("hy", &y)];
    for (id, column) in ids.iter().zip(&grid) {
        batch.push((id.as_str(), column));
    }
    r.add_columns(&batch).expect("grid upload");
    ids
}

fn colorbar() -> ColorBarOptions {
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Custom {
        stops: vec![LOW, HIGH],
    };
    bar.axis.min = 0.0;
    bar.axis.max = z(COLS - 1, ROWS - 1);
    bar.axis.major_spacing = 5.0;
    bar
}

/// Grid, legend and axes off; the lattice sits inside 0..6.5 on both axes.
fn chart() -> Chart {
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
    config.colorbar = Some(colorbar());
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 6.5);
    chart.set_y_range(0.0, 6.5);
    chart
}

fn hexbin(ids: &[String], orientation: MatrixOrientation) -> SeriesConfig {
    SeriesConfig {
        series_id: "hex".into(),
        source_id: None,
        z_column: None,
        label: None,
        x_column: "hx".into(),
        y_column: "hy".into(),
        render_type: DataRenderType::Heatmap {
            matrix: MatrixRef {
                columns: ids.to_vec(),
                orientation,
                grid_layout: GridLayout::Hexagonal,
            },
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            },
            surface: None,
        },
    }
}

/// Data (x, y) → export pixel at `scale`, through the chart's data area and
/// axis ranges.
fn data_to_px(chart: &Chart, x: f64, y: f64, scale: f64) -> (f64, f64) {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let tx = (x - cfg.bottom_x.min) / (cfg.bottom_x.max - cfg.bottom_x.min);
    let ty = (y - cfg.left_y.min) / (cfg.left_y.max - cfg.left_y.min);
    (
        (f64::from(da.x) + tx * f64::from(da.width)) * scale,
        (f64::from(da.y + da.height) - ty * f64::from(da.height)) * scale,
    )
}

/// Writes the image when `FIGGY_INTERACTION_ARTIFACTS` names a directory.
fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        let png = renderer::encode_png(image).unwrap();
        std::fs::write(format!("{path}/{name}.png"), png).unwrap();
    }
}

fn pixel(img: &RasterImage, (x, y): (f64, f64)) -> [u8; 4] {
    let x = (x.floor() as i64).clamp(0, i64::from(img.width) - 1) as u32;
    let y = (y.floor() as i64).clamp(0, i64::from(img.height) - 1) as u32;
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

/// The pixel is the colour the CPU ramp gives `want`, within the two
/// rounding steps (f32 ramp maths, 8-bit quantization).
fn assert_colour_at(img: &RasterImage, at: (f64, f64), want: Color, what: &str) {
    let got = pixel(img, at).map(i32::from);
    let expected = [to8(want.r), to8(want.g), to8(want.b), to8(want.a)];
    assert!(
        got.iter().zip(expected).all(|(g, e)| (*g - e).abs() <= 2),
        "{what} at px {at:?}: got {got:?}, expected {expected:?}"
    );
}

fn is_selection(p: [u8; 4]) -> bool {
    p[3] > 180 && p[0] > 180 && p[2] > 180 && p[1] < 80
}

/// The point `f` of the way from the centre of `(c, r)` to the centre of
/// `(nc, nr)`.
fn towards(c: i64, r: i64, (nc, nr): (i64, i64), f: f64) -> (f64, f64) {
    let (ax, ay) = center(c, r);
    let (bx, by) = center(nc, nr);
    (ax + f * (bx - ax), ay + f * (by - ay))
}

/// Every cell is its colour at its centre and up to its Voronoi edge with
/// each neighbour (0.45 of the way is still the cell, 0.55 is the
/// neighbour), and nothing is painted past the lattice.
#[test]
fn every_cell_is_painted_to_its_voronoi_edge() {
    let mut r = renderer();
    let ids = upload(&mut r, &MatrixOrientation::ColumnsAreX);
    let chart = chart();
    let bar = colorbar();
    let series = [hexbin(&ids, MatrixOrientation::ColumnsAreX)];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    save("hexbin-cells", &img);
    for c in 0..COLS {
        for row in 0..ROWS {
            let (x, y) = center(c as i64, row as i64);
            assert_colour_at(
                &img,
                data_to_px(&chart, x, y, 1.0),
                bar.color_for_z(z(c, row)),
                &format!("the centre of ({c}, {row})"),
            );
        }
    }
    // An interior cell against each of its six neighbours.
    let (c, row) = (2i64, 1i64);
    for (k, n) in neighbors(c, row).into_iter().enumerate() {
        let (x, y) = towards(c, row, n, 0.45);
        assert_colour_at(
            &img,
            data_to_px(&chart, x, y, 1.0),
            bar.color_for_z(z(c as usize, row as usize)),
            &format!("0.45 of the way to neighbour {k} {n:?} is still (2, 1)"),
        );
        let (x, y) = towards(c, row, n, 0.55);
        assert_colour_at(
            &img,
            data_to_px(&chart, x, y, 1.0),
            bar.color_for_z(z(n.0 as usize, n.1 as usize)),
            &format!("0.55 of the way to neighbour {k} {n:?} is the neighbour"),
        );
    }
    // Past the lattice: beyond the first column's left edge (the hexagon
    // reaches two thirds of the column pitch) nothing is painted.
    let outside = pixel(&img, data_to_px(&chart, 1.0 - 0.8, 1.0, 1.0));
    assert!(
        outside[3] < 40 || (outside[0] < 40 && outside[1] < 40),
        "the left of the lattice is unpainted, got {outside:?}"
    );
    // And a border cell is a whole hexagon: just inside its outer edge it
    // is painted.
    assert_colour_at(
        &img,
        data_to_px(&chart, 1.0 - 0.6, 1.0, 1.0),
        bar.color_for_z(z(0, 0)),
        "just inside the first column's outer edge",
    );
}

/// `ColumnsAreY` reads the same lattice transposed: the constituent columns
/// are the rows.
#[test]
fn columns_are_y_reads_the_grid_transposed() {
    let mut r = renderer();
    let ids = upload(&mut r, &MatrixOrientation::ColumnsAreY);
    let chart = chart();
    let bar = colorbar();
    let series = [hexbin(&ids, MatrixOrientation::ColumnsAreY)];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    for c in 0..COLS {
        for row in 0..ROWS {
            let (x, y) = center(c as i64, row as i64);
            assert_colour_at(
                &img,
                data_to_px(&chart, x, y, 1.0),
                bar.color_for_z(z(c, row)),
                &format!("the centre of ({c}, {row}) under ColumnsAreY"),
            );
        }
    }
}

/// The 2x export is the 1x lattice doubled.
#[test]
fn the_2x_export_doubles_the_lattice() {
    let mut r = renderer();
    let ids = upload(&mut r, &MatrixOrientation::ColumnsAreX);
    let chart = chart();
    let bar = colorbar();
    let series = [hexbin(&ids, MatrixOrientation::ColumnsAreX)];
    let img = r.export_panel_rgba(&chart, &series, 2.0).unwrap();
    assert_eq!((img.width, img.height), (WIDTH * 2, HEIGHT * 2));
    for (c, row) in [(0, 0), (1, 2), (4, 3)] {
        let (x, y) = center(c as i64, row as i64);
        assert_colour_at(
            &img,
            data_to_px(&chart, x, y, 2.0),
            bar.color_for_z(z(c, row)),
            &format!("the centre of ({c}, {row}) at 2x"),
        );
        let n = neighbors(c as i64, row as i64)[1];
        if (n.1 as usize) < ROWS {
            let (x, y) = towards(c as i64, row as i64, n, 0.55);
            assert_colour_at(
                &img,
                data_to_px(&chart, x, y, 2.0),
                bar.color_for_z(z(n.0 as usize, n.1 as usize)),
                &format!("past the top edge of ({c}, {row}) at 2x"),
            );
        }
    }
}

fn request(at: (f64, f64)) -> GpuPickRequest {
    GpuPickRequest {
        canvas_position_px: [at.0 as f32, at.1 as f32],
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

fn cell(picked: Option<PickedData>) -> Option<(usize, usize)> {
    match picked {
        Some(PickedData::MatrixCell {
            x_index, y_index, ..
        }) => Some((x_index, y_index)),
        None => None,
        other => panic!("picked {other:?}"),
    }
}

/// The pick answers the cell the fill painted: the centre, both sides of
/// each Voronoi edge of an interior cell, and nothing past the lattice.
#[test]
fn the_pick_answers_the_painted_cell() {
    let mut r = renderer();
    let ids = upload(&mut r, &MatrixOrientation::ColumnsAreX);
    let chart = chart();
    let series = vec![hexbin(&ids, MatrixOrientation::ColumnsAreX)];
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(chart.config().clone(), series).unwrap();
    let mut pick = |at: (f64, f64)| {
        cell(pollster::block_on(r.pick_chart_data(id, request(at)).unwrap().resolve()).unwrap())
    };
    for c in 0..COLS {
        for row in 0..ROWS {
            let (x, y) = center(c as i64, row as i64);
            assert_eq!(
                pick(data_to_px(&chart, x, y, 1.0)),
                Some((c, row)),
                "the centre of ({c}, {row})"
            );
        }
    }
    let (c, row) = (2i64, 1i64);
    for n in neighbors(c, row) {
        let (x, y) = towards(c, row, n, 0.45);
        assert_eq!(
            pick(data_to_px(&chart, x, y, 1.0)),
            Some((2, 1)),
            "0.45 to {n:?}"
        );
        let (x, y) = towards(c, row, n, 0.55);
        assert_eq!(
            pick(data_to_px(&chart, x, y, 1.0)),
            Some((n.0 as usize, n.1 as usize)),
            "0.55 to {n:?}"
        );
    }
    assert_eq!(
        pick(data_to_px(&chart, 0.2, 1.0, 1.0)),
        None,
        "past the lattice"
    );
    // The picked cell steps over the lattice as any matrix cell.
    let mut stepped = chart.config().clone();
    stepped.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::MatrixCell {
            source_id: None,
            series_id: "hex".into(),
            x_index: 2,
            y_index: 1,
        }],
        ..DataSelectionsConfig::default()
    });
    let series = vec![hexbin(&ids, MatrixOrientation::ColumnsAreX)];
    let len = |id: &str| match id {
        "hx" => Some(COLS),
        "hy" => Some(ROWS),
        _ => Some(ROWS),
    };
    assert_eq!(stepped.step_selection(&series, 1, 1, len), Ok(true));
    assert!(matches!(
        stepped.picked_data.as_ref().unwrap().refs[0],
        PickedDataRef::MatrixCell {
            x_index: 3,
            y_index: 2,
            ..
        }
    ));
}

/// The signed pixel distance from export pixel `p` to the bisector between
/// the centres `a` and `b` (positive on `a`'s side): the CPU twin of
/// `hex_edge_distance_px`, in the data metric mapped through the axes'
/// pixel scales.
fn bisector_distance_px(chart: &Chart, p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let sx = f64::from(da.width) / (cfg.bottom_x.max - cfg.bottom_x.min);
    let sy = -f64::from(da.height) / (cfg.left_y.max - cfg.left_y.min);
    let m = data_to_px(chart, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, 1.0);
    let normal = ((b.0 - a.0) / sx, (b.1 - a.1) / sy);
    let len = normal.0.hypot(normal.1);
    ((m.0 - p.0) * normal.0 + (m.1 - p.1) * normal.1) / len
}

/// The selected hexagon's outline runs along its six edges and nowhere
/// else: every highlight pixel lies inside the cell within the stroke width
/// of one of its bisectors.
#[test]
fn the_selected_hexagon_outlines_its_six_edges() {
    let mut r = renderer();
    let ids = upload(&mut r, &MatrixOrientation::ColumnsAreX);
    let mut chart = chart();
    let width_px = 3.0f32;
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::MatrixCell {
            source_id: None,
            series_id: "hex".into(),
            x_index: 2,
            y_index: 1,
        }],
        highlight_color: Color::new(1.0, 0.0, 1.0, 1.0),
        outline_width_px: width_px,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 0.0,
    });
    let series = [hexbin(&ids, MatrixOrientation::ColumnsAreX)];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    save("hexbin-selected", &img);
    let (c, row) = (2i64, 1i64);
    let a = center(c, row);
    let mut selected = 0usize;
    let mut near_each_edge = [0usize; 6];
    for y in 0..img.height {
        for x in 0..img.width {
            let p = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            if !is_selection(pixel(&img, p)) {
                continue;
            }
            selected += 1;
            let distances: Vec<f64> = neighbors(c, row)
                .iter()
                .map(|n| bisector_distance_px(&chart, p, a, center(n.0, n.1)))
                .collect();
            let nearest = distances.iter().copied().fold(f64::INFINITY, f64::min);
            // Inside the hexagon (every bisector distance non-negative, with
            // the antialiased fringe) and within the stroke of one edge.
            assert!(
                nearest >= -1.0 && nearest <= f64::from(width_px) + 1.0,
                "highlight pixel {p:?} is {nearest:.2} px from the nearest edge"
            );
            let k = distances
                .iter()
                .position(|d| (*d - nearest).abs() < 1e-9)
                .unwrap();
            near_each_edge[k] += 1;
        }
    }
    assert!(selected > 60, "the outline painted {selected} pixels");
    for (k, n) in near_each_edge.iter().enumerate() {
        assert!(*n >= 4, "edge {k} carries {n} of the outline's pixels");
    }
}

/// The fit covers every hexagon whole: a sixth of the column pitch past
/// the half-pitch bounds on each side, half a row pitch past the raised
/// columns at the top.
#[test]
fn the_fit_reaches_the_hexagons_overhang() {
    let mut r = renderer();
    let ids = upload(&mut r, &MatrixOrientation::ColumnsAreX);
    pollster::block_on(r.ensure_errorbar_extent_engine()).unwrap();
    let series = hexbin(&ids, MatrixOrientation::ColumnsAreX);
    let ticket = r
        .begin_series_fit_extent(&series)
        .unwrap()
        .expect("a field fit");
    let extent = pollster::block_on(ticket.resolve())
        .unwrap()
        .expect("a drawable extent");
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let (x0, x1) = (1.0 - PX / 2.0 - PX / 6.0, 5.0 + PX / 2.0 + PX / 6.0);
    let (y0, y1) = (1.0 - py() / 2.0, 1.0 + 3.0 * py() + py());
    assert!(
        close(extent.x.min, x0) && close(extent.x.max, x1),
        "x extent {:?} vs [{x0}, {x1}]",
        extent.x
    );
    assert!(
        close(extent.y.min, y0) && close(extent.y.max, y1),
        "y extent {:?} vs [{y0}, {y1}]",
        extent.y
    );
}

/// Registration names what a hexagonal grid cannot read: coordinates that
/// are not uniform or too few, a contour, an interpolated fill, a
/// logarithmic axis and a 3D view.
#[test]
fn registration_rejects_what_a_hexagonal_grid_cannot_read() {
    let mut r = renderer();
    let ids = upload(&mut r, &MatrixOrientation::ColumnsAreX);
    r.add_columns(&[
        (
            "uneven",
            &col_f64(vec![1.0, 2.0, 3.0, 4.0, 5.5]) as &dyn ColumnSource,
        ),
        ("lone", &col_f64(vec![1.0]) as &dyn ColumnSource),
    ])
    .unwrap();
    let chart = chart();
    let reason = |r: &mut Renderer, config: &renderer::Config, series: SeriesConfig| match r
        .register_chart(config.clone(), vec![series])
    {
        Err(renderer::FiggyError::InvalidSeriesConfig { reason, .. }) => reason,
        other => panic!("registered: {other:?}"),
    };
    let mut uneven = hexbin(&ids, MatrixOrientation::ColumnsAreX);
    uneven.x_column = "uneven".into();
    assert!(
        reason(&mut r, chart.config(), uneven).contains("x coordinates (\"uneven\") are uniform"),
    );
    let mut lone = hexbin(&ids, MatrixOrientation::ColumnsAreX);
    lone.y_column = "lone".into();
    assert!(reason(&mut r, chart.config(), lone).contains("y coordinates (\"lone\") are uniform"));
    let mut interpolated = hexbin(&ids, MatrixOrientation::ColumnsAreX);
    if let DataRenderType::Heatmap { fill, .. } = &mut interpolated.render_type {
        fill.shading = Shading::Interpolated;
    }
    assert_eq!(
        reason(&mut r, chart.config(), interpolated),
        "a hexagonal grid is shaded flat"
    );
    let contour = SeriesConfig {
        render_type: DataRenderType::HeatmapContour {
            matrix: MatrixRef {
                columns: ids.clone(),
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Hexagonal,
            },
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            },
            contour: ContourConfig {
                levels: vec![5.0],
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                },
                per_level_color: None,
                labels: None,
            },
            surface: None,
        },
        ..hexbin(&ids, MatrixOrientation::ColumnsAreX)
    };
    assert_eq!(
        reason(&mut r, chart.config(), contour),
        "contours read a rectangular lattice: a hexagonal grid draws as a Heatmap only"
    );
    let mut log = chart.config().clone();
    log.bottom_x.scale = AxisScale::Logarithmic;
    log.bottom_x.min = 0.5;
    assert_eq!(
        reason(&mut r, &log, hexbin(&ids, MatrixOrientation::ColumnsAreX)),
        "a hexagonal grid needs linear x and y axes: its lattice is uniform"
    );
    let mut three_d = chart.config().clone();
    three_d.view_3d = Some(View3d::default());
    assert_eq!(
        reason(
            &mut r,
            &three_d,
            hexbin(&ids, MatrixOrientation::ColumnsAreX)
        ),
        "a hexagonal grid draws flat: not on a 3D chart"
    );
    // The well-formed grid registers.
    r.register_chart(
        chart.config().clone(),
        vec![hexbin(&ids, MatrixOrientation::ColumnsAreX)],
    )
    .unwrap();
}

/// Rule 10: a lattice of 2050 columns by 2049 rows (past `1 << 22` cells)
/// registers and draws; the lookup is the same nearest-centre search
/// whatever the count.
#[test]
fn a_four_million_cell_lattice_draws() {
    let mut r = renderer_with(256 * 1024 * 1024);
    let (cols, rows) = (2050usize, 2049usize);
    let x = col_f64((0..cols).map(|c| c as f64).collect());
    let y = col_f64((0..rows).map(|r| r as f64 * py()).collect());
    let grid: Vec<Column<f64>> = (0..cols)
        .map(|c| col_f64((0..rows).map(|r| ((c * 7 + r * 3) % 20) as f64).collect()))
        .collect();
    let ids: Vec<String> = (0..cols).map(|c| format!("big{c}")).collect();
    let mut batch: Vec<(&str, &dyn ColumnSource)> = vec![("bigx", &x), ("bigy", &y)];
    for (id, column) in ids.iter().zip(&grid) {
        batch.push((id.as_str(), column));
    }
    r.add_columns(&batch)
        .expect("a 4.2-million-cell grid uploads");
    let mut chart = chart();
    chart.set_x_range(-1.0, cols as f64);
    chart.set_y_range(-1.0, rows as f64 * py());
    let mut series = hexbin(&ids, MatrixOrientation::ColumnsAreX);
    series.x_column = "bigx".into();
    series.y_column = "bigy".into();
    let started = std::time::Instant::now();
    let img = r.export_panel_rgba(&chart, &[series], 1.0).unwrap();
    eprintln!(
        "{cols} x {rows} hexagonal cells exported in {:?}",
        started.elapsed()
    );
    // A pixel in the middle of the lattice is painted some ramp colour: the
    // cells are far smaller than a pixel, so the exact cell is the lookup's.
    let p = pixel(
        &img,
        data_to_px(&chart, cols as f64 / 2.0, rows as f64 * py() / 2.0, 1.0),
    );
    assert!(
        p[3] > 200 && (p[0] > 40 || p[1] > 40),
        "the lattice paints, got {p:?}"
    );
}
