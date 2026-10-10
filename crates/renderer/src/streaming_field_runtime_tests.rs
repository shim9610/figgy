use super::*;

#[path = "streaming_field_selection_tests.rs"]
mod selection_tests;

#[path = "streaming_field_pick_tests.rs"]
mod pick_tests;

#[path = "streaming_field_edge_tests.rs"]
mod edge_tests;

#[path = "../tests/support/streaming_field_diagnostic_tests.rs"]
mod diagnostic_tests;

fn column(data: Vec<f64>) -> crate::Column<f64> {
    crate::Column {
        min: data.iter().copied().fold(f64::INFINITY, f64::min),
        max: data.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        data,
    }
}

struct Fixture {
    renderer: Renderer,
    chart: Chart,
    id: ChartId,
    view: ChartView,
    series: Vec<SeriesConfig>,
    columns: Vec<(&'static str, crate::Column<f64>)>,
}

fn setup(centers: bool, interpolated: bool, columns_are_y: bool, samples: u32, io: u64) -> Fixture {
    setup_columns(centers, interpolated, columns_are_y, samples, io, 1)
}

fn setup_columns(
    centers: bool,
    interpolated: bool,
    columns_are_y: bool,
    samples: u32,
    io: u64,
    max_columns: usize,
) -> Fixture {
    setup_columns_with_budget(centers, interpolated, columns_are_y, samples, io, max_columns, 4 * 1024 * 1024)
}

fn setup_columns_with_budget(
    centers: bool,
    interpolated: bool,
    columns_are_y: bool,
    samples: u32,
    io: u64,
    max_columns: usize,
    gpu_bytes: u64,
) -> Fixture {
    let (device, queue) = data_render::shared_device().unwrap();
    let mut renderer = Renderer::try_new_with_sample_count(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
        samples,
    )
    .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 360,
        height: 240,
    });
    let mut colorbar = crate::default::default_colorbar_options();
    colorbar.axis.min = 0.1;
    colorbar.axis.max = 4.0;
    colorbar.nan_color = Color::new(0.9, 0.1, 0.4, 0.45);
    config.colorbar = Some(colorbar);
    if columns_are_y {
        config.bottom_x.inverted = true;
        config.top_x.inverted = true;
    }
    let mut chart = Chart::new(config);
    chart.set_x_range(0.1, 4.0);
    chart.set_y_range(0.1, 4.0);
    let coordinate = if centers {
        vec![0.5, 1.5, 3.0]
    } else {
        vec![0.2, 0.9, 2.2, 3.8]
    };
    let columns = vec![
        ("x", column(coordinate.clone())),
        ("y", column(coordinate)),
        ("z0", column(vec![0.2, f64::NAN, 1.4, 9.0])),
        ("z1", column(vec![2.1, 3.8, 2.6])),
    ];
    renderer
        .register_streamed_columns(
            columns
                .iter()
                .map(|(id, values)| crate::StreamColumn {
                    id: (*id).into(),
                    len: values.data.len() as u64,
                    revision: 5,
                    encoding: crate::StreamEncoding::HiLoF32,
                    replay: crate::StreamReplay::RandomAccess,
                    statistics: crate::StreamStatistics::Unknown,
                })
                .collect(),
        )
        .unwrap();
    let series = vec![SeriesConfig {
        series_id: "heat".into(),
        source_id: Some("matrix".into()),
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Heatmap {
            matrix: crate::data_config::MatrixRef {
                columns: vec!["z0".into(), "z1".into(), "z0".into()],
                orientation: if columns_are_y {
                    crate::data_config::MatrixOrientation::ColumnsAreY
                } else {
                    crate::data_config::MatrixOrientation::ColumnsAreX
                },
                grid_layout: if centers {
                    crate::data_config::GridLayout::Centers
                } else {
                    crate::data_config::GridLayout::Edges
                },
            },
            fill: crate::data_config::FieldFillConfig {
                mode: crate::data_config::FillMode::Continuous,
                shading: if interpolated {
                    crate::data_config::Shading::Interpolated
                } else {
                    crate::data_config::Shading::Flat
                },
                opacity: 0.63,
            },
        },
    }];
    let id = renderer
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    renderer
        .configure_streaming(crate::StreamingLimits {
            max_active_charts: 1,
            max_in_flight_chunks: 2,
            max_columns_per_chunk: max_columns,
            max_chunk_input_bytes: io.max(2) * 8,
            max_in_flight_gpu_bytes: gpu_bytes,
        })
        .unwrap();
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    renderer
        .request_auto_streaming_chart(
            id,
            &view,
            crate::StreamingChartOptions {
                size: (360, 240),
                clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                max_primitives_per_chunk: io,
            },
        )
        .unwrap();
    Fixture {
        renderer,
        chart,
        id,
        view,
        series,
        columns,
    }
}

