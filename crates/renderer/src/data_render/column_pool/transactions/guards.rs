//! The transaction guards: upsert, batch insert, streamed promotion, batch
//! sources and removal, with their commits and rollbacks.

use super::*;

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
    pub(in crate::data_render::column_pool) pool: &'a mut ColumnPool,
    pub(in crate::data_render::column_pool) rollback: Option<UpsertRollback>,
    pub(in crate::data_render::column_pool) handle: ColumnHandle,
    pub(in crate::data_render::column_pool) replaced_existing: bool,
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
            // Replacing an Option and dropping the superseded backups cannot
            // allocate or return an error; the drops retire their bytes in
            // the ledger.
            drop(old_backup.take());
            self.pool.backup = old_primary.take();
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
    pub(in crate::data_render::column_pool) pool: &'a mut ColumnPool,
    pub(in crate::data_render::column_pool) rollback: Option<BatchInsertRollback>,
}

/// One replayable streamed source being promoted into the resident pool.
///
/// The adapter is deliberately borrowed: promotion consumes one range write
/// at the call boundary and never stores host payload or trait objects. The
/// renderer freezes `len_values` during metadata validation; pool planning and
/// staging must not call the host trait's potentially mutable `len` again.
#[derive(Clone, Copy)]
pub(crate) struct StreamedColumnUpload<'a> {
    pub id: &'a str,
    pub source: StreamColumnSource<'a>,
    pub len_values: usize,
}

/// A provisionally uploaded streamed-to-resident batch.
///
/// A batch that already fits targets untouched free bytes in the live slab and
/// uses [`ColumnBatchInsert`] for exact allocator rollback. A batch requiring
/// compaction or growth targets a separate complete candidate slab instead, so
/// dropping this guard restores the original layout and buffers exactly.
#[must_use = "dropping an uncommitted streamed promotion leaves the live pool unchanged"]
pub(crate) enum StreamedColumnPromotion<'a> {
    InPlace(ColumnBatchInsert<'a>),
    Candidate(ColumnBatchCandidate<'a>),
}

impl StreamedColumnPromotion<'_> {
    pub(crate) fn pool(&self) -> &ColumnPool {
        match self {
            Self::InPlace(batch) => batch.pool(),
            Self::Candidate(batch) => batch.pool(),
        }
    }

    pub(crate) fn relocated(&self) -> bool {
        matches!(self, Self::Candidate(_))
    }

    pub(crate) fn commit(self) {
        match self {
            Self::InPlace(batch) => batch.commit(),
            Self::Candidate(batch) => batch.commit(),
        }
    }
}

pub(crate) struct ColumnBatchCandidate<'a> {
    pub(in crate::data_render::column_pool) pool: &'a mut ColumnPool,
    pub(in crate::data_render::column_pool) candidate: ColumnPool,
}

impl ColumnBatchCandidate<'_> {
    pub(in crate::data_render::column_pool) fn pool(&self) -> &ColumnPool {
        &self.candidate
    }

    pub(in crate::data_render::column_pool) fn commit(self) {
        let Self { pool, candidate } = self;
        let ColumnPool {
            identity: _,
            primary,
            capacity,
            slots,
            records: _,
            raggeds: _,
            triangles: _,
            free,
            generation,
            allocation_epochs,
            allocation_epoch_counter,
            layout_generation,
            // Both constructors build the candidate without a backup.
            backup: _,
            defrag_policy: _,
            growth_policy: _,
            expand: _,
        } = candidate;
        let old_primary = std::mem::replace(&mut pool.primary, primary);
        let old_capacity = pool.capacity;
        pool.capacity = capacity;
        pool.slots = slots;
        pool.free = free;
        pool.generation = generation;
        pool.allocation_epochs = allocation_epochs;
        pool.allocation_epoch_counter = allocation_epoch_counter;
        pool.layout_generation = layout_generation;

        // A superseded slab retires in the ledger when it drops.
        if old_capacity == capacity {
            pool.backup = Some(old_primary);
        } else {
            pool.backup = None;
        }
    }
}

pub(in crate::data_render::column_pool) struct BatchInsertRollback {
    pub(in crate::data_render::column_pool) region: FreeRegion,
    pub(in crate::data_render::column_pool) undo: RegionUndo,
    pub(in crate::data_render::column_pool) epoch_counter: u64,
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

/// A prepared batch transaction (#311, [`ColumnPool::begin_batch_upsert_sources`]):
/// the complete candidate pool. [`Self::commit`] moves it into the live pool;
/// dropping the guard drops the candidate and leaves the live pool as it was.
#[must_use = "dropping a ColumnSourcesBatch leaves the live pool unchanged"]
pub(crate) struct ColumnSourcesBatch<'a> {
    pub(in crate::data_render::column_pool) pool: &'a mut ColumnPool,
    pub(in crate::data_render::column_pool) candidate: ColumnPool,
}

