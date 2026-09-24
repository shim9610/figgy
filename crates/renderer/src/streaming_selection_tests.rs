use super::*;
use crate::config::{DataSelectionsConfig, PickedPointsConfig};
use crate::data_config::{
    BarOrientation, DataBarBinStyleConfig, DataBarStyleOverride, DataScatterPointStyleOverride,
};

fn column(data: Vec<f32>) -> crate::Column<f32> {
    crate::Column {
        min: data.iter().copied().fold(f32::INFINITY, f32::min),
        max: data.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        data,
    }
}

fn fixture() -> (
    Renderer,
    Renderer,
    ChartId,
    ChartView,
    Chart,
    Vec<SeriesConfig>,
    Vec<(&'static str, crate::Column<f32>)>,
) {
    fixture_with_slots(3)
}

fn fixture_with_slots(
    max_slots: usize,
) -> (
    Renderer,
    Renderer,
    ChartId,
    ChartView,
    Chart,
    Vec<SeriesConfig>,
    Vec<(&'static str, crate::Column<f32>)>,
) {
    fixture_with_display(max_slots, 1.0)
}

fn fixture_with_display(
    max_slots: usize,
    display_scale: f32,
) -> (
    Renderer,
    Renderer,
    ChartId,
    ChartView,
    Chart,
    Vec<SeriesConfig>,
    Vec<(&'static str, crate::Column<f32>)>,
) {
    let (device, queue) = data_render::shared_device().unwrap();
    let make = || {
        Renderer::try_new_with_sample_count(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Rgba8Unorm,
            8192,
            4,
        )
        .unwrap()
    };
    let mut stream = make();
    let mut resident = make();
    let values = vec![
        (
            "x",
            column((0..17).map(|i| 0.1 + i as f32 * 0.05).collect()),
        ),
        (
            "y",
            column((0..17).map(|i| 0.2 + (i % 11) as f32 * 0.06).collect()),
        ),
        ("index", column(vec![0.0; 17])),
    ];
    for (id, values) in &values {
        resident.add_column(*id, values).unwrap();
    }
    stream
        .register_streamed_columns(
            values
                .iter()
                .map(|(id, values)| crate::StreamColumn {
                    id: (*id).into(),
                    len: values.data.len() as u64,
                    revision: 1,
                    encoding: crate::StreamEncoding::ScalarF32,
                    replay: crate::StreamReplay::RandomAccess,
                    statistics: crate::StreamStatistics::Unknown,
                })
                .collect(),
        )
        .unwrap();
    stream
        .configure_streaming(crate::StreamingLimits {
            max_active_charts: 1,
            max_in_flight_chunks: max_slots,
            max_columns_per_chunk: 3,
            max_chunk_input_bytes: 256,
            max_in_flight_gpu_bytes: 1024,
        })
        .unwrap();
    let point = SeriesConfig {
        source_id: Some("source".into()),
        series_id: "point".into(),
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: Color::new(0.0, 0.2, 0.7, 0.4),
                point_shape: ScatterShape::CircleFilled,
                point_size: 0.0,
                point_style_index_column: Some("index".into()),
                point_style_table: Some(vec![DataScatterPointStyleConfig {
                    point_size: Some(9.0),
                    ..Default::default()
                }]),
                point_style_overrides: Some(vec![DataScatterPointStyleOverride {
                    index: 10,
                    style: DataScatterPointStyleConfig {
                        point_size: Some(17.0),
                        ..Default::default()
                    },
                }]),
            },
        },
    };
    let bar = SeriesConfig {
        source_id: Some("source".into()),
        series_id: "bar".into(),
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: Color::new(0.5, 0.2, 0.1, 0.2),
                border_color: Color::new(0.2, 0.3, 0.5, 0.3),
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 0.0,
                width_ratio: 1.0,
                orientation: BarOrientation::Vertical,
                bar_style_overrides: Some(vec![DataBarStyleOverride {
                    index: 10,
                    style: DataBarBinStyleConfig {
                        gap_px: Some(2.0),
                        width_ratio: Some(0.4),
                        ..Default::default()
                    },
                }]),
            },
        },
    };
    let series = vec![point, bar];
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
    let id = stream
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let display = Chart::new(chart.config().scaled(display_scale));
    let view = stream
        .create_chart_view(&display, display.config().chart_area.0)
        .unwrap();
    stream
        .request_auto_streaming_chart_with_display_scale(
            id,
            &view,
            display.config().clone(),
            display_scale,
            crate::StreamingChartOptions {
                size: (
                    display.config().chart_area.0.width,
                    display.config().chart_area.0.height,
                ),
                clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                max_primitives_per_chunk: 3,
            },
        )
        .unwrap();
    (stream, resident, id, view, chart, series, values)
}

