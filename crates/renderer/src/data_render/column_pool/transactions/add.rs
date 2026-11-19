//! Batch inserts of new columns and the single-column upserts.

use super::*;

impl ColumnPool {
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
    pub(in crate::data_render::column_pool) fn begin_add_columns_observed<'a>(
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
        //    device cannot hold it in one. Pair columns are staged in the
        //    region's layout and compact single-lane columns at half their
        //    region (#317); each chunk with a compact column has one transient
        //    storage buffer, and the expansion pipeline is ready before the
        //    sources run.
        let limits = ExpandLimits::of(device);
        let chunks = plan_batch_chunks(columns, ceiling, limits, index_column)?;
        let expand = if chunks.iter().any(|chunk| chunk.transient_bytes > 0) {
            Some(ColumnExpand::ensure(&mut self.expand, device)?.clone())
        } else {
            None
        };
        let mut staging = Vec::new();
        staging
            .try_reserve_exact(chunks.len())
            .map_err(|error| AllocError::AllocationFailed {
                resource: "batch staging registry",
                reason: error.to_string(),
            })?;
        for chunk in &chunks {
            // Compact staging is half a region at most: never above the
            // chunk's share of the region, which the ceiling bounds.
            if chunk.staging_bytes > chunk.byte_size {
                return Err(AllocError::AllocationFailed {
                    resource: "batch staging",
                    reason: "internal: staging larger than its region".into(),
                });
            }
            let buffer = create_buffer_checked(
                device,
                ctx.ledger,
                &BufferDescriptor {
                    label: Some("figgy batch column staging"),
                    size: chunk.staging_bytes,
                    usage: BufferUsages::COPY_SRC,
                    mapped_at_creation: true,
                },
                "batch column staging buffer",
            )?;
            let transient = if chunk.transient_bytes > 0 {
                Some(super::expand::create_transient(
                    device,
                    ctx.ledger,
                    chunk.transient_bytes,
                )?)
            } else {
                None
            };
            staging.push((buffer, transient));
        }

        // ── Staging writes. A failure here un-inserts every id of the batch
        //    (each was new to the pool, step 1) and the reservation un-reserves.
        let mut epoch = first_epoch;
        let written = (|| {
            for (chunk, (buffer, _)) in chunks.iter().zip(&staging) {
                let mut view = buffer
                    .slice(0..chunk.staging_bytes)
                    .get_mapped_range_mut()
                    .map_err(|error| AllocError::Gpu {
                        resource: "batch column staging buffer",
                        reason: error.to_string(),
                    })?;
                for index in chunk.columns.clone() {
                    let (id, source) = columns[index];
                    let byte_size =
                        column_region_bytes(source.len()).ok_or(AllocError::Internal {
                            invariant: "every batch column was sized in step 2",
                        })?;
                    let placement = chunk.placements[index - chunk.columns.start];
                    let lanes = placement.lanes;
                    let raw_bytes = lanes.raw_bytes(source.len());
                    let local = placement.staging;
                    let dst = view.slice(local..local + raw_bytes as usize);
                    let index_lanes = index_column == Some(index);
                    let mut step = crate::data::UniformStep::new();
                    let out_of_range = std::cell::Cell::new(false);
                    let second_lane = std::cell::Cell::new(false);
                    let second_lane_check = lanes.single.then_some(&second_lane);
                    let bounds = if let Some(observe) = observe.as_mut() {
                        let mut per_column = |row: usize, hi: f32, lo: f32| {
                            step.record(row, hi, lo);
                            (*observe)(index, row, hi, lo);
                        };
                        let mut writer =
                            ColumnPairWriter::new_observed(dst, &out_of_range, &mut per_column)
                                .with_index_lanes(index_lanes)
                                .with_single_lane(lanes.compact, second_lane_check);
                        source.write_pairs(&mut writer);
                        writer.bounds()
                    } else {
                        let mut per_column =
                            |row: usize, hi: f32, lo: f32| step.record(row, hi, lo);
                        let mut writer =
                            ColumnPairWriter::new_observed(dst, &out_of_range, &mut per_column)
                                .with_index_lanes(index_lanes)
                                .with_single_lane(lanes.compact, second_lane_check);
                        source.write_pairs(&mut writer);
                        writer.bounds()
                    };
                    let (min, max, min_positive) = slot_statistics(bounds);
                    if out_of_range.get() {
                        return Err(source_wrote_out_of_range());
                    }
                    if second_lane.get() {
                        return Err(source_wrote_second_lane());
                    }
                    epoch += 1;
                    self.allocation_epochs.insert(id.to_string(), epoch);
                    self.slots.insert(
                        id.to_string(),
                        ColumnSlot {
                            id: id.to_string(),
                            offset: region_offset + chunk.region_offset + placement.region as u64,
                            byte_size,
                            len_values: source.len(),
                            generation: self.generation,
                            min,
                            max,
                            min_positive,
                            uniform_step: step.finish(),
                        },
                    );
                }
                drop(view);
                buffer.unmap();
            }
            Ok(())
        })();
        if let Err(error) = written {
            for (id, _) in columns {
                self.slots.remove(*id);
                self.allocation_epochs.remove(*id);
            }
            return Err(error);
        }

