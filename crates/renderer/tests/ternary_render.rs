//! Ternary coordinates (`Config.ternary`) through the public renderer API:
//! the headless export, the GPU picker, the highlight, the fit, the
//! decoration drags and the hit map.
//!
//! House pattern (mirrors `polar_render.rs`): one process-wide device, a
//! renderer per test, and assertions that name the pixel a sample lands on
//! through `Config::ternary_geometry`, so a wrong corner, normalization or
//! component order fails rather than passes.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::{DataSelectionsConfig, PickedDataRef, TickVisibility};
use renderer::data::Column;
use renderer::data_config::LineInterpolation;
use renderer::layout::{ChartArea, Element, NudgeResult, Rect, Side};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, Color, Config, CpuTextMeasure, DataLineStyleConfig, DataRenderType,
    DataScatterStyleConfig, FitExtent, HitMap, LayerRef, MeasureText, RasterImage, Renderer,
    RendererDevice, ScatterShape, SeriesConfig, Subplot, TernaryOptions,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 420;

const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

const MAGENTA: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

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

/// A ternary chart over 0..100 with no grid, legend, axes or labels, so the
/// only red ink is the series.
fn bare_config() -> Config {
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
    for axis in [
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.min = 0.0;
        axis.max = 100.0;
        axis.major_spacing = 20.0;
        axis.label_style.visible = false;
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.title_option.visible = false;
    }
    config.ternary = Some(TernaryOptions {});
    config
}

/// `bare_config` with the three edges, their ticks, labels and titles
/// shown, as a host would draw it.
fn dressed_config() -> Config {
    let mut config = bare_config();
    for (axis, title) in [
        (&mut config.bottom_x, "A"),
        (&mut config.left_y, "B"),
        (&mut config.right_y, "C"),
    ] {
        axis.label_style.visible = true;
        axis.label_style.label_visible = true;
        axis.line_visible = true;
        axis.tick = TickVisibility::Outside;
        axis.title_option.visible = true;
        axis.title_option.text = renderer::text::RichText::plain(
            title,
            axis.title_option.text.color,
            axis.title_option.text.font_size,
            axis.title_option.text.font.clone(),
        );
    }
    config
}

fn marker_style(size: f32) -> DataScatterStyleConfig {
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

fn line_style(width: f32, dash: LineStylePreset) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: dash,
        line_color: RED,
        line_width: width,
        fill: None,
        fill_color: None,
        interpolation: LineInterpolation::Linear,
    }
}

fn ternary(
    id: &str,
    scatter: Option<DataScatterStyleConfig>,
    line: Option<DataLineStyleConfig>,
) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "a".into(),
        y_column: "b".into(),
        render_type: DataRenderType::Ternary {
            c_column: "c".into(),
            scatter,
            line,
        },
    }
}

fn upload(renderer: &mut Renderer, rows: &[[f64; 3]]) {
    for id in ["a", "b", "c"] {
        renderer.remove_column(id).expect("remove");
    }
    let column = |k: usize| col(rows.iter().map(|r| r[k]).collect());
    renderer
        .add_columns(&[
            ("a", &column(0) as &dyn renderer::ColumnSource),
            ("b", &column(1) as &dyn renderer::ColumnSource),
            ("c", &column(2) as &dyn renderer::ColumnSource),
        ])
        .expect("upload");
}

