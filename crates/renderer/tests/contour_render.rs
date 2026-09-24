//! End-to-end pixel verification of the implicit contour path: the field shader
//! draws level-set coverage directly from each cell's bilinear interpolation.
//!
//! House pattern (mirrors `heatmap_render.rs`): each test builds its own
//! instance/adapter/device and early-returns when no usable adapter exists.
//!
//! Attribution strategy: the field is either off or painted in a red→green ramp,
//! and contour lines are pure blue or pure white — colours the ramp cannot
//! produce. Every assertion names a data-space point whose z the fixture states,
//! so the tests say where an isoline *should* be rather than where it happened to
//! land.

use std::sync::{Arc, OnceLock};

use renderer::config::AxisScale;
use renderer::data::Column;
use renderer::data_config::{
    ContourConfig, ContourLabelAnchor, ContourLabelConfig, DataLineStyleConfig, FieldFillConfig,
    FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
};
use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::format::LabelFormat;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, Color, ColorBarOptions, ColorMap, DataRenderType, RasterImage, Renderer, RendererDevice,
    SeriesConfig, encode_png,
};

const WIDTH: u32 = 400;
const HEIGHT: u32 = 400;

const LINE_A: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};
const LINE_B: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};
const RAMP_LOW: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const RAMP_HIGH: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
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

fn colorbar(z_min: f64, z_max: f64) -> ColorBarOptions {
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Custom {
        stops: vec![RAMP_LOW, RAMP_HIGH],
    };
    bar.axis.min = z_min;
    bar.axis.max = z_max;
    bar.axis.scale = AxisScale::Linear;
    bar.axis.major_spacing = (z_max - z_min) / 4.0;
    bar
}

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
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 2.0);
    chart.set_y_range(0.0, 2.0);
    chart
}

fn matrix(columns: &[&str]) -> MatrixRef {
    MatrixRef {
        columns: columns.iter().map(|id| (*id).to_string()).collect(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Centers,
    }
}

fn contour_config(levels: Vec<f64>, per_level_color: Option<Vec<Color>>) -> ContourConfig {
    ContourConfig {
        levels,
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: LINE_A,
            line_width: 3.0,
        },
        per_level_color,
        labels: None,
    }
}

fn contour_only(columns: &[&str], levels: Vec<f64>, colors: Option<Vec<Color>>) -> SeriesConfig {
    SeriesConfig {
        series_id: "iso".into(),
        source_id: None,
        label: None,
        x_column: "gx".into(),
        y_column: "gy".into(),
        render_type: DataRenderType::Contour {
            matrix: matrix(columns),
            contour: contour_config(levels, colors),
        },
    }
}

fn heatmap_contour(columns: &[&str], levels: Vec<f64>) -> SeriesConfig {
    SeriesConfig {
        series_id: "iso".into(),
        source_id: None,
        label: None,
        x_column: "gx".into(),
        y_column: "gy".into(),
        render_type: DataRenderType::HeatmapContour {
            matrix: matrix(columns),
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            },
            contour: contour_config(levels, None),
        },
    }
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

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

fn is_blue(p: &[u8]) -> bool {
    p[3] > 16 && p[2] > 140 && p[0] < 100 && p[1] < 100
}
fn is_white(p: &[u8]) -> bool {
    p[3] > 16 && p[0] > 180 && p[1] > 180 && p[2] > 180
}

/// Whether any pixel within `radius` of a data point matches — the stroke is a
/// few pixels wide and lands on a rasterized grid, so an exact-pixel assertion
/// would be testing rounding rather than contour coverage.
fn near(img: &RasterImage, px: (u32, u32), radius: i32, hit: impl Fn(&[u8]) -> bool) -> bool {
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let x = px.0 as i32 + dx;
            let y = px.1 as i32 + dy;
            if x < 0 || y < 0 || x >= img.width as i32 || y >= img.height as i32 {
                continue;
            }
            if hit(pixel(img, x as u32, y as u32)) {
                return true;
            }
        }
    }
    false
}

/// z = x + y on a 3 x 3 lattice of centres at 0, 1, 2. Every level's isoline is
/// therefore a known straight anti-diagonal, which is what makes "the line is
/// where the level says" an assertion rather than a hope.
fn plane_fixture(renderer: &mut Renderer) {
    renderer
        .add_columns(&[
            (
                "gx",
                &col_f64(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &col_f64(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            // Column c holds x = c, values along y.
            (
                "p0",
                &col_f64(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "p1",
                &col_f64(vec![1.0, 2.0, 3.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "p2",
                &col_f64(vec![2.0, 3.0, 4.0]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("grid upload");
}

/// The implicit field shader puts the line on the isoline and leaves the rest of the field
/// alone. `z = x + y`, level 2, so the line runs from (2, 0) to (0, 2).
#[test]
fn a_contour_traces_the_isoline_and_nothing_else() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let series = vec![contour_only(&["p0", "p1", "p2"], vec![2.0], None)];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    // On the isoline, at three points along it.
    for (x, y) in [(0.25f64, 1.75f64), (1.0, 1.0), (1.75, 0.25)] {
        assert!(
            near(&img, data_to_px(&chart, x, y), 3, is_blue),
            "no contour ink near ({x}, {y}), where z = {}",
            x + y
        );
    }
    // Off it, on both sides, well clear of the stroke.
    for (x, y) in [(0.3f64, 0.3f64), (1.7, 1.7), (0.2, 1.0), (1.0, 0.2)] {
        assert!(
            !near(&img, data_to_px(&chart, x, y), 2, is_blue),
            "contour ink at ({x}, {y}), where z = {} and the level is 2",
            x + y
        );
    }
}

/// Each level draws in its own colour from the indexed group-2 colour table.
#[test]
fn each_level_draws_in_its_own_colour() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let series = vec![contour_only(
        &["p0", "p1", "p2"],
        vec![1.0, 3.0],
        Some(vec![LINE_A, LINE_B]),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    // z = 1 runs through (0.5, 0.5); z = 3 through (1.5, 1.5).
    assert!(
        near(&img, data_to_px(&chart, 0.5, 0.5), 3, is_blue),
        "level 1 must be blue"
    );
    assert!(
        near(&img, data_to_px(&chart, 1.5, 1.5), 3, is_white),
        "level 3 must be white"
    );
    // And not each other's colour.
    assert!(
        !near(&img, data_to_px(&chart, 0.5, 0.5), 2, is_white),
        "level 1 must not be drawn in level 3's colour"
    );
}

/// The 1024-level ceiling is a real draw path, not only a validation bound.
/// Two reachable duplicate levels straddle a 32-entry lookup-block boundary;
/// all other levels are outside the field. The result must exactly match the
/// same two levels declared alone, including source-over colour order.
#[test]
fn a_1024_level_draw_preserves_duplicate_order_across_lookup_blocks() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let lower = Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 0.5,
    };
    let upper = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 0.5,
    };

    let mut levels = vec![99.0; renderer::MAX_CONTOUR_LEVELS];
    levels[31] = 2.0;
    levels[32] = 2.0;
    let mut colors = vec![Color::BLACK; renderer::MAX_CONTOUR_LEVELS];
    colors[31] = lower;
    colors[32] = upper;
    let expanded = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(&["p0", "p1", "p2"], levels, Some(colors))],
            1.0,
        )
        .unwrap();
    let reference = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(
                &["p0", "p1", "p2"],
                vec![2.0, 2.0],
                Some(vec![lower, upper]),
            )],
            1.0,
        )
        .unwrap();

    assert_eq!(expanded.rgba, reference.rgba);
}

#[test]
fn level_1023_keeps_its_declared_colour_and_numeric_lookup_index() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let marker = Color {
        r: 0.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    let mut levels = vec![99.0; renderer::MAX_CONTOUR_LEVELS];
    levels[1023] = 2.0;
    let mut colors = vec![Color::BLACK; renderer::MAX_CONTOUR_LEVELS];
    colors[1023] = marker;

    let expanded = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(&["p0", "p1", "p2"], levels, Some(colors))],
            1.0,
        )
        .unwrap();
    let reference = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(
                &["p0", "p1", "p2"],
                vec![2.0],
                Some(vec![marker]),
            )],
            1.0,
        )
        .unwrap();

    assert_eq!(expanded.rgba, reference.rgba);
}

