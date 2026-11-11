//! A renderer bound to a window surface: WindowedRenderer and the
//! for_window constructors.

use super::*;

/// Returned by `Renderer::for_window`. The caller never touches the surface,
/// surface config, encoder, or render pass — just calls `draw` per frame.
///
/// This type owns what the surface needs: its size and format (`resize`),
/// the frame draws (`draw`, `draw_registered`, `draw_prepared`,
/// `first_frame_ready`) and the picks placed in the surface
/// (`pick_chart_at`, `pick_chart_data_at`). Everything else is the
/// [`Renderer`]'s, reached through `Deref` and `DerefMut`; every `Renderer`
/// call reports its own GPU error, so nothing is forwarded by hand (#160).
/// [`Renderer::ensure_target_format`] changes the pipelines' format, not
/// the surface's; `resize` sets them back to the surface's format.
pub struct WindowedRenderer<'w> {
    pub(super) inner: Renderer,
    pub(super) surface: wgpu::Surface<'w>,
    pub(super) surface_config: wgpu::SurfaceConfiguration,
    pub(super) msaa_target: Option<MsaaTarget>,
    /// Instance and adapter must outlive the surface; keep them pinned here.
    pub(super) _instance: wgpu::Instance,
    pub(super) _adapter: wgpu::Adapter,
}

impl<'w> WindowedRenderer<'w> {
    /// Build a standalone renderer while reporting timestamp-free startup
    /// stage boundaries to `observer`.
    pub async fn for_window_async_observed(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self> {
        Renderer::for_window_async_observed(target, size, pool_capacity_bytes, observer).await
    }

    /// [`Self::for_window_async_observed`] on native, without an executor.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn for_window_blocking_observed(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self> {
        Renderer::for_window_blocking_observed(target, size, pool_capacity_bytes, observer)
    }

    pub fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    /// Pick one chart using this window's current physical surface size.
    pub fn pick_chart_at(
        &mut self,
        id: ChartId,
        canvas_position_px: [f32; 2],
        max_distance_px: f32,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        let config = self
            .inner
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        let logical = config.config.document_area();
        let (display_scale, display_panel_px) = fit_display_panel(
            (logical.width, logical.height),
            (self.surface_config.width, self.surface_config.height),
        );
        self.inner.pick_chart(
            id,
            GpuPickRequest {
                canvas_position_px,
                display_panel_px,
                display_scale,
                max_distance_px,
            },
        )
    }

    /// Typed data pick using this window's current physical surface size.
    pub fn pick_chart_data_at(
        &mut self,
        id: ChartId,
        canvas_position_px: [f32; 2],
        max_distance_px: f32,
    ) -> Result<crate::GpuDataPickTicket> {
        let config = self
            .inner
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let logical = config.config.document_area();
        let (display_scale, display_panel_px) = fit_display_panel(
            (logical.width, logical.height),
            (self.surface_config.width, self.surface_config.height),
        );
        self.inner.pick_chart_data(
            id,
            GpuPickRequest {
                canvas_position_px,
                display_panel_px,
                display_scale,
                max_distance_px,
            },
        )
    }

