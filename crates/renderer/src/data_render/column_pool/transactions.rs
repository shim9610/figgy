//! Uploads and the explicit insert, upsert, promotion and removal transactions
//! with their rollbacks.

use super::*;

/// Invoke the fused source capability once while the write-only view is
/// live, measuring the values' uniform step (`UniformStep`, #134) as they
/// are written.
pub(super) fn write_staging_pairs(
    staging: &Buffer,
    raw_bytes: u64,
    write_pairs: impl FnOnce(ColumnPairWriter<'_>) -> ColumnUploadStats,
) -> (ColumnUploadStats, Option<f64>) {
    let mut view = staging
        .slice(0..raw_bytes)
        .get_mapped_range_mut()
        .expect("column staging is mapped at creation");
    let mut step = crate::data::UniformStep::new();
    let stats = {
        let mut observe = |index: usize, hi: f32, lo: f32| step.record(index, hi, lo);
        write_pairs(ColumnPairWriter::new_observed(view.slice(..), &mut observe))
    };
    drop(view);
    staging.unmap();
    (stats, step.finish())
}

/// Source metadata captured before an upload is prepared.
///
/// This deliberately excludes allocator state and encoded-value statistics,
/// which belong to [`ColumnSlot`] and [`ColumnUploadStats`] respectively.
pub(super) struct ColumnInputMeta {
    pub(super) id: ColumnId,
    pub(super) len_values: usize,
    pub(super) min: f64,
    pub(super) max: f64,
}

pub(super) fn write_scalar_source_as_pairs(
    source: &dyn ColumnSource,
    dst: ColumnPairWriter<'_>,
) -> ColumnUploadStats {
    source.write_f32_pair_le_into_with_stats(dst)
}

#[derive(Clone, Copy)]
pub(super) struct StagedStreamedColumn {
    pub(super) offset: u64,
    pub(super) byte_size: u64,
    pub(super) len_values: usize,
    pub(super) min: f64,
    pub(super) max: f64,
    pub(super) min_positive: Option<f64>,
}

pub(super) fn stage_streamed_columns(
    device: &Device,
    columns: &[StreamedColumnUpload<'_>],
    total: u64,
) -> Result<(Buffer, Vec<StagedStreamedColumn>), AllocError> {
    let staging = create_buffer_checked(
        device,
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
        .expect("streamed promotion staging is mapped at creation");
    let mut offset = 0u64;
    let written = (|| {
        for column in columns {
            let len_values = column.len_values;
            let raw_bytes = column_raw_bytes(len_values);
            let byte_size = column_region_bytes(len_values).expect("promotion batch was sized");
            let start = offset as usize;
            let end = start + raw_bytes as usize;
            let bounds = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                column
                    .source
                    .write_range(0, ColumnPairWriter::new(view.slice(start..end)))
            }))
            .map_err(|_| AllocError::AllocationFailed {
                resource: "streamed promotion source",
                reason: "source panicked while writing the requested range".into(),
            })?
            .map_err(|error| AllocError::AllocationFailed {
                resource: "streamed promotion source",
                reason: error.reason().to_owned(),
            })?;
            crate::streaming_source::validate_statistics(
                len_values as u64,
                crate::StreamStatistics::Known(bounds),
            )
            .map_err(|reason| AllocError::AllocationFailed {
                resource: "streamed promotion source",
                reason: reason.to_owned(),
            })?;
            let (min, max, min_positive) = bounds
                .map_or((f64::INFINITY, f64::NEG_INFINITY, None), |bounds| {
                    (bounds.min, bounds.max, bounds.min_positive)
                });
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

pub(super) enum UpsertBufferRollback {
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

pub(super) struct UpsertRollback {
    pub(super) slots: HashMap<ColumnId, ColumnSlot>,
    pub(super) free: Vec<FreeRegion>,
    pub(super) generation: u32,
    pub(super) allocation_epochs: HashMap<ColumnId, u64>,
    pub(super) allocation_epoch_counter: u64,
    pub(super) layout_generation: u64,
    pub(super) buffer: UpsertBufferRollback,
}

pub(super) enum PreparedUpsertTarget {
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
    pub(super) pool: &'a mut ColumnPool,
    pub(super) rollback: Option<UpsertRollback>,
    pub(super) handle: ColumnHandle,
    pub(super) replaced_existing: bool,
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
    pub(super) pool: &'a mut ColumnPool,
    pub(super) rollback: Option<BatchInsertRollback>,
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
    pub(super) pool: &'a mut ColumnPool,
    pub(super) candidate: Option<ColumnPool>,
}

impl ColumnBatchCandidate<'_> {
    pub(super) fn pool(&self) -> &ColumnPool {
        self.candidate
            .as_ref()
            .expect("streamed promotion candidate remains live")
    }

    pub(super) fn commit(mut self) {
        let candidate = self
            .candidate
            .take()
            .expect("streamed promotion candidate remains live");
        let ColumnPool {
            identity: _,
            primary,
            capacity,
            slots,
            records: _,
            raggeds: _,
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
        let old_capacity = self.pool.capacity;
        self.pool.capacity = capacity;
        self.pool.slots = slots;
        self.pool.free = free;
        self.pool.generation = generation;
        self.pool.allocation_epochs = allocation_epochs;
        self.pool.allocation_epoch_counter = allocation_epoch_counter;
        self.pool.layout_generation = layout_generation;

        if old_capacity == capacity {
            if let Some(superseded) = self.pool.backup.replace(old_primary) {
                self.pool.note_buffer_retired(superseded.size());
            }
        } else {
            let mut released = old_primary.size();
            released =
                released.saturating_add(self.pool.backup.take().map_or(0, |buffer| buffer.size()));
            self.pool.note_buffer_retired(released);
        }
    }
}

impl Drop for ColumnBatchCandidate<'_> {
    fn drop(&mut self) {
        let Some(candidate) = self.candidate.take() else {
            return;
        };
        self.pool.note_buffer_retired(candidate.primary.size());
    }
}

pub(super) struct BatchInsertRollback {
    pub(super) region: FreeRegion,
    pub(super) undo: RegionUndo,
    pub(super) epoch_counter: u64,
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
                self.pool.note_buffer_retired(candidate.size());
            }
        }
    }
}

#[must_use = "dropping a ColumnBatchUpsert leaves the live pool unchanged"]
pub(crate) struct ColumnBatchUpsert<'a> {
    pub(super) pool: &'a mut ColumnPool,
    pub(super) candidate: Option<ColumnPool>,
    pub(super) handles: [ColumnHandle; 4],
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
            records: _,
            raggeds: _,
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
        if let Some(superseded) = self.pool.backup.replace(old_primary) {
            self.pool.note_buffer_retired(superseded.size());
        }
        self.handles
    }
}

