use super::*;

fn pump_with_selection(f: &mut Fixture, operation: StreamingOperation) {
    for _ in 0..10000 {
        match f
            .renderer
            .request_stream_operation_ranges(operation)
            .unwrap()
        {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                let values: Vec<_> = ranges
                    .iter()
                    .map(|range| {
                        let original = &f.columns.iter().find(|(id, _)| *id == range.id).unwrap().1;
                        column(
                            original.data
                                [range.offset as usize..(range.offset + range.len) as usize]
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
                        source: crate::StreamColumnSource::HiLo(values),
                    })
                    .collect();
                f.renderer
                    .submit_stream_operation_ranges(operation, &bindings)
                    .unwrap();
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. } => f.renderer.wait_idle(),
            crate::AutoStreamingRangeRequest::Complete { .. } => return,
            crate::AutoStreamingRangeRequest::AllSubmitted { .. } => f.renderer.wait_idle(),
        }
    }
    panic!("field selection export did not terminate");
}

#[test]
fn heatmap_selection_matches_resident_with_bounded_axis_neighbours() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    for (centers, interpolated, cy, same_axis) in [
        (false, false, false, false),
        (true, false, true, false),
        (false, true, true, false),
        (true, true, false, false),
        (false, false, false, true),
        (true, false, true, true),
        (false, true, true, true),
        (true, true, false, true),
    ] {
        let mut f = setup_columns(centers, interpolated, cy, 4, 8, 2);
        if same_axis {
            f.renderer.cancel_streaming_chart(f.id).unwrap();
            f.series[0].y_column = "x".into();
            f.renderer.set_chart_series(f.id, f.series.clone()).unwrap();
            f.renderer
                .request_auto_streaming_chart(
                    f.id,
                    &f.view,
                    crate::StreamingChartOptions {
                        size: (360, 240),
                        clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                        max_primitives_per_chunk: 8,
                    },
                )
                .unwrap();
        }
        finish_display(&mut f);
        // Completed data remains unchanged while a new selected-cell suffix is supplied.
        let original_job = f.renderer.active_stream_job(f.id);
        let mut config = f.chart.config().clone();
        config.picked_data = Some(crate::config::DataSelectionsConfig {
            visible: true,
            refs: [0, 1, if interpolated { 1 } else { 2 }]
                .into_iter()
                .map(|index| PickedDataRef::MatrixCell {
                    source_id: Some("matrix".into()),
                    series_id: "heat".into(),
                    x_index: index,
                    y_index: 0,
                })
                .collect(),
            highlight_color: Color::new(0.9, 0.1, 0.6, 0.65),
            outline_width_px: 3.0,
            ..Default::default()
        });
        // Selection uses two axis ranges, independently of the draw's one-column tickets.
        f.renderer.set_chart_config(f.id, config.clone()).unwrap();
        f.chart = Chart::new(config);
        let mut selected_ranges = 0;
        for _ in 0..30 {
            match f.renderer.request_stream_selection_ranges(f.id).unwrap() {
                crate::StreamingSelectionRequest::Ready { ticket, ranges, .. } => {
                    assert!(ranges.len() <= 2);
                    assert!(
                        ranges
                            .iter()
                            .all(|range| (range.id == "x" || range.id == "y") && range.len <= 3)
                    );
                    selected_ranges += 1;
                    let values: Vec<_> = ranges
                        .iter()
                        .map(|range| {
                            let original =
                                &f.columns.iter().find(|(id, _)| *id == range.id).unwrap().1;
                            column(
                                original.data
                                    [range.offset as usize..(range.offset + range.len) as usize]
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
                            source: crate::StreamColumnSource::HiLo(values),
                        })
                        .collect();
                    f.renderer
                        .submit_stream_selection_ranges(ticket, &bindings)
                        .unwrap();
                }
                crate::StreamingSelectionRequest::Backpressure { .. } => f.renderer.wait_idle(),
                crate::StreamingSelectionRequest::Complete { .. } => break,
                crate::StreamingSelectionRequest::Failed { .. } => panic!("selection failed"),
            }
        }
        assert_eq!(selected_ranges, 3);
        assert_eq!(f.renderer.active_stream_job(f.id), original_job);
        let (device, queue) = data_render::shared_device().unwrap();
        let mut resident = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        for (id, data) in &f.columns {
            resident.add_hilo_column(*id, data).unwrap();
        }
        for scale in [1.0, 2.0] {
            let op = f
                .renderer
                .begin_stream_export(f.id, scale, Color::new(0.0, 0.0, 0.0, 0.0), 8)
                .unwrap();
            pump_with_selection(&mut f, op);
            let actual = pollster::block_on(f.renderer.finish_stream_export(op)).unwrap();
            let expected = resident
                .export_panel_rgba(&f.chart, &f.series, scale)
                .unwrap();
            assert_eq!(
                actual.rgba, expected.rgba,
                "cell selection centers={centers} interpolated={interpolated} cy={cy} scale={scale}"
            );
        }
    }
}

#[test]
fn heatmap_provider_residency_sizes_tables_and_commits_the_full_grid_closure() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut f = setup(true, true, false, 1, 3);
    finish_display(&mut f);
    f.renderer
        .set_auto_resident_working_set_limit(Some(1024 * 1024));
    let _ = f.renderer.set_memory_budget(Some(
        f.renderer.gpu_memory_usage().total_bytes() + 8 * 1024 * 1024,
    ));
    let (token, report) = f
        .renderer
        .begin_stream_residency(f.id, f.chart.config(), 3)
        .unwrap();
    let token = token.unwrap();
    let report = report.unwrap();
    assert!(report.is_admissible());
    let stops = f
        .chart
        .config()
        .colorbar
        .as_ref()
        .unwrap()
        .colormap
        .stops()
        .len()
        .max(1) as u64;
    assert_eq!(
        report.derived_resident_bytes,
        24 + 4 + stops * 16 + 16 + 8 + 64 + 80
    );
    for _ in 0..1000 {
        match f.renderer.request_stream_residency_ranges(token).unwrap() {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                assert_eq!(ranges.len(), 1);
                let range = &ranges[0];
                let original = &f.columns.iter().find(|(id, _)| *id == range.id).unwrap().1;
                let values = column(
                    original.data[range.offset as usize..(range.offset + range.len) as usize]
                        .to_vec(),
                );
                f.renderer
                    .submit_stream_residency_ranges(
                        token,
                        &[crate::StreamRangeSourceBinding {
                            id: &range.id,
                            revision: range.revision,
                            source_len: range.source_len,
                            offset: range.offset,
                            source: crate::StreamColumnSource::HiLo(&values),
                        }],
                    )
                    .unwrap();
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. }
            | crate::AutoStreamingRangeRequest::AllSubmitted { .. } => f.renderer.wait_idle(),
            crate::AutoStreamingRangeRequest::Complete { .. } => break,
        }
    }
    f.renderer.finish_stream_residency(token).unwrap();
    for id in ["x", "y", "z0", "z1"] {
        assert!(f.renderer.pool.handle_for(id).is_some());
        assert!(!f.renderer.streaming_sources.contains_key(id));
    }
    assert!(f.renderer.active_stream_job(f.id).is_none());
    let image = f
        .renderer
        .export_panel_rgba(&f.chart, &f.series, 1.0)
        .unwrap();
    assert_eq!((image.width, image.height), (360, 240));
}
