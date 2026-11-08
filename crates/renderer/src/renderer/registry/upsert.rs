//! Column upserts and replacements, and the picker hand-off around a pool mutation.

use super::*;

/// Renderer-level guard for an atomic scalar or hi/lo column upsert.
///
/// The provisional pool is available for picker/bind-group preparation.  A
/// dropped guard restores the previous column mapping; `commit` is infallible
/// and allocation-free.
#[must_use = "dropping a RendererColumnUpsert rolls the provisional pool state back"]
pub struct RendererColumnUpsert<'a> {
    pub(in crate::renderer) streaming_sources: &'a HashMap<ColumnId, RegisteredStreamColumn>,
    pub(in crate::renderer) inner: data_render::column_pool::ColumnUpsert<'a>,
    pub(in crate::renderer) chart_states: &'a mut HashMap<ChartId, ChartRenderState>,
    pub(in crate::renderer) visual_revision: &'a mut RenderRevision,
    pub(in crate::renderer) pending_defrag: &'a mut bool,
    pub(in crate::renderer) invalidation: Option<ColumnInvalidationPlan>,
    pub(in crate::renderer) picker: Option<PreparedActivePickerMutation<'a>>,
    pub(in crate::renderer) pending_defrag_after_commit: Option<bool>,
    pub(in crate::renderer) device: &'a wgpu::Device,
    pub(in crate::renderer) queue: &'a wgpu::Queue,
    pub(in crate::renderer) errorbar_extent_engine:
        Option<&'a crate::gpu_extent::GpuErrorbarExtentEngine>,
}

/// Prepared renderer side of the bundled demo transaction.
/// A prepared replacement of some columns and one chart
/// ([`Renderer::begin_replace_chart_columns`]): nothing is published until
/// [`Self::commit`], and dropping it leaves the renderer as it was.
#[must_use = "dropping a ChartColumnsReplacement leaves renderer authority unchanged"]
pub struct ChartColumnsReplacement<'a, const N: usize> {
    pub(in crate::renderer) inner: data_render::column_pool::ColumnBatchUpsert<'a, N>,
    pub(in crate::renderer) chart_states: &'a mut HashMap<ChartId, ChartRenderState>,
    pub(in crate::renderer) visual_revision: &'a mut RenderRevision,
    pub(in crate::renderer) pending_defrag: &'a mut bool,
    pub(in crate::renderer) chart_plan: ChartReplacementPlan,
    pub(in crate::renderer) picker: Option<PreparedActivePickerMutation<'a>>,
}

impl<const N: usize> ChartColumnsReplacement<'_, N> {
    /// Publish the columns, the chart, the picker and the maintenance bit
    /// together; the handles are the columns' in the order they were given.
    pub fn commit(self) -> [ColumnHandle; N] {
        let Self {
            inner,
            chart_states,
            visual_revision,
            pending_defrag,
            chart_plan,
            picker,
        } = self;
        let handles = inner.commit();
        chart_plan.publish(chart_states, visual_revision);
        if let Some(picker) = picker {
            picker.commit();
        }
        *pending_defrag = false;
        handles
    }
}

