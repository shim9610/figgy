use super::streaming_runtime::*;
use super::*;
use crate::streaming::{ColumnRange, StreamError, StreamLimits};
use std::sync::atomic::Ordering;

#[path = "streaming_style_runtime_tests.rs"]
mod styled_runtime;

fn limits(slots: usize) -> StreamLimits {
    StreamLimits {
        max_jobs: 4,
        max_slots: slots,
        max_columns_per_request: 32,
        max_chunk_bytes: 1024,
        max_in_flight_bytes: 4096,
    }
}

fn source(id: &str, revision: u64) -> crate::StreamColumn {
    crate::StreamColumn {
        id: id.into(),
        len: 8,
        revision,
        encoding: crate::StreamEncoding::ScalarF32,
        replay: crate::StreamReplay::RandomAccess,
        statistics: crate::StreamStatistics::Unknown,
    }
}

fn declaration(id: &str, x: &str, y: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_width: 2.0,
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
            },
        },
    }
}

fn prefix_error_column(error: &mut ErrorRef) {
    let prefix = |id: &mut String| *id = format!("r{id}");
    match error {
        ErrorRef::Symmetric { column } => prefix(column),
        ErrorRef::Asymmetric { lower, upper } => {
            prefix(lower);
            prefix(upper);
        }
    }
}

#[test]
fn streaming_accepts_mapped_errorbars_but_rejects_contour() {
    let config = crate::default::default_config();
    let mut series = declaration("s", "x", "y");
    series.render_type = DataRenderType::ScatterErrorbarX {
        scatter: DataScatterStyleConfig {
            point_color: Color::new(0.0, 0.0, 0.0, 1.0),
            point_shape: ScatterShape::CircleFilled,
            point_size: 5.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
        },
        err_x: ErrorRef::Symmetric { column: "e".into() },
        err_style: DataErrorBarStyleConfig {
            error_bar_color: Color::new(0.0, 0.0, 0.0, 1.0),
            error_bar_width: 2.0,
            error_bar_cap_size: 4.0,
            cap_width: 2.0,
            error_bar_style_table: None,
            error_bar_style_index_column: Some("style".into()),
            error_bar_style_overrides: None,
        },
    };
    assert!(PrepareContext::validate_stream_series(&config, &series).is_ok());
    series.render_type = DataRenderType::Contour {
        matrix: crate::data_config::MatrixRef {
            columns: vec!["z".into()],
            orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
            grid_layout: crate::data_config::GridLayout::Edges,
        },
        contour: crate::data_config::ContourConfig {
            levels: vec![1.0],
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_width: 1.0,
                line_color: Color::new(0.0, 0.0, 0.0, 1.0),
            },
            per_level_color: None,
            labels: None,
        },
    };
    assert!(PrepareContext::validate_stream_series(&config, &series).is_err());
}

#[test]
fn streamed_mapped_scatter_and_errorbar_keep_global_override_ids() {
    let x = [0.12f32, 0.23, 0.34, 0.45, 0.56, 0.67, 0.78, 0.89];
    let y = [0.16f32, 0.77, 0.31, 0.68, f32::NAN, 0.27, 0.82, 0.43];
    let ex_lo = [0.04f32; 8];
    let ex_hi = [0.08f32; 8];
    let ey_lo = [0.06f32; 8];
    let ey_hi = [0.03f32; 8];
    let scatter_index = [0.0f32, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
    let error_index = [1.0f32, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
    let values = |id: &str| -> &[f32] {
        match id {
            "x" | "rx" => &x,
            "y" | "ry" => &y,
            "ex_lo" | "rex_lo" => &ex_lo,
            "ex_hi" | "rex_hi" => &ex_hi,
            "ey_lo" | "rey_lo" => &ey_lo,
            "ey_hi" | "rey_hi" => &ey_hi,
            "scatter_index" | "rscatter_index" => &scatter_index,
            "error_index" | "rerror_index" => &error_index,
            _ => panic!("unexpected mapped source {id}"),
        }
    };
    for samples in [1, 4] {
        let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
        let mut r = Renderer::try_new_with_sample_count(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
            samples,
        )
        .unwrap();
        r.configure_streaming_runtime(limits(2)).unwrap();
        let source_ids = [
            "x", "y", "ex_lo", "ex_hi", "ey_lo", "ey_hi", "scatter_index", "error_index",
        ];
        r.register_streamed_columns(source_ids.map(|id| source(id, 1)).to_vec())
            .unwrap();
        for id in source_ids {
            let resident_id = format!("r{id}");
            r.add_column(
                &resident_id,
                &crate::Column {
                    data: values(id).to_vec(),
                    min: 0.0,
                    max: 1.0,
                },
            )
            .unwrap();
        }
        let scatter = DataScatterStyleConfig {
            point_color: Color::new(0.1, 0.1, 0.1, 1.0),
            point_shape: ScatterShape::CircleFilled,
            point_size: 16.0,
            point_style_table: Some(vec![
                DataScatterPointStyleConfig {
                    point_color: Some(Color::new(1.0, 0.0, 0.0, 0.8)),
                    ..Default::default()
                },
                DataScatterPointStyleConfig {
                    point_color: Some(Color::new(0.0, 0.0, 1.0, 0.8)),
                    ..Default::default()
                },
            ]),
            point_style_index_column: Some("scatter_index".into()),
            point_style_overrides: Some(vec![
                crate::data_config::DataScatterPointStyleOverride {
                    index: 3,
                    style: DataScatterPointStyleConfig {
                        point_color: Some(Color::new(0.0, 1.0, 0.0, 1.0)),
                        ..Default::default()
                    },
                },
                crate::data_config::DataScatterPointStyleOverride {
                    index: 6,
                    style: DataScatterPointStyleConfig {
                        point_color: Some(Color::new(1.0, 0.0, 1.0, 1.0)),
                        ..Default::default()
                    },
                },
            ]),
        };
        let err_style = DataErrorBarStyleConfig {
            error_bar_color: Color::new(0.1, 0.1, 0.1, 0.7),
            error_bar_width: 5.0,
            error_bar_cap_size: 13.0,
            cap_width: 3.0,
            error_bar_style_table: Some(vec![
                DataErrorBarPointStyleConfig {
                    error_bar_color: Some(Color::new(0.9, 0.3, 0.0, 0.7)),
                    ..Default::default()
                },
                DataErrorBarPointStyleConfig {
                    error_bar_color: Some(Color::new(0.0, 0.5, 0.9, 0.7)),
                    ..Default::default()
                },
            ]),
            error_bar_style_index_column: Some("error_index".into()),
            error_bar_style_overrides: Some(vec![
                crate::data_config::DataErrorBarPointStyleOverride {
                    index: 3,
                    style: DataErrorBarPointStyleConfig {
                        error_bar_color: Some(Color::new(0.0, 1.0, 0.2, 1.0)),
                        ..Default::default()
                    },
                },
                crate::data_config::DataErrorBarPointStyleOverride {
                    index: 6,
                    style: DataErrorBarPointStyleConfig {
                        error_bar_color: Some(Color::new(1.0, 0.0, 0.9, 1.0)),
                        ..Default::default()
                    },
                },
            ]),
        };
        let mut stream_series = declaration("mapped", "x", "y");
        stream_series.render_type = DataRenderType::ScatterErrorbarXY {
            scatter,
            err_x: ErrorRef::Asymmetric {
                lower: "ex_lo".into(),
                upper: "ex_hi".into(),
            },
            err_y: ErrorRef::Asymmetric {
                lower: "ey_lo".into(),
                upper: "ey_hi".into(),
            },
            err_style,
        };
        let mut resident_series = stream_series.clone();
        resident_series.x_column = "rx".into();
        resident_series.y_column = "ry".into();
        if let DataRenderType::ScatterErrorbarXY {
            scatter,
            err_x,
            err_y,
            err_style,
        } = &mut resident_series.render_type
        {
            scatter.point_style_index_column = Some("rscatter_index".into());
            err_style.error_bar_style_index_column = Some("rerror_index".into());
            prefix_error_column(err_x);
            prefix_error_column(err_y);
        }
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 320,
            height: 240,
        });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);
        let resident_id = r.register_chart(chart.config().clone(), vec![resident_series]).unwrap();
        let stream_id = r.register_chart(chart.config().clone(), vec![stream_series]).unwrap();
        let view = r.create_chart_view(&chart, chart.config().chart_area.0).unwrap();
        let frame = r.prepare_registered(&[RegisteredChartDrawItem {
            chart_id: resident_id,
            view: &view,
        }]).unwrap();
        let reference = draw_target(&r, samples);
        clear_draw_target(&r, &reference);
        let reference_view = reference.create_view(&Default::default());
        let mut encoder = r.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &reference_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_viewport(0.0, 0.0, 320.0, 240.0, 0.0, 1.0);
            let data = chart.config().data_area().unwrap().0;
            pass.set_scissor_rect(data.x, data.y, data.width, data.height);
            data_render::issue_series_data(&mut pass, &frame.items[0].series[0].layers());
        }
        r.queue.submit([encoder.finish()]);
        let expected = read_draw_target(&r, &reference);
        assert!(expected.chunks_exact(4).any(|pixel| pixel != [255, 255, 255, 255]));
        for chunk_size in [1, 2, 4] {
            let target = draw_target(&r, samples);
            clear_draw_target(&r, &target);
            let job = r.begin_chart_stream_draw(stream_id, &view, &target, chunk_size).unwrap();
            let mut saw_errorbar = false;
            let mut saw_scatter = false;
            loop {
                wait_stream_slots(&mut r, 0);
                let ticket = match r.request_chart_stream_draw(job).unwrap() {
                    StreamDrawRequestStatus::Ready(ticket) => ticket,
                    StreamDrawRequestStatus::AllSubmitted => break,
                    StreamDrawRequestStatus::Backpressure => panic!("completed slots were drained"),
                };
                let columns: Vec<_> = r.stream_request_columns(ticket).unwrap().iter()
                    .map(|column| (column.column.clone(), column.range)).collect();
                match columns.len() {
                    7 => {
                        saw_errorbar = true;
                        assert_eq!(columns.last().unwrap().0, "error_index");
                    }
                    3 => {
                        saw_scatter = true;
                        assert_eq!(columns.last().unwrap().0, "scatter_index");
                    }
                    count => panic!("mapped pass requested {count} columns"),
                }
                let inputs: Vec<_> = columns.iter().map(|(id, range)| StreamInput {
                    column: id,
                    bytes: bytemuck::cast_slice(&values(id)[range.offset as usize
                        ..(range.offset + range.len) as usize]),
                }).collect();
                let submission = r.submit_chart_stream_draw(ticket, &inputs, &view, &target).unwrap();
                r.device.poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(30)),
                }).unwrap();
            }
            assert!(saw_errorbar && saw_scatter);
            assert_eq!(read_draw_target(&r, &target), expected,
                "mapped samples={samples} chunk={chunk_size}");
            r.cancel_chart_stream(stream_id).unwrap();
        }
    }
}

#[test]
fn mapped_style_upload_reserves_meta_budget_and_preserves_retry_ticket() {
    let (mut r, _, _) = renderer(2);
    r.register_streamed_columns(vec![source("idx", 1)]).unwrap();
    let mut series = declaration("mapped-budget", "x", "a");
    series.render_type = DataRenderType::Scatter {
        scatter: DataScatterStyleConfig {
            point_color: Color::new(1.0, 0.0, 0.0, 1.0),
            point_shape: ScatterShape::CircleFilled,
            point_size: 8.0,
            point_style_table: Some(vec![DataScatterPointStyleConfig::default()]),
            point_style_index_column: Some("idx".into()),
            point_style_overrides: None,
        },
    };
    let config = crate::default::default_config();
    let chart = r.register_chart(config.clone(), vec![series]).unwrap();
    let view = r.create_chart_view(&Chart::new(config),
        r.chart_config(chart).unwrap().chart_area.0).unwrap();
    let target = draw_target(&r, 1);
    clear_draw_target(&r, &target);
    let job = r.begin_chart_stream_draw(chart, &view, &target, 1).unwrap();
    let ticket = match r.request_chart_stream_draw(job).unwrap() {
        StreamDrawRequestStatus::Ready(ticket) => ticket,
        status => panic!("expected first mapped draw request, got {status:?}"),
    };
    let columns: Vec<_> = r.stream_request_columns(ticket).unwrap().iter()
        .map(|column| column.column.clone()).collect();
    assert_eq!(columns, ["x", "a", "idx"]);
    let bytes = [0u8; 4];
    let inputs: Vec<_> = columns.iter().map(|column| StreamInput {
        column,
        bytes: &bytes,
    }).collect();
    let before = r.gpu_memory_usage();
    // Three one-row hi/lo upload pairs consume 48B. The mapped lookup
    // requires another 16B; the 63B ceiling must reject before staging.
    assert!(r.set_memory_budget(Some(before.total_bytes() + 63)).is_none());
    assert!(matches!(r.submit_chart_stream_draw(ticket, &inputs, &view, &target),
        Err(StreamRequestError::Upload(
            crate::streaming_upload::ChunkUploadError::Input(StreamError::TooLarge)
        ))));
    assert_eq!(r.gpu_memory_usage(), before);
    assert_eq!(r.stream_request_columns(ticket).unwrap().len(), 3);
    assert!(r.set_memory_budget(Some(before.total_bytes() + 64)).is_none());
    let submission = r.submit_chart_stream_draw(ticket, &inputs, &view, &target).unwrap();
    r.device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(std::time::Duration::from_secs(30)),
    }).unwrap();
    r.cancel_chart_stream(chart).unwrap();
}

#[test]
fn short_mapped_style_index_fails_before_stream_draw_job() {
    let (mut r, _, _) = renderer(2);
    let mut short = source("idx", 1);
    short.len = 2;
    r.register_streamed_columns(vec![short]).unwrap();
    let mut series = declaration("short-style", "x", "a");
    series.render_type = DataRenderType::Scatter {
        scatter: DataScatterStyleConfig {
            point_color: Color::new(1.0, 0.0, 0.0, 1.0),
            point_shape: ScatterShape::CircleFilled,
            point_size: 8.0,
            point_style_table: Some(vec![DataScatterPointStyleConfig::default()]),
            point_style_index_column: Some("idx".into()),
            point_style_overrides: None,
        },
    };
    let config = crate::default::default_config();
    let chart = r.register_chart(config.clone(), vec![series]).unwrap();
    let view = r.create_chart_view(&Chart::new(config),
        r.chart_config(chart).unwrap().chart_area.0).unwrap();
    let target = draw_target(&r, 1);
    let before = r.gpu_memory_usage();
    assert!(matches!(r.begin_chart_stream_draw(chart, &view, &target, 1),
        Err(StreamRequestError::State(FiggyError::InvalidSeriesConfig { .. }))));
    assert_eq!(r.gpu_memory_usage(), before);
    assert_eq!(r.stream_request_usage(), (0, 0, 0));
}

#[test]
fn mapped_base_style_storage_is_preflighted_and_charged_once() {
    let (mut r, a, b) = renderer(2);
    let mut series = declaration("mapped-base", "x", "a");
    series.render_type = DataRenderType::Scatter {
        scatter: DataScatterStyleConfig {
            point_color: Color::new(1.0, 0.0, 0.0, 1.0),
            point_shape: ScatterShape::CircleFilled,
            point_size: 8.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: Some(vec![crate::data_config::DataScatterPointStyleOverride {
                index: 3,
                style: DataScatterPointStyleConfig::default(),
            }]),
        },
    };
    let config = crate::default::default_config();
    let chart = r.register_chart(config.clone(), vec![series]).unwrap();
    let view = r.create_chart_view(&Chart::new(config),
        r.chart_config(chart).unwrap().chart_area.0).unwrap();
    let target = draw_target(&r, 1);
    let before = r.gpu_memory_usage();
    // Four primitive uniforms plus empty-table dummy row, sparse row and meta.
    let style_bytes = r.style_allocation_bytes(&r.chart_series(chart).unwrap()[0]).unwrap();
    assert!(r.set_memory_budget(Some(before.total_bytes() + style_bytes - 1)).is_none());
    assert!(matches!(r.begin_chart_stream_draw(chart, &view, &target, 1),
        Err(StreamRequestError::State(FiggyError::GpuResourceLimit { resource: "chart style budget", .. }))));
    assert_eq!(r.gpu_memory_usage(), before);
    assert_eq!(r.stream_request_usage(), (0, 0, 0));
    assert!(r.set_memory_budget(Some(before.total_bytes() + style_bytes)).is_none());
    let extra_config = crate::default::default_config();
    let c = r.register_chart(extra_config.clone(), vec![declaration("c", "x", "a")]).unwrap();
    let d = r.register_chart(extra_config, vec![declaration("d", "x", "a")]).unwrap();
    for id in [a, b, c, d] {
        r.begin_chart_stream(id).unwrap();
    }
    assert!(matches!(r.begin_chart_stream_draw(chart, &view, &target, 1),
        Err(StreamRequestError::Scheduler(StreamError::TooManyJobs))));
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(r.chart_states[&chart].prepared_styles.is_none());
    r.cancel_chart_stream(d).unwrap();
    let first = r.begin_chart_stream_draw(chart, &view, &target, 1).unwrap();
    let admitted = r.gpu_memory_usage();
    assert_eq!(admitted.live_bytes_of(crate::GpuResourceKind::Uniform)
        - before.live_bytes_of(crate::GpuResourceKind::Uniform), style_bytes);
    r.cancel_chart_stream(chart).unwrap();
    let second = r.begin_chart_stream_draw(chart, &view, &target, 1).unwrap();
    assert_ne!(first, second);
    assert_eq!(r.gpu_memory_usage().live_bytes_of(crate::GpuResourceKind::Uniform),
        admitted.live_bytes_of(crate::GpuResourceKind::Uniform));
    r.cancel_chart_stream(chart).unwrap();
}

#[test]
fn six_distinct_errorbar_columns_obey_request_column_limit_before_job_start() {
    let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    let mut budget = limits(2);
    budget.max_columns_per_request = 5;
    renderer.configure_streaming_runtime(budget).unwrap();
    renderer
        .register_streamed_columns(
            ["x", "y", "ex_lo", "ex_hi", "ey_lo", "ey_hi"]
                .map(|id| source(id, 1))
                .to_vec(),
        )
        .unwrap();
    let mut series = declaration("s", "x", "y");
    series.render_type = DataRenderType::LineScatterErrorbarXY {
        scatter: DataScatterStyleConfig {
            point_color: Color::new(0.0, 0.0, 0.0, 1.0),
            point_shape: ScatterShape::CircleFilled,
            point_size: 5.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
        },
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_width: 2.0,
            line_color: Color::new(0.0, 0.0, 0.0, 1.0),
        },
        err_x: ErrorRef::Asymmetric {
            lower: "ex_lo".into(),
            upper: "ex_hi".into(),
        },
        err_y: ErrorRef::Asymmetric {
            lower: "ey_lo".into(),
            upper: "ey_hi".into(),
        },
        err_style: DataErrorBarStyleConfig {
            error_bar_color: Color::new(0.0, 0.0, 0.0, 1.0),
            error_bar_width: 2.0,
            error_bar_cap_size: 4.0,
            cap_width: 2.0,
            error_bar_style_table: None,
            error_bar_style_index_column: None,
            error_bar_style_overrides: None,
        },
    };
    let config = crate::default::default_config();
    let chart = renderer.register_chart(config.clone(), vec![series]).unwrap();
    let view = renderer
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let target = draw_target(&renderer, 1);
    let before = renderer.gpu_memory_usage();
    assert!(matches!(
        renderer.begin_chart_stream_draw(chart, &view, &target, 2),
        Err(StreamRequestError::Scheduler(StreamError::TooLarge))
    ));
    assert_eq!(renderer.gpu_memory_usage(), before);
    assert_eq!(renderer.stream_request_usage(), (0, 0, 0));
}

