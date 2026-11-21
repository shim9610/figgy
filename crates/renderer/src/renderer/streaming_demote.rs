//! [`Renderer::demote_resident_columns_to_streamed`]'s phases, split out
//! move-only (#341): the changed column closure, the metadata and source
//! checks, and the registry entries the commit publishes. Streaming is not
//! extended (rule 8).

use super::*;

impl Renderer {
    /// The changed ids, each once, and the chart closure they reach, every
    /// column of it either resident or streamed.
    pub(super) fn demotion_closure<'a>(
        &self,
        changed_ids: &[&'a str],
    ) -> Result<(
        std::collections::HashSet<&'a str>,
        std::collections::HashSet<ColumnId>,
    )> {
        let mut changed = std::collections::HashSet::new();
        changed.try_reserve(changed_ids.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "resident demotion changed id set",
                reason: error.to_string(),
            }
        })?;
        for id in changed_ids {
            if !changed.insert(*id) {
                return Err(data_render::AllocError::DuplicateId((*id).to_owned()).into());
            }
        }
        let closure = self.transition_column_closure(changed.iter().copied())?;
        for id in &closure {
            let resident = self.pool.slot(id).is_some();
            let streamed = self.streaming_sources.contains_key(id);
            if resident && streamed {
                return Err(FiggyError::InvalidStreamSource {
                    id: id.clone(),
                    reason: "column is simultaneously resident and streamed",
                });
            }
            if !resident && !streamed {
                return Err(FiggyError::UnknownColumn { id: id.clone() });
            }
        }
        Ok((changed, closure))
    }

    /// The metadata and sources cover the closure exactly, once each, and
    /// every revision moves its column's lineage forward.
    pub(super) fn check_demotion_sources(
        &self,
        closure: &std::collections::HashSet<ColumnId>,
        changed: &std::collections::HashSet<&str>,
        columns: Vec<crate::StreamColumn>,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<Vec<crate::StreamColumn>> {
        let mut seen = std::collections::HashSet::new();
        seen.try_reserve(columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident demotion metadata id set",
                reason: error.to_string(),
            })?;
        for source in &columns {
            source.validate()?;
            if !seen.insert(source.id.as_str()) {
                return Err(data_render::AllocError::DuplicateId(source.id.clone()).into());
            }
            if !closure.contains(&source.id) {
                return Err(FiggyError::InvalidStreamSource {
                    id: source.id.clone(),
                    reason: "demotion metadata is outside the changed chart closure",
                });
            }
        }
        if let Some(missing) = closure.iter().find(|id| !seen.contains(id.as_str())) {
            return Err(FiggyError::InvalidStreamSource {
                id: missing.clone(),
                reason: "demotion metadata must cover the complete referenced-column chart closure",
            });
        }

        let mut seen_bindings = std::collections::HashSet::new();
        seen_bindings.try_reserve(sources.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "resident demotion source id set",
                reason: error.to_string(),
            }
        })?;
        for binding in sources {
            if !seen_bindings.insert(binding.id) {
                return Err(FiggyError::InvalidStreamSource {
                    id: binding.id.to_owned(),
                    reason: "demotion source id appears more than once",
                });
            }
            if !closure.contains(binding.id) {
                return Err(FiggyError::InvalidStreamSource {
                    id: binding.id.to_owned(),
                    reason: "demotion source is outside the changed chart closure",
                });
            }
        }
        if let Some(missing) = closure
            .iter()
            .find(|id| !seen_bindings.contains(id.as_str()))
        {
            return Err(FiggyError::InvalidStreamSource {
                id: missing.clone(),
                reason: "demotion sources must cover the complete referenced-column chart closure",
            });
        }

        for source in &columns {
            let binding = sources
                .iter()
                .find(|binding| binding.id == source.id)
                .ok_or(FiggyError::internal(
                    "exact closure source coverage was validated",
                ))?;
            if binding.revision != source.revision {
                return Err(FiggyError::InvalidStreamSource {
                    id: source.id.clone(),
                    reason: "demotion source revision does not match its metadata",
                });
            }
            let binding_len = u64::try_from(binding.source.len()).map_err(|_| {
                FiggyError::InvalidStreamSource {
                    id: source.id.clone(),
                    reason: "demotion source length exceeds the host index representation",
                }
            })?;
            if binding_len != source.len {
                return Err(FiggyError::InvalidStreamSource {
                    id: source.id.clone(),
                    reason: "demotion source length does not match its metadata",
                });
            }
            if binding.source.encoding() != source.encoding {
                return Err(FiggyError::InvalidStreamSource {
                    id: source.id.clone(),
                    reason: "demotion source encoding does not match its metadata",
                });
            }
            if let Some(slot) = self.pool.slot(&source.id) {
                if !changed.contains(source.id.as_str())
                    && u64::try_from(slot.len_values).ok() != Some(source.len)
                {
                    return Err(FiggyError::InvalidStreamSource {
                        id: source.id.clone(),
                        reason: "unchanged closure source length does not match the resident column",
                    });
                }
                match self.stream_revision_lineage.get(&source.id) {
                    Some(last) if source.revision <= *last => {
                        return Err(FiggyError::InvalidStreamSource {
                            id: source.id.clone(),
                            reason: "demotion revision must exceed the logical stream lineage",
                        });
                    }
                    None if source.revision == 0 => {
                        return Err(FiggyError::InvalidStreamSource {
                            id: source.id.clone(),
                            reason: "initial demotion revision must be nonzero",
                        });
                    }
                    _ => {}
                }
                continue;
            }

            let registered = self
                .streaming_sources
                .get(&source.id)
                .ok_or(FiggyError::internal(
                    "closure column was validated streamed",
                ))?;
            if source.revision < registered.revision {
                return Err(FiggyError::InvalidStreamSource {
                    id: source.id.clone(),
                    reason: "streamed normalization revision is stale",
                });
            }
            if source.revision == registered.revision {
                if source.len != registered.len
                    || source.encoding != registered.encoding
                    || source.replay != registered.replay
                {
                    return Err(FiggyError::InvalidStreamSource {
                        id: source.id.clone(),
                        reason: "same-revision streamed metadata does not match registered content metadata",
                    });
                }
                if self.stream_revision_lineage.get(&source.id) != Some(&source.revision) {
                    return Err(FiggyError::internal(
                        "registered stream revision remains the logical lineage",
                    ));
                }
                continue;
            }
            if !changed.contains(source.id.as_str()) {
                return Err(FiggyError::InvalidStreamSource {
                    id: source.id.clone(),
                    reason: "unchanged streamed closure companion must keep its revision",
                });
            }
            if self
                .stream_revision_lineage
                .get(&source.id)
                .is_some_and(|last| source.revision <= *last)
            {
                return Err(FiggyError::InvalidStreamSource {
                    id: source.id.clone(),
                    reason: "streamed replacement revision must exceed the logical lineage",
                });
            }
        }
        Ok(columns)
    }

    /// The registry entries of the columns the demotion changes, with their
    /// fit epochs.
    pub(super) fn demotion_entries(
        &self,
        columns: Vec<crate::StreamColumn>,
        mutation_count: usize,
    ) -> Result<Vec<(String, String, u64, bool, RegisteredStreamColumn)>> {
        let mut entries = Vec::new();
        entries.try_reserve_exact(mutation_count).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "resident demotion entries",
                reason: error.to_string(),
            }
        })?;
        let mut fit_epoch = self.next_stream_source_fit_epoch;
        for source in columns {
            let was_resident = self.pool.slot(&source.id).is_some();
            let replaces_stream = self
                .streaming_sources
                .get(&source.id)
                .is_some_and(|registered| source.revision > registered.revision);
            if !was_resident && !replaces_stream {
                continue;
            }
            fit_epoch = fit_epoch
                .checked_add(1)
                .ok_or(FiggyError::CounterExhausted {
                    counter: "stream source fit epoch",
                })?;
            let mut key = String::new();
            key.try_reserve_exact(source.id.len()).map_err(|error| {
                FiggyError::StateAllocationFailed {
                    resource: "streaming source id",
                    reason: error.to_string(),
                }
            })?;
            key.push_str(&source.id);
            let mut lineage_key = String::new();
            lineage_key
                .try_reserve_exact(source.id.len())
                .map_err(|error| FiggyError::StateAllocationFailed {
                    resource: "stream revision lineage id",
                    reason: error.to_string(),
                })?;
            lineage_key.push_str(&source.id);
            let revision = source.revision;
            entries.push((
                key,
                lineage_key,
                revision,
                was_resident,
                RegisteredStreamColumn {
                    column: source,
                    fit_epoch,
                    statistics_cache: StreamStatisticsCache::default(),
                },
            ));
        }
        Ok(entries)
    }
}