fn select(config: &mut Config, indices: &[usize]) {
    config.picked_points = Some(PickedPointsConfig {
        refs: indices
            .iter()
            .map(|index| PickedPointRef {
                source_id: Some("source".into()),
                series_id: "point".into(),
                point_index: *index,
            })
            .collect(),
        ring_color: Color::new(1.0, 0.0, 0.0, 0.45),
        ring_width_px: 3.0,
        radius_extra_px: 4.0,
        ..Default::default()
    });
    config.picked_data = Some(DataSelectionsConfig {
        refs: indices
            .iter()
            .flat_map(|index| {
                [
                    PickedDataRef::Point {
                        source_id: Some("source".into()),
                        series_id: "point".into(),
                        point_index: *index,
                    },
                    PickedDataRef::HistogramBin {
                        source_id: Some("source".into()),
                        series_id: "bar".into(),
                        bin_index: *index,
                    },
                ]
            })
            .collect(),
        highlight_color: Color::new(0.0, 1.0, 0.0, 0.5),
        outline_width_px: 2.0,
        point_radius_extra_px: 7.0,
        ..Default::default()
    });
}

fn supply(
    r: &mut Renderer,
    ticket: StreamingSelectionTicket,
    ranges: &[crate::AutoStreamRange],
    values: &[(&str, crate::Column<f32>)],
) {
    let payload: Vec<_> = ranges
        .iter()
        .map(|range| {
            let values = &values
                .iter()
                .find(|(id, _)| *id == range.id)
                .unwrap()
                .1
                .data;
            column(values[range.offset as usize..(range.offset + range.len) as usize].to_vec())
        })
        .collect();
    let bindings: Vec<_> = ranges
        .iter()
        .zip(&payload)
        .map(|(range, data)| crate::StreamRangeSourceBinding {
            id: &range.id,
            revision: range.revision,
            source_len: range.source_len,
            offset: range.offset,
            source: crate::StreamColumnSource::Scalar(data),
        })
        .collect();
    r.submit_stream_selection_ranges(ticket, &bindings).unwrap();
}

fn pump(r: &mut Renderer, id: ChartId, values: &[(&str, crate::Column<f32>)]) {
    loop {
        match r.request_stream_selection_ranges(id).unwrap() {
            StreamingSelectionRequest::Ready { ticket, ranges, .. } => {
                supply(r, ticket, &ranges, values)
            }
            StreamingSelectionRequest::Backpressure { .. } => r.wait_idle(),
            StreamingSelectionRequest::Complete { .. } => break,
            StreamingSelectionRequest::Failed { .. } => panic!("unexpected failed selection"),
        }
    }
}

fn finish_export(
    r: &mut Renderer,
    operation: crate::StreamingOperation,
    values: &[(&str, crate::Column<f32>)],
) -> crate::RasterImage {
    loop {
        match r.request_stream_operation_ranges(operation).unwrap() {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                let payload: Vec<_> = ranges
                    .iter()
                    .map(|range| {
                        let data = &values
                            .iter()
                            .find(|(id, _)| *id == range.id)
                            .unwrap()
                            .1
                            .data;
                        column(
                            data[range.offset as usize..(range.offset + range.len) as usize]
                                .to_vec(),
                        )
                    })
                    .collect();
                let bindings: Vec<_> = ranges
                    .iter()
                    .zip(&payload)
                    .map(|(range, data)| crate::StreamRangeSourceBinding {
                        id: &range.id,
                        revision: range.revision,
                        source_len: range.source_len,
                        offset: range.offset,
                        source: crate::StreamColumnSource::Scalar(data),
                    })
                    .collect();
                r.submit_stream_operation_ranges(operation, &bindings)
                    .unwrap();
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. } => r.wait_idle(),
            crate::AutoStreamingRangeRequest::AllSubmitted { .. }
            | crate::AutoStreamingRangeRequest::Complete { .. } => break,
        }
    }
    pollster::block_on(r.finish_stream_export(operation)).unwrap()
}

