//! The renderer's picker: its pipeline state, active registry and mutations.

use super::*;

pub(in crate::renderer) enum PickerPipelineState {
    Disabled,
    Ready {
        point: Arc<crate::gpu_pick::PickPipelineBundle>,
        data: Arc<crate::gpu_data_pick::DataPickPipelineBundle>,
    },
    Failed(crate::gpu_pick::GpuPickError),
}

pub(in crate::renderer) struct ActivePickRegistry {
    pub(in crate::renderer) chart_id: ChartId,
    /// The plan's descriptors the registry was built from, one per slot in
    /// slot order: the next plan reuses a slot whose descriptor it equals.
    pub(in crate::renderer) descriptors: Vec<OwnedGpuPickSeriesDescriptor>,
    pub(in crate::renderer) engine: crate::gpu_pick::GpuPickEngine,
}

impl ActivePickRegistry {
    pub(in crate::renderer) fn references_any_column(&self, ids: &[&str]) -> bool {
        self.descriptors
            .iter()
            .any(|descriptor| descriptor.resource.references_any_column(ids))
    }
}

pub(in crate::renderer) enum PreparedActivePickerMutation<'a> {
    Clear(&'a mut Option<ActivePickRegistry>),
    Update {
        transition: crate::gpu_pick::PreparedPickRegistryTransition<'a>,
        active_chart_id: &'a mut ChartId,
        active_descriptors: &'a mut Vec<OwnedGpuPickSeriesDescriptor>,
        next_chart_id: ChartId,
        next_descriptors: Vec<OwnedGpuPickSeriesDescriptor>,
    },
}

impl PreparedActivePickerMutation<'_> {
    pub(in crate::renderer) fn commit(self) {
        match self {
            Self::Clear(active) => *active = None,
            Self::Update {
                transition,
                active_chart_id,
                active_descriptors,
                next_chart_id,
                next_descriptors,
            } => {
                transition.commit();
                *active_chart_id = next_chart_id;
                *active_descriptors = next_descriptors;
            }
        }
    }
}

pub(in crate::renderer) struct RendererPicker {
    pub(in crate::renderer) pipeline_state: PickerPipelineState,
    pub(in crate::renderer) active: Option<ActivePickRegistry>,
    /// The exact pipelines created so far, by module and variant: compiled on
    /// the first pick that needs each (`PickExactPipelines`).
    pub(in crate::renderer) exact: crate::gpu_pick::PickExactPipelines,
    /// The typed data pick kernels created so far: compiled on the first
    /// pick that dispatches each (`DataPickKernels`, #327).
    pub(in crate::renderer) data_kernels: crate::gpu_data_pick::DataPickKernels,
    /// The nearest-datum pipelines (#381), created on the first nearest
    /// pick for the bundle's shader set.
    pub(in crate::renderer) nearest: Option<crate::gpu_pick::NearestPickPipelines>,
    /// The typed picks prepared for the chart picked last, one per layer
    /// (#445): kept while the key's generations hold, so a pick at another
    /// cursor reuses the series' bind groups, style maps and scratch.
    pub(in crate::renderer) typed_prepared: Vec<TypedPickCacheEntry>,
    /// The typed picks' readback ring (#445).
    pub(in crate::renderer) data_readbacks: crate::gpu_data_pick::DataPickReadbacks,
    #[cfg(test)]
    pub(in crate::renderer) injected_activation_failure: Option<crate::gpu_pick::GpuPickError>,
}

impl RendererPicker {
    pub(in crate::renderer) fn disabled() -> Self {
        Self {
            pipeline_state: PickerPipelineState::Disabled,
            active: None,
            exact: crate::gpu_pick::PickExactPipelines::default(),
            data_kernels: crate::gpu_data_pick::DataPickKernels::default(),
            nearest: None,
            typed_prepared: Vec::new(),
            data_readbacks: crate::gpu_data_pick::DataPickReadbacks::default(),
            #[cfg(test)]
            injected_activation_failure: None,
        }
    }