/// The pixel the components `[a, b, c]` land on, from the same triangle the
/// renderer draws into, at `scale`.
fn ternary_px(config: &Config, row: [f64; 3], scale: f32) -> (f64, f64) {
    let g = config
        .scaled(scale)
        .ternary_geometry()
        .expect("ternary chart");
    let sum = row[0] + row[1] + row[2];
    let p = g.point([
        (row[0] / sum) as f32,
        (row[1] / sum) as f32,
        (row[2] / sum) as f32,
    ]);
    (f64::from(p[0]), f64::from(p[1]))
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

fn at(img: &RasterImage, (x, y): (f64, f64)) -> &[u8] {
    pixel(
        img,
        (x.floor() as i64).clamp(0, img.width as i64 - 1) as u32,
        (y.floor() as i64).clamp(0, img.height as i64 - 1) as u32,
    )
}

fn is_red(p: &[u8]) -> bool {
    p[3] > 180 && p[0] > 180 && p[1] < 80 && p[2] < 80
}

fn is_magenta(p: &[u8]) -> bool {
    p[3] > 180 && p[0] > 180 && p[1] < 80 && p[2] > 180
}

fn count(img: &RasterImage, pred: fn(&[u8]) -> bool) -> usize {
    img.rgba.chunks_exact(4).filter(|p| pred(p)).count()
}

fn export(
    renderer: &mut Renderer,
    config: &Config,
    series: &[SeriesConfig],
    scale: f32,
) -> RasterImage {
    renderer
        .export_panel_rgba(&Chart::new(config.clone()), series, scale)
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

/// The GPU pick at `at` (canvas pixels at `scale`) of a chart registered
/// with `series`.
fn pick(
    r: &mut Renderer,
    config: &Config,
    series: &[SeriesConfig],
    scale: f32,
    at: (f64, f64),
    reach: f32,
) -> Option<renderer::PickedData> {
    r.enable_gpu_picking().unwrap();
    let id = r.register_chart(config.clone(), series.to_vec()).unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0 as f32, at.1 as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: (WIDTH as f32 * scale) as u32,
            height: (HEIGHT as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: reach,
    };
    let hit = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    hit
}

fn picked_point(hit: Option<renderer::PickedData>) -> Option<(String, usize)> {
    match hit {
        Some(renderer::PickedData::Point {
            series_id,
            point_index,
            ..
        }) => Some((series_id, point_index)),
        None => None,
        Some(other) => panic!("picked {other:?}"),
    }
}

/// Unnormalized rows: the corners, the edge midpoints and the centroid at
/// sums of 1, 100 and 7.3 land where their fractions place them, at 1x and
/// 2x, and nowhere a rectangular chart would put them.
#[test]
fn points_land_at_their_normalized_position() {
    let mut r = renderer();
    let base: [[f64; 3]; 7] = [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.5, 0.5, 0.0],
        [0.0, 0.5, 0.5],
        [0.5, 0.0, 0.5],
        [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
    ];
    let config = bare_config();
    let series = [ternary("points", Some(marker_style(8.0)), None)];
    for sum in [1.0, 100.0, 7.3] {
        let rows: Vec<[f64; 3]> = base
            .iter()
            .map(|r| [r[0] * sum, r[1] * sum, r[2] * sum])
            .collect();
        upload(&mut r, &rows);
        for scale in [1.0f32, 2.0] {
            let img = export(&mut r, &config, &series, scale);
            save(&format!("ternary-points-{sum}-{scale}x"), &img);
            for row in &rows {
                let p = ternary_px(&config, *row, scale);
                assert!(
                    is_red(at(&img, p)),
                    "sum {sum} at {scale}x: {row:?} should draw at {p:?}, got {:?}",
                    at(&img, p)
                );
            }
            // Seven whole markers and nothing else.
            let ink = count(&img, is_red) as f32 / (scale * scale);
            assert!(
                (7.0 * 100.0..7.0 * 260.0).contains(&ink),
                "sum {sum}: {ink}"
            );
        }
    }
    // A rectangular chart would put (1, 0) at the bottom-right corner of
    // the data area, far from any corner of the triangle.
    let img = export(&mut r, &config, &series, 1.0);
    let area = config.data_area().unwrap().0;
    let corner = ((area.x + area.width) as f64 - 3.0, area.y as f64 + 3.0);
    assert!(!is_red(at(&img, corner)));
}

/// A sample with a negative or non-finite component, or a zero sum, is not
/// drawn and breaks the line on both of its sides (D4).
#[test]
fn invalid_samples_are_masked_and_break_the_line() {
    let mut r = renderer();
    let rows = [
        [60.0, 20.0, 20.0],
        [20.0, 60.0, 20.0],
        [-5.0, 50.0, 55.0],
        [20.0, 20.0, 60.0],
        [f64::NAN, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [40.0, 40.0, 20.0],
    ];
    upload(&mut r, &rows);
    let config = bare_config();
    let series = [ternary(
        "path",
        Some(marker_style(6.0)),
        Some(line_style(3.0, LineStylePreset::Solid)),
    )];
    let img = export(&mut r, &config, &series, 1.0);
    save("ternary-masked", &img);
    let mid = |a: [f64; 3], b: [f64; 3]| {
        let (pa, pb) = (ternary_px(&config, a, 1.0), ternary_px(&config, b, 1.0));
        ((pa.0 + pb.0) * 0.5, (pa.1 + pb.1) * 0.5)
    };
    assert!(
        is_red(at(&img, mid(rows[0], rows[1]))),
        "the first segment draws"
    );
    for k in [0, 1, 3, 6] {
        assert!(
            is_red(at(&img, ternary_px(&config, rows[k], 1.0))),
            "row {k}"
        );
    }
    // The segments to and from the negative row are not drawn.
    assert!(
        !is_red(at(&img, mid(rows[1], rows[3]))),
        "no chord across the gap"
    );
    assert!(
        !is_red(at(&img, mid(rows[3], rows[6]))),
        "no chord across NaN and 0"
    );
}

/// A segment is the straight chord between its ends (placement is a
/// projective map), and a dashed line keeps its gaps along it.
#[test]
fn segments_are_straight_chords_and_dashes_keep_their_gaps() {
    let mut r = renderer();
    let rows = [[70.0, 10.0, 20.0], [10.0, 70.0, 20.0], [10.0, 10.0, 80.0]];
    upload(&mut r, &rows);
    let config = bare_config();
    let solid = export(
        &mut r,
        &config,
        &[ternary(
            "path",
            None,
            Some(line_style(3.0, LineStylePreset::Solid)),
        )],
        1.0,
    );
    let dashed = export(
        &mut r,
        &config,
        &[ternary(
            "path",
            None,
            Some(line_style(3.0, LineStylePreset::Dash)),
        )],
        1.0,
    );
    save("ternary-solid", &solid);
    save("ternary-dashed", &dashed);
    for (a, b) in [(rows[0], rows[1]), (rows[1], rows[2])] {
        let (pa, pb) = (ternary_px(&config, a, 1.0), ternary_px(&config, b, 1.0));
        for t in [0.25, 0.5, 0.75] {
            let p = (pa.0 + (pb.0 - pa.0) * t, pa.1 + (pb.1 - pa.1) * t);
            assert!(is_red(at(&solid, p)), "chord at {t}: {p:?}");
        }
    }
    let (full, dashes) = (count(&solid, is_red), count(&dashed, is_red));
    assert!(
        dashes * 10 > full * 3 && dashes * 10 < full * 9,
        "dashes {dashes} of {full}"
    );
}

/// The ternary grid draws a line of constant value at each major tick of
/// each axis, parallel to the edge it does not meet.
#[test]
fn grid_lines_are_parallel_to_each_edge() {
    let mut r = renderer();
    upload(&mut r, &[[1.0, 1.0, 1.0]]);
    let mut config = bare_config();
    config.grid.show_major_x = true;
    config.grid.show_major_y = true;
    config.grid.major_x_color = RED;
    config.grid.major_y_color = RED;
    config.grid.major_x_width = 2.0;
    config.grid.major_y_width = 2.0;
    let img = export(
        &mut r,
        &config,
        &[ternary(
            "p",
            None,
            Some(line_style(1.0, LineStylePreset::Solid)),
        )],
        1.0,
    );
    save("ternary-grid", &img);
    // On the line of 40 % of each component, a quarter of the way along.
    for k in 0..3 {
        let mut a = [0.0; 3];
        let mut b = [0.0; 3];
        a[k] = 40.0;
        b[k] = 40.0;
        a[(k + 1) % 3] = 60.0;
        b[(k + 2) % 3] = 60.0;
        let (pa, pb) = (ternary_px(&config, a, 1.0), ternary_px(&config, b, 1.0));
        let p = (pa.0 + (pb.0 - pa.0) * 0.25, pa.1 + (pb.1 - pa.1) * 0.25);
        assert!(
            is_red(at(&img, p)),
            "component {k} at {p:?}: {:?}",
            at(&img, p)
        );
    }
    // Between the 40 and 60 % lines of every family: no grid.
    let centre = ternary_px(&config, [50.0, 25.0, 25.0], 1.0);
    assert!(!is_red(at(&img, (centre.0 + 0.0, centre.1 + 12.0))));
}

/// Markers and lines pick where they paint, at 1x and 2x: the marker by its
/// index, a point on the chord by the nearer end, and nothing off it.
#[test]
fn markers_and_lines_pick_where_they_paint() {
    let mut r = renderer();
    let rows = [[70.0, 10.0, 20.0], [10.0, 70.0, 20.0], [5.0, 5.0, 2.0]];
    upload(&mut r, &rows);
    let config = bare_config();
    let markers = [ternary("points", Some(marker_style(10.0)), None)];
    let path = [ternary(
        "path",
        None,
        Some(line_style(3.0, LineStylePreset::Solid)),
    )];
    for scale in [1.0f32, 2.0] {
        for (k, row) in rows.iter().enumerate() {
            let p = ternary_px(&config, *row, scale);
            assert_eq!(
                picked_point(pick(&mut r, &config, &markers, scale, p, 0.0)),
                Some(("points".into(), k)),
                "{scale}x marker {k}"
            );
        }
        let (a, b) = (
            ternary_px(&config, rows[0], scale),
            ternary_px(&config, rows[1], scale),
        );
        let near_a = (a.0 + (b.0 - a.0) * 0.3, a.1 + (b.1 - a.1) * 0.3);
        assert_eq!(
            picked_point(pick(&mut r, &config, &path, scale, near_a, 1.0)),
            Some(("path".into(), 0)),
            "{scale}x chord"
        );
        // 20 px off the chord, across it.
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dy);
        let off = (
            near_a.0 - dy / len * 20.0 * f64::from(scale),
            near_a.1 + dx / len * 20.0 * f64::from(scale),
        );
        assert_eq!(
            picked_point(pick(&mut r, &config, &path, scale, off, 3.0)),
            None
        );
    }
}

/// The picked point's ring is drawn around its marker where the marker
/// paints, at 1x and 2x.
#[test]
fn the_picked_point_ring_is_on_its_marker() {
    let mut r = renderer();
    let rows = [[70.0, 10.0, 20.0], [10.0, 70.0, 20.0], [20.0, 20.0, 60.0]];
    upload(&mut r, &rows);
    let mut config = bare_config();
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "points".into(),
            point_index: 1,
        }],
        highlight_color: MAGENTA,
        outline_width_px: 3.0,
        point_radius_extra_px: 3.0,
        ..DataSelectionsConfig::default()
    });
    let series = [ternary("points", Some(marker_style(10.0)), None)];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        save(&format!("ternary-ring-{scale}x"), &img);
        let (cx, cy) = ternary_px(&config, rows[1], scale);
        let mut found = 0;
        let mut far = 0;
        for (i, p) in img.rgba.chunks_exact(4).enumerate() {
            if !is_magenta(p) {
                continue;
            }
            let (x, y) = (
                (i as u32 % img.width) as f64 + 0.5,
                (i as u32 / img.width) as f64 + 0.5,
            );
            if (x - cx).hypot(y - cy) < 24.0 * f64::from(scale) {
                found += 1;
            } else {
                far += 1;
            }
        }
        assert!(found > 20, "{scale}x: {found} ring pixels at the marker");
        assert_eq!(far, 0, "{scale}x: ring pixels away from the marker");
    }
}

