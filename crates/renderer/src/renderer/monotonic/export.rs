//! Envelope column uploads, the export's transient envelopes and forgetting
//! groups and charts.

use super::*;

impl Renderer {
    /// Replaces pool column `id` with `rows` through the renderer's upsert
    /// (chart revisions, picker and resident display caches follow). An
    /// envelope of the same width replaces one of the same size: the hole
    /// it leaves is the next one's, so it asks for no defragmentation.
    pub(super) fn upload_envelope_column(
        &mut self,
        id: &str,
        rows: &EnvelopeRows<'_>,
    ) -> Result<()> {
        let same_size = self
            .pool
            .slot(id)
            .is_some_and(|slot| slot.len_values == rows.len());
        let pending = self.pending_defrag;
        let invalidation = self.prepare_column_invalidation(id)?;
        let old_layout_generation = self.pool.layout_generation();
        let inner = self.pool.begin_upsert_column(
            id.to_owned(),
            rows,
            pool_alloc_ctx(&self.device, &self.queue, &self.gpu_ledger),
        )?;
        finish_renderer_column_upsert(
            inner,
            &self.streaming_sources,
            id,
            old_layout_generation,
            &mut self.chart_states,
            &mut self.visual_revision,
            &mut self.pending_defrag,
            &mut self.picker,
            Some(invalidation),
            &self.device,
            &self.queue,
            self.errorbar_extent_engine.as_ref(),
        )?
        .commit();
        if same_size {
            self.pending_defrag = pending && self.pending_defrag;
        }
        Ok(())
    }

