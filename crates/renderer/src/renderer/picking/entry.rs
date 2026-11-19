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
        // are waited for, issued first if they still wait: the last to
        // resolve enables the picker in its completion, and the enable below
        // then finds it enabled. Otherwise it compiles as it always did.
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = self.compile_queue.clone()
            && matches!(
                self.picker.ready_bundle(),
                Err(crate::gpu_pick::GpuPickError::PickerDisabled)
            )
        {
            let set = self.pipelines.shaders.set;
            for label in crate::gpu_pick::GATE_PIPELINE_LABELS {
                let _ = queue.wait_pick(set, label).await;
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

    /// Compile, through the browser's async API, the exact pick pipelines
    /// a pick of chart `id` at `request` needs and has not created yet
    /// (#299), so the pick itself creates them from the device's
    /// compilation cache instead of compiling on the page's thread. Each
    /// is then created and kept, as the pick would. A compile the browser
    /// rejects is left to the pick, which creates the pipeline
    /// synchronously and reports its error the way it always does.
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
        while let Some(key) = self.missing_exact_pick_key(id, request)? {
            let bundle = self.picker.ready_bundle()?;
            // A compile the chart's warm-up scheduled (#363) is waited for,
            // issued first if it still waits: its completion puts the
            // pipeline in the picker's cache, which the pick then reads.
            // Without one the pick compiles as it always did.
            if let Some(queue) = self.compile_queue.clone() {
                let _ = queue.wait_pick(bundle.set, &key.compile_label()).await;
            }
            if self.picker.exact.get().get(key).is_none() {
                let (constants, count) = key.constants(self.device());
                let compiled = crate::browser_compile::compute_with_constants(
                    self.device(),
                    "figgy GPU pick exact candidate pipeline",
                    key.source(bundle.set),
                    "pick_exact_candidates",
                    &constants[..count],
                )
                .await;
                if compiled.is_err() {
                    // The pick creates it synchronously below and reports the
                    // same failure through the device's error scope.
                }
            }
            let picker = &mut self.picker;
            let active = picker
                .active
                .as_ref()
                .ok_or(crate::gpu_pick::GpuPickError::Internal(
                    "successful chart preparation publishes an active picker registry",
                ))?;
            picker.exact.get_mut().ensure(active.engine.bundle(), key)?;
        }
        Ok(())
    }

    /// Compile, on the browser's compile queue, the pick pipelines chart
    /// `id`'s picks can need, ahead of the waiting compiles (#363): while
    /// the picker is not enabled its gate and reduce pipelines, then its
    /// exact keys ([`Self::for_each_exact_pick_key`]), then the typed data
    /// pick kernels the chart's series can dispatch
    /// ([`Self::data_pick_warm_kernels`]), which go first: they are the
    /// longest to compile. Each that the
    /// picker's caches do not hold yet is scheduled with a commit: its
    /// completion puts the pipeline straight into that cache when it
    /// resolves (the last of gate and reduce enables the picker), so no
    /// frame is needed after it. Called for each chart a screen frame
    /// draws; it does nothing until the chart's config or series revision
    /// or the shader set moves. Never the whole variant table: only the
    /// chart's own keys. A failure leaves the pick to compile, or to report,
    /// as it always did, so nothing is reported here.
    #[cfg(target_arch = "wasm32")]
    pub(in crate::renderer) fn warm_exact_picks(&mut self, id: ChartId) {
        let Some(queue) = self.compile_queue.clone() else {
            return;
        };
        let set = self.pipelines.shaders.set;
        let Some(state) = self.chart_states.get(&id) else {
            return;
        };
        let stamp = (state.revisions.config, state.revisions.series, set);
        if state.pick_warmup == Some(stamp) {
            return;
        }
        let mut exact_keys = false;
        let walked = self.for_each_exact_pick_key(id, |_| exact_keys = true);
        let mut pickable = walked.is_ok() && exact_keys;
        let kernels = self.data_pick_warm_kernels(id).ok();
        pickable |= kernels
            .is_some_and(|kernels| kernels != crate::gpu_data_pick::DataPickKernelSet::EMPTY);
        // A chart whose exact keys cannot be read still warms its data pick
        // kernels; its point pick compiles as it always did.
        let walked = walked.is_ok();
        if pickable {
            let pick_state = self.picker.pipeline_state.share();
            if matches!(
                self.picker.ready_bundle(),
                Err(crate::gpu_pick::GpuPickError::PickerDisabled)
            ) {
                let inputs = std::rc::Rc::new(self.picker_enable_inputs());
                if queue
                    .schedule_pick_compute(
                        set,
                        crate::gpu_pick::PickShader::Gate.assembled_for(set).source,
                        &crate::gpu_pick::GATE_PIPELINE_LABELS,
                        |own| {
                            Box::new(warm_commit::Gate {
                                state: std::rc::Rc::clone(&pick_state),
                                inputs: std::rc::Rc::clone(&inputs),
                                own,
                            })
                        },
                    )
                    .is_err()
                {
                    // The first pick enables the picker as it always did.
                }
            }
            let exact = self.picker.exact.share();
            let device = self.device.clone();
            let _ = walked
                && self
                    .for_each_exact_pick_key(id, |key| {
                        if exact.borrow().get(key).is_some() {
                            return;
                        }
                        let (constants, count) = key.constants(&device);
                        if queue
                            .schedule_exact_pick(
                                set,
                                key.compile_label(),
                                key.source(set),
                                &constants[..count],
                                Box::new(warm_commit::Exact {
                                    state: std::rc::Rc::clone(&pick_state),
                                    exact: std::rc::Rc::clone(&exact),
                                    key,
                                    set,
                                }),
                            )
                            .is_err()
                        {
                            // The first pick compiles this one as it always did.
                        }
                    })
                    .is_ok();
            // Then the typed data pick kernels the chart's series can
            // dispatch, pushed last so they are issued first: they are the
            // longest compiles, and a pick needs every one (#363).
            let data = self.picker.data_kernels.share();
            for kernel in kernels.into_iter().flat_map(|kernels| kernels.iter()) {
                if data.borrow().get(kernel).is_none()
                    && queue
                        .schedule_pick_compute(set, kernel.source(set), &[kernel.label()], |_| {
                            Box::new(warm_commit::Data {
                                state: std::rc::Rc::clone(&pick_state),
                                data: std::rc::Rc::clone(&data),
                                kernel,
                                set,
                            })
                        })
                        .is_err()
                {
                    // The first pick compiles this one as it always did.
                }
            }
        }
        if let Some(state) = self.chart_states.get_mut(&id) {
            state.pick_warmup = Some(stamp);
        }
    }

    /// What a warm-up commit enables the picker with: the inputs
    /// [`Self::enable_gpu_picking`] passes, owned (#363).
    #[cfg(target_arch = "wasm32")]
    fn picker_enable_inputs(&self) -> warm_commit::EnableInputs {
        warm_commit::EnableInputs {
            device: self.device.clone(),
            queue: self.queue.clone(),
            ledger: Arc::clone(&self.gpu_ledger),
            health: self.health.clone(),
            bar: self.pipelines.shaders.bar.clone(),
            field: self.pipelines.shaders.field.clone(),
            vector_grid: self.pipelines.shaders.vector_grid.clone(),
            line: self.pipelines.shaders.line.clone(),
            set: self.pipelines.shaders.set,
            transform_bgl: self.transform_bgl.clone(),
            field_bgl: self.field_bgl.clone(),
            bar_style_bgl: self.per_point_style_map_bgl.clone(),
            vector_bgl: self.vector_bgl.clone(),
            category_bgl: self.category_bar_data_bgl.clone(),
            parallel_bgl: self.parallel_bgl.clone(),
        }
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
        let queue = self.compile_queue.clone();
        for kernel in needed.iter() {
            if self.picker.data_kernels.get().get(kernel).is_some() {
                continue;
            }
            // A compile a chart's warm-up scheduled (#363) is waited for,
            // issued first if it still waits: its completion puts the
            // kernel in the picker's cache. Without one the kernel compiles
            // as it always did.
            if let Some(queue) = &queue {
                let _ = queue.wait_pick(bundle.set, kernel.label()).await;
            }
            if self.picker.data_kernels.get().get(kernel).is_none() {
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
            }
            self.picker.data_kernels.get_mut().ensure(&bundle, kernel)?;
        }
        Ok(())
    }

    /// The first exact pick variant a pick of chart `id` at `request`
    /// needs that is not created yet: the chart prepared and the layer
    /// routed as [`Self::pick_chart`] does.
    #[cfg(target_arch = "wasm32")]
    fn missing_exact_pick_key(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<Option<crate::gpu_pick::PickExactKey>, crate::gpu_pick::GpuPickError>
    {
        let Some(query) = self.exact_pick_query(id, request)? else {
            return Ok(None);
        };
        let picker = &self.picker;
        let active = picker
            .active
            .as_ref()
            .ok_or(crate::gpu_pick::GpuPickError::Internal(
                "successful chart preparation publishes an active picker registry",
            ))?;
        Ok(active
            .engine
            .exact_variants(&query)
            .find(|key| picker.exact.get().get(*key).is_none()))
    }

    /// The query a pick of chart `id` at `request` scans with, after the
    /// chart is prepared and the layer routed as [`Self::pick_chart`] does:
    /// its exact variants are the active registry's under it. `None` when
    /// the chart needs the full shaders and the picker's are basic (the
    /// pick reports this itself). Also the pick variant table's source
    /// (`picking/variant_table_tests.rs`, #363).
    #[cfg(any(target_arch = "wasm32", test))]
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

    /// Drop the pick pipelines no registered chart can pick with any more
    /// (#363): each exact key and typed data pick kernel that no chart's
    /// pick plan (every layer a pick can reach) uses. Run by the host's
    /// release of unused GPU memory, never on its own: a removed chart's
    /// pipelines stay until then, so the same or a similar chart registered
    /// again reuses them instead of compiling. A live chart's are never
    /// dropped. While a chart draws from a stream, whose picks scan with
    /// their resident series' variants, everything is kept. The picker's
    /// gate and reduce pipelines stay: a fixed three that every pick uses,
    /// part of the enabled picker a host keeps until it drops the renderer.
    /// A warm-up compile that resolves later may put its pipeline back,
    /// where it waits for reuse like any other, and the next release drops
    /// it again if still no chart uses it.
    pub(in crate::renderer) fn release_unused_pick_pipelines(&mut self) {
        let streamed = self.chart_states.values().any(|state| {
            state
                .series
                .iter()
                .any(|series| self.require_resident_series(series).is_err())
        });
        if streamed {
            return;
        }
        // The live keys, gathered once: one entry per series and layer.
        let mut live: Vec<crate::gpu_pick::PickExactKey> = Vec::new();
        let mut kernels = crate::gpu_data_pick::DataPickKernelSet::EMPTY;
        for &id in self.chart_states.keys() {
            if self
                .for_each_exact_pick_key(id, |key| {
                    if !live.contains(&key) {
                        live.push(key);
                    }
                })
                .is_err()
            {
                // A chart whose keys cannot be read keeps every pipeline.
                return;
            }
            match self.data_pick_warm_kernels(id) {
                Ok(needed) => kernels = kernels.union(needed),
                Err(_) => return,
            }
        }
        self.picker
            .exact
            .get_mut()
            .retain(|key| live.contains(&key));
        self.picker.data_kernels.get_mut().retain(kernels);
    }

    /// Submit one exact pick against renderer-owned chart state.
    pub fn pick_chart(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        let result = self.pick_chart_unreported(id, request);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

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
            picker.exact.get_mut().ensure(active.engine.bundle(), key)?;
        }
        let mut ticket = active.engine.pick_with_arcs(
            &self.pool,
            query,
            request.display_scale,
            &arcs,
            &rings,
            inputs.member_count(),
            &|member| inputs.member_transform(member),
            &picker.exact.get(),
        )?;
        ticket.monotonic = monotonic_rows;
        Ok(ticket)
    }
}