fn pump(f: &mut Fixture, operation: Option<StreamingOperation>) {
    pump_count(f, operation);
}

fn pump_count(f: &mut Fixture, operation: Option<StreamingOperation>) -> usize {
    let mut reads = 0;
    for _ in 0..10000 {
        let request = if let Some(operation) = operation {
            f.renderer
                .request_stream_operation_ranges(operation)
                .unwrap()
        } else {
            f.renderer.auto_stream_chart_request_ranges(f.id).unwrap()
        };
        match request {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                reads += 1;
                assert_eq!(ranges.len(), 1);
                let range = &ranges[0];
                let original = &f.columns.iter().find(|(id, _)| *id == range.id).unwrap().1;
                let payload = column(
                    original.data[range.offset as usize..(range.offset + range.len) as usize]
                        .to_vec(),
                );
                let bindings = [crate::StreamRangeSourceBinding {
                    id: &range.id,
                    revision: range.revision,
                    source_len: range.source_len,
                    offset: range.offset,
                    source: if range.encoding == crate::StreamEncoding::HiLoF32 { crate::StreamColumnSource::HiLo(&payload) } else { crate::StreamColumnSource::Scalar(&payload) },
                }];
                if let Some(operation) = operation {
                    f.renderer
                        .submit_stream_operation_ranges(operation, &bindings)
                        .unwrap();
                } else {
                    f.renderer
                        .auto_stream_chart_submit_ranges(f.id, &bindings)
                        .unwrap();
                }
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. } => f.renderer.wait_idle(),
            crate::AutoStreamingRangeRequest::AllSubmitted { .. }
            | crate::AutoStreamingRangeRequest::Complete { .. } => return reads,
        }
    }
    panic!("field replay did not terminate");
}

fn finish_display(f: &mut Fixture) {
    pump(f, None);
    f.renderer.wait_idle();
    drop(
        f.renderer
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: f.id,
                view: &f.view,
            }])
            .unwrap(),
    );
    assert!(matches!(
        f.renderer.auto_stream_chart_request_ranges(f.id).unwrap(),
        crate::AutoStreamingRangeRequest::Complete { .. }
    ));
}

#[test]
fn heatmap_runtime_exports_match_resident_lattice_and_duplicate_columns() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    for (centers, interpolated, cy, samples, io) in [
        (false, false, false, 1, 2),
        (true, false, true, 4, 3),
        (false, true, true, 4, 2),
        (true, true, false, 1, 3),
    ] {
        let mut f = setup(centers, interpolated, cy, samples, io);
        finish_display(&mut f);
        let job = f.renderer.active_stream_job(f.id);
        let (device, queue) = data_render::shared_device().unwrap();
        let mut resident = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        for (id, column) in &f.columns {
            resident.add_hilo_column(*id, column).unwrap();
        }
        for scale in [1.0, 2.0] {
            let operation = f
                .renderer
                .begin_stream_export(f.id, scale, Color::new(0.0, 0.0, 0.0, 0.0), io)
                .unwrap();
            pump(&mut f, Some(operation));
            let actual = pollster::block_on(f.renderer.finish_stream_export(operation)).unwrap();
            let expected = resident
                .export_panel_rgba(&f.chart, &f.series, scale)
                .unwrap();
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
            assert!(
                actual.rgba == expected.rgba,
                "centers={centers} interpolated={interpolated} cy={cy} samples={samples} io={io} scale={scale}; differing bytes={}",
                actual
                    .rgba
                    .iter()
                    .zip(&expected.rgba)
                    .filter(|(a, b)| a != b)
                    .count()
            );
            assert_eq!(f.renderer.active_stream_job(f.id), job);
        }
    }
}