/// Column-backed series have no count limit (rule 10): a ternary series of
/// 300,000 rows draws and its last row picks.
#[test]
fn three_hundred_thousand_rows_draw_and_pick() {
    let mut r = renderer();
    let n = 300_000usize;
    let rows: Vec<[f64; 3]> = (0..n)
        .map(|i| {
            let t = i as f64 / n as f64;
            [10.0 + 60.0 * t, 20.0, 70.0 - 60.0 * t]
        })
        .collect();
    upload(&mut r, &rows);
    let config = bare_config();
    let series = [ternary("many", Some(marker_style(3.0)), None)];
    let started = std::time::Instant::now();
    let img = export(&mut r, &config, &series, 1.0);
    eprintln!("300k ternary markers exported in {:?}", started.elapsed());
    assert!(is_red(at(&img, ternary_px(&config, rows[n / 2], 1.0))));
    // A row alone far from the others picks by its index.
    let mut lonely = rows.clone();
    lonely[n - 1] = [5.0, 90.0, 5.0];
    upload(&mut r, &lonely);
    let started = std::time::Instant::now();
    let hit = picked_point(pick(
        &mut r,
        &config,
        &series,
        1.0,
        ternary_px(&config, lonely[n - 1], 1.0),
        0.0,
    ));
    eprintln!("300k ternary pick in {:?}", started.elapsed());
    assert_eq!(hit, Some(("many".into(), n - 1)));
}