#[test]
fn histogram_cursor_requests_one_extra_edge_but_not_one_extra_value() {
    for (orientation, alias) in [
        (crate::data_config::BarOrientation::Vertical, false),
        (crate::data_config::BarOrientation::Horizontal, false),
        (crate::data_config::BarOrientation::Vertical, true),
    ] {
        let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
        let mut renderer = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        renderer.configure_streaming_runtime(limits(2)).unwrap();
        let (x_id, y_id) = if alias { ("same", "same") } else { ("x", "y") };
        renderer
            .register_streamed_columns(if alias {
                let mut column = source("same", 1);
                column.len = 9;
                vec![column]
            } else {
                let mut x = source("x", 1);
                let mut y = source("y", 1);
                if orientation == crate::data_config::BarOrientation::Vertical {
                    x.len = 9;
                } else {
                    y.len = 9;
                }
                vec![x, y]
            })
            .unwrap();
        let mut series = declaration("h", x_id, y_id);
        series.render_type = DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: Color::new(0.0, 0.8, 0.0, 1.0),
                border_color: Color::new(0.0, 0.0, 0.0, 1.0),
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 0.0,
                width_ratio: 1.0,
                orientation: orientation.clone(),
                bar_style_overrides: None,
            },
        };
        let config = crate::default::default_config();
        let chart = renderer.register_chart(config.clone(), vec![series]).unwrap();
        let view = renderer
            .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
            .unwrap();
        let target = draw_target(&renderer, 1);
        let job = renderer
            .begin_chart_stream_draw(chart, &view, &target, 3)
            .unwrap();
        let ticket = match renderer.request_chart_stream_draw(job).unwrap() {
            StreamDrawRequestStatus::Ready(ticket) => ticket,
            other => panic!("expected histogram ticket, got {other:?}"),
        };
        let columns = renderer.stream_request_columns(ticket).unwrap();
        let edge_id = if orientation == crate::data_config::BarOrientation::Vertical {
            x_id
        } else {
            y_id
        };
        assert_eq!(columns.len(), if alias { 1 } else { 2 });
        assert!(columns.iter().all(|column| {
            column.range.offset == 0
                && column.range.len == if column.column == edge_id { 4 } else { 3 }
        }));
        renderer.cancel_chart_stream(chart).unwrap();
    }
}

#[test]
fn streamed_histogram_winner_and_mapped_bins_match_resident_full_chart() {
    let _font_registration = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .expect("font registration test lock poisoned");
    // Fifteen narrow bins share a handful of screen columns; the final bin is
    // wide enough to exercise the ordinary bar draw as well as the envelope.
    let edges: Vec<f32> = (0..16)
        .map(|i| i as f32 * 0.25)
        .chain(std::iter::once(12.0))
        .collect();
    let values: Vec<f32> = (0..16)
        .map(|i| if i == 5 { f32::NAN } else { 1.0 + ((i * 7) % 9) as f32 })
        .collect();
    for orientation in [
        crate::data_config::BarOrientation::Vertical,
        crate::data_config::BarOrientation::Horizontal,
    ] {
        for mapped in [false, true] {
            for samples in [1, 4] {
                let (device, queue) =
                    crate::data_render::shared_device().expect("stream GPU required");
                let mut r = Renderer::try_new_with_sample_count(
                    RendererDevice::new(device, queue),
                    wgpu::TextureFormat::Rgba8Unorm,
                    4096,
                    samples,
                )
                .unwrap();
                r.configure_streaming_runtime(limits(2)).unwrap();
                let (x, y, rx, ry) = match orientation {
                    crate::data_config::BarOrientation::Vertical => ("edge", "value", "redge", "rvalue"),
                    crate::data_config::BarOrientation::Horizontal => ("value", "edge", "rvalue", "redge"),
                };
                let mut edge_source = source("edge", 1);
                edge_source.len = edges.len() as u64;
                let mut value_source = source("value", 1);
                value_source.len = values.len() as u64;
                r.register_streamed_columns(vec![edge_source, value_source]).unwrap();
                for (id, data) in [("redge", &edges), ("rvalue", &values)] {
                    r.add_column(
                        id,
                        &crate::Column {
                            data: data.clone(),
                            min: 0.0,
                            max: 12.0,
                        },
                    )
                    .unwrap();
                }
                let mut config = crate::default::default_config();
                config.chart_area = crate::layout::ChartArea(Rect {
                    x: 0,
                    y: 0,
                    width: 320,
                    height: 240,
                });
                let mut chart = Chart::new(config);
                match orientation {
                    crate::data_config::BarOrientation::Vertical => {
                        chart.set_x_range(0.0, 100.0);
                        chart.set_y_range(0.0, 10.0);
                    }
                    crate::data_config::BarOrientation::Horizontal => {
                        chart.set_x_range(0.0, 10.0);
                        chart.set_y_range(0.0, 100.0);
                    }
                }
                let declarations: Vec<_> = [
                    ("first", Color::new(0.0, 0.7, 0.2, 0.55)),
                    ("second", Color::new(0.2, 0.1, 0.8, 0.35)),
                ]
                .into_iter()
                .map(|(id, fill)| {
                    let mut series = declaration(id, x, y);
                    series.render_type = DataRenderType::Histogram {
                        bar: DataBarStyleConfig {
                            fill_color: fill,
                            border_color: Color::new(0.0, 0.0, 0.0, 0.8),
                            border_width: 1.0,
                            baseline: 0.0,
                            gap_px: 0.0,
                            width_ratio: 1.0,
                            orientation: orientation.clone(),
                            bar_style_overrides: mapped.then(|| vec![
                                crate::data_config::DataBarStyleOverride {
                                    index: 2,
                                    style: crate::data_config::DataBarBinStyleConfig {
                                        border_color: Some(Color::new(1.0, 0.0, 0.0, 0.9)),
                                        ..Default::default()
                                    },
                                },
                                crate::data_config::DataBarStyleOverride {
                                    index: 12,
                                    style: crate::data_config::DataBarBinStyleConfig {
                                        fill_color: Some(Color::new(1.0, 0.8, 0.0, 0.6)),
                                        ..Default::default()
                                    },
                                },
                            ]),
                        },
                    };
                    series
                })
                .collect();
                let mut resident = declarations.clone();
                for series in &mut resident {
                    series.x_column = rx.into();
                    series.y_column = ry.into();
                }
                r.add_column(
                    "pedge",
                    &crate::Column {
                        data: edges[..5].to_vec(),
                        min: 0.0,
                        max: 1.0,
                    },
                )
                .unwrap();
                r.add_column(
                    "pvalue",
                    &crate::Column {
                        data: values[..4].to_vec(),
                        min: 0.0,
                        max: 10.0,
                    },
                )
                .unwrap();
                let mut prefix_series = declarations[0].clone();
                match orientation {
                    crate::data_config::BarOrientation::Vertical => {
                        prefix_series.x_column = "pedge".into();
                        prefix_series.y_column = "pvalue".into();
                    }
                    crate::data_config::BarOrientation::Horizontal => {
                        prefix_series.x_column = "pvalue".into();
                        prefix_series.y_column = "pedge".into();
                    }
                }
                let resident_id = r.register_chart(chart.config().clone(), resident).unwrap();
                let prefix_id = r
                    .register_chart(chart.config().clone(), vec![prefix_series])
                    .unwrap();
                let stream_id = r.register_chart(chart.config().clone(), declarations).unwrap();
                let view = r
                    .create_chart_view(&chart, chart.config().chart_area.0)
                    .unwrap();
                let frame = r
                    .prepare_registered(&[RegisteredChartDrawItem {
                        chart_id: resident_id,
                        view: &view,
                    }])
                    .unwrap();
                let expected = paint_frame_pixels(&r, &frame, samples);
                let prefix_frame = r
                    .prepare_registered(&[RegisteredChartDrawItem {
                        chart_id: prefix_id,
                        view: &view,
                    }])
                    .unwrap();
                let expected_prefix = paint_frame_pixels(&r, &prefix_frame, samples);
                assert!(expected.chunks_exact(4).any(|p| p != [255, 255, 255, 255]));
                for chunk_size in [1, 2, 4] {
                    let job = r
                        .begin_chart_stream_surface(
                            stream_id,
                            &view,
                            (320, 240),
                            wgpu::Color::WHITE,
                            chunk_size,
                        )
                        .unwrap();
                    let mut submissions = 0;
                    assert!(r.refresh_chart_stream_display(job, &view).unwrap());
                    loop {
                        wait_stream_slots(&mut r, 0);
                        let ticket = match r.request_chart_stream_draw(job).unwrap() {
                            StreamDrawRequestStatus::Ready(ticket) => ticket,
                            StreamDrawRequestStatus::AllSubmitted => break,
                            StreamDrawRequestStatus::Backpressure => panic!("slots drained"),
                        };
                        let columns: Vec<_> = r
                            .stream_request_columns(ticket)
                            .unwrap()
                            .iter()
                            .map(|column| (column.column.clone(), column.range))
                            .collect();
                        assert_eq!(columns.len(), 2);
                        let inputs: Vec<_> = columns
                            .iter()
                            .map(|(id, range)| {
                                let data = if id == "edge" { &edges } else { &values };
                                StreamInput {
                                    column: id,
                                    bytes: bytemuck::cast_slice(
                                        &data[range.offset as usize
                                            ..(range.offset + range.len) as usize],
                                    ),
                                }
                            })
                            .collect();
                        r.submit_chart_stream_surface(ticket, &inputs, &view)
                            .unwrap_or_else(|error| panic!("orientation={orientation:?}, mapped={mapped}, samples={samples}, chunk={chunk_size}, columns={columns:?}: {error:?}"));
                        assert!(r.refresh_chart_stream_display(job, &view).unwrap());
                        assert!(!r.refresh_chart_stream_display(job, &view).unwrap());
                        if chunk_size == 4 && submissions == 0 {
                            let display = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
                            assert_eq!(
                                read_draw_target(&r, &display),
                                expected_prefix,
                                "partial orientation={orientation:?}, mapped={mapped}, samples={samples}"
                            );
                        }
                        submissions += 1;
                    }
                    assert_eq!(submissions, 2 * 16usize.div_ceil(chunk_size as usize));
                    let display = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
                    assert_eq!(
                        read_draw_target(&r, &display),
                        expected,
                        "orientation={orientation:?}, mapped={mapped}, samples={samples}, chunk={chunk_size}"
                    );
                    r.cancel_chart_stream(stream_id).unwrap();
                }
            }
        }
    }
}

#[test]
fn streamed_histogram_same_id_edges_and_values_match_resident() {
    let _font_registration = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .expect("font registration test lock poisoned");
    let values: Vec<f32> = (0..17).map(|i| i as f32 * 0.25).collect();
    let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    r.configure_streaming_runtime(limits(2)).unwrap();
    let mut source = source("same", 1);
    source.len = values.len() as u64;
    r.register_streamed_columns(vec![source]).unwrap();
    r.add_column(
        "rsame",
        &crate::Column {
            data: values.clone(),
            min: 0.0,
            max: 4.0,
        },
    )
    .unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 100.0);
    chart.set_y_range(0.0, 10.0);
    let mut series = declaration("alias", "same", "same");
    series.render_type = DataRenderType::Histogram {
        bar: DataBarStyleConfig {
            fill_color: Color::new(0.0, 0.6, 0.0, 0.5),
            border_color: Color::new(0.0, 0.0, 0.0, 0.9),
            border_width: 1.0,
            baseline: 0.0,
            gap_px: 0.0,
            width_ratio: 1.0,
            orientation: crate::data_config::BarOrientation::Vertical,
            bar_style_overrides: None,
        },
    };
    let stream_id = r.register_chart(chart.config().clone(), vec![series.clone()]).unwrap();
    series.x_column = "rsame".into();
    series.y_column = "rsame".into();
    let resident_id = r.register_chart(chart.config().clone(), vec![series]).unwrap();
    let view = r.create_chart_view(&chart, chart.config().chart_area.0).unwrap();
    let frame = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id: resident_id,
            view: &view,
        }])
        .unwrap();
    let expected = paint_frame_pixels(&r, &frame, 1);
    for chunk_size in [1, 2, 4] {
        let job = r
            .begin_chart_stream_surface(
                stream_id,
                &view,
                (320, 240),
                wgpu::Color::WHITE,
                chunk_size,
            )
            .unwrap();
        loop {
            wait_stream_slots(&mut r, 0);
            let ticket = match r.request_chart_stream_draw(job).unwrap() {
                StreamDrawRequestStatus::Ready(ticket) => ticket,
                StreamDrawRequestStatus::AllSubmitted => break,
                StreamDrawRequestStatus::Backpressure => panic!("slots drained"),
            };
            let requested = r.stream_request_columns(ticket).unwrap();
            assert_eq!(requested.len(), 1);
            let range = requested[0].range;
            let input = StreamInput {
                column: "same",
                bytes: bytemuck::cast_slice(
                    &values[range.offset as usize..(range.offset + range.len) as usize],
                ),
            };
            r.submit_chart_stream_surface(ticket, &[input], &view).unwrap();
            r.refresh_chart_stream_display(job, &view).unwrap();
        }
        let display = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
        assert_eq!(
            read_draw_target(&r, &display),
            expected,
            "alias chunk={chunk_size}"
        );
        r.cancel_chart_stream(stream_id).unwrap();
    }
}

#[test]
fn malformed_first_histogram_supply_is_allocation_free_and_retryable_at_cap() {
    let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    r.configure_streaming_runtime(limits(2)).unwrap();
    let edges: Vec<f32> = (0..9).map(|i| i as f32 * 0.25).collect();
    let values: Vec<f32> = (0..8).map(|i| (i + 1) as f32).collect();
    let mut edge_source = source("edge", 1);
    edge_source.len = edges.len() as u64;
    r.register_streamed_columns(vec![edge_source, source("value", 1)])
        .unwrap();
    let mut series = declaration("h", "edge", "value");
    series.render_type = DataRenderType::Histogram {
        bar: DataBarStyleConfig {
            fill_color: Color::new(0.0, 0.8, 0.0, 1.0),
            border_color: Color::BLACK,
            border_width: 1.0,
            baseline: 0.0,
            gap_px: 0.0,
            width_ratio: 1.0,
            orientation: crate::data_config::BarOrientation::Vertical,
            bar_style_overrides: None,
        },
    };
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 100.0);
    chart.set_y_range(0.0, 10.0);
    let chart_id = r.register_chart(chart.config().clone(), vec![series]).unwrap();
    let view = r.create_chart_view(&chart, chart.config().chart_area.0).unwrap();
    let target = draw_target(&r, 1);
    clear_draw_target(&r, &target);
    let job = r
        .begin_chart_stream_draw(chart_id, &view, &target, 3)
        .unwrap();
    let ticket = match r.request_chart_stream_draw(job).unwrap() {
        StreamDrawRequestStatus::Ready(ticket) => ticket,
        other => panic!("expected ticket, got {other:?}"),
    };
    let before = r.gpu_memory_usage();
    assert_eq!(r.set_memory_budget(Some(before.total_bytes() + 8000)), None);
    assert!(r.submit_chart_stream_draw(ticket, &[], &view, &target).is_err());
    assert_eq!(r.gpu_memory_usage(), before);
    assert_eq!(
        r.request_chart_stream_draw(job).unwrap(),
        StreamDrawRequestStatus::Ready(ticket)
    );
    let columns: Vec<_> = r
        .stream_request_columns(ticket)
        .unwrap()
        .iter()
        .map(|column| (column.column.clone(), column.range))
        .collect();
    let inputs: Vec<_> = columns
        .iter()
        .map(|(id, range)| {
            let data = if id == "edge" { &edges } else { &values };
            StreamInput {
                column: id,
                bytes: bytemuck::cast_slice(
                    &data[range.offset as usize..(range.offset + range.len) as usize],
                ),
            }
        })
        .collect();
    r.submit_chart_stream_draw(ticket, &inputs, &view, &target)
        .unwrap();
    wait_stream_slots(&mut r, 0);
    let retirement_deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while r.gpu_memory_usage().retired_bytes() != before.retired_bytes() {
        r.service_gpu_completions().unwrap();
        assert!(
            std::time::Instant::now() < retirement_deadline,
            "stream retirement callback timeout"
        );
        std::thread::yield_now();
    }
    assert_eq!(r.gpu_memory_usage().retired_bytes(), before.retired_bytes());
    assert!(r.gpu_memory_usage().live_bytes() > before.live_bytes());
    loop {
        let ticket = match r.request_chart_stream_draw(job).unwrap() {
            StreamDrawRequestStatus::Ready(ticket) => ticket,
            StreamDrawRequestStatus::AllSubmitted => break,
            StreamDrawRequestStatus::Backpressure => panic!("completed slot remained occupied"),
        };
        let columns: Vec<_> = r
            .stream_request_columns(ticket)
            .unwrap()
            .iter()
            .map(|column| (column.column.clone(), column.range))
            .collect();
        let inputs: Vec<_> = columns
            .iter()
            .map(|(id, range)| {
                let data = if id == "edge" { &edges } else { &values };
                StreamInput {
                    column: id,
                    bytes: bytemuck::cast_slice(
                        &data[range.offset as usize
                            ..(range.offset + range.len) as usize],
                    ),
                }
            })
            .collect();
        r.submit_chart_stream_draw(ticket, &inputs, &view, &target)
            .unwrap();
        wait_stream_slots(&mut r, 0);
    }
    wait_stream_retirement(&r);
    assert_eq!(r.gpu_memory_usage().total_bytes(), before.total_bytes());
    assert_eq!(r.gpu_memory_usage().retired_bytes(), before.retired_bytes());
}

fn renderer(slots: usize) -> (Renderer, ChartId, ChartId) {
    let (device, queue) =
        crate::data_render::shared_device().expect("stream request tests require GPU");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    r.register_streamed_columns(vec![source("x", 1), source("a", 1), source("b", 1)])
        .unwrap();
    let a = r
        .register_chart(
            crate::default::default_config(),
            vec![declaration("s", "x", "a")],
        )
        .unwrap();
    let b = r
        .register_chart(
            crate::default::default_config(),
            vec![declaration("s", "x", "b")],
        )
        .unwrap();
    r.configure_streaming_runtime(limits(slots)).unwrap();
    (r, a, b)
}

#[test]
fn automatic_stream_builds_chart_local_packed_rows_without_promoting_global_columns() {
    let (mut r, chart_id, other_chart) = renderer(2);
    let budget = r.gpu_memory_usage().total_bytes() + 64 * 1024 * 1024;
    let _ = r.set_memory_budget(Some(budget));
    let _ = r.set_auto_resident_working_set_limit(Some(500_000_000));
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r.create_chart_view(&Chart::new(config.clone()), config.chart_area.0).unwrap();
    r.request_auto_streaming_chart(chart_id, &view, crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    }).unwrap();
    let x = crate::Column { data: (0..8).map(|i| i as f32 / 8.0).collect(), min: 0.0, max: 0.875 };
    let y = crate::Column { data: (0..8).map(|i| i as f32 / 16.0).collect(), min: 0.0, max: 0.4375 };
    let bindings = [
        crate::StreamSourceBinding { id: "x", revision: 1, source: crate::StreamColumnSource::Scalar(&x) },
        crate::StreamSourceBinding { id: "a", revision: 1, source: crate::StreamColumnSource::Scalar(&y) },
    ];
    loop {
        match r.auto_stream_chart_step(chart_id, &bindings).unwrap() {
            crate::AutoStreamingProgress::Submitted { .. } => {},
            crate::AutoStreamingProgress::Backpressure { .. } => wait_stream_slots(&mut r, 0),
            crate::AutoStreamingProgress::AllSubmitted { .. } => break,
            crate::AutoStreamingProgress::Complete { .. } => panic!("premature completion"),
        }
    }
    wait_stream_slots(&mut r, 0);
    r.prepare_registered(&[RegisteredChartDrawItem { chart_id, view: &view }]).unwrap();
    assert!(matches!(r.auto_stream_chart_step(chart_id, &bindings).unwrap(),
        crate::AutoStreamingProgress::Complete { .. }));
    let (chunks, packed_bytes) = r.view_cache_test_status(chart_id).expect("packed view cache");
    assert_eq!(chunks, 1, "small visible source chunks should share one GPU buffer");
    assert!(packed_bytes <= 500_000_000);
    assert!(r.pool.slot("x").is_none());
    assert!(r.pool.slot("a").is_none());
    assert!(r.streaming_sources.contains_key("x"));
    assert!(r.streaming_sources.contains_key("a"));
    assert!(r.chart_config(other_chart).is_ok());

    // A narrower view uses the packed GPU rows, with no source-range read.
    let mut narrowed = config.clone();
    narrowed.bottom_x.min = 0.25;
    narrowed.bottom_x.max = 0.75;
    narrowed.left_y.min = 0.125;
    narrowed.left_y.max = 0.375;
    r.set_chart_config(chart_id, narrowed.clone()).unwrap();
    let narrowed_view = r.create_chart_view(&Chart::new(narrowed.clone()), narrowed.chart_area.0).unwrap();
    assert!(matches!(r.request_auto_streaming_chart(chart_id, &narrowed_view, crate::StreamingChartOptions {
        size: (narrowed.chart_area.0.width, narrowed.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    }).unwrap(), crate::AutoStreamingRequest::Started { .. }));
    assert!(matches!(r.auto_stream_chart_request_ranges(chart_id).unwrap(),
        crate::AutoStreamingRangeRequest::AllSubmitted { .. }));
    assert!(r.view_cache_test_status(chart_id).is_some());
    assert!(r.pool.slot("x").is_none());
    wait_stream_slots(&mut r, 0);
    r.prepare_registered(&[RegisteredChartDrawItem { chart_id, view: &narrowed_view }]).unwrap();
    assert!(matches!(r.auto_stream_chart_request_ranges(chart_id).unwrap(),
        crate::AutoStreamingRangeRequest::Complete { .. }));
    let area = narrowed.data_area().unwrap().0;
    let hit = pollster::block_on(r.pick_chart_view_cache(
        chart_id,
        [area.x as f32 + area.width as f32 * 0.5,
         area.y as f32 + area.height as f32 * 0.5],
        20.0,
    )).unwrap().expect("packed view GPU pick");
    assert_eq!(hit.series_id, "s");
    assert_eq!(hit.point_index, 4, "GPU pick must return the original source row");
    assert_eq!(r.next_view_point_index(chart_id, None, "s", 4, true), Some(5));
    assert_eq!(r.next_view_point_index(chart_id, None, "s", 4, false), Some(3));
    let mut selected = narrowed;
    selected.picked_points = Some(crate::PickedPointsConfig {
        refs: vec![crate::PickedPointRef {
            source_id: None, series_id: "s".into(), point_index: 4,
        }],
        ..Default::default()
    });
    r.set_chart_config(chart_id, selected).unwrap();
    assert!(matches!(r.request_stream_selection_ranges(chart_id).unwrap(),
        crate::StreamingSelectionRequest::Complete { .. }),
        "packed selection must use its resident GPU page, not request the source row");
    assert_eq!(r.view_selection_ring_count(chart_id), Some(1),
        "packed selection must prepare the requested ring");
}

