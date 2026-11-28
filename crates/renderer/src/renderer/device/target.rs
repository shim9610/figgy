//! The renderer's target format and sample count, device accessors, and waits.

use super::*;

pub(in crate::renderer) struct MsaaTarget {
    /// Charged to the ledger for as long as this target lives; the sample
    /// count multiplies the bytes, which is why it is worth its own row.
    pub(in crate::renderer) _texture: TrackedTexture,
    pub(in crate::renderer) view: wgpu::TextureView,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::renderer) fn create_msaa_target(
    device: &wgpu::Device,
    ledger: &Arc<GpuLedger>,
    caps: RendererDeviceCaps,
    label: &'static str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> Result<Option<MsaaTarget>> {
    if sample_count <= 1 {
        return Ok(None);
    }
    validate_target_sample_count(caps, format, sample_count)?;
    validate_texture_extent(caps, label, width, height)?;
    let desc = wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };
    let texture = TrackedTexture::new(
        ledger,
        GpuResourceKind::MsaaTarget,
        create_texture_checked(device, &desc, label)?,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(Some(MsaaTarget {
        _texture: texture,
        view,
    }))
}

impl Renderer {
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn pool(&self) -> &ColumnPool {
        &self.pool
    }

    /// The device bundle this renderer was built from (#313): the same
    /// device and queue, the same device-lost and error record and the same
    /// GPU ledger and memory budget, starting on this renderer's shader set.
    /// A [`crate::radial::RadialRenderer`] (or another [`Renderer`]) built
    /// from it shares the device, its loss and the one budget with this
    /// renderer. Dropping the renderer built from it, or the bundle, never
    /// destroys the device while another holder of it remains: the device
    /// lives while any holder of it does. In the browser, the last renderer
    /// dropped on a device figgy requested itself (a windowed or headless
    /// renderer's) destroys it after its submitted work (#322).
    pub fn renderer_device(&self) -> RendererDevice {
        RendererDevice {
            device: self.device.clone(),
            queue: self.queue.clone(),
            health: self.health.clone(),
            ledger: Arc::clone(&self.gpu_ledger),
            shader_set: self.shader_set(),
            pipeline_cache: self.pipeline_cache.clone(),
        }
    }

    /// Requested bytes of tracked GPU allocations, one row per [`GpuResourceKind`],
    /// for the whole device bundle: every renderer built from this renderer's
    /// `RendererDevice` charges the same ledger, column pools included (#160).
    /// Final-owner drops move bytes to *retired*. A submission boundary
    /// captures retired resources; their bytes remain charged until GPU completion,
    /// when the handles are destroyed and their credits returned.
    ///
    /// This excludes lazy Milkyway/Constellation style textures, host-owned GPU
    /// resources, renderers built from another `RendererDevice`, CPU/output
    /// buffers and shader/pipeline/driver
    /// internals. It is not a measurement of physical VRAM residency. Streaming
    /// export admission separately checks known style-texture payload sizes.
    /// See [`Self::set_memory_budget`] for the allocation policy's scope.
    pub fn gpu_memory_usage(&self) -> GpuMemoryUsage {
        self.gpu_ledger.snapshot()
    }

    /// The device bundle's memory budget, if any; see
    /// [`Self::set_memory_budget`] for scope.
    pub fn memory_budget(&self) -> Option<u64> {
        self.gpu_ledger.budget()
    }