impl Drop for ColumnBatchUpsert<'_> {
    fn drop(&mut self) {
        let Some(candidate) = self.candidate.take() else {
            return;
        };
        self.pool.note_buffer_retired(candidate.primary.size());
    }
}

pub(super) struct RemovalRollback {
    pub(super) slots: HashMap<ColumnId, ColumnSlot>,
    pub(super) free: Vec<FreeRegion>,
    pub(super) generation: u32,
    pub(super) allocation_epochs: HashMap<ColumnId, u64>,
}

/// A provisionally-removed column.
///
/// The removed slot and allocator metadata are visible through [`Self::pool`]
/// so dependent GPU state can be prepared before publication. Committing only
/// releases the rollback snapshot; dropping restores the exact prior state.
#[must_use = "dropping a ColumnRemoval rolls the provisional removal back"]
pub struct ColumnRemoval<'a> {
    pub(super) pool: &'a mut ColumnPool,
    pub(super) rollback: Option<RemovalRollback>,
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

impl ColumnPool {
    /// Pure pool half of resident admission. It mirrors the batch insertion
    /// ladder (first-fit, optional compaction, optional growth) without
    /// allocating, submitting, defragmenting, or changing allocator state.
    pub(crate) fn plan_resident_batch_capacity(
        &self,
        required_bytes: u64,
        device_ceiling: u64,
    ) -> Result<ResidentBatchCapacityPlan, ResidentBatchCapacityError> {
        if required_bytes == 0 || self.largest_free_region() >= required_bytes {
            return Ok(ResidentBatchCapacityPlan {
                capacity_after: self.capacity,
                transition_bytes: 0,
            });
        }

        let may_compact = self.defrag_policy == DefragPolicy::OnAllocFailure;
        if may_compact && self.free_bytes() >= required_bytes {
            let reusable_backup = self
                .backup
                .as_ref()
                .is_some_and(|buffer| buffer.size() == self.capacity);
            return Ok(ResidentBatchCapacityPlan {
                capacity_after: self.capacity,
                transition_bytes: (!reusable_backup).then_some(self.capacity).unwrap_or(0),
            });
        }
        if self.growth_policy != GrowthPolicy::OnAllocFailure {
            return Err(ResidentBatchCapacityError::Policy);
        }

        // Keep this arithmetic identical to `grow_for_pending_upload`: when
        // compaction is disabled, its first-fit deficit is measured from the
        // largest region even though growth itself packs the complete pool.
        let usable = if may_compact {
            self.free_bytes()
        } else {
            self.largest_free_region()
        };
        let deficit = required_bytes.saturating_sub(usable);
        let minimum = self
            .capacity
            .checked_add(deficit)
            .ok_or(ResidentBatchCapacityError::Overflow)?;
        let doubled = self.capacity.checked_mul(2).unwrap_or(u64::MAX);
        let requested = doubled.max(minimum).min(device_ceiling).max(minimum);
        let target = try_align_up(requested, ALIGN).ok_or(ResidentBatchCapacityError::Overflow)?;
        if target > device_ceiling {
            return Err(ResidentBatchCapacityError::DeviceLimit);
        }
        Ok(ResidentBatchCapacityPlan {
            capacity_after: target,
            transition_bytes: target,
        })
    }

    /// Prepare a same-id streamed-to-resident promotion without publishing any
    /// live allocator state. Sources may mix scalar and hi/lo encodings.
    ///
    /// `target_capacity` comes from the renderer's immediately preceding
    /// resident-admission check. It is used only when the batch cannot fit an
    /// untouched free region and a complete candidate slab is required.
    pub(crate) fn begin_streamed_promotion<'a>(
        &'a mut self,
        columns: &[StreamedColumnUpload<'_>],
        target_capacity: u64,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<StreamedColumnPromotion<'a>, AllocError> {
        if columns.is_empty() {
            return Ok(StreamedColumnPromotion::InPlace(ColumnBatchInsert {
                pool: self,
                rollback: None,
            }));
        }
        let ceiling = buffer_ceiling(ctx.device);

        let mut seen = std::collections::HashSet::new();
        seen.try_reserve(columns.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "streamed promotion id set",
                reason: error.to_string(),
            })?;
        for column in columns {
            if self.slots.contains_key(column.id) || !seen.insert(column.id) {
                return Err(AllocError::DuplicateId(column.id.to_owned()));
            }
        }

        let total = streamed_batch_region_bytes(columns, ceiling)?;
        let in_place = self.largest_free_region() >= total;
        let capacity_after = if in_place {
            self.capacity
        } else {
            let planned = self
                .plan_resident_batch_capacity(total, ceiling)
                .map_err(|error| match error {
                    ResidentBatchCapacityError::Policy => out_of_space(total, &self.free),
                    ResidentBatchCapacityError::DeviceLimit => AllocError::ResourceLimit {
                        resource: "streamed promotion candidate",
                        requested: target_capacity,
                        limit: ceiling,
                    },
                    ResidentBatchCapacityError::Overflow => AllocError::ResourceLimit {
                        resource: "streamed promotion candidate",
                        requested: u64::MAX,
                        limit: ceiling,
                    },
                })?;
            if planned.capacity_after != target_capacity {
                return Err(AllocError::AllocationFailed {
                    resource: "streamed promotion admission",
                    reason: "pool capacity changed after admission".into(),
                });
            }
            planned.capacity_after
        };

