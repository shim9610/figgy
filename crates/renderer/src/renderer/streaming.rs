//! Streamed columns on the renderer: registration and replacement, the
//! streamed statistics cache, promotion to and demotion from resident
//! columns, and the resident-to-stream handoff.

use super::*;

pub(super) struct RegisteredStreamColumn {
    pub(super) column: crate::StreamColumn,
    pub(super) fit_epoch: u64,
    pub(super) statistics_cache: StreamStatisticsCache,
}

impl std::ops::Deref for RegisteredStreamColumn {
    type Target = crate::StreamColumn;

    fn deref(&self) -> &Self::Target {
        &self.column
    }
}

/// Normal streaming coalesces to one interval. This ceiling keeps deliberately
/// fragmented random-access supply from turning statistics metadata into an
/// unbounded per-point mirror of the source.
pub(super) const MAX_STREAM_STATISTICS_RANGES: usize = 4096;

#[derive(Default)]
pub(super) struct StreamStatisticsCache {
    /// Sorted, disjoint measured ranges. Adjacent sequential chunks coalesce,
    /// while sparse chunks remain cached so their values are never rescanned.
    pub(super) covered: Vec<std::ops::Range<u64>>,
    pub(super) bounds: Option<crate::StreamBounds>,
}

impl StreamStatisticsCache {
    pub(super) fn covers(&self, range: std::ops::Range<u64>) -> bool {
        range.is_empty()
            || self
                .covered
                .iter()
                .any(|covered| covered.start <= range.start && covered.end >= range.end)
    }

    pub(super) fn try_coverage_copy(
        &self,
    ) -> std::result::Result<Self, std::collections::TryReserveError> {
        let mut covered = Vec::new();
        covered.try_reserve_exact(self.covered.len())?;
        covered.extend(self.covered.iter().cloned());
        Ok(Self {
            covered,
            bounds: self.bounds,
        })
    }

    pub(super) fn uncovered(
        &self,
        range: std::ops::Range<u64>,
    ) -> std::result::Result<Vec<std::ops::Range<u64>>, std::collections::TryReserveError> {
        let mut gaps = Vec::new();
        gaps.try_reserve(self.covered.len().saturating_add(1))?;
        let mut cursor = range.start;
        for covered in &self.covered {
            if covered.end <= cursor {
                continue;
            }
            if covered.start >= range.end {
                break;
            }
            if covered.start > cursor {
                gaps.push(cursor..covered.start.min(range.end));
            }
            cursor = cursor.max(covered.end);
            if cursor >= range.end {
                break;
            }
        }
        if cursor < range.end {
            gaps.push(cursor..range.end);
        }
        Ok(gaps)
    }

    pub(super) fn coverage_len_after_insert(&self, range: &std::ops::Range<u64>) -> usize {
        let index = self
            .covered
            .partition_point(|cached| cached.end < range.start);
        let mut merged_end = range.end;
        let mut removed = 0usize;
        while index + removed < self.covered.len()
            && self.covered[index + removed].start <= merged_end
        {
            merged_end = merged_end.max(self.covered[index + removed].end);
            removed += 1;
        }
        self.covered.len() - removed + 1
    }

    pub(super) fn merge_bounds(&mut self, bounds: Option<crate::StreamBounds>) {
        let Some(bounds) = bounds else {
            return;
        };
        self.bounds = Some(match self.bounds {
            Some(current) => crate::StreamBounds {
                min: current.min.min(bounds.min),
                max: current.max.max(bounds.max),
                min_positive: match (current.min_positive, bounds.min_positive) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                },
            },
            None => bounds,
        });
    }

    pub(super) fn insert_measured(
        &mut self,
        range: std::ops::Range<u64>,
        bounds: Option<crate::StreamBounds>,
    ) {
        let mut start = range.start;
        let mut end = range.end;
        let index = self.covered.partition_point(|cached| cached.end < start);
        while index < self.covered.len() && self.covered[index].start <= end {
            let cached = self.covered.remove(index);
            start = start.min(cached.start);
            end = end.max(cached.end);
        }
        self.covered.insert(index, start..end);
        self.merge_bounds(bounds);
    }
}