#[test]
fn non_finite_levels_never_draw_contour_lines() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let finite = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 0.75,
    };
    let expanded = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(
                &["p0", "p1", "p2"],
                vec![f64::NAN, f64::NEG_INFINITY, f64::INFINITY, 2.0],
                Some(vec![Color::WHITE, Color::WHITE, Color::WHITE, finite]),
            )],
            1.0,
        )
        .unwrap();
    let reference = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(
                &["p0", "p1", "p2"],
                vec![2.0],
                Some(vec![finite]),
            )],
            1.0,
        )
        .unwrap();

    assert_eq!(expanded.rgba, reference.rgba);
}

/// Every stored corner is finite, but the horizontal corner differences exceed
/// f32. The shared sampler must reject the cell before either contour coverage
/// or automatic-anchor Newton arithmetic can consume the overflowed derivative.
#[test]
fn finite_corners_whose_derivatives_overflow_draw_no_contour_or_anchor() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let lo = -(f32::MAX as f64);
    let hi = f32::MAX as f64;
    renderer
        .add_columns(&[
            (
                "overflow_x",
                &col_f64(vec![0.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "overflow_y",
                &col_f64(vec![0.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "overflow_z0",
                &col_f64(vec![lo, lo]) as &dyn renderer::ColumnSource,
            ),
            (
                "overflow_z1",
                &col_f64(vec![hi, hi]) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("overflow grid upload");
    let chart = bare_chart(colorbar(-1.0, 1.0));
    let mut series = contour_only(&["overflow_z0", "overflow_z1"], vec![0.0], None);
    series.x_column = "overflow_x".into();
    series.y_column = "overflow_y".into();
    if let DataRenderType::Contour { contour, .. } = &mut series.render_type {
        contour.labels = Some(auto_label_config(2000.0));
    }
    let image = renderer.export_panel_rgba(&chart, &[series], 1.0).unwrap();

    assert!(
        ink_bounds(&image, is_blue).is_none(),
        "overflowed contour derivatives must not produce line or label ink"
    );
    assert!(
        ink_bounds(&image, is_magenta).is_none(),
        "overflowed contour derivatives must not produce an automatic anchor"
    );
}

#[test]
fn all_1024_reachable_candidates_are_composited() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let layer = Color {
        r: 0.8,
        g: 0.2,
        b: 0.1,
        a: 1.0 / 2048.0,
    };
    let expanded = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(
                &["p0", "p1", "p2"],
                vec![2.0; renderer::MAX_CONTOUR_LEVELS],
                Some(vec![layer; renderer::MAX_CONTOUR_LEVELS]),
            )],
            1.0,
        )
        .unwrap();

    let equivalent_for = |count: usize| {
        let mut premul = [0.0f32; 4];
        let source = [
            layer.r * layer.a,
            layer.g * layer.a,
            layer.b * layer.a,
            layer.a,
        ];
        for _ in 0..count {
            for channel in 0..3 {
                premul[channel] = source[channel] + premul[channel] * (1.0 - source[3]);
            }
            premul[3] = source[3] + premul[3] * (1.0 - source[3]);
        }
        Color {
            r: premul[0] / premul[3],
            g: premul[1] / premul[3],
            b: premul[2] / premul[3],
            a: premul[3],
        }
    };
    let reference = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(
                &["p0", "p1", "p2"],
                vec![2.0],
                Some(vec![equivalent_for(renderer::MAX_CONTOUR_LEVELS)]),
            )],
            1.0,
        )
        .unwrap();
    let under_composited = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(
                &["p0", "p1", "p2"],
                vec![2.0],
                Some(vec![equivalent_for(621)]),
            )],
            1.0,
        )
        .unwrap();
    let centre = data_to_px(&chart, 1.0, 1.0);
    assert_eq!(
        pixel(&expanded, centre.0, centre.1),
        pixel(&reference, centre.0, centre.1)
    );
    assert_ne!(
        pixel(&expanded, centre.0, centre.1),
        pixel(&under_composited, centre.0, centre.1),
        "alpha 1/2048 must distinguish 1024 composites from 621"
    );
}

#[test]
fn sorted_reversed_and_shuffled_1024_levels_render_identically() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let sorted: Vec<f64> = (0..renderer::MAX_CONTOUR_LEVELS)
        .map(|index| 0.01 + 3.98 * index as f64 / (renderer::MAX_CONTOUR_LEVELS - 1) as f64)
        .collect();
    let colors = vec![LINE_A; renderer::MAX_CONTOUR_LEVELS];
    let expected = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(
                &["p0", "p1", "p2"],
                sorted.clone(),
                Some(colors.clone()),
            )],
            1.0,
        )
        .unwrap();

    let mut reversed = sorted.clone();
    reversed.reverse();
    let actual = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(
                &["p0", "p1", "p2"],
                reversed,
                Some(colors.clone()),
            )],
            1.0,
        )
        .unwrap();
    assert_eq!(actual.rgba, expected.rgba);

    let mut shuffled = sorted;
    let mut state = 0x9e37_79b9u32;
    for upper in (1..shuffled.len()).rev() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        shuffled.swap(upper, state as usize % (upper + 1));
    }
    let actual = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(&["p0", "p1", "p2"], shuffled, Some(colors))],
            1.0,
        )
        .unwrap();
    assert_eq!(actual.rgba, expected.rgba);
}

#[test]
fn a_direct_export_rejects_1025_contour_levels() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let levels = vec![2.0; renderer::MAX_CONTOUR_LEVELS + 1];
    for series in [
        contour_only(&["p0", "p1", "p2"], levels.clone(), None),
        heatmap_contour(&["p0", "p1", "p2"], levels.clone()),
    ] {
        assert!(matches!(
            renderer.export_panel_rgba(&chart, &[series], 1.0),
            Err(renderer::FiggyError::InvalidSeriesConfig { .. })
        ));
    }
}

#[test]
fn the_last_1024_level_label_cell_and_manual_index_draw() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let mut levels = vec![99.0; renderer::MAX_CONTOUR_LEVELS];
    levels[renderer::MAX_CONTOUR_LEVELS - 1] = 2.0;
    let series = [labelled(
        levels,
        None,
        label_config(vec![anchor(1023, 0.4, 0.4, 1.0, 0.0)], true),
    )];
    let image = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    assert!(
        near(&image, data_to_px(&chart, 0.4, 0.4), 14, is_blue),
        "the final atlas cell must remain addressable by a manual level index"
    );
}

/// A level nothing reaches draws nothing, and an empty level list draws nothing
/// at all even though the shared field quad is issued.
#[test]
fn a_level_outside_the_data_draws_nothing() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    for levels in [vec![], vec![99.0]] {
        let series = vec![contour_only(&["p0", "p1", "p2"], levels.clone(), None)];
        let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let ink = img.rgba.chunks_exact(4).filter(|p| is_blue(p)).count();
        assert_eq!(ink, 0, "levels {levels:?} left {ink} contour pixels");
    }
}