/// The fit leaves a ternary chart's axes alone: they span the triangle.
#[test]
fn the_fit_leaves_ternary_axes_alone() {
    let mut r = renderer();
    upload(&mut r, &[[3.0, 4.0, 5.0], [30.0, 4.0, 5.0]]);
    let series = [ternary("p", Some(marker_style(4.0)), None)];
    let mut chart = Chart::new(bare_config());
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.05)
        .unwrap();
    chart.auto_fit_x_extent(
        &FitExtent {
            min: 3.0,
            max: 30.0,
            min_positive: Some(3.0),
        },
        0.0,
    );
    for axis in [
        &chart.config().bottom_x,
        &chart.config().left_y,
        &chart.config().right_y,
    ] {
        assert_eq!((axis.min, axis.max), (0.0, 100.0));
    }
}

/// Registration rejects what a ternary chart does not draw, with the
/// model's reason.
#[test]
fn registration_rejects_what_ternary_does_not_draw() {
    let mut r = renderer();
    upload(&mut r, &[[1.0, 1.0, 1.0]]);
    let ok = ternary("p", Some(marker_style(4.0)), None);
    assert!(r.register_chart(bare_config(), vec![ok.clone()]).is_ok());
    let err = |config: Config, series: Vec<SeriesConfig>| {
        let mut r = renderer();
        upload(&mut r, &[[1.0, 1.0, 1.0]]);
        match r.register_chart(config, series) {
            Ok(_) => String::from("registered"),
            Err(e) => e.to_string(),
        }
    };
    let plain = SeriesConfig {
        render_type: DataRenderType::Scatter {
            scatter: marker_style(4.0),
        },
        ..ok.clone()
    };
    assert!(err(bare_config(), vec![plain]).contains("only ternary series"));
    let mut rectangular = bare_config();
    rectangular.ternary = None;
    assert!(err(rectangular, vec![ok.clone()]).contains("ternary chart"));
    // The axis rules are the chart's, checked where it is drawn, as a
    // polar chart's are.
    let mut unequal = bare_config();
    unequal.right_y.max = 1.0;
    let drawn = r.export_panel_rgba(&Chart::new(unequal), std::slice::from_ref(&ok), 1.0);
    assert!(
        drawn
            .as_ref()
            .is_err_and(|e| e.to_string().contains("same")),
        "{:?}",
        drawn.map(|_| ())
    );
    let missing = SeriesConfig {
        render_type: DataRenderType::Ternary {
            c_column: "missing".into(),
            scatter: Some(marker_style(4.0)),
            line: None,
        },
        ..ok
    };
    assert!(err(bare_config(), vec![missing]).contains("missing"));
}

