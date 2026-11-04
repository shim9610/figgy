use super::*;
use crate::test_gpu::BlockOn as _;

fn column(data: Vec<f32>) -> crate::Column<f32> {
    crate::Column { data }
}

fn setup(
    histogram: bool,
) -> (
    Renderer,
    ChartId,
    ChartView,
    Chart,
    Vec<SeriesConfig>,
    crate::Column<f32>,
    crate::Column<f32>,
) {
    let (device, queue) = data_render::shared_device().expect("stream replay test requires GPU");
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Bgra8Unorm,
        4096,
    )
    .unwrap();
    let count = if histogram { 257 } else { 17 };
    let x = column(
        (0..count)
            .map(|index| index as f32 / (count - 1) as f32)
            .collect(),
    );
    let y = column(
        (0..count - usize::from(histogram))
            .map(|index| 0.2 + (index % 13) as f32 / 20.0)
            .collect(),
    );
    renderer
        .register_streamed_columns(
            [("x", &x), ("y", &y)]
                .into_iter()
                .map(|(id, values)| crate::StreamColumn {
                    id: id.into(),
                    len: values.data.len() as u64,
                    revision: 7,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    statistics: crate::StreamStatistics::Unknown,
                })
                .collect(),
        )
        .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 240,
        height: 180,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    let series = vec![SeriesConfig {
        source_id: Some("original".into()),
        series_id: "series".into(),
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: if histogram {
            DataRenderType::Histogram {
                bar: DataBarStyleConfig {
                    fill_color: Color::new(0.2, 0.6, 0.8, 0.45),
                    border_color: Color::new(0.8, 0.2, 0.1, 0.65),
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 0.0,
                    width_ratio: 1.0,
                    orientation: crate::data_config::BarOrientation::Vertical,
                    bar_style_overrides: None,
                },
            }
        } else {
            DataRenderType::Xy(crate::data_config::XySeries {
                scatter: None,
                line: Some(DataLineStyleConfig {
                    line_color: Color::new(0.2, 0.6, 0.8, 0.45),
                    line_width: 2.0,
                    line_style: LineStylePreset::Solid,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                }),
                errors: None,
                stem: None,
            })
        },
    }];
    let id = renderer
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    renderer
        .configure_streaming(crate::StreamingLimits {
            max_active_charts: 1,
            max_in_flight_chunks: 2,
            max_columns_per_chunk: 7,
            max_chunk_input_bytes: 4096,
            max_in_flight_gpu_bytes: 16384,
        })
        .unwrap();
    // Deliberately use a smaller display: export must retain document dimensions.
    let (display, panel, _) = display_config_for_surface(chart.config(), (160, 120));
    let view = renderer
        .create_chart_view(&Chart::new(display.clone()), panel)
        .unwrap();
    renderer
        .request_auto_streaming_chart_with_config(
            id,
            &view,
            display,
            crate::StreamingChartOptions {
                size: (160, 120),
                clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                max_primitives_per_chunk: 13,
            },
        )
        .unwrap();
    let bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision: 7,
            source: crate::StreamColumnSource::Scalar(&x),
        },
        crate::StreamSourceBinding {
            id: "y",
            revision: 7,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];
    loop {
        match renderer.auto_stream_chart_step(id, &bindings).unwrap() {
            crate::AutoStreamingProgress::AllSubmitted { .. } => break,
            crate::AutoStreamingProgress::Complete { .. } => break,
            _ => renderer.wait_idle().unwrap(),
        }
    }
    renderer.wait_idle().unwrap();
    drop(
        renderer
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: id,
                view: &view,
            }])
            .unwrap(),
    );
    assert!(matches!(
        renderer.auto_stream_chart_step(id, &bindings).unwrap(),
        crate::AutoStreamingProgress::Complete { .. }
    ));
    (renderer, id, view, chart, series, x, y)
}

fn pump(
    renderer: &mut Renderer,
    operation: StreamingOperation,
    x: &crate::Column<f32>,
    y: &crate::Column<f32>,
) {
    loop {
        match renderer.request_stream_operation_ranges(operation).unwrap() {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                let values: Vec<_> = ranges
                    .iter()
                    .map(|range| {
                        let source = if range.id == "x" { x } else { y };
                        column(
                            source.data[range.offset as usize..(range.offset + range.len) as usize]
                                .to_vec(),
                        )
                    })
                    .collect();
                let bindings: Vec<_> = ranges
                    .iter()
                    .zip(&values)
                    .map(|(range, values)| crate::StreamRangeSourceBinding {
                        id: &range.id,
                        revision: range.revision,
                        source_len: range.source_len,
                        offset: range.offset,
                        source: crate::StreamColumnSource::Scalar(values),
                    })
                    .collect();
                renderer
                    .submit_stream_operation_ranges(operation, &bindings)
                    .unwrap();
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. } => renderer.wait_idle().unwrap(),
            crate::AutoStreamingRangeRequest::AllSubmitted { .. }
            | crate::AutoStreamingRangeRequest::Complete { .. } => break,
        }
    }
}