/// One isoline per level and no spurious coverage: on a monotone plane every
/// column of the data area that the line crosses holds exactly one run of ink.
/// A duplicated branch shows up here as a second run.
#[test]
fn a_monotone_field_traces_one_unbroken_isoline() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let series = vec![contour_only(&["p0", "p1", "p2"], vec![2.0], None)];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    let da = chart.config().data_area().expect("data area").0;
    let mut columns_with_ink = 0;
    for x in (da.x + 6)..(da.x + da.width - 6) {
        let mut runs = 0;
        let mut inside = false;
        for y in da.y..(da.y + da.height) {
            let ink = is_blue(pixel(&img, x, y));
            if ink && !inside {
                runs += 1;
            }
            inside = ink;
        }
        if runs > 0 {
            columns_with_ink += 1;
        }
        assert!(
            runs <= 1,
            "column {x} crosses the isoline {runs} times; a monotone plane has one"
        );
    }
    assert!(
        columns_with_ink > (da.width as usize) / 2,
        "the isoline spans the field, but only {columns_with_ink} columns hold ink"
    );
}

/// Contour lines are drawn over the fill they describe, not under it.
#[test]
fn contour_lines_land_on_top_of_the_field() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let bar = colorbar(0.0, 4.0);
    let chart = bare_chart(bar.clone());
    let series = vec![heatmap_contour(&["p0", "p1", "p2"], vec![2.0])];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    // On the isoline: the line wins.
    assert!(
        near(&img, data_to_px(&chart, 1.0, 1.0), 3, is_blue),
        "the contour must draw over the fill"
    );
    // Away from it: the fill is there, in the ramp colour for that z.
    let px = data_to_px(&chart, 0.4, 0.4);
    let p = pixel(&img, px.0, px.1);
    let want = bar.color_for_z(0.8);
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
    for (got, expected) in [
        (p[0] as i32, to8(want.r)),
        (p[1] as i32, to8(want.g)),
        (p[2] as i32, to8(want.b)),
    ] {
        assert!(
            (got - expected).abs() <= 3,
            "fill at (0.4, 0.4): got {:?}, expected {:?}",
            &p[..4],
            [to8(want.r), to8(want.g), to8(want.b)]
        );
    }
}

/// Visual probe — assertions above are the gate; this is for eyes.
#[test]
fn contour_probe_renders_lines_and_filled_bands() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    // A saddle: z = (x - 1)^2 - (y - 1)^2, sampled on a 5 x 5 lattice. Level 0
    // is the pair of diagonals through the centre, which is exactly the
    // saddle case that the bilinear implicit form must resolve without a branch.
    let coords = vec![0.0, 0.5, 1.0, 1.5, 2.0];
    let column = |x: f64| -> Column<f64> {
        col_f64(
            coords
                .iter()
                .map(|y| (x - 1.0) * (x - 1.0) - (y - 1.0) * (y - 1.0))
                .collect(),
        )
    };
    let columns: Vec<Column<f64>> = coords.iter().map(|x| column(*x)).collect();
    let ids = ["s0", "s1", "s2", "s3", "s4"];
    let batch: Vec<(&str, &dyn renderer::ColumnSource)> = ids
        .iter()
        .zip(&columns)
        .map(|(id, col)| (*id, col as &dyn renderer::ColumnSource))
        .collect();
    renderer
        .add_columns(&[
            (
                "gx",
                &col_f64(coords.clone()) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &col_f64(coords.clone()) as &dyn renderer::ColumnSource,
            ),
        ])
        .expect("coordinates");
    renderer.add_columns(&batch).expect("saddle grid");

    let dir = std::env::var("FIGGY_PROBE_DIR").unwrap_or_else(|_| ".".to_string());
    let bar = colorbar(-1.0, 1.0);
    let chart = bare_chart(bar);
    // Labels: one anchor per level, placed on that level's line with the tangent
    // that runs along it there.
    let mut labelled_lines = contour_only(&ids, vec![-0.5, 0.0, 0.5], None);
    if let DataRenderType::Contour { contour, .. } = &mut labelled_lines.render_type {
        contour.labels = Some(ContourLabelConfig {
            visible: true,
            font_size: 18.0,
            color: Color::BLACK,
            format: LabelFormat::Decimal,
            significant_digits: 1,
            spacing_px: 140.0,
            // Automatic: the probe shows what the anchor pass chooses.
            anchors: Vec::new(),
            bg_color: Some(Color::new(0.98, 0.98, 0.98, 1.0)),
            bg_padding_px: 2.0,
        });
    }
    let cases: [(&str, SeriesConfig); 3] = [
        ("lines", contour_only(&ids, vec![-0.5, 0.0, 0.5], None)),
        ("over-fill", heatmap_contour(&ids, vec![-0.5, 0.0, 0.5])),
        ("labelled", labelled_lines),
    ];
    for (name, series) in cases {
        let img = renderer.export_panel_rgba(&chart, &[series], 1.0).unwrap();
        let png = encode_png(&img).expect("encode");
        std::fs::write(format!("{dir}/probe_contour_{name}.png"), png).expect("write probe");
    }
}

// ── The implicit form (design B.4.9) ────────────────────────────────────────

/// `z = y` on a 3 x 3 lattice: the isolines are horizontal, so a vertical run of
/// ink measures the stroke width directly.
fn flat_y_fixture(renderer: &mut Renderer) {
    let col = col_f64(vec![0.0, 1.0, 2.0]);
    renderer
        .add_columns(&[
            ("hx", &col as &dyn renderer::ColumnSource),
            ("hy", &col as &dyn renderer::ColumnSource),
            ("h0", &col as &dyn renderer::ColumnSource),
            ("h1", &col as &dyn renderer::ColumnSource),
            ("h2", &col as &dyn renderer::ColumnSource),
        ])
        .expect("flat-y grid upload");
}

fn flat_y_series(levels: Vec<f64>, line_width: f32) -> SeriesConfig {
    let mut contour = contour_config(levels, None);
    contour.line.line_width = line_width;
    SeriesConfig {
        series_id: "flat".into(),
        source_id: None,
        label: None,
        x_column: "hx".into(),
        y_column: "hy".into(),
        render_type: DataRenderType::Contour {
            matrix: matrix(&["h0", "h1", "h2"]),
            contour,
        },
    }
}

/// The stroke is `line_width` pixels wide, and a wider declaration is wider ink.
///
/// The implicit form computes coverage from the bilinear polynomial's
/// gradient-normal quadratic root, so width belongs to the shader rather than a
/// segment quad's geometry.
/// Measured across a horizontal isoline, where a column of pixels crosses it once.
#[test]
fn the_stroke_is_as_wide_as_it_is_declared() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    flat_y_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 2.0));
    let run = |renderer: &mut Renderer, width: f32| {
        let series = vec![flat_y_series(vec![1.0], width)];
        let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (_, y0, _, y1) = ink_bounds(&img, is_blue).expect("no contour ink");
        y1 - y0 + 1
    };
    let thin = run(&mut renderer, 2.0);
    let thick = run(&mut renderer, 8.0);
    // Antialiasing adds a partial pixel on each side, so the ink is the declared
    // width plus about two — not less than it, and not many times it.
    assert!(
        (2..=5).contains(&thin),
        "a 2 px stroke measured {thin} px of ink"
    );
    assert!(
        (8..=11).contains(&thick),
        "an 8 px stroke measured {thick} px of ink"
    );
}