        if !in_place {
            let required =
                self.used_bytes()
                    .checked_add(total)
                    .ok_or(AllocError::ResourceLimit {
                        resource: "streamed promotion candidate",
                        requested: u64::MAX,
                        limit: capacity_after,
                    })?;
            if required > capacity_after || capacity_after > ceiling {
                return Err(AllocError::ResourceLimit {
                    resource: "streamed promotion candidate",
                    requested: required.max(capacity_after),
                    limit: ceiling.min(capacity_after),
                });
            }
            if let Some(budget) = ctx.budget {
                let peak = budget
                    .external_bytes
                    .checked_add(self.gpu_bytes())
                    .and_then(|bytes| bytes.checked_add(self.retired_bytes()))
                    .and_then(|bytes| bytes.checked_add(total))
                    .and_then(|bytes| bytes.checked_add(capacity_after))
                    .ok_or(AllocError::BudgetExceeded {
                        resource: "streamed promotion transition",
                        requested: u64::MAX,
                        limit: budget.ceiling_bytes,
                    })?;
                if peak > budget.ceiling_bytes {
                    return Err(AllocError::BudgetExceeded {
                        resource: "streamed promotion transition",
                        requested: peak,
                        limit: budget.ceiling_bytes,
                    });
                }
            }
        }

        let first_epoch = self.allocation_epoch_counter;
        let count = u64::try_from(columns.len()).map_err(|_| AllocError::CounterExhausted {
            counter: "allocation epoch",
        })?;
        let last_epoch = first_epoch
            .checked_add(count)
            .ok_or(AllocError::CounterExhausted {
                counter: "allocation epoch",
            })?;
        let (staging, staged) = stage_streamed_columns(ctx.device, columns, total)?;