    /// Reconfigure the swap chain after a window resize. The caller is then
    /// responsible for updating each chart's `chart_area` — figgy doesn't
    /// know your panel layout policy.
    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        self.inner.health.check()?;
        data_render::reconfigure_surface(
            &self.surface,
            &self._adapter,
            self.inner.device(),
            &mut self.surface_config,
            w.max(1),
            h.max(1),
        )?;
        let preferred_sample_count =
            preferred_msaa_sample_count(self.inner.caps, self.surface_config.format);
        let (target_sample_count, msaa_target) = match create_msaa_target(
            self.inner.device(),
            self.inner.gpu_ledger(),
            self.inner.caps,
            "figgy frame msaa target",
            self.surface_config.width,
            self.surface_config.height,
            self.surface_config.format,
            preferred_sample_count,
        ) {
            Ok(target) => (preferred_sample_count, target),
            Err(_) if preferred_sample_count > 1 => (1, None),
            Err(e) => return Err(e),
        };
        self.inner
            .ensure_target(self.surface_config.format, target_sample_count)?;
        self.msaa_target = msaa_target;
        // An error the reconfiguration sent to the uncaptured-error handler.
        self.inner.health.check()
    }

    /// Draw one frame.
    ///
    /// Runs the mutable prepare phase (`prepare(items)`), then acquires the
    /// current swap-chain texture, builds an encoder + render pass, records
    /// via `paint_prepared`, and submits and presents. The caller is only responsible for processing per-panel
    /// dirty flags (`chart.consume_*_dirty()` → `refresh_axis`).
    pub fn draw(
        &mut self,
        clear: crate::color::Color,
        items: &mut [ChartDrawItem<'_>],
    ) -> Result<()> {
        let prepared = self.inner.prepare(items)?;
        self.draw_prepared(clear, &prepared)
    }

    /// Draw renderer-owned charts, selecting resident data or an active exact
    /// streaming display independently for each chart.
    pub fn draw_registered(
        &mut self,
        clear: crate::color::Color,
        items: &mut [RegisteredChartDrawItem<'_>],
    ) -> Result<()> {
        let prepared = self.inner.prepare_registered(items)?;
        self.draw_prepared(clear, &prepared)
    }

    /// [`Self::draw_registered`] for monotonic charts (#271).
    pub fn draw_monotonic(
        &mut self,
        clear: crate::color::Color,
        items: &mut [crate::renderer::MonotonicChartDrawItem<'_>],
    ) -> Result<()> {
        let prepared = self.inner.prepare_monotonic(items)?;
        self.draw_prepared(clear, &prepared)
    }

    /// Draw one frame using a prepared data token owned by the host.
    ///
    /// This still records the surface composition pass (grid, annotation
    /// underlay, data, decorations, annotation overlay),
    /// but it does not rerun `prepare`: no transform uniform write, styled
    /// pipeline ensure, or arc-prefix compute unless the host prepared a new
    /// token first.
    pub fn draw_prepared(
        &mut self,
        clear: crate::color::Color,
        prepared: &PreparedFrame,
    ) -> Result<()> {
        self.inner.health.check()?;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(FiggyError::SurfaceAcquireFailed {
                    reason: "lost".into(),
                });
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                return Err(FiggyError::SurfaceAcquireFailed {
                    reason: "outdated".into(),
                });
            }
            other => {
                return Err(FiggyError::SurfaceAcquireFailed {
                    reason: format!("{other:?}"),
                });
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let (color_view, resolve_target, store) = match self.msaa_target.as_ref() {
            Some(msaa) => (&msaa.view, Some(&target), wgpu::StoreOp::Discard),
            None => (&target, None, wgpu::StoreOp::Store),
        };

        let mut encoder =
            self.inner
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy frame encoder"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy frame pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.inner.paint_prepared(
                &mut pass,
                (self.surface_config.width, self.surface_config.height),
                prepared,
            )?;
        }
        self.inner.queue().submit(std::iter::once(encoder.finish()));
        self.inner.end_gpu_frame();
        self.inner.queue().present(frame);
        // An error this frame's work sent to the uncaptured-error handler.
        self.inner.health.check()
    }

    /// Submit one frame and wait until that GPU work has finished executing.
    ///
    /// Dawn / native drivers often compile the used pipelines on this first
    /// real submit rather than at `createRenderPipeline`. Hosts should keep
    /// a loading state up until this future resolves. It does not remove a
    /// main-thread freeze; it only makes that freeze observable as "not
    /// ready yet" instead of a black canvas.
    pub async fn first_frame_ready(
        &mut self,
        clear: crate::color::Color,
        items: &mut [ChartDrawItem<'_>],
    ) -> Result<()> {
        self.draw(clear, items)?;
        #[cfg(target_arch = "wasm32")]
        crate::init::wait_browser_submitted_work(self.inner.queue())
            .await
            .map_err(|reason| FiggyError::GpuResourceAllocationFailed {
                resource: "figgy first frame completion",
                reason,
            })?;
        #[cfg(not(target_arch = "wasm32"))]
        self.inner.wait_submitted_work().await?;
        self.inner.health.check()
    }

    /// [`Self::first_frame_ready`], polling the device on this thread until
    /// the frame's work completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn first_frame_ready_blocking(
        &mut self,
        clear: crate::color::Color,
        items: &mut [ChartDrawItem<'_>],
    ) -> Result<()> {
        self.draw(clear, items)?;
        self.inner.wait_submitted_work_blocking()?;
        self.inner.health.check()
    }

    /// Alias for `Self::first_frame_ready_blocking`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn warm_up_blocking(
        &mut self,
        clear: crate::color::Color,
        items: &mut [ChartDrawItem<'_>],
    ) -> Result<()> {
        self.first_frame_ready_blocking(clear, items)
    }

    /// Alias for [`Self::first_frame_ready`].
    pub async fn warm_up(
        &mut self,
        clear: crate::color::Color,
        items: &mut [ChartDrawItem<'_>],
    ) -> Result<()> {
        self.first_frame_ready(clear, items).await
    }
}

impl Drop for WindowedRenderer<'_> {
    fn drop(&mut self) {
        // Drop has no caller to return an error to, so no path may reach it
        // with an unreported error: every call that records GPU work checks
        // the device health at its end and returns its own GPU error (#158).
        // What can still be seen here is a device lost after the last call (a
        // driver reset, a removed GPU), and on the web a WebGPU error the
        // browser delivered after the last call; a host that needs the latter
        // awaits `wait_submitted_work` before dropping. A lost device has no
        // work left to finish, and polling it would only fail, so the wait is
        // skipped then.
        if self.inner.health.is_lost() {
            return;
        }
        if self.inner.health.check().is_err() {
            // See above: only an error delivered after the last call; it must
            // not stop the wait below.
        }
        if self.inner.wait_idle().is_err() {
            // The device was lost during the wait (`gpu_poll` never panics):
            // nothing is left to wait for.
        }
    }
}

impl Renderer {
    /// `result`, or the GPU error the call that produced it raised: an `Ok`
    /// value is returned only when no GPU error is pending (#158). An `Err`
    /// is returned as it is; a GPU error pending beside it stays counted for
    /// the next check.
    pub(crate) fn reported<T>(&self, result: Result<T>) -> Result<T> {
        self.reported_as(result, std::convert::identity)
    }

    /// [`Self::reported`] for a call with its own error type.
    pub(crate) fn reported_as<T, E>(
        &self,
        result: std::result::Result<T, E>,
        convert: fn(FiggyError) -> E,
    ) -> std::result::Result<T, E> {
        let value = result?;
        self.health.check().map_err(convert)?;
        Ok(value)
    }

    /// Build base Cartesian resources and a surface from a window handle.
    /// The returned [`WindowedRenderer`] retains the instance, adapter and GPU
    /// handles, manages the surface and
    /// expects `draw` to be called once per frame.
    ///
    /// `target` can be anything convertible to `wgpu::SurfaceTarget`
    /// (`Arc<winit::Window>`, a raw-handle wrapper, a web canvas, …). The
    /// figgy crate does not depend on winit types itself. On wasm, drive this
    /// from the host event loop (e.g. `wasm_bindgen_futures::spawn_local`).
    ///
    /// In a browser this also awaits temporary JS render-pipeline warmup; those
    /// pipelines are discarded and do not populate the renderer's optional
    /// caches. Native construction has no JS warmup. Both leave resident optional
    /// caches lazy; use [`Self::prewarm_all`] or detached preparation explicitly.
    pub async fn for_window_async<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        let mut observer = |_| {};
        Self::for_window_async_observed(target, size, pool_capacity_bytes, &mut observer).await
    }

    /// Build a standalone renderer as in [`Self::for_window_async`], reporting
    /// timestamp-free startup stage boundaries to `observer`.
    // Keep the Arc-based RendererDevice API identical across targets. wgpu's
    // WebGPU handles are intentionally !Send/!Sync on the wasm main thread.
    pub async fn for_window_async_observed<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<WindowedRenderer<'w>> {
        Self::for_window_prepared_async(target, size, pool_capacity_bytes, false, observer).await
    }

    /// Limit temporary browser render warmup to Basic entries during window setup.
    ///
    /// Native initialization is the same as [`Self::for_window_async_observed`].
    /// This does not install the retained Basic preparation group. A host gating
    /// first use must compile and install Basic and its other required groups via
    /// [`Self::begin_preparation`], or call [`Self::prewarm_all`]. Normal frame
    /// preparation can also create missing pipelines lazily.
    pub async fn for_window_minimal_async_observed<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<WindowedRenderer<'w>> {
        Self::for_window_prepared_async(target, size, pool_capacity_bytes, true, observer).await
    }

    pub(super) async fn for_window_prepared_async<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        minimal: bool,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<WindowedRenderer<'w>> {
        let (instance, surface) = window_surface(target, observer)?;
        started(observer, "window", "adapter");
        let adapter = data_render::request_adapter_for_surface_async(&instance, &surface)
            .await
            .map_err(|_| FiggyError::AdapterUnavailable)?;
        finished(observer, "window", "adapter");
        started(observer, "window", "device");
        let (device, queue) = data_render::request_device_async(&adapter)
            .await
            .map_err(|e| FiggyError::DeviceCreationFailed {
                reason: format!("{e}"),
            })?;
        finished(observer, "window", "device");
        let target = window_target(&surface, &adapter, device, queue, size, observer)?;
        let inner = Renderer::try_new_with_sample_count_observed_async(
            target.gpu,
            target.surface_config.format,
            pool_capacity_bytes,
            target.sample_count,
            minimal,
            observer,
        )
        .await?;
        Ok(WindowedRenderer {
            inner,
            surface,
            surface_config: target.surface_config,
            msaa_target: target.msaa_target,
            _instance: instance,
            _adapter: adapter,
        })
    }

    /// [`Self::for_window_async`] on native, without an executor: wgpu
    /// completes the adapter and device requests when they are made, and a
    /// native device compiles each pipeline when it is created.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn for_window_blocking<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        let mut observer = |_| {};
        Self::for_window_blocking_observed(target, size, pool_capacity_bytes, &mut observer)
    }

    /// [`Self::for_window_async_observed`] on native, without an executor.
    /// The browser-only minimal warmup has no native counterpart, so
    /// [`Self::for_window_minimal_async_observed`] builds the same renderer.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn for_window_blocking_observed<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<WindowedRenderer<'w>> {
        let (instance, surface) = window_surface(target, observer)?;
        started(observer, "window", "adapter");
        let adapter = data_render::request_adapter_for_surface_blocking(&instance, &surface)?;
        finished(observer, "window", "adapter");
        started(observer, "window", "device");
        let (device, queue) = data_render::request_device_blocking(&adapter)?;
        finished(observer, "window", "device");
        let target = window_target(&surface, &adapter, device, queue, size, observer)?;
        let inner = Renderer::try_new_with_sample_count_observed(
            target.gpu,
            target.surface_config.format,
            pool_capacity_bytes,
            target.sample_count,
            observer,
        )?;
        Ok(WindowedRenderer {
            inner,
            surface,
            surface_config: target.surface_config,
            msaa_target: target.msaa_target,
            _instance: instance,
            _adapter: adapter,
        })
    }
}