/// A grid far larger than the old segment ceiling still draws an unbroken line.
///
/// Marching squares allocated its output before it knew the crossing count, so a
/// dense grid could overrun `plan_capacity` and drop segments silently. The
/// implicit form has no such buffer: the cost is the data area's pixels, and the
/// grid only changes what z is.
#[test]
fn a_dense_grid_draws_an_unbroken_isoline() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    const N: usize = 120;
    let coords: Vec<f64> = (0..N).map(|i| 2.0 * i as f64 / (N - 1) as f64).collect();
    let coord_col = col_f64(coords.clone());
    let columns: Vec<Column<f64>> = coords
        .iter()
        .map(|x| col_f64(coords.iter().map(|y| x + y).collect()))
        .collect();
    let ids: Vec<String> = (0..N).map(|c| format!("d{c}")).collect();
    let mut batch: Vec<(&str, &dyn renderer::ColumnSource)> = Vec::with_capacity(N + 2);
    batch.push(("dx", &coord_col as &dyn renderer::ColumnSource));
    batch.push(("dy", &coord_col as &dyn renderer::ColumnSource));
    for (id, column) in ids.iter().zip(&columns) {
        batch.push((id.as_str(), column as &dyn renderer::ColumnSource));
    }
    renderer.add_columns(&batch).expect("dense grid upload");

    let chart = bare_chart(colorbar(0.0, 4.0));
    let series = vec![SeriesConfig {
        series_id: "dense".into(),
        source_id: None,
        label: None,
        x_column: "dx".into(),
        y_column: "dy".into(),
        render_type: DataRenderType::Contour {
            matrix: MatrixRef {
                columns: ids,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            contour: contour_config(vec![2.0], None),
        },
    }];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    // 33 samples along the whole isoline `x + y == 2`, every one of which must
    // have ink within a couple of pixels.
    for i in 1..34u32 {
        let x = 2.0 * f64::from(i) / 34.0;
        let y = 2.0 - x;
        assert!(
            near(&img, data_to_px(&chart, x, y), 3, is_blue),
            "the isoline is broken at ({x:.2}, {y:.2})"
        );
    }
}

/// A saddle needs no tie-break rule.
///
/// `z = (x-1)^2 - (y-1)^2` at level 0 is the pair of diagonals through the centre.
/// Marching squares has to resolve that ambiguous cell from some extra sample;
/// the level set of the bilinear form is a hyperbola and degenerates into the two
/// lines on its own.
#[test]
fn a_saddle_needs_no_tie_break() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let coords = vec![0.0, 0.5, 1.0, 1.5, 2.0];
    let columns: Vec<Column<f64>> = coords
        .iter()
        .map(|x| {
            col_f64(
                coords
                    .iter()
                    .map(|y| (x - 1.0) * (x - 1.0) - (y - 1.0) * (y - 1.0))
                    .collect(),
            )
        })
        .collect();
    let ids = ["q0", "q1", "q2", "q3", "q4"];
    let coord_col = col_f64(coords.clone());
    let mut batch: Vec<(&str, &dyn renderer::ColumnSource)> = vec![
        ("qx", &coord_col as &dyn renderer::ColumnSource),
        ("qy", &coord_col as &dyn renderer::ColumnSource),
    ];
    for (id, column) in ids.iter().zip(&columns) {
        batch.push((*id, column as &dyn renderer::ColumnSource));
    }
    renderer.add_columns(&batch).expect("saddle grid upload");

    let chart = bare_chart(colorbar(-1.0, 1.0));
    let series = vec![SeriesConfig {
        series_id: "saddle".into(),
        source_id: None,
        label: None,
        x_column: "qx".into(),
        y_column: "qy".into(),
        render_type: DataRenderType::Contour {
            matrix: matrix(&ids),
            contour: contour_config(vec![0.0], None),
        },
    }];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    // Both diagonals, at four points each, and the centre they cross at.
    for t in [0.3f64, 0.7, 1.3, 1.7] {
        assert!(
            near(&img, data_to_px(&chart, t, t), 4, is_blue),
            "the rising diagonal is missing at ({t}, {t})"
        );
        assert!(
            near(&img, data_to_px(&chart, t, 2.0 - t), 4, is_blue),
            "the falling diagonal is missing at ({t}, {:.1})",
            2.0 - t
        );
    }
    // The crossing itself. The quadratic handles the saddle's mixed curvature
    // even where the gradient vanishes, so the X must close at the centre.
    assert!(
        near(&img, data_to_px(&chart, 1.0, 1.0), 3, is_blue),
        "the two diagonals do not meet at the saddle"
    );
    // And points off both diagonals are empty — a case-table mistake connects the
    // wrong pair of crossings and fills one of these. `|x-1| == |y-1|` *is* the
    // level set, so an interior point has to break that equality.
    for (x, y) in [(0.5f64, 1.0f64), (1.5, 1.0), (1.0, 0.5), (1.0, 1.5)] {
        assert!(
            !near(&img, data_to_px(&chart, x, y), 8, is_blue),
            "ink at ({x}, {y}), which is not on either diagonal"
        );
    }
}

/// `z = x*y` on a 65 x 65 lattice over [0, 2]^2, and the ids it registered.
///
/// **Exactly bilinear**, so the renderer's interpolant reproduces it with zero
/// error — any deviation of the drawn line from the analytic hyperbola is the
/// distance estimator's and nothing else. A separable field like `x^2 + y^2`
/// cannot be used for this: its bilinear cross term is identically zero, so the
/// second-order term vanishes and the measurement would only be reading back
/// interpolation error.
fn hyperbola_fixture(renderer: &mut Renderer) -> Vec<String> {
    const N: usize = 65;
    let coords: Vec<f64> = (0..N).map(|i| 2.0 * i as f64 / (N - 1) as f64).collect();
    let coord_col = col_f64(coords.clone());
    let columns: Vec<Column<f64>> = coords
        .iter()
        .map(|x| col_f64(coords.iter().map(|y| x * y).collect()))
        .collect();
    let ids: Vec<String> = (0..N).map(|c| format!("hy{c}")).collect();
    let mut batch: Vec<(&str, &dyn renderer::ColumnSource)> = Vec::with_capacity(N + 2);
    batch.push(("hyx", &coord_col as &dyn renderer::ColumnSource));
    batch.push(("hyy", &coord_col as &dyn renderer::ColumnSource));
    for (id, column) in ids.iter().zip(&columns) {
        batch.push((id.as_str(), column as &dyn renderer::ColumnSource));
    }
    renderer.add_columns(&batch).expect("hyperbola upload");
    ids
}

