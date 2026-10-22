//! Free regions, first-fit allocation, growth, relayout and defragmentation.

use super::*;

#[inline]
pub(super) fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) & !(a - 1)
}

#[inline]
pub(super) fn try_align_up(x: u64, a: u64) -> Option<u64> {
    x.checked_add(a - 1).map(|v| v & !(a - 1))
}

pub(super) fn out_of_space(requested: u64, free: &[FreeRegion]) -> AllocError {
    AllocError::OutOfSpace {
        requested,
        largest_free: free.iter().map(|region| region.size).max().unwrap_or(0),
        total_free: free.iter().map(|region| region.size).sum(),
    }
}

pub(super) fn alloc_region_from(free: &mut Vec<FreeRegion>, size: u64) -> Result<u64, AllocError> {
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

pub(super) fn coalesce_regions(free: &mut Vec<FreeRegion>) {
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

/// A free region. Adjacent regions are merged on coalesce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeRegion {
    pub offset: u64,
    pub size: u64,
}

pub(super) enum RegionReservationRollback {
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
pub(super) struct RegionReservation<'a> {
    pub(super) free: &'a mut Vec<FreeRegion>,
    pub(super) index: usize,
    pub(super) original: FreeRegion,
    pub(super) rollback: Option<RegionReservationRollback>,
}

/// The free-list edit `alloc_region` made, in undo form.
///
/// A transaction that outlives the reservation guard — the free list is one
/// field of the pool, and a guard holding the whole pool cannot also hold a
/// borrow of that field — takes this record instead and replays it if it has
/// to roll back.
pub(super) struct RegionUndo {
    pub(super) index: usize,
    pub(super) original: FreeRegion,
    pub(super) rollback: RegionReservationRollback,
}

impl RegionUndo {
    pub(super) fn apply(self, free: &mut Vec<FreeRegion>) {
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
    pub(super) fn offset(&self) -> u64 {
        self.original.offset
    }

    pub(super) fn commit(mut self) {
        self.rollback = None;
    }

    /// Defuse the guard and hand back its undo record.
    ///
    /// The free list stays edited; the caller becomes responsible for replaying
    /// the record if its own transaction fails.
    pub(super) fn into_undo(mut self) -> RegionUndo {
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

pub(super) fn alloc_region(
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
pub(super) fn column_raw_bytes(len_values: usize) -> u64 {
    (len_values as u64).saturating_mul(COLUMN_VALUE_BYTES as u64)
}

/// One column's padded footprint inside the pool, or `None` on overflow.
pub(super) fn column_region_bytes(len_values: usize) -> Option<u64> {
    try_align_up(column_raw_bytes(len_values), ALIGN)
}

/// Total region a batch occupies, with the per-column limit checks.
///
/// Each column is padded to `ALIGN`, so every per-column offset inside the
/// region is aligned without a second pass.
pub(super) fn batch_region_bytes(
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
pub(super) struct BatchChunk {
    pub(super) columns: std::ops::Range<usize>,
    pub(super) column_offsets: Vec<usize>,
    pub(super) region_offset: u64,
    pub(super) byte_size: u64,
}

pub(super) fn plan_batch_chunks(
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

pub(super) fn streamed_batch_region_bytes(
    columns: &[StreamedColumnUpload<'_>],
    ceiling: u64,
) -> Result<u64, AllocError> {
    let mut total = 0u64;
    for column in columns {
        let len_values = column.len_values;
        if len_values == 0 {
            return Err(AllocError::EmptySource);
        }
        let byte_size = column_region_bytes(len_values).ok_or(AllocError::ResourceLimit {
            resource: "streamed promotion staging buffer",
            requested: column_raw_bytes(len_values),
            limit: ceiling,
        })?;
        if byte_size > ceiling {
            return Err(AllocError::ResourceLimit {
                resource: "streamed promotion staging buffer",
                requested: byte_size,
                limit: ceiling,
            });
        }
        total = total
            .checked_add(byte_size)
            .ok_or(AllocError::ResourceLimit {
                resource: "streamed promotion batch",
                requested: u64::MAX,
                limit: ceiling,
            })?;
    }
    Ok(total)
}

pub(super) struct DefragmentRollback {
    pub(super) slots: Option<HashMap<ColumnId, ColumnSlot>>,
    pub(super) free: Vec<FreeRegion>,
    pub(super) generation: u32,
    pub(super) layout_generation: u64,
    pub(super) swapped_primary: bool,
    pub(super) candidate_was_backup: bool,
    /// Capacity to restore. A growth that rolls back must un-publish the new
    /// size along with the new buffer.
    pub(super) capacity: u64,
}

/// A provisionally-applied column-pool defragmentation.
///
/// GPU copies have already been submitted when a relocating guard is returned.
/// Returned preparation errors occur before the provisional pool is published;
/// asynchronous device failures remain governed by wgpu's device error model.
#[must_use = "dropping a ColumnDefragment rolls the provisional layout back"]
pub struct ColumnDefragment<'a> {
    pub(super) pool: &'a mut ColumnPool,
    pub(super) rollback: Option<DefragmentRollback>,
    pub(super) changed: bool,
    pub(super) relocated: bool,
    pub(super) legacy_result: bool,
    /// A resized candidate cannot reuse the old-capacity slab. On commit the
    /// old primary, parked in `backup` for rollback, is released.
    pub(super) resized: bool,
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
        if self.resized {
            // The old-capacity slab cannot serve a defrag at the new size.
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
            } else {
                self.pool.note_buffer_retired(candidate.size());
            }
        }
    }
}

impl ColumnPool {
    /// The biggest single region first fit could serve from.
    pub fn largest_free_region(&self) -> u64 {
        self.free
            .iter()
            .map(|region| region.size)
            .max()
            .unwrap_or(0)
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
    /// Equal size defragments; a different size grows or shrinks. Shrinking
    /// requires enough capacity for all live columns and uses GPU-internal
    /// copies, never a CPU data mirror.
    ///
    /// A size change drops `backup` before allocating, so the peak is the old
    /// primary plus the candidate rather than three slabs.
    pub fn begin_relayout(
        &mut self,
        ctx: GpuAllocCtx<'_>,
        target_capacity: u64,
    ) -> Result<ColumnDefragment<'_>, AllocError> {
        let (device, queue) = (ctx.device, ctx.queue);
        let capacity = self.plan_relayout_capacity(ctx, target_capacity)?;
        let previous_capacity = self.capacity;
        let resized = capacity != previous_capacity;
        // Empty pool with no size change: just normalize the free list.
        if self.slots.is_empty() && !resized {
            let already =
                self.free.len() == 1 && self.free[0].offset == 0 && self.free[0].size == capacity;
            if already {
                return Ok(ColumnDefragment {
                    pool: self,
                    rollback: None,
                    changed: false,
                    relocated: false,
                    legacy_result: false,
                    resized: false,
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
                resized: false,
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
        if already_packed && !resized {
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
                    resized: false,
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
                resized: false,
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

        // A resized pool cannot reuse a backup at the old capacity.
        if resized && let Some(stale_backup) = self.backup.take() {
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
            resized,
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
    /// With growth disabled this is a no-op; opted-in growth reports the
    /// actual budget, device, or allocation failure to its caller.
    pub(super) fn grow_for_upsert_if_needed(
        &mut self,
        id: &str,
        byte_size: u64,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<(), AllocError> {
        if self.growth_policy != GrowthPolicy::OnAllocFailure {
            return Ok(());
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
            return Ok(());
        }
        let deficit = byte_size - usable;
        self.grow_for_pending_upload(ctx, deficit)
    }

    /// Enlarge enough to hold `needed_bytes` more, preferring to double.
    ///
    /// Doubling keeps repeated uploads from relaying out on every column; the
    /// device ceiling caps it, and the result is never below what the pending
    /// upload actually needs. `plan_relayout_capacity` still has the final say
    /// on ceiling and budget.
    pub(super) fn grow_for_pending_upload(
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
    pub(super) fn plan_relayout_capacity(
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
        let resource = if target < self.capacity {
            "column pool shrink"
        } else {
            "column pool growth"
        };
        if target < self.used_bytes().max(ALIGN) {
            return Err(AllocError::OutOfSpace {
                requested: self.used_bytes(),
                largest_free: target,
                total_free: target,
            });
        }
        if target == self.capacity {
            return Ok(target);
        }
        if target > ceiling {
            return Err(AllocError::ResourceLimit {
                resource,
                requested: target,
                limit: ceiling,
            });
        }
        if let Some(budget) = ctx.budget {
            // Released buffers, including a dropped backup, remain resident
            // until queue completion and cannot finance the new allocation.
            let peak = budget
                .external_bytes
                .checked_add(self.gpu_bytes())
                .and_then(|sum| sum.checked_add(self.retired_bytes()))
                .and_then(|sum| sum.checked_add(target))
                .ok_or(AllocError::BudgetExceeded {
                    resource,
                    requested: u64::MAX,
                    limit: budget.ceiling_bytes,
                })?;
            if peak > budget.ceiling_bytes {
                return Err(AllocError::BudgetExceeded {
                    resource,
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

    /// Sort `free` by offset and merge adjacent regions.
    pub(super) fn coalesce_free(free: &mut Vec<FreeRegion>) {
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