impl RendererColumnUpsert<'_> {
    pub(in crate::renderer) fn inner(&self) -> &data_render::column_pool::ColumnUpsert<'_> {
        &self.inner
    }

    pub fn pool(&self) -> &ColumnPool {
        self.inner().pool()
    }

    pub fn handle(&self) -> ColumnHandle {
        self.inner().handle()
    }

    pub fn replaced_existing(&self) -> bool {
        self.inner().replaced_existing()
    }

    /// Begin an extent job against the provisional mappings.  The column
    /// upload was submitted before this guard was returned, so queue ordering
    /// makes the new bytes visible to the reduction.
    pub fn begin_errorbar_extent(
        &self,
        value_id: &str,
        lower_id: &str,
        upper_id: &str,
    ) -> std::result::Result<
        crate::gpu_extent::GpuErrorbarExtentTicket,
        crate::gpu_extent::GpuExtentError,
    > {
        for id in [value_id, lower_id, upper_id] {
            if self.streaming_sources.contains_key(id) {
                return Err(crate::gpu_extent::GpuExtentError::ColumnNotResident {
                    id: id.to_owned(),
                });
            }
        }
        let engine = prepared_errorbar_extent_engine(self.errorbar_extent_engine)?;
        let ticket = begin_errorbar_extent_from_pool(
            engine,
            self.device,
            self.queue,
            self.pool(),
            value_id,
            lower_id,
            upper_id,
        )?;
        // The dispatch's own GPU error is this call's (#158).
        engine
            .health()
            .check()
            .map_err(crate::gpu_extent::GpuExtentError::from_figgy)?;
        Ok(ticket)
    }

    /// Begin a drawable-series fit-bound job against the provisional
    /// mappings. Inactive error directions are `None`; active directions
    /// require both lower and upper ids.
    pub fn begin_series_extent(
        &self,
        mode: crate::gpu_extent::GpuSeriesExtentMode,
        columns: crate::gpu_extent::GpuSeriesExtentColumnIds<'_>,
    ) -> std::result::Result<
        crate::gpu_extent::GpuSeriesExtentTicket,
        crate::gpu_extent::GpuExtentError,
    > {
        for id in [
            Some(columns.x),
            Some(columns.y),
            columns.x_lower,
            columns.x_upper,
            columns.y_lower,
            columns.y_upper,
        ]
        .into_iter()
        .flatten()
        {
            if self.streaming_sources.contains_key(id) {
                return Err(crate::gpu_extent::GpuExtentError::ColumnNotResident {
                    id: id.to_owned(),
                });
            }
        }
        let engine = prepared_errorbar_extent_engine(self.errorbar_extent_engine)?;
        let ticket = begin_series_extent_from_pool(
            engine,
            self.device,
            self.queue,
            self.pool(),
            mode,
            columns,
        )?;
        engine
            .health()
            .check()
            .map_err(crate::gpu_extent::GpuExtentError::from_figgy)?;
        Ok(ticket)
    }

    /// Begin the exact GPU fit domain for a complete series declaration against
    /// the provisional pool. Matrix-backed series resolve only their cell counts
    /// on the CPU; coordinate values and edge/sample arithmetic stay on the GPU.
    pub fn begin_series_fit_extent(
        &self,
        series: &SeriesConfig,
    ) -> std::result::Result<
        Option<crate::gpu_extent::GpuSeriesExtentTicket>,
        crate::gpu_extent::GpuExtentError,
    > {
        if let Err(FiggyError::ColumnNotResident { id }) =
            require_resident_sources(self.streaming_sources, series)
        {
            return Err(crate::gpu_extent::GpuExtentError::ColumnNotResident { id });
        }
        let engine = prepared_errorbar_extent_engine(self.errorbar_extent_engine)?;
        let ticket = begin_series_fit_extent_from_pool(
            engine,
            self.device,
            self.queue,
            self.pool(),
            series,
        )?;
        engine
            .health()
            .check()
            .map_err(crate::gpu_extent::GpuExtentError::from_figgy)?;
        Ok(ticket)
    }

    pub fn commit(self) -> ColumnHandle {
        let handle = self.inner.commit();
        if let Some(invalidation) = self.invalidation {
            invalidation.publish(self.chart_states, self.visual_revision);
        }
        if let Some(picker) = self.picker {
            picker.commit();
        }
        if let Some(pending) = self.pending_defrag_after_commit {
            *self.pending_defrag = pending;
        }
        handle
    }
}