#[test]
fn packed_view_pick_reduces_across_bounded_submission_batches() {
    let (mut r, chart_id, _) = renderer(2);
    let budget = r.gpu_memory_usage().total_bytes() + 64 * 1024 * 1024;
    let _ = r.set_memory_budget(Some(budget));
    let _ = r.set_auto_resident_working_set_limit(Some(500_000_000));
    let config = r.chart_config(chart_id).unwrap().clone();
    let series: Vec<_> = (0..9)
        .map(|index| declaration(&format!("batch-{index}"), "x", "a"))
        .collect();
    r.set_chart_series(chart_id, series).unwrap();
    let x = crate::Column {
        data: (0..8).map(|index| index as f32 / 10.0).collect(),
        min: 0.0,
        max: 0.7,
    };
    let y = crate::Column {
        data: (0..8).map(|index| index as f32 / 10.0).collect(),
        min: 0.0,
        max: 0.7,
    };
    let bindings = [
        crate::StreamSourceBinding { id: "x", revision: 1, source: crate::StreamColumnSource::Scalar(&x) },
        crate::StreamSourceBinding { id: "a", revision: 1, source: crate::StreamColumnSource::Scalar(&y) },
    ];
    run_auto_stream_for_view_test(&mut r, chart_id, &config, crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    }, &bindings);
    assert_eq!(r.view_cache_test_status(chart_id).unwrap().0, 9);
    let area = config.data_area().unwrap().0;
    let hit = pollster::block_on(r.pick_chart_view_cache(
        chart_id,
        [area.x as f32 + area.width as f32 * 0.4,
         area.y as f32 + area.height as f32 * 0.6],
        16.0,
    )).unwrap().expect("batched packed-view pick");
    assert_eq!(hit.series_id, "batch-8");
    assert_eq!(hit.point_index, 4);
}

#[test]
fn packed_view_redraw_matches_exact_stream_pixels_after_zoom() {
    let _font_registration = crate::text_render::FONT_REGISTRATION_TEST_LOCK.lock().unwrap();
    let x = crate::Column {
        data: [0.3, 0.3, -5.0, -5.0, 0.7, 0.7, 0.8, 0.8].to_vec(),
        min: -5.0, max: 0.8,
    };
    let y = crate::Column {
        data: [0.5, 2.0, 2.0, -5.0, -5.0, 0.5, 0.5, 2.0].to_vec(),
        min: -5.0, max: 2.0,
    };
    let bindings = [
        crate::StreamSourceBinding { id: "x", revision: 1, source: crate::StreamColumnSource::Scalar(&x) },
        crate::StreamSourceBinding { id: "a", revision: 1, source: crate::StreamColumnSource::Scalar(&y) },
    ];
    let options = crate::StreamingChartOptions {
        size: (320, 240), clear_color: Color::WHITE, max_primitives_per_chunk: 8,
    };
    let (mut cached, cached_id, _) = renderer(2);
    let mut full = cached.chart_config(cached_id).unwrap().clone();
    full.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 320, height: 240 });
    let budget = cached.gpu_memory_usage().total_bytes() + 64 * 1024 * 1024;
    let _ = cached.set_memory_budget(Some(budget));
    let _ = cached.set_auto_resident_working_set_limit(Some(500_000_000));
    cached.set_chart_config(cached_id, full.clone()).unwrap();
    run_auto_stream_for_view_test(&mut cached, cached_id, &full, options, &bindings);
    assert!(cached.view_cache_test_status(cached_id).is_some());

    let mut narrow = full.clone();
    narrow.bottom_x.min = 0.25;
    narrow.bottom_x.max = 0.75;
    narrow.left_y.min = 0.25;
    narrow.left_y.max = 0.75;
    cached.set_chart_config(cached_id, narrow.clone()).unwrap();
    let narrow_view = cached.create_chart_view(&Chart::new(narrow.clone()), narrow.chart_area.0).unwrap();
    cached.request_auto_streaming_chart(cached_id, &narrow_view, options).unwrap();
    assert!(matches!(cached.auto_stream_chart_request_ranges(cached_id).unwrap(),
        crate::AutoStreamingRangeRequest::AllSubmitted { .. }));
    wait_stream_slots(&mut cached, 0);
    cached.prepare_registered(&[RegisteredChartDrawItem { chart_id: cached_id, view: &narrow_view }]).unwrap();
    let cached_target = cached.stream_target_test(cached_id).unwrap();
    let cached_pixels = read_draw_target(&cached, &cached_target);

    let (mut streamed, streamed_id, _) = renderer(2);
    streamed.set_chart_config(streamed_id, narrow.clone()).unwrap();
    run_auto_stream_for_view_test(&mut streamed, streamed_id, &narrow, options, &bindings);
    let streamed_target = streamed.stream_target_test(streamed_id).unwrap();
    let streamed_pixels = read_draw_target(&streamed, &streamed_target);
    assert!(streamed_pixels.chunks_exact(4).any(|pixel| {
        pixel[0] > pixel[1].saturating_add(40)
            && pixel[0] > pixel[2].saturating_add(40)
    }), "the comparison must include visible red line pixels, not only axes");
    assert_eq!(cached_pixels, streamed_pixels);
}

fn run_auto_stream_for_view_test(
    r: &mut Renderer,
    chart_id: ChartId,
    config: &crate::Config,
    options: crate::StreamingChartOptions,
    bindings: &[crate::StreamSourceBinding<'_>],
) {
    let view = r.create_chart_view(&Chart::new(config.clone()), config.chart_area.0).unwrap();
    r.request_auto_streaming_chart(chart_id, &view, options).unwrap();
    loop {
        match r.auto_stream_chart_step(chart_id, bindings).unwrap() {
            crate::AutoStreamingProgress::Submitted { .. } => {}
            crate::AutoStreamingProgress::Backpressure { .. } => wait_stream_slots(r, 0),
            crate::AutoStreamingProgress::AllSubmitted { .. } => break,
            crate::AutoStreamingProgress::Complete { .. } => panic!("premature completion"),
        }
    }
    wait_stream_slots(r, 0);
    r.prepare_registered(&[RegisteredChartDrawItem { chart_id, view: &view }]).unwrap();
    assert!(matches!(r.auto_stream_chart_step(chart_id, bindings).unwrap(),
        crate::AutoStreamingProgress::Complete { .. }));
}

#[test]
fn zero_configured_view_limit_disables_candidate_without_changing_stream_draw() {
    let (mut r, chart_id, _) = renderer(2);
    let budget = r.gpu_memory_usage().total_bytes() + 64 * 1024 * 1024;
    let _ = r.set_memory_budget(Some(budget));
    let _ = r.set_auto_resident_working_set_limit(Some(0));
    let config = r.chart_config(chart_id).unwrap().clone();
    let x = crate::Column { data: vec![0.25; 8], min: 0.25, max: 0.25 };
    let y = crate::Column { data: vec![0.5; 8], min: 0.5, max: 0.5 };
    let bindings = [
        crate::StreamSourceBinding { id: "x", revision: 1, source: crate::StreamColumnSource::Scalar(&x) },
        crate::StreamSourceBinding { id: "a", revision: 1, source: crate::StreamColumnSource::Scalar(&y) },
    ];
    run_auto_stream_for_view_test(&mut r, chart_id, &config, crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    }, &bindings);
    let status = r.view_residency_status(chart_id).unwrap();
    assert_eq!(status.state, "streamed");
    assert_eq!(status.refusal_reason, Some("disabled"));
    assert!(r.view_cache_test_status(chart_id).is_none());
}

#[test]
fn packed_view_combined_errorbar_redraw_matches_exact_stream_pixels() {
    let _font_registration = crate::text_render::FONT_REGISTRATION_TEST_LOCK.lock().unwrap();
    let x = crate::Column { data: vec![0.3, 0.4, -0.1, 0.6, 0.7, 0.8, 0.9, 1.0], min: -0.1, max: 1.0 };
    let y = crate::Column { data: vec![0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0], min: 0.3, max: 1.0 };
    let e = crate::Column { data: vec![0.1, 0.1, 0.5, 0.1, 0.1, 0.1, 0.1, 0.1], min: 0.1, max: 0.5 };
    let bindings = [
        crate::StreamSourceBinding { id: "x", revision: 1, source: crate::StreamColumnSource::Scalar(&x) },
        crate::StreamSourceBinding { id: "a", revision: 1, source: crate::StreamColumnSource::Scalar(&y) },
        crate::StreamSourceBinding { id: "e", revision: 1, source: crate::StreamColumnSource::Scalar(&e) },
    ];
    let options = crate::StreamingChartOptions { size: (320, 240), clear_color: Color::WHITE, max_primitives_per_chunk: 3 };
    let configure = |r: &mut Renderer, chart_id: ChartId, config: &crate::Config| {
        r.register_streamed_columns(vec![source("e", 1)]).unwrap();
        let mut series = declaration("s", "x", "a");
        series.render_type = DataRenderType::LineScatterErrorbarX {
            scatter: DataScatterStyleConfig {
                point_color: Color::new(1.0, 0.0, 0.0, 1.0),
                point_shape: ScatterShape::CircleFilled,
                point_size: 5.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
            },
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_width: 2.0,
                line_color: Color::new(1.0, 0.0, 0.0, 1.0),
            },
            err_x: ErrorRef::Symmetric { column: "e".into() },
            err_style: DataErrorBarStyleConfig {
                error_bar_color: Color::new(0.0, 0.0, 1.0, 1.0),
                error_bar_width: 2.0,
                error_bar_cap_size: 4.0,
                cap_width: 2.0,
                error_bar_style_table: None,
                error_bar_style_index_column: None,
                error_bar_style_overrides: None,
            },
        };
        r.set_chart_series(chart_id, vec![series]).unwrap();
        r.set_chart_config(chart_id, config.clone()).unwrap();
    };
    let (mut cached, cached_id, _) = renderer(2);
    let mut full = cached.chart_config(cached_id).unwrap().clone();
    full.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 320, height: 240 });
    let budget = cached.gpu_memory_usage().total_bytes() + 64 * 1024 * 1024;
    let _ = cached.set_memory_budget(Some(budget));
    let _ = cached.set_auto_resident_working_set_limit(Some(500_000_000));
    configure(&mut cached, cached_id, &full);
    run_auto_stream_for_view_test(&mut cached, cached_id, &full, options, &bindings);
    assert_eq!(cached.view_residency_status(cached_id).unwrap().state, "resident");

    let mut narrow = full.clone();
    narrow.bottom_x.min = 0.25;
    narrow.bottom_x.max = 0.75;
    narrow.left_y.min = 0.25;
    narrow.left_y.max = 0.75;
    cached.set_chart_config(cached_id, narrow.clone()).unwrap();
    let narrow_view = cached.create_chart_view(&Chart::new(narrow.clone()), narrow.chart_area.0).unwrap();
    cached.request_auto_streaming_chart(cached_id, &narrow_view, options).unwrap();
    assert!(matches!(cached.auto_stream_chart_request_ranges(cached_id).unwrap(),
        crate::AutoStreamingRangeRequest::AllSubmitted { .. }));
    wait_stream_slots(&mut cached, 0);
    cached.prepare_registered(&[RegisteredChartDrawItem { chart_id: cached_id, view: &narrow_view }]).unwrap();
    let cached_pixels = read_draw_target(&cached, &cached.stream_target_test(cached_id).unwrap());
    let area = narrow.data_area().unwrap().0;
    let hit = pollster::block_on(cached.pick_chart_view_cache(
        cached_id,
        [area.x as f32 + area.width as f32 * 0.1,
         area.y as f32 + area.height as f32 * 0.9],
        12.0,
    )).unwrap().expect("combined packed view pick");
    assert_eq!(hit.point_index, 0);

    let (mut streamed, streamed_id, _) = renderer(2);
    let budget = streamed.gpu_memory_usage().total_bytes() + 64 * 1024 * 1024;
    let _ = streamed.set_memory_budget(Some(budget));
    let _ = streamed.set_auto_resident_working_set_limit(Some(1));
    configure(&mut streamed, streamed_id, &narrow);
    run_auto_stream_for_view_test(&mut streamed, streamed_id, &narrow, options, &bindings);
    let residency = streamed.view_residency_status(streamed_id).unwrap();
    assert_eq!(residency.state, "streamed");
    assert_eq!(residency.refusal_reason, Some("working_set_exceeded"));
    let streamed_pixels = read_draw_target(&streamed, &streamed.stream_target_test(streamed_id).unwrap());
    assert_eq!(cached_pixels, streamed_pixels);
}

fn ranges(y: &str) -> [StreamSourceRange<'_>; 2] {
    [
        StreamSourceRange {
            column: "x",
            offset: 0,
            len: 2,
        },
        StreamSourceRange {
            column: y,
            offset: 0,
            len: 2,
        },
    ]
}

fn request(r: &mut Renderer, job: StreamJob, y: &str) -> StreamTicket {
    match r.request_stream_columns(job, &ranges(y)).unwrap() {
        StreamRequestStatus::Ready(ticket) => ticket,
        StreamRequestStatus::Backpressure => panic!("unexpected backpressure"),
    }
}

fn record_upload(
    r: &mut Renderer,
    ticket: StreamTicket,
    y: &str,
) -> (wgpu::CommandBuffer, crate::streaming_upload::RecordedChunk) {
    let bytes = [0u8; 8];
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let chunk = r
        .accept_stream_columns(
            ticket,
            &[
                StreamInput {
                    column: "x",
                    bytes: &bytes,
                },
                StreamInput {
                    column: y,
                    bytes: &bytes,
                },
            ],
            &mut encoder,
        )
        .unwrap();
    (encoder.finish(), chunk)
}

fn wait_stream_retirement(r: &Renderer) {
    // Shared-device tests can dispatch callbacks on another polling thread.
    // Wait for this ledger's observation, not merely queue work completion.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while r
        .gpu_memory_usage()
        .bytes_of(crate::GpuResourceKind::StreamingUpload)
        != 0
    {
        r.service_gpu_completions().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "stream retirement callback timeout"
        );
        std::thread::yield_now();
    }
}

fn wait_stream_slots(r: &mut Renderer, expected: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        r.service_gpu_completions().unwrap();
        r.service_stream_requests();
        let slots = r.stream_request_usage().1;
        assert!(slots >= expected, "uncompleted reservation was released");
        if slots == expected {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "submission callback timeout"
        );
        std::thread::yield_now();
    }
}

#[cfg(test)]
fn draw_target(r: &Renderer, samples: u32) -> wgpu::Texture {
    r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("stream draw test target"),
        size: wgpu::Extent3d {
            width: 320,
            height: 240,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: r.surface_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | if samples == 1 {
                wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::empty()
            },
        view_formats: &[],
    })
}

fn clear_draw_target(r: &Renderer, target: &wgpu::Texture) {
    let view = target.create_view(&Default::default());
    let mut encoder = r.device.create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    r.queue.submit([encoder.finish()]);
}

#[cfg(test)]
fn read_draw_target(r: &Renderer, target: &wgpu::Texture) -> Vec<u8> {
    let resolved = draw_target(r, 1);
    let source = if target.sample_count() == 1 {
        target
    } else {
        &resolved
    };
    let mut encoder = r.device.create_command_encoder(&Default::default());
    if target.sample_count() != 1 {
        let view = target.create_view(&Default::default());
        let resolve = resolved.create_view(&Default::default());
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: Some(&resolve),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    let readback = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 1280 * 240,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: source,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1280),
                rows_per_image: Some(240),
            },
        },
        source.size(),
    );
    r.queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    slice.get_mapped_range().unwrap().to_vec()
}

fn paint_frame_pixels(r: &Renderer, frame: &PreparedFrame, samples: u32) -> Vec<u8> {
    let target = draw_target(r, samples);
    clear_draw_target(r, &target);
    let view = target.create_view(&Default::default());
    let mut encoder = r.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        r.paint_prepared(&mut pass, (320, 240), frame).unwrap();
    }
    r.queue.submit([encoder.finish()]);
    read_draw_target(r, &target)
}