/// A ternary subplot draws its triangle in its own cell, beside a
/// rectangular parent.
#[test]
fn a_ternary_subplot_draws_in_its_own_cell() {
    let mut r = renderer();
    let rows = [[70.0, 10.0, 20.0], [10.0, 70.0, 20.0], [20.0, 20.0, 60.0]];
    upload(&mut r, &rows);
    let mut parent = renderer::default::default_config();
    parent.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT / 2,
    });
    let mut inner = bare_config();
    inner.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT / 2,
        width: WIDTH,
        height: HEIGHT / 2,
    });
    let cell = inner.clone();
    parent.subplots.push(Subplot {
        config: inner,
        series: vec!["p".into()],
        ..Subplot::default()
    });
    let series = [ternary("p", Some(marker_style(8.0)), None)];
    let img = r
        .export_document_rgba(&Chart::new(parent), &series, 1.0)
        .unwrap();
    save("ternary-subplot", &img);
    assert_eq!((img.width, img.height), (WIDTH, HEIGHT));
    for row in rows {
        let p = ternary_px(&cell, row, 1.0);
        assert!(p.1 > f64::from(HEIGHT / 2), "in the lower cell");
        assert!(is_red(at(&img, p)), "{row:?} at {p:?}");
    }
}

/// An edge drags along its outward normal (`line_offset`) and ignores the
/// component along it; its labels drag in the edge frame; a title drags in
/// screen space; the top axis is not drawn and does not move.
#[test]
fn decorations_drag_in_the_edge_frame() {
    let config = dressed_config();
    let g = config.ternary_geometry().unwrap();
    let measure = CpuTextMeasure::for_style(&config.draw_style);
    for (k, side) in [(0, Side::Bottom), (1, Side::Left), (2, Side::Right)] {
        let n = g.outward_normal(k);
        let along = g.edge_direction(k);
        // Six pixels out and four along the edge.
        let (dx, dy) = (n[0] * 6.0 + along[0] * 4.0, n[1] * 6.0 + along[1] * 4.0);
        let mut moved = config.clone();
        assert_eq!(
            moved.nudge(Element::Axis(side.clone()), &measure, dx, dy),
            NudgeResult::Moved
        );
        let axis = renderer::ternary::ternary_axis(&moved, k);
        assert!(
            (axis.line_offset - 6.0).abs() < 1e-3,
            "{side:?}: {}",
            axis.line_offset
        );
        let mut labelled = config.clone();
        assert_eq!(
            labelled.nudge(Element::AxisLabel(side.clone()), &measure, dx, dy),
            NudgeResult::Moved
        );
        let ls = &renderer::ternary::ternary_axis(&labelled, k).label_style;
        assert!(
            (ls.label_offset_x - 4.0).abs() < 1e-3 && (ls.label_offset_y - 6.0).abs() < 1e-3,
            "{side:?}: ({}, {})",
            ls.label_offset_x,
            ls.label_offset_y
        );
        let mut titled = config.clone();
        assert_eq!(
            titled.nudge(Element::AxisTitle(side.clone()), &measure, 5.0, -3.0),
            NudgeResult::Moved
        );
        let to = &renderer::ternary::ternary_axis(&titled, k).title_option;
        assert_eq!((to.offset_x, to.offset_y), (5.0, -3.0), "{side:?}");
    }
    let mut top = config.clone();
    assert!(matches!(
        top.nudge(Element::Axis(Side::Top), &measure, 0.0, 5.0),
        NudgeResult::Rejected(_)
    ));
    // An edge dragged off the chart is refused and left where it was.
    let mut far = config.clone();
    assert!(matches!(
        far.nudge(Element::Axis(Side::Bottom), &measure, 0.0, 5000.0),
        NudgeResult::Rejected(_)
    ));
    assert_eq!(far.bottom_x.line_offset, 0.0);
}