/// The drawn isoline sits where the level is, including where the curve turns
/// inside a few pixels.
///
/// Coverage comes from the distance to the isoline along the gradient. z is
/// bilinear inside a cell, so along a straight line it is exactly quadratic and
/// that distance has a closed form; the *first-order* form `f / |grad z|` that a
/// distance field usually settles for is biased by roughly `d^2 / 2R`. Measured
/// on this fixture at the vertex of `x*y = L`, where the curvature radius is
/// `sqrt(2L)` — A/B'd by forcing the curvature term to zero, on a 700 px panel
/// with 7.3 px grid cells:
///
/// ```text
///   R (px)    first order    quadratic root
///    164.4       -0.007          -0.007
///     20.8       +0.030          +0.016
///      9.3       +0.123          +0.092
///      4.65      +0.227          +0.119
/// ```
///
/// Sub-pixel either way — not a visible defect, but a positional bias in the one
/// thing the renderer promises, and it halves where curvature is tightest.
///
/// This test runs a smaller panel (coarser cells relative to a pixel), so its own
/// numbers are larger than the table's; the bound is what the sweep here actually
/// holds, not a restatement of the A/B.
///
/// The sweep stops at a curvature radius of about 5 px. Below that the whole
/// vertex fits inside one grid cell *and* inside the stroke, so "where the line
/// is" stops being a question the picture can answer — a level of 2e-4 measures
/// +1.9 px off, which is the stroke covering the entire feature rather than an
/// estimator failing.
#[test]
fn the_isoline_lands_on_its_level_at_tight_curvature() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let ids = hyperbola_fixture(&mut renderer);
    let mut bar = colorbar(0.0, 4.0);
    // No fill: a transparent ramp leaves the stroke as the only ink. Hidden, so
    // it reserves no band and the data area keeps the panel's own aspect — the
    // measurement is a distance in pixels, and an anisotropic px-per-unit would
    // put the stroke's profile and the reference in different metrics.
    bar.colormap = ColorMap::Custom {
        stops: vec![Color::new(0.0, 0.0, 0.0, 0.0); 2],
    };
    bar.visible = false;
    let mut chart = bare_chart(bar);
    chart.set_x_range(0.0, 2.0);
    chart.set_y_range(0.0, 2.0);
    let da = chart.config().data_area().expect("data area");

    for level in [0.25f64, 4.0e-3, 8.0e-4] {
        let mut contour = contour_config(vec![level], None);
        contour.line.line_width = 1.5;
        let series = vec![SeriesConfig {
            series_id: "hyp".into(),
            source_id: None,
            label: None,
            x_column: "hyx".into(),
            y_column: "hyy".into(),
            render_type: DataRenderType::Contour {
                matrix: MatrixRef {
                    columns: ids.clone(),
                    orientation: MatrixOrientation::ColumnsAreX,
                    grid_layout: GridLayout::Centers,
                },
                contour,
            },
        }];
        let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

        // The row nearest the hyperbola's vertex, and that row's exact crossing.
        // Taking the reference from the row's own data y (a pixel centre) keeps
        // the comparison exact rather than assuming the row lands on the vertex.
        let vertex_t = level.sqrt() / 2.0;
        let row = (f64::from(da.y + da.height) - vertex_t * f64::from(da.height)).round() as u32;
        let y_data = 2.0 * (f64::from(da.y + da.height) - f64::from(row)) / f64::from(da.height);
        let expected = f64::from(da.x) + (level / y_data / 2.0) * f64::from(da.width);

        let (mut weight, mut moment) = (0.0f64, 0.0f64);
        for x in da.x..(da.x + da.width) {
            let p = pixel(&img, x, row);
            if !is_blue(p) {
                continue;
            }
            let cov = f64::from(p[3]) / 255.0;
            weight += cov;
            moment += cov * f64::from(x);
        }
        assert!(weight > 0.0, "no isoline drawn for level {level}");
        let drawn = moment / weight;
        let radius_px = (2.0 * level).sqrt() * f64::from(da.width) / 2.0;
        assert!(
            (drawn - expected).abs() < 0.35,
            "at curvature radius {radius_px:.1} px the level-{level} isoline drew \
             {:+.3} px off its own level",
            drawn - expected
        );
    }
}

// ── Inline level labels ─────────────────────────────────────────────────────
//
// Two inputs, one draw. An empty resolved override makes the GPU project and
// select anchors from the implicit field; a non-empty resolved list supplies the
// records directly. Both fill the same 32 B record and indirect draw.
//
// Attribution for the **override** tests: every explicit anchor sits *off* the
// isoline on purpose, because the label takes its level's colour and ink on the
// line could not otherwise be told from ink of the label.
//
// Attribution for the **automatic** tests: an auto label sits *on* its line by
// construction, so colour cannot separate them. Those tests give the label a
// magenta `bg_color` instead and locate it by that box. Magenta and not green:
// the colourbar strip is drawn in this fixture's red-to-green ramp, so green ink
// is already on the panel before a label is placed.

fn label_config(anchors: Vec<ContourLabelAnchor>, visible: bool) -> ContourLabelConfig {
    ContourLabelConfig {
        visible,
        font_size: 24.0,
        color: LINE_A,
        format: LabelFormat::Decimal,
        significant_digits: 0,
        spacing_px: 80.0,
        anchors,
        bg_color: None,
        bg_padding_px: 0.0,
    }
}

/// Labels the GPU places, found by their magenta background box.
fn auto_label_config(spacing_px: f32) -> ContourLabelConfig {
    ContourLabelConfig {
        visible: true,
        font_size: 24.0,
        color: LINE_A,
        format: LabelFormat::Decimal,
        significant_digits: 0,
        spacing_px,
        anchors: Vec::new(),
        bg_color: Some(LABEL_BG),
        bg_padding_px: 3.0,
    }
}

/// The label background. Nothing else in these charts is magenta: the ramp runs
/// red to green and the lines and digits are blue or white.
const LABEL_BG: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

fn is_magenta(p: &[u8]) -> bool {
    p[3] > 16 && p[0] > 140 && p[2] > 140 && p[1] < 100
}

fn anchor(level_index: u32, x: f64, y: f64, tx: f64, ty: f64) -> ContourLabelAnchor {
    ContourLabelAnchor {
        level_index,
        x,
        y,
        tx,
        ty,
    }
}

fn labelled(
    levels: Vec<f64>,
    colors: Option<Vec<Color>>,
    labels: ContourLabelConfig,
) -> SeriesConfig {
    let mut cfg = contour_only(&["p0", "p1", "p2"], levels, colors);
    if let DataRenderType::Contour { contour, .. } = &mut cfg.render_type {
        contour.labels = Some(labels);
    }
    cfg
}

