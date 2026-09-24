use renderer::{
    ChartId, ChartView, ColumnPairWriter, ColumnRangeWriteError, ColumnSource, ColumnUploadStats,
    Renderer, StreamBounds, StreamColumnSource, StreamSourceBinding, StreamingChartOptions,
    StreamingLimits, StreamingProgress,
};

struct VirtualColumn {
    len: usize,
}

impl ColumnSource for VirtualColumn {
    fn len(&self) -> usize {
        self.len
    }
    fn min(&self) -> f64 {
        0.0
    }
    fn max(&self) -> f64 {
        self.len.saturating_sub(1) as f64
    }
    fn write_f32_le_into(&self, dst: &mut [u8]) {
        for (index, bytes) in dst.chunks_exact_mut(4).enumerate() {
            bytes.copy_from_slice(&(index as f32).to_le_bytes());
        }
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        for index in 0..dst.len() {
            dst.write_pair(index, index as f32, 0.0);
        }
        ColumnUploadStats {
            min_positive: (self.len > 1).then_some(1.0),
        }
    }
    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        let end = start
            .checked_add(dst.len() as u64)
            .filter(|end| *end <= self.len as u64)
            .ok_or(ColumnRangeWriteError::InvalidRange)?;
        for index in 0..dst.len() {
            dst.write_pair(index, (start + index as u64) as f32, 0.0);
        }
        Ok(Some(StreamBounds {
            min: start as f64,
            max: end.saturating_sub(1) as f64,
            min_positive: if end <= 1 {
                None
            } else {
                Some(start.max(1) as f64)
            },
        }))
    }
}

#[allow(dead_code)]
fn drive_one_step(
    renderer: &mut Renderer,
    chart: ChartId,
    view: &ChartView,
    x: &VirtualColumn,
    y: &VirtualColumn,
) -> renderer::Result<StreamingProgress> {
    let sources = [
        StreamSourceBinding {
            id: "x",
            revision: 1,
            source: StreamColumnSource::Scalar(x),
        },
        StreamSourceBinding {
            id: "y",
            revision: 1,
            source: StreamColumnSource::Scalar(y),
        },
    ];
    renderer.stream_chart_step(chart, view, &sources)
}

#[allow(dead_code)]
fn configure_and_begin(
    renderer: &mut Renderer,
    chart: ChartId,
    view: &ChartView,
    limits: StreamingLimits,
    options: StreamingChartOptions,
) -> renderer::Result<()> {
    renderer.configure_streaming(limits)?;
    renderer.begin_streaming_chart(chart, view, options)
}

#[test]
fn public_streaming_types_do_not_expose_tickets_or_encoded_payloads() {
    let limits = StreamingLimits {
        max_active_charts: 2,
        max_in_flight_chunks: 2,
        max_columns_per_chunk: 7,
        max_chunk_input_bytes: 1 << 20,
        max_in_flight_gpu_bytes: 1 << 22,
    };
    let options = StreamingChartOptions {
        size: (640, 480),
        clear_color: renderer::Color::WHITE,
        max_primitives_per_chunk: 65_536,
    };
    assert_eq!(limits.max_columns_per_chunk, 7);
    assert_eq!(options.size, (640, 480));
}