        if in_place {
            self.slots.try_reserve(columns.len()).map_err(|error| {
                AllocError::AllocationFailed {
                    resource: "streamed promotion column registry",
                    reason: error.to_string(),
                }
            })?;
            self.allocation_epochs
                .try_reserve(columns.len())
                .map_err(|error| AllocError::AllocationFailed {
                    resource: "streamed promotion epoch registry",
                    reason: error.to_string(),
                })?;
            let reservation = alloc_region(&mut self.free, total)?;
            let region_offset = reservation.offset();
            let mut epoch = first_epoch;
            for (column, staged) in columns.iter().zip(&staged) {
                epoch += 1;
                let id = column.id.to_owned();
                self.allocation_epochs.insert(id.clone(), epoch);
                self.slots.insert(
                    id.clone(),
                    ColumnSlot {
                        id,
                        offset: region_offset + staged.offset,
                        byte_size: staged.byte_size,
                        len_values: staged.len_values,
                        generation: self.generation,
                        min: staged.min,
                        max: staged.max,
                        min_positive: staged.min_positive,
                        uniform_step: None,
                    },
                );
            }
            let mut encoder = ctx
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy streamed promotion in-place"),
                });
            encoder.copy_buffer_to_buffer(&staging, 0, &self.primary, region_offset, total);
            ctx.queue.submit(std::iter::once(encoder.finish()));
            let undo = reservation.into_undo();
            self.allocation_epoch_counter = last_epoch;
            self.note_buffer_created(total);
            self.note_buffer_retired(total);
            return Ok(StreamedColumnPromotion::InPlace(ColumnBatchInsert {
                pool: self,
                rollback: Some(BatchInsertRollback {
                    region: FreeRegion {
                        offset: region_offset,
                        size: total,
                    },
                    undo,
                    epoch_counter: first_epoch,
                }),
            }));
        }

        let next_generation = self.checked_generation_successor()?;
        let next_layout_generation = self.checked_layout_successor()?;
        let final_count =
            self.slots
                .len()
                .checked_add(columns.len())
                .ok_or(AllocError::AllocationFailed {
                    resource: "streamed promotion column registry",
                    reason: "column count overflow".into(),
                })?;
        let mut order = Vec::new();
        order.try_reserve_exact(self.slots.len()).map_err(|error| {
            AllocError::AllocationFailed {
                resource: "streamed promotion survivor order",
                reason: error.to_string(),
            }
        })?;
        order.extend(self.slots.values().map(|slot| slot.id.clone()));
        order.sort_by_key(|id| self.slots[id].offset);

        let mut planned_slots = HashMap::new();
        planned_slots
            .try_reserve(final_count)
            .map_err(|error| AllocError::AllocationFailed {
                resource: "streamed promotion column registry",
                reason: error.to_string(),
            })?;
        let mut planned_epochs = HashMap::new();
        planned_epochs
            .try_reserve(final_count)
            .map_err(|error| AllocError::AllocationFailed {
                resource: "streamed promotion epoch registry",
                reason: error.to_string(),
            })?;
        let mut next_offset = 0u64;
        for id in &order {
            let old = &self.slots[id];
            let mut slot = old.clone();
            slot.offset = next_offset;
            slot.generation = next_generation;
            next_offset =
                next_offset
                    .checked_add(slot.byte_size)
                    .ok_or(AllocError::ResourceLimit {
                        resource: "streamed promotion candidate",
                        requested: u64::MAX,
                        limit: capacity_after,
                    })?;
            planned_epochs.insert(
                id.clone(),
                *self
                    .allocation_epochs
                    .get(id)
                    .expect("live column has an allocation epoch"),
            );
            planned_slots.insert(id.clone(), slot);
        }
        let promoted_offset = next_offset;
        let mut epoch = first_epoch;
        for (column, staged) in columns.iter().zip(&staged) {
            epoch += 1;
            let id = column.id.to_owned();
            let offset = promoted_offset + staged.offset;
            planned_epochs.insert(id.clone(), epoch);
            planned_slots.insert(
                id.clone(),
                ColumnSlot {
                    id,
                    offset,
                    byte_size: staged.byte_size,
                    len_values: staged.len_values,
                    generation: next_generation,
                    min: staged.min,
                    max: staged.max,
                    min_positive: staged.min_positive,
                    uniform_step: None,
                },
            );
        }
        next_offset = promoted_offset
            .checked_add(total)
            .ok_or(AllocError::ResourceLimit {
                resource: "streamed promotion candidate",
                requested: u64::MAX,
                limit: capacity_after,
            })?;
        if next_offset > capacity_after {
            return Err(out_of_space(total, &self.free));
        }
        let mut planned_free = Vec::new();
        if next_offset < capacity_after {
            planned_free
                .try_reserve_exact(1)
                .map_err(|error| AllocError::AllocationFailed {
                    resource: "streamed promotion free list",
                    reason: error.to_string(),
                })?;
            planned_free.push(FreeRegion {
                offset: next_offset,
                size: capacity_after - next_offset,
            });
        }

        let primary = create_buffer_checked(
            ctx.device,
            &BufferDescriptor {
                label: Some("figgy streamed promotion candidate"),
                size: capacity_after,
                usage: BufferUsages::VERTEX
                    | BufferUsages::STORAGE
                    | BufferUsages::COPY_DST
                    | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "streamed promotion candidate",
        )?;
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy streamed promotion candidate"),
            });
        for id in &order {
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
        encoder.copy_buffer_to_buffer(&staging, 0, &primary, promoted_offset, total);
        ctx.queue.submit(std::iter::once(encoder.finish()));
        self.note_buffer_created(total);
        self.note_buffer_created(total.saturating_add(capacity_after));
        self.note_buffer_retired(total);

        let candidate = ColumnPool {
            identity: self.identity.clone(),
            primary,
            capacity: capacity_after,
            slots: planned_slots,
            records: HashMap::new(),
            raggeds: HashMap::new(),
            free: planned_free,
            generation: next_generation,
            allocation_epochs: planned_epochs,
            allocation_epoch_counter: last_epoch,
            layout_generation: next_layout_generation,
            backup: None,
            defrag_policy: self.defrag_policy,
            growth_policy: self.growth_policy,
            retired_bytes: Arc::default(),
            peak_bytes: 0,
            buffer_creations: 0,
        };
        Ok(StreamedColumnPromotion::Candidate(ColumnBatchCandidate {
            pool: self,
            candidate: Some(candidate),
        }))
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
        self.begin_add_columns_observed(columns, ctx, None, None)
    }

    /// [`Self::begin_add_columns`] with an observer of every written pair:
    /// `observe(column, row, hi, lo)`, `column` being the index into
    /// `columns`. Records and ragged columns measure their statistics through
    /// it while the staging write happens. `index_column` names the column
    /// whose pairs are stored as `u32` index lanes
    /// ([`ColumnPairWriter::with_index_lanes`]): a ragged column's offsets.
    pub(super) fn begin_add_columns_observed<'a>(
        &'a mut self,
        columns: &[(&str, &dyn ColumnSource)],
        ctx: GpuAllocCtx<'_>,
        mut observe: Option<&mut dyn FnMut(usize, usize, f32, f32)>,
        index_column: Option<usize>,
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
        //    here instead; an opted-in growth reports its precise failure.
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
                let dst = view.slice(local..local + raw_bytes as usize);
                let index_lanes = index_column == Some(index);
                let mut step = crate::data::UniformStep::new();
                let stats = if let Some(observe) = observe.as_mut() {
                    let mut per_column = |row: usize, hi: f32, lo: f32| {
                        step.record(row, hi, lo);
                        (*observe)(index, row, hi, lo);
                    };
                    write_scalar_source_as_pairs(
                        source,
                        ColumnPairWriter::new_observed(dst, &mut per_column)
                            .with_index_lanes(index_lanes),
                    )
                } else {
                    let mut per_column = |row: usize, hi: f32, lo: f32| step.record(row, hi, lo);
                    write_scalar_source_as_pairs(
                        source,
                        ColumnPairWriter::new_observed(dst, &mut per_column)
                            .with_index_lanes(index_lanes),
                    )
                };
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
                        uniform_step: step.finish(),
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
        // CPU handles are dropped on return, but the submitted copy keeps the
        // staging allocation device-resident until queue completion.
        self.note_buffer_retired(total);
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
    /// [`GrowthPolicy::OnAllocFailure`]. Fixed-capacity allocation reports
    /// `OutOfSpace`; an opted-in growth reports its own failure category.
    pub(super) fn make_room_for_batch(
        &mut self,
        total: u64,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<(), AllocError> {
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
        self.grow_for_pending_upload(ctx, total.saturating_sub(usable))?;
        Ok(())
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
            uniform_step: Option<f64>,
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
            let (stats, uniform_step) = write_staging_pairs(&staging, raw_bytes, |dst| {
                write_scalar_source_as_pairs(source, dst)
            });
            staged.push(StagedColumn {
                id,
                len_values,
                byte_size,
                min: source.min(),
                max: source.max(),
                min_positive: stats.min_positive,
                uniform_step,
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
                uniform_step: column.uniform_step,
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
        self.note_buffer_retired(staged_bytes);

        let handles: [ColumnHandle; 4] = handles
            .try_into()
            .expect("demo batch always prepares exactly four handles");
        let candidate = ColumnPool {
            identity: self.identity.clone(),
            primary,
            capacity: self.capacity,
            slots: planned_slots,
            records: HashMap::new(),
            raggeds: HashMap::new(),
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
            retired_bytes: Arc::default(),
            peak_bytes: 0,
            buffer_creations: 0,
        };
        Ok(ColumnBatchUpsert {
            pool: self,
            candidate: Some(candidate),
            handles,
        })
    }

    pub(super) fn begin_upsert_column_pairs_with<'a>(
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
        // pure `&self` fit check decides. Opted-in growth failures preserve
        // their budget/device/allocation category for the host.
        self.grow_for_upsert_if_needed(&id, byte_size, ctx)?;
        let allocation_epoch = self.checked_allocation_epoch_successor()?;

        // Complete every source-dependent/fallible staging operation before
        // touching allocator metadata or the live primary buffer.
        let staging = create_staging(byte_size)?;
        self.note_buffer_created(byte_size);
        let (stats, uniform_step) = write_staging_pairs(&staging, raw_bytes, write_pairs);
        let min_positive = stats.min_positive;

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
            uniform_step,
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
        self.note_buffer_retired(byte_size);

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
    pub(super) fn try_add_column(
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

    pub(super) fn try_add_hilo_column(
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

    pub(super) fn try_add_column_pairs(
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

    pub(super) fn try_add_column_pairs_with(
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
        let (stats, uniform_step) = write_staging_pairs(&staging, raw_bytes, write_pairs);
        let min_positive = stats.min_positive;

        let slot = ColumnSlot {
            id: id.clone(),
            offset: region_offset,
            byte_size,
            len_values: n,
            generation: self.generation,
            min,
            max,
            min_positive,
            uniform_step,
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
        self.note_buffer_retired(byte_size);
        Ok(handle)
    }

    /// Provisionally remove a column so dependent GPU state can be prepared.
    pub fn begin_remove_column(
        &mut self,
        id: &str,
    ) -> Result<Option<ColumnRemoval<'_>>, AllocError> {
        if !self.slots.contains_key(id) {
            return Ok(None);
        }
        Ok(Some(self.begin_remove_columns(&[id])?))
    }

    /// Provisionally remove several resident columns as one allocator change.
    ///
    /// Every id must be present and distinct. The returned pool view contains
    /// none of them, allowing dependent state to be prepared before commit;
    /// dropping the guard restores every slot, free region, allocation epoch,
    /// and public generation exactly.
    pub(crate) fn begin_remove_columns<'a>(
        &'a mut self,
        ids: &[&str],
    ) -> Result<ColumnRemoval<'a>, AllocError> {
        if ids.is_empty() {
            return Ok(ColumnRemoval {
                pool: self,
                rollback: None,
            });
        }
        let mut seen = std::collections::HashSet::new();
        seen.try_reserve(ids.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "column batch removal id set",
                reason: error.to_string(),
            })?;
        for id in ids {
            if !seen.insert(*id) {
                return Err(AllocError::DuplicateId((*id).to_owned()));
            }
            if !self.slots.contains_key(*id) {
                return Err(AllocError::AllocationFailed {
                    resource: "column batch removal",
                    reason: "resident column disappeared before removal".into(),
                });
            }
        }

        let next_generation = self.checked_generation_successor()?;
        let mut planned_slots = self.slots.clone();
        let mut removed_regions = Vec::new();
        removed_regions
            .try_reserve_exact(ids.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "column batch removal regions",
                reason: error.to_string(),
            })?;
        for id in ids {
            let slot = planned_slots
                .remove(*id)
                .expect("validated resident column remains in removal plan");
            removed_regions.push(FreeRegion {
                offset: slot.offset,
                size: slot.byte_size,
            });
        }
        for slot in planned_slots.values_mut() {
            slot.generation = next_generation;
        }
        let mut planned_free = self.free.clone();
        planned_free
            .try_reserve(removed_regions.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "column batch removal free list",
                reason: error.to_string(),
            })?;
        planned_free.extend(removed_regions);
        Self::coalesce_free(&mut planned_free);
        let mut planned_allocation_epochs = self.allocation_epochs.clone();
        for id in ids {
            planned_allocation_epochs.remove(*id);
        }

        let old_slots = std::mem::replace(&mut self.slots, planned_slots);
        let old_free = std::mem::replace(&mut self.free, planned_free);
        let old_allocation_epochs =
            std::mem::replace(&mut self.allocation_epochs, planned_allocation_epochs);
        let old_generation = std::mem::replace(&mut self.generation, next_generation);

        Ok(ColumnRemoval {
            pool: self,
            rollback: Some(RemovalRollback {
                slots: old_slots,
                free: old_free,
                generation: old_generation,
                allocation_epochs: old_allocation_epochs,
            }),
        })
    }
}

