//! Bounded, chart-local GPU copies of original source pairs. Unlike packed
//! visible rows these preserve adjacency and global indices for arc scans,
//! histogram envelopes and heatmap lattices. No CPU payload is retained.
//!
//! Admission is deliberately conservative: all declared columns must fit one
//! working-set buffer. Large sources continue through the bounded stream path.
//! Only a contiguous submitted prefix of each column is reusable; gaps cause a
//! normal source request. Metadata is O(columns), never O(rows or tickets).
use super::*;
use crate::gpu_memory::GpuResourceKind;
use crate::streaming_upload::UploadedColumn;

struct CachedColumn {
    id: ColumnId,
    revision: u64,
    len: u64,
    encoding: crate::StreamEncoding,
    offset: u64,
    covered: u64,
}

pub(super) struct SourceCache {
    work: TrackedBuffer,
    columns: Vec<CachedColumn>,
}

impl SourceCache {
    pub(super) fn bytes(&self) -> u64 {
        self.work.size()
    }

    fn column(&self, request: &StreamRequestedColumn) -> Option<&CachedColumn> {
        self.columns.iter().find(|c| {
            c.id == request.column
                && c.revision == request.range.revision
                && c.len == request.range.source_len
                && c.encoding == request.range.encoding
        })
    }

    fn resolve(&self, requests: &[StreamRequestedColumn]) -> Option<RecordedChunk> {
        let mut columns = Vec::new();
        columns.try_reserve_exact(requests.len()).ok()?;
        for request in requests {
            let c = self.column(request)?;
            if request.range.offset.checked_add(request.range.len)? > c.covered {
                return None;
            }
            columns.push(UploadedColumn {
                range: request.range,
                offset_bytes: c.offset.checked_add(request.range.offset.checked_mul(8)?)?,
                pair_bytes: request.range.len.checked_mul(8)?,
                statistics: None,
            });
        }
        Some(RecordedChunk::from_packed_view(self.work.clone(), columns))
    }

    fn publish(&mut self, requests: &[StreamRequestedColumn]) {
        for request in requests {
            if let Some(c) = self.columns.iter_mut().find(|c| c.id == request.column) {
                if request.range.offset <= c.covered {
                    c.covered = c.covered.max(request.range.offset + request.range.len);
                }
            }
        }
    }
}

impl Renderer {
    pub(in crate::renderer) fn enforce_source_cache_policy(&mut self) {
        if let Some(runtime) = &mut self.stream_runtime {
            for draw in &mut runtime.draws {
                if draw.source_cache.as_ref().is_some_and(|cache| {
                    !self
                        .auto_resident_working_set_limit
                        .is_some_and(|limit| cache.bytes() <= limit)
                }) {
                    draw.source_cache = None;
                    draw.source_cache_writable = false;
                }
            }
        }
    }

    /// A cache hit runs the ordinary stream executor, including its current
    /// transforms, lattice/arc preparation and queue admission. It is not a
    /// cached image. At most ONE ticket is processed by each host poll.
    pub(super) fn request_auto_data_chunk(
        &mut self,
        job: StreamJob,
    ) -> StreamResult<StreamDrawRequestStatus> {
        let status = self.request_chart_stream_draw(job)?;
        let StreamDrawRequestStatus::Ready(ticket) = status else {
            return Ok(status);
        };
        let cached = {
            let runtime = self.stream_runtime.as_ref().ok_or(StreamError::Stale)?;
            let draw = runtime
                .draws
                .iter()
                .find(|draw| draw.job == job)
                .ok_or(StreamError::Stale)?;
            let cache = draw.source_cache.as_ref();
            let request = runtime
                .requests
                .iter()
                .find(|r| r.ticket == ticket)
                .ok_or(StreamError::Stale)?;
            cache
                .filter(|cache| {
                    self.auto_resident_working_set_limit
                        .is_some_and(|limit| cache.bytes() <= limit)
                })
                .and_then(|cache| cache.resolve(&request.columns))
        };
        if let Some(chunk) = cached {
            // Statistics and fitting remain authoritative. If their coverage
            // is absent, ask the source; GPU bytes alone are not CPU statistics.
            if self
                .prepare_stream_statistics(ticket)?
                .plans
                .iter()
                .all(|p| p.ranges.is_empty())
            {
                let target = self
                    .stream_runtime
                    .as_ref()
                    .unwrap()
                    .draws
                    .iter()
                    .find(|draw| draw.job == job)
                    .unwrap()
                    .target
                    .clone();
                self.submit_chart_stream_draw_supply(
                    ticket,
                    StreamSupply::Cached(&chunk),
                    None,
                    &target,
                )?;
                return Ok(StreamDrawRequestStatus::Backpressure);
            }
        }
        Ok(status)
    }