#[test]
fn mixed_prepared_frame_preserves_resident_chart_and_exact_stream_output() {
    let _font_registration = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .expect("font registration test lock poisoned");
    let x = [0.08f32, 0.22, 0.4, 0.55, 0.63, 0.74, 0.82, 0.93];
    let y = [0.15f32, 0.8, 0.3, f32::NAN, 0.75, 0.2, 0.9, 0.4];
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        for samples in [1, 4] {
            let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
            let mut r = Renderer::try_new_with_sample_count(
                RendererDevice::new(device, queue),
                format,
                4096,
                samples,
            )
            .unwrap();
            r.configure_streaming_runtime(limits(2)).unwrap();
            r.register_streamed_columns(vec![source("x", 1), source("y", 1)])
                .unwrap();
            for (id, values) in [("rx", x), ("ry", y)] {
                r.add_column(
                    id,
                    &crate::Column {
                        data: values.to_vec(),
                        min: 0.08,
                        max: 0.93,
                    },
                )
                .unwrap();
            }
            for (id, value) in [("px", x[0]), ("py", y[0])] {
                r.add_column(
                    id,
                    &crate::Column {
                        data: vec![value],
                        min: 0.0,
                        max: 1.0,
                    },
                )
                .unwrap();
            }
            let panel_a = Rect {
                x: 0,
                y: 0,
                width: 160,
                height: 240,
            };
            let panel_b = Rect {
                x: 160,
                y: 0,
                width: 160,
                height: 240,
            };
            let chart_for = |panel| {
                let mut config = crate::default::default_config();
                config.chart_area = crate::layout::ChartArea(panel);
                let mut chart = Chart::new(config);
                chart.set_x_range(0.0, 1.0);
                chart.set_y_range(0.0, 1.0);
                chart
            };
            let chart_a = chart_for(panel_a);
            let chart_b = chart_for(panel_b);
            let mut stream_series = declaration("a", "x", "y");
            stream_series.render_type = DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_width: 4.0,
                    line_color: Color::new(1.0, 0.0, 0.0, 0.6),
                },
            };
            let mut resident_a = stream_series.clone();
            resident_a.x_column = "rx".into();
            resident_a.y_column = "ry".into();
            let mut prefix_a = resident_a.clone();
            prefix_a.x_column = "px".into();
            prefix_a.y_column = "py".into();
            let mut resident_b = declaration("b", "rx", "ry");
            resident_b.render_type = DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: Color::new(0.0, 0.3, 1.0, 0.8),
                    point_shape: ScatterShape::CircleFilled,
                    point_size: 7.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                },
            };
            let a_stream = r
                .register_chart(chart_a.config().clone(), vec![stream_series])
                .unwrap();
            let a_reference = r
                .register_chart(chart_a.config().clone(), vec![resident_a])
                .unwrap();
            let a_prefix = r
                .register_chart(chart_a.config().clone(), vec![prefix_a])
                .unwrap();
            let b = r
                .register_chart(chart_b.config().clone(), vec![resident_b])
                .unwrap();
            let view_a = r.create_chart_view(&chart_a, panel_a).unwrap();
            let view_a_prefix = r.create_chart_view(&chart_a, panel_a).unwrap();
            let view_b = r.create_chart_view(&chart_b, panel_b).unwrap();
            let reference = r
                .prepare_registered(&[
                    RegisteredChartDrawItem {
                        chart_id: a_reference,
                        view: &view_a,
                    },
                    RegisteredChartDrawItem {
                        chart_id: b,
                        view: &view_b,
                    },
                ])
                .unwrap();
            let expected = paint_frame_pixels(&r, &reference, samples);
            let mut b_only = r
                .prepare_registered(&[RegisteredChartDrawItem {
                    chart_id: b,
                    view: &view_b,
                }])
                .unwrap();
            let job = r
                .begin_chart_stream_surface(a_stream, &view_a, (320, 240), wgpu::Color::WHITE, 2)
                .unwrap();
            let b_revision = view_b.content_revision.load(Ordering::Acquire);
            assert!(matches!(
                r.prepare_registered(&[
                    RegisteredChartDrawItem {
                        chart_id: a_stream,
                        view: &view_a_prefix,
                    },
                    RegisteredChartDrawItem {
                        chart_id: b,
                        view: &view_b,
                    },
                ]),
                Err(FiggyError::StaleStateToken { .. })
            ));
            assert_eq!(view_b.content_revision.load(Ordering::Acquire), b_revision);
            assert!(r.validate_prepared(&b_only).is_ok());
            let mut submitted = 0;
            let mut prior_a_frame: Option<PreparedFrame> = None;
            loop {
                wait_stream_slots(&mut r, 0);
                let ticket = match r.request_chart_stream_draw(job).unwrap() {
                    StreamDrawRequestStatus::Ready(ticket) => ticket,
                    StreamDrawRequestStatus::AllSubmitted => break,
                    StreamDrawRequestStatus::Backpressure => panic!("test waits after each chunk"),
                };
                let columns: Vec<_> = r
                    .stream_request_columns(ticket)
                    .unwrap()
                    .iter()
                    .map(|column| (column.column.clone(), column.range))
                    .collect();
                let inputs: Vec<_> = columns
                    .iter()
                    .map(|(id, range)| StreamInput {
                        column: id,
                        bytes: bytemuck::cast_slice(
                            &(if id == "x" { &x } else { &y })
                                [range.offset as usize..(range.offset + range.len) as usize],
                        ),
                    })
                    .collect();
                r.submit_chart_stream_surface(ticket, &inputs, &view_a)
                    .unwrap();
                submitted += 1;
                if submitted == 1 {
                    let unknown = ChartId {
                        renderer_identity: r.renderer_identity,
                        sequence: u64::MAX,
                    };
                    assert!(matches!(
                        r.prepare_registered(&[
                            RegisteredChartDrawItem {
                                chart_id: a_stream,
                                view: &view_a,
                            },
                            RegisteredChartDrawItem {
                                chart_id: unknown,
                                view: &view_b,
                            },
                        ]),
                        Err(FiggyError::UnknownChart { .. })
                    ));
                    assert!(matches!(
                        r.chart_stream_display(job),
                        Err(StreamRequestError::Scheduler(StreamError::WrongState))
                    ));
                }
                let prefix_len = (submitted * 2 + 1).min(x.len());
                for (id, values) in [("px", &x[..prefix_len]), ("py", &y[..prefix_len])] {
                    r.upsert_column(
                        id,
                        &crate::Column {
                            data: values.to_vec(),
                            min: 0.0,
                            max: 1.0,
                        },
                    )
                    .unwrap();
                }
                let direct_prefix = r
                    .prepare_registered(&[RegisteredChartDrawItem {
                        chart_id: a_prefix,
                        view: &view_a_prefix,
                    }])
                    .unwrap();
                let prefix_pixels = paint_frame_pixels(&r, &direct_prefix, samples);
                if let Some(frame) = &prior_a_frame {
                    assert!(matches!(
                        r.validate_prepared(frame),
                        Err(FiggyError::StalePreparedFrame { .. })
                    ));
                }
                assert!(
                    r.validate_prepared(&b_only).is_ok(),
                    "B token: {:?}",
                    r.validate_prepared(&b_only)
                );
                let mixed = r
                    .prepare_registered(&[
                        RegisteredChartDrawItem {
                            chart_id: a_stream,
                            view: &view_a,
                        },
                        RegisteredChartDrawItem {
                            chart_id: b,
                            view: &view_b,
                        },
                    ])
                    .unwrap();
                let pixels = paint_frame_pixels(&r, &mixed, samples);
                for row in 0..240 {
                    let a_base = row * 1280;
                    assert_eq!(
                        &pixels[a_base..a_base + 160 * 4],
                        &prefix_pixels[a_base..a_base + 160 * 4],
                        "stream partial does not match direct prefix: samples={samples}, row={row}, prefix={prefix_len}"
                    );
                    let base = row * 1280 + 160 * 4;
                    assert_eq!(
                        &pixels[base..base + 160 * 4],
                        &expected[base..base + 160 * 4],
                        "resident chart changed: samples={samples}, row={row}"
                    );
                }
                if submitted == 1 {
                    assert_ne!(pixels, expected);
                }
                r.end_gpu_frame();
                b_only = r
                    .prepare_registered(&[RegisteredChartDrawItem {
                        chart_id: b,
                        view: &view_b,
                    }])
                    .unwrap();
                prior_a_frame = Some(
                    r.prepare_registered(&[RegisteredChartDrawItem {
                        chart_id: a_stream,
                        view: &view_a,
                    }])
                    .unwrap(),
                );
            }
            let final_frame = r
                .prepare_registered(&[
                    RegisteredChartDrawItem {
                        chart_id: a_stream,
                        view: &view_a,
                    },
                    RegisteredChartDrawItem {
                        chart_id: b,
                        view: &view_b,
                    },
                ])
                .unwrap();
            assert_eq!(paint_frame_pixels(&r, &final_frame, samples), expected);
            b_only = r
                .prepare_registered(&[RegisteredChartDrawItem {
                    chart_id: b,
                    view: &view_b,
                }])
                .unwrap();
            let a_only = r
                .prepare_registered(&[RegisteredChartDrawItem {
                    chart_id: a_stream,
                    view: &view_a,
                }])
                .unwrap();
            r.cancel_chart_stream(a_stream).unwrap();
            assert!(matches!(
                r.validate_prepared(&a_only),
                Err(FiggyError::StalePreparedFrame { .. })
            ));
            assert!(r.validate_prepared(&b_only).is_ok());
        }
    }
}

#[test]
fn ordered_stream_draw_matches_resident_pixels_across_chunk_boundaries() {
    let _font_registration = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .expect("font registration test lock poisoned");
    let x = [0.1f32, 0.4, 0.8, 0.2, 0.7, 0.9, 0.3, 0.6];
    let y = [0.1f32, 0.8, 0.2, f32::NAN, 0.7, 0.2, 0.9, 0.4];
    let ex_lo = [0.05f32, 0.08, 0.12, 0.05, 0.11, 0.07, 0.08, 0.06];
    let ex_hi = [0.09f32, 0.05, 0.07, 0.08, 0.05, 0.11, 0.07, 0.10];
    let ey_lo = [0.08f32, 0.09, 0.06, 0.11, 0.05, 0.07, 0.09, 0.06];
    let ey_hi = [0.06f32, 0.04, 0.08, 0.06, 0.12, 0.09, 0.05, 0.10];
    for samples in [1, 4] {
        for mode in 0..9 {
            let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
            let mut r = Renderer::try_new_with_sample_count(
                RendererDevice::new(device, queue),
                wgpu::TextureFormat::Rgba8Unorm,
                4096,
                samples,
            )
            .unwrap();
            r.configure_streaming_runtime(limits(2)).unwrap();
            r.register_streamed_columns(
                ["x", "y", "ex_lo", "ex_hi", "ey_lo", "ey_hi"]
                    .map(|id| {
                        let mut source = source(id, 1);
                        if mode == 8 && id == "ey_hi" {
                            source.len = 5;
                        }
                        source
                    })
                    .to_vec(),
            )
            .unwrap();
            for (id, values) in [
                ("rx", &x[..]),
                ("ry", &y[..]),
                ("rex_lo", &ex_lo[..]),
                ("rex_hi", &ex_hi[..]),
                ("rey_lo", &ey_lo[..]),
                ("rey_hi", &ey_hi[..if mode == 8 { 5 } else { 8 }]),
            ] {
                r.add_column(
                    id,
                    &crate::Column {
                        data: values.to_vec(),
                        min: 0.1,
                        max: 0.9,
                    },
                )
                .unwrap();
            }
            let mut config = crate::default::default_config();
            config.chart_area = crate::layout::ChartArea(Rect {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            });
            let mut chart = Chart::new(config);
            chart.set_x_range(0.0, 1.0);
            chart.set_y_range(0.0, 1.0);
            let mut declarations = Vec::new();
            for (id, color) in [
                ("red", Color::new(1.0, 0.0, 0.0, 0.4)),
                ("blue", Color::new(0.0, 0.0, 1.0, 0.6)),
            ] {
                let mut s = declaration(id, "x", "y");
                let line = DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_width: 5.0,
                    line_color: color,
                };
                let scatter = DataScatterStyleConfig {
                    point_color: if id == "red" {
                        Color::new(0.0, 0.8, 0.0, 0.5)
                    } else {
                        Color::new(1.0, 0.7, 0.0, 0.5)
                    },
                    point_shape: ScatterShape::CircleFilled,
                    point_size: 9.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                };
                let err_style = DataErrorBarStyleConfig {
                    error_bar_color: Color::new(0.1, 0.1, 0.1, 0.6),
                    error_bar_width: 3.0,
                    error_bar_cap_size: 9.0,
                    cap_width: 3.0,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                };
                let err_x_sym = ErrorRef::Symmetric {
                    column: "ex_lo".into(),
                };
                let err_y_sym = ErrorRef::Symmetric {
                    column: "ey_lo".into(),
                };
                let err_x_asym = ErrorRef::Asymmetric {
                    lower: "ex_lo".into(),
                    upper: "ex_hi".into(),
                };
                let err_y_asym = ErrorRef::Asymmetric {
                    lower: "ey_lo".into(),
                    upper: "ey_hi".into(),
                };
                s.render_type = match mode {
                    0 => DataRenderType::Line { line },
                    1 => DataRenderType::Scatter { scatter },
                    2 => DataRenderType::ScatterLine { line, scatter },
                    3 => DataRenderType::ScatterErrorbarX {
                        scatter,
                        err_x: err_x_sym,
                        err_style,
                    },
                    4 => DataRenderType::ScatterErrorbarY {
                        scatter,
                        err_y: err_y_asym,
                        err_style,
                    },
                    5 => DataRenderType::ScatterErrorbarXY {
                        scatter,
                        err_x: err_x_sym,
                        err_y: err_y_asym,
                        err_style,
                    },
                    6 => DataRenderType::LineScatterErrorbarX {
                        line,
                        scatter,
                        err_x: err_x_asym,
                        err_style,
                    },
                    7 => DataRenderType::LineScatterErrorbarY {
                        line,
                        scatter,
                        err_y: err_y_sym,
                        err_style,
                    },
                    8 => DataRenderType::LineScatterErrorbarXY {
                        line,
                        scatter,
                        err_x: err_x_asym,
                        err_y: err_y_asym,
                        err_style,
                    },
                    _ => unreachable!(),
                };
                declarations.push(s);
            }
            let mut resident = declarations.clone();
            for s in &mut resident {
                s.x_column = "rx".into();
                s.y_column = "ry".into();
                match &mut s.render_type {
                    DataRenderType::ScatterErrorbarX { err_x, .. }
                    | DataRenderType::LineScatterErrorbarX { err_x, .. } => {
                        prefix_error_column(err_x)
                    }
                    DataRenderType::ScatterErrorbarY { err_y, .. }
                    | DataRenderType::LineScatterErrorbarY { err_y, .. } => {
                        prefix_error_column(err_y)
                    }
                    DataRenderType::ScatterErrorbarXY { err_x, err_y, .. }
                    | DataRenderType::LineScatterErrorbarXY { err_x, err_y, .. } => {
                        prefix_error_column(err_x);
                        prefix_error_column(err_y);
                    }
                    DataRenderType::Scatter { .. }
                    | DataRenderType::Line { .. }
                    | DataRenderType::ScatterLine { .. }
                    | DataRenderType::Histogram { .. }
                    | DataRenderType::Heatmap { .. }
                    | DataRenderType::Contour { .. }
                    | DataRenderType::HeatmapContour { .. } => {}
                }
            }
            let resident_id = r.register_chart(chart.config().clone(), resident).unwrap();
            let stream_id = r
                .register_chart(chart.config().clone(), declarations)
                .unwrap();
            let view = r
                .create_chart_view(&chart, chart.config().chart_area.0)
                .unwrap();
            let frame = r
                .prepare_registered(&[RegisteredChartDrawItem {
                    chart_id: resident_id,
                    view: &view,
                }])
                .unwrap();
            let reference = draw_target(&r, samples);
            clear_draw_target(&r, &reference);
            let target_view = reference.create_view(&Default::default());
            let mut encoder = r.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_viewport(0.0, 0.0, 320.0, 240.0, 0.0, 1.0);
                let data = chart.config().data_area().unwrap().0;
                pass.set_scissor_rect(data.x, data.y, data.width, data.height);
                for series in &frame.items[0].series {
                    data_render::issue_series_data(&mut pass, &series.layers());
                }
            }
            r.queue.submit([encoder.finish()]);
            let expected = read_draw_target(&r, &reference);
            assert!(expected.chunks_exact(4).any(|p| p != [255, 255, 255, 255]));
            for chunk_size in [1, 2, 4] {
                let target = draw_target(&r, samples);
                clear_draw_target(&r, &target);
                let upload_creations_before = r.gpu_memory_usage()
                    .creations_of(crate::GpuResourceKind::StreamingUpload);
                let job = r
                    .begin_chart_stream_draw(stream_id, &view, &target, chunk_size)
                    .unwrap();
                let mut submitted = 0;
                let mut prior_offset = None;
                let mut rewinds = 0;
                let mut phase_retry_checks = 0;
                loop {
                    r.service_stream_requests();
                    let ticket = match r.request_chart_stream_draw(job).unwrap() {
                        StreamDrawRequestStatus::Ready(t) => t,
                        StreamDrawRequestStatus::AllSubmitted => break,
                        StreamDrawRequestStatus::Backpressure => {
                            panic!("test waits after each chunk")
                        }
                    };
                    let columns: Vec<_> = r
                        .stream_request_columns(ticket)
                        .unwrap()
                        .iter()
                        .map(|c| (c.column.clone(), c.range))
                        .collect();
                    let offset = columns[0].1.offset;
                    if prior_offset.is_some_and(|prior| offset < prior) {
                        rewinds += 1;
                    }
                    prior_offset = Some(offset);
                    let errorbar_phase = mode >= 3
                        && if mode >= 6 {
                            rewinds % 3 == 0
                        } else {
                            rewinds % 2 == 0
                        };
                    let expected_columns = if errorbar_phase {
                        match mode {
                            3 | 7 => 3,
                            4 | 6 => 4,
                            5 => 5,
                            8 => 6,
                            _ => unreachable!(),
                        }
                    } else {
                        2
                    };
                    assert_eq!(columns.len(), expected_columns);
                    assert!(columns
                        .iter()
                        .enumerate()
                        .all(|(index, (id, range))| {
                            range.column == index as u64
                                && !columns[..index].iter().any(|(earlier, _)| earlier == id)
                        }));
                    if mode >= 2 && rewinds == 1 && offset == 0 {
                        phase_retry_checks += 1;
                        // A rejected scatter-phase upload must not consume the
                        // replay ticket or advance the CPU execution cursor.
                        assert_eq!(
                            r.request_chart_stream_draw(job).unwrap(),
                            StreamDrawRequestStatus::Ready(ticket)
                        );
                        let before = r.gpu_memory_usage();
                        assert!(r
                            .submit_chart_stream_draw(ticket, &[], &view, &target)
                            .is_err());
                        assert_eq!(r.gpu_memory_usage(), before);
                        assert_eq!(
                            r.request_chart_stream_draw(job).unwrap(),
                            StreamDrawRequestStatus::Ready(ticket)
                        );
                    }
                    let inputs: Vec<_> = columns
                        .iter()
                        .map(|(id, range)| {
                            let data: &[f32] = match id.as_str() {
                                "x" => &x,
                                "y" => &y,
                                "ex_lo" => &ex_lo,
                                "ex_hi" => &ex_hi,
                                "ey_lo" => &ey_lo,
                                "ey_hi" => &ey_hi,
                                _ => unreachable!(),
                            };
                            StreamInput {
                                column: id,
                                bytes: bytemuck::cast_slice(
                                    &data[range.offset as usize
                                        ..(range.offset + range.len) as usize],
                                ),
                            }
                        })
                        .collect();
                    let index = r
                        .submit_chart_stream_draw(ticket, &inputs, &view, &target)
                        .unwrap();
                    r.device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(index),
                            timeout: Some(std::time::Duration::from_secs(30)),
                        })
                        .unwrap();
                    r.end_gpu_frame();
                    submitted += 1;
                    if submitted == 1 {
                        let partial = read_draw_target(&r, &target);
                        assert!(partial.chunks_exact(4).any(|p| p != [255, 255, 255, 255]));
                        assert_ne!(partial, expected);
                    }
                }
                assert_eq!(
                    read_draw_target(&r, &target),
                    expected,
                    "samples={samples}, mode={mode}, chunk={chunk_size}"
                );
                assert_eq!(rewinds, if mode >= 6 { 5 } else if mode >= 2 { 3 } else { 1 });
                assert_eq!(phase_retry_checks, if mode >= 2 { 1 } else { 0 });
                if mode == 0 && chunk_size == 1 {
                    let upload_creations = r.gpu_memory_usage()
                        .creations_of(crate::GpuResourceKind::StreamingUpload)
                        - upload_creations_before;
                    assert_eq!(
                        upload_creations,
                        u64::try_from(submitted).unwrap() + 1,
                        "each submitted chunk gets staging, but the draw work buffer is allocated once",
                    );
                    assert_eq!(
                        r.gpu_memory_usage()
                            .live_bytes_of(crate::GpuResourceKind::StreamingUpload),
                        0,
                        "completed stream must retain its image, not its raw chunk buffer",
                    );
                }
                assert!(matches!(
                    r.request_chart_stream_draw(job).unwrap(),
                    StreamDrawRequestStatus::AllSubmitted
                ));
                assert_eq!(read_draw_target(&r, &target), expected);
                r.cancel_chart_stream(stream_id).unwrap();
            }

            // Include actual grid and decoration, not just data-only parity.
            let frame = r
                .prepare_registered(&[RegisteredChartDrawItem {
                    chart_id: resident_id,
                    view: &view,
                }])
                .unwrap();
            let reference = draw_target(&r, samples);
            clear_draw_target(&r, &reference);
            let target_view = reference.create_view(&Default::default());
            let mut encoder = r.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                r.paint_prepared(&mut pass, (320, 240), &frame).unwrap();
            }
            r.queue.submit([encoder.finish()]);
            let complete = read_draw_target(&r, &reference);
            let job = r
                .begin_chart_stream_surface(stream_id, &view, (320, 240), wgpu::Color::WHITE, 2)
                .unwrap();
            assert!(r.chart_stream_display(job).is_err());
            assert!(r.refresh_chart_stream_display(job, &view).unwrap());
            assert!(!r.refresh_chart_stream_display(job, &view).unwrap());
            let initial = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
            assert_ne!(read_draw_target(&r, &initial), complete);
            loop {
                wait_stream_slots(&mut r, 0);
                let ticket = match r.request_chart_stream_draw(job).unwrap() {
                    StreamDrawRequestStatus::Ready(t) => t,
                    StreamDrawRequestStatus::AllSubmitted => break,
                    StreamDrawRequestStatus::Backpressure => panic!("slots drained"),
                };
                let columns: Vec<_> = r
                    .stream_request_columns(ticket)
                    .unwrap()
                    .iter()
                    .map(|c| (c.column.clone(), c.range))
                    .collect();
                let inputs: Vec<_> = columns
                    .iter()
                    .map(|(id, range)| {
                        let data: &[f32] = match id.as_str() {
                            "x" => &x,
                            "y" => &y,
                            "ex_lo" => &ex_lo,
                            "ex_hi" => &ex_hi,
                            "ey_lo" => &ey_lo,
                            "ey_hi" => &ey_hi,
                            _ => unreachable!(),
                        };
                        StreamInput {
                            column: id,
                            bytes: bytemuck::cast_slice(
                                &data[range.offset as usize..(range.offset + range.len) as usize],
                            ),
                        }
                    })
                    .collect();
                r.submit_chart_stream_surface(ticket, &inputs, &view)
                    .unwrap();
                assert!(r.refresh_chart_stream_display(job, &view).unwrap());
                assert!(!r.refresh_chart_stream_display(job, &view).unwrap());
                r.end_gpu_frame();
            }
            let display = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
            assert_eq!(
                read_draw_target(&r, &display),
                complete,
                "full chart: samples={samples}, mode={mode}"
            );
            let before = r.gpu_memory_usage();
            assert!(!r.refresh_chart_stream_display(job, &view).unwrap());
            assert_eq!(r.gpu_memory_usage(), before);
            drop((initial, display));
            r.cancel_chart_stream(stream_id).unwrap();
            assert!(r.chart_stream_display(job).is_err());
        }
    }
}

