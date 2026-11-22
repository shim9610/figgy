//! Adding, upserting and invalidating columns.

use super::*;

impl Renderer {
    pub(in crate::renderer) fn prepare_column_invalidation(
        &self,
        id: &str,
    ) -> Result<ColumnInvalidationPlan> {
        self.prepare_columns_invalidation(std::iter::once(id))
    }

    /// The invalidation plan for a mutation that touches several columns at
    /// once. One pass over the charts, whatever the batch size.
    pub(in crate::renderer) fn prepare_columns_invalidation<'i>(
        &self,
        ids: impl Iterator<Item = &'i str> + Clone,
    ) -> Result<ColumnInvalidationPlan> {
        let mut charts = Vec::new();
        charts.try_reserve(self.chart_count()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "column invalidation revisions",
                reason: error.to_string(),
            }
        })?;
        for chart_id in self.every_chart() {
            let state = self
                .chart_states
                .get(chart_id)
                .ok_or(FiggyError::UnknownChart { id: *chart_id })?;
            if series_list_references_any_column(&state.series, ids.clone()) {
                charts.push(ChartColumnInvalidation {
                    id: *chart_id,
                    desired: state
                        .revisions
                        .desired
                        .successor("chart desired revision")?,
                    data: state.revisions.data.successor("chart data revision")?,
                });
            }
        }
        let visual = if charts.is_empty() {
            None
        } else {
            Some(self.visual_revision.successor("renderer visual revision")?)
        };
        Ok(ColumnInvalidationPlan { charts, visual })
    }

    pub(in crate::renderer) fn prepare_column_removal(
        &self,
        id: &str,
        config_override: Option<(ChartId, Config)>,
    ) -> Result<ColumnRemovalPlan> {
        let mut charts = Vec::new();
        charts.try_reserve(self.chart_count()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "column removal candidates",
                reason: error.to_string(),
            }
        })?;
        let mut config_override = config_override;
        for chart_id in self.every_chart() {
            let state = self
                .chart_states
                .get(chart_id)
                .ok_or(FiggyError::UnknownChart { id: *chart_id })?;
            let remove_referencing_series = state
                .series
                .iter()
                .any(|series| series_references_column(series, id));
            let replace_config = config_override
                .as_ref()
                .is_some_and(|(override_chart, _)| override_chart == chart_id);
            if !remove_referencing_series && !replace_config {
                continue;
            }

            let desired = state
                .revisions
                .desired
                .successor("chart desired revision")?;
            let referencing = if remove_referencing_series {
                Some(SeriesRemovalRevisions {
                    series: state.revisions.series.successor("chart series revision")?,
                    data: state.revisions.data.successor("chart data revision")?,
                })
            } else {
                None
            };
            let replacement = config_override
                .take_if(|(override_chart, _)| override_chart == chart_id)
                .map(|(_, config)| -> Result<ChartConfigReplacement> {
                    let view = if stream_config_equal(&state.config, &config, &state.series) {
                        None
                    } else {
                        Some(state.revisions.view.successor("chart view revision")?)
                    };
                    Ok(ChartConfigReplacement {
                        config_revision: state
                            .revisions
                            .config
                            .successor("chart config revision")?,
                        selection: selection_config_revision(
                            state.revisions.selection,
                            &state.config,
                            &config,
                        )?,
                        annotation: annotation_config_revision(
                            state.revisions.annotation,
                            &state.config,
                            &config,
                        )?,
                        view,
                        config,
                    })
                })
                .transpose()?;
            let raster = state.revisions.raster.successor("chart raster revision")?;
            charts.push(ChartSeriesRemoval {
                id: *chart_id,
                referencing,
                replacement,
                desired,
                raster,
            });
        }
        // An override for a chart that is not registered names no chart.
        if let Some((chart_id, _)) = config_override {
            return Err(FiggyError::UnknownChart { id: chart_id });
        }
        let visual = if charts.is_empty() {
            None
        } else {
            Some(self.visual_revision.successor("renderer visual revision")?)
        };
        Ok(ColumnRemovalPlan { charts, visual })
    }

    pub(in crate::renderer) fn remove_column_impl(
        &mut self,
        id: &str,
        config_override: Option<(ChartId, Config)>,
    ) -> Result<bool> {
        if self.pool.slot(id).is_none() && !self.streaming_sources.contains_key(id) {
            return Ok(false);
        }
        // A record's member and a monotonic envelope (#271) are the
        // renderer's to remove.
        if self.pool.is_member_column(id) || self.monotonic.owns_column(id) {
            return Err(FiggyError::ReservedColumnId { id: id.to_owned() });
        }
        if let Some((chart_id, config)) = config_override.as_ref() {
            crate::chart::validate_renderer_config(config)?;
            if !self.chart_states.contains_key(chart_id) {
                return Err(FiggyError::UnknownChart { id: *chart_id });
            }
        }

        let active_plan = match self.picker.active.as_ref() {
            Some(active) => {
                let state =
                    self.chart_states
                        .get(&active.chart_id)
                        .ok_or(FiggyError::UnknownChart {
                            id: active.chart_id,
                        })?;
                let removes_series = state
                    .series
                    .iter()
                    .any(|series| series_references_column(series, id));
                let override_config = config_override
                    .as_ref()
                    .filter(|(chart_id, _)| *chart_id == active.chart_id)
                    .map(|(_, config)| config);
                if removes_series || override_config.is_some() {
                    let candidate = removes_series
                        .then(|| series_after_column_removal(&state.series, id))
                        .transpose()?;
                    Some((
                        active.chart_id,
                        PickChartPlan::for_active_layer_in(
                            &self.pool,
                            override_config.unwrap_or(&state.config),
                            candidate.as_deref().unwrap_or(&state.series),
                        )?,
                    ))
                } else {
                    None
                }
            }
            None => None,
        };
        let removed_column = id.to_string();
        let removal = self.prepare_column_removal(id, config_override)?;
        if self.streaming_sources.contains_key(id) {
            let picker = if let Some((chart_id, plan)) = active_plan {
                self.picker
                    .prepare_active_pool_mutation(&self.pool, chart_id, plan, &[])?
            } else {
                None
            };
            self.streaming_sources.remove(id);
            self.stream_revision_lineage.remove(id);
            removal.publish(
                &mut self.chart_states,
                &mut self.visual_revision,
                &removed_column,
            );
            if let Some(picker) = picker {
                picker.commit();
            }
            return Ok(true);
        }
        let Some(inner) = self.pool.begin_remove_column(id)? else {
            return Ok(false);
        };
        let picker = if let Some((chart_id, plan)) = active_plan {
            self.picker
                .prepare_active_pool_mutation(inner.pool(), chart_id, plan, &[])?
        } else {
            None
        };
        let removed = PreparedColumnRemoval {
            inner,
            chart_states: &mut self.chart_states,
            visual_revision: &mut self.visual_revision,
            pending_defrag: &mut self.pending_defrag,
            removal,
            picker,
            removed_column,
        }
        .commit();
        if removed {
            self.stream_revision_lineage.remove(id);
        }
        Ok(removed)
    }

    /// The revision of column `id`: a streamed source's declared revision, or
    /// the allocation a resident column's latest upload made, which every
    /// upload renews. `None` for an id the renderer does not hold (#160).
    pub fn column_revision(&self, id: &str) -> Option<u64> {
        self.streaming_sources
            .get(id)
            .map(|source| source.column.revision)
            .or_else(|| self.pool.allocation_epoch(id))
    }

    /// The id of every logical column: each streamed source and each resident
    /// column a host registered (record and ragged member columns excepted),
    /// in no particular order (#160).
    pub fn logical_column_ids(&self) -> impl Iterator<Item = &str> + '_ {
        self.streaming_sources.keys().map(String::as_str).chain(
            self.pool
                .column_ids()
                .filter(|id| !self.streaming_sources.contains_key(*id)),
        )
    }

    /// Look up either logical host metadata or the authoritative resident slot.
    pub fn logical_column(&self, id: &str) -> Option<crate::LogicalColumn<'_>> {
        self.streaming_sources
            .get(id)
            .map(|source| crate::LogicalColumn::Streamed(&source.column))
            .or_else(|| self.pool.slot(id).map(crate::LogicalColumn::Resident))
    }

    /// Prepare one commit that uploads `columns` (new ids, or replacements
    /// of resident ones) and replaces chart `chart_id`'s config and series:
    /// [`Self::begin_replace_chart_sources`] with plain columns only and no
    /// removals, whose commit returns the columns' handles in order.
    ///
    /// `prepare_state` receives the complete candidate pool, so the chart's
    /// axis ranges can be derived from the new columns before anything is
    /// published or extent work is submitted. Other charts that read a
    /// replaced column move their data revision. Dropping the returned guard
    /// leaves every column and chart as it was; [`ChartColumnsReplacement::commit`]
    /// publishes them all. A streamed column id is refused.
    pub fn begin_replace_chart_columns<const N: usize, F>(
        &mut self,
        chart_id: ChartId,
        columns: [(&str, &dyn ColumnSource); N],
        prepare_state: F,
    ) -> Result<ChartColumnsReplacement<'_, N>>
    where
        F: FnOnce(&ColumnPool) -> Result<(Config, Vec<SeriesConfig>)>,
    {
        let uploads = columns.map(|(id, source)| (id, BatchSource::Column(source)));
        let inner = self.begin_replace_chart_sources(chart_id, &uploads, &[], prepare_state)?;
        let handles = inner.inner.handles(&columns.map(|(id, _)| id))?;
        Ok(ChartColumnsReplacement { inner, handles })
    }

    /// Prepare one commit (#311) that uploads `uploads` — plain columns,
    /// chunked uploads ([`BatchSource::Staged`], #334, checked as
    /// [`Self::check_column_upload`] checks one), records and ragged
    /// columns, each inserted under its id or replacing
    /// what the id holds of the same kind —, removes `removals` (plain
    /// columns, records, ragged or triangle index columns no series of
    /// another chart reads) and replaces chart `chart_id`'s config and
    /// series.
    ///
    /// The pool side is one candidate pool
    /// ([`ColumnPool::upsert_record`]'s and [`ColumnPool::upsert_ragged`]'s
    /// rules apply to the records and ragged columns): `prepare_state`
    /// receives it complete, uploads in and removals out, so the chart's
    /// axes can be fitted from the new columns before anything is
    /// published. The returned series are validated against it, the chart
    /// plan and the active picker are prepared, and
    /// [`ChartSourcesReplacement::commit`] publishes the pool, the chart and
    /// the picker together. Dropping the guard, or any error before it is
    /// returned, leaves the pool, every chart, the picker and the device
    /// ledger's live bytes as they were. Other charts that read a replaced
    /// column move their data revision.
    ///
    /// Peak memory: the live slab plus every upload's staging, the compact
    /// columns' transient buffer and a candidate slab of the live capacity,
    /// or a grown one when the survivors and the uploads need it; a memory
    /// budget that cannot hold the peak refuses the call with
    /// [`AllocError::BudgetExceeded`](crate::AllocError) before any buffer
    /// is made. A streamed id, in `uploads` or `removals`, is refused (rule
    /// 8), as is an id twice.
    pub fn begin_replace_chart_sources<'a, F>(
        &'a mut self,
        chart_id: ChartId,
        uploads: &[(&str, BatchSource<'_>)],
        removals: &'a [&'a str],
        prepare_state: F,
    ) -> Result<ChartSourcesReplacement<'a>>
    where
        F: FnOnce(&ColumnPool) -> Result<(Config, Vec<SeriesConfig>)>,
    {
        // The guard borrows the renderer, so the check uses a clone of the
        // health record; a failed check drops the guard, which rolls back.
        let health = self.health.clone();
        let guard = self.begin_replace_chart_sources_unreported(
            chart_id,
            uploads,
            removals,
            prepare_state,
        )?;
        health.check()?;
        Ok(guard)
    }

    /// Install several chunked uploads (#334) and, optionally, chart
    /// `chart_id`'s new config and series, in one commit: the
    /// [`Self::begin_replace_chart_sources`] transaction with every upload a
    /// [`BatchSource::Staged`] source. The pool, the chart and the picker
    /// publish together, or nothing changes.
    ///
    /// Every upload is checked as [`Self::commit_column_upload`] checks one
    /// ([`Self::check_column_upload`]), its revision included, so a column
    /// that changed since its upload began refuses the whole transaction.
    /// Without `state` the chart keeps its config and series, which must
    /// still hold against the new columns (a series whose columns' lengths
    /// no longer agree is refused). A streamed or reserved id, and an id
    /// twice, are refused. The number of uploads is not limited. Peak
    /// memory: as [`Self::begin_replace_chart_sources`], the uploads'
    /// staging already being held. The uploads are consumed either way.
    pub fn commit_column_uploads(
        &mut self,
        chart_id: ChartId,
        uploads: Vec<ColumnUpload>,
        state: Option<(Config, Vec<SeriesConfig>)>,
    ) -> Result<()> {
        let result = self.commit_column_uploads_unreported(chart_id, &uploads, state);
        drop(uploads);
        self.reported(result)
    }

    fn commit_column_uploads_unreported(
        &mut self,
        chart_id: ChartId,
        uploads: &[ColumnUpload],
        state: Option<(Config, Vec<SeriesConfig>)>,
    ) -> Result<()> {
        // The chart keeps its own config and series: the SSOT, read again
        // so they are validated against the candidate pool.
        let (config, series) = match state {
            Some(state) => state,
            None => (
                self.chart_config(chart_id)?.clone(),
                self.chart_series(chart_id)?.to_vec(),
            ),
        };
        // The batch view of the uploads: borrowed, nothing is copied.
        let mut batch: Vec<(&str, BatchSource<'_>)> = Vec::new();
        batch.try_reserve_exact(uploads.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "column transaction uploads",
                reason: error.to_string(),
            }
        })?;
        batch.extend(
            uploads
                .iter()
                .map(|upload| (upload.id(), BatchSource::Staged(upload))),
        );
        let health = self.health.clone();
        let guard =
            self.begin_replace_chart_sources_unreported(chart_id, &batch, &[], move |_| {
                Ok((config, series))
            })?;
        // A failed check drops the guard, which rolls back.
        health.check()?;
        guard.commit();
        Ok(())
    }

    pub(super) fn begin_replace_chart_sources_unreported<'a, F>(
        &'a mut self,
        chart_id: ChartId,
        uploads: &[(&str, BatchSource<'_>)],
        removals: &'a [&'a str],
        prepare_state: F,
    ) -> Result<ChartSourcesReplacement<'a>>
    where
        F: FnOnce(&ColumnPool) -> Result<(Config, Vec<SeriesConfig>)>,
    {
        self.health.check()?;
        for (id, source) in uploads {
            match source {
                BatchSource::Column(_) => self.reject_streamed_id(id)?,
                BatchSource::Staged(upload) => {
                    if upload.id() != *id {
                        return Err(FiggyError::InvalidColumnUpload {
                            id: (*id).to_owned(),
                            reason: "the batch installs a chunked upload under another id",
                        });
                    }
                    self.check_column_upload(upload)?;
                }
                BatchSource::Record(_) => self.reject_streamed_composite(id, "a record")?,
                BatchSource::Ragged(_) => self.reject_streamed_composite(id, "a ragged column")?,
            }
        }
        for id in removals {
            if self.streaming_sources.contains_key(*id) {
                return Err(FiggyError::InvalidStreamSource {
                    id: (*id).to_owned(),
                    reason: "a streamed column is removed with remove_column, \
                             not in a batch transaction",
                });
            }
            if self.monotonic.owns_column(id) {
                return Err(FiggyError::ReservedColumnId {
                    id: (*id).to_owned(),
                });
            }
        }
        // A removed id another chart reads would leave that chart reading
        // nothing: the transaction is refused instead.
        for other in self.every_chart() {
            if *other == chart_id {
                continue;
            }
            let state = self
                .chart_states
                .get(other)
                .ok_or(FiggyError::UnknownChart { id: *other })?;
            if let Some(id) = removals
                .iter()
                .find(|id| series_list_references_any_column(&state.series, std::iter::once(**id)))
            {
                return Err(FiggyError::InvalidColumnUpload {
                    id: (*id).to_owned(),
                    reason: "another chart reads a column the replacement removes",
                });
            }
        }
        // Every id whose contents change: the uploads and the removals.
        let mut changed = Vec::new();
        changed
            .try_reserve_exact(uploads.len().saturating_add(removals.len()))
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "chart replacement column ids",
                reason: error.to_string(),
            })?;
        changed.extend(uploads.iter().map(|(id, _)| *id));
        changed.extend(removals.iter().copied());
        let Renderer {
            device,
            queue,
            pool,
            chart_states,
            chart_order,
            monotonic_order,
            visual_revision,
            pending_defrag,
            picker,
            gpu_ledger,
            stream_revision_lineage,
            ..
        } = self;
        let inner = pool.begin_batch_upsert_sources(
            uploads,
            removals,
            pool_alloc_ctx(&*device, &*queue, &gpu_ledger),
        )?;
        let (config, series) = prepare_state(inner.pool())?;
        super::charts::refuse_monotonic_series(&series)?;
        crate::chart::validate_renderer_config(&config)?;
        validate_renderer_series(inner.pool(), &config, &series)?;
        let chart_plan = prepare_chart_replacement_plan(
            chart_states,
            chart_order,
            monotonic_order,
            *visual_revision,
            chart_id,
            config,
            series,
            &changed,
        )?;

        let picker = match picker.active.as_ref().map(|active| active.chart_id) {
            Some(active_id) => {
                let plan = if active_id == chart_id {
                    PickChartPlan::for_active_layer_in(
                        inner.pool(),
                        &chart_plan.config,
                        &chart_plan.series,
                    )?
                } else {
                    let state = chart_states
                        .get(&active_id)
                        .ok_or(FiggyError::UnknownChart { id: active_id })?;
                    PickChartPlan::for_active_layer_in(inner.pool(), &state.config, &state.series)?
                };
                picker.prepare_active_pool_mutation(inner.pool(), active_id, plan, &changed)?
            }
            None => None,
        };

        Ok(ChartSourcesReplacement {
            inner,
            chart_states,
            visual_revision,
            pending_defrag,
            stream_revision_lineage,
            removals,
            chart_plan,
            picker,
        })
    }

    /// Add a column to the pool. `AllocError` is converted to `FiggyError::Pool`.
    pub fn add_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        let result = self.add_column_unreported(id, source);
        self.reported(result)
    }

    pub(super) fn add_column_unreported(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        self.health.check()?;
        let id = id.into();
        if self.pool.slot(&id).is_some() {
            return Err(data_render::AllocError::DuplicateId(id).into());
        }
        Ok(self.begin_upsert_column(id, source)?.commit())
    }

    pub fn add_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        let result = self.add_hilo_column_unreported(id, source);
        self.reported(result)
    }

    pub(super) fn add_hilo_column_unreported(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        self.health.check()?;
        let id = id.into();
        if self.pool.slot(&id).is_some() {
            return Err(data_render::AllocError::DuplicateId(id).into());
        }
        Ok(self.begin_upsert_hilo_column(id, source)?.commit())
    }

    /// Upload many **new** columns in one transaction: one pool region, one
    /// staging buffer, one copy, one submit — instead of one of each per column.
    ///
    /// This is the path a matrix declaration takes. At the thousands of columns
    /// a grid has, per-column [`Self::add_column`] spends the whole upload on
    /// first-fit searches, staging buffers and `queue.submit` calls. Nothing
    /// about the batch is matrix-specific, so any host with many columns gets it.
    ///
    /// Every id must be new to the pool and distinct within the batch —
    /// `DuplicateId` otherwise, with nothing uploaded. An empty slice is a
    /// no-op. No handles come back: a batch may relocate the pool, so callers
    /// re-acquire them through [`Self::handle_for`] exactly as after any other
    /// invalidating mutation.
    ///
    /// Failure-atomic end to end. The pool guard holds the columns provisionally
    /// while the chart revisions and the picker are prepared against them; if
    /// any of that fails the guard is dropped and no column is published.
    ///
    /// One thing a failure does not undo: a compaction or growth that had to
    /// happen first to make one contiguous region. That relayout is not part of
    /// the batch — the pool is simply laid out differently with the same columns
    /// in it — so handles and prepared frames are re-acquired exactly as after a
    /// [`Self::defragment`], and an active picker reports `StaleColumn` until it
    /// is prepared again.
    pub fn add_columns(&mut self, columns: &[(&str, &dyn ColumnSource)]) -> Result<()> {
        let result = self.add_columns_unreported(columns);
        self.reported(result)
    }

    pub(super) fn add_columns_unreported(
        &mut self,
        columns: &[(&str, &dyn ColumnSource)],
    ) -> Result<()> {
        self.health.check()?;
        for (id, _) in columns {
            self.reject_streamed_id(id)?;
        }
        if columns.is_empty() {
            return Ok(());
        }
        let invalidation = self.prepare_columns_invalidation(columns.iter().map(|(id, _)| *id))?;
        let old_layout_generation = self.pool.layout_generation();
        let ctx = pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger);
        let batch = self.pool.begin_add_columns(columns, ctx)?;
        let relocated = batch.pool().layout_generation() != old_layout_generation;
        // No changed-column list: every id here is new to the pool, and
        // `validate_renderer_series` refuses a series that reads a column the
        // pool does not have (test:
        // `a_matrix_column_must_be_registered_and_is_a_referenced_column`), so
        // no picker slot can already be reading one of them. Relocation is the
        // only reason a batch insert can force a rebuild — and it needs no list.
        let picker = prepare_picker_for_pool_mutation(
            &mut self.picker,
            batch.pool(),
            &self.chart_states,
            relocated,
            &[],
        )?;

        batch.commit();
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        if relocated {
            // The batch relaid the pool out, so the maintenance defrag the
            // upsert path would have asked for is already paid for.
            self.pending_defrag = false;
        }
        Ok(())
    }

    /// Begin a failure-atomic scalar insert or same-id replacement.
    pub fn begin_upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        // The guard borrows the renderer, so the check uses a clone of the
        // health record; a failed check drops the guard, which rolls back.
        let health = self.health.clone();
        let guard = self.begin_upsert_column_unreported(id, source)?;
        health.check()?;
        Ok(guard)
    }

    pub(super) fn begin_upsert_column_unreported(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        self.health.check()?;
        let id = id.into();
        self.reject_streamed_id(&id)?;
        let invalidation = self.prepare_column_invalidation(&id)?;
        let old_layout_generation = self.pool.layout_generation();
        let inner = self.pool.begin_upsert_column(
            id.clone(),
            source,
            pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger),
        )?;
        finish_renderer_column_upsert(
            inner,
            &self.streaming_sources,
            &id,
            old_layout_generation,
            &mut self.chart_states,
            &mut self.visual_revision,
            &mut self.pending_defrag,
            &mut self.picker,
            Some(invalidation),
            &self.device,
            &self.queue,
            self.errorbar_extent_engine.as_ref(),
        )
    }

    /// Begin a failure-atomic hi/lo insert or same-id replacement.
    pub fn begin_upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        // The guard borrows the renderer, so the check uses a clone of the
        // health record; a failed check drops the guard, which rolls back.
        let health = self.health.clone();
        let guard = self.begin_upsert_hilo_column_unreported(id, source)?;
        health.check()?;
        Ok(guard)
    }

    pub(super) fn begin_upsert_hilo_column_unreported(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        self.health.check()?;
        let id = id.into();
        self.reject_streamed_id(&id)?;
        let invalidation = self.prepare_column_invalidation(&id)?;
        let old_layout_generation = self.pool.layout_generation();
        let inner = self.pool.begin_upsert_hilo_column(
            id.clone(),
            source,
            pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger),
        )?;
        finish_renderer_column_upsert(
            inner,
            &self.streaming_sources,
            &id,
            old_layout_generation,
            &mut self.chart_states,
            &mut self.visual_revision,
            &mut self.pending_defrag,
            &mut self.picker,
            Some(invalidation),
            &self.device,
            &self.queue,
            self.errorbar_extent_engine.as_ref(),
        )
    }

    /// Begin a chunked upload of the resident column `id` (#311): `len`
    /// values of `encoding`, written chunk by chunk into one mapped staging
    /// buffer ([`ColumnUpload::write_range`]) and installed at once by
    /// [`Self::commit_column_upload`]. Nothing is registered until then.
    ///
    /// `expected_revision` is the [`Self::column_revision`] of the column the
    /// upload replaces, `None` for an id that is not registered; anything
    /// else is refused as stale ([`FiggyError::StaleStateToken`]), and so is
    /// a commit after the column changed. A streamed id and a reserved id
    /// are refused, as for every resident insertion. The staging buffer is
    /// held to the device's buffer limit and the memory budget
    /// ([`FiggyError::Pool`]); it is charged to the ledger while the upload
    /// is open.
    pub fn begin_column_upload(
        &self,
        id: impl Into<ColumnId>,
        len: usize,
        encoding: crate::StreamEncoding,
        expected_revision: Option<u64>,
    ) -> Result<ColumnUpload> {
        self.health.check()?;
        let id = id.into();
        self.reject_streamed_id(&id)?;
        self.check_upload_revision(&id, expected_revision)?;
        let staging = data_render::ColumnStaging::new(
            len,
            encoding,
            pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger),
        )?;
        // The staging buffer was created on this device; an error scope that
        // caught a failure is reported here.
        self.health.check()?;
        Ok(ColumnUpload {
            id,
            staging,
            renderer_identity: self.renderer_identity,
            expected_revision,
        })
    }

    /// Install a chunked upload (#311) as a failure-atomic insert or
    /// same-id replacement, as [`Self::upsert_column`] installs a column a
    /// source writes in one call: the charts that read it move their data
    /// revision, the picker is prepared against the new pool. Refused,
    /// changing nothing, when a value was not written
    /// ([`FiggyError::InvalidColumnUpload`]), when another renderer began
    /// it or the column's revision is no longer the one the upload expects
    /// ([`FiggyError::StaleStateToken`]), or when the pool cannot hold the
    /// column ([`FiggyError::Pool`]). The upload is consumed either way.
    pub fn commit_column_upload(&mut self, upload: ColumnUpload) -> Result<ColumnHandle> {
        let result = self.commit_column_upload_unreported(upload);
        self.reported(result)
    }

    pub(super) fn commit_column_upload_unreported(
        &mut self,
        upload: ColumnUpload,
    ) -> Result<ColumnHandle> {
        self.health.check()?;
        self.check_column_upload(&upload)?;
        let ColumnUpload { id, staging, .. } = upload;
        let health = self.health.clone();
        let invalidation = self.prepare_column_invalidation(&id)?;
        let old_layout_generation = self.pool.layout_generation();
        let inner = self.pool.begin_upsert_staged_column(
            id.clone(),
            staging,
            pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger),
        )?;
        let guard = finish_renderer_column_upsert(
            inner,
            &self.streaming_sources,
            &id,
            old_layout_generation,
            &mut self.chart_states,
            &mut self.visual_revision,
            &mut self.pending_defrag,
            &mut self.picker,
            Some(invalidation),
            &self.device,
            &self.queue,
            self.errorbar_extent_engine.as_ref(),
        )?;
        // A failed check drops the guard, which rolls back.
        health.check()?;
        Ok(guard.commit())
    }

    /// Whether `upload` can be installed now, as
    /// [`Self::commit_column_upload`] and a batch that holds it
    /// ([`BatchSource::Staged`], #334) check it before anything changes:
    /// refused when another renderer began it or the column's revision is
    /// no longer the one it expects ([`FiggyError::StaleStateToken`]), when
    /// a value was not written ([`FiggyError::InvalidColumnUpload`]), or
    /// when its id is streamed or reserved. A host that installs several
    /// uploads together calls it per upload to name the one refused.
    pub fn check_column_upload(&self, upload: &ColumnUpload) -> Result<()> {
        let id = upload.id();
        if upload.renderer_identity != self.renderer_identity {
            return Err(FiggyError::StaleStateToken {
                reason: format!("column upload {id} was begun on another renderer"),
            });
        }
        if !upload.staging.is_complete() {
            return Err(FiggyError::InvalidColumnUpload {
                id: id.to_owned(),
                reason: data_render::ColumnStagingError::Incomplete.reason(),
            });
        }
        self.reject_streamed_id(id)?;
        self.check_upload_revision(id, upload.expected_revision)
    }

    /// Refuse an upload whose expected revision is not column `id`'s.
    fn check_upload_revision(&self, id: &str, expected: Option<u64>) -> Result<()> {
        let current = self.column_revision(id);
        if current == expected {
            return Ok(());
        }
        let describe = |revision: Option<u64>| match revision {
            Some(revision) => format!("revision {revision}"),
            None => "no registered column".to_owned(),
        };
        Err(FiggyError::StaleStateToken {
            reason: format!(
                "column upload {id} expects {}, the renderer holds {}",
                describe(expected),
                describe(current)
            ),
        })
    }

    /// Failure-atomic scalar upsert without an intermediate batch-preparation
    /// step. Hosts that must prepare additional derived state against the
    /// provisional pool can use [`Self::begin_upsert_column`] and commit its
    /// guard explicitly.
    pub fn upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        let result = self.upsert_column_unreported(id, source);
        self.reported(result)
    }

    pub(super) fn upsert_column_unreported(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        Ok(self.begin_upsert_column(id, source)?.commit())
    }

    /// Failure-atomic hi/lo upsert without an intermediate batch-preparation
    /// step.
    pub fn upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        let result = self.upsert_hilo_column_unreported(id, source);
        self.reported(result)
    }

    pub(super) fn upsert_hilo_column_unreported(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        Ok(self.begin_upsert_hilo_column(id, source)?.commit())
    }
}