#[test]
fn streamed_export_replays_document_resolution_and_preserves_display() {
    for histogram in [false, true] {
        let (mut renderer, id, view, chart, series, x, y) = setup(histogram);
        let job = renderer.active_stream_job(id).unwrap();
        let status = renderer.stream_status(id).unwrap();
        let (device, queue) = data_render::shared_device().unwrap();
        let mut resident = Renderer::try_new(
            RendererDevice::new(device, queue).expect("figgy poll thread"),
            wgpu::TextureFormat::Rgba8Unorm,
            16384,
        )
        .unwrap();
        resident.add_column("x", &x).unwrap();
        resident.add_column("y", &y).unwrap();
        for scale in [1.0, 2.0] {
            let operation = renderer
                .begin_stream_export(id, scale, Color::new(0.0, 0.0, 0.0, 0.0), 11)
                .unwrap();
            assert!(
                renderer
                    .begin_stream_export(id, scale, Color::new(0.0, 0.0, 0.0, 0.0), 11)
                    .is_err()
            );
            pump(&mut renderer, operation, &x, &y);
            let actual =
                crate::test_gpu::block_on(renderer.finish_stream_export(operation)).unwrap();
            let expected = resident
                .export_rgba(
                    crate::ExportSource::chart(&chart, &series),
                    crate::ExportOptions::panel(scale),
                )
                .block_on()
                .unwrap();
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
            assert_eq!(
                actual.rgba, expected.rgba,
                "histogram={histogram} scale={scale}"
            );
            assert_eq!(renderer.active_stream_job(id), Some(job));
            assert_eq!(renderer.stream_status(id).unwrap(), status);
            drop(
                renderer
                    .prepare_registered(&[RegisteredChartDrawItem {
                        chart_id: id,
                        view: &view,
                    }])
                    .unwrap(),
            );
            crate::test_gpu::block_on(renderer.cancel_stream_operation_and_wait(operation))
                .unwrap();
        }
    }
}

#[test]
fn completed_stream_export_uses_latest_decoration_without_restarting_data() {
    let (mut renderer, id, _, chart, series, x, y) = setup(false);
    let original_job = renderer.active_stream_job(id);
    let original_count = renderer.stream_status(id).unwrap().submitted_primitives;
    let first = renderer
        .begin_stream_export(id, 1.0, Color::new(0.0, 0.0, 0.0, 0.0), 3)
        .unwrap();
    let mut latest = chart.config().clone();
    latest.chart_title.visible = true;
    latest.chart_title.text.segments = crate::text::rich_segments_from_text("Latest decoration");
    renderer.set_chart_config(id, latest.clone()).unwrap();
    pump(&mut renderer, first, &x, &y);
    let old_export = crate::test_gpu::block_on(renderer.finish_stream_export(first)).unwrap();
    let (device, queue) = data_render::shared_device().unwrap();
    let mut resident = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        16384,
    )
    .unwrap();
    resident.add_column("x", &x).unwrap();
    resident.add_column("y", &y).unwrap();
    let old_expected = resident
        .export_rgba(
            crate::ExportSource::chart(&chart, &series),
            crate::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    assert!(
        old_export.rgba == old_expected.rgba,
        "already-started export must stay frozen"
    );
    let second = renderer
        .begin_stream_export(id, 1.0, Color::new(0.0, 0.0, 0.0, 0.0), 3)
        .unwrap();
    pump(&mut renderer, second, &x, &y);
    let new_export = crate::test_gpu::block_on(renderer.finish_stream_export(second)).unwrap();
    let new_expected = resident
        .export_rgba(
            crate::ExportSource::chart(&Chart::new(latest), &series),
            crate::ExportOptions::panel(1.0),
        )
        .block_on()
        .unwrap();
    assert!(
        old_expected.rgba != new_expected.rgba,
        "fixture must change visible decoration"
    );
    assert!(
        new_export.rgba == new_expected.rgba,
        "new export must capture current decoration"
    );
    assert_eq!(renderer.active_stream_job(id), original_job);
    assert_eq!(
        renderer.stream_status(id).unwrap().submitted_primitives,
        original_count
    );
}

#[test]
fn streamed_operations_reject_stale_supply_cancel_idempotently_and_keep_display() {
    let (mut renderer, id, _, _, _, x, y) = setup(false);
    let screen = renderer.active_stream_job(id);
    let operation = renderer
        .begin_stream_export(id, 1.0, Color::new(0.0, 0.0, 0.0, 0.0), 3)
        .unwrap();
    let crate::AutoStreamingRangeRequest::Ready { ranges, .. } =
        renderer.request_stream_operation_ranges(operation).unwrap()
    else {
        panic!()
    };
    let values: Vec<_> = ranges
        .iter()
        .map(|range| {
            column(
                (if range.id == "x" { &x } else { &y }).data
                    [range.offset as usize..(range.offset + range.len) as usize]
                    .to_vec(),
            )
        })
        .collect();
    let stale: Vec<_> = ranges
        .iter()
        .zip(&values)
        .map(|(range, values)| crate::StreamRangeSourceBinding {
            id: &range.id,
            revision: 8,
            source_len: range.source_len,
            offset: range.offset,
            source: crate::StreamColumnSource::Scalar(values),
        })
        .collect();
    assert!(
        renderer
            .submit_stream_operation_ranges(operation, &stale)
            .is_err()
    );
    renderer.cancel_stream_operation(operation).unwrap();
    renderer.cancel_stream_operation(operation).unwrap();
    assert!(
        renderer
            .submit_stream_operation_ranges(operation, &stale)
            .is_err()
    );
    assert!(renderer.request_stream_operation_ranges(operation).is_err());
    assert_eq!(renderer.active_stream_job(id), screen);
    crate::test_gpu::block_on(renderer.cancel_stream_operation_and_wait(operation)).unwrap();
    assert_eq!(renderer.streaming_usage().in_flight_chunks, 0);
}