#[test]
fn renderer_queue_completion_reclaims_only_completed_ticket() {
    let (mut r, a, b) = renderer(2);
    let ja = r.begin_chart_stream(a).unwrap();
    let jb = r.begin_chart_stream(b).unwrap();
    let ta = request(&mut r, ja, "a");
    let tb = request(&mut r, jb, "b");
    let (commands_a, chunk_a) = record_upload(&mut r, ta, "a");
    let (commands_b, chunk_b) = record_upload(&mut r, tb, "b");
    let index_a = r.queue_stream_recording(ta, commands_a).unwrap();
    r.cancel_chart_stream(a).unwrap();
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index_a),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    r.service_stream_requests();
    // B is still recorded, not submitted. A's callback cannot reclaim it.
    wait_stream_slots(&mut r, 1);
    assert_eq!(r.stream_request_usage(), (1, 1, 64));
    r.service_stream_requests();
    assert_eq!(r.stream_request_usage(), (1, 1, 64));
    let index_b = r.queue_stream_recording(tb, commands_b).unwrap();
    r.cancel_chart_stream(b).unwrap();
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index_b),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    r.service_stream_requests();
    wait_stream_slots(&mut r, 0);
    assert_eq!(r.stream_request_usage(), (0, 0, 0));
    // Admission completion is distinct from dropping retained GPU owners.
    assert!(
        r.gpu_memory_usage()
            .bytes_of(crate::GpuResourceKind::StreamingUpload)
            > 0
    );
    drop((chunk_a, chunk_b));
    r.end_gpu_frame();
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    wait_stream_retirement(&r);
}

#[test]
fn owned_surface_budget_failure_preserves_old_display_and_cancel_retires_targets() {
    let (mut r, a, _) = renderer(2);
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let chart = Chart::new(config);
    r.set_chart_config(a, chart.config().clone()).unwrap();
    let view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let job = r
        .begin_chart_stream_surface(a, &view, (320, 240), wgpu::Color::WHITE, 2)
        .unwrap();
    r.refresh_chart_stream_display(job, &view).unwrap();
    let before = r.gpu_memory_usage();
    assert!(r.set_memory_budget(Some(before.total_bytes())).is_none());
    assert!(matches!(
        r.begin_chart_stream_surface(a, &view, (320, 240), wgpu::Color::WHITE, 2),
        Err(StreamRequestError::Surface(
            super::streaming_surface::StreamSurfaceError::TooLarge
        ))
    ));
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(r.chart_stream_display(job).is_ok());
    assert!(!r.refresh_chart_stream_display(job, &view).unwrap());
    r.cancel_chart_stream(a).unwrap();
    let after = r.gpu_memory_usage();
    let bytes = 320 * 240 * 4 * 2;
    assert_eq!(
        before.live_bytes_of(crate::GpuResourceKind::PanelTexture)
            - after.live_bytes_of(crate::GpuResourceKind::PanelTexture),
        bytes
    );
    assert_eq!(
        after.retired_bytes_of(crate::GpuResourceKind::PanelTexture)
            - before.retired_bytes_of(crate::GpuResourceKind::PanelTexture),
        bytes
    );
    assert_eq!(before.total_bytes(), after.total_bytes());
    r.end_gpu_frame();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while r
        .gpu_memory_usage()
        .retired_bytes_of(crate::GpuResourceKind::PanelTexture)
        > 0
    {
        r.service_gpu_completions().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn stream_draw_rejects_wrong_target_and_changed_view_before_upload() {
    let (mut r, a, _) = renderer(2);
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    let chart = Chart::new(config);
    r.set_chart_config(a, chart.config().clone()).unwrap();
    let view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let target = draw_target(&r, 1);
    let wrong_target = draw_target(&r, 1);
    clear_draw_target(&r, &target);
    let job = r.begin_chart_stream_draw(a, &view, &target, 1).unwrap();
    let ticket = match r.request_chart_stream_draw(job).unwrap() {
        StreamDrawRequestStatus::Ready(t) => t,
        _ => panic!("expected draw request"),
    };
    let bytes = [0u8; 8];
    let inputs = [
        StreamInput {
            column: "x",
            bytes: &bytes,
        },
        StreamInput {
            column: "a",
            bytes: &bytes,
        },
    ];
    let before = r.gpu_memory_usage();
    assert!(
        r.submit_chart_stream_draw(ticket, &inputs, &view, &wrong_target)
            .is_err()
    );
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(r.stream_request_columns(ticket).is_ok());
    assert!(
        r.submit_chart_stream_draw(ticket, &[], &view, &target)
            .is_err()
    );
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(r.stream_request_columns(ticket).is_ok());
    r.submit_chart_stream_draw(ticket, &inputs, &view, &target)
        .unwrap();
    assert!(
        r.submit_chart_stream_draw(ticket, &inputs, &view, &target)
            .is_err()
    );
    view.advance_stream_revision().unwrap();
    view.advance_content_revision().unwrap();
    let after = r.gpu_memory_usage();
    assert!(r.request_chart_stream_draw(job).is_err());
    let cancelled = r.gpu_memory_usage();
    // Invalidating the view cancels this draw. Its reusable upload buffer is
    // no longer live, but may remain charged as retired until GPU completion.
    assert_eq!(cancelled.total_creations(), after.total_creations());
    assert!(cancelled.total_bytes() <= after.total_bytes());
    assert_eq!(
        cancelled.live_bytes_of(crate::GpuResourceKind::StreamingUpload),
        0
    );
}

#[test]
fn post_recording_reservation_failure_reissues_same_cursor_range() {
    let _font_registration = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .expect("font registration test lock poisoned");
    let (mut renderer, chart, _) = renderer(2);
    let config = renderer.chart_config(chart).unwrap().clone();
    let view = renderer
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let target = draw_target(&renderer, 1);
    clear_draw_target(&renderer, &target);
    let before = read_draw_target(&renderer, &target);
    let job = renderer
        .begin_chart_stream_draw(chart, &view, &target, 2)
        .unwrap();
    let old_ticket = match renderer.request_chart_stream_draw(job).unwrap() {
        StreamDrawRequestStatus::Ready(ticket) => ticket,
        other => panic!("expected first ticket, got {other:?}"),
    };
    let old_columns: Vec<_> = renderer
        .stream_request_columns(old_ticket)
        .unwrap()
        .iter()
        .map(|column| (column.column.clone(), column.range))
        .collect();
    let x = [0.1f32, 0.3, 0.5, 0.7, 0.9, 0.2, 0.4, 0.6];
    let y = [0.1f32, 0.9, 0.2, 0.8, 0.3, 0.7, 0.4, 0.6];
    fn supply<'a>(
        columns: &'a [(String, ColumnRange)],
        x: &'a [f32],
        y: &'a [f32],
    ) -> Vec<StreamInput<'a>> {
        columns
            .iter()
            .map(|(id, range)| {
                let data = if id == "x" { x } else { y };
                StreamInput {
                    column: id,
                    bytes: bytemuck::cast_slice(
                        &data[range.offset as usize..(range.offset + range.len) as usize],
                    ),
                }
            })
            .collect()
    }
    renderer.reject_next_stream_completion_reserve_for_test();
    assert!(matches!(
        renderer.submit_chart_stream_draw(old_ticket, &supply(&old_columns, &x, &y), &view, &target),
        Err(StreamRequestError::Scheduler(StreamError::AllocationFailed))
    ));
    assert_eq!(read_draw_target(&renderer, &target), before);
    assert!(renderer.stream_request_columns(old_ticket).is_err());
    assert_eq!(renderer.stream_request_usage().1, 0);

    let new_ticket = match renderer.request_chart_stream_draw(job).unwrap() {
        StreamDrawRequestStatus::Ready(ticket) => ticket,
        other => panic!("expected replay ticket, got {other:?}"),
    };
    assert_ne!(new_ticket, old_ticket);
    let new_columns: Vec<_> = renderer
        .stream_request_columns(new_ticket)
        .unwrap()
        .iter()
        .map(|column| (column.column.clone(), column.range))
        .collect();
    assert_eq!(new_columns, old_columns);
    let submission = renderer
        .submit_chart_stream_draw(new_ticket, &supply(&new_columns, &x, &y), &view, &target)
        .unwrap();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    renderer.end_gpu_frame();
    assert_ne!(read_draw_target(&renderer, &target), before);
}

#[test]
fn renderer_queue_rejects_cancelled_recording_without_releasing_its_reservation() {
    let (mut r, a, _) = renderer(1);
    let job = r.begin_chart_stream(a).unwrap();
    let ticket = request(&mut r, job, "a");
    let (commands, chunk) = record_upload(&mut r, ticket, "a");
    r.cancel_chart_stream(a).unwrap();
    assert!(r.queue_stream_recording(ticket, commands).is_err());
    assert_eq!(r.stream_request_usage(), (0, 1, 64));
    r.service_stream_requests();
    assert_eq!(r.stream_request_usage(), (0, 1, 64));
    drop(chunk);
    r.discard_stream_recording(ticket).unwrap();
    assert_eq!(r.stream_request_usage(), (0, 0, 0));
}

#[test]
fn idle_poll_and_request_service_restore_submission_capacity() {
    let (mut r, a, _) = renderer(1);
    let job = r.begin_chart_stream(a).unwrap();
    // Reuse one bounded slot repeatedly, without manually completing receipts.
    for _ in 0..4 {
        let ticket = request(&mut r, job, "a");
        let (commands, chunk) = record_upload(&mut r, ticket, "a");
        r.queue_stream_recording(ticket, commands).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            r.service_gpu_completions().unwrap();
            r.service_stream_requests();
            if r.stream_request_usage().1 == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "GPU completion timeout"
            );
            std::thread::yield_now();
        }
        assert_eq!(r.stream_request_usage(), (1, 0, 0));
        drop(chunk);
        r.end_gpu_frame();
    }
}

#[test]
fn pending_requests_are_chart_scoped_and_consume_shared_capacity() {
    let (mut r, a, b) = renderer(2);
    let before = r.gpu_memory_usage();
    let ja = r.begin_chart_stream(a).unwrap();
    let jb = r.begin_chart_stream(b).unwrap();
    let ta = request(&mut r, ja, "a");
    let tb = request(&mut r, jb, "b");
    assert!(matches!(
        r.request_stream_columns(jb, &ranges("b")).unwrap(),
        StreamRequestStatus::Backpressure
    ));
    assert_eq!(r.stream_request_usage(), (2, 2, 128));
    assert_eq!(
        r.gpu_memory_usage(),
        before,
        "request metadata must not upload source values"
    );
    r.cancel_chart_stream(a).unwrap();
    assert!(r.stream_request_columns(ta).is_err());
    assert_eq!(r.stream_request_columns(tb).unwrap()[1].column, "b");
    assert_eq!(r.stream_request_usage(), (1, 1, 64));
    request(&mut r, jb, "b");
}

#[test]
fn source_and_chart_changes_reject_old_supply_before_gpu_allocation() {
    let (mut r, a, b) = renderer(4);
    let ja = r.begin_chart_stream(a).unwrap();
    let jb = r.begin_chart_stream(b).unwrap();
    let ta = request(&mut r, ja, "a");
    let tb = request(&mut r, jb, "b");
    r.replace_streamed_columns(vec![source("a", 2)]).unwrap();
    let before = r.gpu_memory_usage();
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let bytes = [0u8; 8];
    assert!(
        r.accept_stream_columns(
            ta,
            &[
                StreamInput {
                    column: "x",
                    bytes: &bytes
                },
                StreamInput {
                    column: "a",
                    bytes: &bytes
                }
            ],
            &mut encoder
        )
        .is_err()
    );
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(r.stream_request_columns(tb).is_ok());
    let ja = r.begin_chart_stream(a).unwrap();
    let ta = request(&mut r, ja, "a");
    let mut view = r.chart_view_state(a).unwrap();
    view.bottom_x.min -= 1.0;
    r.set_chart_view_state(a, view).unwrap();
    assert!(r.stream_request_columns(ta).is_err());
    assert!(r.stream_request_columns(tb).is_ok());
}

#[test]
fn decoration_only_config_change_preserves_stream_request() {
    let (mut r, a, _) = renderer(2);
    let job = r.begin_chart_stream(a).unwrap();
    let ticket = request(&mut r, job, "a");
    let before = r.chart_states[&a].revisions;

    let mut config = r.chart_config(a).unwrap().clone();
    config.chart_title.text.segments = crate::text::rich_segments_from_text("updated title");
    config.bottom_x.label_style.color = Color::new(0.2, 0.8, 0.4, 1.0);
    r.set_chart_config(a, config).unwrap();

    let after = r.chart_states[&a].revisions;
    assert_ne!(after.desired, before.desired);
    assert_ne!(after.raster, before.raster);
    assert_eq!(after.view, before.view, "decoration must not change stream geometry");
    assert!(
        r.stream_request_columns(ticket).is_ok(),
        "decoration-only config changes must not cancel an admitted stream range"
    );
}

#[test]
fn decoration_refresh_recomposes_display_without_resetting_stream_cursor() {
    let (mut r, a, _) = renderer(2);
    let mut config = r.chart_config(a).unwrap().clone();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 320,
        height: 240,
    });
    r.set_chart_config(a, config.clone()).unwrap();
    let mut chart = Chart::new(config);
    let panel = chart.config().chart_area.0;
    let mut view = r.create_chart_view(&chart, panel).unwrap();
    let job = r
        .begin_chart_stream_surface(a, &view, (320, 240), wgpu::Color::WHITE, 2)
        .unwrap();
    assert!(r.refresh_chart_stream_display(job, &view).unwrap());

    let ticket = match r.request_chart_stream_draw(job).unwrap() {
        StreamDrawRequestStatus::Ready(ticket) => ticket,
        status => panic!("expected first stream draw request, got {status:?}"),
    };
    let columns: Vec<_> = r
        .stream_request_columns(ticket)
        .unwrap()
        .iter()
        .map(|column| (column.column.clone(), column.range))
        .collect();
    let values = [0.0f32, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25];
    let inputs: Vec<_> = columns
        .iter()
        .map(|(id, range)| StreamInput {
            column: id,
            bytes: bytemuck::cast_slice(
                &values[range.offset as usize..(range.offset + range.len) as usize],
            ),
        })
        .collect();
    r.submit_chart_stream_surface(ticket, &inputs, &view).unwrap();
    wait_stream_slots(&mut r, 0);
    assert!(r.refresh_chart_stream_display(job, &view).unwrap());

    let submitted_before = 2;
    let prefix = r.chart_stream_prefix_for_test(job).unwrap();
    let prefix_before = read_draw_target(&r, &prefix);
    let display_before = {
        let display = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
        read_draw_target(&r, &display)
    };

    chart.with_decoration_change(|config| {
        config.chart_title.text.segments =
            crate::text::rich_segments_from_text("stream continues");
        config.bottom_x.label_style.color = Color::new(0.1, 0.7, 0.3, 1.0);
    });
    r.set_chart_config(a, chart.config().clone()).unwrap();
    r.refresh_axis(&mut view, &chart, panel).unwrap();

    assert!(
        r.refresh_chart_stream_display(job, &view).unwrap(),
        "a new decoration raster must recompose the display"
    );
    assert_eq!(
        read_draw_target(&r, &prefix),
        prefix_before,
        "decoration refresh must not rewrite the accumulated data prefix"
    );
    let display_after = {
        let display = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
        read_draw_target(&r, &display)
    };
    assert_ne!(display_after, display_before, "new decoration must reach the display");

    let next = match r.request_chart_stream_draw(job).unwrap() {
        StreamDrawRequestStatus::Ready(ticket) => ticket,
        status => panic!("stream cursor was reset by decoration: {status:?}"),
    };
    assert!(
        r.stream_request_columns(next)
            .unwrap()
            .iter()
            .all(|column| column.range.offset == submitted_before),
        "the next range must continue from the pre-decoration cursor"
    );
    r.cancel_chart_stream(a).unwrap();
}

#[test]
fn rejected_mutations_and_bad_payloads_preserve_valid_requests() {
    let (mut r, a, _) = renderer(2);
    let job = r.begin_chart_stream(a).unwrap();
    let ticket = request(&mut r, job, "a");
    assert!(r.replace_streamed_columns(vec![source("a", 1)]).is_err());
    assert!(
        r.set_chart_series(a, vec![declaration("bad", "missing", "a")])
            .is_err()
    );
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let before = r.gpu_memory_usage();
    let short = [0u8; 4];
    assert!(
        r.accept_stream_columns(
            ticket,
            &[
                StreamInput {
                    column: "x",
                    bytes: &short
                },
                StreamInput {
                    column: "a",
                    bytes: &short
                }
            ],
            &mut encoder
        )
        .is_err()
    );
    assert!(r.stream_request_columns(ticket).is_ok());
    assert_eq!(r.gpu_memory_usage(), before);
    let bytes = [0u8; 8];
    let chunk = r
        .accept_stream_columns(
            ticket,
            &[
                StreamInput {
                    column: "x",
                    bytes: &bytes,
                },
                StreamInput {
                    column: "a",
                    bytes: &bytes,
                },
            ],
            &mut encoder,
        )
        .unwrap();
    assert_eq!(chunk.columns.len(), 2);
    assert!(
        r.accept_stream_columns(
            ticket,
            &[
                StreamInput {
                    column: "x",
                    bytes: &bytes
                },
                StreamInput {
                    column: "a",
                    bytes: &bytes
                }
            ],
            &mut encoder
        )
        .is_err()
    );
    r.cancel_chart_stream(a).unwrap();
    assert_eq!(
        r.stream_request_usage().2,
        64,
        "recorded commands retain their reservation after cancel"
    );
    drop(encoder);
    drop(chunk);
    r.discard_stream_recording(ticket).unwrap();
    assert_eq!(r.stream_request_usage().1, 0);
    assert_eq!(r.stream_request_usage().2, 0);
}

#[test]
fn foreign_tickets_and_remove_reregister_cannot_revive_old_work() {
    let (mut r, a, _) = renderer(4);
    let (mut other, oa, _) = renderer(4);
    let old_job = r.begin_chart_stream(a).unwrap();
    let old_ticket = request(&mut r, old_job, "a");
    let other_job = other.begin_chart_stream(oa).unwrap();
    request(&mut other, other_job, "a");
    assert!(other.stream_request_columns(old_ticket).is_err());
    assert!(other.request_stream_columns(old_job, &ranges("a")).is_err());
    r.remove_column("a").unwrap();
    r.register_streamed_columns(vec![source("a", 1)]).unwrap();
    r.set_chart_series(a, vec![declaration("s", "x", "a")])
        .unwrap();
    assert!(r.stream_request_columns(old_ticket).is_err());
    let fresh = r.begin_chart_stream(a).unwrap();
    let fresh_ticket = request(&mut r, fresh, "a");
    r.remove_chart(a).unwrap();
    assert!(r.stream_request_columns(fresh_ticket).is_err());
    assert!(r.request_stream_columns(fresh, &ranges("a")).is_err());
}

#[test]
fn limits_are_not_reset_to_reuse_ticket_identity() {
    let (mut r, a, _) = renderer(2);
    let job = r.begin_chart_stream(a).unwrap();
    let old = request(&mut r, job, "a");
    r.cancel_chart_stream(a).unwrap();
    assert!(matches!(
        r.configure_streaming_runtime(limits(2)),
        Err(StreamRequestError::Scheduler(StreamError::WrongState))
    ));
    let next = r.begin_chart_stream(a).unwrap();
    let current = request(&mut r, next, "a");
    assert!(r.stream_request_columns(old).is_err());
    assert!(r.stream_request_columns(current).is_ok());
}

#[test]
fn arbitrary_named_columns_are_derived_from_live_registry_not_host_revision_claims() {
    let (mut r, a, _) = renderer(2);
    let columns: Vec<_> = (0..17).map(|i| source(&format!("c{i}"), i + 1)).collect();
    r.register_streamed_columns(columns.clone()).unwrap();
    r.set_chart_series(
        a,
        columns
            .iter()
            .map(|c| declaration(&c.id, "x", &c.id))
            .collect(),
    )
    .unwrap();
    let job = r.begin_chart_stream(a).unwrap();
    let ranges: Vec<_> = columns
        .iter()
        .map(|c| StreamSourceRange {
            column: &c.id,
            offset: 2,
            len: 3,
        })
        .collect();
    let StreamRequestStatus::Ready(ticket) = r.request_stream_columns(job, &ranges).unwrap() else {
        panic!("unexpected backpressure")
    };
    let resolved = r.stream_request_columns(ticket).unwrap();
    assert_eq!(resolved.len(), 17);
    for (expected, actual) in columns.iter().zip(resolved) {
        assert_eq!(actual.column, expected.id);
        assert_eq!(actual.range.revision, expected.revision);
        assert_eq!(actual.range.source_len, expected.len);
        assert_eq!(actual.range.offset, 2);
    }
    let before = r.stream_request_usage();
    assert!(
        r.request_stream_columns(
            job,
            &[StreamSourceRange {
                column: "b",
                offset: 0,
                len: 2
            }]
        )
        .is_err()
    );
    assert_eq!(r.stream_request_usage(), before);
}

