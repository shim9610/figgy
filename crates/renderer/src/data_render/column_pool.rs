//! GPU columnar memory pool — column-sized slabs in one big buffer, with an
//! offset table and ping-pong defrag.
//!
//! - One `primary` GPU buffer holds every column, packed by offset.
//! - `slots: HashMap<ColumnId, ColumnSlot>` is the SSoT mapping id → byte
//!   range; `free: Vec<FreeRegion>` tracks holes (first-fit on add, coalesce
//!   with neighbors on remove).
//! - `add_hilo_column` is a zero-copy stream upload: a staging buffer created
//!   with `mapped_at_creation` lets `HiLoColumnSource::write_f32_pair_le_into`
//!   fill split `(hi, lo)` bytes directly with no intermediate `Vec`.
//! - `defragment` packs survivors into a backup buffer with GPU-internal
//!   copies (no PCIe traffic) then swaps `primary <-> backup`.
//!
//! `ColumnHandle` carries a `generation` value; defrag and `clear` bump
//! `generation`, so callers detect staleness via `is_valid_handle` and
//! re-fetch with `handle_for`.
//!
//! Auto-defrag is opt-in via [`DefragPolicy`] (default `Manual`). With
//! `OnAllocFailure`, an `OutOfSpace` from `add_column` triggers one
//! `defragment()` and a single retry.
//!
//! All offsets and sizes are [`ALIGN`] = 256-byte aligned to satisfy wgpu's
//! storage-binding alignment; vertex slices reuse the same value to keep
//! mode switching free of caveats.

use std::collections::HashMap;

use wgpu::{Buffer, BufferDescriptor, BufferUsages, Device, Queue};

use crate::data::{COLUMN_VALUE_BYTES, ColumnSource, HiLoColumnSource};

// Defined in the model crate (`model::data`); re-exported here so
// `data_render::ColumnId` stays a valid path.
pub use crate::data::ColumnId;

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
    /// Smallest strictly-positive value, scanned once at upload — the
    /// log-axis auto-fit lower bound when the data contains zeros or
    /// negatives. `None` when no positive value exists.
    ///
    /// Scalar stats like this are the ONLY per-value information retained on
    /// the CPU. The pool deliberately keeps no copy of the data itself —
    /// per-point geometry (dashed-line arc length) is computed on the GPU
    /// (`line_arc.wgsl`). Do not reintroduce CPU shadows.
    pub min_positive: Option<f64>,
}

/// A free region. Adjacent regions are merged on coalesce.
#[derive(Debug, Clone, Copy)]
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

impl RegionReservation<'_> {
    fn offset(&self) -> u64 {
        self.original.offset
    }

    fn commit(mut self) {
        self.rollback = None;
    }
}

impl Drop for RegionReservation<'_> {
    fn drop(&mut self) {
        match self.rollback.take() {
            Some(RegionReservationRollback::ExactFit) => {
                // `remove` retained enough Vec capacity for this insertion,
                // so rollback itself does not allocate.
                self.free.insert(self.index, self.original);
            }
            Some(RegionReservationRollback::Split) => {
                self.free[self.index] = self.original;
            }
            None => {}
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

fn write_scalar_source_as_pairs(source: &dyn ColumnSource, dst: &mut [u8]) {
    source.write_f32_zero_lo_pair_le_into(dst);
}

/// Lightweight handle handed out to the chart layer. `generation` lets
/// callers detect a stale handle after a defrag.
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
        }
    }
}

impl std::error::Error for AllocError {}

fn create_buffer_checked(
    device: &Device,
    desc: &BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<Buffer, AllocError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_buffer(desc))).map_err(
        |_| AllocError::AllocationFailed {
            resource,
            reason: "wgpu Device::create_buffer panicked".into(),
        },
    )
}