    /// Set the device bundle's budget used by checked allocation and admission
    /// paths: one budget for every renderer built from this renderer's
    /// `RendererDevice` (#160).
    ///
    /// **What it governs.** Checks at participating allocation paths, including
    /// pool growth. This is not a universal device allocator or a hard VRAM
    /// ceiling; see [`Self::gpu_memory_usage`] for accounting exclusions.
    /// Setting a budget never shrinks or evicts what is held, and it does *not*
    /// retroactively bound the pool's initial capacity: that slab is created
    /// during construction from the `pool_capacity_bytes` the host itself
    /// passed, before any budget exists. A pool growth that would cross the
    /// ceiling is refused as `AllocError::ResourceLimit`, the same failure the
    /// pool reports for a device ceiling.
    ///
    /// **Returns** the current tracked total when the new ceiling is below it,
    /// `None` when the total fits or the budget is disabled. The budget is
    /// applied either way, so a host can lower it before releasing
    /// resources; the returned total tells it at the call, rather than at a
    /// later failed growth, that the ceiling is already exceeded.
    #[must_use = "a returned total means the new ceiling is already exceeded"]
    pub fn set_memory_budget(&mut self, budget: Option<u64>) -> Option<u64> {
        self.gpu_ledger.set_budget(budget);
        let ceiling = budget?;
        let current = self.gpu_memory_usage().total_bytes();
        (current > ceiling).then_some(current)
    }

    /// Set the policy cap used only by renderer-selected residency changes.
    /// Native callers that never set this remain on the existing explicit
    /// promote/demote API. Returning the previous cap makes temporary host
    /// policy changes reversible without another getter.
    pub fn set_auto_resident_working_set_limit(&mut self, limit: Option<u64>) -> Option<u64> {
        let previous = std::mem::replace(&mut self.auto_resident_working_set_limit, limit);
        self.enforce_source_cache_policy();
        previous
    }

    /// Submission boundary: the host has submitted or discarded every command
    /// buffer recorded before retired owners were released. Snapshot those
    /// bytes now, but keep counting them until the queue reports completion.
    ///
    /// Split prepare/paint hosts call this once, after submitting all panels in
    /// the frame. Calling it from `prepare` would be too early because a host
    /// may still hold an unsubmitted command buffer. Idempotent.
    pub fn end_gpu_frame(&mut self) {
        // Cached bodies whose view and tokens are gone retire with this
        // frame's batch instead of waiting for the next prepare.
        self.resident_displays
            .retain(|display| display.view.strong_count() > 0);
        retire_on_completion(&self.gpu_ledger, &self.queue);
    }

    /// Service submitted GPU work and retirement callbacks without waiting.
    ///
    /// This does not submit work or report a submission boundary: call
    /// [`Self::end_gpu_frame`] after submitting or discarding recorded commands
    /// first. Native hosts must keep servicing completions even while idle if
    /// they need retired memory to become available without another submission.
    /// On WebGPU the browser delivers completion callbacks asynchronously.
    /// Polling errors are returned to the host rather than treated as completion.
    pub fn service_gpu_completions(&self) -> Result<wgpu::PollStatus> {
        gpu_poll(&self.health, &self.device, wgpu::PollType::Poll)
    }