#[test]
fn late_gpu_completion_releases_only_its_recording_after_chart_cancel() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (mut r, a, b) = renderer(2);
    let ja = r.begin_chart_stream(a).unwrap();
    let ta = request(&mut r, ja, "a");
    let bytes = [0u8; 8];
    let mut encoder_a = r.device.create_command_encoder(&Default::default());
    let chunk_a = r
        .accept_stream_columns(
            ta,
            &[
                StreamInput {
                    column: "x",
                    bytes: &bytes,
                },
                StreamInput {
                    column: "a",
                    bytes: &bytes,
                },
            ],
            &mut encoder_a,
        )
        .unwrap();
    r.cancel_chart_stream(a).unwrap();
    let jb = r.begin_chart_stream(b).unwrap();
    let tb = request(&mut r, jb, "b");
    let mut encoder_b = r.device.create_command_encoder(&Default::default());
    let chunk_b = r
        .accept_stream_columns(
            tb,
            &[
                StreamInput {
                    column: "x",
                    bytes: &bytes,
                },
                StreamInput {
                    column: "b",
                    bytes: &bytes,
                },
            ],
            &mut encoder_b,
        )
        .unwrap();
    assert_eq!(r.stream_request_usage(), (1, 2, 128));

    let ra = r.submit_stream_recording(ta).unwrap();
    r.queue.submit([encoder_a.finish()]);
    let done_a = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done_a);
    r.queue
        .on_submitted_work_done(move || flag.store(true, Ordering::Release));
    let rb = r.submit_stream_recording(tb).unwrap();
    let submission_b = r.queue.submit([encoder_b.finish()]);
    let done_b = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done_b);
    r.queue
        .on_submitted_work_done(move || flag.store(true, Ordering::Release));
    drop((chunk_a, chunk_b));
    // Actual completion is observed before the executor acknowledges a receipt.
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission_b),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    assert!(done_a.load(Ordering::Acquire));
    assert!(done_b.load(Ordering::Acquire));
    r.complete_stream_submission(ra).unwrap();
    assert_eq!(r.stream_request_usage(), (1, 1, 64));
    assert!(r.complete_stream_submission(ra).is_err());
    assert_eq!(r.stream_request_usage(), (1, 1, 64));
    r.cancel_chart_stream(b).unwrap();
    assert_eq!(r.stream_request_usage(), (0, 1, 64));
    r.complete_stream_submission(rb).unwrap();
    assert_eq!(r.stream_request_usage(), (0, 0, 0));
    r.end_gpu_frame();
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    wait_stream_retirement(&r);
}

#[test]
fn successful_target_change_invalidates_old_requests() {
    let (mut r, a, b) = renderer(2);
    let ja = r.begin_chart_stream(a).unwrap();
    let jb = r.begin_chart_stream(b).unwrap();
    let ta = request(&mut r, ja, "a");
    let tb = request(&mut r, jb, "b");
    r.ensure_target_format(wgpu::TextureFormat::Bgra8Unorm)
        .unwrap();
    assert!(r.stream_request_columns(ta).is_err());
    assert!(r.stream_request_columns(tb).is_err());
    assert_eq!(r.stream_request_usage(), (0, 0, 0));
    let fresh = r.begin_chart_stream(a).unwrap();
    request(&mut r, fresh, "a");
}

#[test]
fn renderer_budget_rejection_keeps_ticket_retryable_without_gpu_allocation() {
    let (mut r, a, _) = renderer(1);
    let job = r.begin_chart_stream(a).unwrap();
    let ticket = request(&mut r, job, "a");
    let before = r.gpu_memory_usage();
    assert!(r.set_memory_budget(Some(before.total_bytes())).is_none());
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let bytes = [0u8; 8];
    let inputs = [
        StreamInput {
            column: "x",
            bytes: &bytes,
        },
        StreamInput {
            column: "a",
            bytes: &bytes,
        },
    ];
    assert!(matches!(
        r.accept_stream_columns(ticket, &inputs, &mut encoder),
        Err(StreamRequestError::Upload(
            crate::streaming_upload::ChunkUploadError::Input(StreamError::TooLarge)
        ))
    ));
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(r.streaming_sources["x"].statistics_cache.covered.is_empty());
    assert!(r.streaming_sources["a"].statistics_cache.covered.is_empty());
    assert!(r.stream_request_columns(ticket).is_ok());
    assert!(
        r.set_memory_budget(Some(before.total_bytes() + 64))
            .is_none()
    );
    let chunk = r
        .accept_stream_columns(ticket, &inputs, &mut encoder)
        .unwrap();
    assert_eq!(chunk.charged_bytes(), 64);
    assert_eq!(r.streaming_sources["x"].statistics_cache.covered, vec![0..2]);
    assert_eq!(r.streaming_sources["a"].statistics_cache.covered, vec![0..2]);
    drop(encoder);
    drop(chunk);
    r.discard_stream_recording(ticket).unwrap();
}

#[test]
fn renderer_caches_staging_extrema_once_and_publishes_only_complete_coverage() {
    let (mut r, a, _) = renderer(2);
    let x = [f32::NAN, -2.0, 4.0, 1.0, 100.0, 2.0, -0.0, 3.0];
    let y = [10.0f32, 9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0];
    let job = r.begin_chart_stream(a).unwrap();
    for (offset, len) in [(0u64, 5usize), (4, 4)] {
        let requested = [
            StreamSourceRange { column: "x", offset, len: len as u64 },
            StreamSourceRange { column: "a", offset, len: len as u64 },
        ];
        let StreamRequestStatus::Ready(ticket) =
            r.request_stream_columns(job, &requested).unwrap()
        else { panic!("unexpected statistics backpressure"); };
        let start = offset as usize;
        let mut encoder = r.device.create_command_encoder(&Default::default());
        let chunk = r.accept_stream_columns(ticket, &[
            StreamInput { column: "x", bytes: bytemuck::cast_slice(&x[start..start + len]) },
            StreamInput { column: "a", bytes: bytemuck::cast_slice(&y[start..start + len]) },
        ], &mut encoder).unwrap();
        assert!(chunk.columns.iter().all(|column| column.statistics.is_some()));
        if offset == 4 {
            assert_eq!(chunk.columns[0].statistics, Some(Some(crate::StreamBounds {
                min: 0.0, max: 3.0, min_positive: Some(2.0),
            })), "the overlapped line halo value at index 4 must not be measured twice");
        }
        drop((encoder, chunk));
        r.discard_stream_recording(ticket).unwrap();
        if offset == 0 {
            assert_eq!(r.logical_column("x").unwrap().statistics(), crate::StreamStatistics::Pending);
            assert_eq!(r.streaming_sources["x"].statistics_cache.covered, vec![0..5]);
        }
    }
    assert_eq!(r.streaming_sources["x"].statistics_cache.covered, vec![0..8]);
    assert_eq!(r.logical_column("x").unwrap().statistics(), crate::StreamStatistics::Known(Some(crate::StreamBounds {
        min: -2.0, max: 100.0, min_positive: Some(1.0),
    })));
    assert_eq!(r.logical_column("a").unwrap().statistics(), crate::StreamStatistics::Known(Some(crate::StreamBounds {
        min: 3.0, max: 10.0, min_positive: Some(3.0),
    })));

    let fit_epoch = r.streaming_sources["x"].fit_epoch;
    let mut config = r.chart_config(a).unwrap().clone();
    config.bottom_x.scale = AxisScale::Logarithmic;
    config.bottom_x.min = 0.5;
    config.bottom_x.max = 100.0;
    r.set_chart_config(a, config).unwrap();
    let job = r.begin_chart_stream(a).unwrap();
    let repeated = [
        StreamSourceRange { column: "x", offset: 0, len: 4 },
        StreamSourceRange { column: "a", offset: 0, len: 4 },
    ];
    let StreamRequestStatus::Ready(ticket) = r.request_stream_columns(job, &repeated).unwrap()
    else { panic!("unexpected repeated-range backpressure"); };
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let chunk = r.accept_stream_columns(ticket, &[
        StreamInput { column: "x", bytes: bytemuck::cast_slice(&x[..4]) },
        StreamInput { column: "a", bytes: bytemuck::cast_slice(&y[..4]) },
    ], &mut encoder).unwrap();
    assert!(chunk.columns.iter().all(|column| column.statistics.is_none()));
    assert_eq!(r.streaming_sources["x"].fit_epoch, fit_epoch);
    drop((encoder, chunk));
    r.discard_stream_recording(ticket).unwrap();
}

#[test]
fn renderer_canonicalizes_same_ticket_ranges_and_bounds_sparse_metadata() {
    let (mut r, a, _) = renderer(2);
    let values = [8.0f32, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0];
    let job = r.begin_chart_stream(a).unwrap();
    let reversed = [
        StreamSourceRange { column: "x", offset: 4, len: 4 },
        StreamSourceRange { column: "x", offset: 0, len: 4 },
    ];
    let StreamRequestStatus::Ready(ticket) = r.request_stream_columns(job, &reversed).unwrap()
    else { panic!("unexpected reverse-range backpressure"); };
    let fit_epoch = r.next_stream_source_fit_epoch;
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let chunk = r.accept_stream_columns(ticket, &[
        StreamInput { column: "x", bytes: bytemuck::cast_slice(&values[4..]) },
        StreamInput { column: "x", bytes: bytemuck::cast_slice(&values[..4]) },
    ], &mut encoder).unwrap();
    assert!(chunk.columns.iter().all(|column| column.statistics.is_some()));
    assert_eq!(r.next_stream_source_fit_epoch, fit_epoch + 1);
    assert_eq!(r.streaming_sources["x"].statistics_cache.covered, vec![0..8]);
    assert_eq!(r.logical_column("x").unwrap().statistics(), crate::StreamStatistics::Known(Some(crate::StreamBounds {
        min: 1.0, max: 8.0, min_positive: Some(1.0),
    })));
    drop((encoder, chunk));
    r.discard_stream_recording(ticket).unwrap();

    let (mut r, a, _) = renderer(2);
    let job = r.begin_chart_stream(a).unwrap();
    let late = [StreamSourceRange { column: "x", offset: 4, len: 4 }];
    let StreamRequestStatus::Ready(ticket) = r.request_stream_columns(job, &late).unwrap()
    else { panic!("unexpected sparse-range backpressure"); };
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let chunk = r.accept_stream_columns(ticket, &[StreamInput {
        column: "x", bytes: bytemuck::cast_slice(&values[4..]),
    }], &mut encoder).unwrap();
    assert_eq!(chunk.columns[0].statistics, Some(Some(crate::StreamBounds {
        min: 1.0, max: 4.0, min_positive: Some(1.0),
    })));
    assert_eq!(r.streaming_sources["x"].statistics_cache.covered, vec![4..8]);
    assert_eq!(r.logical_column("x").unwrap().statistics(), crate::StreamStatistics::Pending);
    drop((encoder, chunk));
    r.discard_stream_recording(ticket).unwrap();

    let job = r.begin_chart_stream(a).unwrap();
    let early = [StreamSourceRange { column: "x", offset: 0, len: 4 }];
    let StreamRequestStatus::Ready(ticket) = r.request_stream_columns(job, &early).unwrap()
    else { panic!("unexpected gap-fill backpressure"); };
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let chunk = r.accept_stream_columns(ticket, &[StreamInput {
        column: "x", bytes: bytemuck::cast_slice(&values[..4]),
    }], &mut encoder).unwrap();
    assert_eq!(chunk.columns[0].statistics, Some(Some(crate::StreamBounds {
        min: 5.0, max: 8.0, min_positive: Some(5.0),
    })));
    assert_eq!(r.streaming_sources["x"].statistics_cache.covered, vec![0..8]);
    assert_eq!(r.logical_column("x").unwrap().statistics(), crate::StreamStatistics::Known(Some(crate::StreamBounds {
        min: 1.0, max: 8.0, min_positive: Some(1.0),
    })));
    drop((encoder, chunk));
    r.discard_stream_recording(ticket).unwrap();
}

#[test]
fn statistics_completion_epoch_overflow_is_preflighted_and_retryable() {
    let (mut r, a, _) = renderer(1);
    let values = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let job = r.begin_chart_stream(a).unwrap();
    let requested = [StreamSourceRange { column: "x", offset: 0, len: 8 }];
    let StreamRequestStatus::Ready(ticket) = r.request_stream_columns(job, &requested).unwrap()
    else { panic!("unexpected statistics overflow backpressure"); };
    let before = r.gpu_memory_usage();
    let fit_epoch = r.next_stream_source_fit_epoch;
    r.next_stream_source_fit_epoch = u64::MAX;
    let mut encoder = r.device.create_command_encoder(&Default::default());
    assert!(matches!(r.accept_stream_columns(ticket, &[StreamInput {
        column: "x", bytes: bytemuck::cast_slice(&values),
    }], &mut encoder), Err(StreamRequestError::State(FiggyError::CounterExhausted { .. }))));
    assert_eq!(r.gpu_memory_usage(), before);
    assert!(r.streaming_sources["x"].statistics_cache.covered.is_empty());
    assert!(r.stream_request_columns(ticket).is_ok());
    r.next_stream_source_fit_epoch = fit_epoch;
    let chunk = r.accept_stream_columns(ticket, &[StreamInput {
        column: "x", bytes: bytemuck::cast_slice(&values),
    }], &mut encoder).unwrap();
    assert_eq!(r.streaming_sources["x"].statistics_cache.covered, vec![0..8]);
    drop((encoder, chunk));
    r.discard_stream_recording(ticket).unwrap();
}

#[test]
fn complete_statistics_stale_fit_token_without_cancelling_same_revision_stream_ticket() {
    let (mut r, a, _) = renderer(2);
    let old_fit = r.begin_fit_commit(a).unwrap();
    let before_data = r.chart_states[&a].revisions.data;
    let before_desired = r.chart_states[&a].revisions.desired;
    let before_visual = r.visual_revision;
    let job = r.begin_chart_stream(a).unwrap();
    let ticket = request(&mut r, job, "a");
    let requested = r.stream_request_columns(ticket).unwrap()
        .iter().map(|column| (column.column.clone(), column.range)).collect::<Vec<_>>();
    r.commit_streamed_statistics("x", 1, Some(crate::StreamBounds {
        min: 0.0,
        max: 1.0,
        min_positive: Some(0.25),
    })).unwrap();
    assert_eq!(r.chart_states[&a].revisions.data, before_data);
    assert_eq!(r.chart_states[&a].revisions.desired, before_desired);
    assert_eq!(r.visual_revision, before_visual);
    assert_ne!(r.begin_fit_commit(a).unwrap(), old_fit);
    assert_eq!(r.stream_request_columns(ticket).unwrap()
        .iter().map(|column| (column.column.clone(), column.range)).collect::<Vec<_>>(), requested);
    assert_eq!(r.stream_request_usage().1, 1);
    r.cancel_chart_stream(a).unwrap();
}

#[test]
fn public_column_source_step_owns_cursor_and_never_requires_encoded_payloads() {
    struct Virtual {
        len: usize,
        scale: f32,
        fail_once: std::cell::Cell<bool>,
        panic_once: std::cell::Cell<bool>,
        calls: std::cell::Cell<u32>,
        furthest_start: std::cell::Cell<u64>,
    }

    impl crate::ColumnSource for Virtual {
        fn len(&self) -> usize { self.len }
        fn min(&self) -> f64 { 0.0 }
        fn max(&self) -> f64 {
            (self.len.saturating_sub(1) as f32 * self.scale) as f64
        }
        fn write_f32_le_into(&self, _dst: &mut [u8]) {
            panic!("streaming must not invoke the full-column encoder")
        }
        fn write_f32_pair_le_into_with_stats(
            &self,
            _dst: crate::ColumnPairWriter<'_>,
        ) -> crate::ColumnUploadStats {
            panic!("streaming must not invoke the full-column writer")
        }
        fn write_f32_pair_range_into_with_stats(
            &self,
            start: u64,
            mut dst: crate::ColumnPairWriter<'_>,
        ) -> std::result::Result<Option<crate::StreamBounds>, crate::ColumnRangeWriteError> {
            let end = start.checked_add(dst.len() as u64)
                .filter(|end| *end <= self.len as u64)
                .ok_or(crate::ColumnRangeWriteError::InvalidRange)?;
            if self.fail_once.replace(false) {
                return Err(crate::ColumnRangeWriteError::SourceFailed);
            }
            if self.panic_once.replace(false) {
                panic!("simulated source panic");
            }
            self.calls.set(self.calls.get() + 1);
            self.furthest_start.set(self.furthest_start.get().max(start));
            for local in 0..dst.len() {
                let value = (start + local as u64) as f32 * self.scale;
                dst.write_pair(local, value, 0.0);
            }
            let first = start as f32 * self.scale;
            let last = end.saturating_sub(1) as f32 * self.scale;
            Ok(Some(crate::StreamBounds {
                min: first.min(last) as f64,
                max: first.max(last) as f64,
                min_positive: (end > start)
                    .then_some(if first > 0.0 { first } else { self.scale.abs() })
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .map(f64::from),
            }))
        }
    }

    let (mut r, chart_id, _) = renderer(2);
    let config = r.chart_config(chart_id).unwrap().clone();
    let chart = Chart::new(config.clone());
    let view = r.create_chart_view(&chart, config.chart_area.0).unwrap();
    r.begin_streaming_chart(chart_id, &view, crate::StreamingChartOptions {
        size: (320, 240),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    }).unwrap();

    let x = Virtual {
        len: 8,
        scale: 0.1,
        fail_once: std::cell::Cell::new(true),
        panic_once: std::cell::Cell::new(true),
        calls: std::cell::Cell::new(0),
        furthest_start: std::cell::Cell::new(0),
    };
    let y = Virtual {
        len: 8,
        scale: 0.2,
        fail_once: std::cell::Cell::new(false),
        panic_once: std::cell::Cell::new(false),
        calls: std::cell::Cell::new(0),
        furthest_start: std::cell::Cell::new(0),
    };
    let bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&x),
        },
        crate::StreamSourceBinding {
            id: "a",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];

    assert!(matches!(
        r.stream_chart_step(chart_id, &view, &bindings),
        Err(FiggyError::InvalidStreamSource { id, .. }) if id == "x"
    ));
    assert_eq!(r.streaming_usage().in_flight_chunks, 1);
    assert!(matches!(
        r.stream_chart_step(chart_id, &view, &bindings),
        Err(FiggyError::InvalidStreamSource { id, .. }) if id == "x"
    ));
    assert_eq!(r.streaming_usage().in_flight_chunks, 1);

    let mut last_submitted = 0;
    loop {
        match r.stream_chart_step(chart_id, &view, &bindings).unwrap() {
            crate::StreamingProgress::Submitted {
                submitted_primitives,
                total_primitives,
            } => {
                assert_eq!(total_primitives, 7);
                assert!(submitted_primitives > last_submitted);
                last_submitted = submitted_primitives;
            }
            crate::StreamingProgress::Backpressure {
                submitted_primitives,
                total_primitives,
            } => {
                assert_eq!(submitted_primitives, last_submitted);
                assert_eq!(total_primitives, 7);
                wait_stream_slots(&mut r, 0);
            }
            crate::StreamingProgress::AllSubmitted { total_primitives } => {
                assert_eq!(total_primitives, 7);
                assert_eq!(last_submitted, 7);
                break;
            }
        }
    }
    assert_eq!(x.calls.get(), 4);
    assert_eq!(y.calls.get(), 4);
    assert_eq!(x.furthest_start.get(), 6);
    assert_eq!(y.furthest_start.get(), 6);
    r.cancel_streaming_chart(chart_id).unwrap();
    assert_eq!(r.streaming_usage().active_charts, 0);
}

