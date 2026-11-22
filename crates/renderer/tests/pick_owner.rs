//! The series-owner ink mask (#381): each texel of a panel's ink mask names
//! the topmost series drawn there, and the owner of the inked texel nearest
//! to the cursor decides the pick. A point-datum owner (a line, a scatter)
//! selects its nearest datum; a body owner (a histogram's bars) selects its
//! typed hit, on its own layer when another one is active. At 1x and 2x, for bars under a line on the same layer and on
//! an overlay layer, for a line and a scatter in either order, and for a
//! panel with more series than an 8 or 16 bit slot could name.
//!
//! Grid, legend, title and axis lines are off, so the only ink is the data.

#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;
use harness::{BLUE, RED, markers, upload};
use test_gpu::shared_device;

use renderer::config::AxisOptions;
use renderer::data_config::{
    BarOrientation, DataBarStyleConfig, DataLineStyleConfig, HistogramSeries, LineInterpolation,
    XySeries,
};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, ChartId, Config, DataPickResult, DataRenderType, GpuPickRequest, LayerRef, OverlayLayer,
    PickedData, RegisteredChartDrawItem, Renderer, RendererDevice, SeriesConfig,
};

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.enable_gpu_picking().unwrap();
    r
}

/// A bare `width` x `height` chart: x over 0..10, y over 0..100, nothing
/// drawn but data.
fn bare(width: u32, height: u32) -> Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width,
        height,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    config.chart_title.visible = false;
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.line_visible = false;
    }
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 100.0);
    chart.config().clone()
}

fn line(id: &str, x: &str, y: &str, width: f32) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Xy(XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: BLUE,
                line_width: width,
                fill: None,
                fill_color: None,
                interpolation: LineInterpolation::Linear,
            }),
            errors: None,
            stem: None,
        }),
    }
}

fn bars(id: &str, edges: &str, heights: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: edges.into(),
        y_column: heights.into(),
        z_column: None,
        render_type: DataRenderType::Histogram(HistogramSeries {
            bar: DataBarStyleConfig {
                fill_color: RED,
                border_color: RED,
                border_width: 0.0,
                baseline: 0.0,
                gap_px: 0.0,
                width_ratio: 1.0,
                orientation: BarOrientation::Vertical,
                bar_style_overrides: None,
            },
        }),
    }
}

/// Register `config` with `series`, shown at `scale`.
fn register(r: &mut Renderer, config: &Config, series: &[SeriesConfig], scale: f32) -> ChartId {
    let id = r.register_chart(config.clone(), series.to_vec()).unwrap();
    if scale != 1.0 {
        let document = r.chart_config(id).unwrap().document_area();
        r.set_chart_display(
            id,
            Some((
                ((document.x + document.width) as f32 * scale) as u32,
                ((document.y + document.height) as f32 * scale) as u32,
            )),
        )
        .unwrap();
    }
    id
}

/// One on-screen prepare of chart `id`, as a host's frame runs it.
fn draw(r: &mut Renderer, id: ChartId) {
    let (config, panel, scale) = r.chart_display_config(id).unwrap();
    let chart = Chart::new(config.into_owned());
    let mut view = r.create_chart_view_at(&chart, panel, scale).unwrap();
    r.prepare_registered(&mut [RegisteredChartDrawItem {
        chart_id: id,
        view: &mut view,
    }])
    .unwrap();
}

/// Where base-axis `(x, y)` lands on chart `id` as drawn.
fn placed(r: &Renderer, id: ChartId, x: f64, y: f64) -> (f32, f32) {
    let (config, _, _) = r.chart_display_config(id).unwrap();
    let area = config.data_area().unwrap().0;
    let (ax, ay, aw, ah) = (
        area.x as f32,
        area.y as f32,
        area.width as f32,
        area.height as f32,
    );
    let fx = renderer::layout::axis_fraction(x, &config.bottom_x, aw);
    let fy = renderer::layout::axis_fraction(y, &config.left_y, ah);
    (ax + aw * fx, ay + ah * (1.0 - fy))
}