    /// The prepared typed pick of `key` when one is cached and no pick
    /// still holds it (allocation identity: the cache's reference is the
    /// only one), so its queries can be written for a new pick (#445).
    pub(in crate::renderer) fn reusable_typed_pick(
        &self,
        key: &TypedPickKey,
    ) -> Option<Arc<crate::gpu_data_pick::PreparedDataPick>> {
        self.typed_prepared
            .iter()
            .find(|entry| entry.key == *key)
            .filter(|entry| Arc::strong_count(&entry.prepared) == 1)
            .map(|entry| Arc::clone(&entry.prepared))
    }

    /// Cache `prepared` under `key`, replacing the entry of its layer; an
    /// entry of another chart is dropped first, so the cache holds one
    /// chart's layers at most.
    pub(in crate::renderer) fn keep_typed_pick(
        &mut self,
        key: TypedPickKey,
        prepared: &Arc<crate::gpu_data_pick::PreparedDataPick>,
    ) -> Result<()> {
        self.typed_prepared
            .retain(|entry| entry.key.chart == key.chart && entry.key.layer != key.layer);
        self.typed_prepared
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "typed pick cache",
                reason: error.to_string(),
            })?;
        self.typed_prepared.push(TypedPickCacheEntry {
            key,
            prepared: Arc::clone(prepared),
        });
        Ok(())
    }

    /// Drop every cached typed pick and readback buffer (#445); a pick
    /// still holding one keeps it until it resolves.
    pub(in crate::renderer) fn release_typed_picks(&mut self) {
        self.typed_prepared.clear();
        self.data_readbacks.clear();
    }
}

/// What a prepared typed pick was prepared for (#445): the chart (its
/// issued id) and layer, and the generations of everything its series'
/// bind groups, style maps and query templates are built from. A
/// staleness check by generation (`ARCHITECTURE.md`, "Identity and
/// staleness checks"): any change moves one of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::renderer) struct TypedPickKey {
    pub(in crate::renderer) chart: ChartId,
    pub(in crate::renderer) layer: crate::LayerRef,
    /// The chart's desired revision: its document, series and columns.
    pub(in crate::renderer) desired: RenderRevision,
    /// The renderer's visual revision: anything drawn anywhere.
    pub(in crate::renderer) visual: RenderRevision,
    /// The pool's layout generation: the buffer the bind groups read.
    pub(in crate::renderer) pool_layout: u64,
    /// The text renderer's font generation: the value label atlases.
    pub(in crate::renderer) font: u64,
    pub(in crate::renderer) set: ShaderSet,
    /// The display the pick places the document in.
    pub(in crate::renderer) display_scale: f32,
    pub(in crate::renderer) display_panel_px: Rect,
}

/// One prepared typed pick of the cache (#445).
pub(in crate::renderer) struct TypedPickCacheEntry {
    pub(in crate::renderer) key: TypedPickKey,
    pub(in crate::renderer) prepared: Arc<crate::gpu_data_pick::PreparedDataPick>,
}