/// Bounding box of the pixels matching `hit`, as `(x0, y0, x1, y1)`.
fn ink_bounds(img: &RasterImage, hit: impl Fn(&[u8]) -> bool) -> Option<(u32, u32, u32, u32)> {
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for y in 0..img.height {
        for x in 0..img.width {
            if hit(pixel(img, x, y)) {
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    bounds
}

/// A label is drawn at its anchor, in its level's colour, and nowhere else.
#[test]
fn a_contour_label_draws_at_its_anchor() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    // z = 0.8 at (0.4, 0.4), so the level-2 isoline is nowhere near: blue ink
    // there is the label and cannot be the line.
    let series = vec![labelled(
        vec![2.0],
        None,
        label_config(vec![anchor(0, 0.4, 0.4, 1.0, 0.0)], true),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    assert!(
        near(&img, data_to_px(&chart, 0.4, 0.4), 14, is_blue),
        "no label ink at the anchor"
    );
    // And the same chart without the label has none there.
    let bare = vec![contour_only(&["p0", "p1", "p2"], vec![2.0], None)];
    let without = renderer.export_panel_rgba(&chart, &bare, 1.0).unwrap();
    assert!(
        !near(&without, data_to_px(&chart, 0.4, 0.4), 14, is_blue),
        "the anchor position already had ink before the label was added"
    );
}

/// The label runs along its tangent: a horizontal tangent makes a wide, short
/// mark and a vertical one makes a tall, narrow mark, from the same text.
#[test]
fn a_label_rotates_along_its_tangent() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    // Two digits, so the mark is clearly longer than it is tall when upright.
    let mut measure = |tx: f64, ty: f64| {
        let series = vec![labelled(
            vec![22.0],
            None,
            label_config(vec![anchor(0, 1.0, 1.0, tx, ty)], true),
        )];
        let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (x0, y0, x1, y1) = ink_bounds(&img, is_blue).expect("label ink");
        (x1 - x0, y1 - y0)
    };
    let (flat_w, flat_h) = measure(1.0, 0.0);
    let (up_w, up_h) = measure(0.0, 1.0);
    assert!(
        flat_w > flat_h,
        "a horizontal tangent must read across: {flat_w}x{flat_h}"
    );
    assert!(
        up_h > up_w,
        "a vertical tangent must read up the page: {up_w}x{up_h}"
    );
}

/// Hidden labels draw nothing — the anchors are still there, and are still
/// ignored.
#[test]
fn an_invisible_label_draws_nothing() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let labels = label_config(vec![anchor(0, 0.4, 0.4, 1.0, 0.0)], false);
    let series = vec![labelled(vec![2.0], None, labels)];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert!(
        !near(&img, data_to_px(&chart, 0.4, 0.4), 14, is_blue),
        "a label was drawn where none was asked for"
    );
}

/// An anchor naming a level the declaration no longer has is stale, not fatal:
/// it draws nothing and its neighbours still draw.
#[test]
fn an_anchor_naming_a_missing_level_is_skipped() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let series = vec![labelled(
        vec![2.0],
        None,
        label_config(
            vec![
                // z = 1.3 here, so the level-2 isoline is well clear and any
                // ink would have to be the stale anchor's label.
                anchor(7, 0.3, 1.0, 1.0, 0.0),
                anchor(0, 0.4, 0.4, 1.0, 0.0),
            ],
            true,
        ),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert!(
        !near(&img, data_to_px(&chart, 0.3, 1.0), 14, is_blue),
        "an anchor for level 7 of a one-level declaration drew something"
    );
    assert!(
        near(&img, data_to_px(&chart, 0.4, 0.4), 14, is_blue),
        "the valid anchor beside it must still draw"
    );
}

/// If every explicit anchor is stale, the resolved override is empty and the
/// automatic placement path remains active.
#[test]
fn all_stale_anchors_fall_back_to_automatic_placement() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let mut labels = label_config(vec![anchor(7, 0.3, 1.0, 1.0, 0.0)], true);
    labels.bg_color = Some(LABEL_BG);
    labels.bg_padding_px = 3.0;
    let series = vec![labelled(vec![2.0], None, labels)];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    assert!(
        ink_bounds(&img, is_magenta).is_some(),
        "an empty resolved override must leave automatic label placement active"
    );
}

/// Explicit anchors are resolved in declaration order and capped at the shared
/// 1024-label draw capacity. The 1025th valid anchor must not replace an earlier
/// one.
#[test]
fn explicit_anchors_keep_the_first_1024_in_input_order() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let filler = (0..1023).map(|_| anchor(0, 0.3, 0.3, 1.0, 0.0));
    let kept_1024th = std::iter::once(anchor(0, 1.0, 0.7, 1.0, 0.0));
    let dropped_1025th = std::iter::once(anchor(0, 1.7, 1.7, 1.0, 0.0));
    let mut labels = label_config(
        filler.chain(kept_1024th).chain(dropped_1025th).collect(),
        true,
    );
    labels.bg_color = Some(LABEL_BG);
    labels.bg_padding_px = 3.0;
    let series = vec![labelled(vec![2.0], None, labels)];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    assert!(
        near(&img, data_to_px(&chart, 1.0, 0.7), 16, is_magenta),
        "the distinguishable 1024th anchor must remain drawable"
    );
    assert!(
        !near(&img, data_to_px(&chart, 1.7, 1.7), 16, is_magenta),
        "the 1025th anchor must be dropped instead of replacing an earlier one"
    );
}

/// A one-cell automatic lattice projects one candidate for every level. The
/// first 256 levels occupy only the low end of the diagonal; the remaining 768
/// reach its far end. A 256-cap regression therefore matches the baseline there,
/// while the 1024 contract paints an unmistakable background box.
#[test]
fn automatic_labels_keep_all_1024_levels() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let all_levels: Vec<f64> = (0..renderer::MAX_CONTOUR_LEVELS)
        .map(|index| 0.1 + 3.8 * index as f64 / (renderer::MAX_CONTOUR_LEVELS - 1) as f64)
        .collect();
    let render = |renderer: &mut Renderer, levels: Vec<f64>| {
        let count = levels.len();
        let transparent = Color::new(0.0, 0.0, 0.0, 0.0);
        let mut labels = auto_label_config(2000.0);
        labels.font_size = 8.0;
        labels.bg_color = Some(LABEL_BG);
        labels.bg_padding_px = 2.0;
        let series = [labelled(levels, Some(vec![transparent; count]), labels)];
        renderer.export_panel_rgba(&chart, &series, 1.0).unwrap()
    };
    let baseline = render(&mut renderer, all_levels[..256].to_vec());
    let full = render(&mut renderer, all_levels);
    let baseline_bounds = ink_bounds(&baseline, is_magenta).expect("256-label baseline");
    let full_bounds = ink_bounds(&full, is_magenta).expect("1024-label output");
    assert!(
        full_bounds.0 == baseline_bounds.0
            && full_bounds.2 > baseline_bounds.2 + 80
            && full_bounds.1 + 60 < baseline_bounds.1
            && full_bounds.3 == baseline_bounds.3,
        "automatic placement did not extend beyond the first 256 levels: \
         baseline={baseline_bounds:?}, full={full_bounds:?}"
    );
}

/// Label typography owns one colour independently of the stroke palette.
#[test]
fn a_label_color_is_independent_of_per_level_stroke_colors() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let mut labels = label_config(
        vec![anchor(0, 1.6, 0.2, 1.0, 0.0), anchor(1, 0.3, 0.3, 1.0, 0.0)],
        true,
    );
    labels.color = LINE_B;
    let series = vec![labelled(
        vec![1.0, 3.0],
        Some(vec![LINE_A, LABEL_BG]),
        labels,
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    // Neither anchor sits on an isoline (z = 1.8 and z = 0.6 there), and both
    // labels stay white despite two unrelated per-level stroke colours.
    assert!(
        near(&img, data_to_px(&chart, 1.6, 0.2), 14, is_white),
        "level 0's label did not use ContourLabelConfig.color"
    );
    assert!(
        near(&img, data_to_px(&chart, 0.3, 0.3), 14, is_white),
        "level 1's label did not use ContourLabelConfig.color"
    );
}

/// The label text comes from the *level value*, not from the anchor's index: a
/// negative level carries its sign, so its mark is wider than the same digits
/// without one.
#[test]
fn a_label_reads_its_level_value() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(-4.0, 4.0));
    let width_of = |renderer: &mut Renderer, level: f64| {
        let series = vec![labelled(
            vec![level],
            None,
            label_config(vec![anchor(0, 1.0, 1.0, 1.0, 0.0)], true),
        )];
        let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (x0, _, x1, _) = ink_bounds(&img, is_blue).expect("label ink");
        x1 - x0
    };
    // Level -8 has no isoline on this field (z runs 0..4), so all the ink is the
    // label — and likewise for 8.
    let negative = width_of(&mut renderer, -8.0);
    let positive = width_of(&mut renderer, 8.0);
    assert!(
        negative > positive,
        "a negative level's label must carry its sign: {negative} vs {positive}"
    );
}

// ── Automatic anchors ───────────────────────────────────────────────────────

/// A pixel's data-space position — the inverse of `data_to_px`, for asking
/// *where* the GPU put a label rather than only whether it drew one.
fn px_to_data(chart: &Chart, px: (u32, u32)) -> (f64, f64) {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let tx = (px.0 as f64 - da.x as f64) / da.width as f64;
    let ty = ((da.y + da.height) as f64 - px.1 as f64) / da.height as f64;
    (
        cfg.bottom_x.min + tx * (cfg.bottom_x.max - cfg.bottom_x.min),
        cfg.left_y.min + ty * (cfg.left_y.max - cfg.left_y.min),
    )
}

/// An automatically placed label sits **on** the line it names.
///
/// `z = x + y`, level 2, so the isoline is the anti-diagonal `x + y == 2` and the
/// centre of the label's background box has to land on it. `spacing_px` is wider
/// than the panel, so the seed lattice is 1 x 1 and exactly one label is placed —
/// which is what makes the box's centre a single well-defined point.
#[test]
fn an_automatic_anchor_lands_on_its_isoline() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let series = vec![labelled(vec![2.0], None, auto_label_config(2000.0))];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    let (x0, y0, x1, y1) = ink_bounds(&img, is_magenta).expect("no automatic label was placed");
    let centre = ((x0 + x1) / 2, (y0 + y1) / 2);
    let (x, y) = px_to_data(&chart, centre);
    // The box is about 40 x 30 px, i.e. 0.2 x 0.15 in data units, so its centre
    // can be offset from the projected anchor by half the label's own size.
    assert!(
        (x + y - 2.0).abs() < 0.4,
        "label centre at ({x:.2}, {y:.2}) is not on x + y = 2"
    );
}