/// A window's instance and surface: the first step of every windowed
/// constructor.
fn window_surface<'w>(
    target: impl Into<wgpu::SurfaceTarget<'w>>,
    observer: &mut dyn FnMut(InitEvent),
) -> Result<(wgpu::Instance, wgpu::Surface<'w>)> {
    let instance = observe_value(observer, "window", "instance", data_render::create_instance);
    // wgpu reports surface failures as `CreateSurfaceError`; there is no
    // device yet whose error scopes could capture anything else.
    let surface = observe_result(observer, "window", "surface", || {
        data_render::create_surface_for_window(&instance, target).map_err(|e| {
            FiggyError::SurfaceCreationFailed {
                reason: format!("{e}"),
            }
        })
    })?;
    Ok((instance, surface))
}

/// What a windowed renderer is built on once its device exists: the device
/// bundle, the configured surface and the frame's sample count and MSAA
/// target.
struct WindowTarget {
    gpu: RendererDevice,
    surface_config: wgpu::SurfaceConfiguration,
    sample_count: u32,
    msaa_target: Option<MsaaTarget>,
}

/// Bundle the device, configure the surface and probe the frame's MSAA.
fn window_target(
    surface: &wgpu::Surface<'_>,
    adapter: &wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: (u32, u32),
    observer: &mut dyn FnMut(InitEvent),
) -> Result<WindowTarget> {
    // The bundle registers the device's lost callback and error handler
    // before figgy uses the device, once for this device.
    let gpu = RendererDevice::new(device, queue)?;
    let device = gpu.device().clone();
    let surface_config = observe_result(observer, "window", "configure", || {
        data_render::try_configure_surface(surface, adapter, &device, size.0.max(1), size.1.max(1))
    })?;

    let caps = RendererDeviceCaps::from_device(&device);
    // Probing MSAA decides the sample count the renderer's pipelines are
    // compiled against, so it has to happen before the renderer exists.
    // The target is charged to the bundle's ledger, which the renderer
    // shares: one ledger per device bundle, this target included.
    let gpu_ledger = Arc::clone(gpu.ledger());
    let preferred_sample_count = preferred_msaa_sample_count(caps, surface_config.format);
    let (target_sample_count, msaa_target) = observe_result(
        observer,
        "window",
        "figgy frame msaa target",
        || match create_msaa_target(
            &device,
            &gpu_ledger,
            caps,
            "figgy frame msaa target",
            surface_config.width,
            surface_config.height,
            surface_config.format,
            preferred_sample_count,
        ) {
            Ok(target) => Ok((preferred_sample_count, target)),
            Err(_) if preferred_sample_count > 1 => Ok((1, None)),
            Err(e) => Err(e),
        },
    )?;

    Ok(WindowTarget {
        gpu,
        surface_config,
        sample_count: target_sample_count,
        msaa_target,
    })
}

impl<'w> std::ops::Deref for WindowedRenderer<'w> {
    type Target = Renderer;
    fn deref(&self) -> &Renderer {
        &self.inner
    }
}

impl<'w> std::ops::DerefMut for WindowedRenderer<'w> {
    fn deref_mut(&mut self) -> &mut Renderer {
        &mut self.inner
    }
}
