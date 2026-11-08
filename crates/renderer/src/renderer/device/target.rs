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
    /// ceiling is refused as `AllocError::ResourceLimit` — the same failure the
    /// pool already reports for a device ceiling, so no host learns a new error
    /// shape.
    ///
    /// **Returns** the current tracked total when the new ceiling is below it,
    /// `None` when the total fits or the budget is disabled. The policy is applied either way: a
    /// host that lowers it deliberately, intending to release something next,
    /// gets what it asked for. But it learns at the call rather than at
    /// whatever growth fails later — the reason this returns anything at all.
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
        let had_line = self.pipelines.line.is_some();
        let had_line_mapped = self.pipelines.line_mapped.is_some();
        let had_line_parallel = self.pipelines.line_parallel.is_some();
        let had_line_parallel_selection = self.pipelines.line_parallel_selection.is_some();
        let had_area = self.pipelines.area.is_some();
        let had_scatter = self.pipelines.scatter.is_some();
        let had_errorbar = self.pipelines.errorbar.is_some();
        let had_scatter_mapped = self.pipelines.scatter_mapped.is_some();
        let had_pick_ring = self.pipelines.pick_ring.is_some();
        let had_pick_ring_mapped = self.pipelines.pick_ring_mapped.is_some();
        let had_ternary = self.pipelines.ternary.each_ref().map(Option::is_some);
        let had_view3d = self.pipelines.view3d.each_ref().map(Option::is_some);
        let had_errorbar_mapped = self.pipelines.errorbar_mapped.is_some();
        let had_errorbar_stem = self.pipelines.errorbar_stem.is_some();
        let had_bar = self.pipelines.bar.is_some();
        let had_bar_mapped = self.pipelines.bar_mapped.is_some();
        self.pipelines = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            surface_format,
            target_sample_count,
        );
        if had_line {
            self.pipelines.line = Some(
                data_render::create_line_columnar_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.line,
                    &self.transform_bgl,
                    &self.style_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        if had_line_mapped {
            self.pipelines.line_mapped =
                Some(data_render::create_line_mapped_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.line,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.point_value_map_bgl,
                    surface_format,
                    target_sample_count,
                ));
        }
        for (had, selection) in [
            (had_line_parallel, false),
            (had_line_parallel_selection, true),
        ] {
            if had {
                let pipeline = data_render::create_line_parallel_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.line,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.parallel_bgl,
                    surface_format,
                    target_sample_count,
                    selection,
                );
                if selection {
                    self.pipelines.line_parallel_selection = Some(pipeline);
                } else {
                    self.pipelines.line_parallel = Some(pipeline);
                }
            }
        }
        if had_area {
            self.pipelines.area = Some(data_render::create_line_area_pipeline_with_sample_count(
                &self.device,
                &self.pipelines.shaders.line,
                &self.transform_bgl,
                &self.style_bgl,
                surface_format,
                target_sample_count,
            ));
        }
        if had_scatter {
            self.pipelines.scatter = Some(
                data_render::create_scatter_columnar_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.scatter,
                    &self.transform_bgl,
                    &self.style_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        if had_bar {
            self.pipelines.bar = Some(data_render::create_bar_columnar_pipeline_with_sample_count(
                &self.device,
                &self.pipelines.shaders.bar,
                &self.transform_bgl,
                &self.style_bgl,
                surface_format,
                target_sample_count,
            ));
        }
        if had_bar_mapped {
            self.pipelines.bar_mapped = Some(
                data_render::create_bar_columnar_mapped_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.bar,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.per_point_style_map_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        if had_errorbar {
            self.pipelines.errorbar = Some(
                data_render::create_errorbar_columnar_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.errorbar,
                    &self.transform_bgl,
                    &self.style_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        if had_errorbar_stem {
            self.pipelines.errorbar_stem = Some(data_render::create_stem_pipeline(
                &self.device,
                &self.pipelines.shaders.errorbar,
                &self.transform_bgl,
                &self.style_bgl,
                surface_format,
                target_sample_count,
                "vs_stem",
            ));
        }
        if had_scatter_mapped {
            self.pipelines.scatter_mapped = Some(
                data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                    &self.device,
                    &self.pipelines.shaders.scatter,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.per_point_style_map_bgl,
                    &self.point_value_map_bgl,
                    surface_format,
                    target_sample_count,
                    "vs_mapped",
                    "fs_mapped",
                    "figgy scatter mapped pipeline",
                ),
            );
        }
        if had_pick_ring {
            self.pipelines.pick_ring =
                Some(data_render::create_scatter_columnar_pipeline_with_entries(
                    &self.device,
                    &self.pipelines.shaders.scatter,
                    &self.transform_bgl,
                    &self.style_bgl,
                    surface_format,
                    target_sample_count,
                    "vs_pick_ring",
                    "fs_pick_ring",
                    "figgy picked point ring pipeline",
                ));
        }
        for kind in TernaryPipeline::ALL {
            if had_ternary[kind as usize] {
                self.pipelines.ternary[kind as usize] = Some(create_ternary_pipeline(
                    kind,
                    &self.device,
                    &self.pipelines.shaders,
                    [
                        &self.transform_bgl,
                        &self.style_bgl,
                        &self.per_point_style_map_bgl,
                        &self.point_value_map_bgl,
                    ],
                    surface_format,
                    target_sample_count,
                ));
            }
        }
        for kind in View3dPipeline::ALL {
            if had_view3d[kind as usize] {
                self.pipelines.view3d[kind as usize] = Some(create_view3d_pipeline(
                    kind,
                    &self.device,
                    &self.pipelines.shaders,
                    [
                        &self.transform_bgl,
                        &self.style_bgl,
                        &self.per_point_style_map_bgl,
                        &self.surface_bgl,
                        &self.view3d_value_map_bgl,
                        &self.vector_bgl,
                    ],
                    surface_format,
                    target_sample_count,
                ));
            }
        }
        if had_pick_ring_mapped {
            self.pipelines.pick_ring_mapped = Some(
                data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                    &self.device,
                    &self.pipelines.shaders.scatter,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.per_point_style_map_bgl,
                    &self.point_value_map_bgl,
                    surface_format,
                    target_sample_count,
                    "vs_pick_ring_mapped",
                    "fs_pick_ring",
                    "figgy picked point mapped ring pipeline",
                ),
            );
        }
        if had_errorbar_mapped {
            self.pipelines.errorbar_mapped = Some(
                data_render::create_errorbar_columnar_mapped_pipeline_from_shader(
                    &self.device,
                    &self.pipelines.shaders.errorbar,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.per_point_style_map_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        self.surface_format = surface_format;
        self.target_sample_count = target_sample_count;
        self.target_pipeline_generation = target_pipeline_generation;
        Ok(true)
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

    pub fn texture_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.texture_bgl
    }

    pub fn transform_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.transform_bgl
    }

    pub fn style_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.style_bgl
    }

    pub fn axis_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipelines.axis
    }

    pub fn line_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.line.as_ref()
    }

    pub fn scatter_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.scatter.as_ref()
    }

    pub fn errorbar_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.errorbar.as_ref()
    }

    pub fn bar_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.bar.as_ref()
    }

    pub fn bar_mapped_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.bar_mapped.as_ref()
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

    pub fn quad_vertex_buffer(&self) -> &wgpu::Buffer {
        &self.quad_vb
    }
}
