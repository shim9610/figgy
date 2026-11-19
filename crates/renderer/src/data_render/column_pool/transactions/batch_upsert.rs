//! The batch transaction of plain, record and ragged sources (#311).

use super::*;

/// What a member of a batch transaction is to its upload, with the tally
/// its writes feed (#311).
enum MemberRole {
    /// A plain column.
    Plain,
    /// Field `slot` of a record's schema.
    Field { slot: usize, tally: ValueTally },
    /// A ragged column's values.
    Values(ValueTally),
    /// A ragged column's offsets, stored as index lanes.
    Offsets(OffsetTally),
}

impl MemberRole {
    fn record(&mut self, row: usize, hi: f32, lo: f32) {
        match self {
            Self::Plain => {}
            Self::Field { tally, .. } | Self::Values(tally) => tally.record(hi, lo),
            Self::Offsets(tally) => tally.record(row, hi, lo),
        }
    }
}

/// A member's staging and the statistics its writer measured.
struct StagedMember {
    staging: TrackedBuffer,
    min: f64,
    max: f64,
    min_positive: Option<f64>,
    uniform_step: Option<f64>,
}

/// Where a batch member's values come from: a source the batch writes into
/// its own staging, or a chunked upload's staging, already written (#334).
#[derive(Clone, Copy)]
enum MemberInput<'s> {
    Source(&'s dyn ColumnSource),
    Staged(&'s ColumnStaging),
}

/// One pool column a batch transaction uploads: a plain column, or a member
/// of a record or a ragged column (#311).
struct BatchMember<'s> {
    id: ColumnId,
    source: MemberInput<'s>,
    role: MemberRole,
    len_values: usize,
    byte_size: u64,
    /// Compact single-lane or pairs, decided once per column (#317).
    lanes: StagedLanes,
    staged: Option<StagedMember>,
}

impl<'s> BatchMember<'s> {
    fn new(
        id: ColumnId,
        source: &'s dyn ColumnSource,
        role: MemberRole,
        limits: Option<ExpandLimits>,
        ceiling: u64,
    ) -> Result<Self, AllocError> {
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
        let index = matches!(role, MemberRole::Offsets(_));
        Ok(Self {
            id,
            source: MemberInput::Source(source),
            role,
            len_values,
            byte_size,
            lanes: StagedLanes::of(limits, source.lanes(), index, len_values),
            staged: None,
        })
    }

    /// A plain column whose values a chunked upload already wrote into
    /// `staging` (#334). Its staging is the upload's, already charged to
    /// the ledger, and holds `(hi, lo)` pairs, as every chunked upload does.
    fn staged(id: &str, staging: &'s ColumnStaging) -> crate::Result<Self> {
        if !staging.is_complete() {
            return Err(crate::FiggyError::InvalidColumnUpload {
                id: id.to_owned(),
                reason: ColumnStagingError::Incomplete.reason(),
            });
        }
        Ok(Self {
            id: id.to_owned(),
            source: MemberInput::Staged(staging),
            role: MemberRole::Plain,
            len_values: staging.len_values,
            byte_size: staging.byte_size,
            lanes: StagedLanes {
                compact: false,
                single: false,
            },
            staged: None,
        })
    }

    /// The staging bytes the batch creates for this member: none for a
    /// chunked upload, whose staging the ledger already holds.
    fn new_staging_bytes(&self) -> u64 {
        match self.source {
            MemberInput::Source(_) => self.lanes.staged_bytes(self.byte_size),
            MemberInput::Staged(_) => 0,
        }
    }
}

