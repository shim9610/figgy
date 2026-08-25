//! GPU columnar memory pool — column-sized slabs in one big buffer, with an
//! offset table and ping-pong defrag.
//!
//! - One `primary` GPU buffer holds every column, packed by offset.
//! - `slots: HashMap<ColumnId, ColumnSlot>` is the SSoT mapping id → byte
//!   range; `free: Vec<FreeRegion>` tracks holes (first-fit on add, coalesce
//!   with neighbors on remove).
//! - Column sources write pairs and collect `min_positive` in one pass over
//!   the mapped staging range. wgpu 29+ maps that range as write-only, so the
//!   renderer never reads the mapped bytes back.
//! - `defragment` packs survivors into a backup buffer with GPU-internal
//!   copies (no PCIe traffic) then swaps `primary <-> backup`.
//!
//! `ColumnHandle` carries a public `generation` value. Mutations that can
//! invalidate positional handles, including removal, replacement, relocation,
//! and clear, bump it so callers can detect staleness with `is_valid_handle`
//! and re-fetch with `handle_for`.
//!
//! Auto-defrag is opt-in via [`DefragPolicy`] (default `Manual`). With
//! `OnAllocFailure`, an `OutOfSpace` from `add_column` triggers one
//! `defragment()` and a single retry.
//!
//! All offsets and sizes are [`ALIGN`] = 256-byte aligned to satisfy wgpu's
//! storage-binding alignment; vertex slices reuse the same value to keep
//! mode switching free of caveats.

use std::{collections::HashMap, fmt, sync::Arc};

use wgpu::{Buffer, BufferDescriptor, BufferUsages, Device, Queue};

use crate::data::{
    COLUMN_VALUE_BYTES, ColumnPairWriter, ColumnSource, ColumnUploadStats, HiLoColumnSource,
};

// Defined in the model crate (`model::data`); re-exported here so
// `data_render::ColumnId` stays a valid path.
pub use crate::data::ColumnId;

/// Opaque identity of one [`ColumnPool`] instance.
///
/// Clones retain the same identity. Equality is allocation identity rather
/// than a numeric stamp, so independently-created pools cannot compare as the
/// same while either identity is live.
#[derive(Clone)]
pub(crate) struct PoolIdentity(Arc<PoolIdentityInner>);

struct PoolIdentityInner {
    _private: u8,
}

impl PoolIdentity {
    fn new() -> Self {
        Self(Arc::new(PoolIdentityInner { _private: 0 }))
    }

    pub(crate) fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for PoolIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.same_instance(other)
    }
}

impl Eq for PoolIdentity {}

impl fmt::Debug for PoolIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PoolIdentity")
            .finish_non_exhaustive()
    }
}

/// Alignment (in bytes) for every offset and size in the pool.
pub const ALIGN: u64 = 256;

#[inline]
fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) & !(a - 1)
}

#[inline]
fn try_align_up(x: u64, a: u64) -> Option<u64> {
    x.checked_add(a - 1).map(|v| v & !(a - 1))
}

fn out_of_space(requested: u64, free: &[FreeRegion]) -> AllocError {
    AllocError::OutOfSpace {
        requested,
        largest_free: free.iter().map(|region| region.size).max().unwrap_or(0),
        total_free: free.iter().map(|region| region.size).sum(),
    }
}

/// Invoke the fused source capability once while the write-only view is live.
fn write_staging_pairs(
    staging: &Buffer,
    raw_bytes: u64,
    write_pairs: impl FnOnce(ColumnPairWriter<'_>) -> ColumnUploadStats,
) -> ColumnUploadStats {
    let mut view = staging
        .slice(0..raw_bytes)
        .get_mapped_range_mut()
        .expect("column staging is mapped at creation");
    let stats = write_pairs(ColumnPairWriter::new(view.slice(..)));
    drop(view);
    staging.unmap();
    stats
}

fn alloc_region_from(free: &mut Vec<FreeRegion>, size: u64) -> Result<u64, AllocError> {
    let Some(index) = free.iter().position(|region| region.size >= size) else {
        return Err(out_of_space(size, free));
    };
    let region = free[index];
    let offset = region.offset;
    if region.size == size {
        free.remove(index);
    } else {
        free[index] = FreeRegion {
            offset: region.offset + size,
            size: region.size - size,
        };
    }
    Ok(offset)
}

fn coalesce_regions(free: &mut Vec<FreeRegion>) {
    if free.len() < 2 {
        return;
    }
    free.sort_by_key(|region| region.offset);
    let mut merged: Vec<FreeRegion> = Vec::with_capacity(free.len());
    for region in free.drain(..) {
        if let Some(last) = merged.last_mut()
            && last.offset + last.size == region.offset
        {
            last.size += region.size;
            continue;
        }
        merged.push(region);
    }
    *free = merged;
}

/// One column's occupied region inside the pool.
#[derive(Debug, Clone)]
pub struct ColumnSlot {
    pub id: ColumnId,
    pub offset: u64,
    pub byte_size: u64,
    pub len_values: usize,
    pub generation: u32,
    /// Captured from `ColumnSource::min/max` at `add_column` time so auto-fit
    /// can read the range without rescanning data that lives on the GPU.
    pub min: f64,
    pub max: f64,
    /// Smallest strictly-positive value, collected during upload — the
    /// log-axis auto-fit lower bound when the data contains zeros or
    /// negatives. `None` when no positive value exists.
    ///
    /// Scalar stats like this are the ONLY per-value information retained on
    /// the CPU. The pool deliberately keeps no copy of the data itself —
    /// per-point geometry (dashed-line arc length) is computed on the GPU
    /// (`line_arc.wgsl`). Do not reintroduce CPU shadows.
    pub min_positive: Option<f64>,
}

/// Source metadata captured before an upload is prepared.
///
/// This deliberately excludes allocator state and encoded-value statistics,
/// which belong to [`ColumnSlot`] and [`ColumnUploadStats`] respectively.
struct ColumnInputMeta {
    id: ColumnId,
    len_values: usize,
    min: f64,
    max: f64,
}

/// A free region. Adjacent regions are merged on coalesce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeRegion {
    pub offset: u64,
    pub size: u64,
}

enum RegionReservationRollback {
    ExactFit,
    Split,
}

/// A first-fit allocation that restores the free list unless committed.
///
/// This guard deliberately borrows only the free-list field. The upload path
/// can therefore prepare staging data and publish the slot while the
/// reservation remains armed, and any error or unwind before publication
/// restores the allocator to its exact prior ordering and contents.
#[must_use = "dropping an uncommitted reservation restores the free list"]
struct RegionReservation<'a> {
    free: &'a mut Vec<FreeRegion>,
    index: usize,
    original: FreeRegion,
    rollback: Option<RegionReservationRollback>,
}

/// The free-list edit `alloc_region` made, in undo form.
///
/// A transaction that outlives the reservation guard — the free list is one
/// field of the pool, and a guard holding the whole pool cannot also hold a
/// borrow of that field — takes this record instead and replays it if it has
/// to roll back.
struct RegionUndo {
    index: usize,
    original: FreeRegion,
    rollback: RegionReservationRollback,
}

impl RegionUndo {
    fn apply(self, free: &mut Vec<FreeRegion>) {
        match self.rollback {
            RegionReservationRollback::ExactFit => {
                // `remove` retained enough Vec capacity for this insertion,
                // so rollback itself does not allocate.
                debug_assert!(
                    self.index <= free.len(),
                    "free list shrank under a reservation"
                );
                free.insert(self.index, self.original);
            }
            RegionReservationRollback::Split => {
                debug_assert!(
                    self.index < free.len(),
                    "free list shrank under a reservation"
                );
                free[self.index] = self.original;
            }
        }
    }
}

impl RegionReservation<'_> {
    fn offset(&self) -> u64 {
        self.original.offset
    }

    fn commit(mut self) {
        self.rollback = None;
    }

    /// Defuse the guard and hand back its undo record.
    ///
    /// The free list stays edited; the caller becomes responsible for replaying
    /// the record if its own transaction fails.
    fn into_undo(mut self) -> RegionUndo {
        let rollback = self
            .rollback
            .take()
            .expect("a live reservation still carries its undo record");
        RegionUndo {
            index: self.index,
            original: self.original,
            rollback,
        }
    }
}

impl Drop for RegionReservation<'_> {
    fn drop(&mut self) {
        if let Some(rollback) = self.rollback.take() {
            RegionUndo {
                index: self.index,
                original: self.original,
                rollback,
            }
            .apply(&mut *self.free);
        }
    }
}

fn alloc_region(
    free: &mut Vec<FreeRegion>,
    size: u64,
) -> Result<RegionReservation<'_>, AllocError> {
    let Some(index) = free.iter().position(|region| region.size >= size) else {
        return Err(out_of_space(size, free));
    };
    let original = free[index];
    let rollback = if original.size == size {
        free.remove(index);
        RegionReservationRollback::ExactFit
    } else {
        free[index] = FreeRegion {
            offset: original.offset + size,
            size: original.size - size,
        };
        RegionReservationRollback::Split
    };
    Ok(RegionReservation {
        free,
        index,
        original,
        rollback: Some(rollback),
    })
}

/// Auto-defrag policy. No `Default` impl — figgy avoids the `Default` trait;
/// callers set this explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefragPolicy {
    /// No automatic defrag. Caller must invoke `defragment()` directly.
    Manual,
    /// On `add_column`'s `OutOfSpace`, attempt one defrag and retry. If that
    /// still fails, the original `OutOfSpace` is returned.
    OnAllocFailure,
}

/// Whether the pool may enlarge itself when an upload does not fit.
///
/// Default is [`GrowthPolicy::Fixed`]: `OutOfSpace` stays the final answer,
/// which is the meaning released hosts already depend on. Growth is opt-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrowthPolicy {
    /// Never enlarge. A full pool reports `OutOfSpace` as before.
    Fixed,
    /// After a defrag retry still does not fit, relayout into a larger buffer
    /// and retry once. Bounded by the device ceiling and the caller's budget;
    /// if growth cannot happen the original `OutOfSpace` is returned unchanged.
    OnAllocFailure,
}

/// One column's unpadded byte size.
fn column_raw_bytes(len_values: usize) -> u64 {
    (len_values as u64).saturating_mul(COLUMN_VALUE_BYTES as u64)
}

/// One column's padded footprint inside the pool, or `None` on overflow.
fn column_region_bytes(len_values: usize) -> Option<u64> {
    try_align_up(column_raw_bytes(len_values), ALIGN)
}

/// Total region a batch occupies, with the per-column limit checks.
///
/// Each column is padded to `ALIGN`, so every per-column offset inside the
/// region is aligned without a second pass.
fn batch_region_bytes(
    columns: &[(&str, &dyn ColumnSource)],
    ceiling: u64,
) -> Result<u64, AllocError> {
    let mut total = 0u64;
    for (_, source) in columns {
        let len_values = source.len();
        if len_values == 0 {
            return Err(AllocError::EmptySource);
        }
        let byte_size = column_region_bytes(len_values).ok_or(AllocError::ResourceLimit {
            resource: "batch column staging buffer",
            requested: column_raw_bytes(len_values),
            limit: ceiling,
        })?;
        // A single column bigger than the device can hold cannot be staged at
        // all, batched or not.
        if byte_size > ceiling {
            return Err(AllocError::ResourceLimit {
                resource: "batch column staging buffer",
                requested: byte_size,
                limit: ceiling,
            });
        }
        total = total
            .checked_add(byte_size)
            .ok_or(AllocError::ResourceLimit {
                resource: "batch column region",
                requested: u64::MAX,
                limit: ceiling,
            })?;
    }
    Ok(total)
}

/// One staging buffer's worth of a batch: a run of columns, their offsets
/// inside the buffer, and where the run starts inside the pool region.
///
/// Because a chunk mirrors the region layout byte for byte, the upload is one
/// `copy_buffer_to_buffer` per chunk — and one chunk covers the whole batch
/// unless the device ceiling forces a split.
struct BatchChunk {
    columns: std::ops::Range<usize>,
    column_offsets: Vec<usize>,
    region_offset: u64,
    byte_size: u64,
}

fn plan_batch_chunks(
    columns: &[(&str, &dyn ColumnSource)],
    ceiling: u64,
) -> Result<Vec<BatchChunk>, AllocError> {
    let alloc_failed = |error: std::collections::TryReserveError| AllocError::AllocationFailed {
        resource: "batch staging plan",
        reason: error.to_string(),
    };
    let mut chunks: Vec<BatchChunk> = Vec::new();
    let mut start = 0usize;
    let mut offsets: Vec<usize> = Vec::new();
    offsets.try_reserve(columns.len()).map_err(alloc_failed)?;
    let mut chunk_bytes = 0u64;
    let mut region_offset = 0u64;

    for (index, (_, source)) in columns.iter().enumerate() {
        let byte_size = column_region_bytes(source.len()).expect("sized by batch_region_bytes");
        if chunk_bytes + byte_size > ceiling && index > start {
            chunks.try_reserve(1).map_err(alloc_failed)?;
            chunks.push(BatchChunk {
                columns: start..index,
                column_offsets: std::mem::take(&mut offsets),
                region_offset,
                byte_size: chunk_bytes,
            });
            offsets
                .try_reserve(columns.len() - index)
                .map_err(alloc_failed)?;
            region_offset += chunk_bytes;
            chunk_bytes = 0;
            start = index;
        }
        offsets.push(chunk_bytes as usize);
        chunk_bytes += byte_size;
    }
    chunks.try_reserve(1).map_err(alloc_failed)?;
    chunks.push(BatchChunk {
        columns: start..columns.len(),
        column_offsets: offsets,
        region_offset,
        byte_size: chunk_bytes,
    });
    Ok(chunks)
}

fn write_scalar_source_as_pairs(
    source: &dyn ColumnSource,
    dst: ColumnPairWriter<'_>,
) -> ColumnUploadStats {
    source.write_f32_pair_le_into_with_stats(dst)
}

/// Lightweight handle handed out to the chart layer. `generation` lets
/// callers detect a stale handle after any invalidating pool mutation.
#[derive(Debug, Clone, Copy)]
pub struct ColumnHandle {
    pub generation: u32,
    pub offset: u64,
    pub byte_size: u64,
    pub len_values: usize,
}

impl ColumnHandle {
    /// Pass directly into `pool.buffer().slice(byte_range)`.
    pub fn byte_range(&self) -> std::ops::Range<u64> {
        self.offset..(self.offset + self.byte_size)
    }
}

/// Allocation failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocError {
    /// No free region is large enough (try defrag then retry).
    OutOfSpace {
        requested: u64,
        largest_free: u64,
        total_free: u64,
    },
    /// Requested GPU buffer is larger than the device can allocate.
    ResourceLimit {
        resource: &'static str,
        requested: u64,
        limit: u64,
    },
    /// Resource creation failed despite satisfying static device limits.
    AllocationFailed {
        resource: &'static str,
        reason: String,
    },
    /// A column with this id already exists.
    DuplicateId(ColumnId),
    /// Source has length zero.
    EmptySource,
    /// A monotonic identity counter cannot advance without wrapping.
    CounterExhausted { counter: &'static str },
}

impl std::fmt::Display for AllocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocError::OutOfSpace {
                requested,
                largest_free,
                total_free,
            } => write!(
                f,
                "ColumnPool out of space: need {} bytes, largest free = {}, total free = {}",
                requested, largest_free, total_free
            ),
            AllocError::ResourceLimit {
                resource,
                requested,
                limit,
            } => write!(
                f,
                "{resource} exceeds GPU buffer limit: requested {requested}, limit {limit}"
            ),
            AllocError::AllocationFailed { resource, reason } => {
                write!(f, "{resource} allocation failed: {reason}")
            }
            AllocError::DuplicateId(id) => write!(f, "ColumnPool duplicate id: {id}"),
            AllocError::EmptySource => write!(f, "ColumnPool: empty source not allowed"),
            AllocError::CounterExhausted { counter } => {
                write!(f, "ColumnPool {counter} exhausted")
            }
        }
    }
}

impl std::error::Error for AllocError {}

/// Largest single buffer this pool may create on `device`.
///
/// The pool binds its whole primary buffer as ONE storage binding — the
/// arc-length scan and the constellation star pass both do — so a buffer is
/// legal only under **both** device ceilings. Read here, at the allocation
/// site, rather than cached on the pool: one place learns the number, so the
/// renderer's view and the pool's view cannot drift apart.
fn buffer_ceiling(device: &Device) -> u64 {
    let limits = device.limits();
    buffer_ceiling_from(
        limits.max_buffer_size,
        limits.max_storage_buffer_binding_size,
    )
}

/// Pure half of [`buffer_ceiling`], split out so the case that matters —
/// a device whose storage ceiling is the tighter of the two — is testable
/// without owning a device that reports it.
fn buffer_ceiling_from(max_buffer_size: u64, max_storage_buffer_binding_size: u64) -> u64 {
    max_buffer_size.min(max_storage_buffer_binding_size)
}

fn create_buffer_checked(
    device: &Device,
    desc: &BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<Buffer, AllocError> {
    // gpu-alloc: ColumnPool
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_buffer(desc))).map_err(
        |_| AllocError::AllocationFailed {
            resource,
            reason: "wgpu Device::create_buffer panicked".into(),
        },
    )
}

/// A host-declared ceiling and the out-of-pool bytes it is measured against.
///
/// The two travel together because neither means anything alone. Held as
/// separate fields they could disagree — and they did: the caller summed the
/// out-of-pool total on every pool call even with no ceiling set, where the
/// pool never reads it, so the choice was between wasted work and passing a
/// fabricated zero that a future reader outside the budget check would take at
/// face value. As one value there is no zero to fabricate and nothing to
/// compute when there is no ceiling.
#[derive(Clone, Copy, Debug)]
pub struct GpuBudget {
    /// Ceiling for all renderer GPU bytes, pool included.
    pub ceiling_bytes: u64,
    /// GPU bytes held outside this pool (panel textures, MSAA, export targets,
    /// derived buffers), summed by the caller **just before this call** — never
    /// cached inside the pool, because a copy the caller forgets to refresh
    /// diverges silently.
    pub external_bytes: u64,
}

/// Everything an allocating pool call needs from the caller.
///
/// The pool owns its buffers but not the device, so GPU access arrives as an
/// argument. The budget rides along in the same argument so the **allocation
/// site itself** can compare limit, current usage, and ceiling before creating
/// a buffer.
///
/// `plan_relayout_capacity` charges a growth's peak as
/// `external_bytes + old capacity + new capacity` — the two pool buffers
/// coexist across the copy — and refuses the growth when that crosses the
/// ceiling.
#[derive(Clone, Copy)]
pub struct GpuAllocCtx<'a> {
    pub device: &'a Device,
    pub queue: &'a Queue,
    /// `None` = device limits are the only bound.
    pub budget: Option<GpuBudget>,
}

impl<'a> GpuAllocCtx<'a> {
    /// Context with no host ceiling. Device limits still apply.
    pub fn unbudgeted(device: &'a Device, queue: &'a Queue) -> Self {
        Self {
            device,
            queue,
            budget: None,
        }
    }
}

/// GPU column slab + CPU-side offset table.
pub struct ColumnPool {
    identity: PoolIdentity,
    primary: Buffer,
    capacity: u64,
    slots: HashMap<ColumnId, ColumnSlot>,
    /// Kept sorted by offset (coalesce and first-fit both rely on this).
    free: Vec<FreeRegion>,
    generation: u32,
    allocation_epochs: HashMap<ColumnId, u64>,
    allocation_epoch_counter: u64,
    layout_generation: u64,
    /// Ping-pong target for defrag. Lazily created on the first defrag,
    /// then alternates with `primary`.
    backup: Option<Buffer>,
    /// Whether to auto-defrag on alloc failure. Default `Manual`.
    pub defrag_policy: DefragPolicy,
    /// Whether an upload that does not fit may enlarge the pool.
    /// Default `Fixed` — released hosts read `OutOfSpace` as final.
    pub growth_policy: GrowthPolicy,
    /// Bytes whose buffer handle this pool has already dropped but whose
    /// device memory the queue may not have released yet. A budget must keep
    /// counting them until the host submission boundary clears them
    /// ([`Self::clear_retired_bytes`]) — spending them early over-commits.
    retired_bytes: u64,
    /// High-water mark of [`Self::gpu_bytes`] plus whatever transient buffer
    /// coexisted with it (upload staging, candidate primary, the new slab
    /// during a growth copy). Live bytes are derived from the buffers this
    /// pool holds and so cannot drift, but a derivation can never see a
    /// transient that lives and dies inside one call — and that transient is
    /// exactly what an upload or growth peak is made of.
    peak_bytes: u64,
    /// Device buffers created since construction. Counts objects, not bytes,
    /// so a scenario can assert how many allocations it caused.
    buffer_creations: u64,
}

enum UpsertBufferRollback {
    /// The upload used a region that was free before the transaction.  Bytes
    /// belonging to the old slot (when any) were never touched.
    InPlace,
    /// The provisional pool uses a candidate primary.  The old primary and
    /// backup are retained until commit so dropping the guard can restore the
    /// exact pre-transaction pool.
    Candidate {
        old_primary: Option<Buffer>,
        old_backup: Option<Buffer>,
        candidate_was_backup: bool,
    },
}

struct UpsertRollback {
    slots: HashMap<ColumnId, ColumnSlot>,
    free: Vec<FreeRegion>,
    generation: u32,
    allocation_epochs: HashMap<ColumnId, u64>,
    allocation_epoch_counter: u64,
    layout_generation: u64,
    buffer: UpsertBufferRollback,
}

enum PreparedUpsertTarget {
    InPlace,
    FreshCandidate(Buffer),
    BackupCandidate,
}

/// A provisionally-installed column upsert.
///
/// The upload has already been submitted when this guard is returned, but it
/// targets either a previously-free range or a candidate primary buffer.  The
/// old column bytes therefore remain intact.  Callers may build all dependent
/// GPU batches against [`Self::pool`]; [`Self::commit`] then publishes the
/// provisional state without any fallible work or allocation.  Dropping the
/// guard restores the exact old allocator, slots, identity stamps, and buffers.
#[must_use = "dropping a ColumnUpsert rolls the provisional pool state back"]
pub struct ColumnUpsert<'a> {
    pool: &'a mut ColumnPool,
    rollback: Option<UpsertRollback>,
    handle: ColumnHandle,
    replaced_existing: bool,
}