/// Scalar statistics of one member column, measured from the encoded pairs
/// in the write observer.
#[derive(Clone, Copy)]
struct ValueTally {
    min: f64,
    max: f64,
    finite: u32,
    nan: u32,
}

impl ValueTally {
    const EMPTY: Self = Self {
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        finite: 0,
        nan: 0,
    };

    fn record(&mut self, hi: f32, lo: f32) {
        let value = hi as f64 + lo as f64;
        if value.is_nan() {
            self.nan = self.nan.saturating_add(1);
        } else if value.is_finite() && (hi + lo).is_finite() {
            self.finite = self.finite.saturating_add(1);
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        }
    }
}

/// The offsets checks of a ragged column, measured in the write observer in
/// one pass: no value is kept beyond the previous one.
struct OffsetTally {
    next_row: usize,
    in_order: bool,
    integral: bool,
    non_decreasing: bool,
    first: f64,
    previous: f64,
    max_group: f64,
}

impl OffsetTally {
    fn record(&mut self, row: usize, hi: f32, lo: f32) {
        let value = hi as f64 + lo as f64;
        if row != self.next_row {
            self.in_order = false;
        }
        self.next_row = row.saturating_add(1);
        if !(value.is_finite() && value >= 0.0 && value.fract() == 0.0) {
            self.integral = false;
        }
        if row == 0 {
            self.first = value;
        } else if value >= self.previous {
            self.max_group = self.max_group.max(value - self.previous);
        } else {
            self.non_decreasing = false;
        }
        self.previous = value;
    }
}

fn composite_error(id: &str, reason: impl Into<String>) -> crate::FiggyError {
    crate::FiggyError::InvalidSeriesConfig {
        series_id: id.to_owned(),
        reason: reason.into(),
    }
}

/// Remove `old` member columns inside a provisional member batch, let the
/// caller prepare dependent state against the final pool, and publish both.
/// Any failure drops the removal (restoring the old members) and then the
/// batch (un-inserting the new ones): the new members were written to
/// regions that were free before the batch, so the old bytes are untouched.
fn finish_member_batch<T>(
    batch: ColumnBatchInsert<'_>,
    old: &[&str],
    prepare: impl FnOnce(&ColumnPool) -> crate::Result<T>,
) -> crate::Result<T> {
    let removal = batch.pool.begin_remove_columns(old)?;
    let value = prepare(&*removal.pool)?;
    removal.commit();
    batch.commit();
    Ok(value)
}

