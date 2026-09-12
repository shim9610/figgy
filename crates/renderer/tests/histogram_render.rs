//! End-to-end pixel verification of the histogram (bar) render path, through
//! the public renderer API and the headless `export_panel_rgba`.
//!
//! House pattern (mirrors `sketch_render.rs`): each test builds its own
//! instance/adapter/device and early-returns when no usable adapter exists.
//!
//! Attribution strategy: the bar fill is pure red and the border pure blue, so
//! the black deco ink (axes, ticks, titles) matches neither predicate and every
//! assertion is about the bar pipeline alone. Bin positions come from the same
//! `data_area()` + axis range the renderer uses, so the assertions name where a
//! bar *should* be rather than where it happens to have landed.

use std::sync::{Arc, OnceLock};

use renderer::config::AxisScale;
use renderer::data::Column;
use renderer::data_config::{
    BarOrientation, DataBarBinStyleConfig, DataBarStyleConfig, DataBarStyleOverride,
};
use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, Color, DataRenderType, DataSelectionsConfig, PickedDataRef, RasterImage, Renderer,
    RendererDevice, SeriesConfig, encode_png,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;

const FILL: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const BORDER: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
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

/// Grid and legend off: the only non-bar ink is the black deco frame.
fn bare_chart() -> Chart {
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
    Chart::new(config)
}

fn histogram(
    x: &str,
    y: &str,
    orientation: BarOrientation,
    gap_px: f32,
    border_width: f32,
) -> SeriesConfig {
    SeriesConfig {
        series_id: "hist".into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: FILL,
                border_color: BORDER,
                border_width,
                baseline: 0.0,
                gap_px,
                width_ratio: 1.0,
                orientation,
                bar_style_overrides: None,
            },
        },
    }
}

fn is_fill(p: &[u8]) -> bool {
    p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
}
fn is_border(p: &[u8]) -> bool {
    p[3] > 16 && p[2] > 120 && p[0] < 90 && p[1] < 90
}

fn is_override_fill(p: &[u8]) -> bool {
    p[3] > 16 && p[1] > 140 && p[0] < 90 && p[2] < 90
}

fn is_override_border(p: &[u8]) -> bool {
    p[3] > 16 && p[0] > 140 && p[1] > 140 && p[2] < 90
}

