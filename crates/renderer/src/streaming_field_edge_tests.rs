use super::*;

fn supply(
    f: &mut Fixture,
    operation: Option<StreamingOperation>,
    ranges: &[crate::AutoStreamRange],
) {
    assert_eq!(ranges.len(), 1);
    let range = &ranges[0];
    let original = &f.columns.iter().find(|(id, _)| *id == range.id).unwrap().1;
    let payload =
        column(original.data[range.offset as usize..(range.offset + range.len) as usize].to_vec());
    let bindings = [crate::StreamRangeSourceBinding {
        id: &range.id,
        revision: range.revision,
        source_len: range.source_len,
        offset: range.offset,
        source: crate::StreamColumnSource::HiLo(&payload),
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

#[test]
fn heatmap_runtime_offscreen_draw_is_empty_but_fit_and_pick_remain_exact() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    for (origin, target) in [((0, 0), (16, 16)), ((800, 600), (360, 240))] {
        let mut f = setup(false, false, false, 1, 2);
        let mut config = f.chart.config().clone();
        config.chart_area.0.x = origin.0;
        config.chart_area.0.y = origin.1;
        f.chart = Chart::new(config.clone());
        f.renderer.set_chart_config(f.id, config).unwrap();
        f.view = f
            .renderer
            .create_chart_view(&f.chart, f.chart.config().chart_area.0)
            .unwrap();
        f.renderer
            .request_auto_streaming_chart(
                f.id,
                &f.view,
                crate::StreamingChartOptions {
                    size: target,
                    clear_color: Color::new(0.0, 0.0, 0.0, 0.0),
                    max_primitives_per_chunk: 2,
                },
            )
            .unwrap();
        let mut fit_reads = 0;
        for _ in 0..100 {
            match f.renderer.auto_stream_chart_request_ranges(f.id).unwrap() {
                crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                    assert!(
                        ranges
                            .iter()
                            .all(|r| (r.id == "x" || r.id == "y") && r.len == 1),
                        "empty target must not replay field pixels or Z"
                    );
                    fit_reads += ranges.len();
                    supply(&mut f, None, &ranges);
                }
                crate::AutoStreamingRangeRequest::Backpressure { .. } => f.renderer.wait_idle(),
                crate::AutoStreamingRangeRequest::AllSubmitted {
                    total_primitives, ..
                } => {
                    assert_eq!(total_primitives, 1);
                    break;
                }
                crate::AutoStreamingRangeRequest::Complete { .. } => break,
            }
        }
        assert_eq!(fit_reads, 4);
        finish_display(&mut f);
        let job = f.renderer.active_stream_job(f.id).unwrap();
        let draw = f
            .renderer
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|d| d.job == job)
            .unwrap();
        assert!(draw.field.is_none());
        let fitted = draw.field_fits[&0].unwrap();
        let snapshot = draw.auto_snapshot().unwrap();
        let displayed = snapshot.config.clone();
        let (device, queue) = data_render::shared_device().unwrap();
        let mut resident = Renderer::try_new(
            RendererDevice::new(device, queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap();
        for (id, values) in &f.columns {
            resident.add_hilo_column(*id, values).unwrap();
        }
        pollster::block_on(resident.ensure_errorbar_extent_engine()).unwrap();
        let expected_fit = pollster::block_on(
            resident
                .begin_series_fit_extent(&f.series[0])
                .unwrap()
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(fitted, expected_fit);
        let resident_id = resident
            .register_chart(displayed.clone(), f.series.clone())
            .unwrap();
        resident.enable_gpu_picking().unwrap();
        let area = displayed.data_area().unwrap().0;
        let position = [
            area.x as f32 + area.width as f32 * 0.44,
            area.y as f32 + area.height as f32 * 0.61,
        ];
        let query = GpuPickRequest {
            canvas_position_px: position,
            display_panel_px: displayed.chart_area.0,
            display_scale: 1.0,
            max_distance_px: 5.0,
        };
        let expected = pollster::block_on(
            resident
                .pick_chart_data(resident_id, query)
                .unwrap()
                .resolve(),
        )
        .unwrap();
        assert!(
            expected.is_some(),
            "chart-space query remains valid outside raster target"
        );
        let operation =
            pollster::block_on(f.renderer.begin_stream_pick_data(f.id, position, 5.0, 2)).unwrap();
        pump_pick(&mut f, operation);
        assert_eq!(
            pollster::block_on(f.renderer.finish_stream_pick_data(operation)).unwrap(),
            expected
        );
    }
}

struct PartialFailure {
    values: crate::Column<f64>,
    fail: std::cell::Cell<bool>,
}

impl crate::HiLoColumnSource for PartialFailure {
    fn len(&self) -> usize {
        self.values.data.len()
    }
    fn min(&self) -> f64 {
        self.values.min
    }
    fn max(&self) -> f64 {
        self.values.max
    }
    fn write_f32_pair_le_into(&self, _: &mut [u8]) {
        panic!("full-column writer forbidden");
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        _: crate::ColumnPairWriter<'_>,
    ) -> crate::ColumnUploadStats {
        panic!("full-column writer forbidden");
    }
    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut writer: crate::ColumnPairWriter<'_>,
    ) -> std::result::Result<Option<crate::StreamBounds>, crate::ColumnRangeWriteError> {
        if self.fail.replace(false) {
            writer.write_pair(0, 0.5, 0.0);
            return Err(crate::ColumnRangeWriteError::SourceFailed);
        }
        crate::HiLoColumnSource::write_f32_pair_range_into_with_stats(&self.values, start, writer)
    }
}

#[test]
fn heatmap_runtime_fit_and_pick_writer_failure_retire_before_tight_budget_retry() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    for picking in [false, true] {
        let mut f = setup(false, false, false, 1, 2);
        let operation = if picking {
            finish_display(&mut f);
            let area = f.chart.config().data_area().unwrap().0;
            let position = [
                area.x as f32 + area.width as f32 * 0.44,
                area.y as f32 + area.height as f32 * 0.61,
            ];
            Some(
                pollster::block_on(f.renderer.begin_stream_pick_data(f.id, position, 5.0, 2))
                    .unwrap(),
            )
        } else {
            None
        };
        let request = if let Some(op) = operation {
            f.renderer.request_stream_operation_ranges(op)
        } else {
            f.renderer.auto_stream_chart_request_ranges(f.id)
        }
        .unwrap();
        let crate::AutoStreamingRangeRequest::Ready { ranges, .. } = request else {
            panic!("axis range");
        };
        let range = &ranges[0];
        let original = &f.columns.iter().find(|(id, _)| *id == range.id).unwrap().1;
        let source = PartialFailure {
            values: column(
                original.data[range.offset as usize..(range.offset + range.len) as usize].to_vec(),
            ),
            fail: std::cell::Cell::new(true),
        };
        let bindings = [crate::StreamRangeSourceBinding {
            id: &range.id,
            revision: range.revision,
            source_len: range.source_len,
            offset: range.offset,
            source: crate::StreamColumnSource::HiLo(&source),
        }];
        f.renderer.end_gpu_frame();
        f.renderer.wait_idle();
        f.renderer.service_stream_requests();
        let before = f.renderer.gpu_memory_usage().total_bytes();
        let headroom = if picking {
            crate::data_render::stream_field::STEP_BYTES
        } else {
            crate::gpu_errorbar::GpuSeriesExtentTicket::FIELD_BYTES
        };
        let _ = f
            .renderer
            .set_memory_budget(Some(before + range.len * 16 + headroom));
        let failed = if let Some(op) = operation {
            f.renderer
                .submit_stream_operation_ranges(op, &bindings)
                .map(|_| ())
        } else {
            f.renderer
                .auto_stream_chart_submit_ranges(f.id, &bindings)
                .map(|_| ())
        };
        assert!(matches!(
            failed,
            Err(FiggyError::InvalidStreamSource { .. })
        ));
        f.renderer.wait_idle();
        f.renderer.service_stream_requests();
        assert_eq!(
            f.renderer.gpu_memory_usage().total_bytes(),
            before,
            "failed writer staging must retire without another submission"
        );
        if let Some(op) = operation {
            f.renderer
                .submit_stream_operation_ranges(op, &bindings)
                .unwrap();
        } else {
            f.renderer
                .auto_stream_chart_submit_ranges(f.id, &bindings)
                .unwrap();
        }
        let _ = f.renderer.set_memory_budget(None);
        if let Some(op) = operation {
            pump_pick(&mut f, op);
            assert!(
                pollster::block_on(f.renderer.finish_stream_pick_data(op))
                    .unwrap()
                    .is_some()
            );
        } else {
            finish_display(&mut f);
        }
    }
}