#[test]
fn automatic_stream_keeps_decoration_only_changes_and_supersedes_changed_inputs() {
    let (mut r, chart_id, _) = renderer(2);
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    let started = r
        .request_auto_streaming_chart(chart_id, &view, options)
        .unwrap();
    let crate::AutoStreamingRequest::Started { revision, sources } = started else {
        panic!("first automatic request must capture one execution")
    };
    assert_eq!(sources.len(), 2);
    assert!(sources.iter().all(|source| source.revision == 1));

    let job = r.active_stream_job(chart_id).unwrap();
    let snapshot = r.auto_stream_snapshot(job).unwrap();
    assert_eq!(snapshot.desired, revision);
    assert_eq!(snapshot.series[0].y_column, "a");
    assert_eq!(snapshot.sources["x"].revision, 1);
    let StreamDrawRequestStatus::Ready(old_ticket) = r.request_chart_stream_draw(job).unwrap()
    else {
        panic!("first automatic execution must expose its initial range")
    };

    let mut next_config = config.clone();
    next_config.chart_title.visible = !next_config.chart_title.visible;
    r.set_chart_config(chart_id, next_config).unwrap();
    assert!(matches!(
        r.request_auto_streaming_chart(chart_id, &view, options)
            .unwrap(),
        crate::AutoStreamingRequest::Active {
            revision: active,
            pending_latest: None,
        } if active == revision
    ));
    assert_eq!(r.active_stream_job(chart_id), Some(job));
    assert_eq!(
        r.request_chart_stream_draw(job).unwrap(),
        StreamDrawRequestStatus::Ready(old_ticket),
        "decoration-only changes must preserve the admitted cursor"
    );

    r.set_chart_series(chart_id, vec![declaration("latest", "x", "b")])
        .unwrap();
    r.replace_streamed_columns(vec![source("x", 2)]).unwrap();
    let latest = r.chart_states[&chart_id].revisions.desired;
    assert_ne!(latest, revision);
    let crate::AutoStreamingRequest::Started {
        revision: replacement_revision,
        sources,
    } = r
        .request_auto_streaming_chart(chart_id, &view, options)
        .unwrap()
    else {
        panic!("changed stream inputs must start the latest execution immediately")
    };
    assert_eq!(replacement_revision, latest);
    assert!(sources.iter().any(|source| source.id == "x" && source.revision == 2));
    assert!(sources.iter().any(|source| source.id == "b" && source.revision == 1));

    let replacement_job = r.active_stream_job(chart_id).unwrap();
    assert_ne!(replacement_job, job);
    assert!(r.stream_request_columns(old_ticket).is_err());
    let snapshot = r.auto_stream_snapshot(replacement_job).unwrap();
    assert_eq!(snapshot.desired, latest);
    assert_eq!(snapshot.series[0].y_column, "b");
    assert_eq!(snapshot.sources["x"].revision, 2);
    assert_eq!(snapshot.sources["b"].revision, 1);
}

#[test]
fn automatic_stream_size_change_starts_a_new_target_immediately() {
    let (mut r, chart_id, _) = renderer(2);
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    let crate::AutoStreamingRequest::Started { revision, .. } = r
        .request_auto_streaming_chart(chart_id, &view, options)
        .unwrap()
    else {
        panic!("first automatic request must start an execution")
    };
    let first_job = r.active_stream_job(chart_id).unwrap();
    let first_target = r.chart_stream_prefix_for_test(first_job).unwrap();
    assert_eq!((first_target.width(), first_target.height()), options.size);

    let resized = crate::StreamingChartOptions {
        size: (options.size.0 + 17, options.size.1 + 11),
        ..options
    };
    assert!(matches!(
        r.request_auto_streaming_chart(chart_id, &view, resized)
            .unwrap(),
        crate::AutoStreamingRequest::Started {
            revision: replacement,
            ..
        } if replacement == revision
    ));
    let replacement_job = r.active_stream_job(chart_id).unwrap();
    assert_ne!(replacement_job, first_job);
    assert!(r.chart_stream_prefix_for_test(first_job).is_err());
    let replacement_target = r.chart_stream_prefix_for_test(replacement_job).unwrap();
    assert_eq!(
        (replacement_target.width(), replacement_target.height()),
        resized.size
    );
}

#[test]
fn automatic_stream_uses_the_host_display_config_panel() {
    let (mut r, chart_id, _) = renderer(2);
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let mut display_config = config;
    display_config.chart_area = crate::layout::ChartArea(crate::layout::Rect {
        x: 13,
        y: 17,
        width: 451,
        height: 263,
    });
    assert!(matches!(
        r.request_auto_streaming_chart_with_config(
            chart_id,
            &view,
            display_config.clone(),
            crate::StreamingChartOptions {
                size: (640, 360),
                clear_color: Color::WHITE,
                max_primitives_per_chunk: 2,
            },
        )
        .unwrap(),
        crate::AutoStreamingRequest::Started { .. }
    ));
    let job = r.active_stream_job(chart_id).unwrap();
    assert_eq!(
        r.auto_stream_snapshot(job).unwrap().view.panel_rect,
        display_config.chart_area.0
    );
}

#[test]
fn automatic_stream_style_change_restarts_the_data_prefix() {
    let (mut r, chart_id, _) = renderer(2);
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    assert!(matches!(
        r.request_auto_streaming_chart(chart_id, &view, options)
            .unwrap(),
        crate::AutoStreamingRequest::Started { .. }
    ));
    let first_job = r.active_stream_job(chart_id).unwrap();

    let mut styled = declaration("s", "x", "a");
    let DataRenderType::Line { line } = &mut styled.render_type else {
        unreachable!()
    };
    line.line_width = 7.0;
    r.set_chart_series(chart_id, vec![styled]).unwrap();
    let latest = r.chart_states[&chart_id].revisions.desired;

    assert!(matches!(
        r.request_auto_streaming_chart(chart_id, &view, options)
            .unwrap(),
        crate::AutoStreamingRequest::Started { revision, .. } if revision == latest
    ));
    assert_ne!(r.active_stream_job(chart_id), Some(first_job));
}

#[test]
fn automatic_stream_becomes_terminal_only_after_receipts_and_final_display() {
    let (mut r, chart_id, _) = renderer(2);
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    let crate::AutoStreamingRequest::Started { revision, .. } = r
        .request_auto_streaming_chart(chart_id, &view, options)
        .unwrap()
    else {
        panic!("automatic stream did not start")
    };
    let original_job = r.active_stream_job(chart_id).unwrap();
    let x = crate::Column {
        data: (0..8).map(|value| value as f32 / 8.0).collect(),
        min: 0.0,
        max: 0.875,
    };
    let y = crate::Column {
        data: (0..8).map(|value| value as f32 / 16.0).collect(),
        min: 0.0,
        max: 0.4375,
    };
    let bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&x),
        },
        crate::StreamSourceBinding {
            id: "a",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];
    loop {
        match r.auto_stream_chart_step(chart_id, &bindings).unwrap() {
            crate::AutoStreamingProgress::Submitted { .. } => {}
            crate::AutoStreamingProgress::Backpressure { .. } => wait_stream_slots(&mut r, 0),
            crate::AutoStreamingProgress::AllSubmitted {
                revision: actual, ..
            } => {
                assert_eq!(actual, revision);
                break;
            }
            crate::AutoStreamingProgress::Complete { .. } => {
                panic!("completion cannot precede the final display refresh")
            }
        }
    }
    wait_stream_slots(&mut r, 0);
    assert!(matches!(
        r.auto_stream_chart_step(chart_id, &bindings).unwrap(),
        crate::AutoStreamingProgress::AllSubmitted { .. }
    ));
    let final_display = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id,
            view: &view,
        }])
        .unwrap();
    drop(final_display);
    assert!(matches!(
        r.auto_stream_chart_step(chart_id, &bindings).unwrap(),
        crate::AutoStreamingProgress::Complete {
            revision: actual,
            pending_latest: None,
        } if actual == revision
    ));

    let mut changed_config = config.clone();
    changed_config.chart_title.visible = !changed_config.chart_title.visible;
    r.set_chart_config(chart_id, changed_config).unwrap();
    let invalid_options = crate::StreamingChartOptions {
        max_primitives_per_chunk: 0,
        ..options
    };
    assert!(
        r.request_auto_streaming_chart(chart_id, &view, invalid_options)
            .is_err()
    );
    assert_eq!(r.active_stream_job(chart_id), Some(original_job));
    assert!(r.chart_stream_display(original_job).is_ok());
    assert!(matches!(
        r.auto_stream_chart_step(chart_id, &bindings).unwrap(),
        crate::AutoStreamingProgress::Complete {
            revision: actual,
            pending_latest: None,
        } if actual == revision
    ));

    assert!(r.active_stream_job(chart_id).is_some());
    assert!(matches!(r.logical_column("x"), Some(crate::LogicalColumn::Streamed(_))));
}

#[test]
fn automatic_range_request_is_stable_until_submit_advances_the_cpu_cursor() {
    let (mut r, chart_id, _) = renderer(2);
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    r.request_auto_streaming_chart(
        chart_id,
        &view,
        crate::StreamingChartOptions {
            size: (config.chart_area.0.width, config.chart_area.0.height),
            clear_color: Color::WHITE,
            max_primitives_per_chunk: 2,
        },
    )
    .unwrap();

    let first = r.auto_stream_chart_request_ranges(chart_id).unwrap();
    let crate::AutoStreamingRangeRequest::Ready {
        ranges: first_ranges,
        submitted_primitives: 0,
        total_primitives: 7,
        ..
    } = &first
    else {
        panic!("first range request was not ready: {first:?}")
    };
    assert_eq!(first_ranges.len(), 2);
    assert!(first_ranges.iter().all(|range| range.offset == 0));
    assert_eq!(
        first_ranges
            .iter()
            .map(|range| (range.id.as_str(), range.len))
            .collect::<Vec<_>>(),
        vec![("x", 3), ("a", 3)]
    );
    assert_eq!(r.auto_stream_chart_request_ranges(chart_id).unwrap(), first);

    let x = crate::Column {
        data: (0..3).map(|value| value as f32 / 8.0).collect(),
        min: 0.0,
        max: 0.25,
    };
    let y = crate::Column {
        data: (0..3).map(|value| value as f32 / 16.0).collect(),
        min: 0.0,
        max: 0.125,
    };
    let bindings = [
        crate::StreamRangeSourceBinding {
            id: "x",
            revision: 1,
            source_len: 8,
            offset: 0,
            source: crate::StreamColumnSource::Scalar(&x),
        },
        crate::StreamRangeSourceBinding {
            id: "a",
            revision: 1,
            source_len: 8,
            offset: 0,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];
    assert!(matches!(
        r.auto_stream_chart_submit_ranges(chart_id, &bindings)
            .unwrap(),
        crate::AutoStreamingProgress::Submitted {
            submitted_primitives: 2,
            total_primitives: 7,
            ..
        }
    ));

    let next = r.auto_stream_chart_request_ranges(chart_id).unwrap();
    let crate::AutoStreamingRangeRequest::Ready { ranges, .. } = next else {
        panic!("second range request was not ready: {next:?}")
    };
    assert!(ranges.iter().all(|range| range.offset == 2));
    assert_eq!(
        ranges
            .iter()
            .map(|range| (range.id.as_str(), range.len))
            .collect::<Vec<_>>(),
        vec![("x", 3), ("a", 3)]
    );
}

#[test]
fn resident_range_handoff_keeps_old_authority_until_complete_then_commits_once() {
    let (device, queue) =
        crate::data_render::shared_device().expect("stream handoff test requires GPU");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    let x_values: Vec<f32> = (0..8).map(|value| value as f32).collect();
    let y_values: Vec<f32> = (0..8).map(|value| (value * value) as f32).collect();
    r.add_column(
        "handoff-x",
        &crate::Column {
            data: x_values.clone(),
            min: 0.0,
            max: 7.0,
        },
    )
    .unwrap();
    r.add_column(
        "handoff-y",
        &crate::Column {
            data: y_values.clone(),
            min: 0.0,
            max: 49.0,
        },
    )
    .unwrap();
    let config = crate::default::default_config();
    let chart_id = r
        .register_chart(
            config.clone(),
            vec![declaration("handoff", "handoff-x", "handoff-y")],
        )
        .unwrap();
    r.configure_streaming_runtime(limits(2)).unwrap();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let x_epoch = r.pool.allocation_epoch("handoff-x").unwrap();
    let y_epoch = r.pool.allocation_epoch("handoff-y").unwrap();
    let mut x_metadata = source("handoff-x", 1);
    let mut y_metadata = source("handoff-y", 1);
    x_metadata.len = x_values.len() as u64;
    y_metadata.len = y_values.len() as u64;
    assert!(matches!(
        r.request_resident_stream_handoff_with_config(
            chart_id,
            &view,
            config.clone(),
            vec![x_metadata, y_metadata],
            crate::StreamingChartOptions {
                size: (config.chart_area.0.width, config.chart_area.0.height),
                clear_color: Color::WHITE,
                max_primitives_per_chunk: 2,
            },
        )
        .unwrap(),
        crate::AutoStreamingRequest::Started { .. }
    ));

    let completed_revision = loop {
        assert_eq!(r.pool.allocation_epoch("handoff-x"), Some(x_epoch));
        assert_eq!(r.pool.allocation_epoch("handoff-y"), Some(y_epoch));
        assert!(matches!(
            r.logical_column("handoff-x"),
            Some(crate::LogicalColumn::Resident(_))
        ));
        match r.auto_stream_chart_request_ranges(chart_id).unwrap() {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                let x_range = ranges.iter().find(|range| range.id == "handoff-x").unwrap();
                let y_range = ranges.iter().find(|range| range.id == "handoff-y").unwrap();
                let x_start = x_range.offset as usize;
                let y_start = y_range.offset as usize;
                let x_chunk = crate::Column {
                    data: x_values[x_start..x_start + x_range.len as usize].to_vec(),
                    min: 0.0,
                    max: 0.0,
                };
                let y_chunk = crate::Column {
                    data: y_values[y_start..y_start + y_range.len as usize].to_vec(),
                    min: 0.0,
                    max: 0.0,
                };
                r.auto_stream_chart_submit_ranges(
                    chart_id,
                    &[
                        crate::StreamRangeSourceBinding {
                            id: "handoff-x",
                            revision: 1,
                            source_len: x_values.len() as u64,
                            offset: x_range.offset,
                            source: crate::StreamColumnSource::Scalar(&x_chunk),
                        },
                        crate::StreamRangeSourceBinding {
                            id: "handoff-y",
                            revision: 1,
                            source_len: y_values.len() as u64,
                            offset: y_range.offset,
                            source: crate::StreamColumnSource::Scalar(&y_chunk),
                        },
                    ],
                )
                .unwrap();
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. } => {
                wait_stream_slots(&mut r, 0)
            }
            crate::AutoStreamingRangeRequest::AllSubmitted { .. } => {
                wait_stream_slots(&mut r, 0);
                let frame = r
                    .prepare_registered(&[RegisteredChartDrawItem {
                        chart_id,
                        view: &view,
                    }])
                    .unwrap();
                drop(frame);
            }
            crate::AutoStreamingRangeRequest::Complete { revision, .. } => break revision,
        }
    };

    assert_eq!(r.chart_states[&chart_id].revisions.desired, completed_revision);
    assert!(r.pool.slot("handoff-x").is_none());
    assert!(r.pool.slot("handoff-y").is_none());
    assert!(matches!(
        r.logical_column("handoff-x"),
        Some(crate::LogicalColumn::Streamed(source)) if source.revision == 1
    ));
    assert!(matches!(
        r.logical_column("handoff-y"),
        Some(crate::LogicalColumn::Streamed(source)) if source.revision == 1
    ));
    let job = r.stream_status(chart_id).unwrap().job_id;
    assert!(matches!(r.request_auto_streaming_chart(chart_id, &view, crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    }).unwrap(), crate::AutoStreamingRequest::Complete { .. }));
    assert_eq!(r.stream_status(chart_id).unwrap().job_id, job);
}

#[test]
fn resident_handoff_decoration_coalesces_without_replacing_pending_ranges() {
    let (mut r, _, _) = renderer(2);
    let values = crate::Column { data: vec![0.0; 8], min: 0.0, max: 0.0 };
    r.add_column("resident-x", &values).unwrap();
    r.add_column("resident-y", &values).unwrap();
    let config = crate::default::default_config();
    let chart = r.register_chart(config.clone(), vec![declaration("s", "resident-x", "resident-y")]).unwrap();
    let view = r.create_chart_view(&Chart::new(config.clone()), config.chart_area.0).unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    let columns = vec![source("resident-x", 1), source("resident-y", 1)];
    r.request_resident_stream_handoff_with_config(chart, &view, config.clone(), columns.clone(), options).unwrap();
    let job = r.stream_status(chart).unwrap().job_id;
    let pending = r.auto_stream_chart_request_ranges(chart).unwrap();
    let mut decorated = config;
    decorated.chart_title.visible = !decorated.chart_title.visible;
    r.set_chart_config(chart, decorated.clone()).unwrap();
    assert!(matches!(r.request_resident_stream_handoff_with_config(
        chart, &view, decorated.clone(), columns.clone(), options,
    ).unwrap(), crate::AutoStreamingRequest::Active { .. }));
    assert_eq!(r.stream_status(chart).unwrap().job_id, job);
    assert_eq!(r.auto_stream_chart_request_ranges(chart).unwrap(), pending);
    let mut invalid = columns;
    invalid[0].revision = 0;
    assert!(r.request_resident_stream_handoff_with_config(chart, &view, decorated, invalid, options).is_err());
    assert_eq!(r.stream_status(chart).unwrap().job_id, job);
    assert_eq!(r.auto_stream_chart_request_ranges(chart).unwrap(), pending);
}

#[test]
fn cancelling_resident_range_handoff_discards_only_the_candidate_stream() {
    let (device, queue) =
        crate::data_render::shared_device().expect("stream handoff test requires GPU");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    let values = crate::Column {
        data: (0..8).map(|value| value as f32).collect(),
        min: 0.0,
        max: 7.0,
    };
    r.add_column("cancel-x", &values).unwrap();
    r.add_column("cancel-y", &values).unwrap();
    let config = crate::default::default_config();
    let chart_id = r
        .register_chart(
            config.clone(),
            vec![declaration("cancel", "cancel-x", "cancel-y")],
        )
        .unwrap();
    r.configure_streaming_runtime(limits(2)).unwrap();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let x_epoch = r.pool.allocation_epoch("cancel-x").unwrap();
    let y_epoch = r.pool.allocation_epoch("cancel-y").unwrap();
    let mut x = source("cancel-x", 1);
    let mut y = source("cancel-y", 1);
    x.len = 8;
    y.len = 8;
    r.request_resident_stream_handoff_with_config(
        chart_id,
        &view,
        config,
        vec![x, y],
        crate::StreamingChartOptions {
            size: (800, 600),
            clear_color: Color::WHITE,
            max_primitives_per_chunk: 2,
        },
    )
    .unwrap();
    assert_eq!(
        r.interrupt_render(chart_id).unwrap(),
        crate::RenderInterruptStatus::StreamCancelQueued
    );
    r.service_stream_requests();

    assert!(r.active_stream_job(chart_id).is_none());
    assert_eq!(r.pool.allocation_epoch("cancel-x"), Some(x_epoch));
    assert_eq!(r.pool.allocation_epoch("cancel-y"), Some(y_epoch));
    assert!(!r.streaming_sources.contains_key("cancel-x"));
    assert!(!r.streaming_sources.contains_key("cancel-y"));
}