fn is_selection(p: &[u8]) -> bool {
    p[3] > 180 && p[0] > 180 && p[2] > 180 && p[1] < 80
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

/// Fill pixels inside a pixel-space column range.
fn fill_in_columns(img: &RasterImage, x0: u32, x1: u32) -> usize {
    let mut n = 0;
    for y in 0..img.height {
        for x in x0.min(img.width)..x1.min(img.width) {
            if is_fill(pixel(img, x, y)) {
                n += 1;
            }
        }
    }
    n
}

/// Topmost fill row in a pixel-space column range, if any. Lower value = taller
/// bar, because screen y grows downward.
fn top_fill_row(img: &RasterImage, x0: u32, x1: u32) -> Option<u32> {
    (0..img.height)
        .find(|y| (x0.min(img.width)..x1.min(img.width)).any(|x| is_fill(pixel(img, x, *y))))
}

/// Data x → panel pixel x, through the same data area and axis range the
/// renderer draws with.
fn data_x_to_px(chart: &Chart, value: f64) -> f32 {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let t = (value - cfg.bottom_x.min) / (cfg.bottom_x.max - cfg.bottom_x.min);
    da.x as f32 + t as f32 * da.width as f32
}

/// Data y → panel pixel y (screen y grows downward).
fn data_y_to_px(chart: &Chart, value: f64) -> f32 {
    let cfg = chart.config();
    let da = cfg.data_area().expect("data area");
    let t = (value - cfg.left_y.min) / (cfg.left_y.max - cfg.left_y.min);
    (da.y + da.height) as f32 - t as f32 * da.height as f32
}

/// Four bins over x ∈ [0, 4], counts 1 / 4 / 2 / 3 — no two adjacent bins share
/// a height, so a bar drawn at the wrong bin cannot pass the height ordering.
const EDGES: [f64; 5] = [0.0, 1.0, 2.0, 3.0, 4.0];
const COUNTS: [f64; 4] = [1.0, 4.0, 2.0, 3.0];

fn vertical_fixture(renderer: &mut Renderer) -> (Chart, Vec<SeriesConfig>) {
    renderer
        .add_column("edges", &col_f64(EDGES.to_vec()))
        .unwrap();
    renderer
        .add_column("counts", &col_f64(COUNTS.to_vec()))
        .unwrap();
    let mut chart = bare_chart();
    chart.set_x_range(0.0, 4.0);
    chart.set_y_range(0.0, 5.0);
    let series = vec![histogram(
        "edges",
        "counts",
        BarOrientation::Vertical,
        2.0,
        2.0,
    )];
    (chart, series)
}

/// Every declared bin draws, **including the last one** — the failure the
/// design calls out: the line draw call issues `min(x, y) - 1` instances, which
/// for a well-formed `(n + 1, n)` histogram silently drops the final bar. Bar
/// heights also follow their own counts, so the bins are not merely present but
/// in the right places.
#[test]
fn every_bin_draws_and_its_height_follows_its_count() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let (chart, series) = vertical_fixture(&mut renderer);
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    let mut tops = Vec::new();
    for (bin, count) in COUNTS.iter().enumerate() {
        let x0 = data_x_to_px(&chart, EDGES[bin]).ceil() as u32 + 3;
        let x1 = data_x_to_px(&chart, EDGES[bin + 1]).floor() as u32 - 3;
        let ink = fill_in_columns(&img, x0, x1);
        assert!(
            ink > 20,
            "bin {bin} (count {count}, px {x0}..{x1}) has only {ink} fill pixels"
        );
        let top = top_fill_row(&img, x0, x1).expect("a drawn bin has a top row");
        // The bar's top edge is at its count, within a couple of pixels of
        // border and rasterization slack.
        let expected = data_y_to_px(&chart, *count);
        assert!(
            (top as f32 - expected).abs() <= 4.0,
            "bin {bin}: top row {top}, expected ~{expected} for count {count}"
        );
        tops.push(top);
    }

    // Ordering, stated independently of the absolute geometry: taller count →
    // smaller top row.
    assert!(tops[1] < tops[3], "count 4 must reach above count 3");
    assert!(tops[3] < tops[2], "count 3 must reach above count 2");
    assert!(tops[2] < tops[0], "count 2 must reach above count 1");
}

