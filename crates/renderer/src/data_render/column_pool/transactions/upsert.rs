//! Pair and staged upserts, compact expansion and the single-column adds.

use super::*;

impl ColumnPool {
    pub(in crate::data_render::column_pool) fn begin_upsert_column_pairs_with<'a>(
        &'a mut self,
        input: ColumnInputMeta,
        ctx: GpuAllocCtx<'_>,
        write_pairs: impl FnOnce(&mut ColumnPairWriter<'_>),
        create_staging: impl FnOnce(u64) -> Result<TrackedBuffer, AllocError>,
    ) -> Result<ColumnUpsert<'a>, AllocError> {
        let n = input.len_values;
        self.begin_upsert_column_staged_with(input, ctx, |byte_size, staged_lanes| {
            let staging = create_staging(staged_lanes.staged_bytes(byte_size))?;
            let (bounds, uniform_step) = write_staging_pairs(
                &staging,
                staged_lanes.raw_bytes(n),
                staged_lanes,
                write_pairs,
            )?;
            Ok((staging, bounds, uniform_step))
        })
    }

    /// Install a column written range by range into its own staging buffer
    /// ([`ColumnStaging`], #311) as a failure-atomic insert or same-id
    /// replacement, exactly as [`Self::begin_upsert_column`] installs one a
    /// source writes in one call. Every value must have been written.
    pub fn begin_upsert_staged_column<'a>(
        &'a mut self,
        id: ColumnId,
        staging: ColumnStaging,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnUpsert<'a>, AllocError> {
        if !staging.is_complete() {
            return Err(AllocError::Internal {
                invariant: "a staged column is complete before it is installed",
            });
        }
        staging.unmap_for_install();
        let ColumnStaging {
            staging: buffer,
            len_values,
            byte_size: staged_size,
            bounds,
            step,
            ..
        } = staging;
        self.begin_upsert_column_staged_with(
            // A chunked upload stages pairs (#311); its column takes the
            // pair path whatever its source declares.
            ColumnInputMeta {
                id,
                len_values,
                lanes: crate::ColumnLanes::Pair,
            },
            ctx,
            move |byte_size, staged_lanes| {
                if byte_size != staged_size || staged_lanes.compact {
                    return Err(AllocError::Internal {
                        invariant: "a staged column's buffer has the size of its region",
                    });
                }
                Ok((buffer, bounds, step.finish()))
            },
        )
    }

    /// The upsert with the staging step supplied: `stage(byte_size,
    /// staged_lanes)` returns the unmapped staging buffer that holds the
    /// column as `staged_lanes` lays it out (compact or as pairs, #317), with
    /// the statistics and uniform step its writer recorded.
    #[allow(clippy::type_complexity)]
    fn begin_upsert_column_staged_with<'a>(
        &'a mut self,
        input: ColumnInputMeta,
        ctx: GpuAllocCtx<'_>,
        stage: impl FnOnce(
            u64,
            StagedLanes,
        ) -> Result<
            (TrackedBuffer, Option<crate::StreamBounds>, Option<f64>),
            AllocError,
        >,
    ) -> Result<ColumnUpsert<'a>, AllocError> {
        let (device, queue) = (ctx.device, ctx.queue);
        let ceiling = buffer_ceiling(device);
        let ColumnInputMeta {
            id,
            len_values: n,
            lanes,
        } = input;
        if self.is_member_column(&id) {
            return Err(AllocError::MemberColumn(id));
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
        // Growth, when the policy allows it, has to be decided *before* the
        // upsert begins: the returned guard borrows the pool for its whole
        // lifetime, so there is no way to retry after a failed attempt. A
        // pure `&self` fit check decides. Opted-in growth failures preserve
        // their budget/device/allocation category for the host.
        self.grow_for_upsert_if_needed(&id, byte_size, ctx)?;
        let allocation_epoch = self.checked_allocation_epoch_successor()?;

        // Complete every source-dependent/fallible staging operation before
        // touching allocator metadata or the live primary buffer. A compact
        // single-lane column stages half the region (#317).
        let limits = ExpandLimits::of(device);
        let staged_lanes = StagedLanes::of(limits, lanes, false, n);
        let (staging, bounds, uniform_step) = stage(byte_size, staged_lanes)?;
        let (min, max, min_positive) = slot_statistics(bounds);
        let expansion = self.prepare_expansion(ctx, limits, staged_lanes, byte_size)?;

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
                        .ok_or(AllocError::Internal {
                            invariant: "planned survivor remains present",
                        })?;
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
                        ctx.ledger,
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
                if fresh_candidate.is_some() {}
                let candidate = fresh_candidate.as_ref().or(self.backup.as_ref()).ok_or(
                    AllocError::Internal {
                        invariant: "atomic upsert candidate was prepared",
                    },
                )?;
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
        let target_buffer = match &target {
            PreparedUpsertTarget::InPlace => &self.primary,
            PreparedUpsertTarget::FreshCandidate(candidate) => candidate,
            PreparedUpsertTarget::BackupCandidate => {
                self.backup.as_ref().ok_or(AllocError::Internal {
                    invariant: "atomic upsert backup candidate remains available",
                })?
            }
        };
        encode_column_upload(
            device,
            &mut command_encoder,
            &staging,
            expansion.as_ref(),
            target_buffer,
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
                // Checked before the submission above, and nothing since
                // touched the backup.
                let Some(candidate) = self.backup.take() else {
                    return Err(AllocError::Internal {
                        invariant: "submitted backup candidate remains available",
                    });
                };
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

    /// The pipeline and transient buffer a compact column's upload needs
    /// (#317); `None` for a column staged as pairs. The pipeline is created
    /// on the first compact upload and cached on the pool.
    pub(in crate::data_render::column_pool) fn prepare_expansion(
        &mut self,
        ctx: GpuAllocCtx<'_>,
        limits: Option<ExpandLimits>,
        lanes: StagedLanes,
        region_bytes: u64,
    ) -> Result<Option<PreparedExpansion>, AllocError> {
        if !lanes.compact {
            return Ok(None);
        }
        let limits = limits.ok_or(AllocError::Internal {
            invariant: "a compact column was staged on a device that expands",
        })?;
        let pipeline = ColumnExpand::ensure(&mut self.expand, ctx.device)?.clone();
        let transient = super::expand::create_transient(
            ctx.device,
            ctx.ledger,
            lanes.transient_bytes(region_bytes),
        )?;
        Ok(Some(PreparedExpansion {
            expand: pipeline,
            limits,
            transient,
        }))
    }

    /// Single attempt without auto-retry. Internal + test use.
    ///
    /// 1. First-fit allocation from the free list.
    /// 2. Create a `mapped_at_creation: true` staging buffer.
    /// 3. `ColumnSource::write_pairs` writes the values through the
    ///    write-only mapped view; the writer records their statistics.
    /// 4. Unmap, encode a staging→primary copy, submit.
    pub(in crate::data_render::column_pool) fn try_add_column(
        &mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        self.try_add_column_pairs(
            ColumnInputMeta {
                id,
                len_values: source.len(),
                lanes: source.lanes(),
            },
            ctx,
            |dst| source.write_pairs(dst),
        )
    }

    pub(in crate::data_render::column_pool) fn try_add_hilo_column(
        &mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        self.try_add_column_pairs(
            ColumnInputMeta {
                id,
                len_values: source.len(),
                lanes: crate::ColumnLanes::Pair,
            },
            ctx,
            |dst| source.write_pairs(dst),
        )
    }

    pub(in crate::data_render::column_pool) fn try_add_column_pairs(
        &mut self,
        input: ColumnInputMeta,
        ctx: GpuAllocCtx<'_>,
        write_pairs: impl FnOnce(&mut ColumnPairWriter<'_>),
    ) -> Result<ColumnHandle, AllocError> {
        let device = ctx.device;
        self.try_add_column_pairs_with(input, ctx, write_pairs, |byte_size| {
            create_buffer_checked(
                device,
                ctx.ledger,
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

    pub(in crate::data_render::column_pool) fn try_add_column_pairs_with(
        &mut self,
        input: ColumnInputMeta,
        ctx: GpuAllocCtx<'_>,
        write_pairs: impl FnOnce(&mut ColumnPairWriter<'_>),
        create_staging: impl FnOnce(u64) -> Result<TrackedBuffer, AllocError>,
    ) -> Result<ColumnHandle, AllocError> {
        let (device, queue) = (ctx.device, ctx.queue);
        let ceiling = buffer_ceiling(device);
        let ColumnInputMeta {
            id,
            len_values: n,
            lanes,
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
        // A compact single-lane column stages half the region and expands
        // on the GPU (#317); its pipeline and transient buffer are prepared
        // before the region is reserved.
        let limits = ExpandLimits::of(device);
        let staged_lanes = StagedLanes::of(limits, lanes, false, n);
        let expansion = self.prepare_expansion(ctx, limits, staged_lanes, byte_size)?;

        let reservation = alloc_region(&mut self.free, byte_size)?;
        let region_offset = reservation.offset();

        // The source writes and collects stats while the write-only view lives.
        let staging = create_staging(staged_lanes.staged_bytes(byte_size))?;
        let (bounds, uniform_step) = write_staging_pairs(
            &staging,
            staged_lanes.raw_bytes(n),
            staged_lanes,
            write_pairs,
        )?;
        let (min, max, min_positive) = slot_statistics(bounds);

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

        // staging → primary[region_offset..] (GPU-internal copy), or for a
        // compact column staging → transient and the expansion.
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy column upload encoder"),
        });
        encode_column_upload(
            device,
            &mut enc,
            &staging,
            expansion.as_ref(),
            &self.primary,
            region_offset,
            byte_size,
        );
        queue.submit(std::iter::once(enc.finish()));

        self.slots = planned_slots;
        self.allocation_epochs = planned_allocation_epochs;
        self.allocation_epoch_counter = allocation_epoch;
        reservation.commit();
        // Charged here rather than at the `create_staging` call because the
        // reservation borrows the free list until it commits. `staging` is
        // still alive at this point, so the peak it contributes to is real.
        Ok(handle)
    }
}