/// GPU column slab + CPU-side offset table.
pub struct ColumnPool {
    primary: Buffer,
    capacity: u64,
    max_buffer_size: u64,
    slots: HashMap<ColumnId, ColumnSlot>,
    /// Kept sorted by offset (coalesce and first-fit both rely on this).
    free: Vec<FreeRegion>,
    generation: u32,
    /// Ping-pong target for defrag. Lazily created on the first defrag,
    /// then alternates with `primary`.
    backup: Option<Buffer>,
    /// Whether to auto-defrag on alloc failure. Default `Manual`.
    pub defrag_policy: DefragPolicy,
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
/// guard restores the exact old allocator, slots, generation, and buffers.
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
            && let UpsertBufferRollback::Candidate { old_primary, .. } = &mut rollback.buffer
        {
            // Keep the displaced primary as the next defragmentation target.
            // Replacing an Option and dropping the superseded backup cannot
            // allocate or return an error.
            self.pool.backup = old_primary.take();
        }
        self.handle
    }
}

impl Drop for ColumnUpsert<'_> {
    fn drop(&mut self) {
        let Some(mut rollback) = self.rollback.take() else {
            return;
        };

        std::mem::swap(&mut self.pool.slots, &mut rollback.slots);
        std::mem::swap(&mut self.pool.free, &mut rollback.free);
        self.pool.generation = rollback.generation;

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

impl ColumnPool {
    /// New pool. `capacity_bytes` is rounded up to a multiple of `ALIGN`.
    pub fn new(device: &Device, capacity_bytes: u64) -> Result<Self, AllocError> {
        let max_buffer_size = device.limits().max_buffer_size;
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
            primary,
            capacity,
            max_buffer_size,
            slots: HashMap::new(),
            free: vec![FreeRegion {
                offset: 0,
                size: capacity,
            }],
            generation: 0,
            backup: None,
            defrag_policy: DefragPolicy::Manual,
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

    pub fn used_bytes(&self) -> u64 {
        self.slots.values().map(|s| s.byte_size).sum()
    }

    pub fn free_bytes(&self) -> u64 {
        self.free.iter().map(|r| r.size).sum()
    }

    pub fn slot(&self, id: &str) -> Option<&ColumnSlot> {
        self.slots.get(id)
    }

    /// Drop all columns. The primary buffer is reused (capacity unchanged).
    /// Bumps `generation` so any outstanding handles become stale.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.free.push(FreeRegion {
            offset: 0,
            size: self.capacity,
        });
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn handle_for(&self, id: &str) -> Option<ColumnHandle> {
        self.slots.get(id).map(|s| ColumnHandle {
            generation: s.generation,
            offset: s.offset,
            byte_size: s.byte_size,
            len_values: s.len_values,
        })
    }

    /// True if the handle is still valid for the current pool state.
    /// After a defrag or clear, returns false.
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
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnHandle, AllocError> {
        // Only clone `id` for retry under OnAllocFailure; the default path allocates nothing extra.
        let retry_id = if self.defrag_policy == DefragPolicy::OnAllocFailure {
            Some(id.clone())
        } else {
            None
        };
        match self.try_add_column(id, source, device, queue) {
            Ok(h) => Ok(h),
            Err(e @ AllocError::OutOfSpace { .. }) => match retry_id {
                Some(rid) => {
                    self.defragment(device, queue)?;
                    self.try_add_column(rid, source, device, queue)
                }
                None => Err(e),
            },
            Err(e) => Err(e),
        }
    }

    pub fn add_hilo_column(
        &mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnHandle, AllocError> {
        let retry_id = if self.defrag_policy == DefragPolicy::OnAllocFailure {
            Some(id.clone())
        } else {
            None
        };
        match self.try_add_hilo_column(id, source, device, queue) {
            Ok(h) => Ok(h),
            Err(e @ AllocError::OutOfSpace { .. }) => match retry_id {
                Some(rid) => {
                    self.defragment(device, queue)?;
                    self.try_add_hilo_column(rid, source, device, queue)
                }
                None => Err(e),
            },
            Err(e) => Err(e),
        }
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
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnUpsert<'a>, AllocError> {
        self.begin_upsert_column_pairs_with(
            id,
            source.len(),
            source.min(),
            source.max(),
            device,
            queue,
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
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnUpsert<'a>, AllocError> {
        self.begin_upsert_column_pairs_with(
            id,
            source.len(),
            source.min(),
            source.max(),
            device,
            queue,
            |dst| source.write_f32_pair_le_into(dst),
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
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnHandle, AllocError> {
        Ok(self
            .begin_upsert_column(id, source, device, queue)?
            .commit())
    }

    /// Failure-atomic hi/lo upsert when no dependent batch preparation is
    /// needed between prepare and commit.
    pub fn upsert_hilo_column(
        &mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnHandle, AllocError> {
        Ok(self
            .begin_upsert_hilo_column(id, source, device, queue)?
            .commit())
    }

    fn begin_upsert_column_pairs_with<'a>(
        &'a mut self,
        id: ColumnId,
        n: usize,
        min: f64,
        max: f64,
        device: &Device,
        queue: &Queue,
        write_pairs: impl FnOnce(&mut [u8]),
        create_staging: impl FnOnce(u64) -> Result<Buffer, AllocError>,
    ) -> Result<ColumnUpsert<'a>, AllocError> {
        if n == 0 {
            return Err(AllocError::EmptySource);
        }

        let raw_bytes =
            (n as u64)
                .checked_mul(COLUMN_VALUE_BYTES as u64)
                .ok_or(AllocError::ResourceLimit {
                    resource: "column staging buffer",
                    requested: u64::MAX,
                    limit: self.max_buffer_size,
                })?;
        let byte_size = try_align_up(raw_bytes, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: "column staging buffer",
            requested: raw_bytes,
            limit: self.max_buffer_size,
        })?;
        if byte_size > self.max_buffer_size {
            return Err(AllocError::ResourceLimit {
                resource: "column staging buffer",
                requested: byte_size,
                limit: self.max_buffer_size,
            });
        }

        // Complete every source-dependent/fallible staging operation before
        // touching allocator metadata or the live primary buffer.
        let staging = create_staging(byte_size)?;
        let mut min_positive = f64::INFINITY;
        {
            let mut view = staging.slice(..).get_mapped_range_mut();
            write_pairs(&mut view[..raw_bytes as usize]);
            for bytes in view[..raw_bytes as usize].chunks_exact(COLUMN_VALUE_BYTES) {
                let hi = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64;
                let lo = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as f64;
                let value = hi + lo;
                if value > 0.0 && value < min_positive {
                    min_positive = value;
                }
            }
        }
        staging.unmap();
        let min_positive = min_positive.is_finite().then_some(min_positive);

        let old_slot = self.slots.get(&id).cloned();
        let replaced_existing = old_slot.is_some();
        let mut planned_slots = self.slots.clone();
        let mut planned_free = self.free.clone();

        let direct_offset = alloc_region_from(&mut planned_free, byte_size);
        let (target, new_offset, mut command_encoder) = match direct_offset {
            Ok(offset) => {
                if let Some(old) = old_slot.as_ref() {
                    planned_free.push(FreeRegion {
                        offset: old.offset,
                        size: old.byte_size,
                    });
                    coalesce_regions(&mut planned_free);
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
                let next_generation = self.generation.wrapping_add(1);
                let mut next = 0;
                for column_id in &order {
                    let slot = planned_slots
                        .get_mut(column_id)
                        .expect("planned survivor remains present");
                    slot.offset = next;
                    slot.generation = next_generation;
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

        let next_generation =
            if replaced_existing || !matches!(&target, PreparedUpsertTarget::InPlace) {
                self.generation.wrapping_add(1)
            } else {
                self.generation
            };
        if next_generation != self.generation {
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

        Ok(ColumnUpsert {
            pool: self,
            rollback: Some(UpsertRollback {
                slots: old_slots,
                free: old_free,
                generation: old_generation,
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
    /// 3. `ColumnSource::write_f32_zero_lo_pair_le_into` writes bytes directly
    ///    into the mapped slice (no intermediate Vec).
    /// 4. Unmap, encode a staging→primary copy, submit.
    fn try_add_column(
        &mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnHandle, AllocError> {
        self.try_add_column_pairs(
            id,
            source.len(),
            source.min(),
            source.max(),
            device,
            queue,
            |dst| write_scalar_source_as_pairs(source, dst),
        )
    }

    fn try_add_hilo_column(
        &mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        device: &Device,
        queue: &Queue,
    ) -> Result<ColumnHandle, AllocError> {
        self.try_add_column_pairs(
            id,
            source.len(),
            source.min(),
            source.max(),
            device,
            queue,
            |dst| source.write_f32_pair_le_into(dst),
        )
    }

    fn try_add_column_pairs(
        &mut self,
        id: ColumnId,
        n: usize,
        min: f64,
        max: f64,
        device: &Device,
        queue: &Queue,
        write_pairs: impl FnOnce(&mut [u8]),
    ) -> Result<ColumnHandle, AllocError> {
        self.try_add_column_pairs_with(id, n, min, max, device, queue, write_pairs, |byte_size| {
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
        id: ColumnId,
        n: usize,
        min: f64,
        max: f64,
        device: &Device,
        queue: &Queue,
        write_pairs: impl FnOnce(&mut [u8]),
        create_staging: impl FnOnce(u64) -> Result<Buffer, AllocError>,
    ) -> Result<ColumnHandle, AllocError> {
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
                    limit: self.max_buffer_size,
                })?;
        let byte_size = try_align_up(raw_bytes, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: "column staging buffer",
            requested: raw_bytes,
            limit: self.max_buffer_size,
        })?;
        if byte_size > self.max_buffer_size {
            return Err(AllocError::ResourceLimit {
                resource: "column staging buffer",
                requested: byte_size,
                limit: self.max_buffer_size,
            });
        }

        let reservation = alloc_region(&mut self.free, byte_size)?;
        let region_offset = reservation.offset();

        // Staging buffer — write into mapped memory directly, no Vec.
        let staging = create_staging(byte_size)?;
        let mut min_positive = f64::INFINITY;
        {
            // wgpu 27's BufferViewMut derefs to `&mut [u8]`, so the column
            // can serialize itself straight into staging memory.
            let mut view = staging.slice(..).get_mapped_range_mut();
            write_pairs(&mut view[..raw_bytes as usize]);
            // Scalar stats only — read the freshly written bytes once, retain
            // nothing (the upload path stays zero-copy and the pool keeps no
            // CPU shadow of the data).
            for b in view[..raw_bytes as usize].chunks_exact(COLUMN_VALUE_BYTES) {
                let hi = f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64;
                let lo = f32::from_le_bytes([b[4], b[5], b[6], b[7]]) as f64;
                let v = hi + lo;
                if v > 0.0 && v < min_positive {
                    min_positive = v;
                }
            }
        }
        staging.unmap();

        // staging → primary[region_offset..] (GPU-internal copy).
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy column upload encoder"),
        });
        enc.copy_buffer_to_buffer(&staging, 0, &self.primary, region_offset, byte_size);
        queue.submit(std::iter::once(enc.finish()));

        let slot = ColumnSlot {
            id: id.clone(),
            offset: region_offset,
            byte_size,
            len_values: n,
            generation: self.generation,
            min,
            max,
            min_positive: min_positive.is_finite().then_some(min_positive),
        };
        let handle = ColumnHandle {
            generation: slot.generation,
            offset: slot.offset,
            byte_size: slot.byte_size,
            len_values: slot.len_values,
        };
        self.slots.insert(id, slot);
        reservation.commit();
        Ok(handle)
    }

    /// Remove a column. Returns its region to the free list and coalesces
    /// with neighbors.
    pub fn remove_column(&mut self, id: &str) -> bool {
        let Some(slot) = self.slots.remove(id) else {
            return false;
        };
        self.free.push(FreeRegion {
            offset: slot.offset,
            size: slot.byte_size,
        });
        self.coalesce_free();
        true
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
    /// 7. Bump `generation`, invalidating outstanding handles.
    /// 8. Update slot offsets/generation; free list = single tail region
    ///    `[next..capacity)`.
    ///
    /// Returns true iff something actually moved (caller must re-fetch
    /// handles via `handle_for`).
    pub fn defragment(&mut self, device: &Device, queue: &Queue) -> Result<bool, AllocError> {
        // Empty pool: just normalize the free list.
        if self.slots.is_empty() {
            let already = self.free.len() == 1
                && self.free[0].offset == 0
                && self.free[0].size == self.capacity;
            if already {
                return Ok(false);
            }
            self.free.clear();
            self.free.push(FreeRegion {
                offset: 0,
                size: self.capacity,
            });
            self.generation = self.generation.wrapping_add(1);
            return Ok(true);
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
        if already_packed {
            let tail_ok = self.free.len() <= 1
                && self
                    .free
                    .first()
                    .is_none_or(|r| r.offset == next && r.offset + r.size == self.capacity);
            if !tail_ok {
                self.free.clear();
                if next < self.capacity {
                    self.free.push(FreeRegion {
                        offset: next,
                        size: self.capacity - next,
                    });
                }
            }
            return Ok(false);
        }

        // Lazily create backup with the same capacity/usage as primary.
        if self.backup.is_none() {
            let backup_desc = BufferDescriptor {
                label: Some("figgy column pool backup"),
                size: self.capacity,
                usage: BufferUsages::VERTEX
                    | BufferUsages::STORAGE
                    | BufferUsages::COPY_DST
                    | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            };
            self.backup = Some(create_buffer_checked(
                device,
                &backup_desc,
                "column pool backup",
            )?);
        }

        // primary[old_off..] -> backup[new_off..] (GPU-internal copy).
        // The `is_none` branch above guarantees `backup` is `Some`; the
        // graceful exit below exists only to handle invariant violations.
        {
            let Some(backup) = self.backup.as_ref() else {
                return Ok(false);
            };
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
        let Some(new_primary) = self.backup.take() else {
            return Ok(false);
        };
        let old_primary = std::mem::replace(&mut self.primary, new_primary);
        self.backup = Some(old_primary);

        // Bump generation and update slots.
        self.generation = self.generation.wrapping_add(1);
        for (id, &new_off) in order.iter().zip(new_offsets.iter()) {
            if let Some(slot) = self.slots.get_mut(id) {
                slot.offset = new_off;
                slot.generation = self.generation;
            }
        }

        // Free list collapses to a single tail region.
        self.free.clear();
        if next < self.capacity {
            self.free.push(FreeRegion {
                offset: next,
                size: self.capacity - next,
            });
        }
        Ok(true)
    }

    /// Sort `free` by offset and merge adjacent regions.
    fn coalesce_free(&mut self) {
        if self.free.len() < 2 {
            return;
        }
        self.free.sort_by_key(|r| r.offset);
        let mut merged: Vec<FreeRegion> = Vec::with_capacity(self.free.len());
        for r in self.free.drain(..) {
            if let Some(last) = merged.last_mut() {
                if last.offset + last.size == r.offset {
                    last.size += r.size;
                    continue;
                }
            }
            merged.push(r);
        }
        self.free = merged;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;

    struct RawScalarSource<'a> {
        bits: &'a [u32],
        write_address: std::cell::Cell<usize>,
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
            self.write_address.set(dst.as_ptr() as usize);
            assert_eq!(dst.len(), self.bits.len() * std::mem::size_of::<f32>());
            for (chunk, bits) in dst.chunks_exact_mut(4).zip(self.bits) {
                chunk.copy_from_slice(&bits.to_le_bytes());
            }
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
        let dst_address = dst.as_ptr() as usize;
        let source = RawScalarSource {
            bits: &bits,
            write_address: std::cell::Cell::new(0),
        };

        write_scalar_source_as_pairs(&source, &mut dst);

        assert_eq!(source.write_address.get(), dst_address);
        for (pair, bits) in dst.chunks_exact(COLUMN_VALUE_BYTES).zip(bits) {
            assert_eq!(&pair[..4], &bits.to_le_bytes());
            assert_eq!(&pair[4..], &0.0f32.to_le_bytes());
        }
    }

    #[test]
    fn scalar_source_expansion_accepts_empty_input() {
        let source = RawScalarSource {
            bits: &[],
            write_address: std::cell::Cell::new(0),
        };
        let mut dst = [];

        write_scalar_source_as_pairs(&source, &mut dst);

        assert!(dst.is_empty());
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
        let pool = ColumnPool::new(&device, cap).ok()?;
        Some((device, queue, pool))
    }

    fn col_f64(data: Vec<f64>) -> Column<f64> {
        let min = data.iter().copied().fold(f64::INFINITY, f64::min);
        let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column { data, min, max }
    }

    #[derive(Debug, PartialEq)]
    struct PoolState {
        buffer: usize,
        generation: u32,
        used: u64,
        free_bytes: u64,
        free: Vec<(u64, u64)>,
        slot: Option<(u64, u64, usize, u32, u64, u64, Option<u64>)>,
        handle: Option<(u32, u64, u64, usize)>,
    }

    fn pool_state(pool: &ColumnPool, id: &str) -> PoolState {
        PoolState {
            buffer: pool.buffer() as *const Buffer as usize,
            generation: pool.generation(),
            used: pool.used_bytes(),
            free_bytes: pool.free_bytes(),
            free: pool
                .free
                .iter()
                .map(|region| (region.offset, region.size))
                .collect(),
            slot: pool.slot(id).map(|slot| {
                (
                    slot.offset,
                    slot.byte_size,
                    slot.len_values,
                    slot.generation,
                    slot.min.to_bits(),
                    slot.max.to_bits(),
                    slot.min_positive.map(f64::to_bits),
                )
            }),
            handle: pool.handle_for(id).map(|handle| {
                (
                    handle.generation,
                    handle.offset,
                    handle.byte_size,
                    handle.len_values,
                )
            }),
        }
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
                "x".into(),
                column.data.len(),
                column.min,
                column.max,
                &device,
                &queue,
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
            .add_column("x".into(), &column, &device, &queue)
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
            let _ =
                scalar_pool.add_column("scalar".into(), &PanickingScalarSource, &device, &queue);
        }));
        assert!(scalar_result.is_err());
        assert_eq!(pool_state(&scalar_pool, "scalar"), scalar_before);

        let retry = col_f64(vec![2.0]);
        let scalar_handle = scalar_pool
            .add_column("scalar".into(), &retry, &device, &queue)
            .unwrap();
        assert_eq!(scalar_handle.offset, 0);

        let mut hilo_pool = ColumnPool::new(&device, 2 * ALIGN).unwrap();
        let hilo_before = pool_state(&hilo_pool, "hilo");
        let hilo_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = hilo_pool.add_hilo_column("hilo".into(), &PanickingHiLoSource, &device, &queue);
        }));
        assert!(hilo_result.is_err());
        assert_eq!(pool_state(&hilo_pool, "hilo"), hilo_before);

        let hilo_handle = hilo_pool
            .add_hilo_column("hilo".into(), &retry, &device, &queue)
            .unwrap();
        assert_eq!(hilo_handle.offset, 0);
    }

    fn read_column_values(
        device: &Device,
        queue: &Queue,
        pool: &ColumnPool,
        handle: ColumnHandle,
    ) -> Vec<f64> {
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
        let mapped = readback.slice(..handle.byte_size).get_mapped_range();
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
        pool.add_column("x".into(), &old, &device, &queue).unwrap();
        let blocker = col_f64(vec![10.0, 11.0, 12.0]);
        pool.add_column("blocker".into(), &blocker, &device, &queue)
            .unwrap();
        let before = pool_state(&pool, "x");

        let replacement = col_f64(vec![4.0, 5.0, 6.0]);
        let staging_error = match pool.begin_upsert_column_pairs_with(
            "x".into(),
            replacement.data.len(),
            replacement.min,
            replacement.max,
            &device,
            &queue,
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
        let validation_error = match pool.begin_upsert_column("x".into(), &empty, &device, &queue) {
            Ok(_) => panic!("empty replacement unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(validation_error, AllocError::EmptySource);
        assert_eq!(pool_state(&pool, "x"), before);

        let too_large = col_f64((0..50).map(|value| value as f64).collect());
        let capacity_error = match pool.begin_upsert_column("x".into(), &too_large, &device, &queue)
        {
            Ok(_) => panic!("oversized replacement unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(matches!(capacity_error, AllocError::OutOfSpace { .. }));
        assert_eq!(pool_state(&pool, "x"), before);

        let handle = pool
            .upsert_column("x".into(), &replacement, &device, &queue)
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
        let old_handle = pool.add_column("x".into(), &old, &device, &queue).unwrap();
        let before = pool_state(&pool, "x");

        {
            let replacement = col_f64(vec![100.0, 200.0, 300.0]);
            let pending = pool
                .begin_upsert_column("x".into(), &replacement, &device, &queue)
                .unwrap();
            assert_ne!(pending.handle().generation, old_handle.generation);
            assert_eq!(pending.pool().slot("x").unwrap().min, 100.0);
        }

        assert_eq!(pool_state(&pool, "x"), before);
        assert!(pool.is_valid_handle(&old_handle));
        assert_eq!(
            read_column_values(&device, &queue, &pool, old_handle),
            old.data
        );
    }

    #[test]
    fn same_size_replacement_reuses_full_capacity_and_updates_stats() {
        let Some((device, queue, mut pool)) = mk_pool(ALIGN) else {
            return;
        };
        let old = col_f64(vec![-4.0, -2.0, 8.0]);
        let old_handle = pool.add_column("x".into(), &old, &device, &queue).unwrap();
        assert_eq!(pool.free_bytes(), 0);

        let replacement = col_f64(vec![-10.0, 0.0, 3.5]);
        let pending = pool
            .begin_upsert_column("x".into(), &replacement, &device, &queue)
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
            read_column_values(&device, &queue, &pool, new_handle),
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
            .upsert_column("x".into(), &small, &device, &queue)
            .unwrap();
        assert_eq!(inserted.byte_size, ALIGN);

        let larger = col_f64((0..50).map(|value| value as f64).collect());
        let larger_handle = pool
            .upsert_hilo_column("x".into(), &larger, &device, &queue)
            .unwrap();
        assert_eq!(larger_handle.byte_size, 2 * ALIGN);
        assert_eq!(pool.used_bytes(), 2 * ALIGN);
        assert_eq!(pool.slot("x").unwrap().len_values, 50);

        let smaller = col_f64(vec![7.0, 8.0]);
        let smaller_handle = pool
            .upsert_column("x".into(), &smaller, &device, &queue)
            .unwrap();
        assert_eq!(smaller_handle.byte_size, ALIGN);
        assert_eq!(pool.used_bytes(), ALIGN);
        assert_eq!(pool.free_bytes(), 3 * ALIGN);
        assert_eq!(
            read_column_values(&device, &queue, &pool, smaller_handle),
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
            .add_column("x".to_string(), &c, &device, &queue)
            .unwrap();

        // 100 logical f32-pair values = 800 bytes, rounded up to ALIGN(256) -> 1024.
        assert_eq!(h.byte_size, 1024);
        assert_eq!(h.offset, 0);
        assert_eq!(h.len_values, 100);

        assert!(pool.handle_for("x").is_some());
        assert_eq!(pool.used_bytes(), 1024);

        let removed = pool.remove_column("x");
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

        let ha = pool.add_column("a".into(), &a, &device, &queue).unwrap();
        let hb = pool.add_column("b".into(), &b, &device, &queue).unwrap();
        let hc = pool.add_column("c".into(), &c, &device, &queue).unwrap();

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

        pool.add_column("a".into(), &a, &device, &queue).unwrap();
        pool.add_column("b".into(), &b, &device, &queue).unwrap();
        pool.add_column("c".into(), &c, &device, &queue).unwrap();

        // Remove middle b → free = [256..512] + [768..end] (not adjacent).
        pool.remove_column("b");
        assert_eq!(pool.free.len(), 2);

        // Remove a → [0..512] (coalesced with b's hole) + [768..end].
        pool.remove_column("a");
        assert_eq!(pool.free.len(), 2);
        assert_eq!(pool.free[0].offset, 0);
        assert_eq!(pool.free[0].size, 512);

        // Remove c → everything coalesces back to one free region.
        pool.remove_column("c");
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
        let res = pool.add_column("big".into(), &big, &device, &queue);
        assert!(matches!(res, Err(AllocError::OutOfSpace { .. })));
    }

    #[test]
    fn pool_duplicate_id() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let a = col_f64((0..10).map(|i| i as f64).collect());
        pool.add_column("dup".into(), &a, &device, &queue).unwrap();
        let res = pool.add_column("dup".into(), &a, &device, &queue);
        assert!(matches!(res, Err(AllocError::DuplicateId(_))));
    }

    #[test]
    fn pool_handle_byte_range() {
        let Some((device, queue, mut pool)) = mk_pool(8 * 1024) else {
            return;
        };
        let c = col_f64((0..50).map(|i| i as f64).collect());
        let h = pool.add_column("x".into(), &c, &device, &queue).unwrap();
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

        let _ = pool.add_column("a".into(), &a, &device, &queue).unwrap();
        let _ = pool.add_column("b".into(), &b, &device, &queue).unwrap();
        let hc_before = pool.add_column("c".into(), &c, &device, &queue).unwrap();
        let gen_before = pool.generation();

        // Remove middle b → hole at [256..512]; tail free region unchanged.
        pool.remove_column("b");
        assert_eq!(pool.slots["c"].offset, 512);
        assert_eq!(pool.free.len(), 2);
        assert_eq!(pool.free[0].offset, 256);
        assert_eq!(pool.free[0].size, 256);

        // After defrag → a@0, c@256, free is a single tail region.
        let moved = pool.defragment(&device, &queue).unwrap();
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
        pool.add_column("a".into(), &a, &device, &queue).unwrap();
        pool.add_column("b".into(), &b, &device, &queue).unwrap();
        let g0 = pool.generation();

        let moved = pool.defragment(&device, &queue).unwrap();
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

        pool.add_column("a".into(), &a, &device, &queue).unwrap();
        pool.add_column("b".into(), &b, &device, &queue).unwrap();
        pool.add_column("c".into(), &c, &device, &queue).unwrap();

        // Remove middle b → free [256..512] (256 bytes).
        pool.remove_column("b");

        // Adding d uses the [256..512] hole via plain first-fit, so this
        // path doesn't actually exercise the OnAllocFailure retry — it
        // succeeds on the first try.
        pool.add_column("d".into(), &d, &device, &queue).unwrap();
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

        pool.add_column("a".into(), &small_a, &device, &queue)
            .unwrap();
        pool.add_column("b".into(), &small_b, &device, &queue)
            .unwrap();
        pool.add_column("c".into(), &small_c, &device, &queue)
            .unwrap();
        // free = [768..1024]. Remove b → free = [256..512] + [768..1024].
        pool.remove_column("b");
        // big needs 512 contiguous. First-fit fails (largest free is 256),
        // defrag fuses the hole and tail into [512..1024], retry succeeds.
        let res = pool.add_column("big".into(), &big, &device, &queue);
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

        pool.add_column("a".into(), &small_a, &device, &queue)
            .unwrap();
        pool.add_column("b".into(), &small_b, &device, &queue)
            .unwrap();
        pool.add_column("c".into(), &small_c, &device, &queue)
            .unwrap();
        pool.remove_column("b");
        // Manual policy: no retry → OutOfSpace.
        let res = pool.add_column("big".into(), &big, &device, &queue);
        assert!(matches!(res, Err(AllocError::OutOfSpace { .. })));
    }
}
