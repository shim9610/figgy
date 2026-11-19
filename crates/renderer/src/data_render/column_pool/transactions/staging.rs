//! Staging writes, upload encoding and the ranged column staging (#311).

use super::*;

/// Invoke the fused source capability once while the write-only view is
/// live, measuring the values' uniform step (`UniformStep`, #134) as they
/// are written. `lanes` decides the staging layout (#317): `raw_bytes` is
/// the source's bytes in that layout.
pub(in crate::data_render::column_pool) fn write_staging_pairs(
    staging: &Buffer,
    raw_bytes: u64,
    lanes: StagedLanes,
    write_pairs: impl FnOnce(&mut ColumnPairWriter<'_>),
) -> Result<(Option<crate::StreamBounds>, Option<f64>), AllocError> {
    write_staging_pairs_observed(
        staging,
        raw_bytes,
        lanes,
        false,
        &mut |_, _, _| {},
        write_pairs,
    )
}

/// [`write_staging_pairs`] with an observer of every written pair,
/// `observe(row, hi, lo)`, and `index_lanes` for a column stored as `u32`
/// index lanes ([`ColumnPairWriter::with_index_lanes`]): the batch
/// transaction measures a record's or a ragged column's statistics through
/// it while the staging write happens (#311).
pub(in crate::data_render::column_pool) fn write_staging_pairs_observed(
    staging: &Buffer,
    raw_bytes: u64,
    lanes: StagedLanes,
    index_lanes: bool,
    observe: &mut dyn FnMut(usize, f32, f32),
    write_pairs: impl FnOnce(&mut ColumnPairWriter<'_>),
) -> Result<(Option<crate::StreamBounds>, Option<f64>), AllocError> {
    let mut view = staging
        .slice(0..raw_bytes)
        .get_mapped_range_mut()
        .map_err(|error| AllocError::Gpu {
            resource: "column staging buffer",
            reason: error.to_string(),
        })?;
    let mut step = crate::data::UniformStep::new();
    let out_of_range = std::cell::Cell::new(false);
    let second_lane = std::cell::Cell::new(false);
    let bounds = {
        let mut both = |index: usize, hi: f32, lo: f32| {
            step.record(index, hi, lo);
            observe(index, hi, lo);
        };
        let mut writer = ColumnPairWriter::new_observed(view.slice(..), &out_of_range, &mut both)
            .with_index_lanes(index_lanes)
            .with_single_lane(lanes.compact, lanes.single.then_some(&second_lane));
        write_pairs(&mut writer);
        writer.bounds()
    };
    drop(view);
    staging.unmap();
    if out_of_range.get() {
        return Err(source_wrote_out_of_range());
    }
    if second_lane.get() {
        return Err(source_wrote_second_lane());
    }
    Ok((bounds, step.finish()))
}

/// A column's statistics as its slot stores them, from the bounds its writer
/// recorded: `+inf`, `-inf` and no positive value when no written value was
/// finite.
pub(in crate::data_render::column_pool) fn slot_statistics(
    bounds: Option<crate::StreamBounds>,
) -> (f64, f64, Option<f64>) {
    bounds.map_or((f64::INFINITY, f64::NEG_INFINITY, None), |bounds| {
        (bounds.min, bounds.max, bounds.min_positive)
    })
}

/// The error for a source that wrote a pair index outside its own length.
pub(in crate::data_render::column_pool) fn source_wrote_out_of_range() -> AllocError {
    AllocError::AllocationFailed {
        resource: "column source",
        reason: "the source wrote a value index outside its own length".into(),
    }
}

/// The error for a single-lane source ([`ColumnLanes::Single`](crate::ColumnLanes))
/// that wrote a second lane whose bits are not zero (#317).
pub(in crate::data_render::column_pool) fn source_wrote_second_lane() -> AllocError {
    AllocError::AllocationFailed {
        resource: "column source",
        reason: "a single-lane source (ColumnSource::lanes returns Single) wrote a non-zero \
                 second lane; return ColumnLanes::Pair to upload (hi, lo) pairs"
            .into(),
    }
}

/// A compact column's expansion (#317), prepared while the upload can still
/// fail: the cached pipeline and the upload's transient storage buffer.
pub(in crate::data_render::column_pool) struct PreparedExpansion {
    pub(in crate::data_render::column_pool) expand: ColumnExpand,
    pub(in crate::data_render::column_pool) limits: ExpandLimits,
    pub(in crate::data_render::column_pool) transient: TrackedBuffer,
}

/// Encode one column's upload into `target[offset..offset + region_bytes]`:
/// the staging copy for a pair column, or for a compact one the copy of its
/// half-region staging into the transient buffer and the expansion.
pub(in crate::data_render::column_pool) fn encode_column_upload(
    device: &Device,
    encoder: &mut wgpu::CommandEncoder,
    staging: &Buffer,
    expansion: Option<&PreparedExpansion>,
    target: &Buffer,
    offset: u64,
    region_bytes: u64,
) {
    let Some(prepared) = expansion else {
        encoder.copy_buffer_to_buffer(staging, 0, target, offset, region_bytes);
        return;
    };
    encoder.copy_buffer_to_buffer(staging, 0, &prepared.transient, 0, region_bytes / 2);
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("figgy column expand"),
        timestamp_writes: None,
    });
    // One column is a run of one.
    prepared.expand.dispatch_run(
        device,
        &mut pass,
        prepared.limits,
        &prepared.transient,
        target,
        offset,
        region_bytes,
    );
}