#[test]
fn heatmap_runtime_cancel_stale_and_budget_are_transactional() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut f = setup(false, false, false, 1, 2);
    f.renderer.wait_idle();
    let before = f.renderer.gpu_memory_usage().total_bytes();
    let _ = f.renderer.set_memory_budget(Some(before + 1));
    assert!(f.renderer.auto_stream_chart_request_ranges(f.id).is_err());
    assert_eq!(f.renderer.gpu_memory_usage().total_bytes(), before);
    let _ = f.renderer.set_memory_budget(None);
    let crate::AutoStreamingRangeRequest::Ready { ranges, .. } =
        f.renderer.auto_stream_chart_request_ranges(f.id).unwrap()
    else {
        panic!("field source request");
    };
    let range = &ranges[0];
    let payload = column(vec![0.2; range.len as usize]);
    let input = [crate::StreamRangeSourceBinding {
        id: &range.id,
        revision: range.revision,
        source_len: range.source_len,
        offset: range.offset,
        source: crate::StreamColumnSource::HiLo(&payload),
    }];
    f.renderer.reject_next_stream_completion_reserve_for_test();
    assert!(
        f.renderer
            .auto_stream_chart_submit_ranges(f.id, &input)
            .is_err()
    );
    let retry = f.renderer.auto_stream_chart_request_ranges(f.id).unwrap();
    assert!(matches!(
        retry,
        crate::AutoStreamingRangeRequest::Ready { .. }
    ));
    f.renderer
        .auto_stream_chart_submit_ranges(f.id, &input)
        .unwrap();
    f.renderer.cancel_streaming_chart(f.id).unwrap();
    assert!(
        f.renderer
            .auto_stream_chart_submit_ranges(f.id, &input)
            .is_err()
    );
    f.renderer.end_gpu_frame();
    f.renderer.wait_idle();
    f.renderer.service_stream_requests();
    assert_eq!(f.renderer.streaming_usage().reserved_gpu_bytes, 0);
}

#[test]
fn heatmap_runtime_fit_uses_exact_gpu_lattice_and_cached_refit() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    for (centers, interpolated, cy) in [
        (false, false, false),
        (true, false, true),
        (false, true, true),
        (true, true, false),
    ] {
        let mut f = setup(centers, interpolated, cy, 1, 2);
        f.renderer.request_stream_auto_fit(f.id, 0.07).unwrap();
        f.renderer
            .request_auto_streaming_chart(
                f.id,
                &f.view,
                crate::StreamingChartOptions {
                    size: (360, 240),
                    clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                    max_primitives_per_chunk: 2,
                },
            )
            .unwrap();
        finish_display(&mut f);
        let (device, queue) = data_render::shared_device().unwrap();
        let mut resident = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        for (id, column) in &f.columns {
            resident.add_hilo_column(*id, column).unwrap();
        }
        pollster::block_on(resident.ensure_errorbar_extent_engine()).unwrap();
        let ticket = resident
            .begin_series_fit_extent(&f.series[0])
            .unwrap()
            .unwrap();
        let expected = pollster::block_on(ticket.resolve()).unwrap().unwrap();
        let mut expected_config = f.chart.config().clone();
        let x = crate::FitExtent {
            min: expected.x.min,
            max: expected.x.max,
            min_positive: expected.x.min_positive,
        };
        let y = crate::FitExtent {
            min: expected.y.min,
            max: expected.y.max,
            min_positive: expected.y.min_positive,
        };
        crate::chart::apply_auto_fit_all(&mut expected_config, &x, &y, 0.07);
        let config = f.renderer.chart_config(f.id).unwrap();
        assert_eq!(
            (
                config.bottom_x.min,
                config.bottom_x.max,
                config.left_y.min,
                config.left_y.max
            ),
            (
                expected_config.bottom_x.min,
                expected_config.bottom_x.max,
                expected_config.left_y.min,
                expected_config.left_y.max
            )
        );
        let job = f.renderer.active_stream_job(f.id).unwrap();
        let fields = &f
            .renderer
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap()
            .field_fits;
        assert_eq!(fields[&0], Some(expected));
        f.renderer.request_stream_auto_fit(f.id, 0.2).unwrap();
        assert_eq!(f.renderer.active_stream_job(f.id), Some(job));
        assert!(
            f.renderer.chart_states[&f.id]
                .stream_auto_fit_padding
                .is_none(),
            "cached field bounds refit without requesting source rows"
        );
        crate::chart::apply_auto_fit_all(&mut expected_config, &x, &y, 0.2);
        let config = f.renderer.chart_config(f.id).unwrap();
        assert_eq!(
            (
                config.bottom_x.min,
                config.bottom_x.max,
                config.left_y.min,
                config.left_y.max
            ),
            (
                expected_config.bottom_x.min,
                expected_config.bottom_x.max,
                expected_config.left_y.min,
                expected_config.left_y.max
            )
        );
    }
}

