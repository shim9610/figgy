//! Enabling GPU picking, preparing it for a chart, and the chart pick.

use super::*;

impl Renderer {
    /// A lost device or an unreported GPU error, as a pick error.
    pub(super) fn pick_health(&self) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.health
            .check()
            .map_err(|error| crate::gpu_pick::GpuPickError::Gpu(error.to_string()))
    }

    /// Enable exact GPU picking for this renderer.
    ///
    /// This is idempotent. The immutable compute pipeline bundle is created
    /// only by the first successful call; chart registries remain lazy.
    ///
    /// On a browser renderer that compiles in the background
    /// ([`Self::for_window_with_shader_set_async_observed`]), switching to
    /// the full shaders (a chart that needs them) leaves the picker not
    /// enabled until the new set's pick pipelines have compiled (#363), so
    /// a host picks with [`Self::enable_gpu_picking_async`] first, as
    /// `FiggyChart` does, rather than relying on an earlier enable.
    pub fn enable_gpu_picking(&mut self) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.enable_gpu_picking_unreported();
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(in crate::renderer) fn enable_gpu_picking_unreported(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let mut noop = |_| {};
        self.enable_gpu_picking_observed(&mut noop)
    }

    /// Enable exact GPU picking while reporting timestamp-free pipeline stages.
    pub fn enable_gpu_picking_observed(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.enable_gpu_picking_observed_unreported(observer);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) fn enable_gpu_picking_observed_unreported(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        self.picker.enable_observed(
            self.device.clone(),
            self.queue.clone(),
            Arc::clone(&self.gpu_ledger),
            self.health.clone(),
            crate::gpu_data_pick::DataPickShaders {
                bar: &self.pipelines.shaders.bar,
                field: &self.pipelines.shaders.field,
                vector_grid: &self.pipelines.shaders.vector_grid,
                line: &self.pipelines.shaders.line,
                set: self.pipelines.shaders.set,
            },
            &self.transform_bgl,
            &self.field_bgl,
            &self.per_point_style_map_bgl,
            &self.vector_bgl,
            &self.category_bar_data_bgl,
            &self.parallel_bgl,
            observer,
        )
    }

    /// [`Self::enable_gpu_picking`] through the browser's async compile
    /// API. On a renderer that compiles in the background it waits for the
    /// gate and reduce compiles a chart's pick warm-up scheduled (#395).
    /// Call it before each pick on such a renderer: after a switch to the
    /// full shaders the picker is not enabled until it runs, or until a
    /// frame enables it once the new set's pipelines have compiled.
    pub async fn enable_gpu_picking_async(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.enable_gpu_picking_async_unreported().await;
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) async fn enable_gpu_picking_async_unreported(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let mut noop = |_| {};
        self.enable_gpu_picking_observed_async(&mut noop).await
    }

    pub async fn enable_gpu_picking_observed_async(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self
            .enable_gpu_picking_observed_async_unreported(observer)
            .await;
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) async fn enable_gpu_picking_observed_async_unreported(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        // The gate and reduce compiles a chart's warm-up scheduled (#395)
        // are waited for, issued first if they still wait, so the
        // enable finds them compiled instead of starting its own.
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = self.compile_queue.clone()
            && matches!(
                self.picker.ready_bundle(),
                Err(crate::gpu_pick::GpuPickError::PickerDisabled)
            )
        {
            let mut warmed = true;
            for label in crate::gpu_pick::GATE_PIPELINE_LABELS {
                warmed &= queue.wait_pick(self.pipelines.shaders.set, label).await;
            }
            // Every one compiled: the bundle is created from the browser's
            // cache, with no second compile.
            if warmed {
                return self.enable_gpu_picking_observed_unreported(observer);
            }
        }
        self.picker
            .enable_observed_async(
                self.device.clone(),
                self.queue.clone(),
                Arc::clone(&self.gpu_ledger),
                self.health.clone(),
                crate::gpu_data_pick::DataPickShaders {
                    bar: &self.pipelines.shaders.bar,
                    field: &self.pipelines.shaders.field,
                    vector_grid: &self.pipelines.shaders.vector_grid,
                    line: &self.pipelines.shaders.line,
                    set: self.pipelines.shaders.set,
                },
                &self.transform_bgl,
                &self.field_bgl,
                &self.per_point_style_map_bgl,
                &self.vector_bgl,
                &self.category_bar_data_bgl,
                &self.parallel_bgl,
                observer,
            )
            .await
    }

    /// Prepare the renderer-owned picker cache for one chart.
    pub fn prepare_gpu_picking_for_chart(
        &mut self,
        id: ChartId,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.prepare_gpu_picking_for_chart_unreported(id);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) fn prepare_gpu_picking_for_chart_unreported(
        &mut self,
        id: ChartId,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        self.admit_chart_shaders(id);
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        for series in &state.series {
            if let Err(FiggyError::ColumnNotResident { id }) = self.require_resident_series(series)
            {
                return Err(crate::gpu_pick::GpuPickError::ColumnNotResident(id));
            }
        }
        let series = pickable_series(&self.pool, &state.series);
        self.picker
            .prepare_chart(&self.pool, id, &state.config, &series)
    }

    /// Compile, through the browser's async API, the pipelines a pick of
    /// chart `id` ([`Self::pick_chart`]) needs and has not created yet
    /// (#299, #381): the nearest-datum kernel of the renderer's shader set,
    /// and its 3D projection for a 3D chart. The pick then creates them
    /// from the device's compilation cache instead of compiling on the
    /// page's thread; each is created and kept, as the pick would. A
    /// compile the browser rejects is left to the pick, which creates the
    /// pipeline synchronously and reports its error the way it always does.
    /// `request` is not read: one pipeline serves every cursor.
    #[cfg(target_arch = "wasm32")]
    pub async fn compile_pick_pipelines_async(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self
            .compile_pick_pipelines_async_unreported(id, request)
            .await;
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) async fn compile_pick_pipelines_async_unreported(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let _ = request;
        let view3d = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?
            .config
            .view_3d()
            .is_some();
        let set = self.picker.ready_bundle()?.set;
        let (flat, flat_3d) = match &self.picker.nearest {
            Some(pipelines) if pipelines.set() == set => {
                (pipelines.has_flat(), pipelines.has_3d() || !view3d)
            }
            _ => (false, !view3d),
        };
        let [flat_module, module_3d] = crate::gpu_pick::NEAREST_PICK_COMPILES;
        let modules = [
            (!flat).then_some(flat_module),
            (!flat_3d).then_some(module_3d),
        ];
        for (tracked, (label, module)) in modules.into_iter().flatten() {
            // A compile the chart's warm-up scheduled is waited for instead
            // of compiling the pipeline a second time (#381).
            if let Some(queue) = self.compile_queue.clone()
                && queue.wait_pick(set, tracked).await
            {
                continue;
            }
            let compiled = crate::browser_compile::compute_with_constants(
                self.device(),
                label,
                module.of(set).source,
                "pick_nearest_points",
                &[],
            )
            .await;
            if compiled.is_err() {
                // The pick creates it synchronously and reports the same
                // failure through the device's error scope (see above).
            }
        }
        let bundle = self.picker.ready_bundle()?;
        let pipelines = match &mut self.picker.nearest {
            Some(pipelines) if pipelines.set() == set => pipelines,
            slot => slot.insert(crate::gpu_pick::NearestPickPipelines::new(set)),
        };
        pipelines.ensure(&bundle);
        if view3d {
            pipelines.ensure_3d(&bundle);
        }
        Ok(())
    }

    /// Compile, through the browser's async API, the typed data pick
    /// kernels a [`Self::pick_chart_data`] of chart `id` at `request`
    /// dispatches and has not created yet (#327), then create each, as
    /// [`Self::compile_pick_pipelines_async`] does for the nearest-datum
    /// pipelines (#299). Kernels other charts need are not compiled: each
    /// waits for the first pick that dispatches it. A compile the browser
    /// rejects is left to the creation, which reports its error the way a
    /// pick does.
    #[cfg(target_arch = "wasm32")]
    pub async fn compile_data_pick_pipelines_async(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self
            .compile_data_pick_pipelines_async_unreported(id, request)
            .await;
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) async fn compile_data_pick_pipelines_async_unreported(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        let needed = self
            .data_pick_kernels_for(id, request)
            .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        self.compile_data_pick_kernels_async(needed).await
    }

    /// Each of `needed` the picker has not created yet: compiled through
    /// `createComputePipelineAsync`, so its creation then hits the
    /// device's compilation cache instead of compiling on the page's
    /// thread, and created (#327).
    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn compile_data_pick_kernels_async(
        &mut self,
        needed: crate::gpu_data_pick::DataPickKernelSet,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let bundle = self.picker.ready_data_bundle()?;
        for kernel in needed.iter() {
            if self.picker.data_kernels.get(kernel).is_some() {
                continue;
            }
            // A compile the chart's warm-up scheduled is waited for instead
            // of compiling the kernel a second time (#381).
            if let Some(queue) = self.compile_queue.clone()
                && queue.wait_pick(bundle.set, kernel.label()).await
            {
                self.picker.data_kernels.ensure(&bundle, kernel)?;
                continue;
            }
            let compiled = crate::browser_compile::compute_with_constants(
                self.device(),
                kernel.label(),
                kernel.source(bundle.set),
                kernel.entry(),
                &[],
            )
            .await;
            if compiled.is_err() {
                // The creation below reports the same failure through the
                // device's error scope.
            }
            self.picker.data_kernels.ensure(&bundle, kernel)?;
        }
        Ok(())
    }

    /// Compile, on the browser's compile queue, the pipelines chart `id`'s
    /// first pick needs, ahead of the waiting draw compiles (#363, #381),
    /// and create each in the picker's own cache as its compile resolves,
    /// so the first pick finds them created or compiling. Called for each
    /// chart a screen frame draws. When the chart's config or series
    /// revision or the shader set moves, it schedules what the picker has
    /// not created yet: the gate and reduce pipelines while the picker is
    /// not enabled, the nearest-datum kernel of the chart's projection, and
    /// the typed data pick kernels of its series' families
    /// ([`Self::data_pick_kernels_of_chart`]). On later frames, only when a
    /// compile has ended since, it enables the picker once the gate and
    /// reduce compiles are ready and creates each pipeline whose compile is
    /// ready. A pick that comes first waits for the scheduled compiles
    /// instead of starting its own. A monotonic chart picks through its own
    /// query and a chart with a streamed series through the stream's
    /// variants (rule 8): neither is warmed. A failure leaves the pick to
    /// compile, or to report, as it always did, so nothing is reported
    /// here.
    #[cfg(target_arch = "wasm32")]
    pub(in crate::renderer) fn warm_first_pick(&mut self, id: ChartId) {
        use crate::renderer::registry::PickWarmup;
        let Some(queue) = self.compile_queue.clone() else {
            return;
        };
        let set = self.pipelines.shaders.set;
        let Some(state) = self.chart_states.get(&id) else {
            return;
        };
        let stamp = (state.revisions.config, state.revisions.series, set);
        let mut warmup = state.pick_warmup;
        if warmup.map(|w| w.stamp) != Some(stamp) {
            let pickable = !state.series.is_empty()
                && !self.monotonic_order.contains(&id)
                && state
                    .series
                    .iter()
                    .all(|series| self.require_resident_series(series).is_ok());
            let view3d = state.config.view_3d().is_some();
            let kernels = self.data_pick_kernels_of_chart(id);
            let mut scheduled = pickable && kernels.is_ok();
            if scheduled {
                // Each pick compile goes to the front of the queue: the
                // typed kernels are scheduled first so the gate, reduce and
                // nearest pipelines, which every pick needs, are issued
                // before them.
                if let Ok(kernels) = kernels {
                    for kernel in kernels.iter() {
                        if self.picker.data_kernels.get(kernel).is_none() {
                            scheduled &= queue
                                .schedule_pick_entry(
                                    set,
                                    kernel.label(),
                                    kernel.source(set),
                                    kernel.entry(),
                                    &[],
                                )
                                .is_ok();
                        }
                    }
                }
                let created = match &self.picker.nearest {
                    Some(pipelines) if pipelines.set() == set => {
                        [pipelines.has_flat(), pipelines.has_3d() || !view3d]
                    }
                    _ => [false, !view3d],
                };
                for (done, (tracked, (_, module))) in created
                    .into_iter()
                    .zip(crate::gpu_pick::NEAREST_PICK_COMPILES)
                {
                    if !done {
                        scheduled &= queue
                            .schedule_pick_entry(
                                set,
                                tracked,
                                module.of(set).source,
                                "pick_nearest_points",
                                &[],
                            )
                            .is_ok();
                    }
                }
                if matches!(
                    self.picker.ready_bundle(),
                    Err(crate::gpu_pick::GpuPickError::PickerDisabled)
                ) {
                    scheduled &= queue
                        .schedule_pick_compute(
                            set,
                            crate::gpu_pick::PickShader::Gate.assembled_for(set).source,
                            &crate::gpu_pick::GATE_PIPELINE_LABELS,
                        )
                        .is_ok();
                }
            }
            // A chart that cannot be warmed is left to its first pick,
            // which compiles or reports why, until an edit moves the stamp.
            warmup = Some(PickWarmup {
                stamp,
                adopting: scheduled.then_some(u64::MAX),
            });
        }
        let Some(PickWarmup {
            adopting: Some(seen),
            ..
        }) = warmup
        else {
            if let Some(state) = self.chart_states.get_mut(&id) {
                state.pick_warmup = warmup;
            }
            return;
        };
        let settled = queue.settled();
        let adopting = if settled == seen {
            Some(seen)
        } else {
            self.adopt_warmed_picks(id, &queue, set).then_some(settled)
        };
        if let Some(state) = self.chart_states.get_mut(&id) {
            state.pick_warmup = Some(PickWarmup { stamp, adopting });
        }
    }

    /// Create the pick pipelines of chart `id` whose warm-up compiles have
    /// resolved ([`Self::warm_first_pick`]): the picker's bundles once the
    /// gate and reduce compiles are ready, then the nearest-datum kernel and
    /// each typed data pick kernel that is ready, in the picker's caches.
    /// `true` while some compile it waits for is still pending.
    #[cfg(target_arch = "wasm32")]
    fn adopt_warmed_picks(
        &mut self,
        id: ChartId,
        queue: &crate::browser_compile::CompileQueue,
        set: crate::shader_assembly::ShaderSet,
    ) -> bool {
        use crate::compile_schedule::LabelReadiness;
        if matches!(
            self.picker.ready_bundle(),
            Err(crate::gpu_pick::GpuPickError::PickerDisabled)
        ) {
            let mut pending = false;
            let mut ready = true;
            for label in crate::gpu_pick::GATE_PIPELINE_LABELS {
                match queue.readiness(set, label) {
                    LabelReadiness::Ready => {}
                    LabelReadiness::Pending => pending = true,
                    _ => ready = false,
                }
            }
            if pending {
                return true;
            }
            if !ready || self.enable_gpu_picking_unreported().is_err() {
                // The first pick enables the picker, or reports why it
                // cannot, as it always did.
                return false;
            }
        }
        let (Ok(bundle), Ok(data_bundle)) =
            (self.picker.ready_bundle(), self.picker.ready_data_bundle())
        else {
            return false;
        };
        if bundle.set != set {
            return false;
        }
        let mut pending = false;
        let view3d = self
            .chart_states
            .get(&id)
            .is_some_and(|state| state.config.view_3d().is_some());
        let [flat, module_3d] = crate::gpu_pick::NEAREST_PICK_COMPILES;
        let pipelines = match &mut self.picker.nearest {
            Some(pipelines) if pipelines.set() == set => pipelines,
            slot => slot.insert(crate::gpu_pick::NearestPickPipelines::new(set)),
        };
        match queue.readiness(set, flat.0) {
            LabelReadiness::Ready => {
                pipelines.ensure(&bundle);
            }
            LabelReadiness::Pending => pending = true,
            _ => {}
        }
        if view3d {
            match queue.readiness(set, module_3d.0) {
                LabelReadiness::Ready => {
                    pipelines.ensure_3d(&bundle);
                }
                LabelReadiness::Pending => pending = true,
                _ => {}
            }
        }
        let Ok(kernels) = self.data_pick_kernels_of_chart(id) else {
            return pending;
        };
        for kernel in kernels.iter() {
            if self.picker.data_kernels.get(kernel).is_some() {
                continue;
            }
            match queue.readiness(set, kernel.label()) {
                LabelReadiness::Ready => {
                    if self
                        .picker
                        .data_kernels
                        .ensure(&data_bundle, kernel)
                        .is_err()
                    {
                        // The pick creates it and reports the failure.
                    }
                }
                LabelReadiness::Pending => pending = true,
                _ => {}
            }
        }
        pending
    }

    /// Submit one pick against renderer-owned chart state (#381): every
    /// series' datum nearest to the cursor ([`Self::pick_chart_nearest`]),
    /// and the series that owns the drawn ink nearest to the cursor within
    /// `request.max_distance_px` canvas pixels ([`crate::PickResult::owner`]).
    /// The hit is that series' nearest datum when it has point data; a body
    /// or fill owner answers no hit here (its typed hit is
    /// [`Self::pick_chart_data`]'s). The ink is read from each panel's ink
    /// mask, which the chart's on-screen prepare
    /// ([`Self::prepare_registered`]) draws: before the first one the pick
    /// hits nothing. [`crate::gpu_pick::GpuPickTicket::resolve`] answers the
    /// hit, [`crate::gpu_pick::GpuPickTicket::resolve_full`] the hit, the
    /// owner and every series' nearest datum.
    pub fn pick_chart(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        let result = self.pick_chart_inked(id, request);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }
}