/// Rebuild the active picker against a pool that has just been mutated, when it
/// has to be rebuilt.
///
/// Two reasons force it: the pool relocated its columns, so every recorded
/// offset is stale, or the picker reads one of the columns the mutation touched,
/// so its slots have to be rebuilt from the new bytes. When only the first
/// applies, `rebuild_columns` stays empty and matching slots are reused.
///
/// Shared by the single-column upsert and the batch insert so "when must the
/// picker be rebuilt" has one answer.
pub(in crate::renderer) fn prepare_picker_for_pool_mutation<'picker>(
    picker: &'picker mut RendererPicker,
    pool: &ColumnPool,
    chart_states: &HashMap<ChartId, ChartRenderState>,
    relocated: bool,
    changed_columns: &[&str],
) -> Result<Option<PreparedActivePickerMutation<'picker>>> {
    let Some((chart_id, rebuild)) = picker.active.as_ref().map(|active| {
        (
            active.chart_id,
            active.references_any_column(changed_columns),
        )
    }) else {
        return Ok(None);
    };
    if !relocated && !rebuild {
        return Ok(None);
    }
    let state = chart_states
        .get(&chart_id)
        .ok_or(FiggyError::UnknownChart { id: chart_id })?;
    let plan = PickChartPlan::for_active_layer(&state.config, &state.series)?;
    Ok(picker.prepare_active_pool_mutation(
        pool,
        chart_id,
        plan,
        if rebuild { changed_columns } else { &[] },
    )?)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::renderer) fn finish_renderer_column_upsert<'a>(
    inner: data_render::column_pool::ColumnUpsert<'a>,
    streaming_sources: &'a HashMap<ColumnId, RegisteredStreamColumn>,
    changed_column: &str,
    old_layout_generation: u64,
    chart_states: &'a mut HashMap<ChartId, ChartRenderState>,
    visual_revision: &'a mut RenderRevision,
    pending_defrag: &'a mut bool,
    picker: &'a mut RendererPicker,
    invalidation: Option<ColumnInvalidationPlan>,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    errorbar_extent_engine: Option<&'a crate::gpu_extent::GpuErrorbarExtentEngine>,
) -> Result<RendererColumnUpsert<'a>> {
    let relocated = inner.pool().layout_generation() != old_layout_generation;
    let changed_columns = [changed_column];
    let picker = prepare_picker_for_pool_mutation(
        picker,
        inner.pool(),
        &*chart_states,
        relocated,
        &changed_columns,
    )?;
    let pending_defrag_after_commit = if relocated {
        Some(false)
    } else if inner.replaced_existing() {
        Some(true)
    } else {
        None
    };

    Ok(RendererColumnUpsert {
        streaming_sources,
        inner,
        chart_states,
        visual_revision,
        pending_defrag,
        invalidation,
        picker,
        pending_defrag_after_commit,
        device,
        queue,
        errorbar_extent_engine,
    })
}

#[must_use = "dropping a PreparedColumnRemoval rolls the composite removal back"]
pub(in crate::renderer) struct PreparedColumnRemoval<'a> {
    pub(in crate::renderer) inner: data_render::column_pool::ColumnRemoval<'a>,
    pub(in crate::renderer) chart_states: &'a mut HashMap<ChartId, ChartRenderState>,
    pub(in crate::renderer) visual_revision: &'a mut RenderRevision,
    pub(in crate::renderer) pending_defrag: &'a mut bool,
    pub(in crate::renderer) removal: ColumnRemovalPlan,
    pub(in crate::renderer) picker: Option<PreparedActivePickerMutation<'a>>,
    pub(in crate::renderer) removed_column: ColumnId,
}

impl PreparedColumnRemoval<'_> {
    pub(in crate::renderer) fn commit(self) -> bool {
        self.inner.commit();
        self.removal.publish(
            self.chart_states,
            self.visual_revision,
            &self.removed_column,
        );
        if let Some(picker) = self.picker {
            picker.commit();
        }
        *self.pending_defrag = true;
        true
    }
}

pub(in crate::renderer) fn require_resident_sources(
    sources: &HashMap<ColumnId, RegisteredStreamColumn>,
    series: &SeriesConfig,
) -> Result<()> {
    let mut result = Ok(());
    visit_series_columns(series, &mut |id| {
        if result.is_ok() && sources.contains_key(id) {
            result = Err(FiggyError::ColumnNotResident { id: id.to_owned() });
        }
    });
    result
}