#[cfg(test)]
fn screen(r: &mut Renderer, id: ChartId, view: &ChartView) -> Vec<u8> {
    let job = r.active_stream_job(id).unwrap();
    r.refresh_chart_stream_display(job, view).unwrap();
    let texture = wgpu::Texture::clone(r.chart_stream_display(job).unwrap());
    let row = texture.width() * 4;
    let padded = row.div_ceil(256) * 256;
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(texture.height()),
            },
        },
        texture.size(),
    );
    r.queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    r.wait_idle();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let mut pixels: Vec<_> = mapped
        .chunks_exact(padded as usize)
        .flat_map(|row_bytes| row_bytes[..row as usize].iter().copied())
        .collect();
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] != 0 && pixel[3] != 255 {
            let alpha = f32::from(pixel[3]) / 255.0;
            for channel in &mut pixel[..3] {
                *channel = (f32::from(*channel) / alpha).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    pixels
}

#[test]
fn stream_selection_preserves_data_ticket_and_atomically_replaces_ordered_suffix() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let (mut r, mut resident, id, view, mut chart, series, values) = fixture();
    let job = r.active_stream_job(id).unwrap();
    let StreamDrawRequestStatus::Ready(data_ticket) = r.request_chart_stream_draw(job).unwrap()
    else {
        panic!()
    };
    select(chart.config_mut(), &[10, 2, 10]);
    r.set_chart_config(id, chart.config().clone()).unwrap();
    pump(&mut r, id, &values);
    assert_eq!(
        r.request_chart_stream_draw(job).unwrap(),
        StreamDrawRequestStatus::Ready(data_ticket)
    );
    assert_eq!(r.stream_progress_counts(job).unwrap().0, 0);
    assert_eq!(
        r.stream_runtime.as_ref().unwrap().draws[0]
            .selection
            .ready
            .len(),
        9
    );
    let bindings: Vec<_> = values
        .iter()
        .map(|(id, values)| crate::StreamSourceBinding {
            id,
            revision: 1,
            source: crate::StreamColumnSource::Scalar(values),
        })
        .collect();
    loop {
        match r.auto_stream_chart_step(id, &bindings).unwrap() {
            crate::AutoStreamingProgress::AllSubmitted { .. }
            | crate::AutoStreamingProgress::Complete { .. } => break,
            crate::AutoStreamingProgress::Backpressure { .. } => r.wait_idle(),
            _ => {}
        }
    }
    r.wait_idle();
    let old = screen(&mut r, id, &view);
    r.auto_stream_chart_step(id, &bindings).unwrap();
    let progress = r.stream_progress_counts(job).unwrap();
    select(chart.config_mut(), &[3, 4, 3]);
    r.set_chart_config(id, chart.config().clone()).unwrap();
    let StreamingSelectionRequest::Ready { ticket, ranges, .. } =
        r.request_stream_selection_ranges(id).unwrap()
    else {
        panic!()
    };
    chart.config_mut().chart_title.text.segments =
        crate::text::rich_segments_from_text("Decoration only");
    r.set_chart_config(id, chart.config().clone()).unwrap();
    let StreamingSelectionRequest::Ready {
        ticket: unchanged, ..
    } = r.request_stream_selection_ranges(id).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        ticket, unchanged,
        "decoration must preserve the selected-row ticket"
    );
    assert_eq!(screen(&mut r, id, &view), old);
    supply(&mut r, ticket, &ranges, &values);
    assert_eq!(
        screen(&mut r, id, &view),
        old,
        "partial candidate must not replace ready overlay"
    );
    let StreamingSelectionRequest::Ready {
        ticket: suspended,
        ranges: suspended_ranges,
        ..
    } = r.request_stream_selection_ranges(id).unwrap()
    else {
        panic!()
    };
    r.suspend_stream_selection(id).unwrap();
    assert!(r.submit_stream_selection_ranges(suspended, &[]).is_err());
    assert!(r.abandon_stream_selection(suspended).is_err());
    let StreamingSelectionRequest::Ready {
        ticket: resumed,
        ranges: resumed_ranges,
        ..
    } = r.request_stream_selection_ranges(id).unwrap()
    else {
        panic!()
    };
    assert_ne!(resumed, suspended);
    assert_eq!(resumed_ranges, suspended_ranges);
    supply(&mut r, resumed, &resumed_ranges, &values);
    pump(&mut r, id, &values);
    assert_ne!(screen(&mut r, id, &view), old);
    assert_eq!(r.active_stream_job(id), Some(job));
    assert_eq!(r.stream_progress_counts(job).unwrap(), progress);
    assert!(r.submit_stream_selection_ranges(ticket, &[]).is_err());
    let selection_revision = r.chart_states[&id].revisions.selection;
    r.set_chart_config(id, chart.config().clone()).unwrap();
    assert!(
        matches!(r.request_stream_selection_ranges(id).unwrap(), StreamingSelectionRequest::Complete { revision } if revision == selection_revision)
    );
    for scale in [1.0, 2.0] {
        let expected = resident.export_panel_rgba(&chart, &series, scale).unwrap();
        let operation = r
            .begin_stream_export(id, scale, Color::new(0.0, 0.0, 0.0, 0.0), 3)
            .unwrap();
        chart
            .config_mut()
            .picked_points
            .as_mut()
            .unwrap()
            .radius_extra_px += 1.0;
        r.set_chart_config(id, chart.config().clone()).unwrap();
        pump(&mut r, id, &values);
        let actual = finish_export(&mut r, operation, &values);
        assert_eq!(
            actual
                .rgba
                .iter()
                .zip(&expected.rgba)
                .filter(|(a, b)| a != b)
                .count(),
            0,
            "selection export scale={scale}"
        );
    }
    select(chart.config_mut(), &[5]);
    r.set_chart_config(id, chart.config().clone()).unwrap();
    let StreamingSelectionRequest::Ready { ticket, .. } =
        r.request_stream_selection_ranges(id).unwrap()
    else {
        panic!()
    };
    let before = screen(&mut r, id, &view);
    r.abandon_stream_selection(ticket).unwrap();
    assert!(matches!(
        r.request_stream_selection_ranges(id).unwrap(),
        StreamingSelectionRequest::Failed { .. }
    ));
    assert_eq!(screen(&mut r, id, &view), before);
    chart
        .config_mut()
        .picked_data
        .as_mut()
        .unwrap()
        .outline_width_px += 0.5;
    r.set_chart_config(id, chart.config().clone()).unwrap();
    r.wait_idle();
    let bytes = r.gpu_memory_usage().total_bytes();
    r.memory_budget = Some(bytes);
    assert!(r.request_stream_selection_ranges(id).is_err());
    assert_eq!(r.gpu_memory_usage().total_bytes(), bytes);
    assert_eq!(screen(&mut r, id, &view), before);
    r.memory_budget = None;
    chart
        .config_mut()
        .picked_data
        .as_mut()
        .unwrap()
        .outline_width_px += 0.5;
    r.set_chart_config(id, chart.config().clone()).unwrap();
    pump(&mut r, id, &values);
    assert_ne!(screen(&mut r, id, &view), before);
}