impl ColumnUpsert<'_> {
    /// The exact pool view that will remain live after [`Self::commit`].
    pub fn pool(&self) -> &ColumnPool {
        self.pool
    }

    /// Handle for the provisionally-installed column.
    pub fn handle(&self) -> ColumnHandle {
        self.handle
    }

    /// Whether this upsert replaced an existing id rather than inserting one.
    pub fn replaced_existing(&self) -> bool {
        self.replaced_existing
    }

    /// Publish the provisional state.
    ///
    /// All fallible preparation and the upload submission happened in
    /// `begin_upsert_*`; this path only moves already-owned values.
    pub fn commit(mut self) -> ColumnHandle {
        if let Some(mut rollback) = self.rollback.take()
            && let UpsertBufferRollback::Candidate {
                old_primary,
                old_backup,
                ..
            } = &mut rollback.buffer
        {
            // Keep the displaced primary as the next defragmentation target.
            // Replacing an Option and dropping the superseded backup cannot
            // allocate or return an error.
            let mut released = self
                .pool
                .backup
                .take()
                .map_or(0, |superseded| superseded.size());
            released = released.saturating_add(old_backup.take().map_or(0, |backup| backup.size()));
            self.pool.backup = old_primary.take();
            if released > 0 {
                self.pool.note_buffer_retired(released);
            }
        }
        self.handle
    }
}

/// A batch insert that becomes visible only on [`Self::commit`].
///
/// Rollback is exact and allocation-free. Every id in the batch is new, and
/// every one of them lives inside the single region the batch reserved, so
/// un-inserting them is a scan by offset — no clone of `slots`, `free` or
/// `allocation_epochs` is taken, which is the whole point at the thousands of
/// columns a matrix declares. The region itself is restored by replaying the
/// allocator's own undo record.
#[must_use = "dropping an uncommitted batch insert removes the columns again"]
pub struct ColumnBatchInsert<'a> {
    pool: &'a mut ColumnPool,
    rollback: Option<BatchInsertRollback>,
}

struct BatchInsertRollback {
    region: FreeRegion,
    undo: RegionUndo,
    epoch_counter: u64,
}

impl ColumnBatchInsert<'_> {
    /// The exact pool view that will remain live after [`Self::commit`].
    pub fn pool(&self) -> &ColumnPool {
        self.pool
    }

    /// Publish the provisional columns.
    pub fn commit(mut self) {
        self.rollback = None;
    }
}

impl Drop for ColumnBatchInsert<'_> {
    fn drop(&mut self) {
        let Some(rollback) = self.rollback.take() else {
            return;
        };
        let lo = rollback.region.offset;
        let hi = lo.saturating_add(rollback.region.size);
        // No pre-existing slot can be inside the region — it came off the free
        // list — so "lives in the region" names exactly this batch's columns.
        let in_region = |offset: u64| offset >= lo && offset < hi;
        let slots = &self.pool.slots;
        self.pool
            .allocation_epochs
            .retain(|id, _| slots.get(id).is_none_or(|slot| !in_region(slot.offset)));
        self.pool.slots.retain(|_, slot| !in_region(slot.offset));
        self.pool.allocation_epoch_counter = rollback.epoch_counter;
        rollback.undo.apply(&mut self.pool.free);
    }
}

impl Drop for ColumnUpsert<'_> {
    fn drop(&mut self) {
        let Some(mut rollback) = self.rollback.take() else {
            return;
        };

        std::mem::swap(&mut self.pool.slots, &mut rollback.slots);
        std::mem::swap(&mut self.pool.free, &mut rollback.free);
        std::mem::swap(
            &mut self.pool.allocation_epochs,
            &mut rollback.allocation_epochs,
        );
        self.pool.generation = rollback.generation;
        self.pool.allocation_epoch_counter = rollback.allocation_epoch_counter;
        self.pool.layout_generation = rollback.layout_generation;

        if let UpsertBufferRollback::Candidate {
            old_primary,
            old_backup,
            candidate_was_backup,
        } = &mut rollback.buffer
        {
            let Some(old_primary) = old_primary.take() else {
                return;
            };
            let candidate = std::mem::replace(&mut self.pool.primary, old_primary);
            if *candidate_was_backup {
                self.pool.backup = Some(candidate);
            } else {
                self.pool.backup = old_backup.take();
            }
        }
    }
}

#[must_use = "dropping a ColumnBatchUpsert leaves the live pool unchanged"]
pub(crate) struct ColumnBatchUpsert<'a> {
    pool: &'a mut ColumnPool,
    candidate: Option<ColumnPool>,
    handles: [ColumnHandle; 4],
}

impl ColumnBatchUpsert<'_> {
    pub(crate) fn pool(&self) -> &ColumnPool {
        self.candidate
            .as_ref()
            .expect("demo column batch candidate remains live")
    }

    pub(crate) fn commit(mut self) -> [ColumnHandle; 4] {
        let candidate = self
            .candidate
            .take()
            .expect("demo column batch candidate remains live");
        let ColumnPool {
            identity: _,
            primary,
            capacity: _,
            slots,
            free,
            generation,
            allocation_epochs,
            allocation_epoch_counter,
            layout_generation,
            backup,
            defrag_policy: _,
            growth_policy: _,
            retired_bytes: _,
            peak_bytes: _,
            buffer_creations: _,
        } = candidate;
        debug_assert!(backup.is_none());

        let old_primary = std::mem::replace(&mut self.pool.primary, primary);
        self.pool.slots = slots;
        self.pool.free = free;
        self.pool.generation = generation;
        self.pool.allocation_epochs = allocation_epochs;
        self.pool.allocation_epoch_counter = allocation_epoch_counter;
        self.pool.layout_generation = layout_generation;
        self.pool.backup = Some(old_primary);
        self.handles
    }
}

struct RemovalRollback {
    slots: HashMap<ColumnId, ColumnSlot>,
    free: Vec<FreeRegion>,
    generation: u32,
    allocation_epochs: HashMap<ColumnId, u64>,
}

/// A provisionally-removed column.
///
/// The removed slot and allocator metadata are visible through [`Self::pool`]
/// so dependent GPU state can be prepared before publication. Committing only
/// releases the rollback snapshot; dropping restores the exact prior state.
#[must_use = "dropping a ColumnRemoval rolls the provisional removal back"]
pub struct ColumnRemoval<'a> {
    pool: &'a mut ColumnPool,
    rollback: Option<RemovalRollback>,
}

impl ColumnRemoval<'_> {
    /// The exact pool view that will remain live after [`Self::commit`].
    pub fn pool(&self) -> &ColumnPool {
        self.pool
    }

    /// Publish the provisional removal.
    pub fn commit(mut self) -> bool {
        self.rollback.take();
        true
    }
}

impl Drop for ColumnRemoval<'_> {
    fn drop(&mut self) {
        let Some(mut rollback) = self.rollback.take() else {
            return;
        };

        std::mem::swap(&mut self.pool.slots, &mut rollback.slots);
        std::mem::swap(&mut self.pool.free, &mut rollback.free);
        std::mem::swap(
            &mut self.pool.allocation_epochs,
            &mut rollback.allocation_epochs,
        );
        self.pool.generation = rollback.generation;
    }
}

struct DefragmentRollback {
    slots: Option<HashMap<ColumnId, ColumnSlot>>,
    free: Vec<FreeRegion>,
    generation: u32,
    layout_generation: u64,
    swapped_primary: bool,
    candidate_was_backup: bool,
    /// Capacity to restore. A growth that rolls back must un-publish the new
    /// size along with the new buffer.
    capacity: u64,
}

/// A provisionally-applied column-pool defragmentation.
///
/// GPU copies have already been submitted when a relocating guard is returned.
/// Returned preparation errors occur before the provisional pool is published;
/// asynchronous device failures remain governed by wgpu's device error model.
#[must_use = "dropping a ColumnDefragment rolls the provisional layout back"]
pub struct ColumnDefragment<'a> {
    pool: &'a mut ColumnPool,
    rollback: Option<DefragmentRollback>,
    changed: bool,
    relocated: bool,
    legacy_result: bool,
    /// True when this relayout enlarged the pool. On commit the old buffer,
    /// parked in `backup` for rollback, is released.
    grown: bool,
}

impl ColumnDefragment<'_> {
    /// The exact pool view that will remain live after [`Self::commit`].
    pub fn pool(&self) -> &ColumnPool {
        self.pool
    }

    /// Whether any allocator state was normalized or relocated.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Whether live columns moved to a different backing layout.
    pub fn relocated(&self) -> bool {
        self.relocated
    }

    /// Publish the provisional state and return the legacy defragment result.
    pub fn commit(mut self) -> bool {
        self.rollback.take();
        if self.grown {
            // The old, smaller buffer parked in `backup` cannot serve a defrag
            // at the new capacity. Release it instead of carrying it.
            if let Some(old_slab) = self.pool.backup.take() {
                let bytes = old_slab.size();
                drop(old_slab);
                self.pool.note_buffer_retired(bytes);
            }
        }
        self.legacy_result
    }
}

impl Drop for ColumnDefragment<'_> {
    fn drop(&mut self) {
        let Some(mut rollback) = self.rollback.take() else {
            return;
        };

        if let Some(mut slots) = rollback.slots.take() {
            std::mem::swap(&mut self.pool.slots, &mut slots);
        }
        std::mem::swap(&mut self.pool.free, &mut rollback.free);
        self.pool.generation = rollback.generation;
        self.pool.layout_generation = rollback.layout_generation;
        self.pool.capacity = rollback.capacity;

        if rollback.swapped_primary {
            let Some(old_primary) = self.pool.backup.take() else {
                return;
            };
            let candidate = std::mem::replace(&mut self.pool.primary, old_primary);
            if rollback.candidate_was_backup {
                self.pool.backup = Some(candidate);
            }
        }
    }
}

impl ColumnPool {
    /// New pool. `capacity_bytes` is rounded up to a multiple of `ALIGN`.
    pub fn new(ctx: GpuAllocCtx<'_>, capacity_bytes: u64) -> Result<Self, AllocError> {
        let device = ctx.device;
        let max_buffer_size = buffer_ceiling(device);
        let requested = capacity_bytes.max(ALIGN);
        let capacity = try_align_up(requested, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: "column pool buffer",
            requested,
            limit: max_buffer_size,
        })?;
        if capacity > max_buffer_size {
            return Err(AllocError::ResourceLimit {
                resource: "column pool buffer",
                requested: capacity,
                limit: max_buffer_size,
            });
        }
        let primary_desc = BufferDescriptor {
            label: Some("figgy column pool primary"),
            size: capacity,
            // VERTEX | STORAGE so the pool can serve both binding kinds.
            // COPY_DST for staging→primary uploads, COPY_SRC for the
            // primary→backup defrag copy.
            usage: BufferUsages::VERTEX
                | BufferUsages::STORAGE
                | BufferUsages::COPY_DST
                | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        };
        let primary = create_buffer_checked(device, &primary_desc, "column pool buffer")?;
        Ok(Self {
            identity: PoolIdentity::new(),
            primary,
            capacity,
            slots: HashMap::new(),
            free: vec![FreeRegion {
                offset: 0,
                size: capacity,
            }],
            generation: 0,
            allocation_epochs: HashMap::new(),
            allocation_epoch_counter: 0,
            layout_generation: 0,
            backup: None,
            defrag_policy: DefragPolicy::Manual,
            growth_policy: GrowthPolicy::Fixed,
            retired_bytes: 0,
            peak_bytes: capacity,
            buffer_creations: 1,
        })
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    pub fn buffer(&self) -> &Buffer {
        &self.primary
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    #[allow(dead_code)]
    pub(crate) fn identity(&self) -> PoolIdentity {
        self.identity.clone()
    }

    pub(crate) fn layout_generation(&self) -> u64 {
        self.layout_generation
    }

    pub(crate) fn allocation_epoch(&self, id: &str) -> Option<u64> {
        self.allocation_epochs.get(id).copied()
    }

    fn checked_generation_successor(&self) -> Result<u32, AllocError> {
        self.generation
            .checked_add(1)
            .ok_or(AllocError::CounterExhausted {
                counter: "public generation",
            })
    }

    fn checked_layout_successor(&self) -> Result<u64, AllocError> {
        self.layout_generation
            .checked_add(1)
            .ok_or(AllocError::CounterExhausted {
                counter: "layout generation",
            })
    }

    fn checked_allocation_epoch_successor(&self) -> Result<u64, AllocError> {
        self.allocation_epoch_counter
            .checked_add(1)
            .ok_or(AllocError::CounterExhausted {
                counter: "allocation epoch",
            })
    }

    pub fn used_bytes(&self) -> u64 {
        self.slots.values().map(|s| s.byte_size).sum()
    }

    pub fn free_bytes(&self) -> u64 {
        self.free.iter().map(|r| r.size).sum()
    }

    /// GPU bytes this pool holds right now: the live slab plus the ping-pong
    /// backup while one exists.
    ///
    /// Derived from the buffers themselves rather than a counter, so it is the
    /// pool's own authority on its footprint and cannot drift from what the
    /// device was asked for.
    pub fn gpu_bytes(&self) -> u64 {
        self.primary
            .size()
            .saturating_add(self.backup.as_ref().map_or(0, |backup| backup.size()))
    }

    /// The ping-pong defragmentation target, when one has been materialized.
    ///
    /// Exposed for the renderer's accounting tests, which recount the pool's
    /// footprint from the buffers themselves rather than trusting the report.
    #[cfg(test)]
    pub(crate) fn backup_buffer_for_test(&self) -> Option<&Buffer> {
        self.backup.as_ref()
    }

    /// Bytes released by this pool that the device may still be holding.
    pub fn retired_bytes(&self) -> u64 {
        self.retired_bytes
    }

    /// Submission boundary: every command using retired slabs has been queued.
    pub fn clear_retired_bytes(&mut self) {
        self.retired_bytes = 0;
    }

    /// Highest footprint seen, transients included.
    pub fn peak_bytes(&self) -> u64 {
        self.peak_bytes
    }

    /// Forget the recorded peak; the next allocation starts a fresh mark.
    pub fn reset_peak_bytes(&mut self) {
        self.peak_bytes = self.gpu_bytes();
    }

    /// Device buffers this pool has created since construction.
    pub fn buffer_creations(&self) -> u64 {
        self.buffer_creations
    }

    /// Charge a buffer this pool just created whose bytes coexist with
    /// [`Self::gpu_bytes`] — a staging buffer, a candidate primary, or the
    /// larger slab a growth is copying into.
    fn note_buffer_created(&mut self, transient_bytes: u64) {
        self.buffer_creations = self.buffer_creations.saturating_add(1);
        let total = self.gpu_bytes().saturating_add(transient_bytes);
        self.peak_bytes = self.peak_bytes.max(total);
    }

    /// Credit a buffer this pool just dropped. The bytes stay in
    /// [`Self::retired_bytes`] until the submission boundary.
    fn note_buffer_retired(&mut self, bytes: u64) {
        self.retired_bytes = self.retired_bytes.saturating_add(bytes);
    }

    pub fn slot(&self, id: &str) -> Option<&ColumnSlot> {
        self.slots.get(id)
    }

    /// Drop all columns. The primary buffer is reused (capacity unchanged).
    /// Bumps public and layout generations; allocation epochs are removed
    /// without resetting their monotonic issuer.
    pub fn clear(&mut self) -> Result<(), AllocError> {
        let next_generation = self.checked_generation_successor()?;
        let next_layout_generation = self.checked_layout_successor()?;
        self.slots.clear();
        self.allocation_epochs.clear();
        self.free.clear();
        self.free.push(FreeRegion {
            offset: 0,
            size: self.capacity,
        });
        self.generation = next_generation;
        self.layout_generation = next_layout_generation;
        Ok(())
    }

    pub fn handle_for(&self, id: &str) -> Option<ColumnHandle> {
        self.slots.get(id).map(|s| ColumnHandle {
            generation: s.generation,
            offset: s.offset,
            byte_size: s.byte_size,
            len_values: s.len_values,
        })
    }

    /// True if the handle is still valid for the current public pool state.
    /// Removal, replacement, relocation, or clear invalidates previously
    /// issued handles.
    pub fn is_valid_handle(&self, h: &ColumnHandle) -> bool {
        h.generation == self.generation
    }

    /// Add a column to the pool with a zero-copy stream upload.
    ///
    /// When `defrag_policy == OnAllocFailure` and the first attempt returns
    /// `OutOfSpace`, this calls `defragment()` once and retries; the original
    /// `OutOfSpace` is returned if the retry still fails.
    pub fn add_column(
        &mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        // Only clone `id` for retry under OnAllocFailure; the default path allocates nothing extra.
        // Clone `id` only for the retry rungs; with both policies off the
        // default path allocates nothing extra.
        let defrag_retry = self.defrag_policy == DefragPolicy::OnAllocFailure;
        let grow_retry = self.growth_policy == GrowthPolicy::OnAllocFailure;
        let retry_id = (defrag_retry || grow_retry).then(|| id.clone());
        let first = self.try_add_column(id, source, ctx);
        let Err(e @ AllocError::OutOfSpace { .. }) = first else {
            return first;
        };
        let Some(rid) = retry_id else {
            return Err(e);
        };
        // Rung 1: defrag and retry.
        if defrag_retry {
            self.defragment(ctx)?;
            match self.try_add_column(rid.clone(), source, ctx) {
                Err(AllocError::OutOfSpace { .. }) if grow_retry => {}
                other => return other,
            }
        }
        if !grow_retry {
            return Err(e);
        }
        // Rung 2: enlarge and retry. A growth that cannot happen — ceiling,
        // budget, or a failed allocation — leaves the original OutOfSpace as
        // the answer, so what callers read does not change.
        let needed = try_align_up(
            (source.len() as u64).saturating_mul(COLUMN_VALUE_BYTES as u64),
            ALIGN,
        )
        .unwrap_or(u64::MAX);
        if self.grow_for_pending_upload(ctx, needed).is_err() {
            return Err(e);
        }
        self.try_add_column(rid, source, ctx)
    }

    /// Add many columns in one transaction: **one** contiguous pool region,
    /// **one** staging buffer, **one** copy, **one** submit.
    ///
    /// This is the upload path a matrix declaration needs. Calling
    /// [`Self::add_column`] per column costs a first-fit search, a staging
    /// buffer, a command encoder and a `queue.submit` *each*; at the thousands
    /// of columns a grid has, that is the whole cost of the upload. Nothing
    /// here is matrix-specific, so any host with many columns gets the same
    /// path (design §B.2, where it is named `add_matrix_columns`).
    ///
    /// What it does **not** do, unlike the four-column demo batch it replaces:
    /// it does not duplicate the whole pool buffer, it does not stage per
    /// column, and it copies no allocator plan. The region is taken as a single
    /// reservation, so a failure rolls back by returning that one region; slots
    /// are published only once every fallible step is behind us, which is why
    /// the maps are `try_reserve`d up front rather than cloned.
    ///
    /// Every id must be new and distinct within the batch — `DuplicateId`
    /// otherwise, with nothing uploaded. An empty slice is a no-op.
    pub fn add_columns(
        &mut self,
        columns: &[(&str, &dyn ColumnSource)],
        ctx: GpuAllocCtx<'_>,
    ) -> Result<(), AllocError> {
        self.begin_add_columns(columns, ctx)?.commit();
        Ok(())
    }

    /// The failure-atomic form of [`Self::add_columns`].
    ///
    /// Everything fallible — id checks, sizing, registry storage, the region,
    /// staging, the upload submission — happens here; the columns become
    /// visible to the rest of the renderer only on
    /// [`ColumnBatchInsert::commit`]. Dropping the guard un-inserts them and
    /// returns the region, so a caller that still has fallible work of its own
    /// (rebuilding a picker, publishing chart revisions) can do it against the
    /// provisional pool and abandon the whole batch on failure.
    pub fn begin_add_columns<'a>(
        &'a mut self,
        columns: &[(&str, &dyn ColumnSource)],
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnBatchInsert<'a>, AllocError> {
        if columns.is_empty() {
            return Ok(ColumnBatchInsert {
                pool: self,
                rollback: None,
            });
        }
        let (device, queue) = (ctx.device, ctx.queue);
        let ceiling = buffer_ceiling(device);

        // 1) Ids: new to the pool, and distinct inside the batch. The set is
        //    what keeps this O(n): the pairwise scan the four-column path uses
        //    is 12.5M string comparisons at 5000 columns, on the upload path.
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        seen.try_reserve(columns.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "batch column id set",
                reason: error.to_string(),
            })?;
        for (id, _) in columns {
            if self.slots.contains_key(*id) {
                return Err(AllocError::DuplicateId((*id).to_string()));
            }
            if !seen.insert(id) {
                return Err(AllocError::DuplicateId((*id).to_string()));
            }
        }

        // 2) Total region size. Each column is `ALIGN`-padded, so the region's
        //    per-column offsets are aligned by construction.
        let total = batch_region_bytes(columns, ceiling)?;

        // 3) Room, decided before the guard exists. The guard borrows the pool
        //    for its whole lifetime, so — exactly as in
        //    `begin_upsert_column_pairs_with` — there is no retry rung after a
        //    failed attempt. A pure fit check drives compaction and growth
        //    here instead, and room that cannot be made leaves step 6 to
        //    report the allocator's own `OutOfSpace` unchanged.
        self.make_room_for_batch(total, ctx)?;

        // 4) One epoch per column, reserved as a single range so a counter
        //    overflow is caught before anything is published.
        let first_epoch = self.allocation_epoch_counter;
        let last_epoch =
            first_epoch
                .checked_add(columns.len() as u64)
                .ok_or(AllocError::CounterExhausted {
                    counter: "allocation epoch",
                })?;