/// The commits of a browser renderer's pick warm-up (#363): each puts the
/// pipeline its compile resolved into the picker's own cache, which it
/// shares with the renderer (`PickCell`), checking the cache first so a key
/// has one entry however often it is warmed or picked. A commit borrows a
/// cache for one statement and never across an `.await`.
#[cfg(target_arch = "wasm32")]
mod warm_commit {
    use super::*;
    use crate::browser_compile::{CompileQueue, PickCommit};
    use crate::compile_schedule::LabelReadiness;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// The inputs the picker is enabled with, owned.
    pub(super) struct EnableInputs {
        pub(super) device: wgpu::Device,
        pub(super) queue: wgpu::Queue,
        pub(super) ledger: Arc<GpuLedger>,
        pub(super) health: crate::renderer::DeviceHealth,
        pub(super) bar: wgpu::ShaderModule,
        pub(super) field: wgpu::ShaderModule,
        pub(super) vector_grid: wgpu::ShaderModule,
        pub(super) line: wgpu::ShaderModule,
        pub(super) set: crate::shader_assembly::ShaderSet,
        pub(super) transform_bgl: wgpu::BindGroupLayout,
        pub(super) field_bgl: wgpu::BindGroupLayout,
        pub(super) bar_style_bgl: wgpu::BindGroupLayout,
        pub(super) vector_bgl: wgpu::BindGroupLayout,
        pub(super) category_bgl: wgpu::BindGroupLayout,
        pub(super) parallel_bgl: wgpu::BindGroupLayout,
    }