impl ColumnPool {
    /// Begin one failure-atomic transaction (#311) that inserts or replaces
    /// every upload of `uploads` under its own id — plain columns, records
    /// and ragged columns, each replacing only what its id holds of the
    /// same kind — and removes every id of `removals` (a plain column, a
    /// record, a ragged or a triangle index column) together.
    ///
    /// The transaction builds a complete candidate pool: a new slab with the
    /// surviving columns packed first and every upload after them, the
    /// composite registries and fresh statistics. The live pool is not
    /// touched until [`ColumnSourcesBatch::commit`] moves the candidate in;
    /// dropping the guard drops the candidate. The records and ragged
    /// columns go through the rules [`Self::upsert_record`] and
    /// [`Self::upsert_ragged`] apply, measured while their members are
    /// written; single-lane columns stage compact and expand on the GPU, as
    /// every upload does (#317).
    ///
    /// Peak memory: the live slab and whatever the device ledger already
    /// holds, plus every upload's staging, the compact columns' transient
    /// buffer and the candidate slab. The candidate is the live capacity
    /// when the survivors and the uploads fit it, and otherwise, under
    /// [`GrowthPolicy::OnAllocFailure`], twice it or what they need; a
    /// budget that cannot hold that peak refuses the transaction with
    /// [`AllocError::BudgetExceeded`] before any buffer is made. Removed and
    /// replaced columns take no room in the candidate.
    pub(crate) fn begin_batch_upsert_sources<'a>(
        &'a mut self,
        uploads: &[(&str, BatchSource<'_>)],
        removals: &[&str],
        ctx: GpuAllocCtx<'_>,
    ) -> crate::Result<ColumnSourcesBatch<'a>> {
        let (device, queue) = (ctx.device, ctx.queue);
        let ceiling = buffer_ceiling(device);
        fn reserve_failed(
            resource: &'static str,
        ) -> impl FnOnce(std::collections::TryReserveError) -> AllocError {
            move |error| AllocError::AllocationFailed {
                resource,
                reason: error.to_string(),
            }
        }

        // 1) Ids: distinct across uploads and removals, each upload free or
        //    holding its own kind, each removal held.
        let mut seen = std::collections::HashSet::new();
        seen.try_reserve(uploads.len().saturating_add(removals.len()))
            .map_err(reserve_failed("batch id set"))?;
        let mut composites = false;
        for (id, source) in uploads {
            if !seen.insert(*id) {
                return Err(AllocError::DuplicateId((*id).to_owned()).into());
            }
            match source {
                BatchSource::Column(_) | BatchSource::Staged(_) => {
                    if self.is_member_column(id) {
                        return Err(AllocError::MemberColumn((*id).to_owned()).into());
                    }
                    if self.composite_kind(id).is_some() {
                        return Err(AllocError::DuplicateId((*id).to_owned()).into());
                    }
                }
                BatchSource::Record(_) => {
                    self.check_composite_id(id, "record")?;
                    self.check_composite_kind_free(id, "record")?;
                    composites = true;
                }
                BatchSource::Ragged(_) => {
                    self.check_composite_id(id, "ragged column")?;
                    self.check_composite_kind_free(id, "ragged column")?;
                    composites = true;
                }
            }
        }
        for id in removals {
            if !seen.insert(*id) {
                return Err(AllocError::DuplicateId((*id).to_owned()).into());
            }
            if self.held_slots(id).1 == 0 {
                if self.is_member_column(id) {
                    return Err(AllocError::MemberColumn((*id).to_owned()).into());
                }
                return Err(crate::FiggyError::UnknownColumn {
                    id: (*id).to_owned(),
                });
            }
        }
        drop(seen);

        // 2) Members: one per plain column, per present record field and two
        //    per ragged column, sized, with the composite rules checked before
        //    anything is made. A replacement's members take ids distinct from
        //    the members they replace: the token is the next allocation epoch.
        let token = if composites { self.member_token()? } else { 0 };
        let limits = ExpandLimits::of(device);
        let member_count = uploads.iter().fold(0usize, |count, (_, source)| {
            count.saturating_add(match source {
                BatchSource::Column(_) | BatchSource::Staged(_) => 1,
                BatchSource::Record(record) => record.fields.len(),
                BatchSource::Ragged(_) => 2,
            })
        });
        let mut members: Vec<BatchMember<'_>> = Vec::new();
        members
            .try_reserve_exact(member_count)
            .map_err(reserve_failed("batch column staging registry"))?;
        for (id, source) in uploads {
            match *source {
                BatchSource::Column(column) => {
                    members.push(BatchMember::new(
                        (*id).to_owned(),
                        column,
                        MemberRole::Plain,
                        limits,
                        ceiling,
                    )?);
                }
                BatchSource::Staged(upload) => {
                    members.push(BatchMember::staged(id, upload.staging())?);
                }
                BatchSource::Record(record) => {
                    let (by_slot, _) = check_record_source(id, &record)?;
                    for (slot, ((name, _), field)) in record.schema.iter().zip(by_slot).enumerate()
                    {
                        if let Some(field) = field {
                            members.push(BatchMember::new(
                                member_id(id, token, name),
                                field,
                                MemberRole::Field {
                                    slot,
                                    tally: ValueTally::EMPTY,
                                },
                                limits,
                                ceiling,
                            )?);
                        }
                    }
                }
                BatchSource::Ragged(ragged) => {
                    check_ragged_source(id, &ragged)?;
                    members.push(BatchMember::new(
                        member_id(id, token, "values"),
                        ragged.values,
                        MemberRole::Values(ValueTally::EMPTY),
                        limits,
                        ceiling,
                    )?);
                    // The offsets are indices: stored as `u32` lanes, exact
                    // up to `u32::MAX` (SHADER_COMMON.md §5.14).
                    members.push(BatchMember::new(
                        member_id(id, token, "offsets"),
                        ragged.offsets,
                        MemberRole::Offsets(OffsetTally::new()),
                        limits,
                        ceiling,
                    )?);
                }
            }
        }

        // 3) Survivors: every live slot that no upload replaces and no
        //    removal frees, in their live order.
        let mut excluded_count = 0usize;
        for id in uploads
            .iter()
            .map(|(id, _)| *id)
            .chain(removals.iter().copied())
        {
            excluded_count = excluded_count.saturating_add(self.held_slots(id).1);
        }
        let mut excluded = std::collections::HashSet::new();
        excluded
            .try_reserve(excluded_count)
            .map_err(reserve_failed("batch replaced column set"))?;
        for id in uploads
            .iter()
            .map(|(id, _)| *id)
            .chain(removals.iter().copied())
        {
            let (held, count) = self.held_slots(id);
            excluded.extend(held.into_iter().take(count));
        }
        let survivor_count = self.slots.len().saturating_sub(excluded.len());
        let mut survivor_ids = Vec::new();
        survivor_ids
            .try_reserve_exact(survivor_count)
            .map_err(reserve_failed("batch survivor order"))?;
        survivor_ids.extend(
            self.slots
                .values()
                .filter(|slot| !excluded.contains(slot.id.as_str()))
                .map(|slot| slot.id.clone()),
        );
        drop(excluded);
        survivor_ids.sort_by_key(|id| self.slots[id].offset);

        // 4) Counters, before anything is made.
        let next_generation = self.checked_generation_successor()?;
        let next_layout_generation = self.checked_layout_successor()?;
        let first_epoch = self.allocation_epoch_counter;
        let last_epoch = u64::try_from(members.len())
            .ok()
            .and_then(|count| first_epoch.checked_add(count))
            .ok_or(AllocError::CounterExhausted {
                counter: "allocation epoch",
            })?;

        // 5) The candidate's capacity: the live one when everything fits,
        //    else grown as the growth policy allows.
        let overflow = AllocError::ResourceLimit {
            resource: "batch column pool candidate",
            requested: u64::MAX,
            limit: ceiling,
        };
        let survivor_bytes = survivor_ids.iter().try_fold(0u64, |sum, id| {
            sum.checked_add(self.slots[id].byte_size)
                .ok_or(overflow.clone())
        })?;
        let new_bytes = members.iter().try_fold(0u64, |sum, member| {
            sum.checked_add(member.byte_size).ok_or(overflow.clone())
        })?;
        let required = survivor_bytes
            .checked_add(new_bytes)
            .ok_or(overflow.clone())?;
        let capacity = if required <= self.capacity {
            self.capacity
        } else if self.growth_policy == GrowthPolicy::OnAllocFailure {
            // As `grow_for_pending_upload`: double, or what is needed.
            let requested = self
                .capacity
                .saturating_mul(2)
                .max(required)
                .min(ceiling)
                .max(required);
            let target = try_align_up(requested, ALIGN).ok_or(overflow.clone())?;
            if target > ceiling {
                return Err(AllocError::ResourceLimit {
                    resource: "batch column pool candidate",
                    requested: target,
                    limit: ceiling,
                }
                .into());
            }
            target
        } else {
            return Err(out_of_space(new_bytes, &self.free).into());
        };

        // 6) The budget: the staging, the transient buffer and the candidate
        //    slab come on top of everything the ledger holds.
        // A chunked upload's staging is in the ledger already (#334).
        let staging_bytes = members.iter().fold(0u64, |sum, member| {
            sum.saturating_add(member.new_staging_bytes())
        });
        let transient_bytes = members.iter().fold(0u64, |sum, member| {
            sum.saturating_add(member.lanes.transient_bytes(member.byte_size))
        });
        if let Some(limit) = ctx.ledger.budget() {
            let peak = ctx
                .ledger
                .total_bytes()
                .saturating_add(staging_bytes)
                .saturating_add(transient_bytes)
                .saturating_add(capacity);
            if peak > limit {
                return Err(AllocError::BudgetExceeded {
                    resource: "batch column transition",
                    requested: peak,
                    limit,
                }
                .into());
            }
        }

        // 7) Staging: each member written once into its own mapped buffer,
        //    its statistics and its composite tallies measured on the way.
        //    A chunked upload's staging is written already: the member
        //    shares its buffer and takes the statistics its chunks measured.
        for member in &mut members {
            let source = match member.source {
                MemberInput::Source(source) => source,
                MemberInput::Staged(written) => {
                    let (min, max, min_positive) = slot_statistics(written.bounds);
                    member.staged = Some(StagedMember {
                        staging: written.staging.clone(),
                        min,
                        max,
                        min_positive,
                        uniform_step: written.step.finish(),
                    });
                    continue;
                }
            };
            let staging = create_buffer_checked(
                device,
                ctx.ledger,
                &BufferDescriptor {
                    label: Some("figgy batch column staging"),
                    size: member.lanes.staged_bytes(member.byte_size),
                    usage: BufferUsages::COPY_SRC,
                    mapped_at_creation: true,
                },
                "batch column staging buffer",
            )?;
            let index_lanes = matches!(member.role, MemberRole::Offsets(_));
            let role = &mut member.role;
            let (bounds, uniform_step) = write_staging_pairs_observed(
                &staging,
                member.lanes.raw_bytes(member.len_values),
                member.lanes,
                index_lanes,
                &mut |row, hi, lo| role.record(row, hi, lo),
                |dst| source.write_pairs(dst),
            )?;
            let (min, max, min_positive) = slot_statistics(bounds);
            member.staged = Some(StagedMember {
                staging,
                min,
                max,
                min_positive,
                uniform_step,
            });
        }
        // The offsets rules, measured while the offsets were written.
        let mut cursor = 0usize;
        for (id, source) in uploads {
            cursor += match source {
                BatchSource::Column(_) | BatchSource::Staged(_) => 1,
                BatchSource::Record(record) => record.fields.len(),
                BatchSource::Ragged(ragged) => {
                    let shape = check_ragged_source(id, ragged)?;
                    let MemberRole::Offsets(offsets) = &members[cursor + 1].role else {
                        return Err(crate::FiggyError::internal(
                            "a ragged upload stages its offsets second",
                        ));
                    };
                    offsets.check(id, &shape)?;
                    2
                }
            };
        }

        // 8) The candidate layout: survivors packed, then every member.
        let final_count = survivor_ids.len().saturating_add(members.len());
        let mut planned_slots = HashMap::new();
        planned_slots
            .try_reserve(final_count)
            .map_err(reserve_failed("batch column registry"))?;
        let mut planned_allocation_epochs = HashMap::new();
        planned_allocation_epochs
            .try_reserve(final_count)
            .map_err(reserve_failed("batch allocation epoch registry"))?;
        let mut next_offset = 0u64;
        for id in &survivor_ids {
            let old = &self.slots[id];
            let mut slot = old.clone();
            slot.offset = next_offset;
            slot.generation = next_generation;
            planned_allocation_epochs.insert(
                id.clone(),
                *self.allocation_epochs.get(id).ok_or(AllocError::Internal {
                    invariant: "live column has an allocation epoch",
                })?,
            );
            planned_slots.insert(id.clone(), slot);
            next_offset += old.byte_size;
        }
        // The pair members first, then the compact single-lane members as
        // one contiguous run (#317), so one expansion covers every compact
        // member. Epochs follow the members' order either way.
        let pair_bytes = members
            .iter()
            .filter(|member| !member.lanes.compact)
            .fold(0u64, |sum, member| sum + member.byte_size);
        let compact_run = next_offset + pair_bytes;
        let mut compact_offset = compact_run;
        let mut epoch = first_epoch;
        for member in &members {
            let staged = member.staged.as_ref().ok_or(AllocError::Internal {
                invariant: "every batch member was staged",
            })?;
            epoch += 1;
            let offset = if member.lanes.compact {
                compact_offset += member.byte_size;
                compact_offset - member.byte_size
            } else {
                next_offset += member.byte_size;
                next_offset - member.byte_size
            };
            planned_allocation_epochs.insert(member.id.clone(), epoch);
            planned_slots.insert(
                member.id.clone(),
                ColumnSlot {
                    id: member.id.clone(),
                    offset,
                    byte_size: member.byte_size,
                    len_values: member.len_values,
                    generation: next_generation,
                    min: staged.min,
                    max: staged.max,
                    min_positive: staged.min_positive,
                    uniform_step: staged.uniform_step,
                },
            );
        }
        let next_offset = compact_offset;
        let mut planned_free = Vec::new();
        if next_offset < capacity {
            planned_free
                .try_reserve_exact(1)
                .map_err(reserve_failed("batch free list"))?;
            planned_free.push(FreeRegion {
                offset: next_offset,
                size: capacity - next_offset,
            });
        }

        // 9) The candidate registries: the live ones, less the removals, with
        //    every uploaded record and ragged column in its final form.
        let mut records = HashMap::new();
        records
            .try_reserve(self.records.len().saturating_add(uploads.len()))
            .map_err(reserve_failed("batch record registry"))?;
        records.extend(
            self.records
                .iter()
                .map(|(id, slot)| (id.clone(), slot.clone())),
        );
        let mut raggeds = HashMap::new();
        raggeds
            .try_reserve(self.raggeds.len().saturating_add(uploads.len()))
            .map_err(reserve_failed("batch ragged column registry"))?;
        raggeds.extend(
            self.raggeds
                .iter()
                .map(|(id, slot)| (id.clone(), slot.clone())),
        );
        let mut triangles = HashMap::new();
        triangles
            .try_reserve(self.triangles.len())
            .map_err(reserve_failed("batch triangle registry"))?;
        triangles.extend(
            self.triangles
                .iter()
                .map(|(id, slot)| (id.clone(), slot.clone())),
        );
        for id in removals {
            records.remove(*id);
            raggeds.remove(*id);
            triangles.remove(*id);
        }
        let mut cursor = 0usize;
        for (id, source) in uploads {
            // The members of an upload are contiguous, and the first one's
            // epoch is the composite's.
            let epoch = first_epoch + 1 + cursor as u64;
            match source {
                BatchSource::Column(_) | BatchSource::Staged(_) => cursor += 1,
                BatchSource::Record(record) => {
                    let group = &members[cursor..cursor + record.fields.len()];
                    let mut fields: [Option<ColumnId>; MAX_RECORD_FIELDS] = Default::default();
                    let mut tallies = [(0usize, ValueTally::EMPTY); MAX_RECORD_FIELDS];
                    for (member, tally) in group.iter().zip(&mut tallies) {
                        if let MemberRole::Field { slot, tally: field } = member.role {
                            fields[slot] = Some(member.id.clone());
                            *tally = (slot, field);
                        }
                    }
                    let rows = u32::try_from(group[0].len_values).map_err(|_| {
                        crate::FiggyError::internal("a checked record has u32 rows")
                    })?;
                    records.insert(
                        (*id).to_owned(),
                        RecordSlot {
                            schema: record.schema,
                            fields,
                            len: rows,
                            epoch,
                            stats: record_stats(rows, tallies.into_iter().take(group.len())),
                        },
                    );
                    cursor += group.len();
                }
                BatchSource::Ragged(ragged) => {
                    let shape = check_ragged_source(id, ragged)?;
                    let (MemberRole::Values(values), MemberRole::Offsets(offsets)) =
                        (&members[cursor].role, &members[cursor + 1].role)
                    else {
                        return Err(crate::FiggyError::internal(
                            "a ragged upload stages its values, then its offsets",
                        ));
                    };
                    raggeds.insert(
                        (*id).to_owned(),
                        RaggedSlot {
                            values: members[cursor].id.clone(),
                            offsets: members[cursor + 1].id.clone(),
                            groups: shape.groups,
                            total: shape.total32,
                            epoch,
                            stats: offsets.ragged_stats(&shape, values),
                        },
                    );
                    cursor += 2;
                }
            }
        }

        // 10) The compact columns' pipeline and transient buffer (#317), and
        //     the candidate slab, while the batch can still fail.
        let expansion = if transient_bytes > 0 {
            let limits = limits.ok_or(AllocError::Internal {
                invariant: "a compact column was staged on a device that expands",
            })?;
            Some(PreparedExpansion {
                expand: ColumnExpand::ensure(&mut self.expand, device)?.clone(),
                limits,
                transient: super::expand::create_transient(device, ctx.ledger, transient_bytes)?,
            })
        } else {
            None
        };
        let primary = create_buffer_checked(
            device,
            ctx.ledger,
            &BufferDescriptor {
                label: Some("figgy batch column pool candidate"),
                size: capacity,
                usage: BufferUsages::VERTEX
                    | BufferUsages::STORAGE
                    | BufferUsages::COPY_DST
                    | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "batch column pool candidate",
        )?;

        // ── Nothing below can fail. ───────────────────────────────────────
        // A chunked upload's staging is unmapped for its copy only here, so
        // a batch refused above leaves it mapped and the upload usable.
        for member in &members {
            if let MemberInput::Staged(written) = member.source {
                written.unmap_for_install();
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy batch column batch"),
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
        // Pair members copy into the candidate. A compact member's half
        // copies into the transient buffer at half its offset in the compact
        // run, so the transient buffer is the run's compact half and one
        // expansion writes the whole run, after the pair copies and into
        // bytes no copy touches.
        for (member, staged) in members
            .iter()
            .filter_map(|member| member.staged.as_ref().map(|staged| (member, staged)))
        {
            let slot = &planned_slots[&member.id];
            match expansion.as_ref() {
                Some(prepared) if member.lanes.compact => {
                    encoder.copy_buffer_to_buffer(
                        &staged.staging,
                        0,
                        &prepared.transient,
                        (slot.offset - compact_run) / 2,
                        member.lanes.staged_bytes(slot.byte_size),
                    );
                }
                _ => encoder.copy_buffer_to_buffer(
                    &staged.staging,
                    0,
                    &primary,
                    slot.offset,
                    slot.byte_size,
                ),
            }
        }
        if let Some(prepared) = expansion.as_ref() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy batch column expand"),
                timestamp_writes: None,
            });
            prepared.expand.dispatch_run(
                device,
                &mut pass,
                prepared.limits,
                &prepared.transient,
                &primary,
                compact_run,
                next_offset - compact_run,
            );
        }
        queue.submit(std::iter::once(encoder.finish()));

        let candidate = ColumnPool {
            identity: self.identity.clone(),
            primary,
            capacity,
            slots: planned_slots,
            records,
            raggeds,
            triangles,
            free: planned_free,
            generation: next_generation,
            allocation_epochs: planned_allocation_epochs,
            allocation_epoch_counter: last_epoch,
            layout_generation: next_layout_generation,
            backup: None,
            defrag_policy: self.defrag_policy,
            growth_policy: self.growth_policy,
            // The live pool keeps its pipeline; commit moves the layout only.
            expand: None,
        };
        // The candidate carries the new tracked slab and layout across to
        // `commit`; dropping it uncommitted retires the slab in the ledger.
        // The staging buffers drop on return; the submitted copies keep them
        // device-resident until queue completion.
        Ok(ColumnSourcesBatch {
            pool: self,
            candidate,
        })
    }
}