/// The hit map answers each edge, its labels and its title where they are
/// drawn, and nothing for the undrawn top axis.
#[test]
fn the_hit_map_answers_each_edge_part() {
    let config = dressed_config();
    let g = config.ternary_geometry().unwrap();
    let map = HitMap::standard_chart();
    let measure = CpuTextMeasure::for_style(&config.draw_style);
    let hit = |x: f32, y: f32| {
        map.hit_test(&config, &measure, x, y)
            .and_then(|id| map.get(id))
            .map(|e| e.element_id())
    };
    for (k, side) in [(0, "bottom"), (1, "left"), (2, "right")] {
        let [a, b] = g.edge(k);
        // A third of the way along, on the edge.
        let p = [a[0] + (b[0] - a[0]) * 0.35, a[1] + (b[1] - a[1]) * 0.35];
        let id = hit(p[0], p[1]).unwrap_or_else(|| panic!("{side}: no hit on the edge"));
        assert!(id.contains("axis") && id.contains(side), "{side}: {id}");
        // The title, past the labels.
        let m = measure.measure_rich(
            &renderer::ternary::ternary_axis(&config, k)
                .title_option
                .text,
        );
        let title = config.ternary_title_placement(k, (0.0, 0.0), m).unwrap();
        let c = title.rotation_center;
        let id = hit(c.0, c.1).unwrap_or_else(|| panic!("{side}: no hit on the title"));
        assert!(id.contains("title") && id.contains(side), "{side}: {id}");
    }
}