/// Source metadata captured before an upload is prepared.
///
/// This deliberately excludes allocator state and the statistics, which the
/// writer records while the values are written and [`ColumnSlot`] keeps.
pub(in crate::data_render::column_pool) struct ColumnInputMeta {
    pub(in crate::data_render::column_pool) id: ColumnId,
    pub(in crate::data_render::column_pool) len_values: usize,
    /// The source's declared lanes, read once (#317). A `HiLoColumnSource`
    /// is always [`ColumnLanes::Pair`](crate::ColumnLanes).
    pub(in crate::data_render::column_pool) lanes: crate::ColumnLanes,
}

/// A resident column written in ranges across several calls (#311): one
/// mapped staging buffer the size of the column, filled range after range in
/// index order through [`ColumnPairWriter`]s, and installed by
/// [`ColumnPool::begin_upsert_staged_column`] like any other upsert. The
/// values reach the GPU through the staging buffer only; the CPU keeps the
/// statistics the writers recorded (rule 2).
///
/// Ranges arrive in order: each starts at the first value not yet written,
/// so a range that overlaps a written one or leaves a gap is refused, and a
/// refused range changes nothing. The staging buffer is charged to the
/// device ledger while the upload is open; dropping the staging releases it.
pub struct ColumnStaging {
    pub(super) staging: TrackedBuffer,
    pub(super) len_values: usize,
    pub(super) byte_size: u64,
    written: usize,
    pub(super) bounds: Option<crate::StreamBounds>,
    pub(super) step: crate::data::UniformStep,
    encoding: crate::StreamEncoding,
    /// Whether the buffer is still mapped: an install unmaps it before it
    /// copies the values, and a rolled-back batch install leaves it so
    /// (#334). An atomic only for `&self` access; nothing is shared.
    mapped: std::sync::atomic::AtomicBool,
}

/// Why a range write to a [`ColumnStaging`] was refused (#311).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnStagingError {
    /// The range starts before the first value not yet written.
    Overlap,
    /// The range starts after the first value not yet written.
    Gap,
    /// The range runs past the column's length.
    PastEnd,
    /// The range holds no value.
    Empty,
    /// The range's encoding is not the upload's.
    EncodingMismatch,
    /// The source wrote a value outside its own range.
    SourceOutOfRange,
    /// The staging buffer is not mapped for writing.
    Unmapped,
    /// Not every value was written.
    Incomplete,
}