#[test]
fn newer_resident_range_handoff_replaces_only_the_automatic_candidate() {
    let (device, queue) =
        crate::data_render::shared_device().expect("stream handoff test requires GPU");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    let values = crate::Column {
        data: (0..8).map(|value| value as f32).collect(),
        min: 0.0,
        max: 7.0,
    };
    r.add_column("replace-x", &values).unwrap();
    r.add_column("replace-y", &values).unwrap();
    let x_epoch = r.pool.allocation_epoch("replace-x").unwrap();
    let y_epoch = r.pool.allocation_epoch("replace-y").unwrap();
    let config = crate::default::default_config();
    let chart_id = r
        .register_chart(
            config.clone(),
            vec![declaration("replace", "replace-x", "replace-y")],
        )
        .unwrap();
    r.configure_streaming_runtime(limits(2)).unwrap();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let options = crate::StreamingChartOptions {
        size: (800, 600),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    let columns = |revision| {
        let mut x = source("replace-x", revision);
        let mut y = source("replace-y", revision);
        x.len = 8;
        y.len = 8;
        vec![x, y]
    };

    match r
        .request_resident_stream_handoff_with_config(
            chart_id,
            &view,
            config.clone(),
            columns(1),
            options,
        )
        .unwrap()
    {
        crate::AutoStreamingRequest::Started { .. } => {},
        request => panic!("first handoff did not start: {request:?}"),
    }
    match r
        .request_resident_stream_handoff_with_config(
            chart_id,
            &view,
            config,
            columns(2),
            options,
        )
        .unwrap()
    {
        crate::AutoStreamingRequest::Started { .. } => {},
        request => panic!("replacement handoff did not start: {request:?}"),
    }

    assert_eq!(r.pool.allocation_epoch("replace-x"), Some(x_epoch));
    assert_eq!(r.pool.allocation_epoch("replace-y"), Some(y_epoch));
    let crate::AutoStreamingRangeRequest::Ready { ranges, .. } =
        r.auto_stream_chart_request_ranges(chart_id).unwrap()
    else {
        panic!("replacement handoff range was not ready")
    };
    assert!(ranges.iter().all(|range| range.revision == 2));

    assert_eq!(
        r.interrupt_render(chart_id).unwrap(),
        crate::RenderInterruptStatus::StreamCancelQueued
    );
    r.service_stream_requests();
    assert_eq!(r.pool.allocation_epoch("replace-x"), Some(x_epoch));
    assert_eq!(r.pool.allocation_epoch("replace-y"), Some(y_epoch));
}

#[test]
fn mixed_closure_handoff_keeps_both_old_authorities_until_one_stream_commit() {
    let (device, queue) =
        crate::data_render::shared_device().expect("mixed handoff test requires GPU");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    r.register_streamed_columns(vec![source("mixed-x", 1)])
        .unwrap();
    let values = crate::Column {
        data: (0..8).map(|value| value as f32).collect(),
        min: 0.0,
        max: 7.0,
    };
    r.add_column("mixed-y", &values).unwrap();
    let y_epoch = r.pool.allocation_epoch("mixed-y").unwrap();
    let config = crate::default::default_config();
    let chart_id = r
        .register_chart(
            config.clone(),
            vec![declaration("mixed", "mixed-x", "mixed-y")],
        )
        .unwrap();
    r.configure_streaming_runtime(limits(2)).unwrap();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let mut x = source("mixed-x", 1);
    let mut y = source("mixed-y", 1);
    x.len = 8;
    y.len = 8;
    r.request_resident_stream_handoff_with_config(
        chart_id,
        &view,
        config,
        vec![x, y],
        crate::StreamingChartOptions {
            size: (800, 600),
            clear_color: Color::WHITE,
            max_primitives_per_chunk: 8,
        },
    )
    .unwrap();
    assert!(matches!(
        r.logical_column("mixed-x"),
        Some(crate::LogicalColumn::Streamed(source)) if source.revision == 1
    ));
    assert_eq!(r.pool.allocation_epoch("mixed-y"), Some(y_epoch));

    let crate::AutoStreamingRangeRequest::Ready { ranges, .. } =
        r.auto_stream_chart_request_ranges(chart_id).unwrap()
    else {
        panic!("mixed handoff range was not ready")
    };
    let x_range = ranges.iter().find(|range| range.id == "mixed-x").unwrap();
    let y_range = ranges.iter().find(|range| range.id == "mixed-y").unwrap();
    r.auto_stream_chart_submit_ranges(
        chart_id,
        &[
            crate::StreamRangeSourceBinding {
                id: "mixed-x",
                revision: 1,
                source_len: 8,
                offset: x_range.offset,
                source: crate::StreamColumnSource::Scalar(&values),
            },
            crate::StreamRangeSourceBinding {
                id: "mixed-y",
                revision: 1,
                source_len: 8,
                offset: y_range.offset,
                source: crate::StreamColumnSource::Scalar(&values),
            },
        ],
    )
    .unwrap();
    wait_stream_slots(&mut r, 0);
    assert!(matches!(
        r.auto_stream_chart_request_ranges(chart_id).unwrap(),
        crate::AutoStreamingRangeRequest::AllSubmitted { .. }
    ));
    let frame = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id,
            view: &view,
        }])
        .unwrap();
    drop(frame);
    assert!(matches!(
        r.auto_stream_chart_request_ranges(chart_id).unwrap(),
        crate::AutoStreamingRangeRequest::Complete { .. }
    ));
    assert!(r.pool.slot("mixed-y").is_none());
    assert!(matches!(
        r.logical_column("mixed-x"),
        Some(crate::LogicalColumn::Streamed(source)) if source.revision == 1
    ));
    assert!(matches!(
        r.logical_column("mixed-y"),
        Some(crate::LogicalColumn::Streamed(source)) if source.revision == 1
    ));
}

#[test]
fn changed_resident_revision_rejects_handoff_commit_without_removing_resident_data() {
    let (device, queue) =
        crate::data_render::shared_device().expect("stream handoff test requires GPU");
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .unwrap();
    let original = crate::Column {
        data: (0..8).map(|value| value as f32).collect(),
        min: 0.0,
        max: 7.0,
    };
    r.add_column("stale-x", &original).unwrap();
    r.add_column("stale-y", &original).unwrap();
    let config = crate::default::default_config();
    let chart_id = r
        .register_chart(
            config.clone(),
            vec![declaration("stale", "stale-x", "stale-y")],
        )
        .unwrap();
    r.configure_streaming_runtime(limits(2)).unwrap();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let mut x = source("stale-x", 1);
    let mut y = source("stale-y", 1);
    x.len = 8;
    y.len = 8;
    r.request_resident_stream_handoff_with_config(
        chart_id,
        &view,
        config,
        vec![x, y],
        crate::StreamingChartOptions {
            size: (800, 600),
            clear_color: Color::WHITE,
            max_primitives_per_chunk: 8,
        },
    )
    .unwrap();

    let old_epoch = r.pool.allocation_epoch("stale-y").unwrap();
    let replacement = crate::Column {
        data: (0..8).map(|value| -(value as f32)).collect(),
        min: -7.0,
        max: 0.0,
    };
    r.upsert_column("stale-y", &replacement).unwrap();
    let new_epoch = r.pool.allocation_epoch("stale-y").unwrap();
    assert_ne!(old_epoch, new_epoch);

    let crate::AutoStreamingRangeRequest::Ready { ranges, .. } =
        r.auto_stream_chart_request_ranges(chart_id).unwrap()
    else {
        panic!("resident handoff did not retain its pending execution")
    };
    let x_range = ranges.iter().find(|range| range.id == "stale-x").unwrap();
    let y_range = ranges.iter().find(|range| range.id == "stale-y").unwrap();
    r.auto_stream_chart_submit_ranges(
        chart_id,
        &[
            crate::StreamRangeSourceBinding {
                id: "stale-x",
                revision: 1,
                source_len: 8,
                offset: x_range.offset,
                source: crate::StreamColumnSource::Scalar(&original),
            },
            crate::StreamRangeSourceBinding {
                id: "stale-y",
                revision: 1,
                source_len: 8,
                offset: y_range.offset,
                source: crate::StreamColumnSource::Scalar(&original),
            },
        ],
    )
    .unwrap();
    wait_stream_slots(&mut r, 0);
    assert!(matches!(
        r.auto_stream_chart_request_ranges(chart_id).unwrap(),
        crate::AutoStreamingRangeRequest::AllSubmitted { .. }
    ));
    let frame = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id,
            view: &view,
        }])
        .unwrap();
    drop(frame);
    assert!(r.auto_stream_chart_request_ranges(chart_id).is_err());

    assert!(r.active_stream_job(chart_id).is_none());
    assert_eq!(r.pool.allocation_epoch("stale-y"), Some(new_epoch));
    assert!(matches!(
        r.logical_column("stale-y"),
        Some(crate::LogicalColumn::Resident(_))
    ));
    assert!(!r.streaming_sources.contains_key("stale-x"));
    assert!(!r.streaming_sources.contains_key("stale-y"));
}

#[test]
fn automatic_stream_terminalizes_an_already_presented_display_after_receipts_complete() {
    let (mut r, chart_id, _) = renderer(2);
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    r.request_auto_streaming_chart(
        chart_id,
        &view,
        crate::StreamingChartOptions {
            size: (config.chart_area.0.width, config.chart_area.0.height),
            clear_color: Color::WHITE,
            max_primitives_per_chunk: 8,
        },
    )
    .unwrap();
    let x = crate::Column {
        data: (0..8).map(|value| value as f32).collect(),
        min: 0.0,
        max: 7.0,
    };
    let y = crate::Column {
        data: (0..8).map(|value| -(value as f32)).collect(),
        min: -7.0,
        max: 0.0,
    };
    let bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&x),
        },
        crate::StreamSourceBinding {
            id: "a",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];

    assert!(matches!(
        r.auto_stream_chart_step(chart_id, &bindings).unwrap(),
        crate::AutoStreamingProgress::Submitted { .. }
    ));
    let displayed_before_receipt = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id,
            view: &view,
        }])
        .unwrap();
    drop(displayed_before_receipt);
    wait_stream_slots(&mut r, 0);
    assert!(matches!(
        r.auto_stream_chart_step(chart_id, &bindings).unwrap(),
        crate::AutoStreamingProgress::AllSubmitted { .. }
    ));

    let terminal_refresh = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id,
            view: &view,
        }])
        .unwrap();
    drop(terminal_refresh);
    assert!(matches!(
        r.auto_stream_chart_step(chart_id, &bindings).unwrap(),
        crate::AutoStreamingProgress::Complete { .. }
    ));
}

#[test]
fn automatic_stream_collects_bounds_in_upload_pass_and_replays_after_axis_growth() {
    let (mut r, chart_id, _) = renderer(2);
    r.request_stream_auto_fit(chart_id, 0.0).unwrap();
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    r.request_auto_streaming_chart(chart_id, &view, options)
        .unwrap();
    let x = crate::Column {
        data: (10..18).map(|value| value as f32).collect(),
        min: 10.0,
        max: 17.0,
    };
    let y = crate::Column {
        data: (-20..-12).map(|value| value as f32).collect(),
        min: -20.0,
        max: -13.0,
    };
    let bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&x),
        },
        crate::StreamSourceBinding {
            id: "a",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&y),
        },
    ];

    let mut steps = 0usize;
    loop {
        steps += 1;
        assert!(steps < 128, "axis replays must converge");
        match r.auto_stream_chart_step(chart_id, &bindings).unwrap() {
            crate::AutoStreamingProgress::Submitted { .. } => {}
            crate::AutoStreamingProgress::Backpressure { .. } => wait_stream_slots(&mut r, 0),
            crate::AutoStreamingProgress::AllSubmitted { .. } => break,
            crate::AutoStreamingProgress::Complete { .. } => {
                panic!("completion cannot precede the final display refresh")
            }
        }
    }
    wait_stream_slots(&mut r, 0);
    let final_display = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id,
            view: &view,
        }])
        .unwrap();
    drop(final_display);
    assert!(matches!(
        r.auto_stream_chart_step(chart_id, &bindings).unwrap(),
        crate::AutoStreamingProgress::Complete { .. }
    ));

    let fitted = r.chart_config(chart_id).unwrap();
    assert_eq!((fitted.bottom_x.min, fitted.bottom_x.max), (10.0, 17.0));
    assert_eq!((fitted.left_y.min, fitted.left_y.max), (-20.0, -13.0));
    assert_eq!(
        r.logical_column("x").unwrap().statistics(),
        crate::StreamStatistics::Known(Some(crate::StreamBounds {
            min: 10.0,
            max: 17.0,
            min_positive: Some(10.0),
        }))
    );
    assert_eq!(
        r.logical_column("a").unwrap().statistics(),
        crate::StreamStatistics::Known(Some(crate::StreamBounds {
            min: -20.0,
            max: -13.0,
            min_positive: None,
        }))
    );
    let completed = r.stream_status(chart_id).unwrap();
    assert_eq!(completed.status, crate::StreamingState::Complete);
    assert_eq!(completed.submitted_primitives, completed.total_primitives);
    assert_eq!(completed.total_primitives, 7);
    assert!(!completed.auto_fit_pending);
    assert!(matches!(r.request_auto_streaming_chart(chart_id, &view, options).unwrap(),
        crate::AutoStreamingRequest::Complete { .. }));
    assert_eq!(r.stream_status(chart_id).unwrap().job_id, completed.job_id);
    r.request_stream_auto_fit(chart_id, 0.0).unwrap();
    assert!(!r.stream_status(chart_id).unwrap().auto_fit_pending);
    assert!(matches!(r.request_auto_streaming_chart(chart_id, &view, options).unwrap(),
        crate::AutoStreamingRequest::Complete { .. }));
    assert_eq!(r.stream_status(chart_id).unwrap().job_id, completed.job_id);
}

#[test]
fn streaming_status_is_read_only_and_adaptive_budget_preserves_pending_range_and_job() {
    let (mut r, chart, _) = renderer(2);
    let config = r.chart_config(chart).unwrap().clone();
    let view = r.create_chart_view(&Chart::new(config.clone()), config.chart_area.0).unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 4,
    };
    r.request_auto_streaming_chart(chart, &view, options).unwrap();
    let initial = r.stream_status(chart).unwrap();
    let usage = r.streaming_usage();
    let memory = r.gpu_memory_usage().total_bytes();
    for _ in 0..5 { assert_eq!(r.stream_status(chart).unwrap(), initial); }
    assert_eq!(r.streaming_usage(), usage);
    assert_eq!(r.gpu_memory_usage().total_bytes(), memory);
    assert_eq!(initial.submitted_primitives, 0);
    assert_eq!(initial.total_primitives, 7);
    let first = r.auto_stream_chart_request_ranges(chart).unwrap();
    r.set_stream_chunk_budget(chart, 1).unwrap();
    assert_eq!(r.auto_stream_chart_request_ranges(chart).unwrap(), first);
    assert!(r.set_stream_chunk_budget(chart, 0).is_err());
    assert!(r.set_stream_chunk_budget(chart, 5).is_err());
    assert!(matches!(r.request_auto_streaming_chart(chart, &view, options).unwrap(),
        crate::AutoStreamingRequest::Active { .. }));
    assert_eq!(r.stream_status(chart).unwrap().job_id, initial.job_id);
    let values = crate::Column { data: (0..8).map(|v| v as f32).collect(), min: 0.0, max: 7.0 };
    let bindings = [
        crate::StreamSourceBinding { id: "x", revision: 1, source: crate::StreamColumnSource::Scalar(&values) },
        crate::StreamSourceBinding { id: "a", revision: 1, source: crate::StreamColumnSource::Scalar(&values) },
    ];
    r.auto_stream_chart_step(chart, &bindings).unwrap();
    wait_stream_slots(&mut r, 0);
    let crate::AutoStreamingRangeRequest::Ready { ranges, submitted_primitives, .. } =
        r.auto_stream_chart_request_ranges(chart).unwrap() else { panic!("next range"); };
    assert_eq!(submitted_primitives, 4);
    assert!(ranges.iter().all(|range| range.offset == 4 && range.len == 2));
    assert_eq!(r.stream_status(chart).unwrap().job_id, initial.job_id);
}

#[test]
fn streaming_cancel_retains_counts_and_drains_only_own_reservations() {
    let (mut r, a, b) = renderer(3);
    let config = r.chart_config(a).unwrap().clone();
    let view = r.create_chart_view(&Chart::new(config.clone()), config.chart_area.0).unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    r.request_auto_streaming_chart(a, &view, options).unwrap();
    let values = crate::Column { data: vec![0.0; 8], min: 0.0, max: 0.0 };
    let bindings = [
        crate::StreamSourceBinding { id: "x", revision: 1, source: crate::StreamColumnSource::Scalar(&values) },
        crate::StreamSourceBinding { id: "a", revision: 1, source: crate::StreamColumnSource::Scalar(&values) },
    ];
    r.auto_stream_chart_step(a, &bindings).unwrap();
    let before = r.stream_status(a).unwrap();
    r.request_auto_streaming_chart(b, &view, options).unwrap();
    r.auto_stream_chart_request_ranges(b).unwrap();
    r.cancel_streaming_chart(a).unwrap();
    r.end_gpu_frame();
    wait_stream_slots(&mut r, 1);
    let cancelled = r.stream_status(a).unwrap();
    assert_eq!(cancelled.status, crate::StreamingState::Cancelled);
    assert_eq!(cancelled.job_id, before.job_id);
    assert_eq!(cancelled.submitted_primitives, before.submitted_primitives);
    assert_eq!(cancelled.total_primitives, before.total_primitives);
    assert_eq!(cancelled.in_flight_chunks, 0);
    assert_eq!(cancelled.reserved_gpu_bytes, 0);
    assert!(!r.is_streaming_chart(a));
    assert_eq!(r.stream_status(b).unwrap().in_flight_chunks, 1);
    assert!(r.is_streaming_chart(b));
}

#[test]
fn streaming_fit_requested_during_execution_is_part_of_request_identity() {
    let (mut r, chart, _) = renderer(2);
    let config = r.chart_config(chart).unwrap().clone();
    let view = r.create_chart_view(&Chart::new(config.clone()), config.chart_area.0).unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    r.request_auto_streaming_chart(chart, &view, options).unwrap();
    let first = r.stream_status(chart).unwrap();
    r.request_stream_auto_fit(chart, 0.05).unwrap();
    assert!(r.stream_status(chart).unwrap().auto_fit_pending);
    assert!(matches!(r.request_auto_streaming_chart(chart, &view, options).unwrap(),
        crate::AutoStreamingRequest::Started { .. }));
    assert_ne!(r.stream_status(chart).unwrap().job_id, first.job_id);
}

#[test]
fn automatic_stream_does_not_publish_old_statistics_into_a_new_source_revision() {
    let (mut r, chart_id, _) = renderer(2);
    r.request_stream_auto_fit(chart_id, 0.0).unwrap();
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    let options = crate::StreamingChartOptions {
        size: (config.chart_area.0.width, config.chart_area.0.height),
        clear_color: Color::WHITE,
        max_primitives_per_chunk: 2,
    };
    r.request_auto_streaming_chart(chart_id, &view, options)
        .unwrap();
    let old_x = crate::Column {
        data: (10..18).map(|value| value as f32).collect(),
        min: 10.0,
        max: 17.0,
    };
    let old_y = crate::Column {
        data: (-20..-12).map(|value| value as f32).collect(),
        min: -20.0,
        max: -13.0,
    };
    let old_bindings = [
        crate::StreamSourceBinding {
            id: "x",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&old_x),
        },
        crate::StreamSourceBinding {
            id: "a",
            revision: 1,
            source: crate::StreamColumnSource::Scalar(&old_y),
        },
    ];

    loop {
        match r.auto_stream_chart_step(chart_id, &old_bindings).unwrap() {
            crate::AutoStreamingProgress::Submitted { .. } => break,
            crate::AutoStreamingProgress::Backpressure { .. } => wait_stream_slots(&mut r, 0),
            crate::AutoStreamingProgress::AllSubmitted { .. }
            | crate::AutoStreamingProgress::Complete { .. } => {
                panic!("the old revision completed before replacement")
            }
        }
    }
    r.replace_streamed_columns(vec![source("x", 2), source("a", 2)])
        .unwrap();

    let mut steps = 0usize;
    loop {
        steps += 1;
        assert!(steps < 128, "the frozen old revision must still converge");
        match r.auto_stream_chart_step(chart_id, &old_bindings).unwrap() {
            crate::AutoStreamingProgress::Submitted { .. } => {}
            crate::AutoStreamingProgress::Backpressure { .. } => wait_stream_slots(&mut r, 0),
            crate::AutoStreamingProgress::AllSubmitted { .. } => break,
            crate::AutoStreamingProgress::Complete { .. } => {
                panic!("completion cannot precede the final display refresh")
            }
        }
    }
    wait_stream_slots(&mut r, 0);
    let final_display = r
        .prepare_registered(&[RegisteredChartDrawItem {
            chart_id,
            view: &view,
        }])
        .unwrap();
    drop(final_display);
    assert!(matches!(
        r.auto_stream_chart_step(chart_id, &old_bindings).unwrap(),
        crate::AutoStreamingProgress::Complete { .. }
    ));

    assert_eq!(
        r.logical_column("x").unwrap().statistics(),
        crate::StreamStatistics::Unknown
    );
    assert_eq!(
        r.logical_column("a").unwrap().statistics(),
        crate::StreamStatistics::Unknown
    );
    let crate::AutoStreamingRequest::Started { sources, .. } = r
        .request_auto_streaming_chart(chart_id, &view, options)
        .unwrap()
    else {
        panic!("the replacement revision did not become the next execution")
    };
    assert!(
        sources
            .iter()
            .filter(|source| source.id == "x" || source.id == "a")
            .all(|source| source.revision == 2)
    );
}

#[test]
fn interrupt_is_queued_only_for_automatic_streams() {
    let (mut r, chart_id, _) = renderer(2);
    let config = r.chart_config(chart_id).unwrap().clone();
    let view = r
        .create_chart_view(&Chart::new(config.clone()), config.chart_area.0)
        .unwrap();
    assert_eq!(
        r.interrupt_render(chart_id).unwrap(),
        crate::RenderInterruptStatus::Resident
    );
    r.request_auto_streaming_chart(
        chart_id,
        &view,
        crate::StreamingChartOptions {
            size: (config.chart_area.0.width, config.chart_area.0.height),
            clear_color: Color::WHITE,
            max_primitives_per_chunk: 2,
        },
    )
    .unwrap();
    assert_eq!(
        r.interrupt_render(chart_id).unwrap(),
        crate::RenderInterruptStatus::StreamCancelQueued
    );
    assert!(r.is_streaming_chart(chart_id));
    r.service_stream_requests();
    assert!(!r.is_streaming_chart(chart_id));
    assert_eq!(
        r.interrupt_render(chart_id).unwrap(),
        crate::RenderInterruptStatus::Resident
    );

    r.begin_streaming_chart(
        chart_id,
        &view,
        crate::StreamingChartOptions {
            size: (config.chart_area.0.width, config.chart_area.0.height),
            clear_color: Color::WHITE,
            max_primitives_per_chunk: 2,
        },
    )
    .unwrap();
    assert_eq!(
        r.interrupt_render(chart_id).unwrap(),
        crate::RenderInterruptStatus::Resident
    );
    assert!(r.is_streaming_chart(chart_id));
    r.cancel_streaming_chart(chart_id).unwrap();
}