    /// How the warm-up's gate and reduce compiles stand, `own` (resolved,
    /// in hand) counted as ready.
    fn gate_readiness(
        queue: &CompileQueue,
        set: crate::shader_assembly::ShaderSet,
        own: Option<&str>,
    ) -> LabelReadiness {
        let mut out = LabelReadiness::Ready;
        for label in crate::gpu_pick::GATE_PIPELINE_LABELS {
            if Some(label) == own {
                continue;
            }
            match queue.readiness(set, label) {
                LabelReadiness::Ready => {}
                LabelReadiness::Pending => out = LabelReadiness::Pending,
                other => return other,
            }
        }
        out
    }

    /// One of the gate and reduce compiles: the last to resolve enables
    /// the picker, as `enable_gpu_picking` does, while the other two are
    /// still held by their completions.
    pub(super) struct Gate {
        pub(super) state: Rc<RefCell<PickerPipelineState>>,
        pub(super) inputs: Rc<EnableInputs>,
        pub(super) own: &'static str,
    }

    impl PickCommit for Gate {
        fn commit(&mut self, queue: &CompileQueue) -> bool {
            if !matches!(*self.state.borrow(), PickerPipelineState::Disabled) {
                return true;
            }
            match gate_readiness(queue, self.inputs.set, Some(self.own)) {
                LabelReadiness::Ready => {}
                LabelReadiness::Pending => return false,
                _ => return true,
            }
            let inputs = &*self.inputs;
            if inputs.health.check().is_err() {
                // The pick reports the device's state as it always did.
                return true;
            }
            let mut noop = |_| {};
            let enabled = crate::gpu_pick::PickPipelineBundle::new_observed(
                inputs.device.clone(),
                inputs.queue.clone(),
                Arc::clone(&inputs.ledger),
                inputs.health.clone(),
                inputs.set,
                &mut noop,
            )
            .and_then(|point| {
                crate::gpu_data_pick::DataPickPipelineBundle::new_observed(
                    inputs.device.clone(),
                    inputs.queue.clone(),
                    Arc::clone(&inputs.ledger),
                    inputs.health.clone(),
                    crate::gpu_data_pick::DataPickShaders {
                        bar: &inputs.bar,
                        field: &inputs.field,
                        vector_grid: &inputs.vector_grid,
                        line: &inputs.line,
                        set: inputs.set,
                    },
                    &inputs.transform_bgl,
                    &inputs.field_bgl,
                    &inputs.bar_style_bgl,
                    &inputs.vector_bgl,
                    &inputs.category_bgl,
                    &inputs.parallel_bgl,
                    &mut noop,
                )
                .map(|data| PickerPipelineState::Ready { point, data })
            });
            // A failure is the picker's, as an enable would have made it,
            // and its pick reports it.
            *self.state.borrow_mut() = enabled.unwrap_or_else(PickerPipelineState::Failed);
            true
        }
    }