#[test]
fn streamed_pick_matches_resident_and_preserves_display() {
    for histogram in [false, true] {
        let (mut renderer, id, _, chart, series, x, y) = setup(histogram);
        let job = renderer.active_stream_job(id);
        let snapshot = renderer.auto_stream_snapshot(job.unwrap()).unwrap();
        let (device, queue) = data_render::shared_device().unwrap();
        let mut resident = Renderer::try_new(
            RendererDevice::new(device, queue).expect("figgy poll thread"),
            wgpu::TextureFormat::Rgba8Unorm,
            16384,
        )
        .unwrap();
        resident.add_column("x", &x).unwrap();
        resident.add_column("y", &y).unwrap();
        // Match the actual completed view and its currently captured style scale.
        let resident_id = resident
            .register_chart(snapshot.config.clone(), series)
            .unwrap();
        resident.enable_gpu_picking().unwrap();
        let panel = snapshot.config.chart_area.0;
        drop(snapshot);
        for position in [[80.0, 60.0], [130.0, 75.0], [2.0, 2.0]] {
            let request = GpuPickRequest {
                canvas_position_px: position,
                display_panel_px: panel,
                display_scale: 1.0,
                max_distance_px: 20.0,
            };
            let expected = crate::test_gpu::block_on(
                resident
                    .pick_chart_data(resident_id, request)
                    .unwrap()
                    .resolve(),
            )
            .unwrap();
            let operation =
                crate::test_gpu::block_on(renderer.begin_stream_pick_data(id, position, 20.0, 11))
                    .unwrap();
            pump(&mut renderer, operation, &x, &y);
            let actual =
                crate::test_gpu::block_on(renderer.finish_stream_pick_data(operation)).unwrap();
            assert_eq!(
                actual,
                expected,
                "histogram={histogram}, position={position:?}, chart={:?}",
                chart.config().chart_area
            );
            let expected_point = crate::test_gpu::block_on(
                resident.pick_chart(resident_id, request).unwrap().resolve(),
            )
            .unwrap();
            let point_operation =
                crate::test_gpu::block_on(renderer.begin_stream_pick_point(id, position, 20.0, 5))
                    .unwrap();
            pump(&mut renderer, point_operation, &x, &y);
            let actual_point =
                crate::test_gpu::block_on(renderer.finish_stream_pick_point(point_operation))
                    .unwrap();
            assert_eq!(actual_point, expected_point);
            assert_eq!(renderer.active_stream_job(id), job);
        }
    }
}

#[test]
fn stream_export_budget_rejection_is_preallocation_and_replay_survives_screen_retarget() {
    let (mut renderer, id, _, chart, series, x, y) = setup(false);
    renderer.end_gpu_frame();
    renderer.wait_idle().unwrap();
    renderer.service_stream_requests();
    let before = renderer.gpu_memory_usage().total_bytes();
    let screen = renderer.active_stream_job(id);
    let _ = renderer.set_memory_budget(Some(before + 1));
    assert!(
        renderer
            .begin_stream_export(id, 2.0, Color::WHITE, 11)
            .is_err()
    );
    assert_eq!(renderer.gpu_memory_usage().total_bytes(), before);
    assert_eq!(renderer.active_stream_job(id), screen);
    let _ = renderer.set_memory_budget(None);
    let operation = renderer
        .begin_stream_export(id, 2.0, Color::WHITE, 11)
        .unwrap();
    let next_metadata = ["x", "y"]
        .into_iter()
        .map(|source| {
            let mut source = renderer
                .auto_stream_snapshot(screen.unwrap())
                .unwrap()
                .sources[source]
                .clone();
            source.revision = 8;
            source.statistics = crate::StreamStatistics::Unknown;
            source
        })
        .collect();
    renderer.replace_streamed_columns(next_metadata).unwrap();
    renderer
        .ensure_target_format(wgpu::TextureFormat::Rgba8Unorm)
        .unwrap();
    pump(&mut renderer, operation, &x, &y);
    let actual = crate::test_gpu::block_on(renderer.finish_stream_export(operation)).unwrap();
    let (device, queue) = data_render::shared_device().unwrap();
    let mut resident = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        16384,
    )
    .unwrap();
    resident.add_column("x", &x).unwrap();
    resident.add_column("y", &y).unwrap();
    let expected = crate::test_gpu::block_on(resident.export_rgba(
        crate::ExportSource::chart(&chart, &series),
        crate::ExportOptions::panel(2.0).with_clear(Color::WHITE),
    ))
    .unwrap();
    assert_eq!(actual.rgba, expected.rgba);
    crate::test_gpu::block_on(renderer.cancel_stream_operation_and_wait(operation)).unwrap();
    assert_eq!(renderer.streaming_usage().in_flight_chunks, 0);
}