impl ColumnSourcesBatch<'_> {
    /// The exact pool view that will be live after [`Self::commit`].
    pub(crate) fn pool(&self) -> &ColumnPool {
        &self.candidate
    }

    /// The candidate handles of the plain columns `ids`, in order.
    pub(crate) fn handles<const N: usize>(
        &self,
        ids: &[&str; N],
    ) -> Result<[ColumnHandle; N], AllocError> {
        let mut missing = false;
        let handles = std::array::from_fn(|k| match self.candidate.slots.get(ids[k]) {
            Some(slot) => ColumnHandle {
                generation: slot.generation,
                offset: slot.offset,
                byte_size: slot.byte_size,
                len_values: slot.len_values,
            },
            None => {
                missing = true;
                ColumnHandle {
                    generation: 0,
                    offset: 0,
                    byte_size: 0,
                    len_values: 0,
                }
            }
        });
        if missing {
            return Err(AllocError::Internal {
                invariant: "a batch prepares one slot per column",
            });
        }
        Ok(handles)
    }

    /// Publish the candidate: slab, layout, registries and counters.
    pub(crate) fn commit(self) {
        let Self { pool, candidate } = self;
        let ColumnPool {
            identity: _,
            primary,
            capacity,
            slots,
            records,
            raggeds,
            triangles,
            free,
            generation,
            allocation_epochs,
            allocation_epoch_counter,
            layout_generation,
            // The constructor builds the candidate without a backup.
            backup: _,
            defrag_policy: _,
            growth_policy: _,
            expand: _,
        } = candidate;
        let old_primary = std::mem::replace(&mut pool.primary, primary);
        let old_capacity = std::mem::replace(&mut pool.capacity, capacity);
        pool.slots = slots;
        pool.records = records;
        pool.raggeds = raggeds;
        pool.triangles = triangles;
        pool.free = free;
        pool.generation = generation;
        pool.allocation_epochs = allocation_epochs;
        pool.allocation_epoch_counter = allocation_epoch_counter;
        pool.layout_generation = layout_generation;
        // The superseded slab is the backup when it has the capacity; the
        // former backup, or a slab of another size, retires in the ledger
        // when it drops.
        pool.backup = (old_capacity == capacity).then_some(old_primary);
    }
}

/// The const-length plain-column form of [`ColumnSourcesBatch`], with the
/// columns' handles ([`ColumnPool::begin_batch_upsert`]); the pool tests'
/// form of the bundled demo's batch.
#[cfg(test)]
#[must_use = "dropping a ColumnBatchUpsert leaves the live pool unchanged"]
pub(crate) struct ColumnBatchUpsert<'a, const N: usize> {
    pub(in crate::data_render::column_pool) batch: ColumnSourcesBatch<'a>,
    pub(in crate::data_render::column_pool) handles: [ColumnHandle; N],
}

#[cfg(test)]
impl<const N: usize> ColumnBatchUpsert<'_, N> {
    pub(crate) fn pool(&self) -> &ColumnPool {
        self.batch.pool()
    }

    pub(crate) fn commit(self) -> [ColumnHandle; N] {
        self.batch.commit();
        self.handles
    }
}

pub(in crate::data_render::column_pool) struct RemovalRollback {
    pub(in crate::data_render::column_pool) slots: HashMap<ColumnId, ColumnSlot>,
    pub(in crate::data_render::column_pool) free: Vec<FreeRegion>,
    pub(in crate::data_render::column_pool) generation: u32,
    pub(in crate::data_render::column_pool) allocation_epochs: HashMap<ColumnId, u64>,
}

/// A provisionally-removed column.
///
/// The removed slot and allocator metadata are visible through [`Self::pool`]
/// so dependent GPU state can be prepared before publication. Committing only
/// releases the rollback snapshot; dropping restores the exact prior state.
#[must_use = "dropping a ColumnRemoval rolls the provisional removal back"]
pub struct ColumnRemoval<'a> {
    pub(in crate::data_render::column_pool) pool: &'a mut ColumnPool,
    pub(in crate::data_render::column_pool) rollback: Option<RemovalRollback>,
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
