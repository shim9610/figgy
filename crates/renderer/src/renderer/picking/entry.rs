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
    /// gate and reduce compiles a chart's pick warm-up scheduled (#363).
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
        // The gate and reduce compiles a chart's warm-up scheduled (#363)
        // are waited for, issued first if they still wait, so the enable
        // below finds them compiled instead of starting its own.
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = self.compile_queue.clone()
            && matches!(
                self.picker.ready_bundle(),
                Err(crate::gpu_pick::GpuPickError::PickerDisabled)
            )
        {
            for label in crate::gpu_pick::GATE_PIPELINE_LABELS {
                let _ = queue.wait_pick(self.pipelines.shaders.set, label).await;
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
        let modules = [
            (!flat).then_some(crate::gpu_pick::NEAREST_PICK_MODULE),
            (!flat_3d).then_some(crate::gpu_pick::NEAREST_PICK_3D_MODULE),
        ];
        for (label, module) in modules.into_iter().flatten() {
            // A rejected compile is left to the pick (see above).
            let _ = crate::browser_compile::compute_with_constants(
                self.device(),
                label,
                module.of(set).source,
                "pick_nearest_points",
                &[],
            )
            .await;
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

    /// Compile, on the browser's compile queue, the pick pipelines chart
    /// `id`'s first pick needs, ahead of the waiting compiles (#363), and
    /// create each one in the picker's own cache as its compile resolves, so
    /// the first pick finds them created. Called for each chart a screen
    /// frame draws. When the chart's config or series revision or the shader
    /// set moves, it schedules, while the picker is not enabled, its gate
    /// and reduce pipelines, then the chart's exact keys
    /// ([`Self::for_each_exact_pick_key`]), which go first. On later
    /// frames, only when a compile has ended since, it enables the picker
    /// once the gate and reduce compiles are ready and creates each exact
    /// pipeline whose compile is ready. Never the whole variant table: only
    /// the chart's own keys. A failure leaves the pick to compile, or to
    /// report, as it always did, so nothing is reported here.
    #[cfg(target_arch = "wasm32")]
    pub(in crate::renderer) fn warm_exact_picks(&mut self, id: ChartId) {
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
            // The gate and reduce compiles first, and only for a chart a
            // pick can reach: each pick compile goes to the front, so the
            // exact keys, the longest, are issued before them.
            let mut pickable = false;
            let walked = self.for_each_exact_pick_key(id, |_| pickable = true);
            if pickable
                && matches!(
                    self.picker.ready_bundle(),
                    Err(crate::gpu_pick::GpuPickError::PickerDisabled)
                )
                && queue
                    .schedule_pick_compute(
                        set,
                        crate::gpu_pick::PickShader::Gate.assembled_for(set).source,
                        &crate::gpu_pick::GATE_PIPELINE_LABELS,
                    )
                    .is_err()
            {
                // The first pick enables the picker as it always did.
            }
            let device = self.device.clone();
            let scheduled = self.for_each_exact_pick_key(id, |key| {
                let (constants, count) = key.constants(&device);
                if queue
                    .schedule_exact_pick(
                        set,
                        key.compile_label(),
                        key.source(set),
                        &constants[..count],
                    )
                    .is_err()
                {
                    // The first pick compiles this one as it always did.
                }
            });
            // A chart whose keys cannot be walked is left to its first pick,
            // which reports why, until an edit moves the stamp.
            warmup = Some(PickWarmup {
                stamp,
                adopting: (walked.is_ok() && scheduled.is_ok() && pickable).then_some(u64::MAX),
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
    /// resolved ([`Self::warm_exact_picks`]): the picker's bundle once its
    /// gate and reduce compiles are ready, then each exact pipeline that is
    /// ready, in the picker's exact cache. `true` while some compile it
    /// waits for is still pending.
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
        let Ok(bundle) = self.picker.ready_bundle() else {
            return false;
        };
        // The cache is moved out while the chart's keys are walked, which
        // borrow the renderer, and moved back: nothing is copied.
        let mut exact = std::mem::take(&mut self.picker.exact);
        let mut pending = false;
        let walked = self.for_each_exact_pick_key(id, |key| {
            if exact.get(key).is_some() {
                return;
            }
            match queue.readiness(set, &key.compile_label()) {
                LabelReadiness::Ready => {
                    if exact.ensure(&bundle, key).is_err() {
                        // The pick creates it and reports the failure.
                    }
                }
                LabelReadiness::Pending => pending = true,
                _ => {}
            }
        });
        self.picker.exact = exact;
        pending && walked.is_ok()
    }

    /// Compile, through the browser's async API, the typed data pick
    /// kernels a [`Self::pick_chart_data`] of chart `id` at `request`
    /// dispatches and has not created yet (#327), then create each, as
    /// [`Self::compile_pick_pipelines_async`] does for the exact point
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

    /// The query a pick of chart `id` at `request` scans with, after the
    /// chart is prepared and the layer routed as [`Self::pick_chart`] does:
    /// its exact variants are the active registry's under it. `None` when
    /// the chart needs the full shaders and the picker's are basic (the
    /// pick reports this itself). Also the pick variant table's source
    /// (`picking/variant_table_tests.rs`, #363).
    #[cfg(test)]
    pub(super) fn exact_pick_query(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<Option<crate::gpu_pick::GpuPickQuery>, crate::gpu_pick::GpuPickError>
    {
        self.pick_health()?;
        self.admit_chart_shaders(id);
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        let mut displayed = place_document(
            &state.config,
            request.display_scale,
            request.display_panel_px,
        );
        let routed = subplot_route(&state.config, &displayed, request.canvas_position_px);
        if let Some(config) = &routed {
            displayed.active_layer = config.active_layer;
        }
        for series in &state.series {
            if let Err(FiggyError::ColumnNotResident { id }) = self.require_resident_series(series)
            {
                return Err(crate::gpu_pick::GpuPickError::ColumnNotResident(id));
            }
        }
        let series = pickable_series(&self.pool, &state.series);
        self.picker.prepare_chart(
            &self.pool,
            id,
            routed.as_ref().unwrap_or(&state.config),
            &series,
        )?;
        let inputs = active_layer_pick_inputs(&displayed, &series)?;
        let display_config = &inputs.panel;
        let transform = data_render::scatter_transform_from_config(display_config);
        if transform.needs_full_shaders() && self.picker.ready_bundle()?.set == ShaderSet::Basic {
            // The pick reports this itself.
            return Ok(None);
        }
        Ok(Some(crate::gpu_pick::GpuPickQuery {
            transform,
            chart_rect_px: [0.0; 4],
            data_area_px: None,
            canvas_position_px: request.canvas_position_px,
            max_distance_px: request.max_distance_px,
            draw_style: crate::gpu_pick::GpuPickDrawStyle::of(&display_config.draw_style),
        }))
    }

    /// Each exact pick pipeline a pick of chart `id` can need, given to
    /// `each`, possibly more than once: for every layer a pick can reach (the
    /// active one and, from the base layer, each subplot the pointer can
    /// route to, as [`subplot_route`] does), each resident series' stored key
    /// from its pick descriptor ([`crate::gpu_pick::resident_exact_key`]) with
    /// the layer's query bits. Read from the stored config and series alone:
    /// no picker, slot or GPU object is needed, so a chart's pipelines can
    /// compile before its first pick (#363). A monotonic chart picks through
    /// its own query and a chart with a streamed series through the stream's
    /// variants (rule 8): neither gives any.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(in crate::renderer) fn for_each_exact_pick_key(
        &self,
        id: ChartId,
        mut each: impl FnMut(crate::gpu_pick::PickExactKey),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        if self.monotonic_order.contains(&id)
            || state
                .series
                .iter()
                .any(|series| self.require_resident_series(series).is_err())
        {
            return Ok(());
        }
        let series = pickable_series(&self.pool, &state.series);
        let mut layer =
            |config: &Config| -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
                let inputs = active_layer_pick_inputs(config, &series)?;
                let plan = PickChartPlan::new(&inputs.panel, &inputs.series)?;
                let transform = data_render::scatter_transform_from_config(&inputs.panel);
                let draw_style = crate::gpu_pick::GpuPickDrawStyle::of(&inputs.panel.draw_style);
                let polar = transform.polar_angle[0] > 0.5;
                for descriptor in &plan.descriptors {
                    let (key, flags) =
                        crate::gpu_pick::resident_exact_key(&descriptor.descriptor());
                    each(crate::gpu_pick::exact_variant_for(
                        key,
                        flags,
                        &draw_style,
                        polar,
                    ));
                }
                Ok(())
            };
        layer(&state.config)?;
        if state.config.active_layer() == crate::LayerRef::Base {
            for index in 0..state.config.subplots.len() {
                let mut routed = state.config.clone();
                routed.active_layer = crate::LayerRef::Subplot(index);
                layer(&routed)?;
            }
        }
        Ok(())
    }

    /// Submit one pick against renderer-owned chart state (#381): every
    /// series' datum nearest to the cursor ([`Self::pick_chart_nearest`]),
    /// and a hit when the chart's drawn data layer has ink within
    /// `request.max_distance_px` canvas pixels of the cursor. The ink is
    /// read from each panel's ink mask, which the chart's on-screen prepare
    /// ([`Self::prepare_registered`]) draws: before the first one, or after
    /// [`Self::release_unused_pool_memory`], the pick hits nothing until
    /// the next. [`crate::gpu_pick::GpuPickTicket::resolve`] answers the
    /// hit, [`crate::gpu_pick::GpuPickTicket::resolve_full`] the hit and
    /// every series' nearest datum.
    pub fn pick_chart(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        let result = self.pick_chart_inked(id, request);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    /// The exact point query of a registered chart: the point part of the
    /// typed data pick ([`Self::pick_chart_data`]) until it moves to the
    /// nearest-datum rule (#381 phase 4). [`Self::pick_chart`] runs the
    /// nearest-datum pick and its ink hit instead.
    pub(super) fn pick_chart_unreported(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        // A chart with an axis break, a probability axis or polar
        // coordinates is picked with the full shaders only (#299): the picker's bundles follow the renderer's shader set.
        self.admit_chart_shaders(id);
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        let mut displayed = place_document(
            &state.config,
            request.display_scale,
            request.display_panel_px,
        );
        // On the base layer a pointer over a subplot's frame belongs to the
        // subplot drawn on top there (#205), inside the parent chart or
        // outside it: the query runs on that subplot's layer, its registry
        // prepared from the same routed config.
        let routed = subplot_route(&state.config, &displayed, request.canvas_position_px);
        if let Some(config) = &routed {
            displayed.active_layer = config.active_layer;
        }
        for series in &state.series {
            if let Err(FiggyError::ColumnNotResident { id }) = self.require_resident_series(series)
            {
                return Err(crate::gpu_pick::GpuPickError::ColumnNotResident(id));
            }
        }
        // A monotonic series picks the envelope its columns hold, which
        // every call that moves its mapping rebuilds (#271); its rows map
        // back through it.
        let series = pickable_series(&self.pool, &state.series);
        let monotonic_rows = self
            .monotonic
            .pick_rows(&series)
            .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        self.picker.prepare_chart(
            &self.pool,
            id,
            routed.as_ref().unwrap_or(&state.config),
            &series,
        )?;
        let inputs = active_layer_pick_inputs(&displayed, &series)?;
        let (display_config, layer_series) = (&inputs.panel, &inputs.series);
        let (request, moved) = pick_request_outside_gaps(display_config, request);
        let transform = data_render::scatter_transform_from_config(display_config);
        if transform.needs_full_shaders() && self.picker.ready_bundle()?.set == ShaderSet::Basic {
            return Err(crate::gpu_pick::GpuPickError::Internal(
                "a chart with an axis break, a probability axis or polar coordinates is picked with the full shaders",
            ));
        }
        // The line offset members are the last registered series, each
        // drawn (and its arc prefix scanned) at its own panel's transform.
        let first_member = layer_series.len() - inputs.member_count();
        // The arc prefix each registered line was drawn with at this
        // transform, by registry position: its dash phase and sketch wobble
        // walk it. The draw's own cache key, so a pick after a frame reuses
        // the frame's scan and a pick before one runs the same scan.
        let slots = self
            .picker
            .active
            .as_ref()
            .ok_or(crate::gpu_pick::GpuPickError::Internal(
                "successful chart preparation publishes an active picker registry",
            ))?
            .engine
            .series_identities();
        let mut arcs: Vec<Option<wgpu::Buffer>> = Vec::new();
        arcs.try_reserve_exact(slots.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick arc prefixes",
            }
        })?;
        // A region series' ring starts, the scan its draw reads (#49).
        let mut rings: Vec<Option<wgpu::Buffer>> = Vec::new();
        rings.try_reserve_exact(slots.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick ring starts",
            }
        })?;
        for series_id in slots.series_ids() {
            let position = layer_series
                .iter()
                .position(|series| series.series_id == series_id);
            let member_transform = position
                .and_then(|position| position.checked_sub(first_member))
                .map(|member| inputs.member_transform(member));
            let series_transform = member_transform.as_ref().unwrap_or(&transform);
            let series = position.map(|position| &layer_series[position]);
            let arc = series
                .filter(|series| line_pick_walks_arc(&display_config.draw_style, series))
                .map(|series| {
                    let interpolation = extract_line(&series.render_type)
                        .map_or(LineInterpolation::Linear, |line| line.interpolation);
                    // A streamline's arrowheads scan with their slot pitch,
                    // the draw's key at the same display scale.
                    self.preparation_parts().1.ensure_arc_prefix(
                        &series.series_id,
                        &series.x_column,
                        &series.y_column,
                        match &series.render_type {
                            crate::data_config::DataRenderType::Ternary(
                                crate::data_config::TernarySeries { c_column, .. },
                            ) => Some(super::prepare::ArcThird::Ternary(c_column.as_str())),
                            _ => None,
                        },
                        series_transform,
                        super::prepare::streamline_arrow_pitch(
                            &series.render_type,
                            request.display_scale,
                        ),
                        interpolation,
                    )
                })
                .transpose()
                // The scan the draw runs fails the pick with its message.
                .map_err(|error| crate::gpu_pick::GpuPickError::Gpu(error.to_string()))?
                .flatten()
                .map(|arc| arc.prefix.0);
            arcs.push(arc);
            let ring = match series {
                // The scan the draw runs: its failure (a lane past the
                // 32-bit range, rows past the dispatch grid) fails the pick
                // with the draw's message.
                Some(series) => self
                    .preparation_parts()
                    .1
                    .region_ring_starts(display_config, series)
                    .map_err(|error| crate::gpu_pick::GpuPickError::Gpu(error.to_string()))?,
                None => None,
            };
            rings.push(ring);
        }
        let chart_rect = display_config.chart_area.0;
        let data_area_px = display_config.data_area().ok().map(|area| {
            let rect = area.0;
            [
                rect.x as f32 + moved[0],
                rect.y as f32 + moved[1],
                rect.width as f32,
                rect.height as f32,
            ]
        });
        let query = crate::gpu_pick::GpuPickQuery {
            transform,
            chart_rect_px: [
                chart_rect.x as f32,
                chart_rect.y as f32,
                chart_rect.width as f32,
                chart_rect.height as f32,
            ],
            data_area_px,
            canvas_position_px: request.canvas_position_px,
            max_distance_px: request.max_distance_px,
            draw_style: crate::gpu_pick::GpuPickDrawStyle::of(&display_config.draw_style),
        };
        let picker = &mut self.picker;
        let active = picker
            .active
            .as_ref()
            .ok_or(crate::gpu_pick::GpuPickError::Internal(
                "successful chart preparation publishes an active picker registry",
            ))?;
        if active.chart_id != id {
            return Err(crate::gpu_pick::GpuPickError::Internal(
                "the active picker registry is the prepared chart's",
            ));
        }
        // The exact pipelines these series need, compiled on the first pick
        // that needs each.
        for key in active.engine.exact_variants(&query) {
            picker.exact.ensure(active.engine.bundle(), key)?;
        }
        let mut ticket = active.engine.pick_with_arcs(
            &self.pool,
            query,
            request.display_scale,
            &arcs,
            &rings,
            inputs.member_count(),
            &|member| inputs.member_transform(member),
            &picker.exact,
        )?;
        ticket.monotonic = monotonic_rows;
        Ok(ticket)
    }
}