        // 5) Storage for the new slots, reserved while failing is still free.
        //    After this the publish loop below cannot fail, which is what makes
        //    the transaction atomic without cloning the registries.
        self.slots
            .try_reserve(columns.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "batch column registry",
                reason: error.to_string(),
            })?;
        self.allocation_epochs
            .try_reserve(columns.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "batch allocation epoch registry",
                reason: error.to_string(),
            })?;

        // 6) One reservation for the whole batch. Dropping it un-reserves.
        let reservation = alloc_region(&mut self.free, total)?;
        let region_offset = reservation.offset();

        // 7) Staging. One buffer for the batch, split into chunks only when the
        //    device cannot hold it in one — each chunk mirrors the region's
        //    layout exactly, so it lands in a single `copy_buffer_to_buffer`.
        let chunks = plan_batch_chunks(columns, ceiling)?;
        let mut staging = Vec::new();
        staging
            .try_reserve_exact(chunks.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "batch staging registry",
                reason: error.to_string(),
            })?;
        for chunk in &chunks {
            staging.push(create_buffer_checked(
                device,
                &BufferDescriptor {
                    label: Some("figgy batch column staging"),
                    size: chunk.byte_size,
                    usage: BufferUsages::COPY_SRC,
                    mapped_at_creation: true,
                },
                "batch column staging buffer",
            )?);
        }

        // ── Nothing below can fail. ───────────────────────────────────────
        let mut epoch = first_epoch;
        for (chunk, buffer) in chunks.iter().zip(&staging) {
            let mut view = buffer
                .slice(0..chunk.byte_size)
                .get_mapped_range_mut()
                .expect("batch staging is mapped at creation");
            for index in chunk.columns.clone() {
                let (id, source) = columns[index];
                let raw_bytes = column_raw_bytes(source.len());
                let byte_size = column_region_bytes(source.len()).expect("sized in step 2");
                let local = chunk.column_offsets[index - chunk.columns.start];
                let stats = write_scalar_source_as_pairs(
                    source,
                    ColumnPairWriter::new(view.slice(local..local + raw_bytes as usize)),
                );
                epoch += 1;
                self.allocation_epochs.insert(id.to_string(), epoch);
                self.slots.insert(
                    id.to_string(),
                    ColumnSlot {
                        id: id.to_string(),
                        offset: region_offset + chunk.region_offset + local as u64,
                        byte_size,
                        len_values: source.len(),
                        generation: self.generation,
                        min: source.min(),
                        max: source.max(),
                        min_positive: stats.min_positive,
                    },
                );
            }
            drop(view);
            buffer.unmap();
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy batch column upload"),
        });
        for (chunk, buffer) in chunks.iter().zip(&staging) {
            encoder.copy_buffer_to_buffer(
                buffer,
                0,
                &self.primary,
                region_offset + chunk.region_offset,
                chunk.byte_size,
            );
        }
        queue.submit(std::iter::once(encoder.finish()));

        // The free-list edit outlives the reservation guard, so take its undo
        // record: from here the batch guard owns the restoration.
        let undo = reservation.into_undo();
        self.allocation_epoch_counter = last_epoch;
        // Every staging buffer is still alive here, so the peak they contribute
        // together is `total`; the creation count stays truthful by charging
        // once per buffer actually made (one, unless the ceiling forced a
        // split).
        for _ in &staging {
            self.note_buffer_created(total);
        }
        Ok(ColumnBatchInsert {
            pool: self,
            rollback: Some(BatchInsertRollback {
                region: FreeRegion {
                    offset: region_offset,
                    size: total,
                },
                undo,
                epoch_counter: first_epoch,
            }),
        })
    }

    /// Compact and/or enlarge until one contiguous region can hold `total`.
    ///
    /// The batch counterpart of [`Self::grow_for_upsert_if_needed`], and for
    /// the same reason: the decision has to be made while the pool is still
    /// un-borrowed. Compaction happens only under
    /// [`DefragPolicy::OnAllocFailure`] and growth only under
    /// [`GrowthPolicy::OnAllocFailure`]; failing to make room is not itself an
    /// error, because `alloc_region` reports it with the exact free list.
    fn make_room_for_batch(&mut self, total: u64, ctx: GpuAllocCtx<'_>) -> Result<(), AllocError> {
        // First fit accepts the batch iff some single region is big enough.
        if self.largest_free_region() >= total {
            return Ok(());
        }
        let may_compact = self.defrag_policy == DefragPolicy::OnAllocFailure;
        if may_compact && self.free_bytes() >= total {
            // Compaction gathers every free byte into one region, so it alone
            // is the difference between fitting and not.
            self.defragment(ctx)?;
            return Ok(());
        }
        if self.growth_policy != GrowthPolicy::OnAllocFailure {
            return Ok(());
        }
        // Growth relays the pool out, which compacts as a side effect — so
        // after it every free byte is one region either way.
        let usable = if may_compact {
            self.free_bytes()
        } else {
            self.largest_free_region()
        };
        let _ = self.grow_for_pending_upload(ctx, total.saturating_sub(usable));
        Ok(())
    }

    /// The biggest single region first fit could serve from.
    fn largest_free_region(&self) -> u64 {
        self.free
            .iter()
            .map(|region| region.size)
            .max()
            .unwrap_or(0)
    }

    pub fn add_hilo_column(
        &mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        // Clone `id` only for the retry rungs; with both policies off the
        // default path allocates nothing extra.
        let defrag_retry = self.defrag_policy == DefragPolicy::OnAllocFailure;
        let grow_retry = self.growth_policy == GrowthPolicy::OnAllocFailure;
        let retry_id = (defrag_retry || grow_retry).then(|| id.clone());
        let first = self.try_add_hilo_column(id, source, ctx);
        let Err(e @ AllocError::OutOfSpace { .. }) = first else {
            return first;
        };
        let Some(rid) = retry_id else {
            return Err(e);
        };
        // Rung 1: defrag and retry.
        if defrag_retry {
            self.defragment(ctx)?;
            match self.try_add_hilo_column(rid.clone(), source, ctx) {
                Err(AllocError::OutOfSpace { .. }) if grow_retry => {}
                other => return other,
            }
        }
        if !grow_retry {
            return Err(e);
        }
        // Rung 2: enlarge and retry. A growth that cannot happen — ceiling,
        // budget, or a failed allocation — leaves the original OutOfSpace as
        // the answer, so what callers read does not change.
        let needed = try_align_up(
            (source.len() as u64).saturating_mul(COLUMN_VALUE_BYTES as u64),
            ALIGN,
        )
        .unwrap_or(u64::MAX);
        if self.grow_for_pending_upload(ctx, needed).is_err() {
            return Err(e);
        }
        self.try_add_hilo_column(rid, source, ctx)
    }

    /// Begin a failure-atomic scalar insert or same-id replacement.
    ///
    /// The returned guard exposes the exact post-commit pool view.  Dropping
    /// it restores the old view; committing it is infallible and allocation
    /// free.  Unlike `add_column`, an existing id is replaced.
    pub fn begin_upsert_column<'a>(
        &'a mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnUpsert<'a>, AllocError> {
        let device = ctx.device;
        self.begin_upsert_column_pairs_with(
            ColumnInputMeta {
                id,
                len_values: source.len(),
                min: source.min(),
                max: source.max(),
            },
            ctx,
            |dst| write_scalar_source_as_pairs(source, dst),
            |byte_size| {
                create_buffer_checked(
                    device,
                    &BufferDescriptor {
                        label: Some("figgy column upsert staging"),
                        size: byte_size,
                        usage: BufferUsages::COPY_SRC,
                        mapped_at_creation: true,
                    },
                    "column staging buffer",
                )
            },
        )
    }

    /// Begin a failure-atomic hi/lo insert or same-id replacement.
    pub fn begin_upsert_hilo_column<'a>(
        &'a mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnUpsert<'a>, AllocError> {
        let device = ctx.device;
        self.begin_upsert_column_pairs_with(
            ColumnInputMeta {
                id,
                len_values: source.len(),
                min: source.min(),
                max: source.max(),
            },
            ctx,
            |dst| source.write_f32_pair_le_into_with_stats(dst),
            |byte_size| {
                create_buffer_checked(
                    device,
                    &BufferDescriptor {
                        label: Some("figgy column upsert staging"),
                        size: byte_size,
                        usage: BufferUsages::COPY_SRC,
                        mapped_at_creation: true,
                    },
                    "column staging buffer",
                )
            },
        )
    }

    /// Failure-atomic scalar upsert when no dependent batch preparation is
    /// needed between prepare and commit.
    pub fn upsert_column(
        &mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        Ok(self.begin_upsert_column(id, source, ctx)?.commit())
    }

    /// Failure-atomic hi/lo upsert when no dependent batch preparation is
    /// needed between prepare and commit.
    pub fn upsert_hilo_column(
        &mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        Ok(self.begin_upsert_hilo_column(id, source, ctx)?.commit())
    }

    pub(crate) fn begin_demo_batch_upsert<'a>(
        &'a mut self,
        columns: [(ColumnId, &dyn ColumnSource); 4],
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnBatchUpsert<'a>, AllocError> {
        let (device, queue) = (ctx.device, ctx.queue);
        let ceiling = buffer_ceiling(device);
        struct StagedColumn {
            id: ColumnId,
            len_values: usize,
            byte_size: u64,
            min: f64,
            max: f64,
            min_positive: Option<f64>,
            staging: Buffer,
        }

        for index in 0..columns.len() {
            if columns[..index]
                .iter()
                .any(|(id, _)| id == &columns[index].0)
            {
                return Err(AllocError::DuplicateId(columns[index].0.clone()));
            }
        }

        let mut staged = Vec::new();
        // Every staging buffer stays alive until the batch submits, so the
        // charge is cumulative rather than per-buffer.
        let mut staged_bytes = 0u64;
        staged
            .try_reserve_exact(columns.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "demo column staging registry",
                reason: error.to_string(),
            })?;
        for (id, source) in columns {
            let len_values = source.len();
            if len_values == 0 {
                return Err(AllocError::EmptySource);
            }
            let raw_bytes = (len_values as u64)
                .checked_mul(COLUMN_VALUE_BYTES as u64)
                .ok_or(AllocError::ResourceLimit {
                    resource: "column staging buffer",
                    requested: u64::MAX,
                    limit: ceiling,
                })?;
            let byte_size = try_align_up(raw_bytes, ALIGN).ok_or(AllocError::ResourceLimit {
                resource: "column staging buffer",
                requested: raw_bytes,
                limit: ceiling,
            })?;
            if byte_size > ceiling {
                return Err(AllocError::ResourceLimit {
                    resource: "column staging buffer",
                    requested: byte_size,
                    limit: ceiling,
                });
            }

            let staging = create_buffer_checked(
                device,
                &BufferDescriptor {
                    label: Some("figgy demo column staging"),
                    size: byte_size,
                    usage: BufferUsages::COPY_SRC,
                    mapped_at_creation: true,
                },
                "demo column staging buffer",
            )?;
            staged_bytes = staged_bytes.saturating_add(byte_size);
            self.note_buffer_created(staged_bytes);
            let min_positive = write_staging_pairs(&staging, raw_bytes, |dst| {
                write_scalar_source_as_pairs(source, dst)
            })
            .min_positive;
            staged.push(StagedColumn {
                id,
                len_values,
                byte_size,
                min: source.min(),
                max: source.max(),
                min_positive,
                staging,
            });
        }

        let next_generation = self.checked_generation_successor()?;
        let next_layout_generation = self.checked_layout_successor()?;
        let mut next_allocation_epoch = self.allocation_epoch_counter;
        let mut replacement_epochs = Vec::new();
        replacement_epochs
            .try_reserve_exact(staged.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "demo allocation epochs",
                reason: error.to_string(),
            })?;
        for _ in &staged {
            next_allocation_epoch =
                next_allocation_epoch
                    .checked_add(1)
                    .ok_or(AllocError::CounterExhausted {
                        counter: "allocation epoch",
                    })?;
            replacement_epochs.push(next_allocation_epoch);
        }

        let is_replaced = |id: &str| staged.iter().any(|column| column.id == id);
        let survivor_count = self.slots.len().saturating_sub(
            staged
                .iter()
                .filter(|column| self.slots.contains_key(&column.id))
                .count(),
        );
        let final_count =
            survivor_count
                .checked_add(staged.len())
                .ok_or(AllocError::AllocationFailed {
                    resource: "demo column registry",
                    reason: "column count overflow".into(),
                })?;
        let mut survivor_ids = Vec::new();
        survivor_ids
            .try_reserve_exact(survivor_count)
            .map_err(|error| AllocError::AllocationFailed {
                resource: "demo survivor order",
                reason: error.to_string(),
            })?;
        survivor_ids.extend(
            self.slots
                .values()
                .filter(|slot| !is_replaced(&slot.id))
                .map(|slot| slot.id.clone()),
        );
        survivor_ids.sort_by_key(|id| self.slots[id].offset);

        let mut planned_slots = HashMap::new();
        planned_slots
            .try_reserve(final_count)
            .map_err(|error| AllocError::AllocationFailed {
                resource: "demo column registry",
                reason: error.to_string(),
            })?;
        let mut planned_allocation_epochs = HashMap::new();
        planned_allocation_epochs
            .try_reserve(final_count)
            .map_err(|error| AllocError::AllocationFailed {
                resource: "demo allocation epoch registry",
                reason: error.to_string(),
            })?;
        let mut next_offset = 0u64;
        for id in &survivor_ids {
            let old = &self.slots[id];
            let end = next_offset
                .checked_add(old.byte_size)
                .ok_or_else(|| out_of_space(old.byte_size, &self.free))?;
            if end > self.capacity {
                return Err(out_of_space(old.byte_size, &self.free));
            }
            let mut slot = old.clone();
            slot.offset = next_offset;
            slot.generation = next_generation;
            planned_allocation_epochs.insert(
                id.clone(),
                *self
                    .allocation_epochs
                    .get(id)
                    .expect("live column has an allocation epoch"),
            );
            planned_slots.insert(id.clone(), slot);
            next_offset = end;
        }

        let mut handles = Vec::new();
        handles
            .try_reserve_exact(staged.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "demo column handles",
                reason: error.to_string(),
            })?;
        for (column, allocation_epoch) in staged.iter().zip(replacement_epochs) {
            let end = next_offset.checked_add(column.byte_size).ok_or_else(|| {
                out_of_space(
                    column.byte_size,
                    &[FreeRegion {
                        offset: next_offset,
                        size: self.capacity.saturating_sub(next_offset),
                    }],
                )
            })?;
            if end > self.capacity {
                return Err(out_of_space(
                    column.byte_size,
                    &[FreeRegion {
                        offset: next_offset,
                        size: self.capacity.saturating_sub(next_offset),
                    }],
                ));
            }
            let slot = ColumnSlot {
                id: column.id.clone(),
                offset: next_offset,
                byte_size: column.byte_size,
                len_values: column.len_values,
                generation: next_generation,
                min: column.min,
                max: column.max,
                min_positive: column.min_positive,
            };
            handles.push(ColumnHandle {
                generation: slot.generation,
                offset: slot.offset,
                byte_size: slot.byte_size,
                len_values: slot.len_values,
            });
            planned_allocation_epochs.insert(column.id.clone(), allocation_epoch);
            planned_slots.insert(column.id.clone(), slot);
            next_offset = end;
        }
        let mut planned_free = Vec::new();
        if next_offset < self.capacity {
            planned_free
                .try_reserve_exact(1)
                .map_err(|error| AllocError::AllocationFailed {
                    resource: "demo free list",
                    reason: error.to_string(),
                })?;
            planned_free.push(FreeRegion {
                offset: next_offset,
                size: self.capacity - next_offset,
            });
        }

        let primary = create_buffer_checked(
            device,
            &BufferDescriptor {
                label: Some("figgy demo column pool candidate"),
                size: self.capacity,
                usage: BufferUsages::VERTEX
                    | BufferUsages::STORAGE
                    | BufferUsages::COPY_DST
                    | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "demo column pool candidate",
        )?;
        self.note_buffer_created(self.capacity.saturating_add(staged_bytes));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy demo column batch"),
        });
        for id in &survivor_ids {
            let old = &self.slots[id];
            let new = &planned_slots[id];
            encoder.copy_buffer_to_buffer(
                &self.primary,
                old.offset,
                &primary,
                new.offset,
                old.byte_size,
            );
        }
        for column in &staged {
            let slot = &planned_slots[&column.id];
            encoder.copy_buffer_to_buffer(
                &column.staging,
                0,
                &primary,
                slot.offset,
                slot.byte_size,
            );
        }
        queue.submit(std::iter::once(encoder.finish()));

        let handles: [ColumnHandle; 4] = handles
            .try_into()
            .expect("demo batch always prepares exactly four handles");
        let candidate = ColumnPool {
            identity: self.identity.clone(),
            primary,
            capacity: self.capacity,
            slots: planned_slots,
            free: planned_free,
            generation: next_generation,
            allocation_epochs: planned_allocation_epochs,
            allocation_epoch_counter: next_allocation_epoch,
            layout_generation: next_layout_generation,
            backup: None,
            defrag_policy: self.defrag_policy,
            growth_policy: self.growth_policy,
            // The candidate is a shell that only carries the new buffer and
            // layout across to `commit`. Its allocations are charged to the
            // real pool at the creation site, so the shell meters nothing.
            retired_bytes: 0,
            peak_bytes: 0,
            buffer_creations: 0,
        };
        Ok(ColumnBatchUpsert {
            pool: self,
            candidate: Some(candidate),
            handles,
        })
    }

    fn begin_upsert_column_pairs_with<'a>(
        &'a mut self,
        input: ColumnInputMeta,
        ctx: GpuAllocCtx<'_>,
        write_pairs: impl FnOnce(ColumnPairWriter<'_>) -> ColumnUploadStats,
        create_staging: impl FnOnce(u64) -> Result<Buffer, AllocError>,
    ) -> Result<ColumnUpsert<'a>, AllocError> {
        let (device, queue) = (ctx.device, ctx.queue);
        let ceiling = buffer_ceiling(device);
        let ColumnInputMeta {
            id,
            len_values: n,
            min,
            max,
        } = input;
        if n == 0 {
            return Err(AllocError::EmptySource);
        }

        let raw_bytes =
            (n as u64)
                .checked_mul(COLUMN_VALUE_BYTES as u64)
                .ok_or(AllocError::ResourceLimit {
                    resource: "column staging buffer",
                    requested: u64::MAX,
                    limit: ceiling,
                })?;
        let byte_size = try_align_up(raw_bytes, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: "column staging buffer",
            requested: raw_bytes,
            limit: ceiling,
        })?;
        if byte_size > ceiling {
            return Err(AllocError::ResourceLimit {
                resource: "column staging buffer",
                requested: byte_size,
                limit: ceiling,
            });
        }
        // Growth, when the policy allows it, has to be decided *before* the
        // upsert begins: the returned guard borrows the pool for its whole
        // lifetime, so there is no way to retry after a failed attempt. A
        // pure `&self` fit check decides, and a growth that cannot happen
        // leaves the attempt below to report the exact error it always did.
        self.grow_for_upsert_if_needed(&id, byte_size, ctx);
        let allocation_epoch = self.checked_allocation_epoch_successor()?;

        // Complete every source-dependent/fallible staging operation before
        // touching allocator metadata or the live primary buffer.
        let staging = create_staging(byte_size)?;
        self.note_buffer_created(byte_size);
        let min_positive = write_staging_pairs(&staging, raw_bytes, write_pairs).min_positive;

        let old_slot = self.slots.get(&id).cloned();
        let replaced_existing = old_slot.is_some();
        let mut planned_slots = self.slots.clone();
        let mut planned_free = self.free.clone();
        let mut planned_allocation_epochs = self.allocation_epochs.clone();

        let direct_offset = alloc_region_from(&mut planned_free, byte_size);
        let (target, new_offset, mut command_encoder) = match direct_offset {
            Ok(offset) => {
                if let Some(old) = old_slot.as_ref() {
                    planned_free.push(FreeRegion {
                        offset: old.offset,
                        size: old.byte_size,
                    });
                    Self::coalesce_free(&mut planned_free);
                }
                let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy column atomic upsert"),
                });
                (PreparedUpsertTarget::InPlace, offset, encoder)
            }
            Err(direct_error) => {
                let mut virtual_free = self.free.clone();
                if let Some(old) = old_slot.as_ref() {
                    virtual_free.push(FreeRegion {
                        offset: old.offset,
                        size: old.byte_size,
                    });
                    coalesce_regions(&mut virtual_free);
                }
                let total_final_free: u64 = virtual_free.iter().map(|region| region.size).sum();
                let may_compact =
                    replaced_existing || self.defrag_policy == DefragPolicy::OnAllocFailure;
                if !may_compact {
                    return Err(direct_error);
                }
                if total_final_free < byte_size {
                    return Err(out_of_space(byte_size, &virtual_free));
                }

                // A same-primary copy could overwrite old bytes before the
                // caller's dependent batches are ready.  Pack survivors into
                // a candidate instead; the old primary stays live in the
                // rollback record until commit.
                planned_slots.remove(&id);
                let mut order: Vec<ColumnId> = planned_slots.keys().cloned().collect();
                order.sort_by_key(|column_id| self.slots[column_id].offset);
                let mut next = 0;
                for column_id in &order {
                    let slot = planned_slots
                        .get_mut(column_id)
                        .expect("planned survivor remains present");
                    slot.offset = next;
                    next = align_up(next + slot.byte_size, ALIGN);
                }
                let offset = next;
                next = align_up(next + byte_size, ALIGN);
                if next > self.capacity {
                    return Err(out_of_space(byte_size, &virtual_free));
                }
                planned_free.clear();
                if next < self.capacity {
                    planned_free.push(FreeRegion {
                        offset: next,
                        size: self.capacity - next,
                    });
                }

                let fresh_candidate = if self.backup.is_none() {
                    Some(create_buffer_checked(
                        device,
                        &BufferDescriptor {
                            label: Some("figgy column pool atomic upsert candidate"),
                            size: self.capacity,
                            usage: BufferUsages::VERTEX
                                | BufferUsages::STORAGE
                                | BufferUsages::COPY_DST
                                | BufferUsages::COPY_SRC,
                            mapped_at_creation: false,
                        },
                        "column pool upsert candidate",
                    )?)
                } else {
                    None
                };
                if fresh_candidate.is_some() {
                    self.note_buffer_created(self.capacity.saturating_add(byte_size));
                }
                let candidate = fresh_candidate
                    .as_ref()
                    .or(self.backup.as_ref())
                    .expect("atomic upsert candidate was prepared");
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy column atomic upsert candidate"),
                });
                for column_id in &order {
                    let old = &self.slots[column_id];
                    let new = &planned_slots[column_id];
                    encoder.copy_buffer_to_buffer(
                        &self.primary,
                        old.offset,
                        candidate,
                        new.offset,
                        old.byte_size,
                    );
                }
                let target = match fresh_candidate {
                    Some(candidate) => PreparedUpsertTarget::FreshCandidate(candidate),
                    None => PreparedUpsertTarget::BackupCandidate,
                };
                (target, offset, encoder)
            }
        };

        let relocates = !matches!(&target, PreparedUpsertTarget::InPlace);
        let invalidates_handles = replaced_existing || relocates;
        let next_generation = if invalidates_handles {
            self.checked_generation_successor()?
        } else {
            self.generation
        };
        let next_layout_generation = if relocates {
            self.checked_layout_successor()?
        } else {
            self.layout_generation
        };
        if invalidates_handles {
            for slot in planned_slots.values_mut() {
                slot.generation = next_generation;
            }
        }

        let slot = ColumnSlot {
            id: id.clone(),
            offset: new_offset,
            byte_size,
            len_values: n,
            generation: next_generation,
            min,
            max,
            min_positive,
        };
        let handle = ColumnHandle {
            generation: slot.generation,
            offset: slot.offset,
            byte_size: slot.byte_size,
            len_values: slot.len_values,
        };
        planned_allocation_epochs.insert(id.clone(), allocation_epoch);
        planned_slots.insert(id, slot);
        command_encoder.copy_buffer_to_buffer(
            &staging,
            0,
            match &target {
                PreparedUpsertTarget::InPlace => &self.primary,
                PreparedUpsertTarget::FreshCandidate(candidate) => candidate,
                PreparedUpsertTarget::BackupCandidate => self
                    .backup
                    .as_ref()
                    .expect("atomic upsert backup candidate remains available"),
            },
            new_offset,
            byte_size,
        );
        let upload = command_encoder.finish();

        // queue.submit/device loss is deliberately outside the Result
        // rollback boundary.  Every Result-returning and allocating operation
        // is complete; only ownership moves follow this submission.
        queue.submit(std::iter::once(upload));

        let buffer_rollback = match target {
            PreparedUpsertTarget::InPlace => UpsertBufferRollback::InPlace,
            PreparedUpsertTarget::FreshCandidate(candidate) => {
                let old_backup = self.backup.take();
                let old_primary = std::mem::replace(&mut self.primary, candidate);
                UpsertBufferRollback::Candidate {
                    old_primary: Some(old_primary),
                    old_backup,
                    candidate_was_backup: false,
                }
            }
            PreparedUpsertTarget::BackupCandidate => {
                let candidate = self
                    .backup
                    .take()
                    .expect("submitted backup candidate remains available");
                let old_primary = std::mem::replace(&mut self.primary, candidate);
                UpsertBufferRollback::Candidate {
                    old_primary: Some(old_primary),
                    old_backup: None,
                    candidate_was_backup: true,
                }
            }
        };
        let old_slots = std::mem::replace(&mut self.slots, planned_slots);
        let old_free = std::mem::replace(&mut self.free, planned_free);
        let old_generation = std::mem::replace(&mut self.generation, next_generation);
        let old_allocation_epochs =
            std::mem::replace(&mut self.allocation_epochs, planned_allocation_epochs);
        let old_allocation_epoch_counter =
            std::mem::replace(&mut self.allocation_epoch_counter, allocation_epoch);
        let old_layout_generation =
            std::mem::replace(&mut self.layout_generation, next_layout_generation);

        Ok(ColumnUpsert {
            pool: self,
            rollback: Some(UpsertRollback {
                slots: old_slots,
                free: old_free,
                generation: old_generation,
                allocation_epochs: old_allocation_epochs,
                allocation_epoch_counter: old_allocation_epoch_counter,
                layout_generation: old_layout_generation,
                buffer: buffer_rollback,
            }),
            handle,
            replaced_existing,
        })
    }

    /// Single attempt without auto-retry. Internal + test use.
    ///
    /// 1. First-fit allocation from the free list.
    /// 2. Create a `mapped_at_creation: true` staging buffer.
    /// 3. `ColumnSource::write_f32_pair_le_into_with_stats` writes bytes and
    ///    collects encoded-value stats through the write-only mapped view.
    /// 4. Unmap, encode a staging→primary copy, submit.
    fn try_add_column(
        &mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        self.try_add_column_pairs(
            ColumnInputMeta {
                id,
                len_values: source.len(),
                min: source.min(),
                max: source.max(),
            },
            ctx,
            |dst| write_scalar_source_as_pairs(source, dst),
        )
    }

    fn try_add_hilo_column(
        &mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        self.try_add_column_pairs(
            ColumnInputMeta {
                id,
                len_values: source.len(),
                min: source.min(),
                max: source.max(),
            },
            ctx,
            |dst| source.write_f32_pair_le_into_with_stats(dst),
        )
    }

    fn try_add_column_pairs(
        &mut self,
        input: ColumnInputMeta,
        ctx: GpuAllocCtx<'_>,
        write_pairs: impl FnOnce(ColumnPairWriter<'_>) -> ColumnUploadStats,
    ) -> Result<ColumnHandle, AllocError> {
        let device = ctx.device;
        self.try_add_column_pairs_with(input, ctx, write_pairs, |byte_size| {
            create_buffer_checked(
                device,
                &BufferDescriptor {
                    label: Some("figgy column staging"),
                    size: byte_size,
                    usage: BufferUsages::COPY_SRC,
                    mapped_at_creation: true,
                },
                "column staging buffer",
            )
        })
    }

    fn try_add_column_pairs_with(
        &mut self,
        input: ColumnInputMeta,
        ctx: GpuAllocCtx<'_>,
        write_pairs: impl FnOnce(ColumnPairWriter<'_>) -> ColumnUploadStats,
        create_staging: impl FnOnce(u64) -> Result<Buffer, AllocError>,
    ) -> Result<ColumnHandle, AllocError> {
        let (device, queue) = (ctx.device, ctx.queue);
        let ceiling = buffer_ceiling(device);
        let ColumnInputMeta {
            id,
            len_values: n,
            min,
            max,
        } = input;
        if self.slots.contains_key(&id) {
            return Err(AllocError::DuplicateId(id));
        }
        if n == 0 {
            return Err(AllocError::EmptySource);
        }

        let raw_bytes =
            (n as u64)
                .checked_mul(COLUMN_VALUE_BYTES as u64)
                .ok_or(AllocError::ResourceLimit {
                    resource: "column staging buffer",
                    requested: u64::MAX,
                    limit: ceiling,
                })?;
        let byte_size = try_align_up(raw_bytes, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: "column staging buffer",
            requested: raw_bytes,
            limit: ceiling,
        })?;
        if byte_size > ceiling {
            return Err(AllocError::ResourceLimit {
                resource: "column staging buffer",
                requested: byte_size,
                limit: ceiling,
            });
        }
        let allocation_epoch = self.checked_allocation_epoch_successor()?;

        let reservation = alloc_region(&mut self.free, byte_size)?;
        let region_offset = reservation.offset();

        // The source writes and collects stats while the write-only view lives.
        let staging = create_staging(byte_size)?;
        let min_positive = write_staging_pairs(&staging, raw_bytes, write_pairs).min_positive;

        let slot = ColumnSlot {
            id: id.clone(),
            offset: region_offset,
            byte_size,
            len_values: n,
            generation: self.generation,
            min,
            max,
            min_positive,
        };
        let handle = ColumnHandle {
            generation: slot.generation,
            offset: slot.offset,
            byte_size: slot.byte_size,
            len_values: slot.len_values,
        };
        let mut planned_slots = self.slots.clone();
        let mut planned_allocation_epochs = self.allocation_epochs.clone();
        planned_allocation_epochs.insert(id.clone(), allocation_epoch);
        planned_slots.insert(id, slot);

        // staging → primary[region_offset..] (GPU-internal copy).
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy column upload encoder"),
        });
        enc.copy_buffer_to_buffer(&staging, 0, &self.primary, region_offset, byte_size);
        queue.submit(std::iter::once(enc.finish()));

        self.slots = planned_slots;
        self.allocation_epochs = planned_allocation_epochs;
        self.allocation_epoch_counter = allocation_epoch;
        reservation.commit();
        // Charged here rather than at the `create_staging` call because the
        // reservation borrows the free list until it commits. `staging` is
        // still alive at this point, so the peak it contributes to is real.
        self.note_buffer_created(byte_size);
        Ok(handle)
    }

    /// Provisionally remove a column so dependent GPU state can be prepared.
    pub fn begin_remove_column(
        &mut self,
        id: &str,
    ) -> Result<Option<ColumnRemoval<'_>>, AllocError> {
        let Some(slot) = self.slots.get(id).cloned() else {
            return Ok(None);
        };
        let next_generation = self.checked_generation_successor()?;
        let mut planned_slots = self.slots.clone();
        planned_slots.remove(id);
        for slot in planned_slots.values_mut() {
            slot.generation = next_generation;
        }
        let mut planned_free = self.free.clone();
        planned_free.push(FreeRegion {
            offset: slot.offset,
            size: slot.byte_size,
        });
        Self::coalesce_free(&mut planned_free);
        let mut planned_allocation_epochs = self.allocation_epochs.clone();
        planned_allocation_epochs.remove(id);

        let old_slots = std::mem::replace(&mut self.slots, planned_slots);
        let old_free = std::mem::replace(&mut self.free, planned_free);
        let old_allocation_epochs =
            std::mem::replace(&mut self.allocation_epochs, planned_allocation_epochs);
        let old_generation = std::mem::replace(&mut self.generation, next_generation);

        Ok(Some(ColumnRemoval {
            pool: self,
            rollback: Some(RemovalRollback {
                slots: old_slots,
                free: old_free,
                generation: old_generation,
                allocation_epochs: old_allocation_epochs,
            }),
        }))
    }

    /// Remove a column. Returns its region to the free list and coalesces
    /// with neighbors. Public handles are invalidated, the removed allocation
    /// epoch is discarded, and the layout generation remains unchanged.
    pub fn remove_column(&mut self, id: &str) -> Result<bool, AllocError> {
        let Some(removal) = self.begin_remove_column(id)? else {
            return Ok(false);
        };
        Ok(removal.commit())
    }

    /// Pack every live column tightly from offset 0 of `primary` (ping-pong).
    ///
    /// Algorithm:
    /// 1. Sort slots by current offset (preserves cache locality).
    /// 2. Compute new ALIGN-rounded packed offsets.
    /// 3. If already packed, normalize the free list to one tail region and
    ///    return false.
    /// 4. Lazily create `backup` (same capacity / usage as `primary`).
    /// 5. `copy_buffer_to_buffer` each slot from `primary[old_off..]` into
    ///    `backup[new_off..]` — all GPU-internal, no PCIe traffic.
    /// 6. Submit, then swap `primary <-> backup`.
    /// 7. Bump public and layout generations, invalidating outstanding handles.
    /// 8. Update slot offsets/generation; free list = single tail region
    ///    `[next..capacity)`.
    ///
    /// Returns true iff something actually moved (caller must re-fetch
    /// handles via `handle_for`).
    /// Repack every live column into a buffer of `target_capacity`.
    ///
    /// `target_capacity == capacity()` is a defragmentation and behaves exactly
    /// as it always has. A larger target is growth: the survivors are copied
    /// into a bigger buffer by the same GPU-internal copy, and `capacity` is
    /// republished with them. Shrinking is refused rather than silently treated
    /// as a defrag.
    ///
    /// Growth drops `backup` before allocating, so the peak is the old buffer
    /// plus the new one rather than three buffers, and because a backup sized
    /// for the old capacity cannot serve a later defrag at the new one.
    pub fn begin_relayout(
        &mut self,
        ctx: GpuAllocCtx<'_>,
        target_capacity: u64,
    ) -> Result<ColumnDefragment<'_>, AllocError> {
        let (device, queue) = (ctx.device, ctx.queue);
        let capacity = self.plan_relayout_capacity(ctx, target_capacity)?;
        let previous_capacity = self.capacity;
        let growing = capacity > previous_capacity;
        // Empty pool with no size change: just normalize the free list.
        if self.slots.is_empty() && !growing {
            let already =
                self.free.len() == 1 && self.free[0].offset == 0 && self.free[0].size == capacity;
            if already {
                return Ok(ColumnDefragment {
                    pool: self,
                    rollback: None,
                    changed: false,
                    relocated: false,
                    legacy_result: false,
                    grown: false,
                });
            }
            let next_generation = self.checked_generation_successor()?;
            let next_layout_generation = self.checked_layout_successor()?;
            let normalized_free = vec![FreeRegion {
                offset: 0,
                size: capacity,
            }];
            let old_free = std::mem::replace(&mut self.free, normalized_free);
            let old_generation = std::mem::replace(&mut self.generation, next_generation);
            let old_layout_generation =
                std::mem::replace(&mut self.layout_generation, next_layout_generation);
            return Ok(ColumnDefragment {
                pool: self,
                rollback: Some(DefragmentRollback {
                    slots: None,
                    free: old_free,
                    generation: old_generation,
                    layout_generation: old_layout_generation,
                    swapped_primary: false,
                    candidate_was_backup: false,
                    capacity: previous_capacity,
                }),
                changed: true,
                relocated: false,
                legacy_result: true,
                grown: false,
            });
        }

        // Pack in the current offset order.
        let mut order: Vec<ColumnId> = self.slots.keys().cloned().collect();
        order.sort_by_key(|id| self.slots[id].offset);

        let mut new_offsets: Vec<u64> = Vec::with_capacity(order.len());
        let mut next: u64 = 0;
        for id in &order {
            new_offsets.push(next);
            next = align_up(next + self.slots[id].byte_size, ALIGN);
        }

        // Already packed? Normalize free list and return false.
        let already_packed = order
            .iter()
            .zip(new_offsets.iter())
            .all(|(id, &n)| self.slots[id].offset == n);
        if already_packed && !growing {
            let tail_ok = self.free.len() <= 1
                && self
                    .free
                    .first()
                    .is_none_or(|r| r.offset == next && r.offset + r.size == capacity);
            if tail_ok {
                return Ok(ColumnDefragment {
                    pool: self,
                    rollback: None,
                    changed: false,
                    relocated: false,
                    legacy_result: false,
                    grown: false,
                });
            }
            let mut normalized_free = Vec::with_capacity(usize::from(next < capacity));
            if next < capacity {
                normalized_free.push(FreeRegion {
                    offset: next,
                    size: capacity - next,
                });
            }
            let generation = self.generation;
            let layout_generation = self.layout_generation;
            let old_free = std::mem::replace(&mut self.free, normalized_free);
            return Ok(ColumnDefragment {
                pool: self,
                rollback: Some(DefragmentRollback {
                    slots: None,
                    free: old_free,
                    generation,
                    layout_generation,
                    swapped_primary: false,
                    candidate_was_backup: false,
                    capacity: previous_capacity,
                }),
                changed: true,
                relocated: false,
                legacy_result: false,
                grown: false,
            });
        }
        let next_generation = self.checked_generation_successor()?;
        let next_layout_generation = self.checked_layout_successor()?;

        let mut planned_slots = self.slots.clone();
        for (id, &new_off) in order.iter().zip(new_offsets.iter()) {
            if let Some(slot) = planned_slots.get_mut(id) {
                slot.offset = new_off;
                slot.generation = next_generation;
            }
        }
        let mut planned_free = Vec::with_capacity(usize::from(next < capacity));
        if next < capacity {
            planned_free.push(FreeRegion {
                offset: next,
                size: capacity - next,
            });
        }

        // Growth cannot reuse a backup sized for the old capacity, and holding
        // it would make the peak three buffers instead of two.
        if growing && let Some(stale_backup) = self.backup.take() {
            self.note_buffer_retired(stale_backup.size());
        }
        // Lazily create backup with the same capacity/usage as primary.
        let candidate_was_backup = self.backup.is_some();
        if self.backup.is_none() {
            let backup_desc = BufferDescriptor {
                label: Some("figgy column pool backup"),
                size: capacity,
                usage: BufferUsages::VERTEX
                    | BufferUsages::STORAGE
                    | BufferUsages::COPY_DST
                    | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            };
            let fresh_backup = create_buffer_checked(device, &backup_desc, "column pool backup")?;
            self.note_buffer_created(capacity);
            self.backup = Some(fresh_backup);
        }

        // primary[old_off..] -> backup[new_off..] (GPU-internal copy).
        // The `is_none` branch above guarantees `backup` is `Some`; the
        // checked error below exists only to handle invariant violations.
        {
            let backup = self
                .backup
                .as_ref()
                .ok_or_else(|| AllocError::AllocationFailed {
                    resource: "column pool defragmentation",
                    reason: "backup buffer missing after preparation".into(),
                })?;
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy column pool defrag"),
            });
            for (id, &new_off) in order.iter().zip(new_offsets.iter()) {
                let slot = &self.slots[id];
                enc.copy_buffer_to_buffer(
                    &self.primary,
                    slot.offset,
                    backup,
                    new_off,
                    slot.byte_size,
                );
            }
            queue.submit(std::iter::once(enc.finish()));
        }

        // primary <-> backup ping-pong swap.
        let new_primary = self
            .backup
            .take()
            .ok_or_else(|| AllocError::AllocationFailed {
                resource: "column pool defragmentation",
                reason: "backup buffer missing before publication".into(),
            })?;
        let old_primary = std::mem::replace(&mut self.primary, new_primary);
        self.backup = Some(old_primary);

        self.capacity = capacity;
        let old_slots = std::mem::replace(&mut self.slots, planned_slots);
        let old_free = std::mem::replace(&mut self.free, planned_free);
        let old_generation = std::mem::replace(&mut self.generation, next_generation);
        let old_layout_generation =
            std::mem::replace(&mut self.layout_generation, next_layout_generation);

        Ok(ColumnDefragment {
            pool: self,
            rollback: Some(DefragmentRollback {
                slots: Some(old_slots),
                free: old_free,
                generation: old_generation,
                layout_generation: old_layout_generation,
                swapped_primary: true,
                candidate_was_backup,
                capacity: previous_capacity,
            }),
            changed: true,
            relocated: true,
            legacy_result: true,
            grown: growing,
        })
    }

    /// Enlarge before an upsert that the current layout cannot satisfy.
    ///
    /// The upsert path is the only upload route the renderer exposes, so
    /// without this the growth policy would be unreachable from every public
    /// host API. It grows by the *deficit* — what the existing machinery
    /// (a contiguous free region, or the compaction the upsert would do
    /// anyway) cannot cover — so a pool that merely needs packing is packed
    /// rather than enlarged, and the bytes-per-value ratio stays flat.
    ///
    /// Silent on failure by design: the caller then hits the same
    /// `OutOfSpace` it produced before growth existed, which is the error
    /// released hosts already handle.
    fn grow_for_upsert_if_needed(&mut self, id: &str, byte_size: u64, ctx: GpuAllocCtx<'_>) {
        if self.growth_policy != GrowthPolicy::OnAllocFailure {
            return;
        }
        let replaced_existing = self.slots.contains_key(id);
        let may_compact = replaced_existing || self.defrag_policy == DefragPolicy::OnAllocFailure;
        let usable = if may_compact {
            // Compaction gathers every free byte, and the bytes of the column
            // being replaced become free as part of the same transaction.
            self.free_bytes()
                .saturating_add(self.slots.get(id).map_or(0, |slot| slot.byte_size))
        } else {
            // Without compaction only one contiguous region can serve it.
            self.largest_free_region()
        };
        if usable >= byte_size {
            return;
        }
        let deficit = byte_size - usable;
        let _ = self.grow_for_pending_upload(ctx, deficit);
    }

    /// Enlarge enough to hold `needed_bytes` more, preferring to double.
    ///
    /// Doubling keeps repeated uploads from relaying out on every column; the
    /// device ceiling caps it, and the result is never below what the pending
    /// upload actually needs. `plan_relayout_capacity` still has the final say
    /// on ceiling and budget.
    fn grow_for_pending_upload(
        &mut self,
        ctx: GpuAllocCtx<'_>,
        needed_bytes: u64,
    ) -> Result<(), AllocError> {
        let ceiling = buffer_ceiling(ctx.device);
        let minimum = self
            .capacity
            .checked_add(needed_bytes)
            .ok_or(AllocError::ResourceLimit {
                resource: "column pool growth",
                requested: u64::MAX,
                limit: ceiling,
            })?;
        let doubled = self.capacity.saturating_mul(2).max(minimum);
        let target = doubled.min(ceiling).max(minimum);
        self.grow_to(ctx, target)?;
        Ok(())
    }

    /// Resolve and validate the capacity a relayout may publish.
    ///
    /// Reads the device ceiling here, at the allocation decision, and compares
    /// it against the caller's budget minus what is already held elsewhere.
    /// The peak charged to the budget is the old buffer plus the new one,
    /// because both exist between the copy and the swap.
    fn plan_relayout_capacity(
        &self,
        ctx: GpuAllocCtx<'_>,
        target_capacity: u64,
    ) -> Result<u64, AllocError> {
        let ceiling = buffer_ceiling(ctx.device);
        let requested = target_capacity.max(ALIGN);
        let target = try_align_up(requested, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: "column pool relayout",
            requested,
            limit: ceiling,
        })?;
        if target < self.capacity {
            return Err(AllocError::AllocationFailed {
                resource: "column pool relayout",
                reason: "shrinking the pool is not supported".into(),
            });
        }
        if target == self.capacity {
            return Ok(target);
        }
        if target > ceiling {
            return Err(AllocError::ResourceLimit {
                resource: "column pool growth",
                requested: target,
                limit: ceiling,
            });
        }
        if let Some(budget) = ctx.budget {
            // Old and new buffers coexist across the copy; backup is released
            // first, so it is not part of the peak.
            let peak = budget
                .external_bytes
                .checked_add(self.capacity)
                .and_then(|sum| sum.checked_add(target))
                .ok_or(AllocError::ResourceLimit {
                    resource: "column pool growth",
                    requested: u64::MAX,
                    limit: budget.ceiling_bytes,
                })?;
            if peak > budget.ceiling_bytes {
                return Err(AllocError::ResourceLimit {
                    resource: "column pool growth",
                    requested: peak,
                    limit: budget.ceiling_bytes,
                });
            }
        }
        Ok(target)
    }

    /// Pack every live column tightly at the current capacity.
    pub fn begin_defragment(
        &mut self,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnDefragment<'_>, AllocError> {
        self.begin_relayout(ctx, self.capacity)
    }

    /// Relayout into a larger buffer and publish immediately.
    pub fn grow_to(
        &mut self,
        ctx: GpuAllocCtx<'_>,
        target_capacity: u64,
    ) -> Result<bool, AllocError> {
        Ok(self.begin_relayout(ctx, target_capacity)?.commit())
    }

    /// Pack every live column tightly and publish immediately.
    pub fn defragment(&mut self, ctx: GpuAllocCtx<'_>) -> Result<bool, AllocError> {
        Ok(self.begin_defragment(ctx)?.commit())
    }

    /// Sort `free` by offset and merge adjacent regions.
    fn coalesce_free(free: &mut Vec<FreeRegion>) {
        if free.len() < 2 {
            return;
        }
        free.sort_by_key(|r| r.offset);
        let mut merged: Vec<FreeRegion> = Vec::with_capacity(free.len());
        for r in free.drain(..) {
            if let Some(last) = merged.last_mut()
                && last.offset + last.size == r.offset
            {
                last.size += r.size;
                continue;
            }
            merged.push(r);
        }
        *free = merged;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;

    // The pool binds its whole primary buffer as one storage binding, so both
    // device ceilings apply. Before this was centralized the pool knew only
    // `max_buffer_size`, which let a buffer legal by that number alone exceed
    // `max_storage_buffer_binding_size` and fail later inside wgpu, on the
    // first draw that binds the pool as storage.

    #[test]
    fn storage_ceiling_wins_when_it_is_the_tighter_one() {
        assert_eq!(buffer_ceiling_from(256 << 20, 128 << 20), 128 << 20);
    }

    #[test]
    fn buffer_ceiling_wins_when_it_is_the_tighter_one() {
        assert_eq!(buffer_ceiling_from(64 << 20, 128 << 20), 64 << 20);
    }

    #[test]
    fn equal_ceilings_are_that_value() {
        assert_eq!(buffer_ceiling_from(128 << 20, 128 << 20), 128 << 20);
    }

    // ---- growth (begin_relayout with a larger target) ----

    #[test]
    fn growth_preserves_every_byte_and_republishes_capacity() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        let values = vec![1.5_f64, -2.5, 3.0, 0.25];
        pool.add_column("x".into(), &col_f64(values.clone()), ctx)
            .expect("first column fits");
        let before = read_column_values(ctx, &pool, pool.handle_for("x").expect("x present"));

        let grown = pool
            .grow_to(ctx, 8 * ALIGN)
            .expect("growth within the device ceiling");
        assert!(grown, "growth relocates, so it reports a change");
        assert_eq!(pool.capacity(), 8 * ALIGN, "new capacity is published");
        assert_eq!(
            read_column_values(ctx, &pool, pool.handle_for("x").expect("x present")),
            before,
            "growth copies bytes, it does not rewrite them"
        );
    }

    #[test]
    fn growth_invalidates_old_handles_and_requires_refetch() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        let stale = pool
            .add_column("x".into(), &col_f64(vec![1.0, 2.0]), ctx)
            .expect("first column fits");
        let layout_before = pool.layout_generation();

        pool.grow_to(ctx, 8 * ALIGN).expect("growth");

        assert!(
            !pool.is_valid_handle(&stale),
            "a handle taken before growth must not be reused"
        );
        assert!(
            pool.handle_for("x").is_some(),
            "the column is still there under the same id"
        );
        assert!(
            pool.layout_generation() > layout_before,
            "layout generation must advance so live PreparedFrames go stale"
        );
    }

    #[test]
    fn growth_releases_the_old_buffer_instead_of_keeping_it_as_backup() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        // A backup only appears when a defrag actually relocates, so leave a
        // gap first: an already-packed pool returns early without allocating.
        pool.add_column("a".into(), &col_f64(vec![1.0]), ctx)
            .expect("first column fits");
        pool.add_column("b".into(), &col_f64(vec![2.0]), ctx)
            .expect("second column fits");
        pool.remove_column("a")
            .expect("remove leaves a gap at offset 0");
        pool.defragment(ctx)
            .expect("defrag relocates b into the gap");
        assert!(pool.backup.is_some(), "defrag leaves a backup to ping-pong");

        pool.grow_to(ctx, 8 * ALIGN).expect("growth");

        assert!(
            pool.backup.is_none(),
            "a backup sized for the old capacity cannot serve the new one"
        );
    }

    #[test]
    fn growth_beyond_the_budget_is_refused_and_leaves_the_pool_intact() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let unbudgeted = GpuAllocCtx::unbudgeted(&device, &queue);
        pool.add_column("x".into(), &col_f64(vec![1.0, 2.0]), unbudgeted)
            .expect("first column fits");
        let before =
            read_column_values(unbudgeted, &pool, pool.handle_for("x").expect("x present"));
        let capacity_before = pool.capacity();
        let layout_before = pool.layout_generation();

        // Peak is old + new; a budget just under that must refuse.
        let target = 8 * ALIGN;
        let budget = capacity_before + target - 1;
        let budgeted = GpuAllocCtx {
            device: &device,
            queue: &queue,
            budget: Some(GpuBudget {
                ceiling_bytes: budget,
                external_bytes: 0,
            }),
        };
        let error = pool
            .grow_to(budgeted, target)
            .expect_err("growth past the budget must be refused");
        match error {
            AllocError::ResourceLimit {
                requested, limit, ..
            } => {
                assert_eq!(limit, budget);
                assert!(requested > limit);
            }
            other => panic!("expected ResourceLimit, got {other:?}"),
        }
        assert_eq!(pool.capacity(), capacity_before, "capacity unchanged");
        assert_eq!(
            pool.layout_generation(),
            layout_before,
            "a refused growth must not advance the layout"
        );
        assert_eq!(
            read_column_values(unbudgeted, &pool, pool.handle_for("x").expect("x present")),
            before,
            "data survives a refused growth"
        );
    }

    #[test]
    fn external_bytes_count_against_the_same_budget() {
        let Some((device, queue, pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let target = 8 * ALIGN;
        let peak = pool.capacity() + target;
        // Exactly enough for the buffers alone...
        let tight = GpuAllocCtx {
            device: &device,
            queue: &queue,
            budget: Some(GpuBudget {
                ceiling_bytes: peak,
                external_bytes: 0,
            }),
        };
        assert!(
            pool.plan_relayout_capacity(tight, target).is_ok(),
            "peak equal to the budget is allowed"
        );
        // ...but one byte held outside the pool tips it over.
        let with_external = GpuAllocCtx {
            device: &device,
            queue: &queue,
            budget: Some(GpuBudget {
                ceiling_bytes: peak,
                external_bytes: 1,
            }),
        };
        assert!(
            pool.plan_relayout_capacity(with_external, target).is_err(),
            "bytes held outside the pool must count against the budget"
        );
    }

    #[test]
    fn dropping_a_growth_guard_restores_capacity_and_the_old_buffer() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        pool.add_column("x".into(), &col_f64(vec![1.0, 2.0]), ctx)
            .expect("first column fits");
        let handle_before = pool.handle_for("x").expect("x present");
        let before = read_column_values(ctx, &pool, handle_before);
        let capacity_before = pool.capacity();
        let layout_before = pool.layout_generation();

        {
            let guard = pool
                .begin_relayout(ctx, 8 * ALIGN)
                .expect("growth prepares");
            assert_eq!(
                guard.pool().capacity(),
                8 * ALIGN,
                "guard shows the new size"
            );
            // Dropped without commit.
        }

        assert_eq!(
            pool.capacity(),
            capacity_before,
            "an abandoned growth must un-publish the new capacity"
        );
        assert_eq!(pool.layout_generation(), layout_before);
        assert!(
            pool.is_valid_handle(&handle_before),
            "the pre-growth handle is valid again after rollback"
        );
        assert_eq!(
            read_column_values(ctx, &pool, pool.handle_for("x").expect("x present")),
            before,
            "rollback restores the original buffer, bytes intact"
        );
    }

    #[test]
    fn growth_beyond_the_device_ceiling_is_refused_and_leaves_the_pool_intact() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        let handle = pool
            .add_column("keep".into(), &col_f64(vec![1.0, 2.0, 3.0]), ctx)
            .expect("first column fits");
        let capacity = pool.capacity();
        let layout_generation = pool.layout_generation();

        let ceiling = buffer_ceiling(&device);
        let over = ceiling
            .checked_add(ALIGN)
            .expect("ceiling + ALIGN fits u64");
        let Err(error) = pool.grow_to(ctx, over) else {
            panic!("a target above the device ceiling must be refused");
        };
        match error {
            AllocError::ResourceLimit {
                requested, limit, ..
            } => {
                assert_eq!(requested, over);
                assert_eq!(limit, ceiling, "the combined ceiling is the limit");
            }
            other => panic!("expected ResourceLimit, got {other:?}"),
        }

        // Refused before anything was published: same capacity, same layout
        // generation, same handle, same bytes.
        assert_eq!(pool.capacity(), capacity);
        assert_eq!(pool.layout_generation(), layout_generation);
        let refetched = pool.handle_for("keep").expect("column still registered");
        assert_eq!(refetched.offset, handle.offset);
        assert_eq!(refetched.generation, handle.generation);
        assert_eq!(refetched.byte_size, handle.byte_size);
        assert_eq!(refetched.len_values, handle.len_values);
        let stored = read_column_values(ctx, &pool, handle);
        assert_eq!(stored, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn growth_peak_is_the_old_and_new_buffers_and_settles_to_the_new_one() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        pool.add_column("keep".into(), &col_f64(vec![1.0, 2.0]), ctx)
            .expect("first column fits");
        let before = pool.capacity();
        assert_eq!(
            pool.gpu_bytes(),
            before,
            "no backup yet, so the footprint is the slab"
        );
        pool.reset_peak_bytes();

        pool.grow_to(ctx, 8 * ALIGN)
            .expect("growth fits the device");
        let after = pool.capacity();
        assert!(after >= 8 * ALIGN);

        // `commit` released the old slab, so the settled footprint is the new
        // slab alone — but the peak has to remember that both existed while the
        // copy ran, since that is the moment a budget has to survive.
        assert_eq!(
            pool.gpu_bytes(),
            after,
            "the old buffer must be gone once the growth committed"
        );
        assert_eq!(
            pool.peak_bytes(),
            before + after,
            "peak must be old + new: they coexist across the GPU copy"
        );
        assert_eq!(
            pool.retired_bytes(),
            before,
            "the old slab counts as retired until the submission boundary"
        );
        pool.clear_retired_bytes();
        assert_eq!(pool.retired_bytes(), 0);
    }

    #[test]
    fn relayout_refuses_to_shrink() {
        let Some((device, queue, mut pool)) = mk_pool(8 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        let error = pool
            .grow_to(ctx, 2 * ALIGN)
            .expect_err("shrinking must be refused, not silently treated as defrag");
        assert!(matches!(error, AllocError::AllocationFailed { .. }));
        assert_eq!(pool.capacity(), 8 * ALIGN);
    }

    #[test]
    fn same_capacity_relayout_is_the_existing_defragment() {
        let Some((device, queue, mut pool)) = mk_pool(8 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        pool.add_column("a".into(), &col_f64(vec![1.0]), ctx)
            .unwrap();
        pool.add_column("b".into(), &col_f64(vec![2.0]), ctx)
            .unwrap();
        pool.remove_column("a").unwrap();
        let capacity = pool.capacity();

        let relayout = pool.grow_to(ctx, capacity).expect("same-capacity relayout");

        assert!(relayout, "a gap was packed, so it reports a change");
        assert_eq!(pool.capacity(), capacity, "capacity is untouched");
        assert_eq!(pool.handle_for("b").map(|h| h.offset), Some(0));
    }

    #[test]
    fn fixed_growth_policy_keeps_out_of_space_final() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        assert_eq!(pool.growth_policy, GrowthPolicy::Fixed, "default is Fixed");
        let too_big = col_f64(vec![0.0; (4 * ALIGN / COLUMN_VALUE_BYTES as u64) as usize]);

        let error = pool
            .add_column("big".into(), &too_big, ctx)
            .expect_err("a column larger than the pool cannot fit");

        assert!(
            matches!(error, AllocError::OutOfSpace { .. }),
            "with growth off the answer stays OutOfSpace, got {error:?}"
        );
        assert_eq!(pool.capacity(), 2 * ALIGN, "pool did not grow");
    }

    #[test]
    fn upsert_path_growth_makes_an_oversized_upload_fit() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        pool.growth_policy = GrowthPolicy::OnAllocFailure;
        let n = (4 * ALIGN / COLUMN_VALUE_BYTES as u64) as usize;
        let values: Vec<f64> = (0..n).map(|i| i as f64).collect();

        // `upsert_column` is the route every renderer/host upload takes, so
        // growth has to be reachable from here and not only from `add_column`.
        let handle = pool
            .upsert_column("big".into(), &col_f64(values), ctx)
            .expect("the upsert path grows too");

        assert!(pool.capacity() >= 4 * ALIGN, "pool enlarged");
        assert_eq!(handle.len_values, n);
        let stored = read_column_values(ctx, &pool, pool.handle_for("big").expect("big present"));
        assert_eq!(stored.len(), n);
        assert_eq!(stored[n - 1], (n - 1) as f64);
    }

    #[test]
    fn upsert_growth_stays_out_of_the_way_when_packing_is_enough() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        pool.growth_policy = GrowthPolicy::OnAllocFailure;
        let per_column = (ALIGN / COLUMN_VALUE_BYTES as u64) as usize;
        let block: Vec<f64> = (0..per_column).map(|i| i as f64).collect();
        for id in ["a", "b", "c", "d"] {
            pool.add_column(id.into(), &col_f64(block.clone()), ctx)
                .expect("four aligned columns fill the pool exactly");
        }
        let capacity = pool.capacity();

        // Replacing one column in place needs no new bytes; growing here would
        // inflate bytes-per-value for every host that overwrites a column.
        pool.upsert_column("b".into(), &col_f64(block.clone()), ctx)
            .expect("same-size replacement fits without growth");
        assert_eq!(
            pool.capacity(),
            capacity,
            "in-place replacement must not grow"
        );

        // Two freed slots, fragmented: compaction covers a double-size column,
        // so growth must still stay out of it.
        pool.remove_column("a").expect("remove a");
        pool.remove_column("c").expect("remove c");
        pool.defrag_policy = DefragPolicy::OnAllocFailure;
        let double: Vec<f64> = (0..(per_column * 2)).map(|i| i as f64).collect();
        pool.upsert_column("wide".into(), &col_f64(double), ctx)
            .expect("compaction gathers the two freed slots");
        assert_eq!(
            pool.capacity(),
            capacity,
            "compaction was enough; the pool must not have grown"
        );
    }

    #[test]
    fn growth_policy_on_alloc_failure_makes_the_upload_fit() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let ctx = GpuAllocCtx::unbudgeted(&device, &queue);
        pool.growth_policy = GrowthPolicy::OnAllocFailure;
        let n = (4 * ALIGN / COLUMN_VALUE_BYTES as u64) as usize;
        let values: Vec<f64> = (0..n).map(|i| i as f64).collect();

        let handle = pool
            .add_column("big".into(), &col_f64(values.clone()), ctx)
            .expect("growth makes room for a column the pool could not hold");

        assert!(pool.capacity() >= 4 * ALIGN, "pool enlarged");
        assert_eq!(handle.len_values, n);
        let stored = read_column_values(ctx, &pool, pool.handle_for("big").expect("big present"));
        assert_eq!(stored.len(), n);
        assert_eq!(stored[0], 0.0);
        assert_eq!(stored[n - 1], (n - 1) as f64);
    }

    #[test]
    fn pool_refuses_capacity_above_the_device_ceiling() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            eprintln!("no GPU adapter; skipping device ceiling check");
            return;
        };
        let ceiling = buffer_ceiling(&device);
        let over = ceiling
            .checked_add(ALIGN)
            .expect("ceiling + ALIGN fits u64");
        let Err(error) = ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), over) else {
            panic!("capacity above the ceiling must be refused, not created");
        };
        match error {
            AllocError::ResourceLimit {
                requested, limit, ..
            } => {
                assert!(requested > limit, "requested {requested} vs limit {limit}");
                assert_eq!(limit, ceiling, "limit must be the combined ceiling");
            }
            other => panic!("expected ResourceLimit, got {other:?}"),
        }
    }

    struct RawScalarSource<'a> {
        bits: &'a [u32],
    }

    impl ColumnSource for RawScalarSource<'_> {
        fn len(&self) -> usize {
            self.bits.len()
        }

        fn min(&self) -> f64 {
            0.0
        }

        fn max(&self) -> f64 {
            0.0
        }

        fn write_f32_le_into(&self, dst: &mut [u8]) {
            assert_eq!(dst.len(), self.bits.len() * std::mem::size_of::<f32>());
            for (chunk, bits) in dst.chunks_exact_mut(4).zip(self.bits) {
                chunk.copy_from_slice(&bits.to_le_bytes());
            }
        }

        fn write_f32_pair_le_into_with_stats(
            &self,
            mut dst: ColumnPairWriter<'_>,
        ) -> ColumnUploadStats {
            assert_eq!(dst.len(), self.bits.len());
            let mut min_positive: Option<f64> = None;
            for (index, bits) in self.bits.iter().enumerate() {
                let value = f32::from_bits(*bits);
                dst.write_pair(index, value, 0.0);
                if value.is_finite()
                    && value > 0.0
                    && match min_positive {
                        Some(current) => (value as f64) < current,
                        None => true,
                    }
                {
                    min_positive = Some(value as f64);
                }
            }
            ColumnUploadStats { min_positive }
        }
    }

    struct PanickingScalarSource;

    impl ColumnSource for PanickingScalarSource {
        fn len(&self) -> usize {
            1
        }

        fn min(&self) -> f64 {
            1.0
        }

        fn max(&self) -> f64 {
            1.0
        }

        fn write_f32_le_into(&self, _dst: &mut [u8]) {
            panic!("injected scalar writer panic");
        }

        fn write_f32_zero_lo_pair_le_into(&self, dst: &mut [u8]) {
            dst[0] = 0xa5;
            panic!("injected scalar writer panic");
        }

        fn write_f32_pair_le_into_with_stats(
            &self,
            mut dst: ColumnPairWriter<'_>,
        ) -> ColumnUploadStats {
            dst.write_pair(0, 9.0, 0.0);
            panic!("injected scalar writer panic");
        }
    }

    struct PanickingHiLoSource;

    impl HiLoColumnSource for PanickingHiLoSource {
        fn len(&self) -> usize {
            1
        }

        fn min(&self) -> f64 {
            1.0
        }

        fn max(&self) -> f64 {
            1.0
        }

        fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
            dst[0] = 0x5a;
            panic!("injected hi/lo writer panic");
        }

        fn write_f32_pair_le_into_with_stats(
            &self,
            mut dst: ColumnPairWriter<'_>,
        ) -> ColumnUploadStats {
            dst.write_pair(0, 9.0, 0.25);
            panic!("injected hi/lo writer panic");
        }
    }

    fn free_state(free: &[FreeRegion]) -> Vec<(u64, u64)> {
        free.iter()
            .map(|region| (region.offset, region.size))
            .collect()
    }

    #[test]
    fn region_reservation_restores_or_commits_exact_free_list_edit() {
        let original = vec![
            FreeRegion {
                offset: 0,
                size: ALIGN,
            },
            FreeRegion {
                offset: 3 * ALIGN,
                size: 2 * ALIGN,
            },
            FreeRegion {
                offset: 8 * ALIGN,
                size: ALIGN,
            },
        ];

        let mut exact = original.clone();
        {
            let reservation = alloc_region(&mut exact, ALIGN).unwrap();
            assert_eq!(reservation.offset(), 0);
        }
        assert_eq!(free_state(&exact), free_state(&original));

        let reservation = alloc_region(&mut exact, ALIGN).unwrap();
        assert_eq!(reservation.offset(), 0);
        reservation.commit();
        assert_eq!(
            free_state(&exact),
            vec![(3 * ALIGN, 2 * ALIGN), (8 * ALIGN, ALIGN)]
        );

        let mut split = vec![
            FreeRegion {
                offset: ALIGN,
                size: 3 * ALIGN,
            },
            FreeRegion {
                offset: 8 * ALIGN,
                size: ALIGN,
            },
        ];
        let split_original = free_state(&split);
        {
            let reservation = alloc_region(&mut split, ALIGN).unwrap();
            assert_eq!(reservation.offset(), ALIGN);
        }
        assert_eq!(free_state(&split), split_original);

        let reservation = alloc_region(&mut split, ALIGN).unwrap();
        assert_eq!(reservation.offset(), ALIGN);
        reservation.commit();
        assert_eq!(
            free_state(&split),
            vec![(2 * ALIGN, 2 * ALIGN), (8 * ALIGN, ALIGN)]
        );
    }

    #[test]
    fn scalar_source_expands_to_pairs_without_changing_bits() {
        let bits = [
            0x0000_0000,
            0x8000_0000,
            0x3f80_0001,
            0x7fc1_2345,
            0x7f80_0000,
            0xff80_0000,
        ];
        let mut dst = vec![0xa5; bits.len() * COLUMN_VALUE_BYTES];
        let source = RawScalarSource { bits: &bits };

        let stats = write_scalar_source_as_pairs(
            &source,
            ColumnPairWriter::from_bytes_for_test(dst.as_mut_slice()),
        );

        assert_eq!(stats.min_positive, Some(f32::from_bits(0x3f80_0001) as f64));
        for (pair, bits) in dst.chunks_exact(COLUMN_VALUE_BYTES).zip(bits) {
            assert_eq!(&pair[..4], &bits.to_le_bytes());
            assert_eq!(&pair[4..], &0.0f32.to_le_bytes());
        }
    }

    #[test]
    fn scalar_source_expansion_accepts_empty_input() {
        let source = RawScalarSource { bits: &[] };
        let mut dst = [];

        let stats = write_scalar_source_as_pairs(
            &source,
            ColumnPairWriter::from_bytes_for_test(dst.as_mut_slice()),
        );

        assert!(dst.is_empty());
        assert_eq!(stats.min_positive, None);
    }

    fn mk_pool(
        cap: u64,
    ) -> Option<(
        std::sync::Arc<wgpu::Device>,
        std::sync::Arc<wgpu::Queue>,
        ColumnPool,
    )> {
        // Shared device across the test binary — see `data_render::shared_device`.
        let (device, queue) = crate::data_render::shared_device()?;
        let pool = ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), cap).ok()?;
        Some((device, queue, pool))
    }

    fn col_f64(data: Vec<f64>) -> Column<f64> {
        let min = data.iter().copied().fold(f64::INFINITY, f64::min);
        let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column { data, min, max }
    }

    #[test]
    fn independently_created_pools_have_distinct_identities() {
        let Some((device, queue, pool_a)) = mk_pool(ALIGN) else {
            return;
        };
        let pool_b = ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), ALIGN).unwrap();

        let identity_a = pool_a.identity();
        assert_eq!(identity_a, identity_a.clone());
        assert_ne!(identity_a, pool_b.identity());
    }

    #[test]
    fn pool_identity_survives_upsert_drop_commit_and_defragmentation() {
        let Some((device, queue, mut pool)) = mk_pool(3 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0]);
        for id in ["identity-a", "identity-b", "identity-c"] {
            pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
        }
        let identity = pool.identity();

        {
            let replacement = col_f64(vec![2.0]);
            let pending = pool
                .begin_upsert_column(
                    "identity-a".into(),
                    &replacement,
                    GpuAllocCtx::unbudgeted(&device, &queue),
                )
                .unwrap();
            assert_eq!(pending.pool().identity(), identity);
        }
        assert_eq!(pool.identity(), identity);

        let replacement = col_f64(vec![3.0]);
        let pending = pool
            .begin_upsert_column(
                "identity-a".into(),
                &replacement,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(pending.pool().identity(), identity);
        pending.commit();
        assert_eq!(pool.identity(), identity);

        assert!(pool.remove_column("identity-b").unwrap());
        assert!(
            pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap()
        );
        assert_eq!(pool.identity(), identity);
    }

    #[derive(Debug, PartialEq)]
    struct SlotState {
        offset: u64,
        byte_size: u64,
        len_values: usize,
        generation: u32,
        min_bits: u64,
        max_bits: u64,
        min_positive_bits: Option<u64>,
    }

    #[derive(Debug, PartialEq)]
    struct HandleState {
        generation: u32,
        offset: u64,
        byte_size: u64,
        len_values: usize,
    }

    #[derive(Debug, PartialEq)]
    struct NamedSlotState {
        id: ColumnId,
        slot: SlotState,
    }

    #[derive(Debug, PartialEq)]
    struct PoolState {
        buffer: u64,
        backup: Option<u64>,
        generation: u32,
        layout_generation: u64,
        allocation_epoch_counter: u64,
        allocation_epochs: Vec<(ColumnId, u64)>,
        used: u64,
        free_bytes: u64,
        free: Vec<(u64, u64)>,
        slot: Option<SlotState>,
        handle: Option<HandleState>,
    }

    fn buffer_identity(buffer: &Buffer) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(buffer, &mut hasher);
        std::hash::Hasher::finish(&hasher)
    }

    fn pool_state(pool: &ColumnPool, id: &str) -> PoolState {
        let mut allocation_epochs = pool
            .allocation_epochs
            .iter()
            .map(|(id, epoch)| (id.clone(), *epoch))
            .collect::<Vec<_>>();
        allocation_epochs.sort_by(|a, b| a.0.cmp(&b.0));

        PoolState {
            buffer: buffer_identity(pool.buffer()),
            backup: pool.backup.as_ref().map(buffer_identity),
            generation: pool.generation(),
            layout_generation: pool.layout_generation(),
            allocation_epoch_counter: pool.allocation_epoch_counter,
            allocation_epochs,
            used: pool.used_bytes(),
            free_bytes: pool.free_bytes(),
            free: pool
                .free
                .iter()
                .map(|region| (region.offset, region.size))
                .collect(),
            slot: pool.slot(id).map(|slot| SlotState {
                offset: slot.offset,
                byte_size: slot.byte_size,
                len_values: slot.len_values,
                generation: slot.generation,
                min_bits: slot.min.to_bits(),
                max_bits: slot.max.to_bits(),
                min_positive_bits: slot.min_positive.map(f64::to_bits),
            }),
            handle: pool.handle_for(id).map(|handle| HandleState {
                generation: handle.generation,
                offset: handle.offset,
                byte_size: handle.byte_size,
                len_values: handle.len_values,
            }),
        }
    }

    #[derive(Debug, PartialEq)]
    struct FullPoolState {
        identity: PoolIdentity,
        base: PoolState,
        slots: Vec<NamedSlotState>,
    }

    fn full_pool_state(pool: &ColumnPool) -> FullPoolState {
        let mut slots = pool
            .slots
            .values()
            .map(|slot| NamedSlotState {
                id: slot.id.clone(),
                slot: SlotState {
                    offset: slot.offset,
                    byte_size: slot.byte_size,
                    len_values: slot.len_values,
                    generation: slot.generation,
                    min_bits: slot.min.to_bits(),
                    max_bits: slot.max.to_bits(),
                    min_positive_bits: slot.min_positive.map(f64::to_bits),
                },
            })
            .collect::<Vec<_>>();
        slots.sort_by(|left, right| left.id.cmp(&right.id));
        FullPoolState {
            identity: pool.identity(),
            base: pool_state(pool, "demo_x"),
            slots,
        }
    }

    fn demo_batch_columns(sources: [&dyn ColumnSource; 4]) -> [(ColumnId, &dyn ColumnSource); 4] {
        let ids = ["demo_x", "demo_sin", "demo_t", "demo_rc"];
        std::array::from_fn(|index| (ids[index].to_string(), sources[index]))
    }

    #[test]
    fn demo_batch_capacity_failures_at_each_replacement_preserve_live_pool() {
        let Some((device, queue, _)) = mk_pool(4 * ALIGN) else {
            return;
        };
        for failed_index in 0..4 {
            let mut pool =
                ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), 4 * ALIGN).unwrap();
            let before = full_pool_state(&pool);
            let mut columns = Vec::new();
            for index in 0..4 {
                let aligned_regions = if index == failed_index {
                    5usize - failed_index
                } else {
                    1
                };
                columns.push(col_f64(vec![1.0; aligned_regions * ALIGN as usize / 8]));
            }
            let sources: [&dyn ColumnSource; 4] =
                [&columns[0], &columns[1], &columns[2], &columns[3]];
            assert!(matches!(
                pool.begin_demo_batch_upsert(
                    demo_batch_columns(sources),
                    GpuAllocCtx::unbudgeted(&device, &queue),
                ),
                Err(AllocError::OutOfSpace { .. })
            ));
            assert_eq!(full_pool_state(&pool), before, "replacement {failed_index}");
        }
    }

    #[test]
    fn demo_batch_epoch_failures_at_each_replacement_preserve_live_pool() {
        let Some((device, queue, _)) = mk_pool(8 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0, 2.0]);
        for failed_index in 0..4 {
            let mut pool =
                ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), 8 * ALIGN).unwrap();
            pool.add_column(
                "survivor".into(),
                &values,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
            pool.allocation_epoch_counter = u64::MAX - failed_index as u64;
            let before = full_pool_state(&pool);
            let sources: [&dyn ColumnSource; 4] = [&values, &values, &values, &values];
            let error = match pool.begin_demo_batch_upsert(
                demo_batch_columns(sources),
                GpuAllocCtx::unbudgeted(&device, &queue),
            ) {
                Ok(_) => panic!("replacement {failed_index} epoch must fail"),
                Err(error) => error,
            };
            assert_eq!(
                error,
                AllocError::CounterExhausted {
                    counter: "allocation epoch",
                }
            );
            assert_eq!(full_pool_state(&pool), before, "replacement {failed_index}");
        }
    }

    #[test]
    fn demo_batch_generation_and_layout_overflow_preserve_live_pool() {
        let Some((device, queue, _)) = mk_pool(8 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0, 2.0]);
        for counter in ["public generation", "layout generation"] {
            let mut pool =
                ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), 8 * ALIGN).unwrap();
            pool.add_column(
                "survivor".into(),
                &values,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
            if counter == "public generation" {
                pool.generation = u32::MAX;
            } else {
                pool.layout_generation = u64::MAX;
            }
            let before = full_pool_state(&pool);
            let sources: [&dyn ColumnSource; 4] = [&values, &values, &values, &values];
            let error = match pool.begin_demo_batch_upsert(
                demo_batch_columns(sources),
                GpuAllocCtx::unbudgeted(&device, &queue),
            ) {
                Ok(_) => panic!("{counter} must reject overflow"),
                Err(error) => error,
            };
            assert_eq!(error, AllocError::CounterExhausted { counter });
            assert_eq!(full_pool_state(&pool), before);
        }
    }

    #[test]
    fn demo_batch_drop_preserves_fragmented_pool_with_existing_backup() {
        let Some((device, queue, mut pool)) = mk_pool(10 * ALIGN) else {
            return;
        };
        let old = col_f64(vec![1.0, 2.0]);
        for id in ["survivor-a", "demo_x", "hole", "demo_rc", "survivor-b"] {
            pool.add_column(id.into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
        }
        assert!(pool.remove_column("hole").unwrap());
        assert!(
            pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap()
        );
        assert!(pool.backup.is_some());
        assert!(pool.remove_column("demo_x").unwrap());
        let before = full_pool_state(&pool);
        let survivor_a = read_column_values(
            GpuAllocCtx::unbudgeted(&device, &queue),
            &pool,
            pool.handle_for("survivor-a").unwrap(),
        );
        let replacements = [
            col_f64(vec![10.0, 11.0]),
            col_f64(vec![20.0, 21.0]),
            col_f64(vec![30.0, 31.0]),
            col_f64(vec![40.0, 41.0]),
        ];
        {
            let sources: [&dyn ColumnSource; 4] = [
                &replacements[0],
                &replacements[1],
                &replacements[2],
                &replacements[3],
            ];
            let pending = pool
                .begin_demo_batch_upsert(
                    demo_batch_columns(sources),
                    GpuAllocCtx::unbudgeted(&device, &queue),
                )
                .unwrap();
            assert_ne!(buffer_identity(pending.pool().buffer()), before.base.buffer);
        }
        assert_eq!(full_pool_state(&pool), before);
        assert_eq!(
            read_column_values(
                GpuAllocCtx::unbudgeted(&device, &queue),
                &pool,
                pool.handle_for("survivor-a").unwrap(),
            ),
            survivor_a
        );
    }

    #[test]
    fn demo_batch_commit_publishes_complete_packed_pool_and_preserves_survivor_epochs() {
        let Some((device, queue, mut pool)) = mk_pool(10 * ALIGN) else {
            return;
        };
        let old = col_f64(vec![1.0, 2.0]);
        for id in ["survivor", "demo_x", "hole", "demo_sin"] {
            pool.add_column(id.into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
        }
        assert!(pool.remove_column("hole").unwrap());
        assert!(
            pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap()
        );
        let before = full_pool_state(&pool);
        let primary = buffer_identity(pool.buffer());
        let old_backup = pool.backup.as_ref().map(buffer_identity);
        let generation = pool.generation();
        let layout_generation = pool.layout_generation();
        let survivor_epoch = pool.allocation_epoch("survivor").unwrap();
        let old_demo_x_epoch = pool.allocation_epoch("demo_x").unwrap();
        let replacements = [
            col_f64(vec![10.0, 11.0]),
            col_f64(vec![20.0, 21.0]),
            col_f64(vec![30.0, 31.0]),
            col_f64(vec![40.0, 41.0]),
        ];
        let sources: [&dyn ColumnSource; 4] = [
            &replacements[0],
            &replacements[1],
            &replacements[2],
            &replacements[3],
        ];
        let handles = pool
            .begin_demo_batch_upsert(
                demo_batch_columns(sources),
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap()
            .commit();

        let committed = full_pool_state(&pool);
        assert_eq!(committed.identity, before.identity);
        assert_eq!(pool.generation(), generation + 1);
        assert_eq!(pool.layout_generation(), layout_generation + 1);
        assert_eq!(pool.backup.as_ref().map(buffer_identity), Some(primary));
        assert_ne!(pool.backup.as_ref().map(buffer_identity), old_backup);
        assert_eq!(pool.allocation_epoch("survivor"), Some(survivor_epoch));
        assert_ne!(pool.allocation_epoch("demo_x"), Some(old_demo_x_epoch));
        let replacement_epochs = ["demo_x", "demo_sin", "demo_t", "demo_rc"]
            .map(|id| pool.allocation_epoch(id).unwrap());
        assert!(replacement_epochs.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(pool.free.len(), 1);
        assert_eq!(pool.free[0].offset, pool.used_bytes());
        for (index, id) in ["demo_x", "demo_sin", "demo_t", "demo_rc"]
            .iter()
            .enumerate()
        {
            assert_eq!(pool.handle_for(id).unwrap().offset, handles[index].offset);
            assert_eq!(
                read_column_values(
                    GpuAllocCtx::unbudgeted(&device, &queue),
                    &pool,
                    handles[index]
                ),
                replacements[index].data
            );
        }
        assert_eq!(
            read_column_values(
                GpuAllocCtx::unbudgeted(&device, &queue),
                &pool,
                pool.handle_for("survivor").unwrap(),
            ),
            old.data
        );
    }

    #[test]
    fn column_removal_guard_drop_restores_exact_pool_state() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0]);
        pool.add_column(
            "remove-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "remove-b".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let identity = pool.identity();
        let before_a = pool_state(&pool, "remove-a");
        let before_b = pool_state(&pool, "remove-b");

        {
            let removal = pool
                .begin_remove_column("remove-a")
                .unwrap()
                .expect("existing column must produce a guard");
            assert_eq!(removal.pool().identity(), identity);
            assert!(removal.pool().handle_for("remove-a").is_none());
            assert!(removal.pool().handle_for("remove-b").is_some());
        }

        assert_eq!(pool.identity(), identity);
        assert_eq!(pool_state(&pool, "remove-a"), before_a);
        assert_eq!(pool_state(&pool, "remove-b"), before_b);
    }

    #[test]
    fn column_removal_guard_commit_publishes_without_changing_identity() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0]);
        pool.add_column(
            "remove-commit".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let identity = pool.identity();

        let removal = pool
            .begin_remove_column("remove-commit")
            .unwrap()
            .expect("existing column must produce a guard");
        assert!(removal.commit());

        assert_eq!(pool.identity(), identity);
        assert!(pool.handle_for("remove-commit").is_none());
        assert_eq!(pool.free_bytes(), pool.capacity());
        assert!(pool.begin_remove_column("missing").unwrap().is_none());
    }

    #[test]
    fn defragment_guard_fresh_candidate_drop_restores_primary_and_no_backup() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0]);
        for id in ["fresh-a", "fresh-b", "fresh-c"] {
            pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
        }
        pool.remove_column("fresh-b").unwrap();
        let identity = pool.identity();
        let before_a = pool_state(&pool, "fresh-a");
        let before_c = pool_state(&pool, "fresh-c");
        assert!(pool.backup.is_none());

        {
            let defrag = pool
                .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
            assert!(defrag.changed());
            assert!(defrag.relocated());
            assert_eq!(defrag.pool().identity(), identity);
            assert_eq!(defrag.pool().slots["fresh-c"].offset, ALIGN);
            assert!(defrag.pool().backup.is_some());
        }

        assert_eq!(pool.identity(), identity);
        assert!(pool.backup.is_none());
        assert_eq!(pool_state(&pool, "fresh-a"), before_a);
        assert_eq!(pool_state(&pool, "fresh-c"), before_c);
    }

    #[test]
    fn defragment_guard_existing_backup_drop_restores_both_buffers() {
        let Some((device, queue, mut pool)) = mk_pool(5 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0]);
        for id in ["reuse-a", "reuse-b", "reuse-c", "reuse-d"] {
            pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
        }
        pool.remove_column("reuse-b").unwrap();
        assert!(
            pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap()
        );
        assert!(pool.backup.is_some());

        pool.remove_column("reuse-c").unwrap();
        let before_a = pool_state(&pool, "reuse-a");
        let before_d = pool_state(&pool, "reuse-d");
        let primary_before = buffer_identity(pool.buffer());
        let backup_before = pool.backup.as_ref().map(buffer_identity);

        {
            let defrag = pool
                .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
            assert!(defrag.changed());
            assert!(defrag.relocated());
            assert_eq!(defrag.pool().slots["reuse-d"].offset, ALIGN);
            assert_eq!(
                buffer_identity(defrag.pool().buffer()),
                backup_before.unwrap()
            );
        }

        assert_eq!(buffer_identity(pool.buffer()), primary_before);
        assert_eq!(pool.backup.as_ref().map(buffer_identity), backup_before);
        assert_eq!(pool_state(&pool, "reuse-a"), before_a);
        assert_eq!(pool_state(&pool, "reuse-d"), before_d);
    }

    #[test]
    fn defragment_guard_distinguishes_noop_normalization_and_relocation() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let identity = pool.identity();

        let no_op = pool
            .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert!(!no_op.changed());
        assert!(!no_op.relocated());
        assert!(!no_op.commit());

        pool.free = vec![
            FreeRegion {
                offset: 0,
                size: ALIGN,
            },
            FreeRegion {
                offset: ALIGN,
                size: pool.capacity() - ALIGN,
            },
        ];
        let empty_before = pool_state(&pool, "missing");
        {
            let normalized = pool
                .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
            assert!(normalized.changed());
            assert!(!normalized.relocated());
            assert_eq!(normalized.pool().identity(), identity);
            assert_eq!(normalized.pool().free.len(), 1);
        }
        assert_eq!(pool_state(&pool, "missing"), empty_before);

        let normalized = pool
            .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert!(normalized.changed());
        assert!(!normalized.relocated());
        assert!(normalized.commit());
        assert_eq!(pool.identity(), identity);
        assert_eq!(pool.free.len(), 1);

        let values = col_f64(vec![1.0]);
        pool.add_column(
            "packed".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.free = vec![
            FreeRegion {
                offset: ALIGN,
                size: ALIGN,
            },
            FreeRegion {
                offset: 2 * ALIGN,
                size: 2 * ALIGN,
            },
        ];
        let generation = pool.generation();
        let layout_generation = pool.layout_generation();
        let normalized = pool
            .begin_defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert!(normalized.changed());
        assert!(!normalized.relocated());
        assert!(!normalized.commit());
        assert_eq!(pool.generation(), generation);
        assert_eq!(pool.layout_generation(), layout_generation);
        assert_eq!(pool.free.len(), 1);
    }

    #[test]
    fn legacy_staging_failure_restores_allocator_and_allows_retry() {
        let Some((device, queue, mut pool)) = mk_pool(ALIGN) else {
            return;
        };
        let column = col_f64(vec![1.0, 2.0, 3.0]);
        let before = pool_state(&pool, "x");

        let error = pool
            .try_add_column_pairs_with(
                ColumnInputMeta {
                    id: "x".into(),
                    len_values: column.data.len(),
                    min: column.min,
                    max: column.max,
                },
                GpuAllocCtx::unbudgeted(&device, &queue),
                |dst| write_scalar_source_as_pairs(&column, dst),
                |_| {
                    Err(AllocError::AllocationFailed {
                        resource: "column staging buffer",
                        reason: "injected legacy staging failure".into(),
                    })
                },
            )
            .unwrap_err();

        assert!(matches!(error, AllocError::AllocationFailed { .. }));
        assert_eq!(pool_state(&pool, "x"), before);

        let handle = pool
            .add_column(
                "x".into(),
                &column,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(handle.offset, 0);
        assert_eq!(pool.slot("x").unwrap().len_values, column.data.len());
    }

    #[test]
    fn legacy_writer_panics_restore_scalar_and_hilo_allocator_state() {
        let Some((device, queue, mut scalar_pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let scalar_before = pool_state(&scalar_pool, "scalar");
        let scalar_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = scalar_pool.add_column(
                "scalar".into(),
                &PanickingScalarSource,
                GpuAllocCtx::unbudgeted(&device, &queue),
            );
        }));
        assert!(scalar_result.is_err());
        assert_eq!(pool_state(&scalar_pool, "scalar"), scalar_before);

        let retry = col_f64(vec![2.0]);
        let scalar_handle = scalar_pool
            .add_column(
                "scalar".into(),
                &retry,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(scalar_handle.offset, 0);

        let mut hilo_pool =
            ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), 2 * ALIGN).unwrap();
        let hilo_before = pool_state(&hilo_pool, "hilo");
        let hilo_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = hilo_pool.add_hilo_column(
                "hilo".into(),
                &PanickingHiLoSource,
                GpuAllocCtx::unbudgeted(&device, &queue),
            );
        }));
        assert!(hilo_result.is_err());
        assert_eq!(pool_state(&hilo_pool, "hilo"), hilo_before);

        let hilo_handle = hilo_pool
            .add_hilo_column(
                "hilo".into(),
                &retry,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(hilo_handle.offset, 0);
    }

    // ── Batch upload (W6) ───────────────────────────────────────────────────

    /// The whole batch lands in one contiguous region, in declaration order,
    /// with each column's own bytes and statistics — and it costs **one** GPU
    /// buffer, not one per column.
    #[test]
    fn a_column_batch_uploads_into_one_contiguous_region() {
        let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
            return;
        };
        let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
        let a = col_f64(vec![1.0, 2.0, 3.0]);
        let b = col_f64(vec![10.0, 20.0]);
        let c = col_f64(vec![-5.0, 0.0, 5.0, 7.5]);

        let before_creations = pool.buffer_creations();
        pool.add_columns(
            &[
                ("a", &a as &dyn ColumnSource),
                ("b", &b as &dyn ColumnSource),
                ("c", &c as &dyn ColumnSource),
            ],
            ctx(),
        )
        .expect("batch upload");

        // One staging buffer for the batch, not one per column.
        assert_eq!(
            pool.buffer_creations() - before_creations,
            1,
            "a batch must create one staging buffer"
        );

        // Contiguous, in order, ALIGN-aligned.
        let mut expected_offset = 0u64;
        for (id, len) in [("a", 3usize), ("b", 2), ("c", 4)] {
            let slot = pool.slot(id).expect("batch column is registered");
            assert_eq!(slot.offset, expected_offset, "{id} offset");
            assert_eq!(slot.len_values, len, "{id} length");
            assert_eq!(slot.byte_size % ALIGN, 0, "{id} padding");
            expected_offset += slot.byte_size;
        }
        assert_eq!(pool.used_bytes(), expected_offset);

        // Values and statistics survive the shared staging buffer.
        let handle = |id: &str| pool.handle_for(id).expect("handle");
        assert_eq!(read_column_values(ctx(), &pool, handle("a")), a.data);
        assert_eq!(read_column_values(ctx(), &pool, handle("b")), b.data);
        assert_eq!(read_column_values(ctx(), &pool, handle("c")), c.data);
        assert_eq!(pool.slot("c").unwrap().min, -5.0);
        assert_eq!(pool.slot("c").unwrap().max, 7.5);
        assert_eq!(
            pool.slot("c").unwrap().min_positive,
            Some(5.0),
            "min_positive comes from the write, not the source"
        );
    }

    /// Adding N columns one at a time costs N staging buffers and N submits;
    /// the batch costs one of each. This is the measurement the batch exists
    /// for, so it is asserted rather than described.
    #[test]
    fn the_batch_costs_one_buffer_where_singles_cost_one_each() {
        let Some((device, queue, mut pool)) = mk_pool(256 * ALIGN) else {
            return;
        };
        let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
        let sources: Vec<Column<f64>> = (0..8).map(|i| col_f64(vec![i as f64, 1.0])).collect();

        let singles_before = pool.buffer_creations();
        for (index, source) in sources.iter().enumerate() {
            pool.add_column(format!("single{index}"), source, ctx())
                .expect("single add");
        }
        let singles = pool.buffer_creations() - singles_before;

        let batch: Vec<(&str, &dyn ColumnSource)> = vec![
            ("b0", &sources[0] as &dyn ColumnSource),
            ("b1", &sources[1] as &dyn ColumnSource),
            ("b2", &sources[2] as &dyn ColumnSource),
            ("b3", &sources[3] as &dyn ColumnSource),
            ("b4", &sources[4] as &dyn ColumnSource),
            ("b5", &sources[5] as &dyn ColumnSource),
            ("b6", &sources[6] as &dyn ColumnSource),
            ("b7", &sources[7] as &dyn ColumnSource),
        ];
        let batch_before = pool.buffer_creations();
        pool.add_columns(&batch, ctx()).expect("batch add");
        let batched = pool.buffer_creations() - batch_before;

        assert_eq!(singles, 8, "one staging buffer per single add");
        assert_eq!(batched, 1, "one staging buffer for the whole batch");
    }

    /// A rejected batch changes nothing: no slot, no epoch, and the free list is
    /// byte-identical — the single reservation rolled back.
    #[test]
    fn a_rejected_batch_leaves_the_pool_untouched() {
        let Some((device, queue, mut pool)) = mk_pool(8 * ALIGN) else {
            return;
        };
        let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
        let a = col_f64(vec![1.0, 2.0]);
        let empty: Column<f64> = Column {
            data: Vec::new(),
            min: 0.0,
            max: 0.0,
        };
        pool.add_column("live".into(), &a, ctx()).expect("seed");
        let free_before = pool.free.clone();
        let slots_before = pool.slots.len();
        let epoch_before = pool.allocation_epoch_counter;

        // An id already in the pool.
        assert!(matches!(
            pool.add_columns(
                &[
                    ("fresh", &a as &dyn ColumnSource),
                    ("live", &a as &dyn ColumnSource),
                ],
                ctx()
            ),
            Err(AllocError::DuplicateId(_))
        ));
        // A duplicate inside the batch.
        assert!(matches!(
            pool.add_columns(
                &[
                    ("twice", &a as &dyn ColumnSource),
                    ("twice", &a as &dyn ColumnSource),
                ],
                ctx()
            ),
            Err(AllocError::DuplicateId(_))
        ));
        // An empty source.
        assert!(matches!(
            pool.add_columns(
                &[
                    ("fresh", &a as &dyn ColumnSource),
                    ("nothing", &empty as &dyn ColumnSource),
                ],
                ctx()
            ),
            Err(AllocError::EmptySource)
        ));
        // More than the pool can hold.
        let big: Vec<Column<f64>> = (0..16).map(|_| col_f64(vec![1.0; 64])).collect();
        let too_much: Vec<(&str, &dyn ColumnSource)> = big
            .iter()
            .enumerate()
            .map(|(i, source)| {
                (
                    [
                        "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8", "t9", "ta", "tb",
                        "tc", "td", "te", "tf",
                    ][i],
                    source as &dyn ColumnSource,
                )
            })
            .collect();
        assert!(matches!(
            pool.add_columns(&too_much, ctx()),
            Err(AllocError::OutOfSpace { .. })
        ));

        assert_eq!(pool.slots.len(), slots_before, "no slot was published");
        assert!(pool.slot("fresh").is_none());
        assert_eq!(
            pool.free, free_before,
            "the reservation rolled back exactly"
        );
        assert_eq!(
            pool.allocation_epoch_counter, epoch_before,
            "no epoch was consumed"
        );
        // And the surviving column still reads back.
        let handle = pool.handle_for("live").expect("handle");
        assert_eq!(read_column_values(ctx(), &pool, handle), a.data);
    }

    /// An empty batch is a no-op, not an error.
    #[test]
    fn an_empty_batch_does_nothing() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let before = (pool.slots.len(), pool.buffer_creations());
        pool.add_columns(&[], GpuAllocCtx::unbudgeted(&device, &queue))
            .expect("empty batch");
        assert_eq!((pool.slots.len(), pool.buffer_creations()), before);
    }

    /// Chunking is only about the device's staging ceiling: the plan splits at
    /// that boundary, every chunk mirrors the region layout, and the offsets
    /// stay contiguous across chunks. Tested at the planner because a real
    /// device's ceiling is far too large to reach from a unit test.
    #[test]
    fn the_staging_plan_splits_only_at_the_device_ceiling() {
        let one = col_f64(vec![1.0; 32]); // 256 B raw = exactly one ALIGN
        let sources: Vec<Column<f64>> = (0..5).map(|_| one.clone()).collect();
        let columns: Vec<(&str, &dyn ColumnSource)> = vec![
            ("c0", &sources[0] as &dyn ColumnSource),
            ("c1", &sources[1] as &dyn ColumnSource),
            ("c2", &sources[2] as &dyn ColumnSource),
            ("c3", &sources[3] as &dyn ColumnSource),
            ("c4", &sources[4] as &dyn ColumnSource),
        ];
        assert_eq!(batch_region_bytes(&columns, u64::MAX).unwrap(), 5 * ALIGN);

        // Ceiling above the batch: one chunk, one copy.
        let whole = plan_batch_chunks(&columns, 64 * ALIGN).unwrap();
        assert_eq!(whole.len(), 1);
        assert_eq!(whole[0].byte_size, 5 * ALIGN);
        assert_eq!(whole[0].region_offset, 0);
        assert_eq!(whole[0].column_offsets, vec![0, 256, 512, 768, 1024]);

        // Ceiling of two columns: three chunks, contiguous in the region.
        let split = plan_batch_chunks(&columns, 2 * ALIGN).unwrap();
        assert_eq!(split.len(), 3);
        assert_eq!(split[0].columns, 0..2);
        assert_eq!(split[1].columns, 2..4);
        assert_eq!(split[2].columns, 4..5);
        let mut expected_offset = 0;
        for chunk in &split {
            assert_eq!(chunk.region_offset, expected_offset);
            assert!(chunk.byte_size <= 2 * ALIGN);
            assert_eq!(chunk.column_offsets[0], 0, "each chunk starts at its own 0");
            expected_offset += chunk.byte_size;
        }
        assert_eq!(expected_offset, 5 * ALIGN, "chunks tile the region exactly");
    }

    /// A single column larger than the device can stage is refused whether it
    /// arrives alone or in a batch — the batch does not make it possible.
    #[test]
    fn a_column_over_the_device_ceiling_is_refused_in_a_batch_too() {
        let huge = col_f64(vec![1.0; 64]);
        let columns: Vec<(&str, &dyn ColumnSource)> = vec![("huge", &huge as &dyn ColumnSource)];
        assert!(matches!(
            batch_region_bytes(&columns, ALIGN),
            Err(AllocError::ResourceLimit { .. })
        ));
    }

    /// The guard's whole reason to exist: a caller with its own fallible work
    /// can look at the provisional pool and then abandon it, and the pool is
    /// byte-for-byte what it was — without the batch ever cloning a registry.
    #[test]
    fn an_abandoned_batch_guard_un_inserts_every_column() {
        let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
            return;
        };
        let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
        let live = col_f64(vec![1.0, 2.0]);
        let a = col_f64(vec![3.0, 4.0, 5.0]);
        let b = col_f64(vec![6.0]);
        pool.add_column("live".into(), &live, ctx()).expect("seed");

        let free_before = pool.free.clone();
        let used_before = pool.used_bytes();
        let epoch_before = pool.allocation_epoch_counter;
        let generation_before = pool.generation();
        let layout_before = pool.layout_generation();

        let guard = pool
            .begin_add_columns(
                &[
                    ("a", &a as &dyn ColumnSource),
                    ("b", &b as &dyn ColumnSource),
                ],
                ctx(),
            )
            .expect("batch begins");
        // Provisionally visible through the guard's own pool view.
        assert!(guard.pool().slot("a").is_some(), "provisional slot a");
        assert!(guard.pool().slot("b").is_some(), "provisional slot b");
        assert!(guard.pool().used_bytes() > used_before);
        drop(guard);

        assert!(pool.slot("a").is_none(), "abandoned batch left slot a");
        assert!(pool.slot("b").is_none(), "abandoned batch left slot b");
        assert_eq!(pool.slots.len(), 1, "only the seeded column survives");
        assert_eq!(pool.allocation_epochs.len(), 1, "epochs rolled back too");
        assert_eq!(pool.free, free_before, "the region went back exactly");
        assert_eq!(pool.used_bytes(), used_before);
        assert_eq!(pool.allocation_epoch_counter, epoch_before);
        assert_eq!(pool.generation(), generation_before, "no invalidation");
        assert_eq!(pool.layout_generation(), layout_before);

        // The seeded column is untouched, and the freed region is reusable.
        let handle = pool.handle_for("live").expect("handle");
        assert_eq!(read_column_values(ctx(), &pool, handle), live.data);
        pool.add_columns(&[("a", &a as &dyn ColumnSource)], ctx())
            .expect("the region is allocatable again");
        assert_eq!(
            pool.slot("a").expect("re-added").offset,
            pool.slot("live").expect("seed").byte_size,
            "the retry takes the same offset the abandoned batch had"
        );
    }

    /// Committing publishes exactly once: the guard is the only gate, and a
    /// committed batch behaves like `add_columns` did.
    #[test]
    fn a_committed_batch_guard_publishes_every_column() {
        let Some((device, queue, mut pool)) = mk_pool(64 * ALIGN) else {
            return;
        };
        let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
        let a = col_f64(vec![1.5, 2.5]);
        let b = col_f64(vec![-1.0, 0.0, 1.0]);
        pool.begin_add_columns(
            &[
                ("a", &a as &dyn ColumnSource),
                ("b", &b as &dyn ColumnSource),
            ],
            ctx(),
        )
        .expect("batch begins")
        .commit();

        let handle = |id: &str| pool.handle_for(id).expect("handle");
        assert_eq!(read_column_values(ctx(), &pool, handle("a")), a.data);
        assert_eq!(read_column_values(ctx(), &pool, handle("b")), b.data);
        assert_eq!(
            pool.allocation_epochs.len(),
            2,
            "one epoch per published column"
        );
    }

    /// Fragmentation alone must not fail a batch under `OnAllocFailure`: the
    /// batch needs one *contiguous* region, and compaction is what produces it.
    /// The pre-decision replaces the old attempt-then-retry rung, so the
    /// observable outcome is what is asserted.
    #[test]
    fn a_batch_compacts_when_fragmentation_is_the_only_obstacle() {
        let Some((device, queue, mut pool)) = mk_pool(6 * ALIGN) else {
            return;
        };
        let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
        pool.defrag_policy = DefragPolicy::OnAllocFailure;
        let one = col_f64(vec![1.0; 32]); // exactly one ALIGN
        for id in ["a", "b", "c", "d", "e", "f"] {
            pool.add_column(id.to_string(), &one, ctx())
                .expect("fill the pool");
        }
        // Free two ALIGNs that are not adjacent.
        pool.remove_column("b").expect("remove b");
        pool.remove_column("e").expect("remove e");
        assert_eq!(pool.free_bytes(), 2 * ALIGN);
        assert_eq!(pool.largest_free_region(), ALIGN, "fragmented");

        let x = col_f64(vec![7.0; 32]);
        let y = col_f64(vec![8.0; 32]);
        pool.add_columns(
            &[
                ("x", &x as &dyn ColumnSource),
                ("y", &y as &dyn ColumnSource),
            ],
            ctx(),
        )
        .expect("compaction made one region out of two");
        assert_eq!(pool.used_bytes(), 6 * ALIGN);
        assert_eq!(pool.free_bytes(), 0);
        let handle = |id: &str| pool.handle_for(id).expect("handle");
        assert_eq!(read_column_values(ctx(), &pool, handle("x")), x.data);
        assert_eq!(read_column_values(ctx(), &pool, handle("y")), y.data);
        // Compaction relocated the survivors, so their bytes must still read.
        assert_eq!(read_column_values(ctx(), &pool, handle("a")), one.data);
    }

    /// With growth off, the same fragmented pool that cannot fit the batch says
    /// so — and stays exactly as it was.
    #[test]
    fn a_batch_too_big_for_a_fixed_pool_is_refused_without_compacting() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
        pool.defrag_policy = DefragPolicy::OnAllocFailure;
        let one = col_f64(vec![1.0; 32]);
        for id in ["a", "b", "c"] {
            pool.add_column(id.to_string(), &one, ctx()).expect("fill");
        }
        let layout_before = pool.layout_generation();
        let big = col_f64(vec![1.0; 64]); // two ALIGNs, only one is free
        assert!(matches!(
            pool.add_columns(&[("big", &big as &dyn ColumnSource)], ctx()),
            Err(AllocError::OutOfSpace { .. })
        ));
        assert_eq!(
            pool.layout_generation(),
            layout_before,
            "a batch that cannot fit must not pay for a pointless compaction"
        );
        assert!(pool.slot("big").is_none());
    }

    /// A batch bigger than the pool grows it, once, under `OnAllocFailure`.
    #[test]
    fn a_batch_grows_the_pool_when_compaction_is_not_enough() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let ctx = || GpuAllocCtx::unbudgeted(&device, &queue);
        pool.growth_policy = GrowthPolicy::OnAllocFailure;
        let one = col_f64(vec![1.0; 32]);
        pool.add_column("seed".into(), &one, ctx()).expect("seed");
        let capacity_before = pool.capacity();

        let columns: Vec<Column<f64>> = (0..4).map(|_| one.clone()).collect();
        let batch: Vec<(&str, &dyn ColumnSource)> = vec![
            ("g0", &columns[0] as &dyn ColumnSource),
            ("g1", &columns[1] as &dyn ColumnSource),
            ("g2", &columns[2] as &dyn ColumnSource),
            ("g3", &columns[3] as &dyn ColumnSource),
        ];
        pool.add_columns(&batch, ctx()).expect("growth made it fit");
        assert!(
            pool.capacity() >= capacity_before + 3 * ALIGN,
            "capacity {} did not grow enough from {capacity_before}",
            pool.capacity()
        );
        assert_eq!(pool.used_bytes(), 5 * ALIGN);
        let handle = |id: &str| pool.handle_for(id).expect("handle");
        assert_eq!(read_column_values(ctx(), &pool, handle("seed")), one.data);
        assert_eq!(read_column_values(ctx(), &pool, handle("g3")), one.data);
    }

    fn read_column_values(
        ctx: GpuAllocCtx<'_>,
        pool: &ColumnPool,
        handle: ColumnHandle,
    ) -> Vec<f64> {
        let (device, queue) = (ctx.device, ctx.queue);
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("figgy column pool test readback"),
            size: handle.byte_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy column pool test readback"),
        });
        encoder.copy_buffer_to_buffer(pool.buffer(), handle.offset, &readback, 0, handle.byte_size);
        let (sender, receiver) = std::sync::mpsc::channel();
        encoder.map_buffer_on_submit(
            &readback,
            wgpu::MapMode::Read,
            0..handle.byte_size,
            move |result| {
                let _ = sender.send(result);
            },
        );
        let submission = queue.submit(std::iter::once(encoder.finish()));
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("readback poll");
        receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("readback callback")
            .expect("map readback");
        let mapped = readback
            .slice(..handle.byte_size)
            .get_mapped_range()
            .expect("column readback is mapped after map_async");
        let values = mapped[..handle.len_values * COLUMN_VALUE_BYTES]
            .chunks_exact(COLUMN_VALUE_BYTES)
            .map(|bytes| {
                let hi = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64;
                let lo = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as f64;
                hi + lo
            })
            .collect();
        drop(mapped);
        readback.unmap();
        values
    }

    #[test]
    fn upsert_prepare_failures_leave_old_state_and_retry() {
        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        let old = col_f64(vec![1.0, 2.0, 3.0]);
        pool.add_column("x".into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let blocker = col_f64(vec![10.0, 11.0, 12.0]);
        pool.add_column(
            "blocker".into(),
            &blocker,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let before = pool_state(&pool, "x");

        let replacement = col_f64(vec![4.0, 5.0, 6.0]);
        let staging_error = match pool.begin_upsert_column_pairs_with(
            ColumnInputMeta {
                id: "x".into(),
                len_values: replacement.data.len(),
                min: replacement.min,
                max: replacement.max,
            },
            GpuAllocCtx::unbudgeted(&device, &queue),
            |dst| write_scalar_source_as_pairs(&replacement, dst),
            |_| {
                Err(AllocError::AllocationFailed {
                    resource: "column staging buffer",
                    reason: "injected test failure".into(),
                })
            },
        ) {
            Ok(_) => panic!("injected staging failure unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(matches!(staging_error, AllocError::AllocationFailed { .. }));
        assert_eq!(pool_state(&pool, "x"), before);

        let empty = col_f64(Vec::new());
        let validation_error = match pool.begin_upsert_column(
            "x".into(),
            &empty,
            GpuAllocCtx::unbudgeted(&device, &queue),
        ) {
            Ok(_) => panic!("empty replacement unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(validation_error, AllocError::EmptySource);
        assert_eq!(pool_state(&pool, "x"), before);

        let too_large = col_f64((0..50).map(|value| value as f64).collect());
        let capacity_error = match pool.begin_upsert_column(
            "x".into(),
            &too_large,
            GpuAllocCtx::unbudgeted(&device, &queue),
        ) {
            Ok(_) => panic!("oversized replacement unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(matches!(capacity_error, AllocError::OutOfSpace { .. }));
        assert_eq!(pool_state(&pool, "x"), before);

        let handle = pool
            .upsert_column(
                "x".into(),
                &replacement,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(handle.len_values, replacement.data.len());
        assert_eq!(pool.slot("x").unwrap().min, 4.0);
    }

    #[test]
    fn dropped_upsert_restores_old_mapping_stats_and_bytes() {
        let Some((device, queue, mut pool)) = mk_pool(ALIGN) else {
            return;
        };
        let old = col_f64(vec![1.25, 2.5, 4.75]);
        let old_handle = pool
            .add_column("x".into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let before = pool_state(&pool, "x");

        {
            let replacement = col_f64(vec![100.0, 200.0, 300.0]);
            let pending = pool
                .begin_upsert_column(
                    "x".into(),
                    &replacement,
                    GpuAllocCtx::unbudgeted(&device, &queue),
                )
                .unwrap();
            assert_ne!(pending.handle().generation, old_handle.generation);
            assert_eq!(pending.pool().slot("x").unwrap().min, 100.0);
        }

        assert_eq!(pool_state(&pool, "x"), before);
        assert!(pool.is_valid_handle(&old_handle));
        assert_eq!(
            read_column_values(GpuAllocCtx::unbudgeted(&device, &queue), &pool, old_handle),
            old.data
        );
    }

    #[test]
    fn same_size_replacement_reuses_full_capacity_and_updates_stats() {
        let Some((device, queue, mut pool)) = mk_pool(ALIGN) else {
            return;
        };
        let old = col_f64(vec![-4.0, -2.0, 8.0]);
        let old_handle = pool
            .add_column("x".into(), &old, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert_eq!(pool.free_bytes(), 0);

        let replacement = col_f64(vec![-10.0, 0.0, 3.5]);
        let pending = pool
            .begin_upsert_column(
                "x".into(),
                &replacement,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert!(pending.replaced_existing());
        assert_eq!(pending.pool().free_bytes(), 0);
        let new_handle = pending.commit();

        assert_eq!(pool.capacity(), ALIGN);
        assert_eq!(pool.used_bytes(), ALIGN);
        assert_eq!(pool.free_bytes(), 0);
        assert_ne!(new_handle.generation, old_handle.generation);
        assert!(!pool.is_valid_handle(&old_handle));
        let slot = pool.slot("x").unwrap();
        assert_eq!(
            (slot.min, slot.max, slot.min_positive),
            (-10.0, 3.5, Some(3.5))
        );
        assert_eq!(
            read_column_values(GpuAllocCtx::unbudgeted(&device, &queue), &pool, new_handle),
            replacement.data
        );
    }

    #[test]
    fn upsert_inserts_and_replaces_larger_and_smaller_columns() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let small = col_f64((0..20).map(|value| value as f64).collect());
        let inserted = pool
            .upsert_column("x".into(), &small, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert_eq!(inserted.byte_size, ALIGN);

        let larger = col_f64((0..50).map(|value| value as f64).collect());
        let larger_handle = pool
            .upsert_hilo_column(
                "x".into(),
                &larger,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(larger_handle.byte_size, 2 * ALIGN);
        assert_eq!(pool.used_bytes(), 2 * ALIGN);
        assert_eq!(pool.slot("x").unwrap().len_values, 50);

        let smaller = col_f64(vec![7.0, 8.0]);
        let smaller_handle = pool
            .upsert_column(
                "x".into(),
                &smaller,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(smaller_handle.byte_size, ALIGN);
        assert_eq!(pool.used_bytes(), ALIGN);
        assert_eq!(pool.free_bytes(), 3 * ALIGN);
        assert_eq!(
            read_column_values(
                GpuAllocCtx::unbudgeted(&device, &queue),
                &pool,
                smaller_handle
            ),
            smaller.data
        );
    }

    #[test]
    fn pool_basic_add_lookup_remove() {
        let Some((device, queue, mut pool)) = mk_pool(64 * 1024) else {
            println!("no adapter — skipping");
            return;
        };

        let c = col_f64((0..100).map(|i| i as f64).collect());
        let h = pool
            .add_column(
                "x".to_string(),
                &c,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();

        // 100 logical f32-pair values = 800 bytes, rounded up to ALIGN(256) -> 1024.
        assert_eq!(h.byte_size, 1024);
        assert_eq!(h.offset, 0);
        assert_eq!(h.len_values, 100);

        assert!(pool.handle_for("x").is_some());
        assert_eq!(pool.used_bytes(), 1024);

        let removed = pool.remove_column("x").unwrap();
        assert!(removed);
        assert!(pool.handle_for("x").is_none());
        assert_eq!(pool.used_bytes(), 0);
        assert_eq!(pool.free_bytes(), pool.capacity());
    }

    #[test]
    fn pool_sequential_alloc_offsets() {
        let Some((device, queue, mut pool)) = mk_pool(64 * 1024) else {
            return;
        };
        let a = col_f64((0..25).map(|i| i as f64).collect()); // 200 -> 256
        let b = col_f64((0..120).map(|i| i as f64).collect()); // 960 -> 1024
        let c = col_f64((0..10).map(|i| i as f64).collect()); // 80 -> 256

        let ha = pool
            .add_column("a".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let hb = pool
            .add_column("b".into(), &b, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let hc = pool
            .add_column("c".into(), &c, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();

        assert_eq!(ha.offset, 0);
        assert_eq!(hb.offset, 256);
        assert_eq!(hc.offset, 256 + 1024);
        assert_eq!(pool.used_bytes(), 256 + 1024 + 256);
    }

    #[test]
    fn pool_coalesce_after_remove() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let a = col_f64((0..25).map(|i| i as f64).collect()); // 256
        let b = col_f64((0..25).map(|i| i as f64).collect()); // 256
        let c = col_f64((0..25).map(|i| i as f64).collect()); // 256

        pool.add_column("a".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        pool.add_column("b".into(), &b, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        pool.add_column("c".into(), &c, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();

        // Remove middle b → free = [256..512] + [768..end] (not adjacent).
        pool.remove_column("b").unwrap();
        assert_eq!(pool.free.len(), 2);

        // Remove a → [0..512] (coalesced with b's hole) + [768..end].
        pool.remove_column("a").unwrap();
        assert_eq!(pool.free.len(), 2);
        assert_eq!(pool.free[0].offset, 0);
        assert_eq!(pool.free[0].size, 512);

        // Remove c → everything coalesces back to one free region.
        pool.remove_column("c").unwrap();
        assert_eq!(pool.free.len(), 1);
        assert_eq!(pool.free[0].offset, 0);
        assert_eq!(pool.free[0].size, pool.capacity());
    }

    #[test]
    fn pool_out_of_space() {
        let Some((device, queue, mut pool)) = mk_pool(1024) else {
            return;
        };
        let big = col_f64((0..500).map(|i| i as f64).collect()); // 4000 -> 4096
        let res = pool.add_column("big".into(), &big, GpuAllocCtx::unbudgeted(&device, &queue));
        assert!(matches!(res, Err(AllocError::OutOfSpace { .. })));
    }

    #[test]
    fn pool_duplicate_id() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let a = col_f64((0..10).map(|i| i as f64).collect());
        pool.add_column("dup".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let res = pool.add_column("dup".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue));
        assert!(matches!(res, Err(AllocError::DuplicateId(_))));
    }

    #[test]
    fn pool_handle_byte_range() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let c = col_f64((0..50).map(|i| i as f64).collect());
        let h = pool
            .add_column("x".into(), &c, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let r = h.byte_range();
        assert_eq!(r.start, 0);
        assert_eq!(r.end, 512);
    }

    #[test]
    fn pool_defragment_compacts_after_remove() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let a = col_f64((0..25).map(|i| i as f64).collect()); // 256
        let b = col_f64((0..25).map(|i| i as f64).collect()); // 256
        let c = col_f64((0..25).map(|i| i as f64).collect()); // 256

        let _ = pool
            .add_column("a".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let _ = pool
            .add_column("b".into(), &b, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let hc_before = pool
            .add_column("c".into(), &c, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let gen_before = pool.generation();

        // Remove middle b → hole at [256..512]; tail free region unchanged.
        pool.remove_column("b").unwrap();
        assert_eq!(pool.slots["c"].offset, 512);
        assert_eq!(pool.free.len(), 2);
        assert_eq!(pool.free[0].offset, 256);
        assert_eq!(pool.free[0].size, 256);

        // After defrag → a@0, c@256, free is a single tail region.
        let moved = pool
            .defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert!(moved);
        assert_eq!(pool.slots["a"].offset, 0);
        assert_eq!(pool.slots["c"].offset, 256);
        assert_eq!(pool.free.len(), 1);
        assert_eq!(pool.free[0].offset, 512);
        assert_eq!(pool.free[0].size, pool.capacity() - 512);

        // Generation bumped; old handle is stale.
        assert_ne!(pool.generation(), gen_before);
        assert!(!pool.is_valid_handle(&hc_before));

        // A re-fetched handle is valid.
        let hc_new = pool.handle_for("c").unwrap();
        assert!(pool.is_valid_handle(&hc_new));
        assert_eq!(hc_new.offset, 256);
    }

    #[test]
    fn pool_defragment_no_op_when_packed() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let a = col_f64((0..25).map(|i| i as f64).collect());
        let b = col_f64((0..25).map(|i| i as f64).collect());
        pool.add_column("a".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        pool.add_column("b".into(), &b, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        let g0 = pool.generation();

        let moved = pool
            .defragment(GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert!(!moved);
        assert_eq!(pool.generation(), g0); // no-op leaves generation alone.
    }

    #[test]
    fn pool_on_alloc_failure_auto_defrags() {
        // Capacity holds exactly three 256-byte slots.
        let Some((device, queue, mut pool)) = mk_pool(3 * 256) else {
            return;
        };
        pool.defrag_policy = DefragPolicy::OnAllocFailure;

        let a = col_f64((0..25).map(|i| i as f64).collect());
        let b = col_f64((0..25).map(|i| i as f64).collect());
        let c = col_f64((0..25).map(|i| i as f64).collect());
        let d = col_f64((0..25).map(|i| i as f64).collect());

        pool.add_column("a".into(), &a, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        pool.add_column("b".into(), &b, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        pool.add_column("c".into(), &c, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();

        // Remove middle b → free [256..512] (256 bytes).
        pool.remove_column("b").unwrap();

        // Adding d uses the [256..512] hole via plain first-fit, so this
        // path doesn't actually exercise the OnAllocFailure retry — it
        // succeeds on the first try.
        pool.add_column("d".into(), &d, GpuAllocCtx::unbudgeted(&device, &queue))
            .unwrap();
        assert_eq!(pool.slots["d"].offset, 256);
    }

    #[test]
    fn pool_on_alloc_failure_auto_defrag_then_succeed() {
        // Capacity = 4 * 256 = 1024. After three 256-byte slots and the
        // middle one removed, fragmentation makes first-fit for a 512-byte
        // slot fail; OnAllocFailure should defrag and the retry should win.
        let Some((device, queue, mut pool)) = mk_pool(4 * 256) else {
            return;
        };
        pool.defrag_policy = DefragPolicy::OnAllocFailure;

        let small_a = col_f64((0..25).map(|i| i as f64).collect()); // 256
        let small_b = col_f64((0..25).map(|i| i as f64).collect()); // 256
        let small_c = col_f64((0..25).map(|i| i as f64).collect()); // 256
        let big = col_f64((0..50).map(|i| i as f64).collect()); // 400 -> 512

        pool.add_column(
            "a".into(),
            &small_a,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "b".into(),
            &small_b,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "c".into(),
            &small_c,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        // free = [768..1024]. Remove b → free = [256..512] + [768..1024].
        pool.remove_column("b").unwrap();
        // big needs 512 contiguous. First-fit fails (largest free is 256),
        // defrag fuses the hole and tail into [512..1024], retry succeeds.
        let res = pool.add_column("big".into(), &big, GpuAllocCtx::unbudgeted(&device, &queue));
        assert!(
            res.is_ok(),
            "auto defrag retry should have succeeded: {res:?}"
        );
        // big lands right after a/c, at offset 512.
        assert_eq!(pool.slots["big"].offset, 512);
    }

    #[test]
    fn pool_manual_policy_does_not_auto_defrag() {
        let Some((device, queue, mut pool)) = mk_pool(4 * 256) else {
            return;
        };
        // Default policy is Manual.
        let small_a = col_f64((0..25).map(|i| i as f64).collect());
        let small_b = col_f64((0..25).map(|i| i as f64).collect());
        let small_c = col_f64((0..25).map(|i| i as f64).collect());
        let big = col_f64((0..50).map(|i| i as f64).collect());

        pool.add_column(
            "a".into(),
            &small_a,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "b".into(),
            &small_b,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "c".into(),
            &small_c,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.remove_column("b").unwrap();
        // Manual policy: no retry → OutOfSpace.
        let res = pool.add_column("big".into(), &big, GpuAllocCtx::unbudgeted(&device, &queue));
        assert!(matches!(res, Err(AllocError::OutOfSpace { .. })));
    }

    #[test]
    fn remove_epochs_prevent_offset_reuse_aba() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0]);

        let a_handle = pool
            .add_column(
                "epoch-a".into(),
                &values,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let a_epoch = pool.allocation_epoch("epoch-a").unwrap();
        let b_handle = pool
            .add_column(
                "epoch-b".into(),
                &values,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let b_epoch = pool.allocation_epoch("epoch-b").unwrap();
        let layout_generation = pool.layout_generation();

        assert!(pool.remove_column("epoch-a").unwrap());
        assert_eq!(pool.layout_generation(), layout_generation);
        assert_eq!(pool.allocation_epoch("epoch-b"), Some(b_epoch));
        assert!(!pool.is_valid_handle(&a_handle));
        assert!(!pool.is_valid_handle(&b_handle));
        assert!(pool.is_valid_handle(&pool.handle_for("epoch-b").unwrap()));

        let c_handle = pool
            .add_column(
                "epoch-c".into(),
                &values,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(c_handle.offset, a_handle.offset);
        assert_ne!(pool.allocation_epoch("epoch-c"), Some(a_epoch));
        assert!(!pool.is_valid_handle(&a_handle));

        assert!(pool.remove_column("epoch-c").unwrap());
        let a_readded = pool
            .add_column(
                "epoch-a".into(),
                &values,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(a_readded.offset, a_handle.offset);
        assert_ne!(pool.allocation_epoch("epoch-a"), Some(a_epoch));
        assert!(!pool.is_valid_handle(&a_handle));
    }

    #[test]
    fn defragment_preserves_allocation_epochs_and_bumps_layout() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0]);

        pool.add_column(
            "layout-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "layout-b".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "layout-c".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        assert!(pool.remove_column("layout-b").unwrap());

        let a_epoch = pool.allocation_epoch("layout-a");
        let c_epoch = pool.allocation_epoch("layout-c");
        let c_handle = pool.handle_for("layout-c").unwrap();
        let generation = pool.generation();
        let layout_generation = pool.layout_generation();

        assert!(
            pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap()
        );
        assert_eq!(pool.generation(), generation.checked_add(1).unwrap());
        assert_eq!(
            pool.layout_generation(),
            layout_generation.checked_add(1).unwrap()
        );
        assert_eq!(pool.allocation_epoch("layout-a"), a_epoch);
        assert_eq!(pool.allocation_epoch("layout-c"), c_epoch);
        assert!(!pool.is_valid_handle(&c_handle));
        assert!(pool.is_valid_handle(&pool.handle_for("layout-c").unwrap()));
    }

    #[test]
    fn clear_and_counter_exhaustion_preserve_stamp_contracts() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let values = col_f64(vec![1.0]);

        let old_handle = pool
            .add_column(
                "clear-a".into(),
                &values,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let old_epoch = pool.allocation_epoch("clear-a").unwrap();
        let allocation_epoch_counter = pool.allocation_epoch_counter;
        let generation = pool.generation();
        let layout_generation = pool.layout_generation();
        pool.clear().unwrap();
        assert_eq!(pool.generation(), generation.checked_add(1).unwrap());
        assert_eq!(
            pool.layout_generation(),
            layout_generation.checked_add(1).unwrap()
        );
        assert_eq!(pool.allocation_epoch_counter, allocation_epoch_counter);
        assert_eq!(pool.allocation_epoch("clear-a"), None);
        assert!(!pool.is_valid_handle(&old_handle));
        pool.add_column(
            "clear-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        assert!(pool.allocation_epoch("clear-a").unwrap() > old_epoch);

        pool.generation = u32::MAX;
        for slot in pool.slots.values_mut() {
            slot.generation = u32::MAX;
        }
        let before_public_exhaustion = pool_state(&pool, "clear-a");
        assert_eq!(
            pool.remove_column("clear-a").unwrap_err(),
            AllocError::CounterExhausted {
                counter: "public generation"
            }
        );
        assert_eq!(pool_state(&pool, "clear-a"), before_public_exhaustion);
        assert_eq!(
            pool.clear().unwrap_err(),
            AllocError::CounterExhausted {
                counter: "public generation"
            }
        );
        assert_eq!(pool_state(&pool, "clear-a"), before_public_exhaustion);
        let replacement_error = match pool.begin_upsert_column(
            "clear-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        ) {
            Err(error) => error,
            Ok(_) => panic!("replacement must fail when public generation is exhausted"),
        };
        assert_eq!(
            replacement_error,
            AllocError::CounterExhausted {
                counter: "public generation"
            }
        );
        assert_eq!(pool_state(&pool, "clear-a"), before_public_exhaustion);

        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        pool.add_column(
            "epoch-full-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.allocation_epoch_counter = u64::MAX;
        let before_epoch_exhaustion = pool_state(&pool, "epoch-full-a");
        assert_eq!(
            pool.add_column(
                "epoch-full-b".into(),
                &values,
                GpuAllocCtx::unbudgeted(&device, &queue)
            )
            .unwrap_err(),
            AllocError::CounterExhausted {
                counter: "allocation epoch"
            }
        );
        assert_eq!(pool_state(&pool, "epoch-full-a"), before_epoch_exhaustion);

        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        pool.add_column(
            "layout-prefix".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "layout-live".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        assert!(pool.remove_column("layout-prefix").unwrap());
        pool.layout_generation = u64::MAX;
        let before_layout_exhaustion = pool_state(&pool, "layout-live");
        assert_eq!(
            pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap_err(),
            AllocError::CounterExhausted {
                counter: "layout generation"
            }
        );
        assert_eq!(pool_state(&pool, "layout-live"), before_layout_exhaustion);
        assert_eq!(
            pool.clear().unwrap_err(),
            AllocError::CounterExhausted {
                counter: "layout generation"
            }
        );
        assert_eq!(pool_state(&pool, "layout-live"), before_layout_exhaustion);
    }

    #[test]
    fn every_relocating_counter_failure_is_fail_closed() {
        let values = col_f64(vec![1.0]);

        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        pool.add_column(
            "epoch-upsert-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.allocation_epoch_counter = u64::MAX;
        let before_epoch_upsert = pool_state(&pool, "epoch-upsert-a");
        let epoch_error = match pool.begin_upsert_column(
            "epoch-upsert-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        ) {
            Err(error) => error,
            Ok(_) => panic!("upsert must fail when allocation epoch is exhausted"),
        };
        assert_eq!(
            epoch_error,
            AllocError::CounterExhausted {
                counter: "allocation epoch"
            }
        );
        assert_eq!(pool_state(&pool, "epoch-upsert-a"), before_epoch_upsert);

        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        for id in ["public-compact-a", "public-compact-b"] {
            pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
        }
        pool.generation = u32::MAX;
        for slot in pool.slots.values_mut() {
            slot.generation = u32::MAX;
        }
        let before_public_compaction = pool_state(&pool, "public-compact-a");
        let public_compaction_error = match pool.begin_upsert_column(
            "public-compact-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        ) {
            Err(error) => error,
            Ok(_) => panic!("compacting upsert must fail when public generation is exhausted"),
        };
        assert_eq!(
            public_compaction_error,
            AllocError::CounterExhausted {
                counter: "public generation"
            }
        );
        assert_eq!(
            pool_state(&pool, "public-compact-a"),
            before_public_compaction
        );

        let Some((device, queue, mut pool)) = mk_pool(2 * ALIGN) else {
            return;
        };
        for id in ["layout-compact-a", "layout-compact-b"] {
            pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
        }
        pool.layout_generation = u64::MAX;
        let before_layout_compaction = pool_state(&pool, "layout-compact-a");
        let layout_compaction_error = match pool.begin_upsert_column(
            "layout-compact-a".into(),
            &values,
            GpuAllocCtx::unbudgeted(&device, &queue),
        ) {
            Err(error) => error,
            Ok(_) => panic!("compacting upsert must fail when layout generation is exhausted"),
        };
        assert_eq!(
            layout_compaction_error,
            AllocError::CounterExhausted {
                counter: "layout generation"
            }
        );
        assert_eq!(
            pool_state(&pool, "layout-compact-a"),
            before_layout_compaction
        );

        let Some((device, queue, mut pool)) = mk_pool(3 * ALIGN) else {
            return;
        };
        for id in ["public-defrag-a", "public-defrag-b", "public-defrag-c"] {
            pool.add_column(id.into(), &values, GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
        }
        assert!(pool.remove_column("public-defrag-a").unwrap());
        pool.generation = u32::MAX;
        for slot in pool.slots.values_mut() {
            slot.generation = u32::MAX;
        }
        let before_public_defrag = pool_state(&pool, "public-defrag-b");
        assert_eq!(
            pool.defragment(GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap_err(),
            AllocError::CounterExhausted {
                counter: "public generation"
            }
        );
        assert_eq!(pool_state(&pool, "public-defrag-b"), before_public_defrag);
    }

    #[test]
    fn successful_stamp_transitions_separate_replacement_from_pool_relocation() {
        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        let first = col_f64(vec![1.0]);
        let replacement = col_f64(vec![2.0]);
        pool.add_column(
            "direct-target".into(),
            &first,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "direct-survivor".into(),
            &first,
            GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let old_target_handle = pool.handle_for("direct-target").unwrap();
        let old_survivor_handle = pool.handle_for("direct-survivor").unwrap();
        let old_target_epoch = pool.allocation_epoch("direct-target").unwrap();
        let survivor_epoch = pool.allocation_epoch("direct-survivor").unwrap();
        let generation = pool.generation();
        let layout_generation = pool.layout_generation();

        let new_target_handle = pool
            .upsert_column(
                "direct-target".into(),
                &replacement,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();

        assert_eq!(pool.generation(), generation.checked_add(1).unwrap());
        assert_eq!(pool.layout_generation(), layout_generation);
        assert_ne!(
            pool.allocation_epoch("direct-target"),
            Some(old_target_epoch)
        );
        assert_eq!(
            pool.allocation_epoch("direct-survivor"),
            Some(survivor_epoch)
        );
        assert!(!pool.is_valid_handle(&old_target_handle));
        assert!(!pool.is_valid_handle(&old_survivor_handle));
        assert!(pool.is_valid_handle(&new_target_handle));
        assert!(pool.is_valid_handle(&pool.handle_for("direct-survivor").unwrap()));

        let Some((device, queue, mut pool)) = mk_pool(4 * ALIGN) else {
            return;
        };
        for id in ["relocate-a", "relocate-b", "relocate-c", "relocate-d"] {
            pool.add_column(id.into(), &first, GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap();
        }
        assert!(pool.remove_column("relocate-b").unwrap());
        assert!(pool.remove_column("relocate-d").unwrap());
        pool.defrag_policy = DefragPolicy::OnAllocFailure;
        let a_handle = pool.handle_for("relocate-a").unwrap();
        let c_handle = pool.handle_for("relocate-c").unwrap();
        let a_epoch = pool.allocation_epoch("relocate-a").unwrap();
        let c_epoch = pool.allocation_epoch("relocate-c").unwrap();
        let generation = pool.generation();
        let layout_generation = pool.layout_generation();
        let large = col_f64(vec![3.0; 33]);

        let new_handle = pool
            .upsert_column(
                "relocate-new".into(),
                &large,
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();

        assert_eq!(pool.generation(), generation.checked_add(1).unwrap());
        assert_eq!(
            pool.layout_generation(),
            layout_generation.checked_add(1).unwrap()
        );
        assert_eq!(pool.allocation_epoch("relocate-a"), Some(a_epoch));
        assert_eq!(pool.allocation_epoch("relocate-c"), Some(c_epoch));
        assert!(pool.allocation_epoch("relocate-new").is_some());
        assert!(!pool.is_valid_handle(&a_handle));
        assert!(!pool.is_valid_handle(&c_handle));
        assert!(pool.is_valid_handle(&new_handle));
        assert!(pool.is_valid_handle(&pool.handle_for("relocate-a").unwrap()));
        assert!(pool.is_valid_handle(&pool.handle_for("relocate-c").unwrap()));
    }
}