/// Rows of every streamed column of `wide_stream`.
const WIDE_ROWS: usize = 24;

/// The streamed columns of `wide_stream`: `x`, `y`, the four error columns,
/// the marker and error bar style indices `ps` / `es`, and `far`, whose
/// values all lie above the visible range.
fn wide_values() -> Vec<(&'static str, crate::Column<f32>)> {
    let n = WIDE_ROWS;
    let ramp = |f: &dyn Fn(usize) -> f32| column((0..n).map(f).collect());
    vec![
        ("x", ramp(&|i| 0.05 + i as f32 * 0.9 / n as f32)),
        ("y", ramp(&|i| 0.2 + (i % 7) as f32 * 0.08)),
        ("ex_lo", ramp(&|_| 0.02)),
        ("ex_hi", ramp(&|_| 0.03)),
        ("ey_lo", ramp(&|_| 0.04)),
        ("ey_hi", ramp(&|_| 0.05)),
        ("ps", ramp(&|i| (i % 2) as f32)),
        ("es", ramp(&|i| ((i + 1) % 2) as f32)),
        ("far", ramp(&|_| 5.0)),
    ]
}

/// A scatter of `x` over `y` with a two-row style table read through `ps`.
fn styled_scatter(id: &str, x: &str, y: &str) -> SeriesConfig {
    SeriesConfig {
        source_id: None,
        series_id: id.into(),
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(DataScatterStyleConfig {
                point_color: Color::new(0.1, 0.1, 0.6, 0.8),
                point_shape: ScatterShape::CircleFilled,
                point_size: 6.0,
                point_style_index_column: Some("ps".into()),
                point_style_table: Some(vec![
                    DataScatterPointStyleConfig {
                        point_size: Some(4.0),
                        ..Default::default()
                    },
                    DataScatterPointStyleConfig {
                        point_size: Some(8.0),
                        ..Default::default()
                    },
                ]),
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            }),
            line: None,
            errors: None,
            stem: None,
        }),
    }
}

/// A line of `x` over `y`.
fn wide_line(id: &str, y: &str) -> SeriesConfig {
    SeriesConfig {
        source_id: None,
        series_id: id.into(),
        label: None,
        x_column: "x".into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig {
                line_color: Color::new(0.2, 0.6, 0.2, 0.7),
                line_width: 1.5,
                line_style: LineStylePreset::Solid,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            }),
            errors: None,
            stem: None,
        }),
    }
}

/// The seven-column series: a line, styled markers and styled asymmetric
/// error bars on both axes, each role on a column of its own.
fn seven_column_series(id: &str) -> SeriesConfig {
    let DataRenderType::Xy(crate::data_config::XySeries {
        scatter: Some(scatter),
        line: None,
        errors: None,
        ..
    }) = styled_scatter(id, "x", "y").render_type
    else {
        unreachable!()
    };
    let DataRenderType::Xy(crate::data_config::XySeries {
        scatter: None,
        line: Some(line),
        errors: None,
        ..
    }) = wide_line(id, "y").render_type
    else {
        unreachable!()
    };
    SeriesConfig {
        render_type: DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(scatter),
            line: Some(line),
            errors: Some(crate::data_config::ErrorBars {
                x: Some(ErrorRef::Asymmetric {
                    lower: "ex_lo".into(),
                    upper: "ex_hi".into(),
                }),
                y: Some(ErrorRef::Asymmetric {
                    lower: "ey_lo".into(),
                    upper: "ey_hi".into(),
                }),
                style: DataErrorBarStyleConfig {
                    error_bar_color: Color::new(0.6, 0.1, 0.1, 0.8),
                    error_bar_width: 1.0,
                    error_bar_cap_size: 3.0,
                    cap_width: 1.0,
                    error_bar_style_table: Some(vec![
                        Default::default(),
                        crate::data_config::DataErrorBarPointStyleConfig {
                            error_bar_width: Some(2.0),
                            ..Default::default()
                        },
                    ]),
                    error_bar_style_index_column: Some("es".into()),
                    error_bar_style_overrides: None,
                },
            }),
            stem: None,
        }),
        ..wide_line(id, "y")
    }
}

