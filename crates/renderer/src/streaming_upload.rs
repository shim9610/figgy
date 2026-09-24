//! Bounded borrowed-input upload. This is not a resident pool or a render pass.
//!
//! A chunk binds its entire work buffer; column offsets are pair-aligned, not
//! independent storage-binding offsets. No source payload survives recording.

use std::sync::Arc;

use crate::StreamBounds;
use crate::data::{COLUMN_VALUE_BYTES, ColumnPairWriter, StreamColumnSource};
use crate::gpu_memory::{GpuLedger, GpuResourceKind, TrackedBuffer};
use crate::streaming::{ColumnInput, ColumnRange, SourceEncoding, StreamError};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChunkUploadBudget {
    pub max_columns: usize,
    pub max_input_bytes: u64,
    pub max_work_buffer_bytes: u64,
    /// Staging plus work bytes, including any allocation padding.
    pub max_upload_bytes: u64,
    pub renderer_budget_bytes: u64,
    /// ALL pool live + retired bytes, including old slabs and pending copies,
    /// which are not in the external resource ledger. Never pass just capacity.
    pub pool_bytes: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct StreamSourceSlice<'a> {
    pub source: StreamColumnSource<'a>,
    pub source_len: u64,
    /// `None` means `source` spans the full logical column. `Some(offset)`
    /// means it contains only the pending request beginning at that offset.
    pub source_offset: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkUploadError {
    Input(StreamError),
    AllocationFailed,
    MappingFailed,
    WriterFailed,
    SourceWrite {
        index: usize,
        error: crate::ColumnRangeWriteError,
    },
}

impl From<StreamError> for ChunkUploadError {
    fn from(value: StreamError) -> Self {
        Self::Input(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct UploadedColumn {
    pub range: ColumnRange,
    pub offset_bytes: u64,
    pub pair_bytes: u64,
    /// `None`: statistics were not requested (already covered or a raw upload
    /// caller). `Some(None)`: the newly measured uncovered ranges had no finite
    /// GPU-reconstructible value.
    pub statistics: Option<Option<StreamBounds>>,
}

/// Global source subranges whose values have not been measured for this
/// revision. Ranges are sorted, disjoint, and lie inside the uploaded chunk.
#[derive(Debug, Default)]
pub(crate) struct ChunkStatisticsPlan {
    pub ranges: Vec<std::ops::Range<u64>>,
}

/// Own both allocation lifetimes until the caller releases the recorded work.
/// Dropping before submission retires the charges; only the existing submission
/// boundary/completion ledger may credit them. Any prepared clone of the work
/// buffer must carry `work.shared_charge()` along with the cloned GPU handle.
#[derive(Debug)]
pub(crate) struct RecordedChunk {
    pub work: TrackedBuffer,
    pub columns: Vec<UploadedColumn>,
    staging: Option<TrackedBuffer>,
}

impl RecordedChunk {
    pub(crate) fn from_packed_view(work: TrackedBuffer, columns: Vec<UploadedColumn>) -> Self {
        Self { work, columns, staging: None }
    }
    /// Resolve a checked subrange for the current chunk's queued draw. The
    /// work buffer may be overwritten by a later submission, so this handle
    /// is not a persistent pool registration or allocation identity. Callers
    /// must retain the work charge and validate their ticket/epoch separately.
    pub(crate) fn column_handle(
        &self,
        requested: ColumnRange,
    ) -> Result<crate::data_render::column_pool::ColumnHandle, StreamError> {
        requested.byte_len()?;
        let mut identity_found = false;
        let column = self
            .columns
            .iter()
            .find(|column| {
                let same_identity = column.range.column == requested.column
                    && column.range.revision == requested.revision
                    && column.range.source_len == requested.source_len
                    && column.range.encoding == requested.encoding;
                identity_found |= same_identity;
                same_identity
                    && requested
                        .offset
                        .checked_sub(column.range.offset)
                        .and_then(|local| local.checked_add(requested.len))
                        .is_some_and(|end| end <= column.range.len)
            })
            .ok_or(if identity_found {
                StreamError::InvalidRange
            } else {
                StreamError::Stale
            })?;
        let local = requested
            .offset
            .checked_sub(column.range.offset)
            .ok_or(StreamError::InvalidRange)?;
        if local
            .checked_add(requested.len)
            .ok_or(StreamError::Overflow)?
            > column.range.len
        {
            return Err(StreamError::InvalidRange);
        }
        let offset = column
            .offset_bytes
            .checked_add(checked_pair_bytes(local)?)
            .ok_or(StreamError::Overflow)?;
        let byte_size = checked_pair_bytes(requested.len)?;
        if offset.checked_add(byte_size).ok_or(StreamError::Overflow)? > self.work.size() {
            return Err(StreamError::InvalidRange);
        }
        // Existing primitive draw counts use u32; never truncate a large span.
        u32::try_from(requested.len).map_err(|_| StreamError::TooLarge)?;
        Ok(crate::data_render::column_pool::ColumnHandle {
            generation: 0,
            offset,
            byte_size,
            len_values: usize::try_from(requested.len).map_err(|_| StreamError::Overflow)?,
        })
    }

    pub(crate) fn charged_bytes(&self) -> u64 {
        self.work.charged_bytes() + self.staging.as_ref().map_or(0, TrackedBuffer::charged_bytes)
    }
}

fn checked_pair_bytes(len: u64) -> Result<u64, StreamError> {
    len.checked_mul(COLUMN_VALUE_BYTES as u64)
        .ok_or(StreamError::Overflow)
}

fn work_allocation_bytes(
    required: u64,
    preferred: u64,
    budget: ChunkUploadBudget,
    device_limit: u64,
    ledger_bytes: u64,
) -> u64 {
    let preferred = preferred.max(required);
    let fits = preferred <= budget.max_work_buffer_bytes.min(device_limit)
        && usize::try_from(preferred).is_ok()
        && required.checked_add(preferred).is_some_and(|bytes| bytes <= budget.max_upload_bytes)
        && ledger_bytes.checked_add(budget.pool_bytes)
            .and_then(|bytes| bytes.checked_add(required))
            .and_then(|bytes| bytes.checked_add(preferred))
            .is_some_and(|bytes| bytes <= budget.renderer_budget_bytes);
    if fits { preferred } else { required }
}

fn validate_layout(
    columns: &[ColumnInput<'_>],
    budget: ChunkUploadBudget,
    device_limit: u64,
    ledger_bytes: u64,
    reusable_work_bytes: u64,
) -> Result<(Vec<UploadedColumn>, u64), ChunkUploadError> {
    if columns.is_empty() || columns.len() > budget.max_columns {
        return Err(StreamError::InvalidRange.into());
    }
    let mut input_bytes = 0u64;
    let mut work_bytes = 0u64;
    // Validate every length and the complete budget before even allocating the
    // metadata vector, let alone a GPU buffer or invoking a source writer.
    for column in columns {
        let bytes = column.range.byte_len()?;
        if u64::try_from(column.bytes.len()).map_err(|_| StreamError::Overflow)? != bytes {
            return Err(StreamError::InvalidPayload.into());
        }
        input_bytes = input_bytes
            .checked_add(bytes)
            .ok_or(StreamError::Overflow)?;
        work_bytes = work_bytes
            .checked_add(checked_pair_bytes(column.range.len)?)
            .ok_or(StreamError::Overflow)?;
    }
    // Every value is eight bytes: mapping and buffer-copy padding are already
    // included, with no zero-sized buffers or unaligned final copies.
    let upload_bytes = work_bytes.checked_mul(2).ok_or(StreamError::Overflow)?;
    let new_work_bytes = if reusable_work_bytes >= work_bytes { 0 } else { work_bytes };
    let total = ledger_bytes
        .checked_add(budget.pool_bytes)
        .and_then(|bytes| bytes.checked_add(work_bytes))
        .and_then(|bytes| bytes.checked_add(new_work_bytes))
        .ok_or(StreamError::Overflow)?;
    if input_bytes > budget.max_input_bytes
        || work_bytes > budget.max_work_buffer_bytes.min(device_limit)
        || usize::try_from(work_bytes).is_err()
        || upload_bytes > budget.max_upload_bytes
        || total > budget.renderer_budget_bytes
    {
        return Err(StreamError::TooLarge.into());
    }
    let mut layout = Vec::new();
    layout
        .try_reserve_exact(columns.len())
        .map_err(|_| ChunkUploadError::AllocationFailed)?;
    let mut offset_bytes = 0;
    for column in columns {
        let pair_bytes = checked_pair_bytes(column.range.len)?;
        layout.push(UploadedColumn {
            range: column.range,
            offset_bytes,
            pair_bytes,
            statistics: None,
        });
        offset_bytes += pair_bytes; // The complete checked sum above bounds this.
    }
    Ok((layout, work_bytes))
}

fn validate_source_layout(
    columns: &[ColumnRange],
    budget: ChunkUploadBudget,
    device_limit: u64,
    ledger_bytes: u64,
    reusable_work_bytes: u64,
) -> Result<(Vec<UploadedColumn>, u64), ChunkUploadError> {
    if columns.is_empty() || columns.len() > budget.max_columns {
        return Err(StreamError::InvalidRange.into());
    }
    let mut input_bytes = 0u64;
    let mut work_bytes = 0u64;
    for &range in columns {
        input_bytes = input_bytes
            .checked_add(range.byte_len()?)
            .ok_or(StreamError::Overflow)?;
        work_bytes = work_bytes
            .checked_add(checked_pair_bytes(range.len)?)
            .ok_or(StreamError::Overflow)?;
    }
    let upload_bytes = work_bytes.checked_mul(2).ok_or(StreamError::Overflow)?;
    let new_work_bytes = if reusable_work_bytes >= work_bytes { 0 } else { work_bytes };
    let total = ledger_bytes
        .checked_add(budget.pool_bytes)
        .and_then(|bytes| bytes.checked_add(work_bytes))
        .and_then(|bytes| bytes.checked_add(new_work_bytes))
        .ok_or(StreamError::Overflow)?;
    if input_bytes > budget.max_input_bytes
        || work_bytes > budget.max_work_buffer_bytes.min(device_limit)
        || usize::try_from(work_bytes).is_err()
        || upload_bytes > budget.max_upload_bytes
        || total > budget.renderer_budget_bytes
    {
        return Err(StreamError::TooLarge.into());
    }
    let mut layout = Vec::new();
    layout
        .try_reserve_exact(columns.len())
        .map_err(|_| ChunkUploadError::AllocationFailed)?;
    let mut offset_bytes = 0;
    for &range in columns {
        let pair_bytes = checked_pair_bytes(range.len)?;
        layout.push(UploadedColumn {
            range,
            offset_bytes,
            pair_bytes,
            statistics: None,
        });
        offset_bytes += pair_bytes;
    }
    Ok((layout, work_bytes))
}

/// Record one copy, without submitting or registering data in `ColumnPool`.
pub(crate) fn record_chunk(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
) -> Result<RecordedChunk, ChunkUploadError> {
    record_chunk_maybe_collecting(device, encoder, ledger, budget, columns, None, None, 0)
}

/// Record the same single staging pass as [`record_chunk`], collecting encoded
/// logical extrema only inside each plan's uncovered source ranges. Statistics
/// are calculated while each pair is copied into staging; no second data pass
/// or retained payload is introduced.
pub(crate) fn record_chunk_collecting_statistics(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
    statistics_plans: &[ChunkStatisticsPlan],
) -> Result<RecordedChunk, ChunkUploadError> {
    record_chunk_collecting_statistics_reusing(
        device, encoder, ledger, budget, columns, statistics_plans, None, 0,
    )
}

pub(crate) fn record_chunk_collecting_statistics_reusing(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
    statistics_plans: &[ChunkStatisticsPlan],
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
) -> Result<RecordedChunk, ChunkUploadError> {
    record_chunk_collecting_statistics_reusing_observed(
        device, encoder, ledger, budget, columns, statistics_plans,
        reusable_work, preferred_work_bytes, &mut |_, _, _, _| {},
    )
}

pub(crate) fn record_chunk_collecting_statistics_reusing_observed(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
    statistics_plans: &[ChunkStatisticsPlan],
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
    observe: &mut dyn FnMut(usize, usize, f32, f32),
) -> Result<RecordedChunk, ChunkUploadError> {
    if statistics_plans.len() != columns.len() {
        return Err(StreamError::InvalidPayload.into());
    }
    for (column, plan) in columns.iter().zip(statistics_plans) {
        let end = column
            .range
            .offset
            .checked_add(column.range.len)
            .ok_or(StreamError::Overflow)?;
        let mut previous_end = column.range.offset;
        for range in &plan.ranges {
            if range.start < column.range.offset
                || range.end > end
                || range.start >= range.end
                || range.start < previous_end
            {
                return Err(StreamError::InvalidPayload.into());
            }
            previous_end = range.end;
        }
    }
    record_chunk_maybe_collecting_observed(
        device,
        encoder,
        ledger,
        budget,
        columns,
        Some(statistics_plans),
        reusable_work,
        preferred_work_bytes,
        Some(observe),
    )
}

/// Ask range-capable host sources to write directly into mapped staging. The
/// source references and mapped bytes are borrowed only for this call; no
/// encoded CPU payload is allocated or retained.
pub(crate) fn record_source_chunk_collecting_statistics_reusing<'a>(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnRange],
    source_at: impl FnMut(usize) -> StreamSourceSlice<'a>,
    statistics_plans: &[ChunkStatisticsPlan],
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
) -> Result<RecordedChunk, ChunkUploadError> {
    record_source_chunk_collecting_statistics_reusing_observed(
        device, encoder, ledger, budget, columns, source_at, statistics_plans,
        reusable_work, preferred_work_bytes, &mut |_, _, _, _| {},
    )
}

pub(crate) fn record_source_chunk_collecting_statistics_reusing_observed<'a>(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnRange],
    mut source_at: impl FnMut(usize) -> StreamSourceSlice<'a>,
    statistics_plans: &[ChunkStatisticsPlan],
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
    observe: &mut dyn FnMut(usize, usize, f32, f32),
) -> Result<RecordedChunk, ChunkUploadError> {
    if statistics_plans.len() != columns.len() {
        return Err(StreamError::InvalidPayload.into());
    }
    for (index, (range, plan)) in columns.iter().zip(statistics_plans).enumerate() {
        let supplied = source_at(index);
        if supplied.source.encoding() != range.encoding || supplied.source_len != range.source_len
        {
            return Err(StreamError::InvalidPayload.into());
        }
        let supplied_len =
            u64::try_from(supplied.source.len()).map_err(|_| StreamError::Overflow)?;
        match supplied.source_offset {
            None if supplied_len == range.source_len => {}
            Some(offset) if offset == range.offset && supplied_len == range.len => {}
            _ => return Err(StreamError::InvalidPayload.into()),
        }
        let end = range
            .offset
            .checked_add(range.len)
            .ok_or(StreamError::Overflow)?;
        let mut previous_end = range.offset;
        for measured in &plan.ranges {
            if measured.start < range.offset
                || measured.end > end
                || measured.start >= measured.end
                || measured.start < previous_end
            {
                return Err(StreamError::InvalidPayload.into());
            }
            previous_end = measured.end;
        }
    }
    let limits = device.limits();
    let ledger_bytes = ledger.total_bytes();
    let (mut layout, work_bytes) = validate_source_layout(
        columns,
        budget,
        limits
            .max_buffer_size
            .min(u64::from(limits.max_storage_buffer_binding_size)),
        ledger_bytes,
        reusable_work.map_or(0, |work| work.size()),
    )?;
    let create = |label, size, usage, mapped_at_creation| {
        create_buffer_checked(
            device,
            &wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation,
            },
        )
        .map(|buffer| TrackedBuffer::new(ledger, GpuResourceKind::StreamingUpload, buffer))
    };
    let staging = create("stream chunk staging", work_bytes, wgpu::BufferUsages::COPY_SRC, true)?;
    let written: Result<(), ChunkUploadError> = (|| {
        let mut mapped = staging
            .slice(..)
            .get_mapped_range_mut()
            .map_err(|_| ChunkUploadError::MappingFailed)?;
        for (index, column) in layout.iter_mut().enumerate() {
            let start = usize::try_from(column.offset_bytes).map_err(|_| StreamError::Overflow)?;
            let end = usize::try_from(column.offset_bytes + column.pair_bytes)
                .map_err(|_| StreamError::Overflow)?;
            let supplied = source_at(index);
            let bounds = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut on_pair = |row, hi, lo| observe(index, row, hi, lo);
                supplied.source.write_range(
                    supplied.source_offset.map_or(column.range.offset, |_| 0),
                    ColumnPairWriter::new_observed(mapped.slice(start..end), &mut on_pair),
                )
            }))
            .map_err(|_| ChunkUploadError::SourceWrite {
                index,
                error: crate::ColumnRangeWriteError::SourceFailed,
            })?
            .map_err(|error| ChunkUploadError::SourceWrite { index, error })?;
            crate::streaming_source::validate_statistics(
                column.range.len,
                crate::StreamStatistics::Known(bounds),
            )
            .map_err(|_| ChunkUploadError::SourceWrite {
                index,
                error: crate::ColumnRangeWriteError::SourceFailed,
            })?;
            if !statistics_plans[index].ranges.is_empty() {
                column.statistics = Some(bounds);
            }
        }
        Ok(())
    })();
    staging.unmap();
    written?;
    let work = if let Some(work) = reusable_work.filter(|work| work.size() >= work_bytes) {
        work.clone()
    } else {
        let capacity = work_allocation_bytes(
            work_bytes, preferred_work_bytes, budget,
            limits.max_buffer_size.min(u64::from(limits.max_storage_buffer_binding_size)),
            ledger_bytes,
        );
        create(
            "stream chunk work",
            capacity,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            false,
        )?
    };
    encoder.copy_buffer_to_buffer(&staging, 0, &work, 0, work_bytes);
    Ok(RecordedChunk {
        work,
        columns: layout,
        staging: Some(staging),
    })
}