#[test]
fn selected_bin_outline_uses_the_exact_bar_instance() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let (mut chart, series) = vertical_fixture(&mut renderer);
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: "hist".into(),
            bin_index: 1,
        }],
        highlight_color: Color::new(1.0, 0.0, 1.0, 1.0),
        outline_width_px: 4.0,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 0.0,
    });

    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let selected_x0 = data_x_to_px(&chart, EDGES[1]).floor().max(0.0) as u32;
    let selected_x1 = data_x_to_px(&chart, EDGES[2]).ceil().max(0.0) as u32;
    let mut inside = 0usize;
    let mut outside = 0usize;
    for y in 0..img.height {
        for x in 0..img.width {
            if is_selection(pixel(&img, x, y)) {
                if x >= selected_x0 && x <= selected_x1 {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
    }
    assert!(
        inside > 100,
        "selected bin produced only {inside} highlight pixels"
    );
    assert_eq!(outside, 0, "selection leaked outside the selected bin");
}

/// The border is drawn, in its own colour, around the fill.
#[test]
fn bars_draw_their_border_in_the_border_colour() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let (chart, series) = vertical_fixture(&mut renderer);
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    let border_ink = img.rgba.chunks_exact(4).filter(|p| is_border(p)).count();
    assert!(border_ink > 100, "only {border_ink} border pixels");

    // A zero-width border leaves the fill and removes the border ink, so the
    // colour above is the border and not some incidental blue.
    let mut no_border = series.clone();
    no_border[0].render_type = DataRenderType::Histogram {
        bar: DataBarStyleConfig {
            fill_color: FILL,
            border_color: BORDER,
            border_width: 0.0,
            baseline: 0.0,
            gap_px: 2.0,
            width_ratio: 1.0,
            orientation: BarOrientation::Vertical,
            bar_style_overrides: None,
        },
    };
    let plain = renderer.export_panel_rgba(&chart, &no_border, 1.0).unwrap();
    assert_eq!(
        plain.rgba.chunks_exact(4).filter(|p| is_border(p)).count(),
        0,
        "a zero-width border must draw no border pixels"
    );
    assert!(
        plain.rgba.chunks_exact(4).filter(|p| is_fill(p)).count() > 100,
        "the fill must survive a zero-width border"
    );
}

/// `gap_px` separates neighbouring bars: with a wide gap the bin boundary
/// column carries no bar ink at all, and the bars stay put (the gap is taken
/// from the bar, not added to the layout).
#[test]
fn the_gap_separates_neighbouring_bars() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_column("edges", &col_f64(EDGES.to_vec()))
        .unwrap();
    renderer
        .add_column("counts", &col_f64(COUNTS.to_vec()))
        .unwrap();
    let mut chart = bare_chart();
    chart.set_x_range(0.0, 4.0);
    chart.set_y_range(0.0, 5.0);

    let wide_gap = vec![histogram(
        "edges",
        "counts",
        BarOrientation::Vertical,
        12.0,
        0.0,
    )];
    let img = renderer.export_panel_rgba(&chart, &wide_gap, 1.0).unwrap();

    // The interior bin boundaries (x = 1, 2, 3) must be clear of bar ink.
    for edge in [1.0f64, 2.0, 3.0] {
        let x = data_x_to_px(&chart, edge).round() as u32;
        let ink = fill_in_columns(&img, x, x + 1) + {
            let mut n = 0;
            for y in 0..img.height {
                if is_border(pixel(&img, x, y)) {
                    n += 1;
                }
            }
            n
        };
        assert_eq!(
            ink, 0,
            "bin boundary at x={edge} (px {x}) still has bar ink"
        );
    }

    // A gap wider than the bin cannot invert or erase it: the shader keeps a
    // one-pixel footprint once the raw bin is wide enough to provide one.
    let closed = vec![histogram(
        "edges",
        "counts",
        BarOrientation::Vertical,
        10_000.0,
        0.0,
    )];
    let squeezed = renderer.export_panel_rgba(&chart, &closed, 1.0).unwrap();
    assert!(
        squeezed.rgba.chunks_exact(4).filter(|p| is_fill(p)).count() > 100,
        "a gap wider than the bin must retain a minimum visible footprint"
    );
}

/// A screen-space gap must not erase dense histograms. Once bins project to
/// less than one pixel along their edge axis, there is no pixel budget for a
/// visible gap; the bars must retain their raw subpixel coverage instead of
/// collapsing to zero-width geometry.
#[test]
fn subpixel_bins_survive_a_one_pixel_gap_in_both_orientations() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    const BINS: usize = 2048;
    let edges: Vec<f64> = (0..=BINS).map(|i| i as f64).collect();
    let counts = vec![1.0; BINS];
    renderer.add_column("dense_edges", &col_f64(edges)).unwrap();
    renderer
        .add_column("dense_counts", &col_f64(counts))
        .unwrap();

    for orientation in [BarOrientation::Vertical, BarOrientation::Horizontal] {
        let mut chart = bare_chart();
        let (x, y) = match orientation {
            BarOrientation::Vertical => {
                chart.set_x_range(0.0, BINS as f64);
                chart.set_y_range(0.0, 2.0);
                ("dense_edges", "dense_counts")
            }
            BarOrientation::Horizontal => {
                chart.set_x_range(0.0, 2.0);
                chart.set_y_range(0.0, BINS as f64);
                ("dense_counts", "dense_edges")
            }
        };
        let series = vec![histogram(x, y, orientation.clone(), 1.0, 0.0)];
        let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let fill_pixels = img.rgba.chunks_exact(4).filter(|p| is_fill(p)).count();
        assert!(
            fill_pixels > 1_000,
            "{orientation:?}: subpixel bins were erased by the one-pixel gap ({fill_pixels} fill pixels)"
        );
        for scale in [1.0, 2.0] {
            let stroked = vec![histogram(x, y, orientation.clone(), 1.0, 1.0)];
            let img = renderer.export_panel_rgba(&chart, &stroked, scale).unwrap();
            assert_eq!(
                img.rgba.chunks_exact(4).filter(|p| is_fill(p)).count(),
                0,
                "enabled subpixel stroke must replace fill throughout the envelope"
            );
            assert!(
                img.rgba
                    .chunks_exact(4)
                    .filter(|p| p[2] > 200 && p[0] < 30 && p[1] < 30)
                    .count()
                    > 1_000,
                "subpixel envelope must retain the blue stroke down to zero"
            );
        }
    }
}