    /// Builds, for an export at another mapping than the screen's, the
    /// envelopes of `series` (the export's copy of the chart's series) at the
    /// export's panels (`export_config`, as the export places them) from `sources`,
    /// uploads them to transient columns and points the copies at them. Returns the transient column ids, which
    /// [`Self::end_monotonic_export`] removes. A series whose envelope
    /// already matches the export's mapping keeps its columns; one that
    /// matches nothing and has no source in `sources` fails with
    /// [`FiggyError::MonotonicSourceRequired`].
    pub(in crate::renderer) fn begin_monotonic_export(
        &mut self,
        chart: ChartId,
        export_config: &Config,
        series: &mut [SeriesConfig],
        sources: &[MonotonicGroupRef<'_>],
    ) -> Result<Vec<ColumnId>> {
        let mut transient = Vec::new();
        // A failure at any series removes what the earlier ones built, so a
        // failed export leaves no transient column or group behind (#469).
        match self.build_monotonic_export(chart, export_config, series, sources, &mut transient) {
            Ok(()) => Ok(transient),
            Err(error) => {
                self.end_monotonic_export(&transient);
                Err(error)
            }
        }
    }

    /// The body of [`Self::begin_monotonic_export`]: pushes each transient
    /// column id to `transient` as soon as it is uploaded.
    fn build_monotonic_export(
        &mut self,
        chart: ChartId,
        export_config: &Config,
        series: &mut [SeriesConfig],
        sources: &[MonotonicGroupRef<'_>],
        transient: &mut Vec<ColumnId>,
    ) -> Result<()> {
        if !series.iter().any(|s| s.monotonic().is_some()) {
            return Ok(());
        }
        let panels = DocumentPanels::new(export_config)?;
        for index in 0..series.len() {
            let cfg = &series[index];
            let Some(monotonic) = cfg.monotonic() else {
                continue;
            };
            let panel = panels.of(&cfg.series_id);
            if self.monotonic.check_series(panel, cfg).is_ok() {
                continue;
            }
            let mapping = panel_mapping(panel);
            let Some(data) = sources.iter().find(|s| s.group == monotonic.group) else {
                let change = self
                    .monotonic
                    .check_series(panel, cfg)
                    .err()
                    .and_then(|error| match error {
                        FiggyError::MonotonicSourceRequired { change, .. } => Some(change),
                        _ => None,
                    })
                    .unwrap_or(MappingChange::NotBuilt);
                // A group without data draws nothing, at any scale.
                if change == MappingChange::NotBuilt {
                    continue;
                }
                return Err(FiggyError::MonotonicSourceRequired {
                    series_id: cfg.series_id.clone(),
                    change,
                });
            };
            let channel = monotonic.channel;
            let channels = data.source.channel_count();
            if channel >= channels {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    kind: crate::RuleKind::MonotonicChannel { channel, channels },
                });
            }
            let mut envelope = MonotonicEnvelope::empty();
            envelope
                .set_monotonic_ref(data.source, &mapping)
                .map_err(|error| FiggyError::Monotonic {
                    group: monotonic.group.clone(),
                    error,
                })?;
            let group = monotonic.group.clone();
            let series_id = cfg.series_id.clone();
            let ids = [
                format!("{}\u{1}export", cfg.x_column),
                format!("{}\u{1}export", cfg.y_column),
            ];
            let envelope = Arc::new(envelope);
            for (id, x) in [(&ids[0], true), (&ids[1], false)] {
                if self.pool.slot(id).is_some() || self.monotonic.owns_column(id) {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: series_id.clone(),
                        kind: crate::RuleKind::MonotonicColumnTaken { column: id.clone() },
                    });
                }
                let rows = EnvelopeRows {
                    envelope: &envelope,
                    channel,
                    x,
                };
                self.upload_envelope_column(id, &rows)?;
                transient.push(id.clone());
            }
            // The transient columns are drawn as one more group of this
            // chart, under its own name, and checked like any other.
            let key = format!("{group}\u{1}export");
            let panel_index = {
                let groups = self.monotonic.groups.entry(chart).or_default();
                let state = groups
                    .entry(key.clone())
                    .or_insert_with(|| MonotonicGroupState { panels: Vec::new() });
                state.panels.push(envelope);
                state.panels.len() - 1
            };
            for id in &ids {
                self.monotonic.columns.insert(
                    id.clone(),
                    MonotonicColumn {
                        chart,
                        key: key.clone(),
                        group: group.clone(),
                        series_id: series_id.clone(),
                        panel: panel_index,
                        channel,
                    },
                );
            }
            let [x, y] = ids;
            series[index].x_column = x;
            series[index].y_column = y;
        }
        Ok(())
    }

    /// Removes an export's transient envelope columns and their groups.
    pub(in crate::renderer) fn end_monotonic_export(&mut self, transient: &[ColumnId]) {
        for id in transient {
            if let Some(record) = self.monotonic.columns.remove(id) {
                if let Some(groups) = self.monotonic.groups.get_mut(&record.chart) {
                    groups.remove(&record.key);
                }
            }
            // A failed removal leaves an unreferenced column the next
            // export's id check refuses; the export's own result stands.
            if self.remove_column_impl(id, None).is_err() {
                continue;
            }
        }
    }

    /// Drops chart `chart`'s monotonic state and its envelope columns, when
    /// the chart is removed.
    /// Drops group `group` of chart `chart` and removes its envelope
    /// columns: after a failed append its series draw nothing rather than
    /// an envelope the group no longer has.
    pub(super) fn forget_monotonic_group(&mut self, chart: ChartId, group: &str) {
        for id in self.monotonic.forget_group(chart, group) {
            if self.remove_envelope_column(&id).is_err() {
                continue;
            }
        }
    }

    /// Removes envelope column `id`, whose record is already dropped. A
    /// series that still names it keeps its place in the document and draws
    /// nothing; the active pick registry, which would read the column, is
    /// rebuilt at the next pick.
    pub(super) fn remove_envelope_column(&mut self, id: &str) -> Result<bool> {
        let named = self.picker.active.as_ref().is_some_and(|active| {
            self.chart_states
                .get(&active.chart_id)
                .is_some_and(|state| {
                    state
                        .series
                        .iter()
                        .any(|series| series_references_column(series, id))
                })
        });
        if named {
            self.picker.active = None;
        }
        self.remove_column_impl(id, None)
    }

    pub(in crate::renderer) fn forget_monotonic_chart(&mut self, chart: ChartId) {
        for id in self.monotonic.forget_chart(chart) {
            if self.remove_envelope_column(&id).is_err() {
                continue;
            }
        }
    }
}
