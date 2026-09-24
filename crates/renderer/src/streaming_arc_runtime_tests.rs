use super::*;

fn column(data: Vec<f64>) -> crate::Column<f64> {
    crate::Column {
        min: data.iter().copied().fold(f64::INFINITY, f64::min),
        max: data.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        data,
    }
}

fn source(column: &crate::Column<f64>, hilo: bool) -> crate::StreamColumnSource<'_> {
    if hilo {
        crate::StreamColumnSource::HiLo(column)
    } else {
        crate::StreamColumnSource::Scalar(column)
    }
}

fn parity(
    n: usize,
    cap: Option<u32>,
    io: u64,
    style: DrawStyle,
    line_style: LineStylePreset,
    hilo: bool,
) {
    parity_with_samples(n, cap, io, style, line_style, hilo, 4);
}

fn parity_with_samples(
    n: usize,
    cap: Option<u32>,
    io: u64,
    style: DrawStyle,
    line_style: LineStylePreset,
    hilo: bool,
    samples: u32,
) {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let (device, queue) = data_render::shared_device().unwrap();
    let make = || {
        Renderer::try_new_with_sample_count(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Rgba8Unorm,
            (n as u64 * 32).max(4096),
            samples,
        )
        .unwrap()
    };
    let mut streamed = make();
    let mut resident = make();
    streamed.arc_chunk_override = cap;
    resident.arc_chunk_override = cap;
    let base = if hilo { 1e12 } else { 0.0 };
    let x = column((0..n).map(|i| base + i as f64 / (n - 1) as f64).collect());
    let y = column(
        (0..n)
            .map(|i| {
                if i == 255 {
                    f64::NAN
                } else if hilo && i == 256 {
                    -0.5
                } else {
                    0.5 + (i as f64 * 0.031).sin() * 0.37
                }
            })
            .collect(),
    );
    if hilo {
        resident.add_hilo_column("x", &x).unwrap();
        resident.add_hilo_column("y", &y).unwrap();
    } else {
        resident.add_column("x", &x).unwrap();
        resident.add_column("y", &y).unwrap();
    }
    streamed
        .register_streamed_columns(
            [("x", &x), ("y", &y)]
                .into_iter()
                .map(|(id, column)| crate::StreamColumn {
                    id: id.into(),
                    len: column.data.len() as u64,
                    revision: 1,
                    encoding: if hilo {
                        crate::StreamEncoding::HiLoF32
                    } else {
                        crate::StreamEncoding::ScalarF32
                    },
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
    config.draw_style = style;
    if hilo {
        config.left_y.scale = AxisScale::Logarithmic;
        config.right_y.scale = AxisScale::Logarithmic;
        config.bottom_x.inverted = true;
        config.top_x.inverted = true;
        config.left_y.inverted = true;
        config.right_y.inverted = true;
    }
    let mut chart = Chart::new(config);
    chart.set_x_range(base, base + 1.0);
    chart.set_y_range(if hilo { 0.1 } else { 0.0 }, 1.0);
    let line = DataLineStyleConfig {
        line_width: 2.5,
        line_color: Color::new(0.2, 0.6, 0.8, 0.45),
        line_style,
    };
    let render_type = if matches!(chart.config().draw_style, DrawStyle::Constellation(_)) {
        DataRenderType::ScatterLine {
            scatter: crate::data_config::DataScatterStyleConfig {
                point_size: 4.0,
                point_color: Color::new(0.8, 0.3, 0.2, 0.65),
                point_shape: crate::data_config::ScatterShape::CircleFilled,
                point_style_index_column: None,
                point_style_table: None,
                point_style_overrides: None,
            },
            line,
        }
    } else {
        DataRenderType::Line { line }
    };
    let series = vec![SeriesConfig {
        series_id: "line".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type,
    }];
    let id = streamed
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    streamed
        .configure_streaming(crate::StreamingLimits {
            max_active_charts: 1,
            max_in_flight_chunks: 2,
            max_columns_per_chunk: 2,
            max_chunk_input_bytes: io * if hilo { 16 } else { 8 },
            max_in_flight_gpu_bytes: io * 64,
        })
        .unwrap();
    let view = streamed
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    streamed
        .request_auto_streaming_chart_with_config(
            id,
            &view,
            chart.config().clone(),
            crate::StreamingChartOptions {
                size: (240, 180),
                clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                max_primitives_per_chunk: io,
            },
        )
        .unwrap();
    let bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision: 1,
            source: source(&x, hilo),
        },
        crate::StreamSourceBinding {
            id: "y",
            revision: 1,
            source: source(&y, hilo),
        },
    ];
    if io == 1 {
        streamed.reject_next_stream_completion_reserve_for_test();
        assert!(streamed.auto_stream_chart_step(id, &bindings).is_err());
        let job = streamed.active_stream_job(id).unwrap();
        let arc = streamed
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap()
            .arc
            .as_ref()
            .unwrap();
        assert_eq!((arc.filled, arc.block, arc.drawing), (0, 0, false));
        streamed.auto_stream_chart_step(id, &bindings).unwrap();
        let StreamDrawRequestStatus::Ready(stale) =
            streamed.request_chart_stream_draw(job).unwrap()
        else {
            panic!("second bounded leaf ticket")
        };
        let target = streamed.chart_stream_prefix_for_test(job).unwrap();
        streamed.cancel_chart_stream(id).unwrap();
        assert!(
            streamed
                .submit_chart_stream_draw_supply(
                    stale,
                    StreamSupply::Encoded(&[]),
                    Some(&view),
                    &target
                )
                .is_err()
        );
        assert_eq!(
            streamed
                .gpu_memory_usage()
                .live_bytes_of(GpuResourceKind::ArcScan),
            0
        );
        streamed.end_gpu_frame();
        streamed.wait_idle();
        streamed.service_stream_requests();
        assert_eq!(
            streamed
                .gpu_memory_usage()
                .bytes_of(GpuResourceKind::ArcScan),
            0
        );
        streamed
            .request_auto_streaming_chart_with_config(
                id,
                &view,
                chart.config().clone(),
                crate::StreamingChartOptions {
                    size: (240, 180),
                    clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                    max_primitives_per_chunk: io,
                },
            )
            .unwrap();
    }
    loop {
        match streamed.auto_stream_chart_step(id, &bindings).unwrap() {
            crate::AutoStreamingProgress::AllSubmitted { .. }
            | crate::AutoStreamingProgress::Complete { .. } => break,
            crate::AutoStreamingProgress::Backpressure { .. } => streamed.wait_idle(),
            _ => {}
        }
    }
    streamed.wait_idle();
    drop(
        streamed
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: id,
                view: &view,
            }])
            .unwrap(),
    );
    streamed.auto_stream_chart_step(id, &bindings).unwrap();
    for scale in [1.0, 2.0] {
        let operation = streamed
            .begin_stream_export(id, scale, Color::new(0.0, 0.0, 0.0, 0.0), io)
            .unwrap();
        loop {
            match streamed.request_stream_operation_ranges(operation).unwrap() {
                crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                    assert!(ranges.iter().all(|range| range.len <= io));
                    let data: Vec<_> = ranges
                        .iter()
                        .map(|range| {
                            let values = if range.id == "x" { &x } else { &y };
                            column(
                                values.data
                                    [range.offset as usize..(range.offset + range.len) as usize]
                                    .to_vec(),
                            )
                        })
                        .collect();
                    let supplied: Vec<_> = ranges
                        .iter()
                        .zip(&data)
                        .map(|(range, data)| crate::StreamRangeSourceBinding {
                            id: &range.id,
                            revision: range.revision,
                            source_len: range.source_len,
                            offset: range.offset,
                            source: source(data, hilo),
                        })
                        .collect();
                    streamed
                        .submit_stream_operation_ranges(operation, &supplied)
                        .unwrap();
                }
                crate::AutoStreamingRangeRequest::Backpressure { .. } => streamed.wait_idle(),
                crate::AutoStreamingRangeRequest::AllSubmitted { .. }
                | crate::AutoStreamingRangeRequest::Complete { .. } => break,
            }
        }
        let actual = pollster::block_on(streamed.finish_stream_export(operation)).unwrap();
        let expected = resident.export_panel_rgba(&chart, &series, scale).unwrap();
        let different = actual
            .rgba
            .iter()
            .zip(&expected.rgba)
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            different,
            0,
            "n={n} cap={cap:?} io={io} scale={scale} style={:?}",
            chart.config().draw_style
        );
    }
}