impl ColumnStagingError {
    /// The refusal as a sentence fragment.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Overlap => "the chunk overlaps values already written",
            Self::Gap => "the chunk leaves a gap after the values already written",
            Self::PastEnd => "the chunk runs past the column's length",
            Self::Empty => "the chunk holds no value",
            Self::EncodingMismatch => "the chunk's encoding is not the upload's",
            Self::SourceOutOfRange => "the chunk's source wrote outside its own range",
            Self::Unmapped => "the staging buffer is not mapped for writing",
            Self::Incomplete => "not every value of the column was written",
        }
    }
}

impl ColumnStaging {
    /// A staging buffer for a column of `len_values` values of `encoding`.
    /// The buffer is sized, checked against the device's buffer limit and
    /// the ledger's memory budget, and created mapped; nothing else is
    /// reserved, so the pool region is found when the column is installed.
    pub fn new(
        len_values: usize,
        encoding: crate::StreamEncoding,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<Self, AllocError> {
        const RESOURCE: &str = "column upload staging buffer";
        if len_values == 0 {
            return Err(AllocError::EmptySource);
        }
        let ceiling = buffer_ceiling(ctx.device);
        let raw_bytes = (len_values as u64)
            .checked_mul(COLUMN_VALUE_BYTES as u64)
            .ok_or(AllocError::ResourceLimit {
                resource: RESOURCE,
                requested: u64::MAX,
                limit: ceiling,
            })?;
        let byte_size = try_align_up(raw_bytes, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: RESOURCE,
            requested: raw_bytes,
            limit: ceiling,
        })?;
        if byte_size > ceiling {
            return Err(AllocError::ResourceLimit {
                resource: RESOURCE,
                requested: byte_size,
                limit: ceiling,
            });
        }
        // The staging buffer lives as long as the upload stays open, so it
        // is held to the budget the pool's growth is held to.
        if let Some(limit) = ctx.ledger.budget() {
            let peak = ctx.ledger.total_bytes().checked_add(byte_size).ok_or(
                AllocError::BudgetExceeded {
                    resource: RESOURCE,
                    requested: u64::MAX,
                    limit,
                },
            )?;
            if peak > limit {
                return Err(AllocError::BudgetExceeded {
                    resource: RESOURCE,
                    requested: peak,
                    limit,
                });
            }
        }
        let staging = create_buffer_checked(
            ctx.device,
            ctx.ledger,
            &BufferDescriptor {
                label: Some("figgy column upload staging"),
                size: byte_size,
                usage: BufferUsages::COPY_SRC,
                mapped_at_creation: true,
            },
            RESOURCE,
        )?;
        Ok(Self {
            staging,
            len_values,
            byte_size,
            written: 0,
            bounds: None,
            step: crate::data::UniformStep::new(),
            encoding,
            mapped: std::sync::atomic::AtomicBool::new(true),
        })
    }

    /// The column's length in values.
    pub fn len(&self) -> usize {
        self.len_values
    }

    /// Whether the column has no value (never: an empty upload is refused).
    pub fn is_empty(&self) -> bool {
        self.len_values == 0
    }

    /// How many values were written: the start of the next range.
    pub fn written(&self) -> usize {
        self.written
    }

    /// The encoding every range must have.
    pub fn encoding(&self) -> crate::StreamEncoding {
        self.encoding
    }

    /// Whether every value was written.
    pub fn is_complete(&self) -> bool {
        self.written == self.len_values
    }

    /// The staging bytes this upload holds on the device.
    pub fn staging_bytes(&self) -> u64 {
        self.byte_size
    }

    /// Whether the staging buffer is still mapped for writing: an install
    /// unmaps it before it copies the values (#334).
    fn is_mapped(&self) -> bool {
        self.mapped.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Unmap the staging buffer for the copy that installs it. A buffer an
    /// earlier install that was rolled back unmapped stays as it is: its
    /// values are written, and unmapping it again would be a device error.
    pub(super) fn unmap_for_install(&self) {
        if self
            .mapped
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            self.staging.unmap();
        }
    }