fn request(r: &Renderer, id: ChartId, at: (f32, f32), reach: f32) -> GpuPickRequest {
    let (_, panel, scale) = r.chart_display_config(id).unwrap();
    let document = r.chart_config(id).unwrap().document_area();
    GpuPickRequest {
        canvas_position_px: [at.0, at.1],
        display_panel_px: Rect {
            x: panel.x,
            y: panel.y,
            width: ((document.x + document.width) as f32 * scale) as u32,
            height: ((document.y + document.height) as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: reach,
    }
}

/// The typed data pick at base-axis `(x, y)`: the selection and the owner
/// it came from.
fn pick(r: &mut Renderer, id: ChartId, (x, y): (f64, f64), reach: f32) -> DataPickResult {
    let at = placed(r, id, x, y);
    let request = request(r, id, at, reach);
    r.pick_chart_data(id, request)
        .unwrap()
        .resolve_full_blocking()
        .unwrap()
}

fn owner(result: &DataPickResult) -> Option<&str> {
    result
        .point
        .owner
        .as_ref()
        .map(|owner| owner.series_id.as_str())
}

fn point(result: &DataPickResult) -> Option<(&str, usize)> {
    match &result.picked {
        Some(PickedData::Point {
            series_id,
            point_index,
            ..
        }) => Some((series_id.as_str(), *point_index)),
        _ => None,
    }
}

fn bin(result: &DataPickResult) -> Option<(&str, usize)> {
    match &result.picked {
        Some(PickedData::HistogramBin {
            series_id,
            bin_index,
            ..
        }) => Some((series_id.as_str(), *bin_index)),
        _ => None,
    }
}

/// Bars over 0..10 in five bins and a line over them: through the tops
/// above the bars, and through (3, 50), inside the second bar.
fn bars_and_line(r: &mut Renderer) {
    upload(r, "edges", vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
    upload(r, "heights", vec![50.0, 80.0, 30.0, 60.0, 40.0]);
    upload(r, "lx", vec![1.0, 3.0, 5.0, 7.0, 9.0]);
    upload(r, "ly", vec![90.0, 50.0, 90.0, 95.0, 90.0]);
}

/// A bar chart with a line drawn over it, on the same layer and as an
/// overlay layer: a click inside a bar away from the line picks the bar,
/// near a line point the point, and where the line crosses a bar the line,
/// drawn on top.
#[test]
fn a_bar_under_a_line_picks_the_bar_and_the_line_picks_its_point() {
    for overlay in [false, true] {
        for scale in [1.0f32, 2.0] {
            let mut r = renderer();
            bars_and_line(&mut r);
            let mut config = bare(480, 360);
            if overlay {
                config.overlays.push(OverlayLayer {
                    y_axis: AxisOptions::overlay_y("line", BLUE),
                    link_x: true,
                    link_y: true,
                    series: vec!["line".into()],
                    ..OverlayLayer::default()
                });
            }
            let series = [
                bars("hist", "edges", "heights"),
                line("line", "lx", "ly", 2.0),
            ];
            let id = register(&mut r, &config, &series, scale);
            draw(&mut r, id);
            let what = format!("overlay {overlay}, {scale}x");

            // Inside the third bar, far below the line.
            let result = pick(&mut r, id, (5.0, 10.0), 2.0 * scale);
            assert_eq!(owner(&result), Some("hist"), "{what}");
            assert_eq!(bin(&result), Some(("hist", 2)), "{what}");
            assert_eq!(result.point.hit, None, "a bar is no point hit: {what}");

            // At the line's third point, above the bars.
            let result = pick(&mut r, id, (5.0, 90.0), 2.0 * scale);
            assert_eq!(owner(&result), Some("line"), "{what}");
            assert_eq!(point(&result), Some(("line", 2)), "{what}");

            // Where the line runs inside the second bar: on one layer the
            // line is drawn on top and owns the ink; across layers both are
            // at distance 0, and the tie goes to the active layer's bar.
            let result = pick(&mut r, id, (3.0, 50.0), 0.0);
            if overlay {
                assert_eq!(owner(&result), Some("hist"), "{what}");
                assert_eq!(bin(&result), Some(("hist", 1)), "{what}");
            } else {
                assert_eq!(owner(&result), Some("line"), "{what}");
                assert_eq!(point(&result), Some(("line", 1)), "{what}");
            }

            // Empty space is a miss, with no typed hit there either.
            let result = pick(&mut r, id, (5.0, 60.0), 1.0);
            assert_eq!(owner(&result), None, "{what}");
            assert_eq!(result.picked, None, "{what}");

            // With the overlay active the bars on the base layer still own
            // their ink, and the pick runs the base layer's typed kernel for
            // them: the bar is selected. The line's points still pick, and
            // where the line crosses a bar the tie now goes to the line's
            // active layer.
            if overlay {
                let mut active = r.chart_config(id).unwrap().clone();
                active.set_active_layer(LayerRef::Overlay(0));
                r.set_chart_config(id, active).unwrap();
                draw(&mut r, id);
                let result = pick(&mut r, id, (5.0, 10.0), 2.0 * scale);
                assert_eq!(owner(&result), Some("hist"), "{what}");
                assert_eq!(bin(&result), Some(("hist", 2)), "{what}");
                assert_eq!(result.data, None, "the active layer has no bar: {what}");
                let result = pick(&mut r, id, (5.0, 90.0), 2.0 * scale);
                assert_eq!(point(&result), Some(("line", 2)), "{what}");
                let result = pick(&mut r, id, (3.0, 50.0), 0.0);
                assert_eq!(owner(&result), Some("line"), "{what}");
                assert_eq!(point(&result), Some(("line", 1)), "{what}");
            }
        }
    }
}

/// A line and a scatter on the same points, in either drawing order: at a
/// point the series drawn later owns the ink; between the points only the
/// line does.
#[test]
fn the_topmost_of_a_line_and_a_scatter_owns_the_ink() {
    for scatter_on_top in [true, false] {
        for scale in [1.0f32, 2.0] {
            let mut r = renderer();
            upload(&mut r, "px", vec![2.0, 5.0, 8.0]);
            upload(&mut r, "py", vec![20.0, 80.0, 20.0]);
            let path = line("path", "px", "py", 3.0);
            let dots = markers("dots", "px", "py", RED);
            let series = if scatter_on_top {
                [path, dots]
            } else {
                [dots, path]
            };
            let id = register(&mut r, &bare(480, 360), &series, scale);
            draw(&mut r, id);
            let what = format!("scatter on top {scatter_on_top}, {scale}x");
            let top = if scatter_on_top { "dots" } else { "path" };

            let result = pick(&mut r, id, (5.0, 80.0), 0.0);
            assert_eq!(owner(&result), Some(top), "{what}");
            assert_eq!(point(&result), Some((top, 1)), "{what}");

            // Halfway along the first segment: the line alone.
            let result = pick(&mut r, id, (3.5, 50.0), 0.0);
            assert_eq!(owner(&result), Some("path"), "{what}");
            assert!(
                matches!(point(&result), Some(("path", 0 | 1))),
                "{what}: {result:?}"
            );
        }
    }
}

/// A panel with more series than an 8 bit slot could name: every series
/// draws a marker at the same place, and the last one drawn owns the ink
/// there. Every series is drawn once without a mask first (an export), then
/// by the first on-screen prepare, which records the mask in batches of
/// command buffers. Both times and the mask's memory are printed; none is
/// capped (rule 10). Slots past the
/// 16 bit range are written by the resolve pass in the crate's
/// `the_resolve_pass_writes_any_slot_where_the_series_inked`: registering
/// that many series here crashes lavapipe's descriptor allocator in the
/// series' style bind groups, before any mask is drawn.
#[test]
fn the_last_of_many_series_owns_the_ink() {
    for count in [1usize, 300, 5_000] {
        let mut r = renderer();
        upload(&mut r, "mx", vec![5.0]);
        upload(&mut r, "my", vec![50.0]);
        let series: Vec<_> = (0..count)
            .map(|k| markers(&format!("s{k}"), "mx", "my", BLUE))
            .collect();
        // A small panel with no margins: the data area is the chart.
        let mut config = bare(96, 72);
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
            axis.minor_tick_length = 0.0;
            axis.label_style.label_visible = false;
            axis.title_option.visible = false;
        }
        let id = register(&mut r, &config, &series, 1.0);
        // The same series drawn without any mask first: an export draws the
        // panel once and records no mask, so it is the plain draw's cost.
        let start = std::time::Instant::now();
        let image = r
            .export_png_blocking(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(1.0),
            )
            .expect("the plain draw of every series");
        assert!(!image.is_empty());
        let plain = start.elapsed();
        let before = r.gpu_memory_usage().live_bytes();
        let start = std::time::Instant::now();
        draw(&mut r, id);
        r.wait_submitted_work_blocking().unwrap();
        let recorded = start.elapsed();
        let after = r.gpu_memory_usage().live_bytes();
        let last = format!("s{}", count - 1);
        let result = pick(&mut r, id, (5.0, 50.0), 0.0);
        assert_eq!(owner(&result), Some(last.as_str()), "{count} series");
        assert_eq!(point(&result), Some((last.as_str(), 0)), "{count} series");
        let (_, panel, _) = r.chart_display_config(id).unwrap();
        let mask = 4 * u64::from(panel.width) * u64::from(panel.height);
        assert!(after >= before + mask, "the mask is charged");
        eprintln!(
            "INK_MASK_COST series={count} panel={}x{} mask_bytes={mask} plain_export_ms={} first_prepare_ms={}",
            panel.width,
            panel.height,
            plain.as_millis(),
            recorded.as_millis()
        );
    }
}