/// A renderer streaming `series` from `wide_values` into a 240 x 180 chart
/// with the packed view enabled, run to completion.
fn wide_stream(
    series: Vec<SeriesConfig>,
) -> (
    Renderer,
    ChartId,
    ChartView,
    Vec<(&'static str, crate::Column<f32>)>,
) {
    let (device, queue) = data_render::shared_device().expect("stream test requires GPU");
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    let values = wide_values();
    renderer
        .register_streamed_columns(
            values
                .iter()
                .map(|(id, values)| crate::StreamColumn {
                    id: (*id).into(),
                    len: values.data.len() as u64,
                    revision: 3,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    statistics: crate::StreamStatistics::Unknown,
                })
                .collect(),
        )
        .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 240,
        height: 180,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    let id = renderer
        .register_chart(chart.config().clone(), series)
        .unwrap();
    renderer
        .configure_streaming(crate::StreamingLimits {
            max_active_charts: 1,
            max_in_flight_chunks: 2,
            max_columns_per_chunk: 7,
            max_chunk_input_bytes: 4096,
            max_in_flight_gpu_bytes: 16384,
        })
        .unwrap();
    let budget = renderer.gpu_memory_usage().total_bytes() + 64 * 1024 * 1024;
    let _ = renderer.set_memory_budget(Some(budget));
    let _ = renderer.set_auto_resident_working_set_limit(Some(500_000_000));
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    request_wide_stream(&mut renderer, id, &view);
    finish_wide_stream(&mut renderer, id, &view, &values);
    (renderer, id, view, values)
}

/// Ask for an automatic stream of `id` at the `wide_stream` size.
fn request_wide_stream(renderer: &mut Renderer, id: ChartId, view: &ChartView) {
    renderer
        .request_auto_streaming_chart(
            id,
            view,
            crate::StreamingChartOptions {
                size: (240, 180),
                clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                max_primitives_per_chunk: 7,
            },
        )
        .unwrap();
}

/// The step bindings of every `wide_values` column.
fn wide_bindings<'a>(
    values: &'a [(&'static str, crate::Column<f32>)],
) -> Vec<crate::StreamSourceBinding<'a>> {
    values
        .iter()
        .map(|(id, values)| crate::StreamSourceBinding {
            id,
            revision: 3,
            source: crate::StreamColumnSource::Scalar(values),
        })
        .collect()
}

/// Step the automatic stream of `id` to completion and present it.
fn finish_wide_stream(
    renderer: &mut Renderer,
    id: ChartId,
    view: &ChartView,
    values: &[(&'static str, crate::Column<f32>)],
) {
    let bindings = wide_bindings(values);
    loop {
        match renderer.auto_stream_chart_step(id, &bindings).unwrap() {
            crate::AutoStreamingProgress::AllSubmitted { .. }
            | crate::AutoStreamingProgress::Complete { .. } => break,
            _ => renderer.wait_idle().unwrap(),
        }
    }
    renderer.wait_idle().unwrap();
    drop(
        renderer
            .prepare_registered(&[RegisteredChartDrawItem { chart_id: id, view }])
            .unwrap(),
    );
    assert!(matches!(
        renderer.auto_stream_chart_step(id, &bindings).unwrap(),
        crate::AutoStreamingProgress::Complete { .. }
    ));
}

/// Feed every range `operation` asks for from `values` until it is done.
fn pump_wide(
    renderer: &mut Renderer,
    operation: StreamingOperation,
    values: &[(&'static str, crate::Column<f32>)],
) {
    loop {
        match renderer.request_stream_operation_ranges(operation).unwrap() {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                let slices: Vec<_> = ranges
                    .iter()
                    .map(|range| {
                        let (_, source) = values.iter().find(|(id, _)| *id == range.id).unwrap();
                        column(
                            source.data[range.offset as usize..(range.offset + range.len) as usize]
                                .to_vec(),
                        )
                    })
                    .collect();
                let bindings: Vec<_> = ranges
                    .iter()
                    .zip(&slices)
                    .map(|(range, values)| crate::StreamRangeSourceBinding {
                        id: &range.id,
                        revision: range.revision,
                        source_len: range.source_len,
                        offset: range.offset,
                        source: crate::StreamColumnSource::Scalar(values),
                    })
                    .collect();
                renderer
                    .submit_stream_operation_ranges(operation, &bindings)
                    .unwrap();
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. } => renderer.wait_idle().unwrap(),
            crate::AutoStreamingRangeRequest::AllSubmitted { .. }
            | crate::AutoStreamingRangeRequest::Complete { .. } => break,
        }
    }
}

/// `series` without its marker and error bar style maps: the packed view
/// holds only unstyled geometry.
fn unstyled(mut series: SeriesConfig) -> SeriesConfig {
    match &mut series.render_type {
        DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(scatter),
            line: None,
            errors: None,
            ..
        }) => {
            scatter.point_style_index_column = None;
            scatter.point_style_table = None;
        }
        DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(scatter),
            line: Some(_),
            errors:
                Some(crate::data_config::ErrorBars {
                    x: Some(_),
                    y: Some(_),
                    style: err_style,
                }),
            ..
        }) => {
            scatter.point_style_index_column = None;
            scatter.point_style_table = None;
            err_style.error_bar_style_index_column = None;
            err_style.error_bar_style_table = None;
        }
        _ => {}
    }
    series
}

