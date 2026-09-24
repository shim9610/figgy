#[cfg(test)]
mod tests {
    use super::super::*;

    fn column(values: Vec<f64>) -> crate::Column<f64> {
        crate::Column {
            min: values.iter().copied().fold(f64::INFINITY, f64::min),
            max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            data: values,
        }
    }

    fn setup() -> (Renderer, ChartId, Config, [crate::Column<f64>; 2]) {
        let (device, queue) = data_render::shared_device().unwrap();
        let mut renderer = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        renderer
            .configure_streaming(crate::StreamingLimits {
                max_active_charts: 2,
                max_in_flight_chunks: 2,
                max_columns_per_chunk: 2,
                max_chunk_input_bytes: 64,
                max_in_flight_gpu_bytes: 256,
            })
            .unwrap();
        renderer
            .add_column("survivor", &column(vec![19.25, -2.5]))
            .unwrap();
        let values = [
            column((0..13).map(|i| i as f64 * 0.13 - 0.4).collect()),
            column((0..13).map(|i| 1e12 + i as f64 * 0.125).collect()),
        ];
        renderer
            .register_streamed_columns(
                [
                    ("x", crate::StreamEncoding::ScalarF32),
                    ("y", crate::StreamEncoding::HiLoF32),
                ]
                .into_iter()
                .map(|(id, encoding)| crate::StreamColumn {
                    id: id.into(),
                    len: 13,
                    revision: 1,
                    encoding,
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
            width: 320,
            height: 240,
        });
        let series = SeriesConfig {
            series_id: "candidate".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_width: 1.0,
                    line_color: Color::BLACK,
                    line_style: LineStylePreset::Solid,
                },
            },
        };
        let chart = renderer
            .register_chart(config.clone(), vec![series])
            .unwrap();
        let _ = renderer.set_memory_budget(Some(
            renderer.gpu_memory_usage().total_bytes() + 2 * 1024 * 1024,
        ));
        renderer.set_auto_resident_working_set_limit(Some(1024 * 1024));
        (renderer, chart, config, values)
    }

    fn supply(
        renderer: &mut Renderer,
        token: StreamingResidencyOperation,
        ranges: &[crate::AutoStreamRange],
        values: &[crate::Column<f64>; 2],
    ) {
        assert_eq!(ranges.len(), 1);
        let range = &ranges[0];
        assert!(range.len <= 3);
        let index = usize::from(range.id == "y");
        let part = column(
            values[index].data[range.offset as usize..(range.offset + range.len) as usize].to_vec(),
        );
        renderer
            .submit_stream_residency_ranges(
                token,
                &[crate::StreamRangeSourceBinding {
                    id: &range.id,
                    revision: range.revision,
                    source_len: range.source_len,
                    offset: range.offset,
                    source: if index == 0 {
                        crate::StreamColumnSource::Scalar(&part)
                    } else {
                        crate::StreamColumnSource::HiLo(&part)
                    },
                }],
            )
            .unwrap();
    }

    fn fill(
        renderer: &mut Renderer,
        token: StreamingResidencyOperation,
        values: &[crate::Column<f64>; 2],
    ) {
        loop {
            match renderer.request_stream_residency_ranges(token).unwrap() {
                crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                    supply(renderer, token, &ranges, values)
                }
                crate::AutoStreamingRangeRequest::Backpressure { .. }
                | crate::AutoStreamingRangeRequest::AllSubmitted { .. } => renderer.wait_idle(),
                crate::AutoStreamingRangeRequest::Complete { .. } => break,
            }
        }
    }

    #[test]
    fn streamed_range_residency_promotes_the_transitive_shared_chart_closure() {
        let (mut r, chart, config, values) = setup();
        let mut z = r.streaming_sources["x"].column.clone();
        z.id = "z".into();
        r.register_streamed_columns(vec![z]).unwrap();
        let mut linked = r.chart_series(chart).unwrap()[0].clone();
        linked.series_id = "linked".into();
        linked.x_column = "y".into();
        linked.y_column = "z".into();
        let other = r.register_chart(config.clone(), vec![linked]).unwrap();
        let token = r
            .begin_stream_residency(chart, &config, 3)
            .unwrap()
            .0
            .unwrap();
        fill(&mut r, token, &values);
        r.finish_stream_residency(token).unwrap();
        assert!(r.chart_config(other).is_ok());
        for id in ["x", "y", "z"] {
            assert!(r.pool.handle_for(id).is_some());
            assert!(!r.streaming_sources.contains_key(id));
        }
    }

    #[test]
    fn streamed_range_residency_rejects_new_chart_links_before_commit() {
        for existing in [false, true] {
            let (mut r, chart, config, values) = setup();
            let mut z = r.streaming_sources["x"].column.clone();
            z.id = "z".into();
            r.register_streamed_columns(vec![z]).unwrap();
            let mut linked = r.chart_series(chart).unwrap()[0].clone();
            linked.series_id = "linked".into();
            linked.x_column = "z".into();
            linked.y_column = "z".into();
            let other = existing.then(|| {
                r.register_chart(config.clone(), vec![linked.clone()])
                    .unwrap()
            });
            let token = r
                .begin_stream_residency(chart, &config, 3)
                .unwrap()
                .0
                .unwrap();
            fill(&mut r, token, &values);
            linked.x_column = "x".into();
            if let Some(other) = other {
                r.set_chart_series(other, vec![linked]).unwrap();
            } else {
                r.register_chart(config.clone(), vec![linked]).unwrap();
            }
            assert!(r.finish_stream_residency(token).is_err());
            for id in ["x", "y", "z"] {
                assert!(r.pool.handle_for(id).is_none());
                assert!(r.streaming_sources.contains_key(id));
            }
        }
    }

    #[test]
    fn streamed_range_residency_defers_active_peers_and_retires_completed_peer_displays() {
        let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
            .lock()
            .unwrap();
        let (mut r, peer, config, values) = setup();
        let requester = r
            .register_chart(config.clone(), r.chart_series(peer).unwrap().to_vec())
            .unwrap();
        let view = r
            .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
            .unwrap();
        let _ = r.set_memory_budget(Some(r.gpu_memory_usage().total_bytes() + 8 * 1024 * 1024));
        r.request_auto_streaming_chart(
            peer,
            &view,
            crate::StreamingChartOptions {
                size: (320, 240),
                clear_color: Color::WHITE,
                max_primitives_per_chunk: 3,
            },
        )
        .unwrap();
        assert_eq!(
            r.begin_stream_residency(requester, &config, 3).unwrap(),
            (None, None)
        );
        let bindings = [
            crate::StreamSourceBinding {
                id: "x",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&values[0]),
            },
            crate::StreamSourceBinding {
                id: "y",
                revision: 1,
                source: crate::StreamColumnSource::HiLo(&values[1]),
            },
        ];
        loop {
            match r.auto_stream_chart_step(peer, &bindings).unwrap() {
                crate::AutoStreamingProgress::Complete { .. }
                | crate::AutoStreamingProgress::AllSubmitted { .. } => break,
                crate::AutoStreamingProgress::Backpressure { .. } => r.wait_idle(),
                _ => {}
            }
        }
        r.wait_idle();
        drop(
            r.prepare_registered(&[RegisteredChartDrawItem {
                chart_id: peer,
                view: &view,
            }])
            .unwrap(),
        );
        r.auto_stream_chart_step(peer, &bindings).unwrap();
        let peer_job = r.active_stream_job(peer).unwrap();
        let token = r
            .begin_stream_residency(requester, &config, 3)
            .unwrap()
            .0
            .unwrap();
        fill(&mut r, token, &values);
        assert_eq!(r.active_stream_job(peer), Some(peer_job));
        r.finish_stream_residency(token).unwrap();
        assert!(r.active_stream_job(peer).is_none());
        assert!(r.active_stream_job(requester).is_none());
        assert!(r.pool.handle_for("x").is_some());
    }

    #[test]
    fn streamed_range_residency_finish_preserves_a_peer_started_after_candidate() {
        let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
            .lock()
            .unwrap();
        let (mut r, chart, config, values) = setup();
        let peer = r
            .register_chart(config.clone(), r.chart_series(chart).unwrap().to_vec())
            .unwrap();
        let view = r
            .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
            .unwrap();
        let _ = r.set_memory_budget(Some(r.gpu_memory_usage().total_bytes() + 8 * 1024 * 1024));
        let token = r
            .begin_stream_residency(chart, &config, 3)
            .unwrap()
            .0
            .unwrap();
        r.request_auto_streaming_chart(
            peer,
            &view,
            crate::StreamingChartOptions {
                size: (320, 240),
                clear_color: Color::WHITE,
                max_primitives_per_chunk: 3,
            },
        )
        .unwrap();
        let peer_job = r.active_stream_job(peer).unwrap();
        fill(&mut r, token, &values);
        assert!(r.finish_stream_residency(token).is_err());
        assert_eq!(r.active_stream_job(peer), Some(peer_job));
        assert!(r.pool.handle_for("x").is_none());
        assert!(r.pool.handle_for("y").is_none());
        r.cancel_stream_residency(token).unwrap();
        assert_eq!(r.active_stream_job(peer), Some(peer_job));
    }

    fn pool_words(renderer: &Renderer, id: &str) -> Vec<f32> {
        let handle = renderer.pool.handle_for(id).unwrap();
        let bytes = handle.len_values as u64 * 8;
        let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("resident range fixture readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = renderer.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(renderer.pool.buffer(), handle.offset, &readback, 0, bytes);
        renderer.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        renderer.wait_idle();
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let result = mapped
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        drop(mapped);
        readback.unmap();
        result
    }

    fn handle(renderer: &Renderer, id: &str) -> Option<(u32, u64, u64, usize)> {
        renderer
            .pool
            .handle_for(id)
            .map(|h| (h.generation, h.offset, h.byte_size, h.len_values))
    }

    #[test]
    fn streamed_range_residency_commits_exact_pairs_atomically_and_keeps_survivors() {
        let (mut r, chart, config, values) = setup();
        let original_survivor = pool_words(&r, "survivor");
        let original = handle(&r, "survivor");
        let (token, report) = r.begin_stream_residency(chart, &config, 3).unwrap();
        let token = token.unwrap();
        let report = report.unwrap();
        assert!(report.is_admissible());
        assert_eq!(report.pool_transition_bytes, 4096);
        assert_eq!(report.upload_staging_bytes, 256);
        assert!(r.finish_stream_residency(token).is_err());
        assert_eq!(handle(&r, "survivor"), original);
        assert!(r.pool.handle_for("x").is_none());
        assert!(r.begin_stream_residency(chart, &config, 3).is_err());
        let first = r.request_stream_residency_ranges(token).unwrap();
        assert_eq!(first, r.request_stream_residency_ranges(token).unwrap());
        fill(&mut r, token, &values);
        assert!(r.pool.handle_for("x").is_none());
        assert!(r.streaming_sources.contains_key("x"));
        r.finish_stream_residency(token).unwrap();
        assert!(!r.streaming_sources.contains_key("x"));
        assert!(!r.streaming_sources.contains_key("y"));
        assert_eq!(pool_words(&r, "survivor"), original_survivor);
        for (index, id) in ["x", "y"].into_iter().enumerate() {
            let expected: Vec<f32> = values[index]
                .data
                .iter()
                .flat_map(|value| {
                    let high = *value as f32;
                    [
                        high,
                        if index == 0 {
                            0.0
                        } else {
                            (*value - f64::from(high)) as f32
                        },
                    ]
                })
                .collect();
            assert_eq!(pool_words(&r, id), expected);
        }
        assert!(r.request_stream_residency_ranges(token).is_err());
        r.end_gpu_frame();
        r.wait_idle();
        r.service_gpu_completions().unwrap();
        assert_eq!(
            r.gpu_memory_usage()
                .bytes_of(GpuResourceKind::StreamingUpload),
            0
        );
    }

    #[test]
    fn streamed_range_residency_denial_cancel_and_stale_source_preserve_authority() {
        let (mut r, chart, config, _) = setup();
        let old = handle(&r, "survivor");
        let usage = r.gpu_memory_usage().total_bytes();
        let creations = r.gpu_memory_usage().total_creations();
        let _ = r.set_memory_budget(Some(usage + 4095));
        let (token, report) = r.begin_stream_residency(chart, &config, 3).unwrap();
        assert!(token.is_none());
        assert_eq!(
            report.unwrap().status,
            ResidentAdmissionStatus::MemoryBudgetExceeded
        );
        assert_eq!(r.gpu_memory_usage().total_creations(), creations);
        let _ = r.set_memory_budget(Some(usage + 2 * 1024 * 1024));
        let token = r
            .begin_stream_residency(chart, &config, 3)
            .unwrap()
            .0
            .unwrap();
        r.request_stream_residency_ranges(token).unwrap();
        r.cancel_stream_residency(token).unwrap();
        assert_eq!(handle(&r, "survivor"), old);
        assert!(r.streaming_sources.contains_key("x"));
        assert!(r.request_stream_residency_ranges(token).is_err());
        r.wait_idle();
        r.service_gpu_completions().unwrap();
        let token = r
            .begin_stream_residency(chart, &config, 3)
            .unwrap()
            .0
            .unwrap();
        r.request_stream_residency_ranges(token).unwrap();
        let mut replacement = r.streaming_sources["x"].column.clone();
        replacement.revision = 2;
        r.replace_streamed_columns(vec![replacement]).unwrap();
        assert!(r.request_stream_residency_ranges(token).is_err());
        assert_eq!(handle(&r, "survivor"), old);
        assert_eq!(r.streaming_sources["x"].revision, 2);
        r.end_gpu_frame();
        r.wait_idle();
        r.service_gpu_completions().unwrap();
        assert_eq!(
            r.gpu_memory_usage()
                .bytes_of(GpuResourceKind::StreamingUpload),
            0
        );
    }

    #[test]
    fn streamed_range_residency_pool_mutation_and_chart_cancel_are_stale() {
        let (mut r, chart, config, _) = setup();
        let token = r
            .begin_stream_residency(chart, &config, 3)
            .unwrap()
            .0
            .unwrap();
        r.add_column("new-resident", &column(vec![7.0])).unwrap();
        assert!(r.request_stream_residency_ranges(token).is_err());
        assert!(r.pool.handle_for("new-resident").is_some());
        let token = r
            .begin_stream_residency(chart, &config, 3)
            .unwrap()
            .0
            .unwrap();
        r.cancel_streaming_chart(chart).unwrap();
        assert!(r.request_stream_residency_ranges(token).is_err());
        assert!(r.streaming_sources.contains_key("x"));
    }

    #[test]
    fn streamed_range_residency_candidates_are_chart_keyed_and_share_backpressure() {
        let (mut r, chart_a, config, values) = setup();
        let mut other_sources = vec![
            r.streaming_sources["x"].column.clone(),
            r.streaming_sources["y"].column.clone(),
        ];
        other_sources[0].id = "bx".into();
        other_sources[1].id = "by".into();
        r.register_streamed_columns(other_sources).unwrap();
        let mut other_series = r.chart_states[&chart_a].series.clone();
        other_series[0].x_column = "bx".into();
        other_series[0].y_column = "by".into();
        let chart_b = r.register_chart(config.clone(), other_series).unwrap();
        let a = r
            .begin_stream_residency(chart_a, &config, 3)
            .unwrap()
            .0
            .unwrap();
        let b = r
            .begin_stream_residency(chart_b, &config, 3)
            .unwrap()
            .0
            .unwrap();
        assert_eq!(r.stream_runtime.as_ref().unwrap().residencies.len(), 2);
        let crate::AutoStreamingRangeRequest::Ready { .. } =
            r.request_stream_residency_ranges(a).unwrap()
        else {
            panic!("A reserved");
        };
        let crate::AutoStreamingRangeRequest::Ready { ranges, .. } =
            r.request_stream_residency_ranges(b).unwrap()
        else {
            panic!("B reserved");
        };
        assert_eq!(r.stream_request_usage().1, 2);
        assert_eq!(ranges[0].id, "bx");
        r.cancel_stream_residency(a).unwrap();
        assert!(matches!(
            r.request_stream_residency_ranges(b).unwrap(),
            crate::AutoStreamingRangeRequest::Ready { .. }
        ));
        assert_eq!(r.stream_request_usage().1, 1);
        r.cancel_stream_residency(b).unwrap();
        r.wait_idle();
        r.service_gpu_completions().unwrap();
        let a = r
            .begin_stream_residency(chart_a, &config, 3)
            .unwrap()
            .0
            .unwrap();
        let b = r
            .begin_stream_residency(chart_b, &config, 3)
            .unwrap()
            .0
            .unwrap();
        fill(&mut r, a, &values);
        r.finish_stream_residency(a).unwrap();
        assert!(
            r.request_stream_residency_ranges(b).is_err(),
            "a pool swap invalidates older candidate survivors"
        );
        assert!(r.streaming_sources.contains_key("bx"));
        assert!(r.pool.handle_for("x").is_some());
    }

    #[test]
    fn streamed_range_residency_keeps_completed_display_until_commit_and_defers_running_draw() {
        let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
            .lock()
            .unwrap();
        let (mut r, chart, config, values) = setup();
        let view = r
            .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
            .unwrap();
        let _ = r.set_memory_budget(Some(r.gpu_memory_usage().total_bytes() + 8 * 1024 * 1024));
        r.request_auto_streaming_chart_with_config(
            chart,
            &view,
            config.clone(),
            crate::StreamingChartOptions {
                size: (320, 240),
                clear_color: Color::WHITE,
                max_primitives_per_chunk: 3,
            },
        )
        .unwrap();
        let bindings = [
            crate::StreamSourceBinding {
                id: "x",
                revision: 1,
                source: crate::StreamColumnSource::Scalar(&values[0]),
            },
            crate::StreamSourceBinding {
                id: "y",
                revision: 1,
                source: crate::StreamColumnSource::HiLo(&values[1]),
            },
        ];
        assert_eq!(
            r.begin_stream_residency(chart, &config, 3).unwrap(),
            (None, None)
        );
        loop {
            match r.auto_stream_chart_step(chart, &bindings).unwrap() {
                crate::AutoStreamingProgress::AllSubmitted { .. }
                | crate::AutoStreamingProgress::Complete { .. } => break,
                crate::AutoStreamingProgress::Backpressure { .. } => r.wait_idle(),
                _ => {}
            }
        }
        r.wait_idle();
        r.service_stream_requests();
        drop(
            r.prepare_registered(&[RegisteredChartDrawItem {
                chart_id: chart,
                view: &view,
            }])
            .unwrap(),
        );
        r.auto_stream_chart_step(chart, &bindings).unwrap();
        let job = r.active_stream_job(chart).unwrap();
        let target = r.chart_stream_prefix_for_test(job).unwrap();
        let token = r
            .begin_stream_residency(chart, &config, 3)
            .unwrap()
            .0
            .unwrap();
        assert_eq!(r.active_stream_job(chart), Some(job));
        assert_eq!(r.chart_stream_prefix_for_test(job).unwrap(), target);
        r.cancel_stream_residency(token).unwrap();
        assert_eq!(r.active_stream_job(chart), Some(job));
        assert_eq!(r.chart_stream_prefix_for_test(job).unwrap(), target);
        r.wait_idle();
        r.service_gpu_completions().unwrap();
        let token = r
            .begin_stream_residency(chart, &config, 3)
            .unwrap()
            .0
            .unwrap();
        fill(&mut r, token, &values);
        assert_eq!(r.active_stream_job(chart), Some(job));
        assert_eq!(r.chart_stream_prefix_for_test(job).unwrap(), target);
        r.finish_stream_residency(token).unwrap();
        assert!(r.active_stream_job(chart).is_none());
        assert!(r.pool.handle_for("x").is_some());
    }

    #[test]
    fn streamed_range_residency_retries_submission_failure_without_advancing_or_publishing() {
        let (mut r, chart, config, values) = setup();
        let token = r
            .begin_stream_residency(chart, &config, 3)
            .unwrap()
            .0
            .unwrap();
        let crate::AutoStreamingRangeRequest::Ready { ranges, .. } =
            r.request_stream_residency_ranges(token).unwrap()
        else {
            panic!("candidate request");
        };
        r.set_stream_residency_chunk_budget(token, 1).unwrap();
        assert_eq!(
            r.request_stream_residency_ranges(token).unwrap(),
            crate::AutoStreamingRangeRequest::Ready {
                revision: r.chart_states[&chart].revisions.desired,
                submitted_primitives: 0,
                total_primitives: 26,
                ranges: ranges.clone(),
            }
        );
        let range = &ranges[0];
        let part = column(values[0].data[..range.len as usize].to_vec());
        let binding = crate::StreamRangeSourceBinding {
            id: &range.id,
            revision: 1,
            source_len: 13,
            offset: 0,
            source: crate::StreamColumnSource::Scalar(&part),
        };
        r.reject_next_stream_completion_reserve_for_test();
        assert!(r.submit_stream_residency_ranges(token, &[binding]).is_err());
        let crate::AutoStreamingRangeRequest::Ready {
            submitted_primitives,
            ranges,
            ..
        } = r.request_stream_residency_ranges(token).unwrap()
        else {
            panic!("retry request");
        };
        assert_eq!(submitted_primitives, 0);
        assert_eq!((ranges[0].offset, ranges[0].len), (0, 1));
        assert!(r.pool.handle_for("x").is_none());
        fill(&mut r, token, &values);
        r.finish_stream_residency(token).unwrap();
        assert!(r.pool.handle_for("x").is_some());
    }
}