/// Whether two configs produce the same streamed data prefix.
///
/// Decoration is deliberately excluded: titles, tick-label appearance, grid
/// colours (for Precise), and other raster-only chrome are composited with
/// the preserved prefix. Styled background-dependent blending must replay
/// when its grid changes. Anything that changes data-to-pixel geometry or the set of data
/// primitives remains a stream-breaking view change.
pub(super) fn stream_config_equal(left: &Config, right: &Config, series: &[SeriesConfig]) -> bool {
    // Data coordinates use bottom X and left Y. Top/right axis ranges only
    // affect decoration; changing them must not replay an uncached stream.
    left.chart_area == right.chart_area
        && AxisViewState::from_axis(&left.bottom_x) == AxisViewState::from_axis(&right.bottom_x)
        && AxisViewState::from_axis(&left.left_y) == AxisViewState::from_axis(&right.left_y)
        && left.data_area().ok().map(|area| area.0)
            == right.data_area().ok().map(|area| area.0)
        && overlay_geometry_equal(left, right)
        && subplot_geometry_equal(left, right, series)
        && left.draw_style == right.draw_style
        // The coordinate system maps every sample to its pixel.
        && left.polar == right.polar
        // Z mapping changes the accumulated field pixels, unlike the bar's
        // border, labels or position. XY-only charts do not consume this map.
        && (!series.iter().any(|s| match &s.render_type {
            DataRenderType::Heatmap { .. } | DataRenderType::HeatmapContour { .. } => true,
            // A colour-mapped vector draws through the colourbar too.
            DataRenderType::Vector { style, .. } => style.color_map.is_some(),
            DataRenderType::Scatter { .. }
            | DataRenderType::Line { .. }
            | DataRenderType::ScatterLine { .. }
            | DataRenderType::ScatterErrorbarX { .. }
            | DataRenderType::ScatterErrorbarY { .. }
            | DataRenderType::ScatterErrorbarXY { .. }
            | DataRenderType::LineScatterErrorbarX { .. }
            | DataRenderType::LineScatterErrorbarY { .. }
            | DataRenderType::LineScatterErrorbarXY { .. }
            | DataRenderType::Histogram { .. }
            | DataRenderType::CategoryBars { .. }
            | DataRenderType::BoxPlot { .. }
            | DataRenderType::Radar { .. }
            | DataRenderType::Contour { .. } => false,
        })
            || match (&left.colorbar, &right.colorbar) {
                (Some(a), Some(b)) => a.axis.min == b.axis.min
                    && a.axis.max == b.axis.max && a.axis.scale == b.axis.scale
                    && a.colormap == b.colormap && a.nan_color == b.nan_color,
                (None, None) => true,
                _ => false,
            })
        // Non-Precise blending may depend on the grid/background. Those
        // streams must rebuild their prefix when the grid changes.
        && (matches!(left.draw_style, crate::config::DrawStyle::Precise) || left.grid == right.grid)
}

