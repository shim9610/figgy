//! A colour image (`DataRenderType::RgbImage`, #139) through the public
//! renderer API: the headless export, the cell pick, the selection outline
//! and the registration rules.
//!
//! House pattern (mirrors `hexbin_render.rs`): each test builds its own
//! renderer on the shared device, and every assertion names the pixel a cell
//! lands on through the lattice the model documents (`Centers`: the cell of
//! column `c`, row `r` is the half-pitch box around `(x[c], y[r])`), so a
//! channel read from the wrong table, a wrong neighbour or a cell painted
//! past its edge fails rather than passes.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::data::{Column, ColumnSource};
use renderer::data_config::{
    FieldFillConfig, FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
};
use renderer::layout::{ChartArea, Rect};
use renderer::view3d::View3d;
use renderer::{
    Chart, Color, DataRenderType, DataSelectionsConfig, FiggyError, GpuPickRequest, PickedData,
    PickedDataRef, RasterImage, Renderer, RendererDevice, SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;

/// The lattice: four columns by three rows of unit cells centred at
/// `x = 1..=4`, `y = 1..=3`.
const COLS: usize = 4;
const ROWS: usize = 3;

/// The grey the byte test maps: `128` of `[0, 255]`.
const GREY: f64 = 128.0 / 255.0;

/// The test card, one colour per cell. The first row is the three pure
/// channels and a grey; the second the three secondaries and white; the
/// third black, two mixed cells and a cell whose green is missing.
fn rgb(c: usize, r: usize) -> [f64; 3] {
    match (c, r) {
        (0, 0) => [1.0, 0.0, 0.0],
        (1, 0) => [0.0, 1.0, 0.0],
        (2, 0) => [0.0, 0.0, 1.0],
        (3, 0) => [GREY, GREY, GREY],
        (0, 1) => [1.0, 1.0, 0.0],
        (1, 1) => [0.0, 1.0, 1.0],
        (2, 1) => [1.0, 0.0, 1.0],
        (3, 1) => [1.0, 1.0, 1.0],
        (0, 2) => [0.0, 0.0, 0.0],
        (1, 2) => [0.25, 0.5, 0.75],
        (2, 2) => [0.75, 0.5, 0.25],
        (3, 2) => [0.5, f64::NAN, 0.5],
        _ => unreachable!("off the card"),
    }
}

/// The cell whose green channel is missing: unpainted and not picked.
const MISSING: (usize, usize) = (3, 2);

fn center(c: usize, r: usize) -> (f64, f64) {
    (1.0 + c as f64, 1.0 + r as f64)
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

/// The three channel grids' column ids, red, green and blue.
type ChannelIds = [Vec<String>; 3];

/// Uploads the coordinate columns `{prefix}_x`, `{prefix}_y` and the three
/// channel grids `{prefix}_r*`, `{prefix}_g*`, `{prefix}_b*`, every channel
/// value times `scale`: one constituent column per lattice column under
/// `ColumnsAreX`, per row under `ColumnsAreY`.
fn upload(
    r: &mut Renderer,
    prefix: &str,
    orientation: &MatrixOrientation,
    scale: f64,
) -> ChannelIds {
    let x = col_f64((0..COLS).map(|c| center(c, 0).0).collect());
    let y = col_f64((0..ROWS).map(|r| center(0, r).1).collect());
    let (count, depth, columns_are_y) = match orientation {
        MatrixOrientation::ColumnsAreX => (COLS, ROWS, false),
        MatrixOrientation::ColumnsAreY => (ROWS, COLS, true),
    };
    let channel = |k: usize| -> Vec<Column<f64>> {
        (0..count)
            .map(|n| {
                col_f64(
                    (0..depth)
                        .map(|i| {
                            let (c, r) = if columns_are_y { (i, n) } else { (n, i) };
                            rgb(c, r)[k] * scale
                        })
                        .collect(),
                )
            })
            .collect()
    };
    let grids = [channel(0), channel(1), channel(2)];
    let ids: ChannelIds = ["r", "g", "b"].map(|ch| {
        (0..count)
            .map(|n| format!("{prefix}_{ch}{n}"))
            .collect::<Vec<String>>()
    });
    let x_id = format!("{prefix}_x");
    let y_id = format!("{prefix}_y");
    let mut batch: Vec<(&str, &dyn ColumnSource)> = vec![(&x_id, &x), (&y_id, &y)];
    for (ids, grid) in ids.iter().zip(&grids) {
        for (id, column) in ids.iter().zip(grid) {
            batch.push((id.as_str(), column));
        }
    }
    r.add_columns(&batch).expect("image upload");
    ids
}

fn matrix(columns: &[String], orientation: MatrixOrientation) -> MatrixRef {
    MatrixRef {
        columns: columns.to_vec(),
        orientation,
        grid_layout: GridLayout::Centers,
    }
}

/// The image series over `{prefix}_x`, `{prefix}_y`.
fn image(
    prefix: &str,
    ids: &ChannelIds,
    orientation: MatrixOrientation,
    range: [f64; 2],
    shading: Shading,
) -> SeriesConfig {
    SeriesConfig {
        series_id: "img".into(),
        source_id: None,
        z_column: None,
        label: None,
        x_column: format!("{prefix}_x"),
        y_column: format!("{prefix}_y"),
        render_type: DataRenderType::RgbImage {
            red: matrix(&ids[0], orientation.clone()),
            green: matrix(&ids[1], orientation.clone()),
            blue: matrix(&ids[2], orientation),
            range,
            shading,
            opacity: 1.0,
        },
    }
}

fn flat(prefix: &str, ids: &ChannelIds) -> SeriesConfig {
    image(
        prefix,
        ids,
        MatrixOrientation::ColumnsAreX,
        [0.0, 1.0],
        Shading::Flat,
    )
}

/// Grid, legend and axes off, no colourbar; the lattice's cells sit inside
/// 0.5..4.5 by 0.5..3.5, with room past them on every side.
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
    config.colorbar = None;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 5.5);
    chart.set_y_range(0.0, 4.0);
    chart
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

/// The data point an export pixel's centre maps to at `scale`: the inverse
/// of `data_to_px`, for an assertion about the pixel the export sampled.
fn px_center_to_data(chart: &Chart, at: (f64, f64), scale: f64) -> (f64, f64) {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let (px, py) = (at.0.floor() + 0.5, at.1.floor() + 0.5);
    let tx = (px / scale - f64::from(da.x)) / f64::from(da.width);
    let ty = (f64::from(da.y + da.height) - py / scale) / f64::from(da.height);
    (
        cfg.bottom_x.min + tx * (cfg.bottom_x.max - cfg.bottom_x.min),
        cfg.left_y.min + ty * (cfg.left_y.max - cfg.left_y.min),
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

/// The colour a cell's channels map to through `range`: each channel
/// linearly from `range` to 0..1, clamped, at full opacity.
fn mapped(channels: [f64; 3], scale: f64, range: [f64; 2]) -> Color {
    let t = |v: f64| (((v * scale) - range[0]) / (range[1] - range[0])).clamp(0.0, 1.0) as f32;
    Color::new(t(channels[0]), t(channels[1]), t(channels[2]), 1.0)
}

/// The pixel is `want` within the two rounding steps (f32 channel maths,
/// 8-bit quantization).
fn assert_colour_at(img: &RasterImage, at: (f64, f64), want: Color, what: &str) {
    let got = pixel(img, at).map(i32::from);
    let expected = [to8(want.r), to8(want.g), to8(want.b), to8(want.a)];
    assert!(
        got.iter().zip(expected).all(|(g, e)| (*g - e).abs() <= 2),
        "{what} at px {at:?}: got {got:?}, expected {expected:?}"
    );
}

/// The highlight is orange, a colour the card has nowhere (its magenta
/// cell would read as the usual magenta highlight).
const HIGHLIGHT: Color = Color {
    r: 1.0,
    g: 0.5,
    b: 0.0,
    a: 1.0,
};

fn is_selection(p: [u8; 4]) -> bool {
    p[3] > 180 && p[0] > 180 && (100..=155).contains(&p[1]) && p[2] < 50
}

/// Every cell is its channels' colour at its centre and up to its edge
/// (0.45 of the way to a neighbour is still the cell, 0.55 the neighbour), a
/// cell with a missing channel is the background, and nothing is painted
/// past the lattice, at 1x and 2x.
#[test]
fn cells_paint_their_channels_at_1x_and_2x() {
    let mut r = renderer();
    let ids = upload(&mut r, "img", &MatrixOrientation::ColumnsAreX, 1.0);
    let chart = chart();
    let series = [flat("img", &ids)];
    for scale in [1.0f64, 2.0] {
        let img = r.export_panel_rgba(&chart, &series, scale as f32).unwrap();
        save(&format!("image-cells-{scale}x"), &img);
        assert_eq!(
            (img.width, img.height),
            (
                (WIDTH as f64 * scale) as u32,
                (HEIGHT as f64 * scale) as u32
            )
        );
        // Past the lattice's left edge (the first column reaches x = 0.5)
        // nothing is painted: the background shows.
        let background = pixel(&img, data_to_px(&chart, 0.3, 1.0, scale));
        for c in 0..COLS {
            for row in 0..ROWS {
                let (x, y) = center(c, row);
                let at = data_to_px(&chart, x, y, scale);
                if (c, row) == MISSING {
                    assert_eq!(
                        pixel(&img, at),
                        background,
                        "the cell with a missing channel is unpainted at {scale}x"
                    );
                    continue;
                }
                let want = mapped(rgb(c, row), 1.0, [0.0, 1.0]);
                assert_colour_at(
                    &img,
                    at,
                    want,
                    &format!("the centre of ({c}, {row}) at {scale}x"),
                );
                assert_ne!(
                    pixel(&img, at),
                    background,
                    "({c}, {row}) is painted over the background at {scale}x"
                );
            }
        }
        // The edge between the red and the green cell is at x = 1.5.
        assert_colour_at(
            &img,
            data_to_px(&chart, 1.45, 1.0, scale),
            mapped(rgb(0, 0), 1.0, [0.0, 1.0]),
            &format!("0.45 of the way to the green cell at {scale}x"),
        );
        assert_colour_at(
            &img,
            data_to_px(&chart, 1.55, 1.0, scale),
            mapped(rgb(1, 0), 1.0, [0.0, 1.0]),
            &format!("0.55 of the way to the green cell at {scale}x"),
        );
        // And the edge between the first and the second row is at y = 1.5.
        assert_colour_at(
            &img,
            data_to_px(&chart, 1.0, 1.45, scale),
            mapped(rgb(0, 0), 1.0, [0.0, 1.0]),
            &format!("0.45 of the way up to the yellow cell at {scale}x"),
        );
        assert_colour_at(
            &img,
            data_to_px(&chart, 1.0, 1.55, scale),
            mapped(rgb(0, 1), 1.0, [0.0, 1.0]),
            &format!("0.55 of the way up to the yellow cell at {scale}x"),
        );
        // Past the lattice on the other three sides too.
        for (x, y) in [(4.7, 2.0), (2.0, 0.3), (2.0, 3.7)] {
            assert_eq!(
                pixel(&img, data_to_px(&chart, x, y, scale)),
                background,
                "({x}, {y}) is past the lattice at {scale}x"
            );
        }
    }
}

/// A `[0, 255]` range reads bytes: `255` is full, `128` is the mid grey
/// `128 / 255`, and a value past the range clamps.
#[test]
fn a_byte_range_maps_128_to_mid_grey() {
    let mut r = renderer();
    let ids = upload(&mut r, "byte", &MatrixOrientation::ColumnsAreX, 255.0);
    let chart = chart();
    let range = [0.0, 255.0];
    let series = [image(
        "byte",
        &ids,
        MatrixOrientation::ColumnsAreX,
        range,
        Shading::Flat,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    save("image-bytes", &img);
    let (x, y) = center(3, 0);
    let grey = pixel(&img, data_to_px(&chart, x, y, 1.0));
    assert_eq!(grey, [128, 128, 128, 255], "128 of 255 is the mid grey");
    for (c, row) in [(0, 0), (1, 0), (2, 0), (3, 1), (1, 2)] {
        let (x, y) = center(c, row);
        assert_colour_at(
            &img,
            data_to_px(&chart, x, y, 1.0),
            mapped(rgb(c, row), 255.0, range),
            &format!("({c}, {row}) through the byte range"),
        );
    }
    // A narrower range clamps: with `[0, 127.5]` the grey cell is full
    // white and the quarter-mixed cell's blue (191.25) saturates.
    let series = [image(
        "byte",
        &ids,
        MatrixOrientation::ColumnsAreX,
        [0.0, 127.5],
        Shading::Flat,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert_colour_at(
        &img,
        data_to_px(&chart, x, y, 1.0),
        Color::new(1.0, 1.0, 1.0, 1.0),
        "128 past a range ending at 127.5 clamps to full",
    );
    let (x, y) = center(1, 2);
    assert_colour_at(
        &img,
        data_to_px(&chart, x, y, 1.0),
        Color::new(0.5, 1.0, 1.0, 1.0),
        "(63.75, 127.5, 191.25) over [0, 127.5]",
    );
}

/// The card bilinearly interpolated at data `(x, y)` between the four
/// surrounding centres: the CPU twin of the shader's per-channel blend.
fn bilinear(x: f64, y: f64) -> Color {
    let (fx, fy) = (x - 1.0, y - 1.0);
    let (c0, r0) = (fx.floor() as usize, fy.floor() as usize);
    let (fc, fr) = (fx - fx.floor(), fy - fy.floor());
    let at = |c: usize, r: usize, k: usize| rgb(c, r)[k];
    let channel = |k: usize| {
        let low = at(c0, r0, k) + (at(c0 + 1, r0, k) - at(c0, r0, k)) * fc;
        let high = at(c0, r0 + 1, k) + (at(c0 + 1, r0 + 1, k) - at(c0, r0 + 1, k)) * fc;
        (low + (high - low) * fr) as f32
    };
    Color::new(channel(0), channel(1), channel(2), 1.0)
}

/// `Interpolated` blends each channel between the sample points: halfway
/// between the red and the green centre the colour is half red, half green,
/// and a quad touching the cell with a missing channel is unpainted.
#[test]
fn interpolated_shading_blends_each_channel_between_sample_points() {
    let mut r = renderer();
    let ids = upload(&mut r, "img", &MatrixOrientation::ColumnsAreX, 1.0);
    let chart = chart();
    let series = [image(
        "img",
        &ids,
        MatrixOrientation::ColumnsAreX,
        [0.0, 1.0],
        Shading::Interpolated,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    save("image-interpolated", &img);
    // Interior points of the first two quads (red, green, yellow, cyan;
    // then green, blue, cyan, magenta) and the quad between the mixed cells.
    for (x, y) in [
        (1.5, 1.25),
        (1.5, 2.0),
        (1.25, 1.5),
        (1.5, 1.5),
        (2.5, 1.5),
        (2.75, 1.9),
        (2.5, 2.5),
        (1.9, 2.9),
    ] {
        // The export samples the pixel's centre, so the blend is read there.
        let at = data_to_px(&chart, x, y, 1.0);
        let (sx, sy) = px_center_to_data(&chart, at, 1.0);
        assert_colour_at(
            &img,
            at,
            bilinear(sx, sy),
            &format!("the blend at ({x}, {y})"),
        );
    }
    // It is a ramp, not two flat halves: red falls monotonically from the
    // red centre to the green one.
    let red_at = |x: f64| pixel(&img, data_to_px(&chart, x, 1.25, 1.0))[0];
    let samples: Vec<u8> = [1.1f64, 1.3, 1.5, 1.7, 1.9]
        .iter()
        .map(|&x| red_at(x))
        .collect();
    assert!(
        samples.windows(2).all(|w| w[0] > w[1]),
        "red falls across the quad: {samples:?}"
    );
    // The quad between (2, 2) and the cell with the missing green is
    // unpainted; the quad on its other side paints.
    let background = pixel(&img, data_to_px(&chart, 0.3, 1.0, 1.0));
    assert_eq!(
        pixel(&img, data_to_px(&chart, 3.5, 2.9, 1.0)),
        background,
        "a quad touching the missing channel is unpainted"
    );
    assert_ne!(
        pixel(&img, data_to_px(&chart, 2.5, 2.9, 1.0)),
        background,
        "the quad between the two mixed cells paints"
    );
}

/// `ColumnsAreY` reads the same lattice transposed: the constituent columns
/// are the rows.
#[test]
fn columns_are_y_reads_the_grid_transposed() {
    let mut r = renderer();
    let ids = upload(&mut r, "img", &MatrixOrientation::ColumnsAreY, 1.0);
    let chart = chart();
    let series = [image(
        "img",
        &ids,
        MatrixOrientation::ColumnsAreY,
        [0.0, 1.0],
        Shading::Flat,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let background = pixel(&img, data_to_px(&chart, 0.3, 1.0, 1.0));
    for c in 0..COLS {
        for row in 0..ROWS {
            let (x, y) = center(c, row);
            let at = data_to_px(&chart, x, y, 1.0);
            if (c, row) == MISSING {
                assert_eq!(
                    pixel(&img, at),
                    background,
                    "the missing cell under ColumnsAreY"
                );
                continue;
            }
            assert_colour_at(
                &img,
                at,
                mapped(rgb(c, row), 1.0, [0.0, 1.0]),
                &format!("the centre of ({c}, {row}) under ColumnsAreY"),
            );
        }
    }
}

/// A `Hexagonal` lattice (#134) carries an image too: each hexagon, centred
/// at `(x[c], y[r] + (c mod 2) · py / 2)`, is its channels' colour, through
/// the one cell lookup the heatmap uses.
#[test]
fn a_hexagonal_lattice_paints_its_centres() {
    let mut r = renderer();
    let ids = upload(&mut r, "img", &MatrixOrientation::ColumnsAreX, 1.0);
    let chart = chart();
    let mut series = flat("img", &ids);
    if let DataRenderType::RgbImage {
        red, green, blue, ..
    } = &mut series.render_type
    {
        for channel in [red, green, blue] {
            channel.grid_layout = GridLayout::Hexagonal;
        }
    }
    let img = r.export_panel_rgba(&chart, &[series], 1.0).unwrap();
    save("image-hexagonal", &img);
    let background = pixel(&img, data_to_px(&chart, 0.1, 1.0, 1.0));
    for c in 0..COLS {
        for row in 0..ROWS {
            let (x, y) = center(c, row);
            let y = y + (c % 2) as f64 * 0.5;
            let at = data_to_px(&chart, x, y, 1.0);
            if (c, row) == MISSING {
                assert_eq!(pixel(&img, at), background, "the missing hexagon");
                continue;
            }
            assert_colour_at(
                &img,
                at,
                mapped(rgb(c, row), 1.0, [0.0, 1.0]),
                &format!("the raised centre of ({c}, {row})"),
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

/// The pick answers the cell the fill painted (the centre, both sides of a
/// cell edge), nothing at the cell with a missing channel or past the
/// lattice, and the picked cell steps over the grid as any matrix cell.
#[test]
fn the_pick_answers_the_cell_and_the_selection_steps_the_grid() {
    let mut r = renderer();
    let ids = upload(&mut r, "img", &MatrixOrientation::ColumnsAreX, 1.0);
    let chart = chart();
    let series = vec![flat("img", &ids)];
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(chart.config().clone(), series).unwrap();
    let mut pick = |at: (f64, f64)| {
        cell(pollster::block_on(r.pick_chart_data(id, request(at)).unwrap().resolve()).unwrap())
    };
    for c in 0..COLS {
        for row in 0..ROWS {
            let (x, y) = center(c, row);
            let want = if (c, row) == MISSING {
                None
            } else {
                Some((c, row))
            };
            assert_eq!(
                pick(data_to_px(&chart, x, y, 1.0)),
                want,
                "the centre of ({c}, {row})"
            );
        }
    }
    assert_eq!(
        pick(data_to_px(&chart, 1.45, 1.0, 1.0)),
        Some((0, 0)),
        "0.45 to green"
    );
    assert_eq!(
        pick(data_to_px(&chart, 1.55, 1.0, 1.0)),
        Some((1, 0)),
        "0.55 to green"
    );
    assert_eq!(
        pick(data_to_px(&chart, 0.3, 1.0, 1.0)),
        None,
        "past the lattice"
    );
    assert_eq!(
        pick(data_to_px(&chart, 2.0, 3.7, 1.0)),
        None,
        "above the lattice"
    );
    // The picked cell steps over the grid.
    let mut stepped = chart.config().clone();
    stepped.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::MatrixCell {
            source_id: None,
            series_id: "img".into(),
            x_index: 1,
            y_index: 1,
        }],
        ..DataSelectionsConfig::default()
    });
    let series = vec![flat("img", &ids)];
    let len = |id: &str| match id {
        "img_x" => Some(COLS),
        "img_y" => Some(ROWS),
        _ => Some(ROWS),
    };
    assert_eq!(stepped.step_selection(&series, 1, 1, len), Ok(true));
    assert!(matches!(
        stepped.picked_data.as_ref().unwrap().refs[0],
        PickedDataRef::MatrixCell {
            x_index: 2,
            y_index: 2,
            ..
        }
    ));
}

/// The selected cell is outlined along its four edges, `outline_width_px`
/// inside them, and nowhere else.
#[test]
fn the_picked_highlight_outlines_the_cell() {
    let mut r = renderer();
    let ids = upload(&mut r, "img", &MatrixOrientation::ColumnsAreX, 1.0);
    let mut chart = chart();
    let width_px = 3.0f32;
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::MatrixCell {
            source_id: None,
            series_id: "img".into(),
            x_index: 1,
            y_index: 1,
        }],
        highlight_color: HIGHLIGHT,
        outline_width_px: width_px,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 0.0,
    });
    let series = [flat("img", &ids)];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    save("image-selected", &img);
    // The cell (1, 1) spans 1.5..2.5 in both axes.
    let (left, top) = data_to_px(&chart, 1.5, 2.5, 1.0);
    let (right, bottom) = data_to_px(&chart, 2.5, 1.5, 1.0);
    let mut selected = 0usize;
    let mut near_each_edge = [0usize; 4];
    for y in 0..img.height {
        for x in 0..img.width {
            let p = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            if !is_selection(pixel(&img, p)) {
                continue;
            }
            selected += 1;
            // Inside the cell (with the antialiased fringe) and within the
            // stroke of one edge.
            let distances = [p.0 - left, right - p.0, p.1 - top, bottom - p.1];
            let nearest = distances.iter().copied().fold(f64::INFINITY, f64::min);
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
    // The cell's own colour still shows inside the outline.
    let (x, y) = center(1, 1);
    assert_colour_at(
        &img,
        data_to_px(&chart, x, y, 1.0),
        mapped(rgb(1, 1), 1.0, [0.0, 1.0]),
        "the selected cell's centre",
    );
}

/// Registration names what an image cannot read: channels on different
/// lattices, an empty or non-finite range, an opacity past 1, a 3D view, a
/// polar chart, a streamed source and an unregistered channel column; it
/// needs no colourbar where a heatmap does.
#[test]
fn registration_names_what_an_image_cannot_read() {
    let mut r = renderer();
    let ids = upload(&mut r, "img", &MatrixOrientation::ColumnsAreX, 1.0);
    let chart = chart();
    let reason = |r: &mut Renderer, config: &renderer::Config, series: SeriesConfig| {
        let what = format!("{:?}", series.render_type);
        match r.register_chart(config.clone(), vec![series]) {
            Err(FiggyError::InvalidSeriesConfig { reason, .. }) => reason,
            other => panic!("registered {what}: {other:?}"),
        }
    };
    let mut short = flat("img", &ids);
    if let DataRenderType::RgbImage { green, .. } = &mut short.render_type {
        green.columns.truncate(COLS - 1);
    }
    assert_eq!(
        reason(&mut r, chart.config(), short),
        "an image's channels share one lattice: the same number of columns"
    );
    let mut turned = flat("img", &ids);
    if let DataRenderType::RgbImage { blue, .. } = &mut turned.render_type {
        blue.orientation = MatrixOrientation::ColumnsAreY;
    }
    assert_eq!(
        reason(&mut r, chart.config(), turned),
        "an image's channels share one lattice: the same orientation and grid layout"
    );
    let mut hexagonal = flat("img", &ids);
    if let DataRenderType::RgbImage { red, .. } = &mut hexagonal.render_type {
        red.grid_layout = GridLayout::Hexagonal;
    }
    assert_eq!(
        reason(&mut r, chart.config(), hexagonal),
        "an image's channels share one lattice: the same orientation and grid layout"
    );
    for range in [
        [1.0, 1.0],
        [2.0, 1.0],
        [0.0, f64::INFINITY],
        [f64::NAN, 1.0],
    ] {
        let bad = image(
            "img",
            &ids,
            MatrixOrientation::ColumnsAreX,
            range,
            Shading::Flat,
        );
        assert_eq!(
            reason(&mut r, chart.config(), bad),
            "an image's range is finite with a smaller low end",
            "range {range:?}"
        );
    }
    let mut opaque = flat("img", &ids);
    if let DataRenderType::RgbImage { opacity, .. } = &mut opaque.render_type {
        *opacity = 1.5;
    }
    assert_eq!(
        reason(&mut r, chart.config(), opaque),
        "an image's opacity is 0..=1"
    );
    let mut three_d = chart.config().clone();
    three_d.view_3d = Some(View3d::default());
    assert_eq!(
        reason(&mut r, &three_d, flat("img", &ids)),
        "an image draws flat: not on a 3D chart"
    );
    let mut polar = chart.config().clone();
    polar.polar = Some(renderer::polar::PolarOptions::default());
    assert_eq!(
        reason(&mut r, &polar, flat("img", &ids)),
        "field and contour series are not drawn on polar axes"
    );
    let mut unregistered = flat("img", &ids);
    if let DataRenderType::RgbImage { blue, .. } = &mut unregistered.render_type {
        blue.columns[0] = "nowhere".into();
    }
    assert!(
        r.register_chart(chart.config().clone(), vec![unregistered])
            .is_err(),
        "an unregistered blue column is refused"
    );
    // A streamed source is refused before any chunk is requested (rule 8).
    let streamed = |id: &str| renderer::StreamColumn {
        id: id.into(),
        len: COLS as u64,
        revision: 1,
        encoding: renderer::StreamEncoding::ScalarF32,
        replay: renderer::StreamReplay::RandomAccess,
        statistics: renderer::StreamStatistics::Unknown,
    };
    r.register_streamed_columns(vec![streamed("st_x"), streamed("st_y")])
        .unwrap();
    let mut over_stream = flat("img", &ids);
    over_stream.x_column = "st_x".into();
    over_stream.y_column = "st_y".into();
    let id = r
        .register_chart(chart.config().clone(), vec![over_stream])
        .expect("registration admits the columns; the streaming inspector names the rule");
    match r.inspect_streaming_capability(id) {
        Err(FiggyError::InvalidSeriesConfig { series_id, reason }) => {
            assert_eq!(series_id, "img");
            assert_eq!(
                reason,
                "image series require GPU-resident columns; streamed sources are not supported"
            );
        }
        other => panic!(
            "expected the streamed rejection, got {:?}",
            other.map(|_| ())
        ),
    }
    r.remove_chart(id).unwrap();
    // No colourbar is needed: the chart has none and the image registers
    // and exports, where a heatmap over the same grid asks for one.
    let heatmap = SeriesConfig {
        render_type: DataRenderType::Heatmap {
            matrix: matrix(&ids[0], MatrixOrientation::ColumnsAreX),
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            },
            surface: None,
        },
        ..flat("img", &ids)
    };
    assert!(
        reason(&mut r, chart.config(), heatmap).starts_with("a field series needs Config.colorbar")
    );
    let id = r
        .register_chart(chart.config().clone(), vec![flat("img", &ids)])
        .unwrap();
    r.remove_chart(id).unwrap();
    r.export_panel_rgba(&chart, &[flat("img", &ids)], 1.0)
        .unwrap();
}

/// The big image's channel `k` at column `c`, row `r`: constant over 128 by
/// 128 blocks (red by the block column, green by the block row, blue by
/// their sum), so an export pixel that covers several cells reads one colour.
fn big_channel(c: usize, r: usize, k: usize) -> f64 {
    let (bc, br) = (c / 128, r / 128);
    let step = match k {
        0 => bc % 16,
        1 => br % 16,
        _ => (bc + br) % 16,
    };
    (step * 17) as f64
}

/// Rule 10: a 2048 by 2048 image of three channels (12.6 million values)
/// registers, draws and picks; the lookup is the same per-fragment read
/// whatever the count.
#[test]
fn a_two_thousand_square_image_draws_and_picks() {
    let mut r = renderer_with(256 * 1024 * 1024);
    let (cols, rows) = (2048usize, 2048usize);
    let x = col_f64((0..cols).map(|c| c as f64).collect());
    let y = col_f64((0..rows).map(|r| r as f64).collect());
    let channel = |k: usize| -> Vec<Column<f64>> {
        (0..cols)
            .map(|c| col_f64((0..rows).map(|r| big_channel(c, r, k)).collect()))
            .collect()
    };
    let grids = [channel(0), channel(1), channel(2)];
    let ids: ChannelIds =
        ["r", "g", "b"].map(|ch| (0..cols).map(|c| format!("big_{ch}{c}")).collect());
    let mut batch: Vec<(&str, &dyn ColumnSource)> = vec![("big_x", &x), ("big_y", &y)];
    for (ids, grid) in ids.iter().zip(&grids) {
        for (id, column) in ids.iter().zip(grid) {
            batch.push((id.as_str(), column));
        }
    }
    r.add_columns(&batch)
        .expect("a 12.6-million-value image uploads");
    let mut chart = chart();
    chart.set_x_range(-1.0, cols as f64);
    chart.set_y_range(-1.0, rows as f64);
    let series = image(
        "big",
        &ids,
        MatrixOrientation::ColumnsAreX,
        [0.0, 255.0],
        Shading::Flat,
    );
    let started = std::time::Instant::now();
    let img = r.export_panel_rgba(&chart, &[series.clone()], 1.0).unwrap();
    eprintln!(
        "{cols} x {rows} x 3 image exported in {:?}",
        started.elapsed()
    );
    // The middle of the block (7, 7): red 7 · 17, green 7 · 17, blue 14 · 17.
    let (c, row) = (960usize, 960usize);
    assert_colour_at(
        &img,
        data_to_px(&chart, c as f64, row as f64, 1.0),
        mapped(
            [
                big_channel(c, row, 0),
                big_channel(c, row, 1),
                big_channel(c, row, 2),
            ],
            1.0,
            [0.0, 255.0],
        ),
        "the middle of block (7, 7)",
    );
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), vec![series])
        .unwrap();
    let started = std::time::Instant::now();
    let at = data_to_px(&chart, c as f64, row as f64, 1.0);
    let picked =
        cell(pollster::block_on(r.pick_chart_data(id, request(at)).unwrap().resolve()).unwrap());
    eprintln!("picked in {:?}", started.elapsed());
    // The pixel covers several cells at this size; the pick answers one of
    // the cells under it.
    let (pc, pr) = picked.expect("a cell under the cursor");
    let per_px = cols as f64 / f64::from(WIDTH) + 1.0;
    assert!(
        (pc as f64 - c as f64).abs() <= per_px && (pr as f64 - row as f64).abs() <= per_px * 1.5,
        "picked ({pc}, {pr}) near ({c}, {row})"
    );
}