    /// Where a commit that needs the picker's bundles stands: `Some(done)`
    /// when it cannot wait for them (`true`) or must (`false`), `None` once
    /// they are there.
    fn awaiting_picker(
        state: &PickerPipelineState,
        queue: &CompileQueue,
        set: crate::shader_assembly::ShaderSet,
    ) -> Option<bool> {
        match state {
            PickerPipelineState::Ready { .. } => None,
            PickerPipelineState::Failed(_) => Some(true),
            PickerPipelineState::Disabled => {
                Some(gate_readiness(queue, set, None) != LabelReadiness::Pending)
            }
        }
    }

    /// An exact key: created from the picker's bundle into its exact cache.
    pub(super) struct Exact {
        pub(super) state: Rc<RefCell<PickerPipelineState>>,
        pub(super) exact: Rc<RefCell<crate::gpu_pick::PickExactPipelines>>,
        pub(super) key: crate::gpu_pick::PickExactKey,
        pub(super) set: crate::shader_assembly::ShaderSet,
    }

    impl PickCommit for Exact {
        fn commit(&mut self, queue: &CompileQueue) -> bool {
            let point = {
                let state = self.state.borrow();
                if let Some(done) = awaiting_picker(&state, queue, self.set) {
                    return done;
                }
                match &*state {
                    PickerPipelineState::Ready { point, .. } => Arc::clone(point),
                    _ => return true,
                }
            };
            if self.exact.borrow_mut().ensure(&point, self.key).is_err() {
                // The pick creates it and reports the failure.
            }
            true
        }
    }

    /// A typed data pick kernel: created from the picker's data bundle into
    /// its kernel cache.
    pub(super) struct Data {
        pub(super) state: Rc<RefCell<PickerPipelineState>>,
        pub(super) data: Rc<RefCell<crate::gpu_data_pick::DataPickKernels>>,
        pub(super) kernel: crate::gpu_data_pick::DataPickKernel,
        pub(super) set: crate::shader_assembly::ShaderSet,
    }

    impl PickCommit for Data {
        fn commit(&mut self, queue: &CompileQueue) -> bool {
            let bundle = {
                let state = self.state.borrow();
                if let Some(done) = awaiting_picker(&state, queue, self.set) {
                    return done;
                }
                match &*state {
                    PickerPipelineState::Ready { data, .. } => Arc::clone(data),
                    _ => return true,
                }
            };
            if self.data.borrow_mut().ensure(&bundle, self.kernel).is_err() {
                // The pick creates it and reports the failure.
            }
            true
        }
    }
}
