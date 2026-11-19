//! Records, ragged columns and triangle meshes held as member columns.

use super::*;

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

    /// The triangle index column `id`, when one is resident (#210).
    pub fn triangles(&self, id: &str) -> Option<TriangleRef<'_>> {
        self.triangles
            .get(id)
            .map(|slot| TriangleRef { pool: self, slot })
    }

    /// The scalar statistics of the triangle index column `id`.
    pub fn triangle_stats(&self, id: &str) -> Option<TriangleStats> {
        self.triangles.get(id).map(|slot| slot.stats)
    }

    /// What `id` names when it is a record, a ragged or triangle column or
    /// one of their member columns, for the renderer's id checks.
    pub(crate) fn composite_kind(&self, id: &str) -> Option<&'static str> {
        if self.records.contains_key(id) {
            Some("a record")
        } else if self.raggeds.contains_key(id) {
            Some("a ragged column")
        } else if self.triangles.contains_key(id) {
            Some("a triangle index column")
        } else if self.is_member_column(id) {
            Some("a member column of a record, ragged or triangle column")
        } else {
            None
        }
    }

    /// Whether `id` is a member column of a resident record, ragged or
    /// triangle column.
    pub(crate) fn is_member_column(&self, id: &str) -> bool {
        let Some((owner, _)) = id.split_once(MEMBER_SEPARATOR) else {
            return false;
        };
        if let Some(record) = self.records.get(owner) {
            return record.fields.iter().flatten().any(|field| field == id);
        }
        if let Some(triangles) = self.triangles.get(owner) {
            return triangles.indices == id;
        }
        self.raggeds
            .get(owner)
            .is_some_and(|ragged| ragged.values == id || ragged.offsets == id)
    }

    /// The composite kinds other than `kind` that already hold `id`: a
    /// record, a ragged or a triangle column id is free only when no other
    /// composite holds it (they share one id space).
    pub(super) fn check_composite_kind_free(&self, id: &str, kind: &str) -> crate::Result<()> {
        let taken = if kind != "record" && self.records.contains_key(id) {
            Some("a record")
        } else if kind != "ragged column" && self.raggeds.contains_key(id) {
            Some("a ragged column")
        } else if kind != "triangle index column" && self.triangles.contains_key(id) {
            Some("a triangle index column")
        } else {
            None
        };
        match taken {
            Some(other) => Err(composite_error(
                id,
                crate::RuleKind::IdInUse {
                    other: other.to_string(),
                },
            )),
            None => Ok(()),
        }
    }

    pub(super) fn check_composite_id(&self, id: &str, kind: &str) -> crate::Result<()> {
        if id.is_empty() {
            return Err(composite_error(
                id,
                crate::RuleKind::CompositeIdEmpty {
                    kind: kind.to_string(),
                },
            ));
        }
        if id.contains(MEMBER_SEPARATOR) {
            return Err(composite_error(
                id,
                crate::RuleKind::CompositeIdSeparator {
                    kind: kind.to_string(),
                },
            ));
        }
        if self.slots.contains_key(id) {
            return Err(composite_error(
                id,
                crate::RuleKind::CompositeIdIsColumn {
                    kind: kind.to_string(),
                },
            ));
        }
        Ok(())
    }

    /// The token that keeps a replacement's member ids distinct from the
    /// members it replaces: the allocation epoch its first member will get.
    pub(super) fn member_token(&self) -> crate::Result<u64> {
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
        self.check_composite_kind_free(&id, "record")?;
        let schema = source.schema;
        let (by_slot, rows) = check_record_source(&id, source)?;
        // Member ids in schema order; `order[i]` is the schema slot of batch
        // column `i`.
        let token = self.member_token()?;
        let mut members: [Option<ColumnId>; MAX_RECORD_FIELDS] = Default::default();
        for (slot, (name, _)) in schema.iter().enumerate() {
            if by_slot[slot].is_some() {
                members[slot] = Some(member_id(&id, token, name));
            }
        }
        // The batch columns are exactly the slots that have both a member id
        // and a source; the loop above gives every present source an id.
        let placeholder = source.fields[0].1;
        let mut columns: [(&str, &dyn ColumnSource); MAX_RECORD_FIELDS] =
            [("", placeholder); MAX_RECORD_FIELDS];
        let mut order = [0usize; MAX_RECORD_FIELDS];
        let mut count = 0usize;
        for (slot, (member, field)) in members.iter().zip(by_slot).enumerate() {
            if let (Some(member), Some(field)) = (member.as_deref(), field) {
                columns[count] = (member, field);
                order[count] = slot;
                count += 1;
            }
        }

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
        let epoch =
            batch
                .pool
                .allocation_epoch(columns[0].0)
                .ok_or(crate::FiggyError::internal(
                    "a provisional member has an allocation epoch",
                ))?;
        let value = finish_member_batch(batch, &old_members[..old_count], |pool| {
            prepare(pool, relocated)
        })?;

        let stats = record_stats(
            rows,
            order
                .iter()
                .copied()
                .zip(tallies.iter().copied())
                .take(count),
        );
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
        self.check_composite_kind_free(&id, "ragged column")?;
        let shape = check_ragged_source(&id, source)?;

        let token = self.member_token()?;
        let values_id = member_id(&id, token, "values");
        let offsets_id = member_id(&id, token, "offsets");
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
        let mut offsets = OffsetTally::new();
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
        // A rejection drops `batch`, which un-inserts both members; their
        // bytes went to regions that were free, so nothing live changed.
        offsets.check(&id, &shape)?;
        let relocated = batch.pool.layout_generation != layout_before;
        let epoch = batch
            .pool
            .allocation_epoch(&values_id)
            .ok_or(crate::FiggyError::internal(
                "a provisional member has an allocation epoch",
            ))?;
        let value = finish_member_batch(batch, &old_members[..old_count], |pool| {
            prepare(pool, relocated)
        })?;

        let stats = offsets.ragged_stats(&shape, &values);
        self.raggeds.insert(
            id,
            RaggedSlot {
                values: values_id,
                offsets: offsets_id,
                groups: shape.groups,
                total: shape.total32,
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

    /// Insert or replace the triangle index column `id` (failure-atomic,
    /// #210).
    ///
    /// The indices upload as one member column stored as `u32` lanes
    /// (SHADER_COMMON.md §5.14); the write observer checks them in the same
    /// pass and measures the [`TriangleStats`]. The source must write every
    /// index once, in row order, each a finite integer in `0..=u32::MAX`,
    /// and hold a non-zero multiple of 3 indices; anything else is rejected
    /// before anything is published. A replacement needs room for the new
    /// member while the old one is live, as [`Self::upsert_record`]
    /// describes.
    pub fn upsert_triangles(
        &mut self,
        id: impl Into<TriangleId>,
        source: &TriangleSource<'_>,
        ctx: GpuAllocCtx<'_>,
    ) -> crate::Result<TriangleId> {
        let id = id.into();
        self.upsert_triangles_with(id.clone(), source, ctx, |_, _| Ok(()))?;
        Ok(id)
    }

    /// [`Self::upsert_triangles`] with a `prepare` step, as
    /// [`Self::upsert_record_with`].
    pub(crate) fn upsert_triangles_with<T>(
        &mut self,
        id: TriangleId,
        source: &TriangleSource<'_>,
        ctx: GpuAllocCtx<'_>,
        prepare: impl FnOnce(&ColumnPool, bool) -> crate::Result<T>,
    ) -> crate::Result<T> {
        self.check_composite_id(&id, "triangle index column")?;
        self.check_composite_kind_free(&id, "triangle index column")?;
        let count = source.indices.len();
        if count == 0 {
            return Err(composite_error(&id, crate::RuleKind::TrianglesEmpty));
        }
        if count % 3 != 0 {
            return Err(composite_error(
                &id,
                crate::RuleKind::TriangleIndexCount {
                    count: count as u64,
                },
            ));
        }
        // The shader reads index `3t + k` by its `u32` lane: the index count
        // is the GPU's index space, as a ragged column's value count is.
        let triangles = u32::try_from(count / 3)
            .ok()
            .filter(|_| u32::try_from(count).is_ok())
            .ok_or_else(|| {
                composite_error(
                    &id,
                    crate::RuleKind::TriangleIndicesOverflow {
                        max: u64::from(u32::MAX),
                        count: count as u64,
                    },
                )
            })?;

        let token = self.member_token()?;
        let indices_id = format!("{id}{MEMBER_SEPARATOR}{token}{MEMBER_SEPARATOR}indices");
        let columns: [(&str, &dyn ColumnSource); 1] = [(indices_id.as_str(), source.indices)];
        let old = self.triangles.get(&id).map(|slot| slot.indices.clone());
        let old_members: [&str; 1] = [old.as_deref().unwrap_or("")];
        let old_count = usize::from(old.is_some());

        self.triangles.try_reserve(1).map_err(|error| {
            crate::FiggyError::StateAllocationFailed {
                resource: "triangle column registry",
                reason: error.to_string(),
            }
        })?;
        let layout_before = self.layout_generation;
        let mut tally = IndexTally {
            next_row: 0,
            in_order: true,
            integral: true,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        };
        let mut observe = |_column: usize, row: usize, hi: f32, lo: f32| {
            tally.record(row, hi, lo);
        };
        let observer: &mut dyn FnMut(usize, usize, f32, f32) = &mut observe;
        let batch = self.begin_add_columns_observed(&columns, ctx, Some(observer), Some(0))?;
        // Rejections below drop `batch`, which un-inserts the member; its
        // bytes went to a region that was free, so nothing live changed.
        if !tally.in_order || tally.next_row != count {
            return Err(composite_error(&id, crate::RuleKind::TriangleIndicesOrder));
        }
        if !tally.integral {
            return Err(composite_error(
                &id,
                crate::RuleKind::TriangleIndexRange {
                    max: u64::from(u32::MAX),
                },
            ));
        }
        let relocated = batch.pool.layout_generation != layout_before;
        let epoch = batch
            .pool
            .allocation_epoch(&indices_id)
            .ok_or(crate::FiggyError::internal(
                "a provisional member has an allocation epoch",
            ))?;
        let value = finish_member_batch(batch, &old_members[..old_count], |pool| {
            prepare(pool, relocated)
        })?;

        // Every index was checked to be an integer in `0..=u32::MAX`.
        let stats = TriangleStats {
            triangles,
            min_index: tally.min as u32,
            max_index: tally.max as u32,
        };
        self.triangles.insert(
            id,
            TriangleSlot {
                indices: indices_id,
                triangles,
                epoch,
                stats,
            },
        );
        Ok(value)
    }

    /// Delete the triangle index column `id` and free its member column.
    /// `false` when no such column is resident.
    pub fn remove_triangles(&mut self, id: &str) -> crate::Result<bool> {
        Ok(self.remove_triangles_with(id, |_| Ok(()))?.is_some())
    }

    /// [`Self::remove_triangles`] with a `prepare` step, as
    /// [`Self::remove_record_with`].
    pub(crate) fn remove_triangles_with<T>(
        &mut self,
        id: &str,
        prepare: impl FnOnce(&ColumnPool) -> crate::Result<T>,
    ) -> crate::Result<Option<T>> {
        let Some(slot) = self.triangles.get(id) else {
            return Ok(None);
        };
        let indices = slot.indices.clone();
        let removal = self.begin_remove_columns(&[indices.as_str()])?;
        let value = prepare(&*removal.pool)?;
        removal.commit();
        self.triangles.remove(id);
        Ok(Some(value))
    }
}