fn record_chunk_maybe_collecting(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
    statistics_plans: Option<&[ChunkStatisticsPlan]>,
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
) -> Result<RecordedChunk, ChunkUploadError> {
    record_chunk_maybe_collecting_observed(
        device, encoder, ledger, budget, columns, statistics_plans,
        reusable_work, preferred_work_bytes, None,
    )
}

fn record_chunk_maybe_collecting_observed(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
    statistics_plans: Option<&[ChunkStatisticsPlan]>,
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
    observe: Option<&mut dyn FnMut(usize, usize, f32, f32)>,
) -> Result<RecordedChunk, ChunkUploadError> {
    let mut measured = statistics_plans.map(|_| Vec::new());
    if let Some(measured) = &mut measured {
        measured
            .try_reserve_exact(columns.len())
            .map_err(|_| ChunkUploadError::AllocationFailed)?;
    }
    let mut column_index = 0usize;
    let mut chunk = record_chunk_with_observed(
        device,
        encoder,
        ledger,
        budget,
        columns,
        |input, mut dst| {
            let plan = statistics_plans.map(|plans| &plans[column_index]);
            column_index += 1;
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            let mut min_positive = f64::INFINITY;
            let collect_ranges = plan.map_or(&[][..], |plan| plan.ranges.as_slice());
            let mut collect_index = 0usize;
            let stride = input.range.encoding.bytes_per_value() as usize;
            for (index, bytes) in input.bytes.chunks_exact(stride).enumerate() {
                let hi = f32::from_le_bytes(bytes[..4].try_into().unwrap());
                let lo = match input.range.encoding {
                    SourceEncoding::ScalarF32 => 0.0,
                    SourceEncoding::HiLoF32 => f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                };
                dst.write_pair(index, hi, lo);
                let global_index = input.range.offset + index as u64;
                while collect_index < collect_ranges.len()
                    && collect_ranges[collect_index].end <= global_index
                {
                    collect_index += 1;
                }
                let collect = collect_ranges.get(collect_index).is_some_and(|range| {
                    range.start <= global_index && global_index < range.end
                });
                if collect {
                    let gpu_value = hi + lo;
                    let value = hi as f64 + lo as f64;
                    let value = if value == 0.0 { 0.0 } else { value };
                    if gpu_value.is_finite() && value.is_finite() {
                        min = min.min(value);
                        max = max.max(value);
                        if value > 0.0 {
                            min_positive = min_positive.min(value);
                        }
                    }
                }
            }
            if let Some(measured) = &mut measured {
                measured.push(plan.filter(|plan| !plan.ranges.is_empty()).map(|_| {
                    min.is_finite().then_some(StreamBounds {
                        min,
                        max,
                        min_positive: min_positive.is_finite().then_some(min_positive),
                    })
                }));
            }
            Ok(())
        },
        reusable_work,
        preferred_work_bytes,
        observe,
    )?;
    if let Some(measured) = measured {
        for (column, statistics) in chunk.columns.iter_mut().zip(measured) {
            column.statistics = statistics;
        }
    }
    Ok(chunk)
}