/// STREAM_PHASE_COLUMNS_LE_7, PACKED_PAGE_RANGES_MATCH_PHASE,
/// STREAM_SNAPSHOT_SOURCES_COVER_SERIES and STREAM_STYLES_MATCH_SERIES
/// (#152), after an automatic stream completes: every phase of every
/// snapshot series names one to seven distinct columns, its data columns
/// first (seven for the styled XY error bars, one where every role is the
/// same column); every column a series reads is a snapshot source; the
/// snapshot has one style per series; and every packed page names a
/// snapshot series and holds one range per column of its phase. A scatter,
/// a line, the XY error bar series, a line wholly outside the view (no
/// visible page) and a series on one column, styled (no packed view) and
/// unstyled, then 200 lines; each is picked from its packed view and
/// exported from its replay.
#[test]
fn completed_auto_streams_keep_their_pages_sources_and_styles_per_series() {
    let mixed = vec![
        styled_scatter("scatter", "x", "y"),
        wide_line("line", "y"),
        seven_column_series("bars"),
        wide_line("hidden", "far"),
        styled_scatter("same", "x", "x"),
    ];
    let plain: Vec<SeriesConfig> = mixed.iter().cloned().map(unstyled).collect();
    let many: Vec<SeriesConfig> = (0..200).map(|k| wide_line(&format!("l{k}"), "y")).collect();
    for (series, styled) in [(mixed, true), (plain, false), (many, false)] {
        let (mut renderer, id, _view, values) = wide_stream(series.clone());
        let job = renderer.active_stream_job(id).unwrap();
        let snapshot = renderer
            .auto_stream_snapshot(job)
            .expect("an automatic snapshot");
        assert_eq!(snapshot.series, series);
        assert_eq!(snapshot.styles.styles.len(), snapshot.series.len());
        let style = usize::from(styled);
        for declaration in &snapshot.series {
            visit_series_columns(declaration, &mut |column| {
                assert!(snapshot.sources.contains_key(column), "{column}");
            });
            let phases = stream_draw_phases(&snapshot.config.draw_style, declaration).unwrap();
            assert!(!phases.is_empty());
            for phase in phases {
                let columns = stream_phase_columns(declaration, *phase);
                assert!((1..=7).contains(&columns.count), "{phase:?}");
                assert!(columns.data_count <= columns.count);
                let mut ids = columns.ids[..columns.count].to_vec();
                ids.sort_unstable();
                ids.dedup();
                assert_eq!(ids.len(), columns.count);
                let expected = match (declaration.series_id.as_str(), phase) {
                    ("bars", StreamDrawPhase::Errorbar) => {
                        assert_eq!(columns.data_count, 6);
                        6 + style
                    }
                    ("bars" | "scatter", StreamDrawPhase::Scatter) => 2 + style,
                    ("same", _) => 1 + style,
                    _ => 2,
                };
                assert_eq!(
                    columns.count, expected,
                    "{} {phase:?}",
                    declaration.series_id
                );
            }
        }
        let mut one = styled_scatter("one", "x", "x");
        if let DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(scatter),
            line: None,
            errors: None,
            ..
        }) = &mut one.render_type
        {
            scatter.point_style_index_column = Some("x".into());
        }
        assert_eq!(
            stream_phase_columns(&one, StreamDrawPhase::Scatter).count,
            1
        );

        let cache = renderer
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job && draw.auxiliary.is_none())
            .and_then(|draw| draw.view_cache.clone());
        match cache {
            None => {
                assert!(styled, "no packed view");
                let status = renderer.view_residency_status(id).unwrap();
                assert_eq!(status.refusal_reason, Some("unsupported_geometry"));
            }
            Some(cache) => {
                assert!(!styled);
                assert!(!cache.chunks.is_empty());
                for page in &cache.chunks {
                    assert!(page.series < snapshot.series.len());
                    let declaration = &snapshot.series[page.series];
                    assert_ne!(
                        declaration.series_id, "hidden",
                        "an invisible series has a page"
                    );
                    let columns = stream_phase_columns(declaration, page.phase);
                    assert_eq!(
                        page.ranges.len(),
                        columns.count,
                        "{}",
                        declaration.series_id
                    );
                    assert_eq!(page.chunk.columns.len(), page.ranges.len());
                }
                let picked = crate::test_gpu::block_on(renderer.pick_chart_view_cache(
                    id,
                    [120.0, 90.0],
                    40.0,
                ))
                .unwrap();
                if let Some(point) = picked {
                    assert!(series.iter().any(|s| s.series_id == point.series_id));
                }
            }
        }
        drop(snapshot);
        let operation = renderer
            .begin_stream_export(id, 1.0, Color::new(0.0, 0.0, 0.0, 0.0), 7)
            .unwrap();
        pump_wide(&mut renderer, operation, &values);
        let image = crate::test_gpu::block_on(renderer.finish_stream_export(operation)).unwrap();
        assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
        assert!(image.rgba.chunks_exact(4).any(|p| p[3] != 0));
    }
}