    pub(super) fn accept_cached_source(
        &mut self,
        ticket: StreamTicket,
        chunk: &RecordedChunk,
    ) -> StreamResult<RecordedChunk> {
        self.validate_live_stream_request(ticket)?;
        if self
            .prepare_stream_statistics(ticket)?
            .plans
            .iter()
            .any(|p| !p.ranges.is_empty())
        {
            return Err(StreamError::WrongState.into());
        }
        let runtime = self.stream_runtime.as_mut().ok_or(StreamError::Stale)?;
        Ok(runtime.scheduler.accept_expected(ticket.ticket, |ranges| {
            if ranges.len() != chunk.columns.len()
                || ranges
                    .iter()
                    .zip(&chunk.columns)
                    .any(|(a, b)| *a != b.range)
            {
                return Err(StreamError::InvalidPayload);
            }
            let mut columns = Vec::new();
            columns
                .try_reserve_exact(chunk.columns.len())
                .map_err(|_| StreamError::AllocationFailed)?;
            columns.extend_from_slice(&chunk.columns);
            Ok(RecordedChunk::from_packed_view(chunk.work.clone(), columns))
        })?)
    }

    /// Optional admission must leave room for the stream itself. Never let a
    /// failed cache allocation turn a successful source upload into a failure.
    fn new_source_cache(&self, job: StreamJob, headroom: u64) -> Option<SourceCache> {
        let snapshot = self.auto_stream_snapshot(job)?;
        let limit = self.auto_resident_working_set_limit.filter(|n| *n != 0)?;
        let budget = self.memory_budget?;
        let mut total = 0u64;
        let mut columns = Vec::new();
        columns.try_reserve_exact(snapshot.sources.len()).ok()?;
        for (id, source) in &snapshot.sources {
            let offset = total;
            total = total.checked_add(source.len.checked_mul(8)?)?;
            if total > limit {
                return None;
            }
            columns.push(CachedColumn {
                id: id.clone(),
                revision: source.revision,
                len: source.len,
                encoding: source.encoding,
                offset,
                covered: 0,
            });
        }
        if total == 0
            || total > self.device.limits().max_buffer_size
            || total > u64::from(self.device.limits().max_storage_buffer_binding_size)
            || self
                .gpu_memory_usage()
                .checked_total_bytes()?
                .checked_add(total)?
                .checked_add(headroom)?
                .checked_add(
                    self.stream_runtime
                        .as_ref()?
                        .scheduler
                        .limits()
                        .max_in_flight_bytes,
                )?
                > budget
        {
            return None;
        }
        let buffer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // gpu-alloc: ViewResident
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("figgy bounded original source cache"),
                size: total,
                usage: wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            })
        }))
        .ok()?;
        Some(SourceCache {
            work: TrackedBuffer::new(&self.gpu_ledger, GpuResourceKind::ViewResident, buffer),
            columns,
        })
    }

    pub(super) fn record_source_cache(
        &mut self,
        ticket: StreamTicket,
        chunk: &RecordedChunk,
        encoder: &mut wgpu::CommandEncoder,
        headroom: u64,
    ) {
        let Some(runtime) = self.stream_runtime.as_ref() else {
            return;
        };
        let Some(draw) = runtime.draws.iter().find(|d| d.job == ticket.job) else {
            return;
        };
        let Some(request) = runtime.requests.iter().find(|r| r.ticket == ticket) else {
            return;
        };
        if request.selection
            || draw.auxiliary.is_some()
            || draw.view_candidate.is_some()
            || draw.view_cache.is_some()
            || !matches!(
                &draw.mode,
                StreamExecutionMode::Auto {
                    auto_fit_padding: None,
                    ..
                }
            )
        {
            return;
        }
        if !draw.source_cache_attempted {
            let cache = self.new_source_cache(ticket.job, headroom).map(Arc::new);
            let draw = self
                .stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|d| d.job == ticket.job)
                .unwrap();
            draw.source_cache_attempted = true;
            draw.source_cache_writable = cache.is_some();
            draw.source_cache = cache;
        }
        let runtime = self.stream_runtime.as_mut().unwrap();
        let draw = runtime.draws.iter().find(|d| d.job == ticket.job).unwrap();
        if !draw.source_cache_writable {
            return;
        }
        let Some(cache) = draw.source_cache.as_ref() else {
            return;
        };
        let request = runtime
            .requests
            .iter_mut()
            .find(|r| r.ticket == ticket)
            .unwrap();
        if request.columns.len() != chunk.columns.len() {
            return;
        }
        // All identities/ranges are validated before the encoder is changed.
        if request
            .columns
            .iter()
            .zip(&chunk.columns)
            .any(|(r, u)| cache.column(r).is_none() || r.range != u.range)
        {
            return;
        }
        for (r, uploaded) in request.columns.iter().zip(&chunk.columns) {
            let c = cache.column(r).unwrap();
            encoder.copy_buffer_to_buffer(
                &chunk.work,
                uploaded.offset_bytes,
                &cache.work,
                c.offset + r.range.offset * 8,
                uploaded.pair_bytes,
            );
        }
        request.source_cache_capture = true;
    }

    pub(super) fn publish_source_cache_capture(&mut self, ticket: StreamTicket) {
        let runtime = self.stream_runtime.as_mut().unwrap();
        let Some(request) = runtime.requests.iter_mut().find(|r| r.ticket == ticket) else {
            return;
        };
        if !std::mem::take(&mut request.source_cache_capture) {
            return;
        }
        if let Some(cache) = runtime
            .draws
            .iter_mut()
            .find(|d| d.job == ticket.job)
            .and_then(|d| d.source_cache.as_mut())
            .and_then(Arc::get_mut)
        {
            cache.publish(&request.columns);
        }
    }
}