    /// Write `source.len()` values from value `start`: the source writes
    /// its whole length (`write_pairs`) straight into the mapped staging
    /// range. `start` must be [`Self::written`]. A refused range writes
    /// nothing the upload keeps: the next range may start at the same
    /// place again.
    pub fn write_range(
        &mut self,
        start: u64,
        source: StreamColumnSource<'_>,
    ) -> Result<(), ColumnStagingError> {
        if source.encoding() != self.encoding {
            return Err(ColumnStagingError::EncodingMismatch);
        }
        let len = source.len();
        if len == 0 {
            return Err(ColumnStagingError::Empty);
        }
        if !self.is_mapped() {
            return Err(ColumnStagingError::Unmapped);
        }
        let start = usize::try_from(start).map_err(|_| ColumnStagingError::Gap)?;
        if start < self.written {
            return Err(ColumnStagingError::Overlap);
        }
        if start > self.written {
            return Err(ColumnStagingError::Gap);
        }
        let end = start
            .checked_add(len)
            .filter(|end| *end <= self.len_values)
            .ok_or(ColumnStagingError::PastEnd)?;
        let from = start as u64 * COLUMN_VALUE_BYTES as u64;
        let to = end as u64 * COLUMN_VALUE_BYTES as u64;
        let mut view = self
            .staging
            .slice(from..to)
            .get_mapped_range_mut()
            .map_err(|_| ColumnStagingError::Unmapped)?;
        // The step is measured on a copy, kept only when the range is.
        let mut step = self.step;
        let out_of_range = std::cell::Cell::new(false);
        let bounds = {
            let mut observe = |index: usize, hi: f32, lo: f32| step.record(start + index, hi, lo);
            let mut writer =
                ColumnPairWriter::new_observed(view.slice(..), &out_of_range, &mut observe);
            match source {
                StreamColumnSource::Scalar(source) => source.write_pairs(&mut writer),
                StreamColumnSource::HiLo(source) => source.write_pairs(&mut writer),
            }
            writer.bounds()
        };
        drop(view);
        if out_of_range.get() {
            return Err(ColumnStagingError::SourceOutOfRange);
        }
        self.step = step;
        self.bounds = merge_bounds(self.bounds, bounds);
        self.written = end;
        Ok(())
    }
}

/// The statistics of two written ranges together.
fn merge_bounds(
    a: Option<crate::StreamBounds>,
    b: Option<crate::StreamBounds>,
) -> Option<crate::StreamBounds> {
    match (a, b) {
        (Some(a), Some(b)) => Some(crate::StreamBounds {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
            min_positive: match (a.min_positive, b.min_positive) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (x, y) => x.or(y),
            },
        }),
        (a, b) => a.or(b),
    }
}

#[derive(Clone, Copy)]
pub(in crate::data_render::column_pool) struct StagedStreamedColumn {
    pub(in crate::data_render::column_pool) offset: u64,
    pub(in crate::data_render::column_pool) byte_size: u64,
    pub(in crate::data_render::column_pool) len_values: usize,
    pub(in crate::data_render::column_pool) min: f64,
    pub(in crate::data_render::column_pool) max: f64,
    pub(in crate::data_render::column_pool) min_positive: Option<f64>,
}

