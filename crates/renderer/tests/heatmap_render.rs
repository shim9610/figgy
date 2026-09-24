//! End-to-end pixel verification of the field (heatmap / band) render path,
//! through the public renderer API and the headless `export_panel_rgba`.
//!
//! House pattern (mirrors `histogram_render.rs`): each test builds its own
//! instance/adapter/device and early-returns when no usable adapter exists.
//!
//! Attribution strategy: the colourmap is an explicit red → green ramp, so no
//! cell colour can be confused with the black deco ink or the background, and
//! every assertion names the colour `ColorBarOptions::color_for_z` computes on
//! the CPU. That is the point of most of these tests — the shader reimplements
//! `colormap::sample` over the same stops, and the CPU-drawn colourbar strip
//! walks the same ramp, so a divergence between the two paths is the bug this
//! file exists to catch.

use std::sync::{Arc, OnceLock};

use renderer::config::AxisScale;
use renderer::data::{Column, ColumnPairWriter, ColumnSource, ColumnUploadStats};
use renderer::data_config::{
    ContourConfig, DataLineStyleConfig, FieldFillConfig, FillMode, GridLayout, MatrixOrientation,
    MatrixRef, Shading,
};
use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, Color, ColorBarOptions, ColorMap, DataRenderType, DataSelectionsConfig, PickedDataRef,
    RasterImage, Renderer, RendererDevice, SeriesConfig, encode_png,
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

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let finite = |acc: f64, v: f64| if v.is_finite() { acc.min(v) } else { acc };
    let min = data.iter().copied().fold(f64::INFINITY, finite);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, |acc, v| {
        if v.is_finite() { acc.max(v) } else { acc }
    });
    Column { data, min, max }
}

struct PairColumn {
    pairs: Vec<(f32, f32)>,
}

impl ColumnSource for PairColumn {
    fn len(&self) -> usize {
        self.pairs.len()
    }

    fn max(&self) -> f64 {
        self.pairs
            .iter()
            .map(|(hi, lo)| *hi as f64 + *lo as f64)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    fn min(&self) -> f64 {
        self.pairs
            .iter()
            .map(|(hi, lo)| *hi as f64 + *lo as f64)
            .fold(f64::INFINITY, f64::min)
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        for (bytes, (hi, lo)) in dst.chunks_exact_mut(4).zip(&self.pairs) {
            bytes.copy_from_slice(&(*hi + *lo).to_le_bytes());
        }
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        for (index, (hi, lo)) in self.pairs.iter().copied().enumerate() {
            dst.write_pair(index, hi, lo);
        }
        let min_positive = self
            .pairs
            .iter()
            .map(|(hi, lo)| *hi as f64 + *lo as f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .reduce(f64::min);
        ColumnUploadStats { min_positive }
    }
}

/// The process-wide device, created once.
///
/// A `wgpu` device costs adapter enumeration, driver init and shader-cache setup;
/// creating one per test made this binary's tests contend for the driver and hold
/// a device apiece. `column_upload_stats.rs` already pools this way — the pixel
/// suites simply had not.
///
/// The `Renderer` is still per test: it owns the column pool, so sharing one
/// would let test fixtures collide on column ids. Only the device is shared, and
/// `wgpu::Device` is `Send + Sync`, so the default parallel harness is fine.
fn shared_device() -> Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
    static DEVICE: OnceLock<Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let instance = create_instance();
            let adapter = request_adapter(&instance).ok()?;
            let (device, queue) = request_device(&adapter).ok()?;
            Some((Arc::new(device), Arc::new(queue)))
        })
        .as_ref()
        .map(|(device, queue)| (Arc::clone(device), Arc::clone(queue)))
}

fn try_renderer() -> Option<Renderer> {
    let (device, queue) = shared_device()?;
    Some(
        Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        )
        .expect("renderer init"),
    )
}

fn colorbar(z_min: f64, z_max: f64, scale: AxisScale) -> ColorBarOptions {
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Custom {
        stops: vec![LOW, HIGH],
    };
    bar.nan_color = NAN_PAINT;
    bar.axis.min = z_min;
    bar.axis.max = z_max;
    bar.axis.scale = scale;
    // Tick spacing is config, not something the draw derives: the default 0.2
    // over a range of 5 would crowd 26 labels onto the strip. Five intervals is
    // what a host would ask for.
    bar.axis.major_spacing = (z_max - z_min) / 5.0;
    bar
}