/// STREAM_SNAPSHOT_SOURCES_COVER_SERIES and STREAM_STYLES_MATCH_SERIES
/// (#152) for a resident handoff: the snapshot built from the handoff's
/// columns holds every column its series visit (the marker style index
/// included) and one style per series, on charts of 1 and 50 styled
/// scatters; a handoff that omits the style index column is refused; the
/// handoff runs to completion and the completed stream exports.
#[test]
fn a_resident_handoff_snapshot_covers_every_series_column() {
    for count in [1usize, 50] {
        let (device, queue) = data_render::shared_device().expect("stream test requires GPU");
        let mut renderer = Renderer::try_new(
            RendererDevice::new(device, queue).expect("figgy poll thread"),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        let values: Vec<_> = wide_values()
            .into_iter()
            .filter(|(id, _)| matches!(*id, "x" | "y" | "ps"))
            .collect();
        for (id, column) in &values {
            renderer.add_column(*id, column).unwrap();
        }
        let series: Vec<SeriesConfig> = (0..count)
            .map(|k| styled_scatter(&format!("s{k}"), "x", "y"))
            .collect();
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 240,
            height: 180,
        });
        let id = renderer
            .register_chart(config.clone(), series.clone())
            .unwrap();
        renderer
            .configure_streaming(crate::StreamingLimits {
                max_active_charts: 1,
                max_in_flight_chunks: 2,
                max_columns_per_chunk: 7,
                max_chunk_input_bytes: 4096,
                max_in_flight_gpu_bytes: 16384,
            })
            .unwrap();
        let view = renderer
            .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
            .unwrap();
        let metadata = |ids: &[&str]| -> Vec<crate::StreamColumn> {
            ids.iter()
                .map(|id| crate::StreamColumn {
                    id: (*id).into(),
                    len: WIDE_ROWS as u64,
                    revision: 1,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    statistics: crate::StreamStatistics::Unknown,
                })
                .collect()
        };
        let options = crate::StreamingChartOptions {
            size: (240, 180),
            clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
            max_primitives_per_chunk: 7,
        };
        let missing = renderer.request_resident_stream_handoff_with_config(
            id,
            &view,
            config.clone(),
            metadata(&["x", "y"]),
            options,
        );
        assert!(
            matches!(missing, Err(FiggyError::InvalidStreamSource { .. })),
            "{missing:?}"
        );
        assert!(matches!(
            renderer
                .request_resident_stream_handoff_with_config(
                    id,
                    &view,
                    config.clone(),
                    metadata(&["x", "y", "ps"]),
                    options,
                )
                .unwrap(),
            crate::AutoStreamingRequest::Started { .. }
        ));
        let job = renderer.active_stream_job(id).unwrap();
        let snapshot = renderer
            .auto_stream_snapshot(job)
            .expect("a handoff snapshot");
        assert_eq!(snapshot.series, series);
        assert_eq!(snapshot.styles.styles.len(), snapshot.series.len());
        for declaration in &snapshot.series {
            visit_series_columns(declaration, &mut |column| {
                assert!(snapshot.sources.contains_key(column), "{column}");
            });
        }
        drop(snapshot);
        loop {
            match renderer.auto_stream_chart_request_ranges(id).unwrap() {
                crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                    let slices: Vec<_> = ranges
                        .iter()
                        .map(|range| {
                            let (_, source) =
                                values.iter().find(|(id, _)| *id == range.id).unwrap();
                            column(
                                source.data
                                    [range.offset as usize..(range.offset + range.len) as usize]
                                    .to_vec(),
                            )
                        })
                        .collect();
                    let bindings: Vec<_> = ranges
                        .iter()
                        .zip(&slices)
                        .map(|(range, values)| crate::StreamRangeSourceBinding {
                            id: &range.id,
                            revision: range.revision,
                            source_len: range.source_len,
                            offset: range.offset,
                            source: crate::StreamColumnSource::Scalar(values),
                        })
                        .collect();
                    renderer
                        .auto_stream_chart_submit_ranges(id, &bindings)
                        .unwrap();
                }
                crate::AutoStreamingRangeRequest::Backpressure { .. } => {
                    renderer.wait_idle().unwrap()
                }
                crate::AutoStreamingRangeRequest::AllSubmitted { .. } => {
                    renderer.wait_idle().unwrap();
                    drop(
                        renderer
                            .prepare_registered(&[RegisteredChartDrawItem {
                                chart_id: id,
                                view: &view,
                            }])
                            .unwrap(),
                    );
                }
                crate::AutoStreamingRangeRequest::Complete { .. } => break,
            }
        }
        let job = renderer.active_stream_job(id).unwrap();
        if let Some(snapshot) = renderer.auto_stream_snapshot(job) {
            assert_eq!(snapshot.styles.styles.len(), snapshot.series.len());
            for declaration in &snapshot.series {
                visit_series_columns(declaration, &mut |column| {
                    assert!(snapshot.sources.contains_key(column), "{column}");
                });
            }
        }
        let operation = renderer
            .begin_stream_export(id, 1.0, Color::new(0.0, 0.0, 0.0, 0.0), 7)
            .unwrap();
        pump_wide(&mut renderer, operation, &values);
        let image = crate::test_gpu::block_on(renderer.finish_stream_export(operation)).unwrap();
        assert!(image.rgba.chunks_exact(4).any(|p| p[3] != 0));
    }
}