    /// Handle onto the ledger, for resources this renderer creates but does
    /// not own (the windowed MSAA target).
    pub(crate) fn gpu_ledger(&self) -> &Arc<GpuLedger> {
        &self.gpu_ledger
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    /// Rebuild render-target pipelines if the host swap-chain format changed.
    ///
    /// Column data, chart views, textures, bind groups, styles, and buffers do
    /// not depend on the final color attachment format, so embedders can call
    /// this instead of tearing down the whole renderer state.
    pub fn ensure_target_format(&mut self, surface_format: wgpu::TextureFormat) -> Result<bool> {
        self.health.check()?;
        self.ensure_target(surface_format, self.target_sample_count)
    }

    pub(in crate::renderer) fn ensure_target(
        &mut self,
        surface_format: wgpu::TextureFormat,
        target_sample_count: u32,
    ) -> Result<bool> {
        if self.surface_format == surface_format && self.target_sample_count == target_sample_count
        {
            return Ok(false);
        }
        validate_target_sample_count(self.caps, surface_format, target_sample_count)?;
        let target_pipeline_generation =
            self.target_pipeline_generation
                .checked_add(1)
                .ok_or(FiggyError::CounterExhausted {
                    counter: "target pipeline generation",
                })?;
        // Fresh set with an empty styled cache — any styled / lazy precise
        // variants the next frame needs are recompiled by its prepare phase.
        // Recreate only the lazy variants that were already live so a format
        // change does not re-eager unused data/mapped/ring pipelines.
        let had_precise = self.pipelines.precise.each_ref().map(Option::is_some);
        let had_ternary = self.pipelines.ternary.each_ref().map(Option::is_some);
        let had_view3d = self.pipelines.view3d.each_ref().map(Option::is_some);
        // The new set keeps the old one's shader set (#299).
        let set = self.pipelines.shaders.set;
        self.pipelines = create_target_pipelines(
            &self.device,
            self.pipeline_cache.as_ref(),
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            surface_format,
            target_sample_count,
            set,
        );
        let layouts = PreciseLayouts {
            device: &self.device,
            cache: self.pipeline_cache.as_ref(),
            transform_bgl: &self.transform_bgl,
            style_bgl: &self.style_bgl,
            per_point_style_map_bgl: &self.per_point_style_map_bgl,
            point_value_map_bgl: &self.point_value_map_bgl,
            data_selection_bgl: &self.data_selection_bgl,
            field_bgl: &self.field_bgl,
            vector_bgl: &self.vector_bgl,
            category_bar_data_bgl: &self.category_bar_data_bgl,
            parallel_bgl: &self.parallel_bgl,
            star_data_bgl: &self.star_data_bgl,
            label_pipelines: self.contour_label_pipelines.as_ref(),
            surface_format,
        };
        for kind in PrecisePipeline::ALL {
            if had_precise[kind as usize] && kind.rebuilt_with_target() {
                self.pipelines.ensure_precise(kind, &layouts);
            }
        }
        for kind in TernaryPipeline::ALL {
            if had_ternary[kind as usize] {
                self.pipelines.ensure_ternary(kind, &layouts);
            }
        }
        for kind in View3dPipeline::ALL {
            if had_view3d[kind as usize] {
                self.pipelines.ensure_view3d_pass(
                    &self.device,
                    self.pipeline_cache.as_ref(),
                    [
                        &self.transform_bgl,
                        &self.style_bgl,
                        &self.per_point_style_map_bgl,
                        &self.surface_bgl,
                        &self.view3d_value_map_bgl,
                        &self.vector_bgl,
                    ],
                    surface_format,
                    kind,
                );
            }
        }
        self.surface_format = surface_format;
        self.target_sample_count = target_sample_count;
        self.target_pipeline_generation = target_pipeline_generation;
        Ok(true)
    }

    /// Which shader set this renderer's data shaders are assembled as (#299):
    /// [`ShaderSet::Basic`] until it is first handed a chart with an axis
    /// break, a probability axis or polar coordinates,
    /// [`ShaderSet::Full`] from then on.
    pub fn shader_set(&self) -> ShaderSet {
        self.pipelines.shaders.set
    }

    /// Move this renderer to at least `set` (#299). Asking for
    /// [`ShaderSet::Full`] makes it compile the full modules from now on,
    /// as the first chart with an axis break, a probability axis or polar
    /// coordinates would; a host that knows such a chart is coming calls it
    /// early, so the pipelines are warm when the chart arrives. Asking for
    /// [`ShaderSet::Basic`], or for the set the renderer is already on, does
    /// nothing: a renderer never goes back. To start on the full set
    /// without creating the basic modules at all, build the renderer with it
    /// ([`RendererDevice::with_shader_set`],
    /// [`Self::for_window_with_shader_set_async_observed`]).
    pub fn require_shader_set(&mut self, set: ShaderSet) {
        self.admit_full_shaders(set == ShaderSet::Full);
    }

    /// Move this renderer to the shader set `config` needs (#299, #311),
    /// as `set_chart_state` does when it stores a config: the full set for
    /// an axis break, a probability axis or polar coordinates on any layer,
    /// nothing otherwise. A host that installs a chart through
    /// [`Self::begin_replace_chart_sources`] calls it with the candidate
    /// config first, so the pipelines are compiled for it from the start.
    pub fn require_shaders_for(&mut self, config: &Config) {
        self.admit_full_shaders(config_needs_full_shaders(config));
    }

    /// Compile every data shader as its full module from now on, when
    /// `needed` says a chart that needs one has arrived (#299): a chart with
    /// an axis break, a probability axis or polar coordinates on any layer
    /// ([`config_needs_full_shaders`]).
    ///
    /// A new renderer assembles its data modules with the basic twins of the
    /// fragments that hold those three (`shader_assembly::SET_FRAGMENTS`),
    /// so a chart that uses none of them does not compile them. The first
    /// chart that does replaces every pipeline cache that holds a basic
    /// module: the target's pipeline set (emptied, so each pipeline is
    /// compiled again by the draw that needs it), the contour label and arc
    /// scan pipelines, and the picker's bundles and active registry. The
    /// renderer never goes back: with no cut, no probability code and no
    /// polar flag in a transform the full modules compute what the basic
    /// ones do, so a chart without them draws the same through either.
    ///
    /// Nothing a caller holds is invalidated. A [`PreparedFrame`] prepared
    /// before the switch owns its pipelines and was prepared for charts
    /// that need none of the three; the target generation is unchanged. A
    /// detached preparation begun before it is refused at installation.
    ///
    /// Called by every entry that is handed a chart: registration,
    /// `set_chart_config` and `set_chart_state`, frame preparation, export
    /// and picking.
    /// Drawing and picking also refuse such a chart on basic shaders, so a
    /// missed entry fails instead of drawing it without the feature.
    pub(in crate::renderer) fn admit_full_shaders(&mut self, needed: bool) {
        if !needed || self.pipelines.shaders.set == ShaderSet::Full {
            return;
        }
        self.pipelines = create_target_pipelines(
            &self.device,
            self.pipeline_cache.as_ref(),
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            self.surface_format,
            self.target_sample_count,
            ShaderSet::Full,
        );
        self.contour_label_pipelines = None;
        self.arc_pipelines = None;
        // A renderer that compiles its basic pipelines in the background
        // does the same for the full ones, dropping the basic compiles not
        // issued yet, so a pipeline first drawn later is not compiled cold.
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.compile_queue {
            let sources = data_render::browser_render_sources(ShaderSet::Full);
            if queue
                .switch_to(ShaderSet::Basic, ShaderSet::Full, &sources)
                .is_err()
            {
                // The draw creates each pipeline when it needs it, which
                // reports any failure.
                self.compile_queue = None;
            }
        }
        // The picker's bundles are built from the set's modules and its own
        // of the same set. A picker that was ready is ready again; its
        // registry is rebuilt by the next pick's preparation. A browser
        // renderer that compiles in the background does not compile the new
        // set's gate and reduce pipelines on the page's thread here: the
        // next frame's pick warm-up schedules them on its queue and enables
        // the picker once they have compiled, and an enable before that
        // waits for them (#363).
        let was_ready = matches!(
            self.picker.pipeline_state,
            PickerPipelineState::Ready { .. }
        );
        if was_ready {
            self.picker = RendererPicker::disabled();
            #[cfg(target_arch = "wasm32")]
            if self.compile_queue.is_some() {
                return;
            }
            if self.enable_gpu_picking_unreported().is_err() {
                // The failure is recorded in the picker's state, and every
                // later pick reports it.
            }
        }
    }

    /// [`Self::admit_full_shaders`] for the registered chart `id`; an unknown
    /// id admits nothing, and the caller reports it.
    pub(in crate::renderer) fn admit_chart_shaders(&mut self, id: ChartId) {
        let needed = self
            .chart_states
            .get(&id)
            .is_some_and(|state| config_needs_full_shaders(&state.config));
        self.admit_full_shaders(needed);
    }

    /// Block until submitted work on this device has completed.
    ///
    /// This is useful during shutdown in embedder examples, where dropping
    /// callback-owned GPU resources after the host surface/window has started
    /// tearing down can expose backend or driver lifetime bugs.
    ///
    /// No-op on wasm: the browser drives the device, blocking waits don't
    /// exist there, and the shutdown races this guards against are native
    /// driver concerns.
    ///
    /// Returns the poll's failure, such as [`FiggyError::DeviceLost`].
    pub fn wait_idle(&self) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        gpu_poll(
            &self.health,
            &self.device,
            wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            },
        )?;
        Ok(())
    }

    pub fn axis_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipelines.axis
    }

    /// Wait until previously submitted GPU work has finished executing.
    ///
    /// This does not submit a frame. Hosts that have not drawn yet should call
    /// [`WindowedRenderer::first_frame_ready`] or draw first. On native the
    /// bundle's poll thread completes the wait (see [`RendererDevice::new`]);
    /// on wasm the browser delivers `onSubmittedWorkDone`. A lost device, a
    /// failed poll or a completion callback that never runs is returned as an
    /// error (#152).
    pub async fn wait_submitted_work(&self) -> Result<()> {
        self.health.check()?;
        let mut done = self.submitted_work_done();
        let waited = done.wait(&self.health).await;
        self.submitted_work_waited(waited)
    }

    /// [`Self::wait_submitted_work`], polling the device on this thread
    /// until the work completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_submitted_work_blocking(&self) -> Result<()> {
        self.health.check()?;
        let mut done = self.submitted_work_done();
        let waited = done.wait_blocking(&self.health, &self.device, None);
        self.submitted_work_waited(waited)
    }

    /// The completion of the work submitted so far.
    pub(super) fn submitted_work_done(&self) -> crate::gpu_readback::Completion<()> {
        let (callback, done) = crate::gpu_readback::completion(&self.health);
        self.queue.on_submitted_work_done(move || callback(()));
        done
    }

    pub(super) fn submitted_work_waited(
        &self,
        waited: std::result::Result<(), crate::gpu_readback::ReadbackError>,
    ) -> Result<()> {
        match waited {
            Ok(()) => {}
            Err(crate::gpu_readback::ReadbackError::Poll(error)) => return Err(error),
            Err(_) => {
                // wgpu drops the callback without running it when the device
                // is lost; anything else is a broken completion contract.
                self.health.check()?;
                return Err(FiggyError::internal(
                    "the submitted-work callback was dropped without running",
                ));
            }
        }
        self.health.check()
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }
}