fn pump_pick(f: &mut Fixture, operation: StreamingOperation) -> usize {
    let mut reads = 0;
    for _ in 0..1000 {
        match f.renderer.request_stream_operation_ranges(operation).unwrap() {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                assert_eq!(ranges.len(), 1);
                let range = &ranges[0];
                assert!(range.id == "x" || range.id == "y", "field picking must not read Z");
                let original = &f.columns.iter().find(|(id, _)| *id == range.id).unwrap().1;
                let payload = column(original.data[range.offset as usize..(range.offset + range.len) as usize].to_vec());
                f.renderer.submit_stream_operation_ranges(operation, &[crate::StreamRangeSourceBinding {
                    id: &range.id, revision: range.revision, source_len: range.source_len, offset: range.offset,
                    source: if range.encoding == crate::StreamEncoding::HiLoF32 { crate::StreamColumnSource::HiLo(&payload) } else { crate::StreamColumnSource::Scalar(&payload) },
                }]).unwrap();
                reads += 1;
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. } => f.renderer.wait_idle(),
            crate::AutoStreamingRangeRequest::AllSubmitted { .. } | crate::AutoStreamingRangeRequest::Complete { .. } => return reads,
        }
    }
    panic!("field pick did not terminate");
}

#[test]
fn heatmap_runtime_typed_pick_matches_global_cells_without_z_replay() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK.lock().unwrap();
    for (case, centers, interpolated, cy, hilo) in [
        (0, false, false, false, false), (1, true, false, true, true),
        (2, false, true, true, true), (3, true, true, false, false),
        (4, false, false, false, true), (5, true, true, true, true),
    ] {
        let mut f = setup(centers, interpolated, cy, 1, 2);
        if case == 4 { f.columns[0].1.data[1] = 3.0; f.columns[0].1.data[2] = 1.0; }
        if case == 5 { f.columns[1].1.data[1] = f64::NAN; }
        if case == 1 {
            let mut config = f.chart.config().clone();
            config.bottom_x.scale = AxisScale::Logarithmic;
            config.top_x.scale = AxisScale::Logarithmic;
            f.chart = Chart::new(config.clone());
            f.renderer.set_chart_config(f.id, config).unwrap();
        }
        f.renderer.replace_streamed_columns(f.columns.iter().map(|(id, values)| crate::StreamColumn {
            id: (*id).into(), len: values.data.len() as u64, revision: 6,
            encoding: if hilo { crate::StreamEncoding::HiLoF32 } else { crate::StreamEncoding::ScalarF32 },
            replay: crate::StreamReplay::RandomAccess, statistics: crate::StreamStatistics::Unknown,
        }).collect()).unwrap();
        let mut overlay = f.series[0].clone();
        overlay.series_id = "painted-last".into();
        f.series.push(overlay);
        f.renderer.set_chart_series(f.id, f.series.clone()).unwrap();
        f.renderer.request_auto_streaming_chart(f.id, &f.view, crate::StreamingChartOptions {
            size: (360, 240), clear_color: Color::new(0.0, 0.0, 0.0, 0.0), max_primitives_per_chunk: 2,
        }).unwrap();
        finish_display(&mut f);
        let (device, queue) = data_render::shared_device().unwrap();
        let mut resident = Renderer::try_new(RendererDevice::new(device, queue), wgpu::TextureFormat::Rgba8Unorm, 4096).unwrap();
        for (id, column) in &f.columns {
            if hilo { resident.add_hilo_column(*id, column).unwrap(); } else { resident.add_column(*id, column).unwrap(); }
        }
        let chart_id = resident.register_chart(f.chart.config().clone(), f.series.clone()).unwrap();
        resident.enable_gpu_picking().unwrap();
        let data = f.chart.config().data_area().unwrap().0;
        for fraction in [[0.13, 0.17], [0.42, 0.68], [0.76, 0.33], [0.94, 0.92], [-1.0, -1.0]] {
            let position = [data.x as f32 + data.width as f32 * fraction[0], data.y as f32 + data.height as f32 * fraction[1]];
            let query = GpuPickRequest { canvas_position_px: position, display_panel_px: f.chart.config().chart_area.0, display_scale: 1.0, max_distance_px: 5.0 };
            let expected = pollster::block_on(resident.pick_chart_data(chart_id, query).unwrap().resolve()).unwrap();
            let operation = pollster::block_on(f.renderer.begin_stream_pick_data(f.id, position, 5.0, 2)).unwrap();
            pump_pick(&mut f, operation);
            let actual = pollster::block_on(f.renderer.finish_stream_pick_data(operation)).unwrap();
            assert_eq!(actual, expected, "case={case} fraction={fraction:?}");
            let legacy = pollster::block_on(f.renderer.begin_stream_pick_point(f.id, position, 5.0, 2)).unwrap();
            assert_eq!(pump_pick(&mut f, legacy), 0, "legacy point picker skips matrix inputs");
            assert!(pollster::block_on(f.renderer.finish_stream_pick_point(legacy)).unwrap().is_none());
        }
    }
}