/// `z = x + y` again, on a 9 x 9 lattice.
///
/// The 3 x 3 fixture has only four cells, so its isoline crosses very few of them
/// and a seed lattice has almost nothing to choose between. A spacing test needs
/// enough candidates along the line that the select pass' minimum distance is the
/// thing deciding the result. Returns the constituent column ids.
fn fine_plane_fixture(renderer: &mut Renderer) -> Vec<String> {
    const N: usize = 9;
    let coords: Vec<f64> = (0..N).map(|i| i as f64 * 0.25).collect();
    let coord_col = col_f64(coords.clone());
    let columns: Vec<Column<f64>> = coords
        .iter()
        .map(|x| col_f64(coords.iter().map(|y| x + y).collect()))
        .collect();
    let ids: Vec<String> = (0..N).map(|c| format!("f{c}")).collect();
    let mut batch: Vec<(&str, &dyn renderer::ColumnSource)> = Vec::with_capacity(N + 2);
    batch.push(("fx", &coord_col as &dyn renderer::ColumnSource));
    batch.push(("fy", &coord_col as &dyn renderer::ColumnSource));
    for (id, column) in ids.iter().zip(&columns) {
        batch.push((id.as_str(), column as &dyn renderer::ColumnSource));
    }
    renderer.add_columns(&batch).expect("fine grid upload");
    ids
}

fn fine_labelled(ids: &[String], levels: Vec<f64>, labels: ContourLabelConfig) -> SeriesConfig {
    let mut contour = contour_config(levels, None);
    contour.labels = Some(labels);
    SeriesConfig {
        series_id: "fine".into(),
        source_id: None,
        label: None,
        x_column: "fx".into(),
        y_column: "fy".into(),
        render_type: DataRenderType::Contour {
            matrix: MatrixRef {
                columns: ids.to_vec(),
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            contour,
        },
    }
}

/// Pixel-weighted centroids of the magenta labels.
///
/// A cluster count and real 2D positions, not a bounding box: the question is how
/// many labels were placed and whether they are far enough apart, which a bbox
/// cannot answer.
///
/// Blobs whose centroids fall within `MERGE_PX` are one label. A rotated
/// background box antialiases into pieces that 4-connectivity separates, and its
/// digits punch holes through it — but nothing legitimate puts two *labels* that
/// close, because the separation being measured is far larger.
fn magenta_clusters(img: &RasterImage) -> Vec<(f32, f32)> {
    const MERGE_PX: f32 = 30.0;
    let mut blobs = magenta_blobs(img);
    let mut merged: Vec<((f32, f32), f32)> = Vec::new();
    blobs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (c, n) in blobs {
        match merged
            .iter_mut()
            .find(|(m, _)| ((m.0 - c.0).powi(2) + (m.1 - c.1).powi(2)).sqrt() < MERGE_PX)
        {
            Some((m, w)) => {
                let total = *w + n;
                m.0 = (m.0 * *w + c.0 * n) / total;
                m.1 = (m.1 * *w + c.1 * n) / total;
                *w = total;
            }
            None => merged.push((c, n)),
        }
    }
    merged.into_iter().map(|(c, _)| c).collect()
}

/// Each connected magenta blob as `(centroid, pixel count)`.
fn magenta_blobs(img: &RasterImage) -> Vec<((f32, f32), f32)> {
    let w = img.width as usize;
    let h = img.height as usize;
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    for y0 in 0..h {
        for x0 in 0..w {
            if seen[y0 * w + x0] || !is_magenta(pixel(img, x0 as u32, y0 as u32)) {
                continue;
            }
            let mut stack = vec![(x0, y0)];
            seen[y0 * w + x0] = true;
            let (mut sx, mut sy, mut n) = (0f64, 0f64, 0f64);
            while let Some((x, y)) = stack.pop() {
                sx += x as f64;
                sy += y as f64;
                n += 1.0;
                let mut push = |nx: usize, ny: usize, stack: &mut Vec<(usize, usize)>| {
                    if nx < w
                        && ny < h
                        && !seen[ny * w + nx]
                        && is_magenta(pixel(img, nx as u32, ny as u32))
                    {
                        seen[ny * w + nx] = true;
                        stack.push((nx, ny));
                    }
                };
                if x + 1 < w {
                    push(x + 1, y, &mut stack);
                }
                if x > 0 {
                    push(x - 1, y, &mut stack);
                }
                if y + 1 < h {
                    push(x, y + 1, &mut stack);
                }
                if y > 0 {
                    push(x, y - 1, &mut stack);
                }
            }
            out.push((((sx / n) as f32, (sy / n) as f32), n as f32));
        }
    }
    out
}

/// `spacing_px` is the normal sweep's target minimum separation, and a finer one
/// places more labels. The per-level fallback may violate it to avoid omission.
///
/// This is the guarantee the bucket-based design did not have: winners came from
/// independent grid cells, so two of them could sit adjacent across a shared
/// boundary. The Newton-projected candidates go through one serial pass that
/// rejects anything closer than `spacing_px` (or closer than the two label boxes'
/// half-diagonals, whichever is larger), so the distance below is a real bound and
/// not a hope.
///
/// Still not arc-length-even along the line — the seed lattice is even, the curve
/// is not (design B.4.9 records that, and the predictor-corrector march that would
/// fix it is left as a follow-up).
#[test]
fn spacing_px_separates_automatic_labels() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let ids = fine_plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let clusters = |renderer: &mut Renderer, spacing: f32| {
        let series = vec![fine_labelled(&ids, vec![2.0], auto_label_config(spacing))];
        let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
        magenta_clusters(&img)
    };
    // Wider than the panel: the lattice saturates at 1 x 1, so one label.
    let coarse = clusters(&mut renderer, 2000.0);
    assert_eq!(
        coarse.len(),
        1,
        "one lattice cell must place exactly one label, got {coarse:?}"
    );

    // A pitch several times the label box, so two labels' ink cannot be confused
    // for one another's when the blobs are merged.
    let pitch = 110.0f32;
    let fine = clusters(&mut renderer, pitch);
    assert!(
        fine.len() > coarse.len(),
        "a finer pitch must place more labels: {} vs {}",
        fine.len(),
        coarse.len()
    );
    for (i, a) in fine.iter().enumerate() {
        for b in fine.iter().skip(i + 1) {
            let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
            assert!(
                d >= pitch * 0.85,
                "two labels are {d:.1} px apart, under the {pitch} px minimum: {fine:?}"
            );
        }
    }
}

/// A non-empty anchor list overrides the automatic pass entirely.
///
/// The override is placed where no isoline is, and the point the automatic pass
/// would have chosen — the middle of the line — must come out empty. Getting both
/// would mean two placement paths were running.
#[test]
fn explicit_anchors_override_the_automatic_pass() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let mut labels = auto_label_config(2000.0);
    // z = 0.8 at (0.4, 0.4): well clear of the level-2 isoline.
    labels.anchors = vec![anchor(0, 0.4, 0.4, 1.0, 0.0)];
    let series = vec![labelled(vec![2.0], None, labels)];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    assert!(
        near(&img, data_to_px(&chart, 0.4, 0.4), 20, is_magenta),
        "the explicit anchor was not drawn"
    );
    for (x, y) in [(1.0f64, 1.0f64), (1.75, 0.25), (0.25, 1.75)] {
        assert!(
            !near(&img, data_to_px(&chart, x, y), 14, is_magenta),
            "an automatic label was placed at ({x}, {y}) despite the override"
        );
    }
}