/// Whether an axis needs the full shaders (#299): its breaks cut it (the
/// axes whose transform holds a break table, `data_render::break_constants`)
/// or it has a probability scale (`data_render::probability_code`).
fn axis_needs_full_shaders(axis: &AxisOptions) -> bool {
    axis.breaks
        .as_ref()
        .is_some_and(|breaks| !breaks.ranges.is_empty())
        || matches!(axis.scale, crate::config::AxisScale::Probability(_))
}

/// Whether any layer of `config` uses a feature only the full shaders
/// compile (#299): an axis break, a probability axis or polar coordinates,
/// on the chart itself, an overlay, a subplot or a subplot's overlay. Such
/// a chart is drawn and picked with the full shaders only
/// ([`Renderer::admit_full_shaders`]). True for an expanded panel of such a
/// layer too, whose own two data axes and coordinates carry the layer's.
/// It never answers false for a config whose transform
/// [`needs_full_shaders`](data_render::ScatterTransform::needs_full_shaders).
pub(in crate::renderer) fn config_needs_full_shaders(config: &Config) -> bool {
    config.polar().is_some()
        || [
            &config.bottom_x,
            &config.left_y,
            &config.top_x,
            &config.right_y,
        ]
        .into_iter()
        .chain(
            config
                .overlays
                .iter()
                .flat_map(|overlay| [&overlay.x_axis, &overlay.y_axis]),
        )
        .any(axis_needs_full_shaders)
        || config
            .subplots
            .iter()
            .any(|subplot| config_needs_full_shaders(&subplot.config))
}