#[test]
fn subpixel_envelope_keeps_maxima_without_repeated_alpha_blending() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    const N: usize = 8192;
    renderer
        .add_column(
            "envelope_edges",
            &col_f64((0..=N).map(|i| i as f64).collect()),
        )
        .unwrap();
    renderer
        .add_column(
            "envelope_counts",
            &col_f64((0..N).map(|i| [1.0, 8.0, 2.0, 3.0][i % 4]).collect()),
        )
        .unwrap();
    renderer
        .add_column("reference_edges", &col_f64(vec![0.0, N as f64]))
        .unwrap();
    renderer
        .add_column("reference_counts", &col_f64(vec![8.0]))
        .unwrap();
    for horizontal in [false, true] {
        for scale in [1.0, 2.0] {
            let mut chart = bare_chart();
            if horizontal {
                chart.set_x_range(0.0, 10.0);
                chart.set_y_range(0.0, N as f64);
            } else {
                chart.set_x_range(0.0, N as f64);
                chart.set_y_range(0.0, 10.0);
            }
            let make = |edges, counts, gap| {
                let orientation = if horizontal {
                    BarOrientation::Horizontal
                } else {
                    BarOrientation::Vertical
                };
                let mut series = if horizontal {
                    histogram(counts, edges, orientation, gap, 0.0)
                } else {
                    histogram(edges, counts, orientation, gap, 0.0)
                };
                let DataRenderType::Histogram { bar } = &mut series.render_type else {
                    unreachable!()
                };
                bar.fill_color.a = 0.35;
                series
            };
            let dense = renderer
                .export_panel_rgba(
                    &chart,
                    &[make("envelope_edges", "envelope_counts", 1.0)],
                    scale,
                )
                .unwrap();
            let reference = renderer
                .export_panel_rgba(
                    &chart,
                    &[make("reference_edges", "reference_counts", 0.0)],
                    scale,
                )
                .unwrap();
            let da = chart.config().data_area().unwrap();
            // Stay away from the data-area scissor and outer border. Every
            // column/row here intersects at least one narrow height-eight bin.
            for y in
                ((da.y + 3) as f32 * scale) as u32..((da.y + da.height - 3) as f32 * scale) as u32
            {
                for x in ((da.x + 3) as f32 * scale) as u32
                    ..((da.x + da.width - 3) as f32 * scale) as u32
                {
                    let got = pixel(&dense, x, y);
                    let expected = pixel(&reference, x, y);
                    assert!(
                        got.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 1),
                        "horizontal={horizontal} scale={scale} ({x},{y}): {got:?} != {expected:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn envelope_uses_winner_style_zero_baseline_and_keeps_tiny_edges() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_hilo_column(
            "tiny_edges",
            &col_f64((0..5).map(|i| 100.0 + i as f64 * 1e-10).collect()),
        )
        .unwrap();
    renderer
        .add_column("tiny_values", &col_f64(vec![1.0, 8.0, 3.0, 8.0]))
        .unwrap();
    let mut chart = bare_chart();
    chart.set_x_range(0.0, 1000.0);
    chart.set_y_range(0.0, 10.0);
    let mut series = histogram(
        "tiny_edges",
        "tiny_values",
        BarOrientation::Vertical,
        1.0,
        1.0,
    );
    let DataRenderType::Histogram { bar } = &mut series.render_type else {
        unreachable!()
    };
    bar.baseline = 5.0; // The subpixel fallback explicitly fills to zero.
    bar.bar_style_overrides = Some(vec![DataBarStyleOverride {
        index: 1,
        style: DataBarBinStyleConfig {
            fill_color: Some(Color {
                r: 0.0,
                g: 1.0,
                b: 0.0,
                a: 1.0,
            }),
            border_width: Some(0.0),
            ..Default::default()
        },
    }]);
    let image = renderer.export_panel_rgba(&chart, &[series], 1.0).unwrap();
    let x = data_x_to_px(&chart, 100.0).floor() as u32;
    for value in [2.0, 7.0] {
        assert!(
            is_override_fill(pixel(&image, x, data_y_to_px(&chart, value) as u32)),
            "tiny peak must select the first maximum's style and fill to zero"
        );
    }
}

#[test]
fn width_ratio_centres_a_narrower_bar_inside_each_bin() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let (chart, mut series) = vertical_fixture(&mut renderer);
    let DataRenderType::Histogram { bar } = &mut series[0].render_type else {
        unreachable!()
    };
    bar.gap_px = 0.0;
    bar.border_width = 0.0;
    bar.width_ratio = 0.5;

    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let raw_x0 = data_x_to_px(&chart, EDGES[0]);
    let raw_x1 = data_x_to_px(&chart, EDGES[1]);
    let y = data_y_to_px(&chart, 0.5).round() as u32;
    let xs: Vec<u32> = (raw_x0.floor().max(0.0) as u32..raw_x1.ceil() as u32)
        .filter(|x| is_fill(pixel(&img, *x, y)))
        .collect();
    let first = *xs.first().expect("half-width bar has fill");
    let last = *xs.last().expect("half-width bar has fill");
    let raw_span = raw_x1 - raw_x0;
    let drawn_span = (last - first + 1) as f32;
    assert!(
        (drawn_span - raw_span * 0.5).abs() <= 3.0,
        "drawn {drawn_span}px, raw bin {raw_span}px"
    );
    let drawn_center = (first + last) as f32 * 0.5;
    assert!(
        (drawn_center - (raw_x0 + raw_x1) * 0.5).abs() <= 2.0,
        "narrow bar is not centred in its bin"
    );
}

#[test]
fn one_bin_override_controls_fill_outline_width_and_selection_geometry() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let (mut chart, mut series) = vertical_fixture(&mut renderer);
    let DataRenderType::Histogram { bar } = &mut series[0].render_type else {
        unreachable!()
    };
    bar.width_ratio = 0.9;
    bar.bar_style_overrides = Some(vec![DataBarStyleOverride {
        index: 1,
        style: DataBarBinStyleConfig {
            fill_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
            border_color: Some(Color::new(1.0, 1.0, 0.0, 1.0)),
            border_width: Some(4.0),
            gap_px: Some(0.0),
            width_ratio: Some(0.45),
        },
    }]);

    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let bin_x0 = data_x_to_px(&chart, EDGES[1]).floor().max(0.0) as u32;
    let bin_x1 = data_x_to_px(&chart, EDGES[2]).ceil().max(0.0) as u32;
    let mut override_fill = 0usize;
    let mut override_border = 0usize;
    let mut leaked = 0usize;
    for y in 0..img.height {
        for x in 0..img.width {
            let is_override =
                is_override_fill(pixel(&img, x, y)) || is_override_border(pixel(&img, x, y));
            if is_override && (x < bin_x0 || x > bin_x1) {
                leaked += 1;
            }
            override_fill += usize::from(is_override_fill(pixel(&img, x, y)));
            override_border += usize::from(is_override_border(pixel(&img, x, y)));
        }
    }
    assert!(override_fill > 100, "override fill did not render");
    assert!(override_border > 100, "override outline did not render");
    assert_eq!(leaked, 0, "bin override colours leaked to another bin");

    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: "hist".into(),
            bin_index: 1,
        }],
        highlight_color: Color::new(1.0, 0.0, 1.0, 1.0),
        outline_width_px: 3.0,
        point_radius_extra_px: 0.0,
        contour_width_extra_px: 0.0,
    });
    let selected = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let selected_x: Vec<u32> = (0..selected.width)
        .filter(|x| (0..selected.height).any(|y| is_selection(pixel(&selected, *x, y))))
        .collect();
    let selected_min = *selected_x.first().expect("selected override outline");
    let selected_max = *selected_x.last().expect("selected override outline");
    let raw_span = (bin_x1 - bin_x0) as f32;
    assert!(
        (selected_min as f32) > bin_x0 as f32 + raw_span * 0.2
            && (selected_max as f32) < bin_x1 as f32 - raw_span * 0.2,
        "selection used the raw bin instead of overridden width: {selected_min}..{selected_max} vs {bin_x0}..{bin_x1}"
    );
}