impl ColumnPool {
    /// The record `id`, when one is resident.
    pub fn record(&self, id: &str) -> Option<RecordRef<'_>> {
        self.records
            .get(id)
            .map(|slot| RecordRef { pool: self, slot })
    }

    /// The ragged column `id`, when one is resident.
    pub fn ragged(&self, id: &str) -> Option<RaggedRef<'_>> {
        self.raggeds
            .get(id)
            .map(|slot| RaggedRef { pool: self, slot })
    }

    /// The scalar statistics of the record `id`: its row count and, per
    /// field, presence, finite range, finite count and NaN count.
    pub fn record_stats(&self, id: &str) -> Option<RecordStats> {
        self.records.get(id).map(|slot| slot.stats)
    }

    /// The scalar statistics of the ragged column `id`.
    pub fn ragged_stats(&self, id: &str) -> Option<RaggedStats> {
        self.raggeds.get(id).map(|slot| slot.stats)
    }

    /// What `id` names when it is a record, a ragged column or one of their
    /// member columns, for the renderer's id checks.
    pub(crate) fn composite_kind(&self, id: &str) -> Option<&'static str> {
        if self.records.contains_key(id) {
            Some("a record")
        } else if self.raggeds.contains_key(id) {
            Some("a ragged column")
        } else if self.is_member_column(id) {
            Some("a member column of a record or ragged column")
        } else {
            None
        }
    }

    /// Whether `id` is a member column of a resident record or ragged column.
    pub(crate) fn is_member_column(&self, id: &str) -> bool {
        let Some((owner, _)) = id.split_once(MEMBER_SEPARATOR) else {
            return false;
        };
        if let Some(record) = self.records.get(owner) {
            return record.fields.iter().flatten().any(|field| field == id);
        }
        self.raggeds
            .get(owner)
            .is_some_and(|ragged| ragged.values == id || ragged.offsets == id)
    }

    fn check_composite_id(&self, id: &str, kind: &str) -> crate::Result<()> {
        if id.is_empty() {
            return Err(composite_error(
                id,
                format!("a {kind} id must not be empty"),
            ));
        }
        if id.contains(MEMBER_SEPARATOR) {
            return Err(composite_error(
                id,
                format!("a {kind} id must not contain U+001F, which separates member column ids"),
            ));
        }
        if self.slots.contains_key(id) {
            return Err(composite_error(
                id,
                format!(
                    "the id is a column; columns, records and ragged columns share one id \
                     space, so a {kind} needs an id of its own"
                ),
            ));
        }
        Ok(())
    }

    /// The token that keeps a replacement's member ids distinct from the
    /// members it replaces: the allocation epoch its first member will get.
    fn member_token(&self) -> crate::Result<u64> {
        Ok(self.checked_allocation_epoch_successor()?)
    }

    /// Insert or replace the record `id` (failure-atomic).
    ///
    /// The schema is checked by name (an unknown, repeated or missing
    /// required field is an error), every field must have the same non-zero
    /// length, and a record holds at most [`MAX_RECORD_FIELDS`] fields. The
    /// fields upload as one batch transaction ([`Self::begin_add_columns`]):
    /// one region, one staging buffer, one copy; the write observer measures
    /// the [`RecordStats`] on the way. A replacement uploads the new fields
    /// while the old ones are still live, so it needs room for both — the
    /// batch's own ladder (first fit, then compaction and growth as the
    /// policies allow) provides it — and frees the old fields when it
    /// publishes. A failure leaves the previous record, its fields and the
    /// allocator exactly as they were, apart from a relayout the batch needed
    /// first, as for [`Self::add_columns`].
    ///
    /// Errors are [`crate::FiggyError::InvalidSeriesConfig`] naming `id` for
    /// a source the record rules reject, and [`crate::FiggyError::Pool`] for
    /// allocation failures.
    pub fn upsert_record(
        &mut self,
        id: impl Into<RecordId>,
        source: &RecordSource<'_>,
        ctx: GpuAllocCtx<'_>,
    ) -> crate::Result<RecordId> {
        let id = id.into();
        self.upsert_record_with(id.clone(), source, ctx, |_, _| Ok(()))?;
        Ok(id)
    }

    /// [`Self::upsert_record`], with `prepare(pool, relocated)` run against
    /// the final provisional pool before anything is published; an error from
    /// it abandons the upsert. `relocated` is whether the batch relaid the
    /// pool out first.
    pub(crate) fn upsert_record_with<T>(
        &mut self,
        id: RecordId,
        source: &RecordSource<'_>,
        ctx: GpuAllocCtx<'_>,
        prepare: impl FnOnce(&ColumnPool, bool) -> crate::Result<T>,
    ) -> crate::Result<T> {
        self.check_composite_id(&id, "record")?;
        if self.raggeds.contains_key(&id) {
            return Err(composite_error(
                &id,
                "the id is a ragged column; columns, records and ragged columns share one id space",
            ));
        }
        let schema = source.schema;
        if schema.len() > MAX_RECORD_FIELDS {
            return Err(composite_error(
                &id,
                format!(
                    "the record schema has {} fields; a record holds at most \
                     MAX_RECORD_FIELDS ({MAX_RECORD_FIELDS})",
                    schema.len()
                ),
            ));
        }
        if source.fields.len() > MAX_RECORD_FIELDS {
            return Err(composite_error(
                &id,
                format!(
                    "the record source has {} fields; a record holds at most \
                     MAX_RECORD_FIELDS ({MAX_RECORD_FIELDS})",
                    source.fields.len()
                ),
            ));
        }

        // Fields by schema slot.
        let mut by_slot: [Option<&dyn ColumnSource>; MAX_RECORD_FIELDS] = [None; MAX_RECORD_FIELDS];
        for (name, field) in source.fields {
            let Some(slot) = schema.iter().position(|(known, _)| known == name) else {
                return Err(composite_error(
                    &id,
                    format!("'{name}' is not a field of the record schema"),
                ));
            };
            if by_slot[slot].is_some() {
                return Err(composite_error(
                    &id,
                    format!("the record field '{name}' is given twice"),
                ));
            }
            by_slot[slot] = Some(*field);
        }
        let mut len: Option<(usize, &str)> = None;
        for (slot, (name, required)) in schema.iter().enumerate() {
            let Some(field) = by_slot[slot] else {
                if *required {
                    return Err(composite_error(
                        &id,
                        format!("the required record field '{name}' is missing"),
                    ));
                }
                continue;
            };
            match len {
                None => len = Some((field.len(), *name)),
                Some((rows, first)) if rows != field.len() => {
                    return Err(composite_error(
                        &id,
                        format!(
                            "the record field '{name}' has {} rows but '{first}' has {rows}; \
                             every field of a record has the same length",
                            field.len()
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
        let Some((len, _)) = len else {
            return Err(composite_error(&id, "a record needs at least one field"));
        };
        if len == 0 {
            return Err(composite_error(&id, "a record needs at least one row"));
        }
        let rows = u32::try_from(len).map_err(|_| {
            composite_error(&id, format!("a record holds at most {} rows", u32::MAX))
        })?;

        // Member ids in schema order; `order[i]` is the schema slot of batch
        // column `i`.
        let token = self.member_token()?;
        let mut members: [Option<ColumnId>; MAX_RECORD_FIELDS] = Default::default();
        let mut order = [0usize; MAX_RECORD_FIELDS];
        let mut count = 0usize;
        for (slot, (name, _)) in schema.iter().enumerate() {
            if by_slot[slot].is_some() {
                members[slot] = Some(format!(
                    "{id}{MEMBER_SEPARATOR}{token}{MEMBER_SEPARATOR}{name}"
                ));
                order[count] = slot;
                count += 1;
            }
        }
        let placeholder = source.fields[0].1;
        let columns: [(&str, &dyn ColumnSource); MAX_RECORD_FIELDS] = std::array::from_fn(|i| {
            if i < count {
                let slot = order[i];
                (
                    members[slot]
                        .as_deref()
                        .expect("a present field has a member id"),
                    by_slot[slot].expect("a present field has a source"),
                )
            } else {
                ("", placeholder)
            }
        });

        let old: [Option<ColumnId>; MAX_RECORD_FIELDS] = self
            .records
            .get(&id)
            .map_or_else(Default::default, |slot| slot.fields.clone());
        let mut old_members = [""; MAX_RECORD_FIELDS];
        let mut old_count = 0usize;
        for member in old.iter().flatten() {
            old_members[old_count] = member;
            old_count += 1;
        }

        self.records
            .try_reserve(1)
            .map_err(|error| crate::FiggyError::StateAllocationFailed {
                resource: "record registry",
                reason: error.to_string(),
            })?;
        let layout_before = self.layout_generation;
        let mut tallies = [ValueTally::EMPTY; MAX_RECORD_FIELDS];
        let mut observe = |column: usize, _row: usize, hi: f32, lo: f32| {
            tallies[column].record(hi, lo);
        };
        let observer: &mut dyn FnMut(usize, usize, f32, f32) = &mut observe;
        let batch =
            self.begin_add_columns_observed(&columns[..count], ctx, Some(observer), None)?;
        let relocated = batch.pool.layout_generation != layout_before;
        let epoch = batch
            .pool
            .allocation_epoch(columns[0].0)
            .expect("a provisional member has an allocation epoch");
        let value = finish_member_batch(batch, &old_members[..old_count], |pool| {
            prepare(pool, relocated)
        })?;

        let mut stats = RecordStats {
            len: rows,
            present: [false; MAX_RECORD_FIELDS],
            min: [f64::INFINITY; MAX_RECORD_FIELDS],
            max: [f64::NEG_INFINITY; MAX_RECORD_FIELDS],
            finite: [0; MAX_RECORD_FIELDS],
            nan: [0; MAX_RECORD_FIELDS],
        };
        for (tally, slot) in tallies.iter().zip(&order).take(count) {
            stats.present[*slot] = true;
            stats.min[*slot] = tally.min;
            stats.max[*slot] = tally.max;
            stats.finite[*slot] = tally.finite;
            stats.nan[*slot] = tally.nan;
        }
        self.records.insert(
            id,
            RecordSlot {
                schema,
                fields: members,
                len: rows,
                epoch,
                stats,
            },
        );
        Ok(value)
    }

    /// Insert or replace the ragged column `id` (failure-atomic).
    ///
    /// The values and the offsets upload as one batch transaction; the write
    /// observer measures the [`RaggedStats`] and checks the offsets in the
    /// same pass: they start at 0, are finite integers, never decrease and
    /// end at `values.len()`. An offsets column that fails a check is
    /// rejected and the batch is abandoned before anything is published. A
    /// replacement needs room for the new members while the old ones are
    /// live, as [`Self::upsert_record`] describes.
    pub fn upsert_ragged(
        &mut self,
        id: impl Into<RaggedId>,
        source: &RaggedSource<'_>,
        ctx: GpuAllocCtx<'_>,
    ) -> crate::Result<RaggedId> {
        let id = id.into();
        self.upsert_ragged_with(id.clone(), source, ctx, |_, _| Ok(()))?;
        Ok(id)
    }

    /// [`Self::upsert_ragged`] with a `prepare` step, as
    /// [`Self::upsert_record_with`].
    pub(crate) fn upsert_ragged_with<T>(
        &mut self,
        id: RaggedId,
        source: &RaggedSource<'_>,
        ctx: GpuAllocCtx<'_>,
        prepare: impl FnOnce(&ColumnPool, bool) -> crate::Result<T>,
    ) -> crate::Result<T> {
        self.check_composite_id(&id, "ragged column")?;
        if self.records.contains_key(&id) {
            return Err(composite_error(
                &id,
                "the id is a record; columns, records and ragged columns share one id space",
            ));
        }
        let total = source.values.len();
        if total == 0 {
            return Err(composite_error(
                &id,
                "a ragged column needs at least one value; register none when every group is empty",
            ));
        }
        // Offsets index the values as `u32`: the GPU's index space, as a
        // column's lane base.
        let total32 = u32::try_from(total).map_err(|_| {
            composite_error(
                &id,
                format!(
                    "a ragged column indexes its values with u32 offsets, at most {} values, \
                     got {total}",
                    u32::MAX
                ),
            )
        })?;
        let offsets_len = source.offsets.len();
        if offsets_len < 2 {
            return Err(composite_error(
                &id,
                format!(
                    "a ragged column needs one offset per group plus one, at least two, got \
                     {offsets_len}"
                ),
            ));
        }
        let groups = u32::try_from(offsets_len - 1).map_err(|_| {
            composite_error(
                &id,
                format!("a ragged column holds at most {} groups", u32::MAX),
            )
        })?;

        let token = self.member_token()?;
        let values_id = format!("{id}{MEMBER_SEPARATOR}{token}{MEMBER_SEPARATOR}values");
        let offsets_id = format!("{id}{MEMBER_SEPARATOR}{token}{MEMBER_SEPARATOR}offsets");
        let columns: [(&str, &dyn ColumnSource); 2] = [
            (values_id.as_str(), source.values),
            (offsets_id.as_str(), source.offsets),
        ];
        let old = self
            .raggeds
            .get(&id)
            .map(|slot| (slot.values.clone(), slot.offsets.clone()));
        let old_members: [&str; 2] = old.as_ref().map_or(["", ""], |(values, offsets)| {
            [values.as_str(), offsets.as_str()]
        });
        let old_count = if old.is_some() { 2 } else { 0 };

        self.raggeds
            .try_reserve(1)
            .map_err(|error| crate::FiggyError::StateAllocationFailed {
                resource: "ragged column registry",
                reason: error.to_string(),
            })?;
        let layout_before = self.layout_generation;
        let mut values = ValueTally::EMPTY;
        let mut offsets = OffsetTally {
            next_row: 0,
            in_order: true,
            integral: true,
            non_decreasing: true,
            first: f64::NAN,
            previous: f64::NAN,
            max_group: 0.0,
        };
        let mut observe = |column: usize, row: usize, hi: f32, lo: f32| {
            if column == 0 {
                values.record(hi, lo);
            } else {
                offsets.record(row, hi, lo);
            }
        };
        let observer: &mut dyn FnMut(usize, usize, f32, f32) = &mut observe;
        // The offsets (column 1) are indices: stored as `u32` lanes, exact up
        // to `u32::MAX` (SHADER_COMMON.md §5.14).
        let batch = self.begin_add_columns_observed(&columns, ctx, Some(observer), Some(1))?;
        // Rejections below drop `batch`, which un-inserts both members; their
        // bytes went to regions that were free, so nothing live changed.
        if !offsets.in_order || offsets.next_row != offsets_len {
            return Err(composite_error(
                &id,
                "the offsets source must write every offset once, in row order",
            ));
        }
        if !offsets.integral {
            return Err(composite_error(
                &id,
                "ragged offsets must be finite non-negative integers",
            ));
        }
        if offsets.first != 0.0 {
            return Err(composite_error(
                &id,
                format!("ragged offsets must start at 0, got {}", offsets.first),
            ));
        }
        if !offsets.non_decreasing {
            return Err(composite_error(
                &id,
                "ragged offsets must be non-decreasing: group g is values[offsets[g]..offsets[g + 1]]",
            ));
        }
        if offsets.previous != total as f64 {
            return Err(composite_error(
                &id,
                format!(
                    "the last ragged offset is {} but there are {total} values",
                    offsets.previous
                ),
            ));
        }
        let relocated = batch.pool.layout_generation != layout_before;
        let epoch = batch
            .pool
            .allocation_epoch(&values_id)
            .expect("a provisional member has an allocation epoch");
        let value = finish_member_batch(batch, &old_members[..old_count], |pool| {
            prepare(pool, relocated)
        })?;

        let stats = RaggedStats {
            groups,
            total: total32,
            min: values.min,
            max: values.max,
            finite: values.finite,
            nan: values.nan,
            non_decreasing: offsets.non_decreasing,
            max_group: offsets.max_group as u32,
        };
        self.raggeds.insert(
            id,
            RaggedSlot {
                values: values_id,
                offsets: offsets_id,
                groups,
                total: total32,
                epoch,
                stats,
            },
        );
        Ok(value)
    }

    /// Delete the record `id` and free its member columns. `false` when no
    /// such record is resident.
    pub fn remove_record(&mut self, id: &str) -> crate::Result<bool> {
        Ok(self.remove_record_with(id, |_| Ok(()))?.is_some())
    }

    /// [`Self::remove_record`] with `prepare` run against the provisional
    /// pool before the removal is published. `None` when no such record is
    /// resident.
    pub(crate) fn remove_record_with<T>(
        &mut self,
        id: &str,
        prepare: impl FnOnce(&ColumnPool) -> crate::Result<T>,
    ) -> crate::Result<Option<T>> {
        let Some(slot) = self.records.get(id) else {
            return Ok(None);
        };
        let fields = slot.fields.clone();
        let mut members = [""; MAX_RECORD_FIELDS];
        let mut count = 0usize;
        for member in fields.iter().flatten() {
            members[count] = member;
            count += 1;
        }
        let removal = self.begin_remove_columns(&members[..count])?;
        let value = prepare(&*removal.pool)?;
        removal.commit();
        self.records.remove(id);
        Ok(Some(value))
    }

    /// Delete the ragged column `id` and free its two member columns.
    /// `false` when no such ragged column is resident.
    pub fn remove_ragged(&mut self, id: &str) -> crate::Result<bool> {
        Ok(self.remove_ragged_with(id, |_| Ok(()))?.is_some())
    }

    /// [`Self::remove_ragged`] with a `prepare` step, as
    /// [`Self::remove_record_with`].
    pub(crate) fn remove_ragged_with<T>(
        &mut self,
        id: &str,
        prepare: impl FnOnce(&ColumnPool) -> crate::Result<T>,
    ) -> crate::Result<Option<T>> {
        let Some(slot) = self.raggeds.get(id) else {
            return Ok(None);
        };
        let (values, offsets) = (slot.values.clone(), slot.offsets.clone());
        let removal = self.begin_remove_columns(&[values.as_str(), offsets.as_str()])?;
        let value = prepare(&*removal.pool)?;
        removal.commit();
        self.raggeds.remove(id);
        Ok(Some(value))
    }
}