#[test]
fn stream_arc_exact_dash_and_sketch_with_one_point_io() {
    parity(
        257,
        None,
        1,
        DrawStyle::Precise,
        LineStylePreset::Dash,
        false,
    );
    parity(
        514,
        Some(257),
        2,
        DrawStyle::Sketch(Default::default()),
        LineStylePreset::Solid,
        false,
    );
}

#[test]
fn stream_arc_exact_carry_and_upper_tree() {
    parity(
        514,
        Some(256),
        17,
        DrawStyle::Precise,
        LineStylePreset::Dot,
        false,
    );
    parity(
        65_539,
        None,
        257,
        DrawStyle::Precise,
        LineStylePreset::Dash,
        false,
    );
}

#[test]
fn stream_arc_exact_hilo_log_inverted_nan_and_sketch_dash() {
    parity(
        514,
        Some(257),
        3,
        DrawStyle::Sketch(Default::default()),
        LineStylePreset::Dash,
        true,
    );
}

#[test]
fn stream_arc_exact_constellation_dash_and_dot() {
    for samples in [1, 4] {
        for (io, line_style, hilo) in [
            (1, LineStylePreset::Dash, false),
            (17, LineStylePreset::Dot, true),
        ] {
            parity_with_samples(
                514,
                Some(257),
                io,
                DrawStyle::Constellation(crate::config::ConstellationOptions {
                    star_opacity: 0.71,
                    line_opacity: 0.37,
                }),
                line_style,
                hilo,
                samples,
            );
        }
    }
}

#[test]
fn stream_arc_scratch_is_bounded_and_budget_rejection_is_preallocation() {
    let (device, queue) = data_render::shared_device().unwrap();
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    let config = crate::default::default_config();
    let before = renderer.gpu_memory_usage().total_bytes();
    renderer.memory_budget = Some(before);
    assert!(matches!(
        renderer.new_stream_arc(&config, u64::from(u32::MAX)),
        Err(StreamRequestError::Scheduler(StreamError::TooLarge))
    ));
    assert_eq!(renderer.gpu_memory_usage().total_bytes(), before);
    renderer.memory_budget = None;
    let state = renderer
        .new_stream_arc(&config, u64::from(u32::MAX))
        .unwrap();
    assert!(state.buffers.charge.charged_bytes() < 270_000);
    assert_eq!(
        renderer.gpu_memory_usage().total_bytes() - before,
        state.buffers.charge.charged_bytes()
    );
    drop(state);
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .live_bytes_of(GpuResourceKind::ArcScan),
        0
    );
    assert!(
        renderer
            .gpu_memory_usage()
            .retired_bytes_of(GpuResourceKind::ArcScan)
            > 0
    );
    renderer.end_gpu_frame();
    renderer.wait_idle();
    renderer.service_gpu_completions().unwrap();
    assert_eq!(
        renderer
            .gpu_memory_usage()
            .bytes_of(GpuResourceKind::ArcScan),
        0
    );
}