/// STREAM_DRAW_SERIES_IN_RANGE and STREAM_STYLES_MATCH_SERIES (#152) for
/// automatic streams: the snapshot's series and styles are fixed for the
/// job, so a series edit in the middle of a 300-series stream (to 1
/// series, to 299, or a recolor of the same count) never moves the cursor
/// past the list it walks; stepping on finishes a stream whose snapshot
/// has one style per series and names the edited list, which then picks
/// from its packed view, steps its selection and exports.
#[test]
fn an_auto_stream_walks_its_own_snapshot_through_series_edits() {
    let many: Vec<SeriesConfig> = (0..300).map(|k| wide_line(&format!("l{k}"), "y")).collect();
    let edits: [Vec<SeriesConfig>; 3] = [
        vec![wide_line("only", "y")],
        many[..299].to_vec(),
        many.iter()
            .cloned()
            .map(|mut s| {
                if let DataRenderType::Xy(crate::data_config::XySeries {
                    scatter: None,
                    line: Some(line),
                    errors: None,
                    ..
                }) = &mut s.render_type
                {
                    line.line_color = Color::new(0.8, 0.1, 0.1, 0.9);
                }
                s
            })
            .collect(),
    ];
    for edited in edits {
        let (mut renderer, id, view, values) = wide_stream(many.clone());
        request_wide_stream(&mut renderer, id, &view);
        let mut config = renderer.chart_config(id).unwrap().clone();
        config.bottom_x.max = 0.9;
        renderer.set_chart_config(id, config).unwrap();
        request_wide_stream(&mut renderer, id, &view);
        let bindings = wide_bindings(&values);
        for _ in 0..40 {
            match renderer.auto_stream_chart_step(id, &bindings).unwrap() {
                crate::AutoStreamingProgress::AllSubmitted { .. }
                | crate::AutoStreamingProgress::Complete { .. } => break,
                _ => renderer.wait_idle().unwrap(),
            }
        }
        let job = renderer.active_stream_job(id).unwrap();
        let before = renderer.auto_stream_snapshot(job).unwrap();
        assert_eq!(before.series.len(), 300);
        renderer.set_chart_series(id, edited.clone()).unwrap();
        // The running job keeps walking the snapshot it started with, or is
        // replaced; either way its cursor is within that snapshot's list.
        for _ in 0..40 {
            let step = renderer.auto_stream_chart_step(id, &bindings);
            if let Some(job) = renderer.active_stream_job(id)
                && let Some(snapshot) = renderer.auto_stream_snapshot(job)
            {
                assert_eq!(snapshot.styles.styles.len(), snapshot.series.len());
                if let Some(cursor) = renderer.stream_cursor_series_for_test(job) {
                    assert!(cursor <= snapshot.series.len());
                }
            }
            match step {
                Ok(crate::AutoStreamingProgress::AllSubmitted { .. })
                | Ok(crate::AutoStreamingProgress::Complete { .. }) => break,
                Ok(_) => renderer.wait_idle().unwrap(),
                Err(error) => panic!("{error:?}"),
            }
        }
        drop(before);
        request_wide_stream(&mut renderer, id, &view);
        finish_wide_stream(&mut renderer, id, &view, &values);
        let job = renderer.active_stream_job(id).unwrap();
        let snapshot = renderer.auto_stream_snapshot(job).unwrap();
        assert_eq!(snapshot.series, edited);
        assert_eq!(snapshot.styles.styles.len(), edited.len());
        drop(snapshot);
        let picked =
            crate::test_gpu::block_on(renderer.pick_chart_view_cache(id, [120.0, 90.0], 40.0))
                .unwrap();
        if let Some(point) = &picked {
            assert!(edited.iter().any(|s| s.series_id == point.series_id));
        }
        let mut selected = renderer.chart_config(id).unwrap().clone();
        selected.picked_data = Some(crate::DataSelectionsConfig {
            refs: vec![crate::PickedDataRef::Point {
                source_id: None,
                series_id: edited.last().unwrap().series_id.clone(),
                point_index: 3,
            }],
            ..Default::default()
        });
        renderer.set_chart_config(id, selected).unwrap();
        for _ in 0..100 {
            match renderer.request_stream_selection_ranges(id).unwrap() {
                crate::StreamingSelectionRequest::Ready { ticket, ranges, .. } => {
                    let slices: Vec<_> = ranges
                        .iter()
                        .map(|range| {
                            let (_, source) =
                                values.iter().find(|(id, _)| *id == range.id).unwrap();
                            column(
                                source.data
                                    [range.offset as usize..(range.offset + range.len) as usize]
                                    .to_vec(),
                            )
                        })
                        .collect();
                    let bindings: Vec<_> = ranges
                        .iter()
                        .zip(&slices)
                        .map(|(range, values)| crate::StreamRangeSourceBinding {
                            id: &range.id,
                            revision: range.revision,
                            source_len: range.source_len,
                            offset: range.offset,
                            source: crate::StreamColumnSource::Scalar(values),
                        })
                        .collect();
                    renderer
                        .submit_stream_selection_ranges(ticket, &bindings)
                        .unwrap();
                }
                crate::StreamingSelectionRequest::Backpressure { .. } => {
                    renderer.wait_idle().unwrap()
                }
                crate::StreamingSelectionRequest::Complete { .. }
                | crate::StreamingSelectionRequest::Failed { .. } => break,
            }
        }
        let operation = renderer
            .begin_stream_export(id, 1.0, Color::new(0.0, 0.0, 0.0, 0.0), 7)
            .unwrap();
        pump_wide(&mut renderer, operation, &values);
        let image = crate::test_gpu::block_on(renderer.finish_stream_export(operation)).unwrap();
        assert!(image.rgba.chunks_exact(4).any(|p| p[3] != 0));
    }
}