fn record_chunk_with(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
    write: impl FnMut(&ColumnInput<'_>, ColumnPairWriter<'_>) -> Result<(), ChunkUploadError>,
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
) -> Result<RecordedChunk, ChunkUploadError> {
    record_chunk_with_observed(
        device, encoder, ledger, budget, columns, write, reusable_work,
        preferred_work_bytes, None,
    )
}

fn record_chunk_with_observed(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
    write: impl FnMut(&ColumnInput<'_>, ColumnPairWriter<'_>) -> Result<(), ChunkUploadError>,
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
    observe: Option<&mut dyn FnMut(usize, usize, f32, f32)>,
) -> Result<RecordedChunk, ChunkUploadError> {
    record_chunk_with_factory_observed(device, encoder, ledger, budget, columns, write, reusable_work, preferred_work_bytes, observe, |desc| {
        create_buffer_checked(device, desc)
    })
}

fn create_buffer_checked(
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
) -> Result<wgpu::Buffer, ChunkUploadError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // gpu-alloc: StreamingUpload
        device.create_buffer(desc)
    }))
    .map_err(|_| ChunkUploadError::AllocationFailed)
}

fn record_chunk_with_factory(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
    write: impl FnMut(&ColumnInput<'_>, ColumnPairWriter<'_>) -> Result<(), ChunkUploadError>,
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
    allocate: impl FnMut(&wgpu::BufferDescriptor<'_>) -> Result<wgpu::Buffer, ChunkUploadError>,
) -> Result<RecordedChunk, ChunkUploadError> {
    record_chunk_with_factory_observed(
        device, encoder, ledger, budget, columns, write, reusable_work,
        preferred_work_bytes, None, allocate,
    )
}

fn record_chunk_with_factory_observed(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    ledger: &Arc<GpuLedger>,
    budget: ChunkUploadBudget,
    columns: &[ColumnInput<'_>],
    mut write: impl FnMut(&ColumnInput<'_>, ColumnPairWriter<'_>) -> Result<(), ChunkUploadError>,
    reusable_work: Option<&TrackedBuffer>,
    preferred_work_bytes: u64,
    mut observe: Option<&mut dyn FnMut(usize, usize, f32, f32)>,
    mut allocate: impl FnMut(&wgpu::BufferDescriptor<'_>) -> Result<wgpu::Buffer, ChunkUploadError>,
) -> Result<RecordedChunk, ChunkUploadError> {
    let limits = device.limits();
    let ledger_bytes = ledger.total_bytes();
    let (layout, work_bytes) = validate_layout(
        columns,
        budget,
        limits
            .max_buffer_size
            .min(u64::from(limits.max_storage_buffer_binding_size)),
        ledger_bytes,
        reusable_work.map_or(0, |work| work.size()),
    )?;
    let mut create = |label, size, usage, mapped_at_creation| {
        allocate(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation,
        })
        .map(|buffer| TrackedBuffer::new(ledger, GpuResourceKind::StreamingUpload, buffer))
    };
    let staging = create("stream chunk staging", work_bytes, wgpu::BufferUsages::COPY_SRC, true)?;
    // Keep the caller's encoder entirely untouched until all writers succeed.
    // Unwinding releases the mapped view before unmapping or dropping staging.
    let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut mapped = staging
            .slice(..)
            .get_mapped_range_mut()
            .map_err(|_| ChunkUploadError::MappingFailed)?;
        for (column_index, (input, column)) in columns.iter().zip(&layout).enumerate() {
            let start = column.offset_bytes as usize;
            let end = (column.offset_bytes + column.pair_bytes) as usize;
            if let Some(observe) = observe.as_mut() {
                let mut on_pair = |row, hi, lo| observe(column_index, row, hi, lo);
                write(input, ColumnPairWriter::new_observed(mapped.slice(start..end), &mut on_pair))?;
            } else {
                write(input, ColumnPairWriter::new(mapped.slice(start..end)))?;
            }
        }
        Ok(())
    }))
    .unwrap_or(Err(ChunkUploadError::WriterFailed));
    staging.unmap();
    written?;
    let work = if let Some(work) = reusable_work.filter(|work| work.size() >= work_bytes) {
        work.clone()
    } else {
        let capacity = work_allocation_bytes(
            work_bytes, preferred_work_bytes, budget,
            limits.max_buffer_size.min(u64::from(limits.max_storage_buffer_binding_size)),
            ledger_bytes,
        );
        create(
            "stream chunk work",
            capacity,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            false,
        )?
    };
    encoder.copy_buffer_to_buffer(&staging, 0, &work, 0, work_bytes);
    Ok(RecordedChunk {
        work,
        columns: layout,
        staging: Some(staging),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real resident pool versus borrowed chunk work, through the very same
    /// precise primitive draw routine. Separate Load passes exercise progress;
    /// line chunks own consecutive segments and include the next-point halo.
    #[test]
    fn gpu_chunk_precise_scatter_and_solid_line_match_resident_pixels() {
        use crate::data_render::column_pool::{ColumnPool, GpuAllocCtx};
        use crate::data_render::*;
        let (device, queue) = shared_device().expect("chunk draw requires GPU; no skip");
        let ledger = Arc::new(GpuLedger::new());
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        let mut pool = ColumnPool::new(ctx, 4096).unwrap();
        // The low lane matters: the high lanes all round to the same value.
        let xs = crate::data::Column {
            data: vec![
                1e9 + 1.,
                1e9 + 3.,
                1e9 + 5.,
                1e9 + 5.,
                1e9 + 7.,
                1e9 + 8.,
                1e9 + 2.,
            ],
            min: 1e9,
            max: 1e9 + 10.,
        };
        let ys = crate::data::Column {
            data: vec![2., 8., 4., 4., f64::NAN, 7., 6.],
            min: 0.,
            max: 10.,
        };
        let resident_x = pool.add_hilo_column("x".into(), &xs, ctx).unwrap();
        let resident_y = pool.add_hilo_column("y".into(), &ys, ctx).unwrap();
        let packed = |values: &[f64]| -> Vec<u8> {
            values
                .iter()
                .flat_map(|&v| {
                    let (hi, lo) = crate::data::split_f64_to_f32_pair(v);
                    [hi.to_le_bytes(), lo.to_le_bytes()].concat()
                })
                .collect()
        };
        let xb = packed(&xs.data);
        let yb = packed(&ys.data);
        let tbgl = create_scatter_transform_bind_group_layout(&device);
        let sbgl = create_style_bind_group_layout(&device);
        let shaders = ShaderModules::new(&device);
        let quad = create_unit_centered_quad_vertex_buffer(&device);
        let style = PrimitiveStyle {
            color_premul: [0.15, 0.25, 0.4, 0.5],
            line_width_px: 3.,
            point_radius_px: 5.,
            ..bytemuck::Zeroable::zeroed()
        };
        let sb = create_style_uniform_buffer(&device, &style);
        let sbg = create_style_bind_group(&device, &sbgl, &sb);
        for samples in [1, 4] {
            let line_pipeline = create_line_columnar_pipeline_with_sample_count(
                &device,
                &shaders.line,
                &tbgl,
                &sbgl,
                wgpu::TextureFormat::Rgba8Unorm,
                samples,
            );
            let scatter_pipeline = create_scatter_columnar_pipeline_with_sample_count(
                &device,
                &shaders.scatter,
                &tbgl,
                &sbgl,
                wgpu::TextureFormat::Rgba8Unorm,
                samples,
            );
            for reversed in [false, true] {
                let transform = ScatterTransform {
                    data_min: [1e9, 0.],
                    data_max: [1e9, 10.],
                    data_min_lo: [0., 0.],
                    data_max_lo: [10., 0.],
                    scale_log: [0.; 2],
                    pixel_to_ndc: [2. / 64.; 2],
                    data_to_panel_scale: [if reversed { -1. } else { 1. }, 1.],
                    data_to_panel_offset: [if reversed { 1. } else { 0. }, 0.],
                    style_params: [[0.; 4]; 3],
                };
                let tb = create_scatter_transform_uniform_buffer(&device, &transform);
                let tbg = create_scatter_transform_bind_group(&device, &tbgl, &tb);
                for line in [false, true] {
                    let render = |chunk_size: Option<usize>| -> Vec<u8> {
                        let desc = wgpu::TextureDescriptor {
                            label: None,
                            size: wgpu::Extent3d {
                                width: 64,
                                height: 64,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        };
                        let target = device.create_texture(&desc);
                        let view = target.create_view(&Default::default());
                        let msaa = (samples > 1).then(|| {
                            device.create_texture(&wgpu::TextureDescriptor {
                                sample_count: samples,
                                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                                ..desc
                            })
                        });
                        let msaa_view = msaa.as_ref().map(|t| t.create_view(&Default::default()));
                        let mut encoder = device.create_command_encoder(&Default::default());
                        let total = xs.data.len() - usize::from(line);
                        let step = chunk_size.unwrap_or(total);
                        for start in (0..total).step_by(step) {
                            let end = (start + step).min(total) + usize::from(line);
                            let make_range = |column| ColumnRange {
                                column,
                                revision: 1,
                                source_len: xs.data.len() as u64,
                                offset: start as u64,
                                len: (end - start) as u64,
                                encoding: SourceEncoding::HiLoF32,
                            };
                            let chunk = chunk_size.map(|_| {
                                record_chunk(
                                    &device,
                                    &mut encoder,
                                    &ledger,
                                    budget(),
                                    &[
                                        ColumnInput {
                                            range: make_range(0),
                                            bytes: &xb[start * 8..end * 8],
                                        },
                                        ColumnInput {
                                            range: make_range(1),
                                            bytes: &yb[start * 8..end * 8],
                                        },
                                    ],
                                )
                                .unwrap()
                            });
                            let (buffer, x, y) = match chunk.as_ref() {
                                Some(c) => (
                                    &*c.work,
                                    c.column_handle(make_range(0)).unwrap(),
                                    c.column_handle(make_range(1)).unwrap(),
                                ),
                                None => (pool.buffer(), resident_x, resident_y),
                            };
                            let layers = SeriesLayers {
                                field: None,
                                bar: None,
                                contour: None,
                                errorbar: None,
                                line_extra: None,
                                line: line.then_some(ColumnLineLayer {
                                    pipeline: &line_pipeline,
                                    transform_bg: &tbg,
                                    style_bg: &sbg,
                                    pool_buffer: buffer,
                                    x,
                                    y,
                                    arc: None,
                                    verts_per_instance: 4,
                                    texture_bg: None,
                                }),
                                scatter: (!line).then_some(ColumnScatterLayer {
                                    pipeline: &scatter_pipeline,
                                    transform_bg: &tbg,
                                    style_bg: &sbg,
                                    style_map_bg: None,
                                    quad_vb: &quad,
                                    pool_buffer: buffer,
                                    x,
                                    y,
                                    style_index: None,
                                    texture_bg: None,
                                }),
                                selected_bars: vec![],
                                selected_fields: vec![],
                                picked: vec![],
                            };
                            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: None,
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: msaa_view.as_ref().unwrap_or(&view),
                                    depth_slice: None,
                                    resolve_target: msaa_view.as_ref().map(|_| &view),
                                    ops: wgpu::Operations {
                                        load: if start == 0 {
                                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                                        } else {
                                            wgpu::LoadOp::Load
                                        },
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                            issue_series_data(&mut pass, &layers);
                        }
                        let readback = device.create_buffer(&wgpu::BufferDescriptor {
                            label: None,
                            size: 64 * 256,
                            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                            mapped_at_creation: false,
                        });
                        encoder.copy_texture_to_buffer(
                            wgpu::TexelCopyTextureInfo {
                                texture: &target,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::TexelCopyBufferInfo {
                                buffer: &readback,
                                layout: wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(256),
                                    rows_per_image: Some(64),
                                },
                            },
                            desc.size,
                        );
                        let (tx, rx) = std::sync::mpsc::channel();
                        encoder.map_buffer_on_submit(
                            &readback,
                            wgpu::MapMode::Read,
                            0..64 * 256,
                            move |r| {
                                let _ = tx.send(r);
                            },
                        );
                        let submission = queue.submit([encoder.finish()]);
                        device
                            .poll(wgpu::PollType::Wait {
                                submission_index: Some(submission),
                                timeout: Some(std::time::Duration::from_secs(30)),
                            })
                            .unwrap();
                        rx.recv_timeout(std::time::Duration::from_secs(30))
                            .unwrap()
                            .unwrap();
                        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
                        readback.unmap();
                        ledger.complete_retirement(ledger.take_retirement());
                        bytes
                    };
                    let resident = render(None);
                    assert!(resident.iter().any(|&b| b != 0), "nonempty reference");
                    for step in [1, 2, 3, 4] {
                        let streamed = render(Some(step));
                        let diffs: Vec<_> = resident
                            .iter()
                            .zip(&streamed)
                            .enumerate()
                            .filter(|(_, (a, b))| a != b)
                            .take(8)
                            .collect();
                        assert!(
                            diffs.is_empty(),
                            "line={line}, samples={samples}, reversed={reversed}, step={step}: {diffs:?}"
                        );
                    }
                }
            }
        }
    }

    fn budget() -> ChunkUploadBudget {
        ChunkUploadBudget {
            max_columns: 8,
            max_input_bytes: 1024,
            max_work_buffer_bytes: 2048,
            max_upload_bytes: 4096,
            renderer_budget_bytes: 8192,
            pool_bytes: 0,
        }
    }

    #[test]
    fn gpu_disjoint_same_column_ranges_resolve_containing_span_and_reject_gaps() {
        let (device, queue) = crate::data_render::shared_device()
            .expect("disjoint chunk binding requires a GPU adapter");
        let ledger = Arc::new(GpuLedger::new());
        let first = ColumnRange {
            column: 7,
            revision: 3,
            source_len: 20,
            offset: 2,
            len: 2,
            encoding: SourceEncoding::ScalarF32,
        };
        let second = ColumnRange {
            offset: 10,
            ..first
        };
        let bytes = [0u8; 8];
        let mut encoder = device.create_command_encoder(&Default::default());
        let chunk = record_chunk(
            &device,
            &mut encoder,
            &ledger,
            budget(),
            &[
                ColumnInput {
                    range: first,
                    bytes: &bytes,
                },
                ColumnInput {
                    range: second,
                    bytes: &bytes,
                },
            ],
        )
        .unwrap();
        assert_eq!(chunk.column_handle(first).unwrap().byte_range(), 0..16);
        assert_eq!(chunk.column_handle(second).unwrap().byte_range(), 16..32);
        assert_eq!(
            chunk
                .column_handle(ColumnRange {
                    offset: 11,
                    len: 1,
                    ..second
                })
                .unwrap()
                .byte_range(),
            24..32
        );
        for (offset, len) in [(4, 1), (8, 2), (3, 8), (12, 1)] {
            assert!(matches!(
                chunk.column_handle(ColumnRange {
                    offset,
                    len,
                    ..first
                }),
                Err(StreamError::InvalidRange)
            ));
        }
        assert!(matches!(
            chunk.column_handle(ColumnRange {
                revision: 4,
                ..second
            }),
            Err(StreamError::Stale)
        ));
        let submitted = queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
    }

    fn range(column: u64, len: u64, encoding: SourceEncoding) -> ColumnRange {
        ColumnRange {
            column,
            revision: 7,
            source_len: len + 100,
            offset: 100,
            len,
            encoding,
        }
    }

    #[test]
    fn staging_copy_collects_encoded_extrema_only_for_requested_columns() {
        let (device, queue) = crate::data_render::shared_device()
            .expect("stream statistics test requires a GPU adapter");
        let ledger = Arc::new(GpuLedger::new());
        let scalars = [-0.0f32, 3.5, f32::NAN, f32::INFINITY, -2.0];
        let scalar_bytes = bytemuck::cast_slice(&scalars);
        let pairs = [
            (1_000_000_000.0f32, 0.25f32),
            (-1.0, 0.5),
            (f32::NAN, 0.0),
            (0.0, -0.0),
            (f32::MAX, f32::MAX),
        ];
        let pair_words: Vec<f32> = pairs.into_iter().flat_map(|(hi, lo)| [hi, lo]).collect();
        let pair_bytes = bytemuck::cast_slice(&pair_words);
        let ignored = [7.0f32];
        let invalid = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        let inputs = [
            ColumnInput {
                range: range(0, scalars.len() as u64, SourceEncoding::ScalarF32),
                bytes: scalar_bytes,
            },
            ColumnInput {
                range: range(1, pairs.len() as u64, SourceEncoding::HiLoF32),
                bytes: pair_bytes,
            },
            ColumnInput {
                range: range(2, 1, SourceEncoding::ScalarF32),
                bytes: bytemuck::cast_slice(&ignored),
            },
            ColumnInput {
                range: range(3, invalid.len() as u64, SourceEncoding::ScalarF32),
                bytes: bytemuck::cast_slice(&invalid),
            },
        ];
        let mut encoder = device.create_command_encoder(&Default::default());
        let chunk = record_chunk_collecting_statistics(
            &device,
            &mut encoder,
            &ledger,
            budget(),
            &inputs,
            &[
                ChunkStatisticsPlan {
                    ranges: vec![100..102, 104..105],
                },
                ChunkStatisticsPlan {
                    ranges: vec![100..105],
                },
                ChunkStatisticsPlan::default(),
                ChunkStatisticsPlan {
                    ranges: vec![100..103],
                },
            ],
        )
        .unwrap();
        assert_eq!(
            chunk.columns[0].statistics,
            Some(Some(StreamBounds {
                min: -2.0,
                max: 3.5,
                min_positive: Some(3.5),
            }))
        );
        assert_eq!(
            chunk.columns[1].statistics,
            Some(Some(StreamBounds {
                min: -0.5,
                max: 1_000_000_000.25,
                min_positive: Some(1_000_000_000.25),
            }))
        );
        assert_eq!(chunk.columns[2].statistics, None);
        assert_eq!(chunk.columns[3].statistics, Some(None));
        let submitted = queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
    }

    #[test]
    fn layout_validates_empty_short_overflow_and_device_cap_before_allocating() {
        assert_eq!(
            validate_layout(&[], budget(), 2048, 0, 0),
            Err(StreamError::InvalidRange.into())
        );
        let bytes = [0u8; 8];
        let mut input = ColumnInput {
            range: range(0, 2, SourceEncoding::ScalarF32),
            bytes: &bytes,
        };
        assert_eq!(
            validate_layout(std::slice::from_ref(&input), budget(), 8, 0, 0),
            Err(StreamError::TooLarge.into())
        );
        input.range.len = 3;
        input.range.source_len = 200;
        assert_eq!(
            validate_layout(std::slice::from_ref(&input), budget(), 2048, 0, 0),
            Err(StreamError::InvalidPayload.into())
        );
        input.range.offset = u64::MAX;
        assert_eq!(
            validate_layout(std::slice::from_ref(&input), budget(), 2048, 0, 0),
            Err(StreamError::Overflow.into())
        );
        assert_eq!(checked_pair_bytes(u64::MAX), Err(StreamError::Overflow));
    }

    #[test]
    fn exact_budget_counts_both_buffers_pool_and_retired_ledger_bytes() {
        let bytes = [0u8; 4];
        let input = ColumnInput {
            range: range(0, 1, SourceEncoding::ScalarF32),
            bytes: &bytes,
        };
        let mut caps = budget();
        caps.pool_bytes = 64 + 36; // Live pool plus uncompleted retired slabs.
        caps.renderer_budget_bytes = 216;
        assert!(validate_layout(std::slice::from_ref(&input), caps, 2048, 100, 0).is_ok());
        caps.renderer_budget_bytes -= 1;
        assert_eq!(
            validate_layout(std::slice::from_ref(&input), caps, 2048, 100, 0),
            Err(StreamError::TooLarge.into())
        );
        caps.renderer_budget_bytes = u64::MAX;
        assert_eq!(
            validate_layout(std::slice::from_ref(&input), caps, 2048, u64::MAX, 0),
            Err(StreamError::Overflow.into())
        );
        caps.max_upload_bytes = 15;
        assert_eq!(
            validate_layout(std::slice::from_ref(&input), caps, 2048, 0, 0),
            Err(StreamError::TooLarge.into())
        );
    }

    #[test]
    fn reused_work_charges_only_new_staging_against_renderer_budget() {
        let bytes = 1f32.to_le_bytes();
        let input = ColumnInput {
            range: range(0, 1, SourceEncoding::ScalarF32),
            bytes: &bytes,
        };
        let mut caps = budget();
        caps.renderer_budget_bytes = 16;
        assert_eq!(work_allocation_bytes(8, 32, caps, 2048, 0), 8);
        assert!(validate_layout(std::slice::from_ref(&input), caps, 2048, 8, 8).is_ok());
        assert_eq!(
            validate_layout(std::slice::from_ref(&input), caps, 2048, 8, 0),
            Err(StreamError::TooLarge.into())
        );
    }

    #[test]
    fn ordered_chunks_overwrite_one_work_buffer_without_changing_prior_draw_bytes() {
        let (device, queue) = crate::data_render::shared_device()
            .expect("stream work reuse requires a GPU adapter");
        let ledger = Arc::new(GpuLedger::new());
        let first_bytes = 1f32.to_le_bytes();
        let second_bytes = 2f32.to_le_bytes();
        let make_input = |bytes| ColumnInput {
            range: range(0, 1, SourceEncoding::ScalarF32),
            bytes,
        };
        let mut first_encoder = device.create_command_encoder(&Default::default());
        let first = record_chunk_maybe_collecting(
            &device, &mut first_encoder, &ledger, budget(), &[make_input(&first_bytes)], None, None, 32,
        ).unwrap();
        assert_eq!(first.work.size(), 32, "reserve the admitted chunk cap once");
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reused work ordered readback"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        first_encoder.copy_buffer_to_buffer(&first.work, 0, &readback, 0, 8);
        queue.submit([first_encoder.finish()]);
        // Do not wait for the first submission. The next submission must copy
        // new bytes only after the previous draw/readback consumed the buffer.
        let mut second_encoder = device.create_command_encoder(&Default::default());
        let second = record_chunk_maybe_collecting(
            &device, &mut second_encoder, &ledger, budget(), &[make_input(&second_bytes)],
            None, Some(&first.work), 0,
        ).unwrap();
        second_encoder.copy_buffer_to_buffer(&second.work, 0, &readback, 8, 8);
        assert_eq!(
            ledger.snapshot().creations_of(GpuResourceKind::StreamingUpload),
            3,
            "two chunks need two staging allocations but only one work allocation",
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        second_encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, 0..16, move |result| {
            let _ = sender.send(result);
        });
        drop((first, second));
        let submitted = queue.submit([second_encoder.finish()]);
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submitted),
            timeout: Some(std::time::Duration::from_secs(30)),
        }).unwrap();
        receiver.recv_timeout(std::time::Duration::from_secs(30)).unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let words: Vec<u32> = mapped.chunks_exact(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap())).collect();
        assert_eq!(words, [1f32.to_bits(), 0, 2f32.to_bits(), 0]);
    }

    #[test]
    fn range_source_chunks_reuse_the_same_work_allocation() {
        let (device, _) = crate::data_render::shared_device()
            .expect("range-source work reuse requires a GPU adapter");
        let ledger = Arc::new(GpuLedger::new());
        let source = crate::Column {
            data: vec![1f32, 2f32],
            min: 1.0,
            max: 2.0,
        };
        let supply = |_| StreamSourceSlice {
            source: crate::StreamColumnSource::Scalar(&source),
            source_len: 2,
            source_offset: None,
        };
        let plans = [ChunkStatisticsPlan::default()];
        let first_range = ColumnRange {
            source_len: 2,
            offset: 0,
            ..range(0, 1, SourceEncoding::ScalarF32)
        };
        let second_range = ColumnRange { offset: 1, ..first_range };
        let mut first_encoder = device.create_command_encoder(&Default::default());
        let first = record_source_chunk_collecting_statistics_reusing(
            &device, &mut first_encoder, &ledger, budget(),
            &[first_range], supply, &plans, None, 16,
        ).unwrap();
        assert_eq!(first.work.size(), 16);
        let mut second_encoder = device.create_command_encoder(&Default::default());
        let second = record_source_chunk_collecting_statistics_reusing(
            &device, &mut second_encoder, &ledger, budget(),
            &[second_range], supply, &plans, Some(&first.work), 16,
        ).unwrap();
        assert_eq!(second.work.size(), 16);
        assert_eq!(
            ledger.snapshot().creations_of(GpuResourceKind::StreamingUpload),
            3,
            "two mapped staging buffers and one reusable work buffer",
        );
    }

    #[test]
    fn gpu_borrowed_multi_column_readback_preserves_lane_bits_and_lifetimes() {
        let (device, queue) = crate::data_render::shared_device()
            .expect("bounded upload readback requires a GPU adapter");
        let ledger = Arc::new(GpuLedger::new());
        let scalar_words = [0x8000_0000u32, 0x7fc1_2345, 0x3f80_0001];
        let pair_words = [0x53c5_e7f2u32, 0xc700_0000, 0x7f80_0000, 0x8000_0000];
        let scalar: Vec<u8> = scalar_words.iter().flat_map(|v| v.to_le_bytes()).collect();
        let pairs: Vec<u8> = pair_words.iter().flat_map(|v| v.to_le_bytes()).collect();
        let third = 42f32.to_le_bytes();
        let inputs = [
            ColumnInput {
                range: range(5, 3, SourceEncoding::ScalarF32),
                bytes: &scalar,
            },
            ColumnInput {
                range: range(8, 2, SourceEncoding::HiLoF32),
                bytes: &pairs,
            },
            ColumnInput {
                range: range(11, 1, SourceEncoding::ScalarF32),
                bytes: &third,
            },
        ];
        let mut encoder = device.create_command_encoder(&Default::default());
        let chunk = record_chunk(&device, &mut encoder, &ledger, budget(), &inputs).unwrap();
        let mut requested = inputs[0].range;
        requested.offset += 1;
        requested.len = 1;
        let span = chunk.column_handle(requested).unwrap();
        assert_eq!(span.byte_range(), 8..16);
        assert_eq!(span.len_values, 1);
        requested.len = 3;
        assert!(matches!(
            chunk.column_handle(requested),
            Err(StreamError::InvalidRange)
        ));
        requested.len = 1;
        requested.offset = 99;
        assert!(matches!(
            chunk.column_handle(requested),
            Err(StreamError::InvalidRange)
        ));
        requested.offset = 101;
        requested.revision += 1;
        assert!(matches!(
            chunk.column_handle(requested),
            Err(StreamError::Stale)
        ));
        assert_eq!(
            chunk
                .columns
                .iter()
                .map(|v| v.offset_bytes)
                .collect::<Vec<_>>(),
            [0, 24, 40]
        );
        assert_eq!(
            chunk
                .columns
                .iter()
                .map(|v| v.range.column)
                .collect::<Vec<_>>(),
            [5, 8, 11]
        );
        assert_eq!(chunk.charged_bytes(), 96);
        assert_eq!(ledger.total_bytes(), 96);
        assert_eq!(
            ledger
                .snapshot()
                .creations_of(GpuResourceKind::StreamingUpload),
            2
        );
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk readback test"),
            size: 48,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&chunk.work, 0, &readback, 0, 48);
        let (sender, receiver) = std::sync::mpsc::channel();
        encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, 0..48, move |result| {
            let _ = sender.send(result);
        });
        drop(chunk); // Recorded commands keep GPU handles, ledger keeps the charge.
        assert_eq!(ledger.total_bytes(), 96);
        assert_eq!(ledger.snapshot().retired_bytes(), 96);
        let retirement = ledger.take_retirement();
        let submitted = queue.submit([encoder.finish()]);
        assert_eq!(ledger.total_bytes(), 96, "submission is not completion");
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap()
            .unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let words: Vec<u32> = mapped
            .chunks_exact(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        assert_eq!(
            words,
            [
                scalar_words[0],
                0,
                scalar_words[1],
                0,
                scalar_words[2],
                0,
                pair_words[0],
                pair_words[1],
                pair_words[2],
                pair_words[3],
                42f32.to_bits(),
                0
            ]
        );
        drop(mapped);
        readback.unmap();
        ledger.complete_retirement(retirement);
        assert_eq!(ledger.total_bytes(), 0);
    }

    #[test]
    fn gpu_rejection_and_writer_failure_never_publish_or_record_a_copy() {
        let (device, queue) = crate::data_render::shared_device()
            .expect("bounded upload failure test requires a GPU adapter");
        let ledger = Arc::new(GpuLedger::new());
        let bytes = [0u8; 4];
        let inputs = [ColumnInput {
            range: range(0, 1, SourceEncoding::ScalarF32),
            bytes: &bytes,
        }];
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut caps = budget();
        caps.max_work_buffer_bytes = 7;
        assert!(matches!(
            record_chunk(&device, &mut encoder, &ledger, caps, &inputs),
            Err(ChunkUploadError::Input(StreamError::TooLarge))
        ));
        assert_eq!(ledger.snapshot().total_creations(), 0);
        for panic in [false, true] {
            let result =
                record_chunk_with(&device, &mut encoder, &ledger, budget(), &inputs, |_, _| {
                    if panic {
                        panic!("injected writer panic");
                    }
                    Err(ChunkUploadError::WriterFailed)
                }, None, 0);
            assert!(matches!(result, Err(ChunkUploadError::WriterFailed)));
        }
        assert_eq!(ledger.snapshot().live_bytes(), 0);
        assert_eq!(ledger.snapshot().retired_bytes(), 16);
        assert_eq!(
            ledger.snapshot().total_creations(),
            2,
            "only staging, never work or copies"
        );
        let submitted = queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
    }

    #[test]
    fn gpu_allocation_rejection_is_atomic_at_staging_and_work_creation() {
        let (device, queue) = crate::data_render::shared_device()
            .expect("bounded upload allocation-failure test requires a GPU adapter");
        let bytes = 23f32.to_le_bytes();
        let inputs = [ColumnInput {
            range: range(0, 1, SourceEncoding::ScalarF32),
            bytes: &bytes,
        }];
        // Reject deterministically BEFORE the selected create_buffer call.
        // This tests failure handling, not driver/device OOM behavior.
        for rejected_call in [1, 2] {
            let ledger = Arc::new(GpuLedger::new());
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut allocation_calls = 0;
            let mut writer_calls = 0;
            let result = record_chunk_with_factory(
                &device,
                &mut encoder,
                &ledger,
                budget(),
                &inputs,
                |_, mut dst| {
                    writer_calls += 1;
                    dst.write_pair(0, 23.0, 0.0);
                    Ok(())
                },
                None,
                0,
                |desc| {
                    allocation_calls += 1;
                    if allocation_calls == rejected_call {
                        return Err(ChunkUploadError::AllocationFailed);
                    }
                    assert!(desc.mapped_at_creation, "only staging may be created");
                    create_buffer_checked(&device, desc)
                },
            );
            assert!(matches!(result, Err(ChunkUploadError::AllocationFailed)));
            assert_eq!(allocation_calls, rejected_call);
            assert_eq!(writer_calls, rejected_call - 1);
            let actual_staging_bytes = (rejected_call - 1) as u64 * 8;
            let usage = ledger.snapshot();
            assert_eq!(usage.total_creations(), (rejected_call - 1) as u64);
            assert_eq!(usage.live_bytes(), 0);
            assert_eq!(usage.retired_bytes(), actual_staging_bytes);
            // No destination was created, so no chunk copy can be recorded.
            // Finishing/submitting the same encoder also proves the rejected
            // upload did not poison the caller's command recording state.
            let retirement = ledger.take_retirement();
            let submitted = queue.submit([encoder.finish()]);
            assert_eq!(ledger.total_bytes(), actual_staging_bytes);
            let (sender, receiver) = std::sync::mpsc::channel();
            queue.on_submitted_work_done(move || {
                let _ = sender.send(());
            });
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submitted),
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .unwrap();
            receiver
                .recv_timeout(std::time::Duration::from_secs(30))
                .unwrap();
            ledger.complete_retirement(retirement);
            assert_eq!(ledger.total_bytes(), 0);
        }
    }

    #[test]
    fn gpu_scheduler_late_cancelled_submission_releases_only_its_own_receipt() {
        use crate::streaming::{
            RequestStatus, SourceStamp, StreamLimits, StreamScheduler, ViewEpoch,
        };
        let (device, queue) = crate::data_render::shared_device()
            .expect("bounded upload scheduler test requires a GPU adapter");
        let ledger = Arc::new(GpuLedger::new());
        let mut scheduler = StreamScheduler::new(
            17,
            StreamLimits {
                max_jobs: 2,
                max_slots: 2,
                max_columns_per_request: 2,
                max_chunk_bytes: 32,
                max_in_flight_bytes: 32,
            },
        )
        .unwrap();
        let col = range(1, 1, SourceEncoding::ScalarF32);
        let bytes = 17f32.to_le_bytes();
        let inputs = [ColumnInput {
            range: col,
            bytes: &bytes,
        }];
        let a = scheduler
            .start_job(10, SourceStamp(1), ViewEpoch(1))
            .unwrap();
        let RequestStatus::Ready(ta) = scheduler.request(a, &[col]).unwrap() else {
            panic!()
        };
        let mut encoder_a = device.create_command_encoder(&Default::default());
        let chunk_a = scheduler
            .accept(ta, &inputs, |input| {
                record_chunk(&device, &mut encoder_a, &ledger, budget(), input)
                    .map_err(|_| StreamError::WriterFailed)
            })
            .unwrap();
        scheduler.cancel(a);
        assert_eq!(scheduler.charged_bytes(), 16);
        let b = scheduler
            .start_job(10, SourceStamp(2), ViewEpoch(2))
            .unwrap();
        let RequestStatus::Ready(tb) = scheduler.request(b, &[col]).unwrap() else {
            panic!()
        };
        let mut encoder_b = device.create_command_encoder(&Default::default());
        let chunk_b = scheduler
            .accept(tb, &inputs, |input| {
                record_chunk(&device, &mut encoder_b, &ledger, budget(), input)
                    .map_err(|_| StreamError::WriterFailed)
            })
            .unwrap();
        assert_eq!(ledger.total_bytes(), 32);
        assert_eq!(scheduler.charged_bytes(), 32);
        assert_eq!(
            scheduler.request(b, &[col]).unwrap(),
            RequestStatus::Backpressure
        );
        // The old recorded copy submits only after the replacement was built.
        drop(chunk_a);
        let retired_a = ledger.take_retirement();
        let receipt_a = scheduler.submit(ta).unwrap();
        let submission = queue.submit([encoder_a.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        queue.on_submitted_work_done(move || {
            let _ = sender.send(receipt_a);
        });
        assert_eq!(ledger.total_bytes(), 32);
        assert_eq!(scheduler.charged_bytes(), 32);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        let completed = receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
        scheduler.complete(completed).unwrap();
        ledger.complete_retirement(retired_a);
        assert_eq!(scheduler.charged_bytes(), 16);
        assert_eq!(ledger.total_bytes(), 16);
        assert_eq!(scheduler.active_jobs(), 1);
        assert_eq!(scheduler.complete(completed), Err(StreamError::Stale));
        // B is still recorded and owned: A's callback cannot release it.
        assert_eq!(chunk_b.charged_bytes(), 16);
        drop(encoder_b);
        drop(chunk_b);
        scheduler.discard_recorded(tb).unwrap();
        assert_eq!(scheduler.charged_bytes(), 0);
        assert_eq!(
            ledger.total_bytes(),
            16,
            "discard alone is not a reported boundary"
        );
        let retired_b = ledger.take_retirement();
        let (sender, receiver) = std::sync::mpsc::channel();
        queue.on_submitted_work_done(move || {
            let _ = sender.send(());
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
        ledger.complete_retirement(retired_b);
        assert_eq!(ledger.total_bytes(), 0);
    }
}