impl RendererPicker {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) fn enable_observed(
        &mut self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        gpu_ledger: Arc<GpuLedger>,
        health: crate::renderer::DeviceHealth,
        pipeline_cache: Option<wgpu::PipelineCache>,
        shaders: crate::gpu_data_pick::DataPickShaders<'_>,
        transform_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        bar_style_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        category_bgl: &wgpu::BindGroupLayout,
        parallel_bgl: &wgpu::BindGroupLayout,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        match &self.pipeline_state {
            PickerPipelineState::Ready { .. } => return Ok(()),
            PickerPipelineState::Failed(error) => return Err(error.clone()),
            PickerPipelineState::Disabled => {}
        }

        #[cfg(test)]
        if let Some(error) = self.injected_activation_failure.take() {
            self.pipeline_state = PickerPipelineState::Failed(error.clone());
            return Err(error);
        }

        match crate::gpu_pick::PickPipelineBundle::new_observed(
            device.clone(),
            queue.clone(),
            Arc::clone(&gpu_ledger),
            health.clone(),
            pipeline_cache.clone(),
            shaders.set,
            observer,
        ) {
            Ok(point) => match crate::gpu_data_pick::DataPickPipelineBundle::new_observed(
                device,
                queue,
                gpu_ledger,
                health,
                pipeline_cache,
                shaders,
                transform_bgl,
                field_bgl,
                bar_style_bgl,
                vector_bgl,
                category_bgl,
                parallel_bgl,
                observer,
            ) {
                Ok(data) => {
                    self.pipeline_state = PickerPipelineState::Ready { point, data };
                    Ok(())
                }
                Err(error) => {
                    self.pipeline_state = PickerPipelineState::Failed(error.clone());
                    Err(error)
                }
            },
            Err(error) => {
                self.pipeline_state = PickerPipelineState::Failed(error.clone());
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) async fn enable_observed_async(
        &mut self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        gpu_ledger: Arc<GpuLedger>,
        health: crate::renderer::DeviceHealth,
        pipeline_cache: Option<wgpu::PipelineCache>,
        shaders: crate::gpu_data_pick::DataPickShaders<'_>,
        transform_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        bar_style_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        category_bgl: &wgpu::BindGroupLayout,
        parallel_bgl: &wgpu::BindGroupLayout,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        match &self.pipeline_state {
            PickerPipelineState::Ready { .. } => return Ok(()),
            PickerPipelineState::Failed(error) => return Err(error.clone()),
            PickerPipelineState::Disabled => {}
        }

        #[cfg(test)]
        if let Some(error) = self.injected_activation_failure.take() {
            self.pipeline_state = PickerPipelineState::Failed(error.clone());
            return Err(error);
        }

        match crate::gpu_pick::PickPipelineBundle::new_observed_async(
            device.clone(),
            queue.clone(),
            Arc::clone(&gpu_ledger),
            health.clone(),
            pipeline_cache.clone(),
            shaders.set,
            observer,
        )
        .await
        {
            Ok(point) => {
                // The data bundle creates no kernel (#327): nothing to await.
                match crate::gpu_data_pick::DataPickPipelineBundle::new_observed(
                    device,
                    queue,
                    gpu_ledger,
                    health,
                    pipeline_cache,
                    shaders,
                    transform_bgl,
                    field_bgl,
                    bar_style_bgl,
                    vector_bgl,
                    category_bgl,
                    parallel_bgl,
                    observer,
                ) {
                    Ok(data) => {
                        self.pipeline_state = PickerPipelineState::Ready { point, data };
                        Ok(())
                    }
                    Err(error) => {
                        self.pipeline_state = PickerPipelineState::Failed(error.clone());
                        Err(error)
                    }
                }
            }
            Err(error) => {
                self.pipeline_state = PickerPipelineState::Failed(error.clone());
                Err(error)
            }
        }
    }

    pub(in crate::renderer) fn ready_bundle(
        &self,
    ) -> std::result::Result<Arc<crate::gpu_pick::PickPipelineBundle>, crate::gpu_pick::GpuPickError>
    {
        match &self.pipeline_state {
            PickerPipelineState::Disabled => Err(crate::gpu_pick::GpuPickError::PickerDisabled),
            PickerPipelineState::Ready { point, .. } => Ok(Arc::clone(point)),
            PickerPipelineState::Failed(error) => Err(error.clone()),
        }
    }

    /// The exact pipelines, shared by resident and streamed picks: a
    /// streamed chunk scans with its resident series' variant.
    pub(in crate::renderer) fn exact_pipelines(
        &mut self,
    ) -> &mut crate::gpu_pick::PickExactPipelines {
        &mut self.exact
    }

    pub(in crate::renderer) fn ready_data_bundle(
        &self,
    ) -> std::result::Result<
        Arc<crate::gpu_data_pick::DataPickPipelineBundle>,
        crate::gpu_pick::GpuPickError,
    > {
        match &self.pipeline_state {
            PickerPipelineState::Disabled => Err(crate::gpu_pick::GpuPickError::PickerDisabled),
            PickerPipelineState::Ready { data, .. } => Ok(Arc::clone(data)),
            PickerPipelineState::Failed(error) => Err(error.clone()),
        }
    }

    #[cfg(test)]
    pub(in crate::renderer) fn fail_activation_once(
        &mut self,
        error: crate::gpu_pick::GpuPickError,
    ) {
        self.injected_activation_failure = Some(error);
    }

    pub(in crate::renderer) fn prepare_chart(
        &mut self,
        pool: &ColumnPool,
        chart_id: ChartId,
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let plan = PickChartPlan::for_active_layer_in(pool, config, series)?;
        self.prepare_plan(pool, chart_id, plan)
    }

    /// Makes `plan` the active registry of chart `chart_id`, reusing every
    /// slot whose descriptor it already holds.
    pub(in crate::renderer) fn prepare_plan(
        &mut self,
        pool: &ColumnPool,
        chart_id: ChartId,
        plan: PickChartPlan,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let bundle = self.ready_bundle()?;
        if self.active.as_ref().is_some_and(|active| {
            active.chart_id == chart_id && active.descriptors == plan.descriptors
        }) {
            return Ok(());
        }

        if let Some(active) = self.active.as_mut() {
            let slots = plan.registry_slots_rebuilding(Some(&active.descriptors), &[])?;
            active
                .engine
                .prepare_registry_transition(pool, slots)?
                .commit();
            active.chart_id = chart_id;
            active.descriptors = plan.descriptors;
        } else {
            let mut engine = crate::gpu_pick::GpuPickEngine::from_bundle(bundle)?;
            let slots = plan.registry_slots_rebuilding(None, &[])?;
            engine.prepare_registry_transition(pool, slots)?.commit();
            self.active = Some(ActivePickRegistry {
                chart_id,
                descriptors: plan.descriptors,
                engine,
            });
        }
        Ok(())
    }

    pub(in crate::renderer) fn prepare_active_chart_mutation<'picker>(
        &'picker mut self,
        pool: &ColumnPool,
        chart_id: ChartId,
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<
        Option<PreparedActivePickerMutation<'picker>>,
        crate::gpu_pick::GpuPickError,
    > {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.chart_id == chart_id)
        {
            let mut nonresident = false;
            for item in series {
                visit_series_columns(item, &mut |id| nonresident |= pool.slot(id).is_none());
            }
            if nonresident {
                return Ok(Some(PreparedActivePickerMutation::Clear(&mut self.active)));
            }
        }
        let plan = PickChartPlan::for_active_layer_in(pool, config, series)?;
        self.prepare_active_plan(pool, chart_id, plan, &[], false)
    }

    pub(in crate::renderer) fn prepare_active_pool_mutation<'picker>(
        &'picker mut self,
        pool: &ColumnPool,
        chart_id: ChartId,
        plan: PickChartPlan,
        rebuild_columns: &[&str],
    ) -> std::result::Result<
        Option<PreparedActivePickerMutation<'picker>>,
        crate::gpu_pick::GpuPickError,
    > {
        self.prepare_active_plan(pool, chart_id, plan, rebuild_columns, true)
    }

    pub(in crate::renderer) fn prepare_active_plan<'picker>(
        &'picker mut self,
        pool: &ColumnPool,
        chart_id: ChartId,
        plan: PickChartPlan,
        rebuild_columns: &[&str],
        force: bool,
    ) -> std::result::Result<
        Option<PreparedActivePickerMutation<'picker>>,
        crate::gpu_pick::GpuPickError,
    > {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.chart_id == chart_id)
        else {
            return Ok(None);
        };
        if !force && active.descriptors == plan.descriptors {
            return Ok(None);
        }

        let ActivePickRegistry {
            chart_id: active_chart_id,
            descriptors: active_descriptors,
            engine,
        } = active;
        let slots = plan.registry_slots_rebuilding(Some(active_descriptors), rebuild_columns)?;
        let transition = engine.prepare_registry_transition(pool, slots)?;
        Ok(Some(PreparedActivePickerMutation::Update {
            transition,
            active_chart_id,
            active_descriptors,
            next_chart_id: chart_id,
            next_descriptors: plan.descriptors,
        }))
    }
}
