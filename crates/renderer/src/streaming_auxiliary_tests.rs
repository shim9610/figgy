use super::*;

fn column(data: Vec<f32>) -> crate::Column<f32> {
    crate::Column {
        min: data.iter().copied().fold(f32::INFINITY, f32::min),
        max: data.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        data,
    }
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
        RendererDevice::new(device, queue),
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
            DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_color: Color::new(0.2, 0.6, 0.8, 0.45),
                    line_width: 2.0,
                    line_style: LineStylePreset::Solid,
                },
            }
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
            _ => renderer.wait_idle(),
        }
    }
    renderer.wait_idle();
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
            crate::AutoStreamingRangeRequest::Backpressure { .. } => renderer.wait_idle(),
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
            RendererDevice::new(device, queue),
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
            let actual = pollster::block_on(renderer.finish_stream_export(operation)).unwrap();
            let expected = resident.export_panel_rgba(&chart, &series, scale).unwrap();
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
            pollster::block_on(renderer.cancel_stream_operation_and_wait(operation)).unwrap();
        }
    }
}

#[test]
fn completed_stream_export_uses_latest_decoration_without_restarting_data() {
    let (mut renderer, id, _, chart, series, x, y) = setup(false);
    let original_job = renderer.active_stream_job(id);
    let original_count = renderer.stream_status(id).unwrap().submitted_primitives;
    let first = renderer.begin_stream_export(id, 1.0, Color::new(0.0, 0.0, 0.0, 0.0), 3).unwrap();
    let mut latest = chart.config().clone();
    latest.chart_title.visible = true;
    latest.chart_title.text.segments = crate::text::rich_segments_from_text("Latest decoration");
    renderer.set_chart_config(id, latest.clone()).unwrap();
    pump(&mut renderer, first, &x, &y);
    let old_export = pollster::block_on(renderer.finish_stream_export(first)).unwrap();
    let (device, queue) = data_render::shared_device().unwrap();
    let mut resident = Renderer::try_new(RendererDevice::new(device, queue), wgpu::TextureFormat::Rgba8Unorm, 16384).unwrap();
    resident.add_column("x", &x).unwrap();
    resident.add_column("y", &y).unwrap();
    let old_expected = resident.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert!(old_export.rgba == old_expected.rgba, "already-started export must stay frozen");
    let second = renderer.begin_stream_export(id, 1.0, Color::new(0.0, 0.0, 0.0, 0.0), 3).unwrap();
    pump(&mut renderer, second, &x, &y);
    let new_export = pollster::block_on(renderer.finish_stream_export(second)).unwrap();
    let new_expected = resident.export_panel_rgba(&Chart::new(latest), &series, 1.0).unwrap();
    assert!(old_expected.rgba != new_expected.rgba, "fixture must change visible decoration");
    assert!(new_export.rgba == new_expected.rgba, "new export must capture current decoration");
    assert_eq!(renderer.active_stream_job(id), original_job);
    assert_eq!(renderer.stream_status(id).unwrap().submitted_primitives, original_count);
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
    pollster::block_on(renderer.cancel_stream_operation_and_wait(operation)).unwrap();
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
            RendererDevice::new(device, queue),
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
            let expected = pollster::block_on(
                resident
                    .pick_chart_data(resident_id, request)
                    .unwrap()
                    .resolve(),
            )
            .unwrap();
            let operation =
                pollster::block_on(renderer.begin_stream_pick_data(id, position, 20.0, 11))
                    .unwrap();
            pump(&mut renderer, operation, &x, &y);
            let actual = pollster::block_on(renderer.finish_stream_pick_data(operation)).unwrap();
            assert_eq!(
                actual,
                expected,
                "histogram={histogram}, position={position:?}, chart={:?}",
                chart.config().chart_area
            );
            let expected_point =
                pollster::block_on(resident.pick_chart(resident_id, request).unwrap().resolve())
                    .unwrap();
            let point_operation =
                pollster::block_on(renderer.begin_stream_pick_point(id, position, 20.0, 5))
                    .unwrap();
            pump(&mut renderer, point_operation, &x, &y);
            let actual_point =
                pollster::block_on(renderer.finish_stream_pick_point(point_operation)).unwrap();
            assert_eq!(actual_point, expected_point);
            assert_eq!(renderer.active_stream_job(id), job);
        }
    }
}

#[test]
fn stream_export_budget_rejection_is_preallocation_and_replay_survives_screen_retarget() {
    let (mut renderer, id, _, chart, series, x, y) = setup(false);
    renderer.end_gpu_frame();
    renderer.wait_idle();
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
    let actual = pollster::block_on(renderer.finish_stream_export(operation)).unwrap();
    let (device, queue) = data_render::shared_device().unwrap();
    let mut resident = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        16384,
    )
    .unwrap();
    resident.add_column("x", &x).unwrap();
    resident.add_column("y", &y).unwrap();
    let expected = pollster::block_on(resident.export_panel_rgba_with_clear_async(
        &chart,
        &series,
        2.0,
        Color::WHITE,
    ))
    .unwrap();
    assert_eq!(actual.rgba, expected.rgba);
    pollster::block_on(renderer.cancel_stream_operation_and_wait(operation)).unwrap();
    assert_eq!(renderer.streaming_usage().in_flight_chunks, 0);
}