/// `bg_color` paints behind the text and `None` leaves it transparent.
///
/// The same anchor, the same string, twice: with a background the box is there,
/// without it nothing but the digits are.
#[test]
fn bg_color_paints_behind_the_label() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let at = data_to_px(&chart, 0.4, 0.4);

    let mut with_bg = auto_label_config(2000.0);
    with_bg.anchors = vec![anchor(0, 0.4, 0.4, 1.0, 0.0)];
    let series = vec![labelled(vec![2.0], None, with_bg)];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert!(
        near(&img, at, 20, is_magenta),
        "bg_color painted nothing behind the label"
    );

    let series = vec![labelled(
        vec![2.0],
        None,
        label_config(vec![anchor(0, 0.4, 0.4, 1.0, 0.0)], true),
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert!(
        near(&img, at, 20, is_blue),
        "the label itself must still draw without a background"
    );
    assert!(
        !near(&img, at, 20, is_magenta),
        "a background was painted without bg_color"
    );
}

/// The line itself is absent under a transparent label; readability does not
/// depend on painting an opaque rectangle over an already-drawn stroke.
#[test]
fn contour_line_is_interrupted_under_a_transparent_label() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let at = data_to_px(&chart, 1.0, 1.0); // z = 2, exactly on the isoline.

    let baseline = renderer
        .export_panel_rgba(
            &chart,
            &[contour_only(&["p0", "p1", "p2"], vec![2.0], None)],
            1.0,
        )
        .unwrap();
    assert!(
        near(&baseline, at, 2, is_blue),
        "fixture line missed anchor"
    );

    let mut labels = label_config(vec![anchor(0, 1.0, 1.0, 1.0, -1.0)], true);
    labels.color = Color::new(0.0, 0.0, 0.0, 0.0);
    labels.bg_padding_px = 3.0;
    let labelled = renderer
        .export_panel_rgba(&chart, &[labelled(vec![2.0], None, labels)], 1.0)
        .unwrap();
    assert!(
        !near(&labelled, at, 2, is_blue),
        "contour stroke remained under the transparent label quad"
    );
}

/// Two renders of the same input are **byte-identical**.
///
/// Fixed candidate slots and one serial selection pass make the result independent
/// of GPU scheduling, so two otherwise identical frames must match exactly.
#[test]
fn automatic_label_placement_is_deterministic() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let build = || {
        vec![labelled(
            vec![1.0, 2.0, 3.0],
            None,
            auto_label_config(120.0),
        )]
    };
    let first = renderer.export_panel_rgba(&chart, &build(), 1.0).unwrap();
    let second = renderer.export_panel_rgba(&chart, &build(), 1.0).unwrap();
    assert!(
        ink_bounds(&first, is_magenta).is_some(),
        "the fixture drew no labels, so the comparison would prove nothing"
    );
    assert_eq!(
        first.rgba, second.rgba,
        "two renders of the same labelled contour differ"
    );
}

/// A new anchor key dispatches once; exact-key frames reuse the built buffers.
///
/// Placement is screen-space, so it cannot be cached across a pan — but nothing
/// it writes to may be reallocated per frame either. The `ContourScratch` row
/// carries candidate and selected-anchor buffers, indirect args, params and the
/// atlas, so its creation count answers both questions at once.
#[test]
fn re_dispatching_anchors_allocates_nothing() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    plane_fixture(&mut renderer);
    let chart = bare_chart(colorbar(0.0, 4.0));
    let build = || vec![labelled(vec![2.0], None, auto_label_config(120.0))];
    let _ = renderer.export_panel_rgba(&chart, &build(), 1.0).unwrap();
    let after_first = renderer
        .gpu_memory_usage()
        .creations_of(renderer::GpuResourceKind::ContourScratch);
    for _ in 0..3 {
        let _ = renderer.export_panel_rgba(&chart, &build(), 1.0).unwrap();
    }
    let after_more = renderer
        .gpu_memory_usage()
        .creations_of(renderer::GpuResourceKind::ContourScratch);
    assert_eq!(
        after_first,
        after_more,
        "three more frames of anchor dispatch created {} more objects",
        after_more - after_first
    );
}

/// Probe of the field panel `examples/winit_simple.rs` builds — the same 40 x 40
/// Gaussian bump, uploaded as one batch, filled, contoured and labelled.
///
/// Its job is the integration the unit fixtures above are too small to show: a
/// real grid, forty columns through `add_columns`, interpolated shading, four
/// levels and their labels, all in one panel. It writes a PNG rather than
/// asserting a colour, so the windowed demo's look is inspectable without a
/// window.
#[test]
fn field_panel_probe_matches_the_winit_demo() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    const G: usize = 40;
    const SPREAD: f64 = 0.05;
    let coords: Vec<f64> = (0..G).map(|i| i as f64 / (G - 1) as f64).collect();
    let bump = |x: f64, y: f64| {
        let (dx, dy) = (x - 0.5, y - 0.5);
        (-(dx * dx + dy * dy) / SPREAD).exp()
    };
    let grid: Vec<Column<f64>> = coords
        .iter()
        .map(|x| col_f64(coords.iter().map(|y| bump(*x, *y)).collect()))
        .collect();
    let ids: Vec<String> = (0..G).map(|c| format!("bz{c}")).collect();
    let coord_col = col_f64(coords.clone());
    let mut batch: Vec<(&str, &dyn renderer::ColumnSource)> = Vec::with_capacity(G + 2);
    batch.push(("bx", &coord_col as &dyn renderer::ColumnSource));
    batch.push(("by", &coord_col as &dyn renderer::ColumnSource));
    for (id, column) in ids.iter().zip(&grid) {
        batch.push((id.as_str(), column as &dyn renderer::ColumnSource));
    }
    let creations_before = renderer
        .gpu_memory_usage()
        .creations_of(renderer::GpuResourceKind::ColumnPool);
    renderer.add_columns(&batch).expect("grid batch upload");
    let creations = renderer
        .gpu_memory_usage()
        .creations_of(renderer::GpuResourceKind::ColumnPool)
        - creations_before;
    assert!(
        creations <= 1,
        "42 columns must upload through one staging buffer, not {creations}"
    );

    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.min = 0.0;
    bar.axis.max = 1.0;
    bar.axis.major_spacing = 0.25;
    let mut chart = bare_chart(bar);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);

    // Levels only — where the labels go is the anchor pass' answer, not this
    // fixture's. The earlier version of this probe inverted the Gaussian to place
    // them, which only worked because it knew the closed form of its own data.
    let levels = vec![0.2f64, 0.4, 0.6, 0.8];
    let series = vec![SeriesConfig {
        series_id: "bump".into(),
        source_id: None,
        label: None,
        x_column: "bx".into(),
        y_column: "by".into(),
        render_type: DataRenderType::HeatmapContour {
            matrix: MatrixRef {
                columns: ids,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            },
            contour: ContourConfig {
                levels,
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::new(0.08, 0.08, 0.08, 1.0),
                    line_width: 1.5,
                },
                per_level_color: None,
                labels: Some(ContourLabelConfig {
                    visible: true,
                    font_size: 13.0,
                    color: Color::BLACK,
                    format: LabelFormat::Decimal,
                    significant_digits: 1,
                    spacing_px: 120.0,
                    anchors: Vec::new(),
                    bg_color: Some(Color::new(0.98, 0.98, 0.98, 1.0)),
                    bg_padding_px: 2.0,
                }),
            },
        },
    }];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let dir = std::env::var("FIGGY_PROBE_DIR").unwrap_or_else(|_| ".".to_string());
    let png = encode_png(&img).expect("encode");
    std::fs::write(format!("{dir}/probe_contour_demo_panel.png"), png).expect("write probe");
}