/// Streamed promotion stages `(hi, lo)` pairs whatever a source's
/// [`ColumnSource::lanes`] says: streaming is not extended (CLAUDE.md rule
/// 8), so the compact single-lane upload (#317) is resident only.
pub(in crate::data_render::column_pool) fn stage_streamed_columns(
    device: &Device,
    ledger: &Arc<GpuLedger>,
    columns: &[StreamedColumnUpload<'_>],
    total: u64,
) -> Result<(TrackedBuffer, Vec<StagedStreamedColumn>), AllocError> {
    let staging = create_buffer_checked(
        device,
        ledger,
        &BufferDescriptor {
            label: Some("figgy streamed promotion staging"),
            size: total,
            usage: BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        },
        "streamed promotion staging buffer",
    )?;
    let mut staged = Vec::new();
    staged
        .try_reserve_exact(columns.len())
        .map_err(|error| AllocError::AllocationFailed {
            resource: "streamed promotion staging plan",
            reason: error.to_string(),
        })?;

    let mut view = staging
        .slice(..)
        .get_mapped_range_mut()
        .map_err(|error| AllocError::Gpu {
            resource: "streamed promotion staging buffer",
            reason: error.to_string(),
        })?;
    let mut offset = 0u64;
    let written = (|| {
        for column in columns {
            let len_values = column.len_values;
            let raw_bytes = column_raw_bytes(len_values);
            let byte_size = column_region_bytes(len_values).ok_or(AllocError::Internal {
                invariant: "a staged promotion column has a region size",
            })?;
            let start = offset as usize;
            let end = start + raw_bytes as usize;
            let out_of_range = std::cell::Cell::new(false);
            // `write_range` is the host's code. A panic there is the host's,
            // not figgy's, and is returned as this promotion's failure, as
            // the streamed upload does (`streaming_upload.rs`).
            let bounds = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut writer = ColumnPairWriter::new(view.slice(start..end), &out_of_range);
                column
                    .source
                    .write_range(0, &mut writer)
                    .map(|()| writer.bounds())
            }))
            .map_err(|_| AllocError::AllocationFailed {
                resource: "streamed promotion source",
                reason: "source panicked while writing the requested range".into(),
            })?
            .map_err(|error| AllocError::AllocationFailed {
                resource: "streamed promotion source",
                reason: error.reason().to_owned(),
            })?;
            if out_of_range.get() {
                return Err(source_wrote_out_of_range());
            }
            let (min, max, min_positive) = slot_statistics(bounds);
            staged.push(StagedStreamedColumn {
                offset,
                byte_size,
                len_values,
                min,
                max,
                min_positive,
            });
            offset += byte_size;
        }
        Ok(())
    })();
    drop(view);
    staging.unmap();
    written?;
    Ok((staging, staged))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResidentBatchCapacityPlan {
    pub capacity_after: u64,
    /// Newly allocated pool bytes that coexist with the current pool while a
    /// relayout is prepared. Zero means the existing layout or backup serves
    /// the request without another pool buffer.
    pub transition_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResidentBatchCapacityError {
    Overflow,
    DeviceLimit,
    Policy,
}

pub(in crate::data_render::column_pool) enum UpsertBufferRollback {
    /// The upload used a region that was free before the transaction.  Bytes
    /// belonging to the old slot (when any) were never touched.
    InPlace,
    /// The provisional pool uses a candidate primary.  The old primary and
    /// backup are retained until commit so dropping the guard can restore the
    /// exact pre-transaction pool.
    Candidate {
        old_primary: Option<TrackedBuffer>,
        old_backup: Option<TrackedBuffer>,
        candidate_was_backup: bool,
    },
}

pub(in crate::data_render::column_pool) struct UpsertRollback {
    pub(in crate::data_render::column_pool) slots: HashMap<ColumnId, ColumnSlot>,
    pub(in crate::data_render::column_pool) free: Vec<FreeRegion>,
    pub(in crate::data_render::column_pool) generation: u32,
    pub(in crate::data_render::column_pool) allocation_epochs: HashMap<ColumnId, u64>,
    pub(in crate::data_render::column_pool) allocation_epoch_counter: u64,
    pub(in crate::data_render::column_pool) layout_generation: u64,
    pub(in crate::data_render::column_pool) buffer: UpsertBufferRollback,
}

pub(in crate::data_render::column_pool) enum PreparedUpsertTarget {
    InPlace,
    FreshCandidate(TrackedBuffer),
    BackupCandidate,
}