/// `orientation` alone swaps the roles: the same two columns draw bars running
/// along x instead of y.
#[test]
fn horizontal_bars_run_along_x() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_column("edges", &col_f64(EDGES.to_vec()))
        .unwrap();
    renderer
        .add_column("counts", &col_f64(COUNTS.to_vec()))
        .unwrap();
    let mut chart = bare_chart();
    // x carries the counts now, y the bin edges.
    chart.set_x_range(0.0, 5.0);
    chart.set_y_range(0.0, 4.0);
    let series = vec![histogram(
        "counts",
        "edges",
        BarOrientation::Horizontal,
        2.0,
        0.0,
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    // Each bin is a horizontal band; its bar reaches right to its own count.
    for (bin, count) in COUNTS.iter().enumerate() {
        let y0 = data_y_to_px(&chart, EDGES[bin + 1]).ceil() as u32 + 3;
        let y1 = data_y_to_px(&chart, EDGES[bin]).floor() as u32 - 3;
        let right = (0..img.width)
            .rev()
            .find(|x| (y0..y1).any(|y| is_fill(pixel(&img, *x, y))))
            .unwrap_or_else(|| panic!("bin {bin} drew no horizontal bar"));
        let expected = data_x_to_px(&chart, *count);
        assert!(
            (right as f32 - expected).abs() <= 4.0,
            "bin {bin}: bar ends at px {right}, expected ~{expected} for count {count}"
        );
    }
}

/// A declaration that outruns its data draws the bins it can and reports the
/// truncation — it does not error, and it does not read past the data.
#[test]
fn a_short_count_column_draws_fewer_bars_and_says_so() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_column("edges", &col_f64(EDGES.to_vec()))
        .unwrap();
    renderer
        .add_column("counts2", &col_f64(vec![1.0, 4.0]))
        .unwrap();
    let mut chart = bare_chart();
    chart.set_x_range(0.0, 4.0);
    chart.set_y_range(0.0, 5.0);
    let series = vec![histogram(
        "edges",
        "counts2",
        BarOrientation::Vertical,
        2.0,
        0.0,
    )];

    let chart_id = renderer
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let info = renderer.series_draw_info(chart_id, "hist").unwrap();
    assert_eq!(info.drawn_count, 2);
    assert!(info.truncated, "4 bins declared, 2 counts supplied");

    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
    for bin in 0..2 {
        let x0 = data_x_to_px(&chart, EDGES[bin]).ceil() as u32 + 3;
        let x1 = data_x_to_px(&chart, EDGES[bin + 1]).floor() as u32 - 3;
        assert!(fill_in_columns(&img, x0, x1) > 20, "bin {bin} must draw");
    }
    for bin in 2..4 {
        let x0 = data_x_to_px(&chart, EDGES[bin]).ceil() as u32 + 3;
        let x1 = data_x_to_px(&chart, EDGES[bin + 1]).floor() as u32 - 3;
        assert_eq!(
            fill_in_columns(&img, x0, x1),
            0,
            "bin {bin} has no count and must not draw"
        );
    }
}