/// Grid and legend off: the only non-field ink is the black deco frame.
fn bare_chart(bar: ColorBarOptions) -> Chart {
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
    config.colorbar = Some(bar);
    Chart::new(config)
}

fn heatmap(
    columns: &[&str],
    orientation: MatrixOrientation,
    grid_layout: GridLayout,
    shading: Shading,
    mode: FillMode,
    levels: Vec<f64>,
) -> SeriesConfig {
    let matrix = MatrixRef {
        columns: columns.iter().map(|id| (*id).to_string()).collect(),
        orientation,
        grid_layout,
    };
    let fill = FieldFillConfig {
        mode,
        shading,
        opacity: 1.0,
    };
    SeriesConfig {
        series_id: "field".into(),
        source_id: None,
        label: None,
        x_column: "gx".into(),
        y_column: "gy".into(),
        render_type: if levels.is_empty() {
            DataRenderType::Heatmap { matrix, fill }
        } else {
            DataRenderType::HeatmapContour {
                matrix,
                fill,
                contour: ContourConfig {
                    levels,
                    line: DataLineStyleConfig {
                        line_style: LineStylePreset::Solid,
                        line_color: Color::new(0.0, 0.0, 0.0, 0.0),
                        line_width: 0.0,
                    },
                    per_level_color: None,
                    labels: None,
                },
            }
        },
    }
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

fn is_selection(p: &[u8]) -> bool {
    p[3] > 180 && p[0] > 180 && p[2] > 180 && p[1] < 80
}

/// Data (x, y) → panel pixel, through the same data area and axis range the
/// renderer draws with.
fn data_to_px(chart: &Chart, x: f64, y: f64) -> (u32, u32) {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let tx = (x - cfg.bottom_x.min) / (cfg.bottom_x.max - cfg.bottom_x.min);
    let ty = (y - cfg.left_y.min) / (cfg.left_y.max - cfg.left_y.min);
    (
        (da.x as f64 + tx * da.width as f64).round() as u32,
        ((da.y + da.height) as f64 - ty * da.height as f64).round() as u32,
    )
}

fn to8(v: f32) -> i32 {
    (v.clamp(0.0, 1.0) * 255.0).round() as i32
}

/// Assert the pixel at a data-space point is the colour the CPU says that z has.
///
/// Two rounding steps separate the two numbers (f32 ramp maths, then 8-bit
/// quantization), so the tolerance is 2/255 — tight enough that a wrong `t`, a
/// wrong stop pair, or a missing premultiply all fail.
fn assert_colour_at(img: &RasterImage, px: (u32, u32), want: Color, what: &str) {
    assert_colour_within(img, px, want, 2, what);
}

fn assert_colour_within(img: &RasterImage, px: (u32, u32), want: Color, tol: i32, what: &str) {
    let p = pixel(img, px.0, px.1);
    let got = [p[0] as i32, p[1] as i32, p[2] as i32, p[3] as i32];
    let expected = [to8(want.r), to8(want.g), to8(want.b), to8(want.a)];
    let close = got.iter().zip(expected).all(|(g, e)| (*g - e).abs() <= tol);
    assert!(
        close,
        "{what} at px {px:?}: got {got:?}, expected {expected:?} (tol {tol})"
    );
}

/// A 3 x 2 grid over x edges [0,1,2,3] and y edges [0,1,2], with the six z
/// values 0..5. No two cells share a value, so a cell drawn in the wrong place
/// shows the wrong colour rather than passing.
const Z: [[f64; 2]; 3] = [[0.0, 1.0], [2.0, 3.0], [4.0, 5.0]];

fn edges_fixture(renderer: &mut Renderer) {
    renderer
        .add_columns(&[
            (
                "gx",
                &col_f64(vec![0.0, 1.0, 2.0, 3.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &col_f64(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            ("z0", &col_f64(Z[0].to_vec()) as &dyn renderer::ColumnSource),
            ("z1", &col_f64(Z[1].to_vec()) as &dyn renderer::ColumnSource),
            ("z2", &col_f64(Z[2].to_vec()) as &dyn renderer::ColumnSource),
        ])
        .expect("grid upload");
}

fn ranged_chart(bar: ColorBarOptions) -> Chart {
    let mut chart = bare_chart(bar);
    chart.set_x_range(0.0, 3.0);
    chart.set_y_range(0.0, 2.0);
    chart
}

/// Every cell is painted, at its own place, with its own value's colour — and
/// the colour is the one `ColorBarOptions::color_for_z` computes on the CPU.
#[test]
fn every_cell_carries_its_own_value_colour() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    edges_fixture(&mut renderer);
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let chart = ranged_chart(bar.clone());
    let series = vec![heatmap(
        &["z0", "z1", "z2"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Edges,
        Shading::Flat,
        FillMode::Continuous,
        Vec::new(),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    for (c, column) in Z.iter().enumerate() {
        for (r, z) in column.iter().enumerate() {
            let px = data_to_px(&chart, c as f64 + 0.5, r as f64 + 0.5);
            assert_colour_at(&img, px, bar.color_for_z(*z), &format!("cell ({c},{r})"));
        }
    }
}

#[test]
fn selected_matrix_cell_outlines_the_canonical_xy_cell() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    edges_fixture(&mut renderer);
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let mut chart = ranged_chart(bar);
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::MatrixCell {
            source_id: None,
            series_id: "field".into(),
            x_index: 1,
            y_index: 0,
        }],
        highlight_color: Color::new(1.0, 0.0, 1.0, 1.0),
        outline_width_px: 4.0,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 0.0,
    });
    let series = vec![heatmap(
        &["z0", "z1", "z2"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Edges,
        Shading::Flat,
        FillMode::Continuous,
        Vec::new(),
    )];

    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let (x0, y1) = data_to_px(&chart, 1.0, 0.0);
    let (x1, y0) = data_to_px(&chart, 2.0, 1.0);
    let mut selected = 0usize;
    let mut leaked = 0usize;
    for y in 0..img.height {
        for x in 0..img.width {
            if is_selection(pixel(&img, x, y)) {
                if x + 3 >= x0 && x <= x1 + 3 && y + 3 >= y0 && y <= y1 + 3 {
                    selected += 1;
                } else {
                    leaked += 1;
                }
            }
        }
    }
    assert!(
        selected > 100,
        "cell selection produced only {selected} pixels"
    );
    assert_eq!(leaked, 0, "cell selection leaked onto another cell");
}

#[test]
fn selected_contour_level_is_redrawn_from_the_same_field_snapshot() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    edges_fixture(&mut renderer);
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let mut chart = ranged_chart(bar);
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::ContourLevel {
            source_id: None,
            series_id: "field".into(),
            level_index: 0,
            x_index: 1,
            y_index: 0,
        }],
        highlight_color: Color::new(1.0, 0.0, 1.0, 1.0),
        outline_width_px: 0.0,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 5.0,
    });
    let series = vec![heatmap(
        &["z0", "z1", "z2"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Edges,
        Shading::Interpolated,
        FillMode::Continuous,
        vec![2.5],
    )];

    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let selected = img
        .rgba
        .chunks_exact(4)
        .filter(|pixel| is_selection(pixel))
        .count();
    assert!(
        selected > 100,
        "selected contour produced only {selected} highlight pixels"
    );
}

/// Outside the declared grid the field paints nothing. The grid covers x ∈
/// [0, 3] while the axis runs to 6, so the right half of the data area must be
/// untouched — which is also what proves the one full-screen quad is not simply
/// filling everything.
#[test]
fn nothing_is_painted_outside_the_grid() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    edges_fixture(&mut renderer);
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let mut chart = bare_chart(bar.clone());
    chart.set_x_range(0.0, 6.0);
    chart.set_y_range(0.0, 2.0);
    let series = vec![heatmap(
        &["z0", "z1", "z2"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Edges,
        Shading::Flat,
        FillMode::Continuous,
        Vec::new(),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    // Inside: the grid is there.
    let inside = data_to_px(&chart, 0.5, 0.5);
    assert_colour_at(&img, inside, bar.color_for_z(0.0), "cell (0,0)");
    // Outside: neither ramp end, and not the NaN colour either — the fragment
    // reported a miss and wrote nothing.
    for x in [3.5f64, 4.5, 5.5] {
        let px = data_to_px(&chart, x, 1.0);
        let p = pixel(&img, px.0, px.1);
        let is_ramp = p[0] > 40 || p[1] > 40 || p[2] > 40;
        assert!(
            !is_ramp,
            "x={x} is outside the grid but px {px:?} holds {:?}",
            &p[..4]
        );
    }
}

/// A cell whose z is NaN takes `nan_color`, not an endpoint colour. "Missing"
/// and "smallest" are different facts.
#[test]
fn a_nan_cell_takes_the_nan_colour() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_columns(&[
            (
                "gx",
                &col_f64(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &col_f64(vec![0.0, 1.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "z0",
                &col_f64(vec![f64::NAN]) as &dyn renderer::ColumnSource,
            ),
            ("z1", &col_f64(vec![5.0]) as &dyn renderer::ColumnSource),
        ])
        .expect("grid upload");
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let mut chart = bare_chart(bar.clone());
    chart.set_x_range(0.0, 2.0);
    chart.set_y_range(0.0, 1.0);
    let series = vec![heatmap(
        &["z0", "z1"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Edges,
        Shading::Flat,
        FillMode::Continuous,
        Vec::new(),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    assert_colour_at(&img, data_to_px(&chart, 0.5, 0.5), NAN_PAINT, "NaN cell");
    assert_colour_at(
        &img,
        data_to_px(&chart, 1.5, 0.5),
        bar.color_for_z(5.0),
        "finite cell",
    );
}

/// `ColumnsAreY` transposes which axis the constituent columns run along. The
/// same five columns and the same z values, read the other way round.
#[test]
fn columns_are_y_reads_the_grid_transposed() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    edges_fixture(&mut renderer);
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    // Constituent columns now run along y, so y needs 4 edges and x needs 3.
    let mut chart = bare_chart(bar.clone());
    chart.set_x_range(0.0, 2.0);
    chart.set_y_range(0.0, 3.0);
    let series = vec![SeriesConfig {
        x_column: "gy".into(),
        y_column: "gx".into(),
        ..heatmap(
            &["z0", "z1", "z2"],
            MatrixOrientation::ColumnsAreY,
            GridLayout::Edges,
            Shading::Flat,
            FillMode::Continuous,
            Vec::new(),
        )
    }];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    // Column c is now the y cell, value r the x cell.
    for (c, column) in Z.iter().enumerate() {
        for (r, z) in column.iter().enumerate() {
            let px = data_to_px(&chart, r as f64 + 0.5, c as f64 + 0.5);
            assert_colour_at(
                &img,
                px,
                bar.color_for_z(*z),
                &format!("transposed cell ({c},{r})"),
            );
        }
    }
}

/// `Centers` reads the coordinates as cell midpoints: the cell around
/// coordinate 1 spans 0.5..1.5, so 0.6 and 1.4 are the same cell while 1.6 is
/// the next one.
#[test]
fn centers_layout_puts_the_cell_around_its_coordinate() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_columns(&[
            (
                "gx",
                &col_f64(vec![1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            // Two y centres: one alone would have no neighbour to measure its
            // height against, and the shader draws a zero-height cell rather
            // than invent one.
            (
                "gy",
                &col_f64(vec![1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "z0",
                &col_f64(vec![0.0, 0.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "z1",
                &col_f64(vec![5.0, 5.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("grid upload");
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let mut chart = bare_chart(bar.clone());
    chart.set_x_range(0.0, 3.0);
    chart.set_y_range(0.0, 2.0);
    let series = vec![heatmap(
        &["z0", "z1"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Centers,
        Shading::Flat,
        FillMode::Continuous,
        Vec::new(),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    let low = bar.color_for_z(0.0);
    let high = bar.color_for_z(5.0);
    assert_colour_at(&img, data_to_px(&chart, 0.6, 1.0), low, "left of centre 1");
    assert_colour_at(&img, data_to_px(&chart, 1.4, 1.0), low, "right of centre 1");
    assert_colour_at(
        &img,
        data_to_px(&chart, 1.6, 1.0),
        high,
        "past the midpoint",
    );
    // The outer half-cells are mirrored, so the field ends at 0.5 and 2.5.
    let outside = data_to_px(&chart, 0.2, 1.0);
    let p = pixel(&img, outside.0, outside.1);
    assert!(
        p[0] < 40 && p[1] < 40,
        "0.2 is before the mirrored half-cell but px {outside:?} holds {:?}",
        &p[..4]
    );
}

/// Interpolated shading spans sample point to sample point and blends the
/// corner values, so the midpoint between two samples is halfway up the ramp —
/// not either endpoint colour.
#[test]
fn interpolated_shading_blends_between_sample_points() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_columns(&[
            (
                "gx",
                &col_f64(vec![0.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &col_f64(vec![0.0, 1.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "z0",
                &col_f64(vec![0.0, 0.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "z1",
                &col_f64(vec![4.0, 4.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("grid upload");
    let bar = colorbar(0.0, 4.0, AxisScale::Linear);
    let mut chart = bare_chart(bar.clone());
    chart.set_x_range(0.0, 2.0);
    chart.set_y_range(0.0, 1.0);
    let series = vec![heatmap(
        &["z0", "z1"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Centers,
        Shading::Interpolated,
        FillMode::Continuous,
        Vec::new(),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    // The midpoint is the exact claim: halfway between two samples is halfway
    // up the ramp. Flat shading would paint one of the two endpoint colours here.
    assert_colour_at(
        &img,
        data_to_px(&chart, 1.0, 0.5),
        bar.color_for_z(2.0),
        "midway",
    );
    // The ends are within a pixel of the sample points, so they are asserted
    // with a pixel's worth of slack rather than exactly.
    assert_colour_within(
        &img,
        data_to_px(&chart, 0.02, 0.5),
        bar.color_for_z(0.0),
        6,
        "at sample 0",
    );
    assert_colour_within(
        &img,
        data_to_px(&chart, 1.98, 0.5),
        bar.color_for_z(4.0),
        6,
        "at sample 1",
    );
    // And it really is a ramp, not two flat halves: red falls monotonically.
    let red_at = |x: f64| {
        pixel(
            &img,
            data_to_px(&chart, x, 0.5).0,
            data_to_px(&chart, x, 0.5).1,
        )[0]
    };
    let samples: Vec<u8> = [0.25f64, 0.75, 1.25, 1.75]
        .iter()
        .map(|x| red_at(*x))
        .collect();
    for pair in samples.windows(2) {
        assert!(
            pair[0] > pair[1],
            "red must fall left to right across the blend: {samples:?}"
        );
    }
}

/// Banded fill quantizes into the intervals between the contour levels: one
/// flat colour per band, and the same colour on both sides of a level's
/// interior. `n` levels give `n + 1` bands, each coloured at its own midpoint.
#[test]
fn bands_quantize_the_ramp_into_flat_steps() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    edges_fixture(&mut renderer);
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let chart = ranged_chart(bar.clone());
    // One level at 2.5: z 0/1/2 fall below it, z 3/4/5 above.
    let series = vec![heatmap(
        &["z0", "z1", "z2"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Edges,
        Shading::Flat,
        FillMode::Bands,
        vec![2.5],
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    // Two bands: the ramp at 0.25 and at 0.75.
    let below = bar.colormap.sample(0.25);
    let above = bar.colormap.sample(0.75);
    for (c, column) in Z.iter().enumerate() {
        for (r, z) in column.iter().enumerate() {
            let px = data_to_px(&chart, c as f64 + 0.5, r as f64 + 0.5);
            let want = if *z >= 2.5 { above } else { below };
            assert_colour_at(&img, px, want, &format!("banded cell ({c},{r}) z={z}"));
        }
    }
}

/// Each stored lane is finite, but reconstructing the logical value overflows
/// f32. Derived arithmetic must treat that as unplaceable instead of allowing a
/// NaN ramp position to fall through to the low endpoint colour.
#[test]
fn interpolated_finite_pairs_that_overflow_reconstruction_take_nan_colour() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let overflow = PairColumn {
        pairs: vec![(f32::MAX, f32::MAX); 2],
    };
    renderer
        .add_columns(&[
            (
                "gx",
                &col_f64(vec![0.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &col_f64(vec![0.0, 1.0]) as &dyn renderer::ColumnSource,
            ),
            ("z0", &overflow as &dyn renderer::ColumnSource),
            ("z1", &overflow as &dyn renderer::ColumnSource),
        ])
        .expect("overflow grid upload");
    let bar = colorbar(0.0, 1.0, AxisScale::Linear);
    let mut chart = bare_chart(bar);
    chart.set_x_range(0.0, 2.0);
    chart.set_y_range(0.0, 1.0);
    let series = [heatmap(
        &["z0", "z1"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Centers,
        Shading::Interpolated,
        FillMode::Continuous,
        Vec::new(),
    )];
    let image = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    assert_colour_at(
        &image,
        data_to_px(&chart, 1.0, 0.5),
        NAN_PAINT,
        "overflowed pair reconstruction",
    );
}

/// Coordinate lanes can each be finite while their reconstructed boundary is
/// infinite. A descending +inf/-inf bracket used to pass the range check and
/// let flat shading paint an arbitrary first cell. Lookup must fail closed.
#[test]
fn nonfinite_coordinate_boundaries_leave_flat_field_unpainted() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let gx = PairColumn {
        pairs: vec![(f32::MAX, f32::MAX), (-f32::MAX, -f32::MAX)],
    };
    renderer
        .add_columns(&[
            ("gx", &gx as &dyn renderer::ColumnSource),
            (
                "gy",
                &col_f64(vec![0.0, 1.0]) as &dyn renderer::ColumnSource,
            ),
            ("z0", &col_f64(vec![0.5]) as &dyn renderer::ColumnSource),
        ])
        .expect("overflow coordinate fixture upload");
    let mut chart = bare_chart(colorbar(0.0, 1.0, AxisScale::Linear));
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    let image = renderer
        .export_panel_rgba(
            &chart,
            &[heatmap(
                &["z0"],
                MatrixOrientation::ColumnsAreX,
                GridLayout::Edges,
                Shading::Flat,
                FillMode::Continuous,
                Vec::new(),
            )],
            1.0,
        )
        .unwrap();
    let empty = renderer.export_panel_rgba(&chart, &[], 1.0).unwrap();
    let center = data_to_px(&chart, 0.5, 0.5);
    assert_eq!(
        pixel(&image, center.0, center.1),
        pixel(&empty, center.0, center.1),
        "non-finite coordinate boundaries must not select a fallback cell"
    );
}

#[test]
fn a_1024_level_banded_fill_counts_unsorted_duplicates_by_value() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    edges_fixture(&mut renderer);
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let chart = ranged_chart(bar.clone());
    let mut levels: Vec<f64> = (0..renderer::MAX_CONTOUR_LEVELS)
        .map(|index| ((index * 37) % 11) as f64 - 2.0)
        .collect();
    levels[31] = f64::NAN;
    levels[32] = f64::NEG_INFINITY;
    levels[1000] = f64::INFINITY;
    levels[1001] = f64::MAX;
    levels[1002] = f64::MIN;
    let series = [heatmap(
        &["z0", "z1", "z2"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Edges,
        Shading::Flat,
        FillMode::Bands,
        levels.clone(),
    )];
    let image = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    for (column, values) in Z.iter().enumerate() {
        for (row, z) in values.iter().enumerate() {
            let z_key = *z as f32;
            let reached = levels
                .iter()
                .map(|level| *level as f32)
                .filter(|level| {
                    *level == f32::NEG_INFINITY || (level.is_finite() && z_key >= *level)
                })
                .count();
            let t = (reached as f32 + 0.5) / (levels.len() as f32 + 1.0);
            assert_colour_at(
                &image,
                data_to_px(&chart, column as f64 + 0.5, row as f64 + 0.5),
                bar.colormap.sample(t),
                &format!("1024-level banded cell ({column},{row}) z={z}"),
            );
        }
    }
}

/// A logarithmic colourbar maps z by its logarithm — the same `t` the axis
/// machinery uses, and the same one the CPU strip walks.
#[test]
fn a_logarithmic_colourbar_maps_z_by_its_logarithm() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_columns(&[
            (
                "gx",
                &col_f64(vec![0.0, 1.0, 2.0, 3.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &col_f64(vec![0.0, 1.0]) as &dyn renderer::ColumnSource,
            ),
            ("z0", &col_f64(vec![1.0]) as &dyn renderer::ColumnSource),
            ("z1", &col_f64(vec![10.0]) as &dyn renderer::ColumnSource),
            ("z2", &col_f64(vec![100.0]) as &dyn renderer::ColumnSource),
        ])
        .expect("grid upload");
    let bar = colorbar(1.0, 100.0, AxisScale::Logarithmic);
    let mut chart = bare_chart(bar.clone());
    chart.set_x_range(0.0, 3.0);
    chart.set_y_range(0.0, 1.0);
    let series = vec![heatmap(
        &["z0", "z1", "z2"],
        MatrixOrientation::ColumnsAreX,
        GridLayout::Edges,
        Shading::Flat,
        FillMode::Continuous,
        Vec::new(),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    // A decade is half the range, so z = 10 is the ramp's midpoint — which a
    // linear mapping would put at 10/100 instead.
    for (c, z) in [1.0f64, 10.0, 100.0].iter().enumerate() {
        assert_colour_at(
            &img,
            data_to_px(&chart, c as f64 + 0.5, 0.5),
            bar.color_for_z(*z),
            &format!("log cell {c} (z={z})"),
        );
    }
    let mid = data_to_px(&chart, 1.5, 0.5);
    assert_colour_at(&img, mid, bar.colormap.sample(0.5), "z=10 is mid-ramp");
}

/// A grid whose data does not line up with its declaration draws the smallest
/// common extent and *says so*, rather than erroring — the report is what makes
/// truncation observable instead of mysterious.
#[test]
fn a_short_grid_draws_less_and_reports_it() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    edges_fixture(&mut renderer);
    // z2 holds one value where the y edges bound two cells.
    renderer
        .add_column("z_short", &col_f64(vec![4.0]))
        .expect("short column");
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let chart = ranged_chart(bar.clone());

    let whole = renderer
        .register_chart(
            chart.config().clone(),
            vec![heatmap(
                &["z0", "z1", "z2"],
                MatrixOrientation::ColumnsAreX,
                GridLayout::Edges,
                Shading::Flat,
                FillMode::Continuous,
                Vec::new(),
            )],
        )
        .unwrap();
    let info = renderer.series_draw_info(whole, "field").unwrap();
    assert_eq!((info.cols, info.rows), (Some(3), Some(2)));
    assert_eq!(info.drawn_count, 6);
    assert!(!info.truncated, "a well-formed grid is not truncated");

    let short = renderer
        .register_chart(
            chart.config().clone(),
            vec![heatmap(
                &["z0", "z1", "z_short"],
                MatrixOrientation::ColumnsAreX,
                GridLayout::Edges,
                Shading::Flat,
                FillMode::Continuous,
                Vec::new(),
            )],
        )
        .unwrap();
    let info = renderer.series_draw_info(short, "field").unwrap();
    assert_eq!(
        (info.cols, info.rows),
        (Some(3), Some(1)),
        "the shortest column decides the rows"
    );
    assert_eq!(info.drawn_count, 3);
    assert!(info.truncated, "two columns offered a second row");

    // A fourth declared column has no x cell to sit in: cols stays 3.
    let extra = renderer
        .register_chart(
            chart.config().clone(),
            vec![heatmap(
                &["z0", "z1", "z2", "z0"],
                MatrixOrientation::ColumnsAreX,
                GridLayout::Edges,
                Shading::Flat,
                FillMode::Continuous,
                Vec::new(),
            )],
        )
        .unwrap();
    let info = renderer.series_draw_info(extra, "field").unwrap();
    assert_eq!((info.cols, info.rows), (Some(3), Some(2)));
    assert!(info.truncated, "the fourth column is surplus");
}

/// Visual probe — writes the rendered fields so a human can look at them.
/// Assertions above are the actual gate; this is for eyes.
#[test]
fn heatmap_probe_renders_every_shading_and_fill() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    edges_fixture(&mut renderer);
    let bar = colorbar(0.0, 5.0, AxisScale::Linear);
    let chart = ranged_chart(bar);
    let cases = [
        (
            "flat-continuous",
            Shading::Flat,
            FillMode::Continuous,
            vec![],
        ),
        (
            "flat-bands",
            Shading::Flat,
            FillMode::Bands,
            vec![1.5f64, 3.5],
        ),
        (
            "interpolated-continuous",
            Shading::Interpolated,
            FillMode::Continuous,
            vec![],
        ),
        (
            "interpolated-bands",
            Shading::Interpolated,
            FillMode::Bands,
            vec![1.5, 3.5],
        ),
    ];
    let dir = std::env::var("FIGGY_PROBE_DIR").unwrap_or_else(|_| ".".to_string());
    for (name, shading, mode, levels) in cases {
        let series = vec![heatmap(
            &["z0", "z1", "z2"],
            MatrixOrientation::ColumnsAreX,
            GridLayout::Edges,
            shading,
            mode,
            levels,
        )];
        let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let png = encode_png(&img).expect("encode");
        std::fs::write(format!("{dir}/probe_heatmap_{name}.png"), png).expect("write probe");
    }
}
