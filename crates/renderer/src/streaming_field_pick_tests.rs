use super::*;

#[test]
fn heatmap_runtime_typed_pick_submission_failure_retries_without_advancing() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap();
    let mut f = setup(false, false, false, 1, 2);
    finish_display(&mut f);
    let area = f.chart.config().data_area().unwrap().0;
    let position = [
        area.x as f32 + area.width as f32 * 0.44,
        area.y as f32 + area.height as f32 * 0.61,
    ];
    let baseline =
        pollster::block_on(f.renderer.begin_stream_pick_data(f.id, position, 5.0, 2)).unwrap();
    pump_pick(&mut f, baseline);
    let expected = pollster::block_on(f.renderer.finish_stream_pick_data(baseline)).unwrap();
    assert!(expected.is_some());
    let operation =
        pollster::block_on(f.renderer.begin_stream_pick_data(f.id, position, 5.0, 2)).unwrap();
    let mut rejected = 0;
    for _ in 0..1000 {
        match f
            .renderer
            .request_stream_operation_ranges(operation)
            .unwrap()
        {
            crate::AutoStreamingRangeRequest::Ready { ranges, .. } => {
                assert_eq!(ranges.len(), 1);
                let range = &ranges[0];
                assert!(range.id == "x" || range.id == "y");
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
                    source: crate::StreamColumnSource::HiLo(&payload),
                }];
                let draw = f
                    .renderer
                    .stream_runtime
                    .as_ref()
                    .unwrap()
                    .draws
                    .iter()
                    .find(|draw| draw.job == operation.0)
                    .unwrap();
                let offset = draw.offset;
                f.renderer.reject_next_stream_completion_reserve_for_test();
                assert!(
                    f.renderer
                        .submit_stream_operation_ranges(operation, &bindings)
                        .is_err()
                );
                let draw = f
                    .renderer
                    .stream_runtime
                    .as_ref()
                    .unwrap()
                    .draws
                    .iter()
                    .find(|draw| draw.job == operation.0)
                    .unwrap();
                assert!(draw.pending.is_none());
                assert_eq!(draw.offset, offset);
                let crate::AutoStreamingRangeRequest::Ready { ranges: retry, .. } = f
                    .renderer
                    .request_stream_operation_ranges(operation)
                    .unwrap()
                else {
                    panic!("same source range must be retryable");
                };
                assert_eq!(
                    (&retry[0].id, retry[0].offset, retry[0].len),
                    (&range.id, range.offset, range.len)
                );
                f.renderer
                    .submit_stream_operation_ranges(operation, &bindings)
                    .unwrap();
                rejected += 1;
            }
            crate::AutoStreamingRangeRequest::Backpressure { .. } => f.renderer.wait_idle(),
            crate::AutoStreamingRangeRequest::AllSubmitted { .. }
            | crate::AutoStreamingRangeRequest::Complete { .. } => break,
        }
    }
    assert!(rejected > 1);
    let actual = pollster::block_on(f.renderer.finish_stream_pick_data(operation)).unwrap();
    assert_eq!(actual, expected);

    let cancelled =
        pollster::block_on(f.renderer.begin_stream_pick_data(f.id, position, 5.0, 2)).unwrap();
    let crate::AutoStreamingRangeRequest::Ready { ranges, .. } = f
        .renderer
        .request_stream_operation_ranges(cancelled)
        .unwrap()
    else {
        panic!("first axis range");
    };
    let range = &ranges[0];
    let original = &f.columns.iter().find(|(id, _)| *id == range.id).unwrap().1;
    let payload =
        column(original.data[range.offset as usize..(range.offset + range.len) as usize].to_vec());
    f.renderer.cancel_stream_operation(cancelled).unwrap();
    assert!(
        f.renderer
            .submit_stream_operation_ranges(
                cancelled,
                &[crate::StreamRangeSourceBinding {
                    id: &range.id,
                    revision: range.revision,
                    source_len: range.source_len,
                    offset: range.offset,
                    source: crate::StreamColumnSource::HiLo(&payload),
                }]
            )
            .is_err()
    );
    f.renderer.wait_idle();
    f.renderer.service_stream_requests();
    assert_eq!(f.renderer.streaming_usage().reserved_gpu_bytes, 0);
}
