//! Resident capacity planning and the streamed-to-resident promotion.

use super::*;

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
            if let Some(limit) = ctx.ledger.budget() {
                // The ledger already holds the live slab and every retired
                // buffer; the staging and the candidate slab come on top.
                let peak = ctx
                    .ledger
                    .total_bytes()
                    .checked_add(total)
                    .and_then(|bytes| bytes.checked_add(capacity_after))
                    .ok_or(AllocError::BudgetExceeded {
                        resource: "streamed promotion transition",
                        requested: u64::MAX,
                        limit,
                    })?;
                if peak > limit {
                    return Err(AllocError::BudgetExceeded {
                        resource: "streamed promotion transition",
                        requested: peak,
                        limit,
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
        let (staging, staged) = stage_streamed_columns(ctx.device, ctx.ledger, columns, total)?;

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
                *self.allocation_epochs.get(id).ok_or(AllocError::Internal {
                    invariant: "live column has an allocation epoch",
                })?,
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
            ctx.ledger,
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

        let candidate = ColumnPool {
            identity: self.identity.clone(),
            primary,
            capacity: capacity_after,
            slots: planned_slots,
            records: HashMap::new(),
            raggeds: HashMap::new(),
            triangles: HashMap::new(),
            free: planned_free,
            generation: next_generation,
            allocation_epochs: planned_epochs,
            allocation_epoch_counter: last_epoch,
            layout_generation: next_layout_generation,
            backup: None,
            defrag_policy: self.defrag_policy,
            growth_policy: self.growth_policy,
            // The live pool keeps its pipeline; commit moves the layout only.
            expand: None,
        };
        // Dropping an uncommitted candidate drops its tracked slab, which
        // retires the bytes in the device ledger.
        Ok(StreamedColumnPromotion::Candidate(ColumnBatchCandidate {
            pool: self,
            candidate,
        }))
    }
}