/// The dressed chart exports at 2x as its 1x layout doubled: every edge,
/// label and title scales with the triangle.
#[test]
fn the_2x_export_doubles_the_1x_layout() {
    let mut r = renderer();
    upload(&mut r, &[[60.0, 20.0, 20.0], [20.0, 60.0, 20.0]]);
    let config = dressed_config();
    let series = [ternary("p", Some(marker_style(8.0)), None)];
    let one = export(&mut r, &config, &series, 1.0);
    let two = export(&mut r, &config, &series, 2.0);
    save("ternary-dressed-1x", &one);
    save("ternary-dressed-2x", &two);
    assert_eq!((two.width, two.height), (one.width * 2, one.height * 2));
    let ink = |img: &RasterImage| {
        img.rgba
            .chunks_exact(4)
            .filter(|p| p[3] > 128 && p[0] < 100 && p[1] < 100 && p[2] < 100)
            .count() as f64
    };
    let (a, b) = (ink(&one), ink(&two));
    assert!(a > 500.0, "the edges, ticks and labels draw: {a}");
    assert!((b / a - 4.0).abs() < 0.8, "2x ink {b} vs 1x {a}");
}

/// SSOT invalidation: replacing the c column redraws a registered chart's
/// dashed line and markers where the new components place them; the arc
/// cache names the c column, so the dashes follow the new path.
#[test]
fn a_replaced_c_column_redraws() {
    let mut r = renderer();
    let before = [[60.0, 20.0, 20.0], [20.0, 60.0, 20.0]];
    let after = [[60.0, 20.0, 120.0], [20.0, 60.0, 120.0]];
    let config = bare_config();
    let series = vec![ternary(
        "path",
        Some(marker_style(8.0)),
        Some(line_style(3.0, LineStylePreset::Dash)),
    )];
    upload(&mut r, &before);
    let id = r.register_chart(config.clone(), series.clone()).unwrap();
    let chart = Chart::new(config.clone());
    let first = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    upload(&mut r, &after);
    let second = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    for k in 0..2 {
        let old = ternary_px(&config, before[k], 1.0);
        let new = ternary_px(&config, after[k], 1.0);
        assert!(
            is_red(at(&first, old)) && !is_red(at(&first, new)),
            "row {k} before"
        );
        assert!(
            is_red(at(&second, new)) && !is_red(at(&second, old)),
            "row {k} after"
        );
    }
    // The dashes run along the new chord only.
    let ink_on = |img: &RasterImage, a: (f64, f64), b: (f64, f64)| {
        (1..40)
            .filter(|i| {
                let t = f64::from(*i) / 40.0;
                is_red(at(img, (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)))
            })
            .count()
    };
    let (na, nb) = (
        ternary_px(&config, after[0], 1.0),
        ternary_px(&config, after[1], 1.0),
    );
    let (oa, ob) = (
        ternary_px(&config, before[0], 1.0),
        ternary_px(&config, before[1], 1.0),
    );
    assert!(ink_on(&second, na, nb) > 10, "dashes on the new chord");
    assert!(ink_on(&second, oa, ob) < 4, "nothing left on the old chord");
    r.remove_chart(id).unwrap();
}