impl Renderer {
    pub(super) fn transition_column_closure<'i>(
        &self,
        roots: impl Iterator<Item = &'i str>,
    ) -> Result<std::collections::HashSet<ColumnId>> {
        let mut closure = std::collections::HashSet::new();
        for id in roots {
            try_insert_column_id(&mut closure, id, "column transition closure")?;
        }

        loop {
            let before = closure.len();
            for state in self.chart_states.values() {
                let touches = state.series.iter().any(|series| {
                    let mut touches = false;
                    visit_series_columns(series, &mut |id| touches |= closure.contains(id));
                    touches
                });
                if !touches {
                    continue;
                }
                let mut insertion_error = None;
                for series in &state.series {
                    visit_series_columns(series, &mut |id| {
                        if insertion_error.is_none()
                            && let Err(error) =
                                try_insert_column_id(&mut closure, id, "column transition closure")
                        {
                            insertion_error = Some(error);
                        }
                    });
                }
                if let Some(error) = insertion_error {
                    return Err(error);
                }
            }
            if closure.len() == before {
                return Ok(closure);
            }
        }
    }

    pub(super) fn validated_streamed_promotion_lengths(
        &self,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<Vec<u64>> {
        let mut lengths = Vec::new();
        lengths.try_reserve_exact(sources.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "streamed promotion lengths",
                reason: error.to_string(),
            }
        })?;
        let mut seen = std::collections::HashSet::new();
        seen.try_reserve(sources.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "streamed promotion id set",
                reason: error.to_string(),
            })?;
        for binding in sources {
            if !seen.insert(binding.id) {
                return Err(FiggyError::InvalidStreamSource {
                    id: binding.id.to_owned(),
                    reason: "promotion source id appears more than once",
                });
            }
            if self.pool.slot(binding.id).is_some() {
                return Err(FiggyError::InvalidStreamSource {
                    id: binding.id.to_owned(),
                    reason: "promotion source is already resident",
                });
            }
            let registered = self.streaming_sources.get(binding.id).ok_or_else(|| {
                FiggyError::UnknownColumn {
                    id: binding.id.to_owned(),
                }
            })?;
            if registered.revision != binding.revision {
                return Err(FiggyError::InvalidStreamSource {
                    id: binding.id.to_owned(),
                    reason: "promotion source revision does not match the registered revision",
                });
            }
            let len = u64::try_from(binding.source.len()).map_err(|_| {
                FiggyError::InvalidStreamSource {
                    id: binding.id.to_owned(),
                    reason: "promotion source length exceeds the host index representation",
                }
            })?;
            if len == 0 {
                return Err(FiggyError::InvalidStreamSource {
                    id: binding.id.to_owned(),
                    reason: "empty streamed columns cannot become resident",
                });
            }
            if registered.len != len {
                return Err(FiggyError::InvalidStreamSource {
                    id: binding.id.to_owned(),
                    reason: "promotion source length does not match the registered length",
                });
            }
            if registered.encoding != binding.source.encoding() {
                return Err(FiggyError::InvalidStreamSource {
                    id: binding.id.to_owned(),
                    reason: "promotion source encoding does not match the registered encoding",
                });
            }
            lengths.push(len);
        }
        let closure = self.transition_column_closure(seen.iter().copied())?;
        if let Some(missing) = closure.iter().find(|id| !seen.contains(id.as_str())) {
            return Err(FiggyError::InvalidStreamSource {
                id: missing.clone(),
                reason: "promotion must cover the complete referenced-column chart closure",
            });
        }
        Ok(lengths)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn resident_admission_for_validated_streamed_lengths(
        &self,
        lengths: &[u64],
        already_resident_working_set_bytes: u64,
        derived_buffer_sizes: &[u64],
        transition_headroom_bytes: u64,
        working_set_limit_bytes: u64,
    ) -> ResidentAdmission {
        let mut report = self.resident_admission(ResidentAdmissionRequest {
            column_value_counts: lengths,
            already_resident_working_set_bytes,
            derived_buffer_sizes,
            transition_headroom_bytes,
            working_set_limit_bytes,
        });

        let needs_candidate = report.resident_column_bytes > report.pool_largest_free_bytes;
        let planned = matches!(
            report.status,
            ResidentAdmissionStatus::Admissible
                | ResidentAdmissionStatus::MemoryBudgetUnset
                | ResidentAdmissionStatus::MemoryBudgetExceeded
        );
        if needs_candidate && planned {
            // The general pool planner may report zero transition bytes when a
            // ping-pong backup exists. Promotion cannot borrow that buffer
            // without moving live rollback ownership, so its dedicated atomic
            // guard uses a fresh candidate and this helper charges it exactly.
            report.pool_transition_bytes = report.pool_capacity_after;
            let Some(peak) = report
                .current_gpu_bytes
                .checked_add(report.pool_transition_bytes)
                .and_then(|bytes| bytes.checked_add(report.upload_staging_bytes))
                .and_then(|bytes| bytes.checked_add(report.derived_resident_bytes))
                .and_then(|bytes| bytes.checked_add(transition_headroom_bytes))
            else {
                report.status = ResidentAdmissionStatus::ArithmeticOverflow;
                report.transition_peak_bytes = u64::MAX;
                return report;
            };
            report.transition_peak_bytes = peak;
            report.status = match report.memory_budget_bytes {
                None => ResidentAdmissionStatus::MemoryBudgetUnset,
                Some(limit) if peak > limit => ResidentAdmissionStatus::MemoryBudgetExceeded,
                Some(_) => ResidentAdmissionStatus::Admissible,
            };
        }
        report
    }

    /// Validate exact same-revision streamed sources and report whether their
    /// failure-atomic promotion can fit the resident path.
    ///
    /// Unlike [`Self::resident_admission`], this helper derives column lengths
    /// from the bindings and accounts for the complete candidate slab required
    /// when promotion must compact or grow. It performs no upload and retains
    /// no borrowed source.
    #[allow(clippy::too_many_arguments)]
    pub fn resident_admission_for_streamed_columns(
        &self,
        sources: &[crate::StreamSourceBinding<'_>],
        already_resident_working_set_bytes: u64,
        derived_buffer_sizes: &[u64],
        transition_headroom_bytes: u64,
        working_set_limit_bytes: u64,
    ) -> Result<ResidentAdmission> {
        let lengths = self.validated_streamed_promotion_lengths(sources)?;
        Ok(self.resident_admission_for_validated_streamed_lengths(
            &lengths,
            already_resident_working_set_bytes,
            derived_buffer_sizes,
            transition_headroom_bytes,
            working_set_limit_bytes,
        ))
    }

    pub(super) fn reject_streamed_id(&self, id: &str) -> Result<()> {
        if self.streaming_sources.contains_key(id) {
            return Err(FiggyError::InvalidStreamSource {
                id: id.to_owned(),
                reason: "remove the streamed registration before resident insertion",
            });
        }
        // A record's or ragged column's id, and its member columns, are not
        // columns a host can insert or replace.
        if self.pool.is_member_column(id) {
            return Err(FiggyError::ReservedColumnId { id: id.to_owned() });
        }
        if self.pool.composite_kind(id).is_some() {
            return Err(data_render::AllocError::DuplicateId(id.to_owned()).into());
        }
        Ok(())
    }

    pub(super) fn require_resident_series(&self, series: &SeriesConfig) -> Result<()> {
        require_resident_sources(&self.streaming_sources, series)
    }

    /// Register any number of replayable sources without uploading or retaining
    /// their payload. Existing ids are rejected, including resident ids.
    pub fn register_streamed_columns(&mut self, columns: Vec<crate::StreamColumn>) -> Result<()> {
        self.mutate_streamed_columns(columns, false)
    }

    /// Replace existing host metadata atomically. Each source revision must
    /// strictly increase; bindings are unchanged and only dependent charts dirty.
    pub fn replace_streamed_columns(&mut self, columns: Vec<crate::StreamColumn>) -> Result<()> {
        self.mutate_streamed_columns(columns, true)
    }

    /// Commit complete statistics only for the still-current source revision.
    /// Partial chunk extrema must never be passed here. Known statistics cannot
    /// be overwritten; source changes require explicit revision replacement.
    /// Statistics change fit tokens, not the source content or an in-flight
    /// draw of that same revision.
    #[cfg(test)]
    pub(super) fn commit_streamed_statistics(
        &mut self,
        id: &str,
        revision: u64,
        bounds: Option<crate::StreamBounds>,
    ) -> Result<()> {
        let source = self
            .streaming_sources
            .get(id)
            .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_owned() })?;
        if source.revision != revision {
            return Err(FiggyError::StaleStateToken {
                reason: "source statistics revision changed".into(),
            });
        }
        let statistics = crate::StreamStatistics::Known(bounds);
        crate::streaming_source::validate_statistics(source.len, statistics).map_err(|reason| {
            FiggyError::InvalidStreamSource {
                id: id.to_owned(),
                reason,
            }
        })?;
        if let crate::StreamStatistics::Known(_) = source.statistics {
            if source.statistics == statistics {
                return Ok(());
            }
            return Err(FiggyError::InvalidStreamSource {
                id: id.to_owned(),
                reason: "complete statistics already committed for this revision",
            });
        }
        let next_fit_epoch = self.next_stream_source_fit_epoch.checked_add(1).ok_or(
            FiggyError::CounterExhausted {
                counter: "stream source fit epoch",
            },
        )?;
        let source = self
            .streaming_sources
            .get_mut(id)
            .expect("validated source remains registered");
        source.column.statistics = statistics;
        source.fit_epoch = next_fit_epoch;
        self.next_stream_source_fit_epoch = next_fit_epoch;
        Ok(())
    }

    pub(super) fn mutate_streamed_columns(
        &mut self,
        columns: Vec<crate::StreamColumn>,
        replace: bool,
    ) -> Result<()> {
        for (index, source) in columns.iter().enumerate() {
            source.validate()?;
            if let Some(kind) = self.pool.composite_kind(&source.id) {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: source.id.clone(),
                    reason: format!(
                        "the id is {kind}; streaming is not extended to records and ragged \
                         columns, which are resident only"
                    ),
                });
            }
            if self.pool.slot(&source.id).is_some()
                || columns[..index].iter().any(|s| s.id == source.id)
            {
                return Err(data_render::AllocError::DuplicateId(source.id.clone()).into());
            }
            match (replace, self.streaming_sources.get(&source.id)) {
                (false, Some(_)) => {
                    return Err(data_render::AllocError::DuplicateId(source.id.clone()).into());
                }
                (true, None) => {
                    return Err(FiggyError::UnknownColumn {
                        id: source.id.clone(),
                    });
                }
                (true, Some(old)) if source.revision <= old.revision => {
                    return Err(FiggyError::InvalidStreamSource {
                        id: source.id.clone(),
                        reason: "replacement revision must strictly increase",
                    });
                }
                _ => {}
            }
            if replace
                && self
                    .stream_revision_lineage
                    .get(&source.id)
                    .is_some_and(|last| source.revision <= *last)
            {
                return Err(FiggyError::InvalidStreamSource {
                    id: source.id.clone(),
                    reason: "replacement revision must exceed the logical stream lineage",
                });
            }
        }
        let count = u64::try_from(columns.len()).map_err(|_| FiggyError::CounterExhausted {
            counter: "stream source fit epoch",
        })?;
        let final_fit_epoch = self.next_stream_source_fit_epoch.checked_add(count).ok_or(
            FiggyError::CounterExhausted {
                counter: "stream source fit epoch",
            },
        )?;
        let invalidation =
            self.prepare_columns_invalidation(columns.iter().map(|s| s.id.as_str()))?;
        self.streaming_sources
            .try_reserve(columns.len())
            .map_err(|e| FiggyError::StateAllocationFailed {
                resource: "streaming source registry",
                reason: e.to_string(),
            })?;
        self.stream_revision_lineage
            .try_reserve(columns.len())
            .map_err(|e| FiggyError::StateAllocationFailed {
                resource: "stream revision lineage",
                reason: e.to_string(),
            })?;
        // Reserve keys before publishing metadata or chart revisions.
        let mut entries = Vec::new();
        entries.try_reserve_exact(columns.len()).map_err(|e| {
            FiggyError::StateAllocationFailed {
                resource: "streaming source entries",
                reason: e.to_string(),
            }
        })?;
        let mut fit_epoch = self.next_stream_source_fit_epoch;
        for mut source in columns {
            fit_epoch = fit_epoch
                .checked_add(1)
                .ok_or(FiggyError::CounterExhausted {
                    counter: "stream source fit epoch",
                })?;
            if source.len == 0 {
                source.statistics = crate::StreamStatistics::Known(None);
            }
            let mut key = String::new();
            key.try_reserve_exact(source.id.len()).map_err(|e| {
                FiggyError::StateAllocationFailed {
                    resource: "streaming source id",
                    reason: e.to_string(),
                }
            })?;
            key.push_str(&source.id);
            let mut lineage_key = String::new();
            lineage_key
                .try_reserve_exact(source.id.len())
                .map_err(|e| FiggyError::StateAllocationFailed {
                    resource: "stream revision lineage id",
                    reason: e.to_string(),
                })?;
            lineage_key.push_str(&source.id);
            let revision = source.revision;
            entries.push((
                key,
                lineage_key,
                revision,
                RegisteredStreamColumn {
                    column: source,
                    fit_epoch,
                    statistics_cache: StreamStatisticsCache::default(),
                },
            ));
        }
        for (key, lineage_key, revision, source) in entries {
            self.streaming_sources.insert(key, source);
            self.stream_revision_lineage.insert(lineage_key, revision);
        }
        self.next_stream_source_fit_epoch = final_fit_epoch;
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        Ok(())
    }

    /// Promote exact registered stream revisions into the resident column pool
    /// as one logical commit.
    ///
    /// A non-admissible report is returned without touching renderer state.
    /// Source writes, pool allocation, chart invalidation and picker rebuilding
    /// are all prepared before publication; any error keeps the streamed
    /// registrations, series, picker, and existing resident layout unchanged.
    #[allow(clippy::too_many_arguments)]
    pub fn promote_streamed_columns_to_resident(
        &mut self,
        sources: &[crate::StreamSourceBinding<'_>],
        already_resident_working_set_bytes: u64,
        derived_buffer_sizes: &[u64],
        transition_headroom_bytes: u64,
        working_set_limit_bytes: u64,
    ) -> Result<ResidentAdmission> {
        // Admission is deliberately repeated at the allocating boundary. The
        // earlier helper result is advisory, not a reservation.
        // Freeze every host length in this validation pass: the pool plan and
        // staging writer must not re-enter a mutable host source for sizing.
        let lengths = self.validated_streamed_promotion_lengths(sources)?;
        let report = self.resident_admission_for_validated_streamed_lengths(
            &lengths,
            already_resident_working_set_bytes,
            derived_buffer_sizes,
            transition_headroom_bytes,
            working_set_limit_bytes,
        );
        if !report.is_admissible() || sources.is_empty() {
            return Ok(report);
        }

        let invalidation =
            self.prepare_columns_invalidation(sources.iter().map(|source| source.id))?;
        let mut changed_columns = Vec::new();
        changed_columns
            .try_reserve_exact(sources.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "streamed promotion changed columns",
                reason: error.to_string(),
            })?;
        changed_columns.extend(sources.iter().map(|source| source.id));
        let mut uploads = Vec::new();
        uploads.try_reserve_exact(sources.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "streamed promotion upload plan",
                reason: error.to_string(),
            }
        })?;
        uploads.extend(sources.iter().zip(&lengths).map(|(source, &len)| {
            data_render::column_pool::StreamedColumnUpload {
                id: source.id,
                source: source.source,
                len_values: usize::try_from(len)
                    .expect("validated host length originated as usize"),
            }
        }));

        let Renderer {
            device,
            queue,
            pool,
            streaming_sources,
            chart_states,
            visual_revision,
            pending_defrag,
            picker,
            gpu_ledger,
            memory_budget,
            ..
        } = self;
        let promotion = pool.begin_streamed_promotion(
            &uploads,
            report.pool_capacity_after,
            pool_alloc_ctx(
                device.as_ref(),
                queue.as_ref(),
                *memory_budget,
                gpu_ledger.as_ref(),
            ),
        )?;
        let relocated = promotion.relocated();
        let prepared_picker = prepare_picker_for_pool_mutation(
            picker,
            promotion.pool(),
            chart_states,
            relocated,
            &changed_columns,
        )?;

        // No operation below this point allocates or can fail. Pool visibility,
        // logical-source authority and dependent derived stamps therefore
        // advance as one renderer commit.
        promotion.commit();
        for source in sources {
            let removed = streaming_sources.remove(source.id);
            debug_assert!(
                removed.is_some(),
                "validated stream source remains registered"
            );
        }
        invalidation.publish(chart_states, visual_revision);
        if let Some(prepared_picker) = prepared_picker {
            prepared_picker.commit();
        }
        if relocated {
            *pending_defrag = false;
        }
        Ok(report)
    }

    pub(super) fn prepare_resident_stream_handoff(
        &self,
        chart: ChartId,
        columns: Vec<crate::StreamColumn>,
    ) -> Result<ResidentStreamHandoff> {
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        if columns.is_empty() {
            return Err(FiggyError::InvalidStreamSource {
                id: String::new(),
                reason: "resident handoff requires the complete non-empty chart column closure",
            });
        }

        let mut supplied = std::collections::HashSet::new();
        supplied
            .try_reserve(columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident stream handoff id set",
                reason: error.to_string(),
            })?;
        for source in &columns {
            source.validate()?;
            if !supplied.insert(source.id.as_str()) {
                return Err(data_render::AllocError::DuplicateId(source.id.clone()).into());
            }
        }

        let mut referenced = std::collections::HashSet::new();
        referenced.try_reserve(columns.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "resident stream handoff chart closure",
                reason: error.to_string(),
            }
        })?;
        let mut insertion_error = None;
        for series in &state.series {
            visit_series_columns(series, &mut |id| {
                if insertion_error.is_none()
                    && let Err(error) = try_insert_column_id(
                        &mut referenced,
                        id,
                        "resident stream handoff chart closure",
                    )
                {
                    insertion_error = Some(error);
                }
            });
        }
        if let Some(error) = insertion_error {
            return Err(error);
        }
        if referenced.len() != supplied.len()
            || referenced.iter().any(|id| !supplied.contains(id.as_str()))
        {
            return Err(FiggyError::InvalidStreamSource {
                id: String::new(),
                reason: "resident handoff metadata must exactly cover every column referenced by the target chart",
            });
        }
        let closure = self.transition_column_closure(referenced.iter().map(String::as_str))?;
        if closure.len() != supplied.len()
            || closure.iter().any(|id| !supplied.contains(id.as_str()))
        {
            return Err(FiggyError::InvalidStreamSource {
                id: String::new(),
                reason: "resident handoff must cover the complete connected chart-column closure",
            });
        }

        let mut allocation_epochs = Vec::new();
        let mut expected_stream_revisions = Vec::new();
        allocation_epochs
            .try_reserve_exact(columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident stream handoff allocation epochs",
                reason: error.to_string(),
            })?;
        expected_stream_revisions
            .try_reserve_exact(columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident stream handoff source revisions",
                reason: error.to_string(),
            })?;
        for source in &columns {
            match (
                self.pool.allocation_epoch(&source.id),
                self.streaming_sources.get(&source.id),
            ) {
                (Some(epoch), None) => {
                    match self.stream_revision_lineage.get(&source.id) {
                        Some(last) if source.revision <= *last => {
                            return Err(FiggyError::InvalidStreamSource {
                                id: source.id.clone(),
                                reason: "resident handoff revision must exceed the logical stream lineage",
                            });
                        }
                        None if source.revision == 0 => {
                            return Err(FiggyError::InvalidStreamSource {
                                id: source.id.clone(),
                                reason: "initial resident handoff revision must be nonzero",
                            });
                        }
                        _ => {}
                    }
                    allocation_epochs.push((source.id.clone(), epoch));
                }
                (None, Some(current)) => {
                    if source.revision < current.revision
                        || self
                            .stream_revision_lineage
                            .get(&source.id)
                            .is_some_and(|last| source.revision < *last)
                    {
                        return Err(FiggyError::InvalidStreamSource {
                            id: source.id.clone(),
                            reason: "mixed handoff stream revision is stale",
                        });
                    }
                    if source.revision == current.revision
                        && (source.len != current.len
                            || source.encoding != current.encoding
                            || source.replay != current.replay)
                    {
                        return Err(FiggyError::InvalidStreamSource {
                            id: source.id.clone(),
                            reason: "same-revision mixed handoff metadata changed",
                        });
                    }
                    expected_stream_revisions.push((source.id.clone(), current.revision));
                }
                (Some(_), Some(_)) => {
                    return Err(FiggyError::InvalidStreamSource {
                        id: source.id.clone(),
                        reason: "column is simultaneously resident and streamed",
                    });
                }
                (None, None) => {
                    return Err(FiggyError::UnknownColumn {
                        id: source.id.clone(),
                    });
                }
            }
        }

        let mut expected_chart_revisions = Vec::new();
        expected_chart_revisions
            .try_reserve(self.chart_states.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident stream handoff chart revisions",
                reason: error.to_string(),
            })?;
        for (id, state) in &self.chart_states {
            if state.series.iter().any(|series| {
                let mut referenced = false;
                visit_series_columns(series, &mut |column| {
                    referenced |= supplied.contains(column)
                });
                referenced
            }) {
                expected_chart_revisions.push((*id, state.revisions.data, state.revisions.series));
            }
        }

        Ok(ResidentStreamHandoff {
            columns,
            allocation_epochs,
            expected_stream_revisions,
            expected_chart_revisions,
        })
    }

    /// Normalize one complete referenced-column chart closure to streamed
    /// authority as one logical commit while preserving every chart config and
    /// series.
    ///
    /// The complete metadata batch, dependent revisions, registry storage,
    /// provisional pool removal, and active-picker transition are prepared
    /// first. Any error therefore leaves the pool, stream registry, charts and
    /// picker unchanged; success publishes all ids together.
    pub fn demote_resident_columns_to_streamed(
        &mut self,
        changed_ids: &[&str],
        columns: Vec<crate::StreamColumn>,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<()> {
        if changed_ids.is_empty() && columns.is_empty() && sources.is_empty() {
            return Ok(());
        }

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
                .expect("exact closure source coverage was validated");
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
                .expect("closure column was validated streamed");
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
                debug_assert_eq!(
                    self.stream_revision_lineage.get(&source.id),
                    Some(&source.revision),
                    "registered stream revision remains the logical lineage"
                );
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

        let mutation_count = columns
            .iter()
            .filter(|source| {
                self.pool.slot(&source.id).is_some()
                    || self
                        .streaming_sources
                        .get(&source.id)
                        .is_some_and(|registered| source.revision > registered.revision)
            })
            .count();
        if mutation_count == 0 {
            return Ok(());
        }

        let count = u64::try_from(mutation_count).map_err(|_| FiggyError::CounterExhausted {
            counter: "stream source fit epoch",
        })?;
        let final_fit_epoch = self.next_stream_source_fit_epoch.checked_add(count).ok_or(
            FiggyError::CounterExhausted {
                counter: "stream source fit epoch",
            },
        )?;
        let invalidation =
            self.prepare_columns_invalidation(columns.iter().map(|source| source.id.as_str()))?;
        self.streaming_sources
            .try_reserve(mutation_count)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "streaming source registry",
                reason: error.to_string(),
            })?;
        self.stream_revision_lineage
            .try_reserve(mutation_count)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "stream revision lineage",
                reason: error.to_string(),
            })?;
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
        let mut ids = Vec::new();
        ids.try_reserve_exact(entries.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "resident demotion ids",
                reason: error.to_string(),
            }
        })?;
        ids.extend(
            entries
                .iter()
                .filter(|(_, _, _, was_resident, _)| *was_resident)
                .map(|(id, _, _, _, _)| id.as_str()),
        );
        let removed_resident = !ids.is_empty();

        let Renderer {
            pool,
            streaming_sources,
            stream_revision_lineage,
            next_stream_source_fit_epoch,
            chart_states,
            visual_revision,
            pending_defrag,
            picker,
            ..
        } = self;
        let removal = pool.begin_remove_columns(&ids)?;
        let prepared_picker = match picker.active.as_ref().map(|active| active.chart_id) {
            Some(chart_id) => {
                let state = chart_states
                    .get(&chart_id)
                    .ok_or(FiggyError::UnknownChart { id: chart_id })?;
                picker.prepare_active_chart_mutation(
                    removal.pool(),
                    chart_id,
                    &state.config,
                    &state.series,
                )?
            }
            None => None,
        };

        // All remaining operations are allocation-free and infallible.
        removal.commit();
        for (id, lineage_id, revision, was_resident, source) in entries {
            let replaced = streaming_sources.insert(id, source);
            debug_assert_eq!(
                replaced.is_some(),
                !was_resident,
                "normalization classification remains current until commit"
            );
            stream_revision_lineage.insert(lineage_id, revision);
        }
        *next_stream_source_fit_epoch = final_fit_epoch;
        invalidation.publish(chart_states, visual_revision);
        if let Some(prepared_picker) = prepared_picker {
            prepared_picker.commit();
        }
        if removed_resident {
            *pending_defrag = true;
        }
        Ok(())
    }

    /// Publish a fully rendered resident-to-streamed replacement. Every
    /// fallible allocation and dependent-state transition is prepared before
    /// the resident pool is touched, so an error leaves the old resident
    /// authority and its picker state intact.
    pub(super) fn commit_completed_resident_stream_handoff(
        &mut self,
        chart: ChartId,
        handoff: &ResidentStreamHandoff,
        statistics: &[(ColumnId, StreamStatisticsCache)],
        rendered_config: &Config,
        auto_fit_padding: Option<f64>,
    ) -> Result<RenderRevision> {
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

        let count =
            u64::try_from(handoff.columns.len()).map_err(|_| FiggyError::CounterExhausted {
                counter: "stream source fit epoch",
            })?;
        let final_fit_epoch = self.next_stream_source_fit_epoch.checked_add(count).ok_or(
            FiggyError::CounterExhausted {
                counter: "stream source fit epoch",
            },
        )?;
        let invalidation = self.prepare_columns_invalidation(
            handoff.columns.iter().map(|source| source.id.as_str()),
        )?;
        let current = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let final_desired =
            invalidation.prospective_desired_revision(chart, current.revisions.desired);
        let auto_fit_revisions = if auto_fit_padding.is_some() {
            Some((
                final_desired.successor("chart desired revision")?,
                current
                    .revisions
                    .config
                    .successor("chart config revision")?,
                current.revisions.view.successor("chart view revision")?,
                current
                    .revisions
                    .raster
                    .successor("chart raster revision")?,
                invalidation
                    .visual
                    .unwrap_or(self.visual_revision)
                    .successor("renderer visual revision")?,
            ))
        } else {
            None
        };

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
        debug_assert_eq!(fit_epoch, final_fit_epoch);

        let mut resident_ids = Vec::new();
        resident_ids
            .try_reserve_exact(entries.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident stream handoff removal ids",
                reason: error.to_string(),
            })?;
        resident_ids.extend(
            entries
                .iter()
                .filter(|(_, _, _, was_resident, _)| *was_resident)
                .map(|(id, _, _, _, _)| id.as_str()),
        );
        let removed_resident = !resident_ids.is_empty();

        let Renderer {
            pool,
            streaming_sources,
            stream_revision_lineage,
            next_stream_source_fit_epoch,
            chart_states,
            visual_revision,
            pending_defrag,
            picker,
            ..
        } = self;
        let removal = pool.begin_remove_columns(&resident_ids)?;
        let prepared_picker = match picker.active.as_ref().map(|active| active.chart_id) {
            Some(active_id) => {
                let state = chart_states
                    .get(&active_id)
                    .ok_or(FiggyError::UnknownChart { id: active_id })?;
                picker.prepare_active_chart_mutation(
                    removal.pool(),
                    active_id,
                    &state.config,
                    &state.series,
                )?
            }
            None => None,
        };

        // No operation below this point allocates or can fail.
        removal.commit();
        for (id, lineage_id, revision, was_resident, source) in entries {
            let replaced = streaming_sources.insert(id, source);
            debug_assert_eq!(replaced.is_some(), !was_resident);
            stream_revision_lineage.insert(lineage_id, revision);
        }
        *next_stream_source_fit_epoch = final_fit_epoch;
        invalidation.publish(chart_states, visual_revision);
        let revision = if let Some((desired, config, view, raster, visual)) = auto_fit_revisions {
            let state = chart_states
                .get_mut(&chart)
                .expect("resident handoff target chart remains registered");
            streaming_runtime::copy_stream_fit_axes(&mut state.config, rendered_config);
            state.stream_auto_fit_padding = None;
            state.revisions.desired = desired;
            state.revisions.config = config;
            state.revisions.view = view;
            state.revisions.raster = raster;
            *visual_revision = visual;
            desired
        } else {
            final_desired
        };
        if let Some(prepared_picker) = prepared_picker {
            prepared_picker.commit();
        }
        if removed_resident {
            *pending_defrag = true;
        }
        Ok(revision)
    }
}