        // ── Nothing below can fail. ───────────────────────────────────────

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy batch column upload"),
        });
        // Per chunk: its pair columns are one contiguous run in the staging
        // and the region, so one copy, as before for an all-pair chunk; its
        // compact columns' halves are one contiguous run in the staging, laid
        // out as the transient buffer wants them, so one more copy (#317).
        for (chunk, (buffer, transient)) in chunks.iter().zip(&staging) {
            let chunk_base = region_offset + chunk.region_offset;
            if chunk.pair_bytes > 0 {
                encoder.copy_buffer_to_buffer(
                    buffer,
                    0,
                    &self.primary,
                    chunk_base,
                    chunk.pair_bytes,
                );
            }
            if let Some(transient) = transient {
                encoder.copy_buffer_to_buffer(
                    buffer,
                    chunk.pair_bytes,
                    transient,
                    0,
                    chunk.transient_bytes,
                );
            }
        }
        // One expansion per chunk's compact run, whatever its column count.
        // The run lies after the chunk's pair columns, so the pair copy and
        // the expansion write disjoint bytes.
        if let (Some(expand), Some(limits)) = (expand.as_ref(), limits) {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy batch column expand"),
                timestamp_writes: None,
            });
            for (chunk, (_, transient)) in chunks.iter().zip(&staging) {
                let Some(transient) = transient else {
                    continue;
                };
                expand.dispatch_run(
                    device,
                    &mut pass,
                    limits,
                    transient,
                    &self.primary,
                    region_offset + chunk.region_offset + chunk.pair_bytes,
                    chunk.compact_bytes(),
                );
            }
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
        for _ in &staging {}
        // CPU handles are dropped on return, but the submitted copy keeps the
        // staging allocation device-resident until queue completion.
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
    pub(in crate::data_render::column_pool) fn make_room_for_batch(
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
                lanes: source.lanes(),
            },
            ctx,
            |dst| source.write_pairs(dst),
            |byte_size| {
                create_buffer_checked(
                    device,
                    ctx.ledger,
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
                lanes: crate::ColumnLanes::Pair,
            },
            ctx,
            |dst| source.write_pairs(dst),
            |byte_size| {
                create_buffer_checked(
                    device,
                    ctx.ledger,
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

    /// The const-length plain-column form of
    /// [`Self::begin_batch_upsert_sources`] (the bundled demo's batch), with
    /// the columns' handles in the order they were given.
    #[cfg(test)]
    pub(crate) fn begin_batch_upsert<'a, const N: usize>(
        &'a mut self,
        columns: [(ColumnId, &dyn ColumnSource); N],
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnBatchUpsert<'a, N>, AllocError> {
        let uploads: [(&str, BatchSource<'_>); N] =
            std::array::from_fn(|k| (columns[k].0.as_str(), BatchSource::Column(columns[k].1)));
        let batch = self
            .begin_batch_upsert_sources(&uploads, &[], ctx)
            .map_err(|error| match error {
                crate::FiggyError::Pool(error) => error,
                _ => AllocError::Internal {
                    invariant: "a plain column batch fails with a pool error",
                },
            })?;
        let handles = batch.handles(&uploads.map(|(id, _)| id))?;
        Ok(ColumnBatchUpsert { batch, handles })
    }

    /// The slot ids `id` holds: itself for a plain column, the members of a
    /// record, a ragged or a triangle index column, none otherwise. A plain
    /// column of a member id holds none: its owner holds it.
    pub(super) fn held_slots(&self, id: &str) -> ([&str; MAX_RECORD_FIELDS], usize) {
        let mut held = [""; MAX_RECORD_FIELDS];
        let mut count = 0usize;
        if let Some(record) = self.records.get(id) {
            for member in record.fields.iter().flatten() {
                held[count] = member;
                count += 1;
            }
        } else if let Some(ragged) = self.raggeds.get(id) {
            held[0] = &ragged.values;
            held[1] = &ragged.offsets;
            count = 2;
        } else if let Some(triangles) = self.triangles.get(id) {
            held[0] = &triangles.indices;
            count = 1;
        } else if self.slots.contains_key(id) && !self.is_member_column(id) {
            held[0] = self.slots[id].id.as_str();
            count = 1;
        }
        (held, count)
    }
}
