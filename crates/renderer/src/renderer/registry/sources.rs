//! Records, ragged columns, triangle indices and column removal.

use super::*;

impl Renderer {
    /// Insert or replace the record `id`: named scalar fields of one length,
    /// checked by name against `source.schema` and uploaded as one batch
    /// transaction ([`ColumnPool::upsert_record`]). Records are resident
    /// only: a streamed id is rejected, and so is an id that names a column
    /// or a ragged column. Failure-atomic, as [`Self::add_columns`]; the
    /// active picker is prepared against the final pool before publication.
    pub fn upsert_record(
        &mut self,
        id: impl Into<RecordId>,
        source: &RecordSource<'_>,
    ) -> Result<RecordId> {
        let result = self.upsert_record_unreported(id, source);
        self.reported(result)
    }

    pub(super) fn upsert_record_unreported(
        &mut self,
        id: impl Into<RecordId>,
        source: &RecordSource<'_>,
    ) -> Result<RecordId> {
        self.health.check()?;
        let id = id.into();
        self.reject_streamed_composite(&id, "a record")?;
        let old = self
            .pool
            .record(&id)
            .map(|record| record.slot().fields.clone())
            .unwrap_or_default();
        let mut changed = [""; data_render::MAX_RECORD_FIELDS + 1];
        changed[0] = id.as_str();
        let mut count = 1usize;
        for member in old.iter().flatten() {
            changed[count] = member;
            count += 1;
        }
        let replaced = count > 1;
        let changed = &changed[..count];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let ctx = pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger);
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let (relocated, picker) =
            self.pool
                .upsert_record_with(id.clone(), source, ctx, move |pool, relocated| {
                    let picker = prepare_picker_for_pool_mutation(
                        picker_state,
                        pool,
                        chart_states,
                        relocated,
                        changed,
                    )?;
                    Ok((relocated, picker))
                })?;
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.settle_composite_defrag(relocated, replaced);
        Ok(id)
    }

    /// Insert or replace the ragged column `id`: groups of values with
    /// offsets, uploaded as one batch transaction whose write observer
    /// rejects offsets that do not start at 0, decrease, or do not end at the
    /// value count ([`ColumnPool::upsert_ragged`]). Resident only, as
    /// [`Self::upsert_record`].
    pub fn upsert_ragged(
        &mut self,
        id: impl Into<RaggedId>,
        source: &RaggedSource<'_>,
    ) -> Result<RaggedId> {
        let result = self.upsert_ragged_unreported(id, source);
        self.reported(result)
    }

    pub(super) fn upsert_ragged_unreported(
        &mut self,
        id: impl Into<RaggedId>,
        source: &RaggedSource<'_>,
    ) -> Result<RaggedId> {
        self.health.check()?;
        let id = id.into();
        self.reject_streamed_composite(&id, "a ragged column")?;
        let old = self
            .pool
            .ragged(&id)
            .map(|ragged| (ragged.slot().values.clone(), ragged.slot().offsets.clone()));
        let changed_ids = match old.as_ref() {
            Some((values, offsets)) => [id.as_str(), values.as_str(), offsets.as_str()],
            None => [id.as_str(), "", ""],
        };
        let replaced = old.is_some();
        let changed = &changed_ids[..if replaced { 3 } else { 1 }];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let ctx = pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger);
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let (relocated, picker) =
            self.pool
                .upsert_ragged_with(id.clone(), source, ctx, move |pool, relocated| {
                    let picker = prepare_picker_for_pool_mutation(
                        picker_state,
                        pool,
                        chart_states,
                        relocated,
                        changed,
                    )?;
                    Ok((relocated, picker))
                })?;
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.settle_composite_defrag(relocated, replaced);
        Ok(id)
    }

    /// Delete the record `id` and free its member columns. `false` when no
    /// such record is resident.
    pub fn remove_record(&mut self, id: &str) -> Result<bool> {
        let result = self.remove_record_unreported(id);
        self.reported(result)
    }

    pub(super) fn remove_record_unreported(&mut self, id: &str) -> Result<bool> {
        self.health.check()?;
        let Some(record) = self.pool.record(id) else {
            return Ok(false);
        };
        let old = record.slot().fields.clone();
        let mut changed = [""; data_render::MAX_RECORD_FIELDS + 1];
        changed[0] = id;
        let mut count = 1usize;
        for member in old.iter().flatten() {
            changed[count] = member;
            count += 1;
        }
        let changed = &changed[..count];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let picker = self.pool.remove_record_with(id, move |pool| {
            prepare_picker_for_pool_mutation(picker_state, pool, chart_states, false, changed)
        })?;
        let Some(picker) = picker else {
            return Ok(false);
        };
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.pending_defrag = true;
        Ok(true)
    }

    /// Delete the ragged column `id` and free its two member columns.
    /// `false` when no such ragged column is resident.
    pub fn remove_ragged(&mut self, id: &str) -> Result<bool> {
        let result = self.remove_ragged_unreported(id);
        self.reported(result)
    }

    pub(super) fn remove_ragged_unreported(&mut self, id: &str) -> Result<bool> {
        self.health.check()?;
        let Some(ragged) = self.pool.ragged(id) else {
            return Ok(false);
        };
        let (values, offsets) = (ragged.slot().values.clone(), ragged.slot().offsets.clone());
        let changed_ids = [id, values.as_str(), offsets.as_str()];
        let changed = &changed_ids[..];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let picker = self.pool.remove_ragged_with(id, move |pool| {
            prepare_picker_for_pool_mutation(picker_state, pool, chart_states, false, changed)
        })?;
        let Some(picker) = picker else {
            return Ok(false);
        };
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.pending_defrag = true;
        Ok(true)
    }

    /// Insert or replace the triangle index column `id` (#210): three vertex
    /// indices per triangle, uploaded as one member column whose write
    /// observer rejects a length that is not a non-zero multiple of 3 or an
    /// index that is not an integer in `0..=u32::MAX`
    /// ([`ColumnPool::upsert_triangles`]). Resident only, as
    /// [`Self::upsert_record`].
    pub fn upsert_triangles(
        &mut self,
        id: impl Into<TriangleId>,
        source: &TriangleSource<'_>,
    ) -> Result<TriangleId> {
        let result = self.upsert_triangles_unreported(id, source);
        self.reported(result)
    }

    pub(super) fn upsert_triangles_unreported(
        &mut self,
        id: impl Into<TriangleId>,
        source: &TriangleSource<'_>,
    ) -> Result<TriangleId> {
        self.health.check()?;
        let id = id.into();
        self.reject_streamed_composite(&id, "a triangle index column")?;
        let old = self
            .pool
            .triangles(&id)
            .map(|triangles| triangles.slot().indices.clone());
        let changed_ids = match old.as_deref() {
            Some(indices) => [id.as_str(), indices],
            None => [id.as_str(), ""],
        };
        let replaced = old.is_some();
        let changed = &changed_ids[..if replaced { 2 } else { 1 }];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let ctx = pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger);
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let (relocated, picker) =
            self.pool
                .upsert_triangles_with(id.clone(), source, ctx, move |pool, relocated| {
                    let picker = prepare_picker_for_pool_mutation(
                        picker_state,
                        pool,
                        chart_states,
                        relocated,
                        changed,
                    )?;
                    Ok((relocated, picker))
                })?;
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.settle_composite_defrag(relocated, replaced);
        Ok(id)
    }

    /// Delete the triangle index column `id` and free its member column.
    /// `false` when no such column is resident.
    pub fn remove_triangles(&mut self, id: &str) -> Result<bool> {
        let result = self.remove_triangles_unreported(id);
        self.reported(result)
    }

    pub(super) fn remove_triangles_unreported(&mut self, id: &str) -> Result<bool> {
        self.health.check()?;
        let Some(triangles) = self.pool.triangles(id) else {
            return Ok(false);
        };
        let indices = triangles.slot().indices.clone();
        let changed_ids = [id, indices.as_str()];
        let changed = &changed_ids[..];
        let invalidation = self.prepare_columns_invalidation(changed.iter().copied())?;
        let (picker_state, chart_states) = (&mut self.picker, &self.chart_states);
        let picker = self.pool.remove_triangles_with(id, move |pool| {
            prepare_picker_for_pool_mutation(picker_state, pool, chart_states, false, changed)
        })?;
        let Some(picker) = picker else {
            return Ok(false);
        };
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        self.pending_defrag = true;
        Ok(true)
    }

    /// Records, ragged and triangle columns are resident only: streaming is
    /// not extended to them (CLAUDE.md rule 8).
    pub(super) fn reject_streamed_composite(&self, id: &str, kind: &str) -> Result<()> {
        if self.streaming_sources.contains_key(id) {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: id.to_owned(),
                kind: crate::RuleKind::CompositeNeedsResident {
                    kind: kind.to_string(),
                },
            });
        }
        Ok(())
    }

    /// Maintenance after a record, ragged or triangle upsert: freed old members leave
    /// holes worth compacting; a relayout without them already compacted.
    pub(super) fn settle_composite_defrag(&mut self, relocated: bool, replaced: bool) {
        if replaced {
            self.pending_defrag = true;
        } else if relocated {
            self.pending_defrag = false;
        }
    }

    /// Remove a column and cascade-remove every renderer-owned series that
    /// references it, so no registered chart can retain a freed data id.
    ///
    /// The core renderer does not rewrite `Config::legend`; legend document
    /// policy belongs to the host. The web wrapper removes rows for its
    /// auto-managed legend and preserves freely edited legend text.
    pub fn remove_column(&mut self, id: &str) -> Result<bool> {
        let result = self.remove_column_unreported(id);
        self.reported(result)
    }

    pub(super) fn remove_column_unreported(&mut self, id: &str) -> Result<bool> {
        self.health.check()?;
        self.remove_column_impl(id, None)
    }

    /// Remove a column while atomically replacing one chart's config.
    ///
    /// This is the host boundary for a derived legend update caused by the
    /// series cascade. A missing column returns `false` without validating or
    /// publishing `next_config`. All fallible validation, revision, pool, and
    /// picker preparation completes before either authority is published.
    pub fn remove_column_with_chart_config(
        &mut self,
        id: &str,
        chart_id: ChartId,
        next_config: Config,
    ) -> Result<bool> {
        let result = self.remove_column_with_chart_config_unreported(id, chart_id, next_config);
        self.reported(result)
    }

    pub(super) fn remove_column_with_chart_config_unreported(
        &mut self,
        id: &str,
        chart_id: ChartId,
        next_config: Config,
    ) -> Result<bool> {
        self.health.check()?;
        self.remove_column_impl(id, Some((chart_id, next_config)))
    }
}