#[test]
fn ssot_colorbar_edits_refresh_existing_heatmap_without_decoration_replay() {
    use crate::config::ColorBarOptions;
    use crate::renderer::streaming_request_tests::read_draw_target;
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK.lock().unwrap();
    // Third column specifies whether the DATA colours/geometry must change.
    let cases: &[(&str, fn(&mut ColorBarOptions), bool)] = &[
        ("colormap", |b| b.colormap = crate::colormap::ColorMap::Magma, true),
        ("z.min", |b| b.axis.min = 1.0, true),
        ("z.max", |b| b.axis.max = 9.0, true),
        ("z.scale", |b| b.axis.scale = AxisScale::Logarithmic, true),
        ("nan_color", |b| b.nan_color = Color::new(0.0, 1.0, 0.0, 1.0), true),
        ("visible", |b| b.visible = false, true),
        ("thickness", |b| b.thickness_px += 4.0, true),
        ("offset_x", |b| b.offset_x += 4.0, false),
        ("offset_y", |b| b.offset_y += 4.0, false),
        ("border_color", |b| b.border_color = Color::new(0.0, 1.0, 0.0, 1.0), false),
        ("z.label_color", |b| b.axis.label_style.color = Color::new(0.0, 0.7, 0.0, 1.0), false),
        ("z.inverted", |b| b.axis.inverted = true, false),
    ];
    let screen = |f: &mut Fixture| {
        let job = f.renderer.active_stream_job(f.id).unwrap();
        let target = wgpu::Texture::clone(f.renderer.chart_stream_display(job).unwrap());
        read_draw_target(&f.renderer, &target)
    };
    let update = |f: &mut Fixture, edit: fn(&mut ColorBarOptions)| {
        let mut config = f.chart.config().clone(); edit(config.colorbar.as_mut().unwrap());
        f.renderer.set_chart_config(f.id,config.clone()).unwrap();
        f.chart = Chart::new(config);
        f.view = f.renderer.create_chart_view(&f.chart,f.chart.config().chart_area.0).unwrap();
        f.renderer.request_auto_streaming_chart(f.id,&f.view,crate::StreamingChartOptions {
            size:(360,240), clear_color:Color::new(0.0,0.0,0.0,0.0),max_primitives_per_chunk:2,
        }).unwrap();
    };
    let mut failures = Vec::new();
    for &(name,edit,replay) in cases {
        let mut f = setup(false,false,false,1,2); finish_display(&mut f);
        let before = screen(&mut f);
        let job = f.renderer.active_stream_job(f.id).unwrap();
        update(&mut f,edit);
        let requested = matches!(f.renderer.auto_stream_chart_request_ranges(f.id).unwrap(),crate::AutoStreamingRangeRequest::Ready { .. });
        if requested != replay { failures.push(format!("{name}: source replay={requested}, expected={replay}")); }
        finish_display(&mut f);
        let actual = screen(&mut f);
        let mut fresh = setup(false,false,false,1,2);
        fresh.renderer.cancel_chart_stream(fresh.id).unwrap();
        fresh.renderer.service_stream_requests();
        update(&mut fresh,edit); finish_display(&mut fresh);
        let expected = screen(&mut fresh);
        if actual != expected {
            failures.push(format!("{name}: stale first-frame pixels"));
        }
        if actual == before {
            failures.push(format!("{name}: no visible change"));
        }
        if !replay && f.renderer.active_stream_job(f.id) != Some(job) {
            failures.push(format!("{name}: decoration restarted data job"));
        }
    }
    drop(font);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn bounded_source_cache_heatmap_edits_recompute_pixels_without_source_reads() {
    use crate::renderer::streaming_request_tests::read_draw_target;
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let edits: &[(&str, fn(&mut Config))] = &[
        ("size", |c| c.chart_area.0.width = 320),
        ("range", |c| c.left_y.max = 2.0),
        ("grid", |c| {
            c.grid.major_x_color = Color::new(0.0, 0.9, 0.0, 1.0)
        }),
        ("colormap", |c| {
            c.colorbar.as_mut().unwrap().colormap = crate::colormap::ColorMap::Magma
        }),
        ("z.min", |c| c.colorbar.as_mut().unwrap().axis.min = 1.0),
        ("z.max", |c| c.colorbar.as_mut().unwrap().axis.max = 9.0),
        ("z.scale", |c| {
            c.colorbar.as_mut().unwrap().axis.scale = AxisScale::Logarithmic
        }),
        ("nan_color", |c| {
            c.colorbar.as_mut().unwrap().nan_color = Color::new(0.0, 1.0, 0.0, 1.0)
        }),
    ];
    let screen = |f: &mut Fixture| {
        let job = f.renderer.active_stream_job(f.id).unwrap();
        let target = wgpu::Texture::clone(f.renderer.chart_stream_display(job).unwrap());
        read_draw_target(&f.renderer, &target)
    };
    let start = |f: &mut Fixture, c: Config| {
        f.renderer.set_chart_config(f.id, c.clone()).unwrap();
        f.chart = Chart::new(c);
        f.view = f
            .renderer
            .create_chart_view(&f.chart, f.chart.config().chart_area.0)
            .unwrap();
        f.renderer
            .request_auto_streaming_chart(
                f.id,
                &f.view,
                crate::StreamingChartOptions {
                    size: (360, 240),
                    clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                    max_primitives_per_chunk: 2,
                },
            )
            .unwrap();
    };
    let mut failures = Vec::new();
    for &(name, edit) in edits {
        let mut f = setup(false, false, false, 1, 2);
        f.renderer.cancel_streaming_chart(f.id).unwrap();
        let _ = f.renderer.set_memory_budget(Some(128 * 1024 * 1024));
        let _ = f
            .renderer
            .set_auto_resident_working_set_limit(Some(1024 * 1024));
        let c = f.chart.config().clone();
        start(&mut f, c);
        finish_display(&mut f);
        let before = screen(&mut f);
        let mut next = f.chart.config().clone();
        edit(&mut next);
        start(&mut f, next.clone());
        let reads = pump_count(&mut f, None);
        finish_display(&mut f);
        let actual = screen(&mut f);
        let mut fresh = setup(false, false, false, 1, 2);
        fresh.renderer.cancel_streaming_chart(fresh.id).unwrap();
        start(&mut fresh, next);
        finish_display(&mut fresh);
        if actual != screen(&mut fresh) {
            failures.push(format!("{name}: stale first frame"));
        }
        if actual == before {
            failures.push(format!("{name}: edit invisible"));
        }
        if reads != 0 {
            failures.push(format!("{name}: {reads} unnecessary source requests"));
        }
    }
    drop(font);
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn progressive_replay_heatmap_fit_publishes_tiles_before_completion() {
    use crate::renderer::streaming_request_tests::read_draw_target;
    let font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    for samples in [1, 4] {
        // Force several screen tiles, with identical geometry at MSAA 1/4.
        let budget = 512 * 1024 * u64::from(samples);
        let mut f = setup_columns_with_budget(false, false, false, samples, 2, 1, budget);
        f.renderer.cancel_streaming_chart(f.id).unwrap();
        f.renderer.request_stream_auto_fit(f.id, 0.05).unwrap();
        let options = crate::StreamingChartOptions {
            size: (360, 240),
            clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
            max_primitives_per_chunk: 2,
        };
        f.renderer
            .request_auto_streaming_chart(f.id, &f.view, options)
            .unwrap();
        let mut previous = None;
        let mut refreshes = 0;
        let mut previous_serial = None;
        let mut replay_preview = None;
        let mut complete = false;
        for _ in 0..1024 {
            let bindings: Vec<_> = f
                .columns
                .iter()
                .map(|(id, c)| crate::StreamSourceBinding {
                    id,
                    revision: 5,
                    source: crate::StreamColumnSource::HiLo(c),
                })
                .collect();
            let step = f.renderer.auto_stream_chart_step(f.id, &bindings).unwrap();
            f.renderer.wait_idle();
            if matches!(step, crate::AutoStreamingProgress::Complete { .. }) {
                f.chart = Chart::new(f.renderer.chart_config(f.id).unwrap().clone());
                f.view = f
                    .renderer
                    .create_chart_view(&f.chart, f.chart.config().chart_area.0)
                    .unwrap();
            }
            f.renderer
                .prepare_registered(&[RegisteredChartDrawItem {
                    chart_id: f.id,
                    view: &f.view,
                }])
                .unwrap();
            let job = f.renderer.active_stream_job(f.id).unwrap();
            let target = wgpu::Texture::clone(f.renderer.chart_stream_display(job).unwrap());
            let pixels = read_draw_target(&f.renderer, &target);
            if f.renderer.progressive_replay_for_test(f.id) {
                let serial = f.renderer.stream_runtime.as_ref().unwrap().draws.iter()
                    .find(|draw| draw.job == job).unwrap().display_serial;
                if previous_serial.is_some_and(|old| old != serial) {
                    refreshes += 1;
                }
                previous_serial = Some(serial);
                // Field bounds are fitted before the first tile is drawn, so
                // this preview is already exact. Replaying identical tiles must
                // refresh the display without erasing strips at their edges.
                if let Some(preview) = &replay_preview {
                    assert_eq!(&pixels, preview, "replay damaged the already exact heatmap preview");
                } else {
                    replay_preview = Some(pixels.clone());
                }
            }
            previous = Some(pixels);
            if matches!(step, crate::AutoStreamingProgress::Complete { .. }) {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert!(refreshes >= 2, "heatmap froze until final swap");
        let final_config = f.renderer.chart_config(f.id).unwrap().clone();
        let mut fresh = setup_columns_with_budget(false, false, false, samples, 2, 1, budget);
        fresh.renderer.cancel_streaming_chart(fresh.id).unwrap();
        fresh
            .renderer
            .set_chart_config(fresh.id, final_config.clone())
            .unwrap();
        fresh.chart = Chart::new(final_config);
        fresh.view = fresh
            .renderer
            .create_chart_view(&fresh.chart, fresh.chart.config().chart_area.0)
            .unwrap();
        fresh
            .renderer
            .request_auto_streaming_chart(fresh.id, &fresh.view, options)
            .unwrap();
        finish_display(&mut fresh);
        let job = fresh.renderer.active_stream_job(fresh.id).unwrap();
        let target = wgpu::Texture::clone(fresh.renderer.chart_stream_display(job).unwrap());
        assert_eq!(
            previous.unwrap(),
            read_draw_target(&fresh.renderer, &target)
        );
    }
    drop(font);
}
