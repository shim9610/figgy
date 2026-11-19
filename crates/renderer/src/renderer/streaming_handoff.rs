//! [`Renderer::commit_completed_resident_stream_handoff`]'s phases, split
//! out move-only (#341): the staleness checks and the registry entries the
//! commit publishes. Streaming is not extended (rule 8).

use super::*;

impl Renderer {
    /// The charts, resident columns and streamed columns the handoff read are
    /// still the ones it rendered.
    pub(super) fn check_stream_handoff_current(
        &self,
        handoff: &ResidentStreamHandoff,
    ) -> Result<()> {
        for (id, data, series) in &handoff.expected_chart_revisions {
            let state = self
                .chart_states
                .get(id)
                .ok_or(FiggyError::UnknownChart { id: *id })?;
            if state.revisions.data != *data || state.revisions.series != *series {
                return Err(FiggyError::StaleStateToken {
                    reason: "chart data or series changed during resident stream handoff".into(),
                });
            }
        }
        for (id, epoch) in &handoff.allocation_epochs {
            if self.pool.allocation_epoch(id) != Some(*epoch)
                || self.streaming_sources.contains_key(id)
            {
                return Err(FiggyError::StaleStateToken {
                    reason: "resident column changed during stream handoff".into(),
                });
            }
        }
        for (id, revision) in &handoff.expected_stream_revisions {
            if self.pool.slot(id).is_some()
                || self
                    .streaming_sources
                    .get(id)
                    .is_none_or(|source| source.revision != *revision)
            {
                return Err(FiggyError::StaleStateToken {
                    reason: "streamed column changed during mixed stream handoff".into(),
                });
            }
        }
        Ok(())
    }

    /// The registry entries of the handed-off columns, with their consumed
    /// statistics and fit epochs; the registries reserve room for them.
    pub(super) fn stream_handoff_entries(
        &mut self,
        handoff: &ResidentStreamHandoff,
        statistics: &[(ColumnId, StreamStatisticsCache)],
        final_fit_epoch: u64,
    ) -> Result<Vec<(String, String, u64, bool, RegisteredStreamColumn)>> {
        self.streaming_sources
            .try_reserve(handoff.columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "streaming source registry",
                reason: error.to_string(),
            })?;
        self.stream_revision_lineage
            .try_reserve(handoff.columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "stream revision lineage",
                reason: error.to_string(),
            })?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(handoff.columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident stream handoff entries",
                reason: error.to_string(),
            })?;
        let mut fit_epoch = self.next_stream_source_fit_epoch;
        for source in &handoff.columns {
            let cache = statistics
                .iter()
                .find_map(|(id, cache)| (id == &source.id).then_some(cache))
                .ok_or_else(|| FiggyError::StaleStateToken {
                    reason: format!(
                        "stream handoff has no statistics for column {:?}",
                        source.id
                    ),
                })?;
            if !cache.covers(0..source.len) {
                return Err(FiggyError::StaleStateToken {
                    reason: format!(
                        "stream handoff did not consume the complete column {:?}",
                        source.id
                    ),
                });
            }
            let cache =
                cache
                    .try_coverage_copy()
                    .map_err(|error| FiggyError::StateAllocationFailed {
                        resource: "resident stream handoff statistics",
                        reason: error.to_string(),
                    })?;
            fit_epoch = fit_epoch
                .checked_add(1)
                .ok_or(FiggyError::CounterExhausted {
                    counter: "stream source fit epoch",
                })?;
            let mut id = String::new();
            id.try_reserve_exact(source.id.len()).map_err(|error| {
                FiggyError::StateAllocationFailed {
                    resource: "resident stream handoff id",
                    reason: error.to_string(),
                }
            })?;
            id.push_str(&source.id);
            let mut lineage_id = String::new();
            lineage_id
                .try_reserve_exact(source.id.len())
                .map_err(|error| FiggyError::StateAllocationFailed {
                    resource: "resident stream handoff lineage id",
                    reason: error.to_string(),
                })?;
            lineage_id.push_str(&source.id);
            let mut column = source.clone();
            column.statistics = crate::StreamStatistics::Known(cache.bounds);
            entries.push((
                id,
                lineage_id,
                source.revision,
                self.pool.slot(&source.id).is_some(),
                RegisteredStreamColumn {
                    column,
                    fit_epoch,
                    statistics_cache: cache,
                },
            ));
        }
        if fit_epoch != final_fit_epoch {
            return Err(FiggyError::internal(
                "the handoff assigns every planned fit epoch",
            ));
        }
        Ok(entries)
    }
}