/// A logarithmic count axis with the usual baseline of 0 needs no special case:
/// `maybe_log` floors the base far below the axis, so each bar runs from its
/// count down past the bottom of the data area and is clipped by the scissor.
/// Nothing produces NaN, and nothing disappears.
#[test]
fn a_logarithmic_count_axis_still_draws_every_bar() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    renderer
        .add_column("edges", &col_f64(EDGES.to_vec()))
        .unwrap();
    renderer
        .add_column("counts", &col_f64(vec![1.0, 100.0, 10.0, 1000.0]))
        .unwrap();
    let mut chart = bare_chart();
    chart.set_x_range(0.0, 4.0);
    chart.config_mut().left_y.scale = AxisScale::Logarithmic;
    // Below the smallest count: a bar whose top *is* the axis minimum has zero
    // height by construction, which would say nothing about the log path.
    chart.config_mut().left_y.min = 0.1;
    chart.config_mut().left_y.max = 10_000.0;
    let series = vec![histogram(
        "edges",
        "counts",
        BarOrientation::Vertical,
        2.0,
        0.0,
    )];
    let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();

    let da = chart.config().data_area().expect("data area");
    let mut tops = Vec::new();
    for bin in 0..4 {
        let x0 = data_x_to_px(&chart, EDGES[bin]).ceil() as u32 + 3;
        let x1 = data_x_to_px(&chart, EDGES[bin + 1]).floor() as u32 - 3;
        assert!(
            fill_in_columns(&img, x0, x1) > 20,
            "bin {bin} vanished on a log axis"
        );
        let top = top_fill_row(&img, x0, x1).expect("top row");
        tops.push(top);
        // The bar reaches the bottom of the data area: the base is below it.
        assert!(
            is_fill(pixel(&img, (x0 + x1) / 2, da.y + da.height - 2)),
            "bin {bin} does not reach the bottom of the data area"
        );
    }
    // Decade ordering survives: 1000 > 100 > 10 > 1.
    assert!(tops[3] < tops[1], "1000 must reach above 100");
    assert!(tops[1] < tops[2], "100 must reach above 10");
    assert!(tops[2] < tops[0], "10 must reach above 1");
}