#[test]
fn stream_selection_suspension_releases_single_slot_for_export_and_resumes_candidate() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let (mut r, mut resident, id, view, mut chart, series, values) = fixture_with_slots(1);
    let bindings: Vec<_> = values
        .iter()
        .map(|(id, values)| crate::StreamSourceBinding {
            id,
            revision: 1,
            source: crate::StreamColumnSource::Scalar(values),
        })
        .collect();
    loop {
        match r.auto_stream_chart_step(id, &bindings).unwrap() {
            crate::AutoStreamingProgress::Complete { .. } => break,
            crate::AutoStreamingProgress::Backpressure { .. }
            | crate::AutoStreamingProgress::AllSubmitted { .. } => {
                r.wait_idle();
                screen(&mut r, id, &view);
            }
            _ => {}
        }
    }
    let job = r.active_stream_job(id).unwrap();
    let old = screen(&mut r, id, &view);
    select(chart.config_mut(), &[10, 2, 10]);
    r.set_chart_config(id, chart.config().clone()).unwrap();
    let StreamingSelectionRequest::Ready { ticket, ranges, .. } =
        r.request_stream_selection_ranges(id).unwrap()
    else {
        panic!()
    };
    supply(&mut r, ticket, &ranges, &values);
    r.wait_idle();
    let StreamingSelectionRequest::Ready {
        ticket: suspended,
        ranges: requested,
        ..
    } = r.request_stream_selection_ranges(id).unwrap()
    else {
        panic!()
    };
    let candidate_len = r.stream_runtime.as_ref().unwrap().draws[0]
        .selection
        .candidate
        .len();
    assert_eq!(candidate_len, 1);
    let operation = r
        .begin_stream_export(id, 1.0, Color::new(0.0, 0.0, 0.0, 0.0), 3)
        .unwrap();
    assert!(matches!(
        r.request_stream_operation_ranges(operation).unwrap(),
        crate::AutoStreamingRangeRequest::Backpressure { .. }
    ));
    r.suspend_stream_selection(id).unwrap();
    assert_eq!(
        r.stream_runtime.as_ref().unwrap().draws[0]
            .selection
            .candidate
            .len(),
        candidate_len
    );
    assert!(r.submit_stream_selection_ranges(suspended, &[]).is_err());
    let actual = finish_export(&mut r, operation, &values);
    assert_eq!(
        actual.rgba,
        resident
            .export_panel_rgba(&chart, &series, 1.0)
            .unwrap()
            .rgba
    );
    assert_eq!(screen(&mut r, id, &view), old);
    assert_eq!(r.active_stream_job(id), Some(job));
    let StreamingSelectionRequest::Ready { ticket, ranges, .. } =
        r.request_stream_selection_ranges(id).unwrap()
    else {
        panic!()
    };
    assert_ne!(ticket, suspended);
    assert_eq!(ranges, requested);
    supply(&mut r, ticket, &ranges, &values);
    pump(&mut r, id, &values);
    assert_ne!(screen(&mut r, id, &view), old);
    select(chart.config_mut(), &[5]);
    r.set_chart_config(id, chart.config().clone()).unwrap();
    let StreamingSelectionRequest::Ready { ticket, .. } =
        r.request_stream_selection_ranges(id).unwrap()
    else {
        panic!()
    };
    r.cancel_chart_stream(id).unwrap();
    assert!(r.submit_stream_selection_ranges(ticket, &[]).is_err());
    r.end_gpu_frame();
    r.wait_idle();
    r.service_stream_requests();
    assert_eq!(r.stream_request_usage(), (0, 0, 0));
}

#[test]
fn stream_selection_and_data_match_resident_at_exact_display_scale() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    for scale in [0.5, 2.0] {
        let (mut r, mut resident, id, view, mut chart, series, values) =
            fixture_with_display(3, scale);
        let bindings: Vec<_> = values
            .iter()
            .map(|(id, values)| crate::StreamSourceBinding {
                id,
                revision: 1,
                source: crate::StreamColumnSource::Scalar(values),
            })
            .collect();
        loop {
            match r.auto_stream_chart_step(id, &bindings).unwrap() {
                crate::AutoStreamingProgress::Complete { .. } => break,
                crate::AutoStreamingProgress::Backpressure { .. }
                | crate::AutoStreamingProgress::AllSubmitted { .. } => {
                    r.wait_idle();
                    screen(&mut r, id, &view);
                }
                _ => {}
            }
        }
        let expected = resident.export_panel_rgba(&chart, &series, scale).unwrap();
        let actual = screen(&mut r, id, &view);
        assert_eq!(
            actual
                .iter()
                .zip(&expected.rgba)
                .filter(|(a, b)| a != b)
                .count(),
            0,
            "data display scale {scale}"
        );
        select(chart.config_mut(), &[10, 2, 10]);
        r.set_chart_config(id, chart.config().clone()).unwrap();
        pump(&mut r, id, &values);
        let expected = resident.export_panel_rgba(&chart, &series, scale).unwrap();
        let actual = screen(&mut r, id, &view);
        assert_eq!(
            actual
                .iter()
                .zip(&expected.rgba)
                .filter(|(a, b)| a != b)
                .count(),
            0,
            "selection display scale {scale}"
        );
    }
}