/// Visual probe — the same shapes the assertions above check, written out as
/// PNGs so the bars can be looked at. Output goes to `target/arch-after/` (or
/// `$FIGGY_LAYOUT_PROBE_DIR`), beside the layout and colourbar probes.
#[test]
fn histogram_probe_renders_every_orientation_and_scale() {
    let Some(mut renderer) = try_renderer() else {
        return;
    };
    let dir = {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("renderer crate is inside the workspace crates directory");
        let configured = std::env::var_os("FIGGY_LAYOUT_PROBE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("target/arch-after"));
        if configured.is_absolute() {
            configured
        } else {
            workspace.join(configured)
        }
    };
    std::fs::create_dir_all(&dir).expect("create probe directory");

    renderer
        .add_column("edges", &col_f64(EDGES.to_vec()))
        .unwrap();
    renderer
        .add_column("counts", &col_f64(COUNTS.to_vec()))
        .unwrap();
    renderer
        .add_column("counts_log", &col_f64(vec![1.0, 100.0, 10.0, 1000.0]))
        .unwrap();
    renderer
        .add_column("counts2", &col_f64(vec![1.0, 4.0]))
        .unwrap();

    // Readable steel blue rather than the assertion fixture's saturated red.
    let styled = |x: &str, y: &str, orientation: BarOrientation| SeriesConfig {
        series_id: "hist".into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: Color::from_rgb8(70, 130, 180),
                border_color: Color::from_rgb8(20, 40, 60),
                border_width: 1.5,
                baseline: 0.0,
                gap_px: 2.0,
                width_ratio: 1.0,
                orientation,
                bar_style_overrides: None,
            },
        },
    };

    let mut cases: Vec<(&str, Chart, Vec<SeriesConfig>)> = Vec::new();

    let mut vertical = bare_chart();
    vertical.config_mut().grid.show_major_y = true;
    vertical.set_x_range(0.0, 4.0);
    vertical.set_y_range(0.0, 5.0);
    cases.push((
        "vertical",
        vertical,
        vec![styled("edges", "counts", BarOrientation::Vertical)],
    ));

    let mut horizontal = bare_chart();
    horizontal.config_mut().grid.show_major_x = true;
    horizontal.set_x_range(0.0, 5.0);
    horizontal.set_y_range(0.0, 4.0);
    cases.push((
        "horizontal",
        horizontal,
        vec![styled("counts", "edges", BarOrientation::Horizontal)],
    ));

    let mut log_y = bare_chart();
    log_y.config_mut().grid.show_major_y = true;
    log_y.set_x_range(0.0, 4.0);
    log_y.config_mut().left_y.scale = AxisScale::Logarithmic;
    log_y.config_mut().left_y.min = 0.1;
    log_y.config_mut().left_y.max = 10_000.0;
    log_y.config_mut().left_y.major_spacing = 1.0;
    cases.push((
        "log-y",
        log_y,
        vec![styled("edges", "counts_log", BarOrientation::Vertical)],
    ));

    let mut truncated = bare_chart();
    truncated.config_mut().grid.show_major_y = true;
    truncated.set_x_range(0.0, 4.0);
    truncated.set_y_range(0.0, 5.0);
    cases.push((
        "truncated",
        truncated,
        vec![styled("edges", "counts2", BarOrientation::Vertical)],
    ));

    for (name, chart, series) in cases {
        let img = renderer.export_panel_rgba(&chart, &series, 1.0).unwrap();
        assert!(
            img.rgba.chunks_exact(4).any(|p| p[3] != 0),
            "{name}: nothing was drawn"
        );
        let png = encode_png(&img).expect("probe png");
        std::fs::write(dir.join(format!("histogram-{name}.png")), png).expect("write probe png");
    }
}
