//! A renderer bound to a window surface: WindowedRenderer and the
//! for_window constructors.

use super::*;

/// Returned by `Renderer::for_window`. The caller never touches the surface,
/// surface config, encoder, or render pass — just calls `draw` per frame.
///
/// Immutable renderer access is available through `Deref<Target = Renderer>`.
/// Mutations use explicit forwarding methods so surface-sensitive operations
/// cannot bypass this owner.
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
    /// `result`, or the GPU error the call that produced it raised.
    ///
    /// Every call that records GPU work ends here (or with its own health
    /// check), so a GPU error is reported by the call that raised it and none
    /// is left for `Drop`, which has no caller to report to (#158).
    fn reported<T>(&self, result: Result<T>) -> Result<T> {
        self.inner.reported_as(result, std::convert::identity)
    }

    /// [`Self::reported`] for a call with its own error type.
    fn reported_as<T, E>(
        &self,
        result: std::result::Result<T, E>,
        convert: fn(FiggyError) -> E,
    ) -> std::result::Result<T, E> {
        self.inner.reported_as(result, convert)
    }

    /// Publish an owned preparation without retaining a mutable borrow while it compiles.
    pub fn install_preparation(&mut self, prepared: PreparedPipelines) -> Result<()> {
        let result = self.inner.install_preparation(prepared);
        self.reported(result)
    }

    /// Allocate a budget-checked style for this surface's renderer.
    pub fn create_style_for_series(&mut self, cfg: &SeriesConfig) -> Result<ChartStyle> {
        let result = self.inner.create_style_for_series(cfg);
        self.reported(result)
    }

    /// High-DPI variant of [`Self::create_style_for_series`].
    pub fn create_style_for_series_scaled(
        &mut self,
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<ChartStyle> {
        let result = self.inner.create_style_for_series_scaled(cfg, scale);
        self.reported(result)
    }

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

    pub fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    pub async fn ensure_errorbar_extent_engine(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_errorbar::GpuErrorbarError> {
        let result = self.inner.ensure_errorbar_extent_engine().await;
        self.reported_as(result, crate::gpu_errorbar::GpuErrorbarError::from_figgy)
    }

    pub async fn prewarm_all_observed(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<()> {
        let result = self.inner.prewarm_all_observed(observer).await;
        self.reported(result)
    }

    pub async fn prewarm_all(&mut self) -> Result<()> {
        let result = self.inner.prewarm_all().await;
        self.reported(result)
    }

    pub fn add_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        let result = self.inner.add_column(id, source);
        self.reported(result)
    }

    pub fn add_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        let result = self.inner.add_hilo_column(id, source);
        self.reported(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin_load_demo<F>(
        &mut self,
        chart_id: ChartId,
        demo_x: &dyn ColumnSource,
        demo_sin: &dyn ColumnSource,
        demo_t: &dyn ColumnSource,
        demo_rc: &dyn ColumnSource,
        prepare_state: F,
    ) -> Result<RendererLoadDemo<'_>>
    where
        F: FnOnce(&ColumnPool) -> Result<(Config, Vec<SeriesConfig>)>,
    {
        // The guard borrows the renderer, so the check uses a clone of the
        // health record; a failed check drops the guard, which rolls back.
        let health = self.inner.health.clone();
        let demo = self.inner.begin_load_demo(
            chart_id,
            demo_x,
            demo_sin,
            demo_t,
            demo_rc,
            prepare_state,
        )?;
        health.check()?;
        Ok(demo)
    }

    pub fn add_columns(&mut self, columns: &[(&str, &dyn ColumnSource)]) -> Result<()> {
        let result = self.inner.add_columns(columns);
        self.reported(result)
    }

    pub fn register_streamed_columns(&mut self, columns: Vec<crate::StreamColumn>) -> Result<()> {
        let result = self.inner.register_streamed_columns(columns);
        self.reported(result)
    }

    pub fn replace_streamed_columns(&mut self, columns: Vec<crate::StreamColumn>) -> Result<()> {
        let result = self.inner.replace_streamed_columns(columns);
        self.reported(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resident_admission_for_streamed_columns(
        &self,
        sources: &[crate::StreamSourceBinding<'_>],
        already_resident_working_set_bytes: u64,
        derived_buffer_sizes: &[u64],
        transition_headroom_bytes: u64,
        working_set_limit_bytes: u64,
    ) -> Result<ResidentAdmission> {
        let result = self.inner.resident_admission_for_streamed_columns(
            sources,
            already_resident_working_set_bytes,
            derived_buffer_sizes,
            transition_headroom_bytes,
            working_set_limit_bytes,
        );
        self.reported(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn promote_streamed_columns_to_resident(
        &mut self,
        sources: &[crate::StreamSourceBinding<'_>],
        already_resident_working_set_bytes: u64,
        derived_buffer_sizes: &[u64],
        transition_headroom_bytes: u64,
        working_set_limit_bytes: u64,
    ) -> Result<ResidentAdmission> {
        let result = self.inner.promote_streamed_columns_to_resident(
            sources,
            already_resident_working_set_bytes,
            derived_buffer_sizes,
            transition_headroom_bytes,
            working_set_limit_bytes,
        );
        self.reported(result)
    }

    pub fn demote_resident_columns_to_streamed(
        &mut self,
        changed_ids: &[&str],
        columns: Vec<crate::StreamColumn>,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<()> {
        let result = self
            .inner
            .demote_resident_columns_to_streamed(changed_ids, columns, sources);
        self.reported(result)
    }

    pub fn configure_streaming(&mut self, limits: crate::StreamingLimits) -> Result<()> {
        let result = self.inner.configure_streaming(limits);
        self.reported(result)
    }

    pub fn begin_streaming_chart(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        options: crate::StreamingChartOptions,
    ) -> Result<()> {
        let result = self.inner.begin_streaming_chart(chart, view, options);
        self.reported(result)
    }

    pub fn request_auto_streaming_chart(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        let result = self
            .inner
            .request_auto_streaming_chart(chart, view, options);
        self.reported(result)
    }

    pub fn request_auto_streaming_chart_with_config(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        render_config: Config,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        let result = self.inner.request_auto_streaming_chart_with_config(
            chart,
            view,
            render_config,
            options,
        );
        self.reported(result)
    }

    pub fn request_resident_stream_handoff_with_config(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        render_config: Config,
        columns: Vec<crate::StreamColumn>,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        let result = self.inner.request_resident_stream_handoff_with_config(
            chart,
            view,
            render_config,
            columns,
            options,
        );
        self.reported(result)
    }

    pub fn request_auto_streaming_chart_with_display_scale(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        render_config: Config,
        display_scale: f32,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        let result = self.inner.request_auto_streaming_chart_with_display_scale(
            chart,
            view,
            render_config,
            display_scale,
            options,
        );
        self.reported(result)
    }

    pub fn request_resident_stream_handoff_with_display_scale(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        render_config: Config,
        display_scale: f32,
        columns: Vec<crate::StreamColumn>,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        let result = self
            .inner
            .request_resident_stream_handoff_with_display_scale(
                chart,
                view,
                render_config,
                display_scale,
                columns,
                options,
            );
        self.reported(result)
    }

    pub fn auto_stream_chart_step(
        &mut self,
        chart: ChartId,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<crate::AutoStreamingProgress> {
        let result = self.inner.auto_stream_chart_step(chart, sources);
        self.reported(result)
    }

    pub fn auto_stream_chart_request_ranges(
        &mut self,
        chart: ChartId,
    ) -> Result<crate::AutoStreamingRangeRequest> {
        let result = self.inner.auto_stream_chart_request_ranges(chart);
        self.reported(result)
    }

    pub fn auto_stream_chart_submit_ranges(
        &mut self,
        chart: ChartId,
        sources: &[crate::StreamRangeSourceBinding<'_>],
    ) -> Result<crate::AutoStreamingProgress> {
        let result = self.inner.auto_stream_chart_submit_ranges(chart, sources);
        self.reported(result)
    }

    pub fn interrupt_render(&mut self, chart: ChartId) -> Result<crate::RenderInterruptStatus> {
        let result = self.inner.interrupt_render(chart);
        self.reported(result)
    }

    pub fn stream_chart_step(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<crate::StreamingProgress> {
        let result = self.inner.stream_chart_step(chart, view, sources);
        self.reported(result)
    }

    pub fn cancel_streaming_chart(&mut self, chart: ChartId) -> Result<()> {
        let result = self.inner.cancel_streaming_chart(chart);
        self.reported(result)
    }

    pub fn set_stream_chunk_budget(&mut self, chart: ChartId, primitives: u64) -> Result<()> {
        let result = self.inner.set_stream_chunk_budget(chart, primitives);
        self.reported(result)
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn cancel_streaming_chart_and_wait(&mut self, chart: ChartId) -> Result<()> {
        let result = self.inner.cancel_streaming_chart_and_wait(chart).await;
        self.reported(result)
    }

    pub fn begin_upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        // See `begin_load_demo`: the upload is done; check before the guard
        // is handed out.
        let health = self.inner.health.clone();
        let upsert = self.inner.begin_upsert_column(id, source)?;
        health.check()?;
        Ok(upsert)
    }

    pub fn begin_upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<RendererColumnUpsert<'_>> {
        let health = self.inner.health.clone();
        let upsert = self.inner.begin_upsert_hilo_column(id, source)?;
        health.check()?;
        Ok(upsert)
    }

    pub fn upsert_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        let result = self.inner.upsert_column(id, source);
        self.reported(result)
    }

    pub fn upsert_hilo_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn HiLoColumnSource,
    ) -> Result<ColumnHandle> {
        let result = self.inner.upsert_hilo_column(id, source);
        self.reported(result)
    }

    pub fn remove_column(&mut self, id: &str) -> Result<bool> {
        let result = self.inner.remove_column(id);
        self.reported(result)
    }

    pub fn upsert_record(
        &mut self,
        id: impl Into<RecordId>,
        source: &RecordSource<'_>,
    ) -> Result<RecordId> {
        let result = self.inner.upsert_record(id, source);
        self.reported(result)
    }

    pub fn upsert_ragged(
        &mut self,
        id: impl Into<RaggedId>,
        source: &RaggedSource<'_>,
    ) -> Result<RaggedId> {
        let result = self.inner.upsert_ragged(id, source);
        self.reported(result)
    }

    pub fn remove_record(&mut self, id: &str) -> Result<bool> {
        let result = self.inner.remove_record(id);
        self.reported(result)
    }

    pub fn remove_ragged(&mut self, id: &str) -> Result<bool> {
        let result = self.inner.remove_ragged(id);
        self.reported(result)
    }

    pub fn remove_column_with_chart_config(
        &mut self,
        id: &str,
        chart_id: ChartId,
        next_config: Config,
    ) -> Result<bool> {
        let result = self
            .inner
            .remove_column_with_chart_config(id, chart_id, next_config);
        self.reported(result)
    }

    pub fn defragment(&mut self) -> Result<bool> {
        let result = self.inner.defragment();
        self.reported(result)
    }

    /// Set the allocation policy described by [`Renderer::set_memory_budget`],
    /// including what the budget does and does
    /// not govern and what a returned total means.
    ///
    /// The window's MSAA target is charged to the same ledger as the
    /// renderer's tracked resources, and the immutable [`Renderer::gpu_memory_usage`]
    /// reached through `Deref` reports it. This does not account for all backend
    /// surface/swapchain resources or enforce a device-wide ceiling.
    #[must_use = "a returned total means the new ceiling is already exceeded"]
    pub fn set_memory_budget(&mut self, budget: Option<u64>) -> Option<u64> {
        self.inner.set_memory_budget(budget)
    }

    pub fn set_auto_resident_working_set_limit(&mut self, limit: Option<u64>) -> Option<u64> {
        self.inner.set_auto_resident_working_set_limit(limit)
    }

    /// Submission boundary — see [`Renderer::end_gpu_frame`]. `draw` calls it
    /// after queue submission; hosts that record their own command buffers call
    /// it after submitting every panel in the frame.
    pub fn end_gpu_frame(&mut self) {
        self.inner.end_gpu_frame();
    }

    pub fn process_pending_maintenance(&mut self) -> Result<bool> {
        let result = self.inner.process_pending_maintenance();
        self.reported(result)
    }

    /// See [`Renderer::release_unused_pool_memory`].
    pub async fn release_unused_pool_memory(&mut self, minimum_capacity: u64) -> Result<()> {
        let result = self
            .inner
            .release_unused_pool_memory(minimum_capacity)
            .await;
        self.reported(result)
    }

    pub fn set_defrag_policy(&mut self, policy: DefragPolicy) {
        self.inner.set_defrag_policy(policy);
    }

    /// See [`Renderer::set_pool_growth_policy`].
    pub fn set_pool_growth_policy(&mut self, policy: data_render::GrowthPolicy) {
        self.inner.set_pool_growth_policy(policy);
    }

    pub fn enable_gpu_picking(&mut self) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.inner.enable_gpu_picking();
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub fn enable_gpu_picking_observed(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.inner.enable_gpu_picking_observed(observer);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub async fn enable_gpu_picking_async(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.inner.enable_gpu_picking_async().await;
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub async fn enable_gpu_picking_observed_async(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.inner.enable_gpu_picking_observed_async(observer).await;
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub fn prepare_gpu_picking_for_chart(
        &mut self,
        id: ChartId,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.inner.prepare_gpu_picking_for_chart(id);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
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
        let result = self.inner.pick_chart(
            id,
            GpuPickRequest {
                canvas_position_px,
                display_panel_px,
                display_scale,
                max_distance_px,
            },
        );
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
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
        let result = self.inner.pick_chart_data(
            id,
            GpuPickRequest {
                canvas_position_px,
                display_panel_px,
                display_scale,
                max_distance_px,
            },
        );
        self.reported(result)
    }

    pub fn register_chart(&mut self, config: Config, series: Vec<SeriesConfig>) -> Result<ChartId> {
        let result = self.inner.register_chart(config, series);
        self.reported(result)
    }

    pub fn remove_chart(&mut self, id: ChartId) -> Result<()> {
        let result = self.inner.remove_chart(id);
        self.reported(result)
    }

    pub fn set_chart_config(&mut self, id: ChartId, config: Config) -> Result<()> {
        let result = self.inner.set_chart_config(id, config);
        self.reported(result)
    }

    pub fn set_chart_series(&mut self, id: ChartId, series: Vec<SeriesConfig>) -> Result<()> {
        let result = self.inner.set_chart_series(id, series);
        self.reported(result)
    }

    pub fn set_chart_state(
        &mut self,
        id: ChartId,
        config: Config,
        series: Vec<SeriesConfig>,
    ) -> Result<()> {
        let result = self.inner.set_chart_state(id, config, series);
        self.reported(result)
    }

    pub fn set_chart_view_state(&mut self, id: ChartId, view: ChartViewState) -> Result<()> {
        let result = self.inner.set_chart_view_state(id, view);
        self.reported(result)
    }

    pub fn set_chart_selection(&mut self, id: ChartId, selected: Option<HitId>) -> Result<()> {
        let result = self.inner.set_chart_selection(id, selected);
        self.reported(result)
    }

    pub fn set_active_layer(
        &mut self,
        id: ChartId,
        layer: crate::LayerRef,
    ) -> Result<crate::LayerRef> {
        let result = self.inner.set_active_layer(id, layer);
        self.reported(result)
    }

    pub fn active_layer(&self, id: ChartId) -> Result<crate::LayerRef> {
        let result = self.inner.active_layer(id);
        self.reported(result)
    }

    pub fn sync_external_invalidations(&mut self) -> Result<bool> {
        let result = self.inner.sync_external_invalidations();
        self.reported(result)
    }

    pub fn request_stream_auto_fit(&mut self, id: ChartId, padding: f64) -> Result<()> {
        let result = self.inner.request_stream_auto_fit(id, padding);
        self.reported(result)
    }

    pub fn commit_auto_fit_all_if_current(
        &mut self,
        token: &FitCommitToken,
        x_extent: &crate::chart::FitExtent,
        y_extent: &crate::chart::FitExtent,
        padding: f64,
    ) -> Result<()> {
        let result = self
            .inner
            .commit_auto_fit_all_if_current(token, x_extent, y_extent, padding);
        self.reported(result)
    }

    pub fn refresh_axis(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        let result = self.inner.refresh_axis(view, chart, panel_rect);
        self.reported(result)
    }

    pub fn refresh_axis_with_selection(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<&str>,
    ) -> Result<()> {
        let result = self
            .inner
            .refresh_axis_with_selection(view, chart, panel_rect, selected);
        self.reported(result)
    }

    /// [`Renderer::selection_box`].
    pub fn selection_box(
        &self,
        chart: &Chart,
        element_id: &str,
    ) -> Option<crate::select::SelectionBox> {
        self.inner.selection_box(chart, element_id)
    }

    pub fn refresh_annotations(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<usize>,
    ) -> Result<()> {
        let result = self
            .inner
            .refresh_annotations(view, chart, panel_rect, selected);
        self.reported(result)
    }

    pub fn update_transform(&mut self, view: &ChartView, chart: &Chart) -> Result<()> {
        let result = self.inner.update_transform(view, chart);
        self.reported(result)
    }

    pub fn prepare(&mut self, items: &[ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        let result = self.inner.prepare(items);
        self.reported(result)
    }

    pub fn prepare_registered(
        &mut self,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        let result = self.inner.prepare_registered(items);
        self.reported(result)
    }

    pub async fn export_panel_rgba_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        let result = self
            .inner
            .export_panel_rgba_async(chart, series, scale)
            .await;
        self.reported(result)
    }

    pub async fn export_panel_rgba_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        let result = self
            .inner
            .export_panel_rgba_with_clear_async(chart, series, scale, clear)
            .await;
        self.reported(result)
    }

    pub async fn export_panel_png_bytes_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let result = self
            .inner
            .export_panel_png_bytes_async(chart, series, scale)
            .await;
        self.reported(result)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_rgba(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        let result = self.inner.export_panel_rgba(chart, series, scale);
        self.reported(result)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_png_bytes(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let result = self.inner.export_panel_png_bytes(chart, series, scale);
        self.reported(result)
    }

    pub async fn export_panel_png_bytes_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<Vec<u8>> {
        let result = self
            .inner
            .export_panel_png_bytes_with_clear_async(chart, series, scale, clear)
            .await;
        self.reported(result)
    }

    pub async fn export_document_rgba_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        let result = self
            .inner
            .export_document_rgba_async(chart, series, scale)
            .await;
        self.reported(result)
    }

    pub async fn export_document_rgba_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        let result = self
            .inner
            .export_document_rgba_with_clear_async(chart, series, scale, clear)
            .await;
        self.reported(result)
    }

    pub async fn export_document_png_bytes_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let result = self
            .inner
            .export_document_png_bytes_async(chart, series, scale)
            .await;
        self.reported(result)
    }

    pub async fn export_document_png_bytes_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<Vec<u8>> {
        let result = self
            .inner
            .export_document_png_bytes_with_clear_async(chart, series, scale, clear)
            .await;
        self.reported(result)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_document_rgba(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        let result = self.inner.export_document_rgba(chart, series, scale);
        self.reported(result)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_document_png_bytes(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let result = self.inner.export_document_png_bytes(chart, series, scale);
        self.reported(result)
    }

    pub async fn export_document_svg_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<String> {
        let result = self
            .inner
            .export_document_svg_async(chart, series, scale)
            .await;
        self.reported(result)
    }

    pub async fn export_document_svg_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<String> {
        let result = self
            .inner
            .export_document_svg_with_clear_async(chart, series, scale, clear)
            .await;
        self.reported(result)
    }

    pub async fn export_document_pdf_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let result = self
            .inner
            .export_document_pdf_async(chart, series, scale)
            .await;
        self.reported(result)
    }

    pub async fn export_document_pdf_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<Vec<u8>> {
        let result = self
            .inner
            .export_document_pdf_with_clear_async(chart, series, scale, clear)
            .await;
        self.reported(result)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_document_svg(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<String> {
        let result = self.inner.export_document_svg(chart, series, scale);
        self.reported(result)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_document_pdf(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let result = self.inner.export_document_pdf(chart, series, scale);
        self.reported(result)
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
    pub fn draw(&mut self, clear: crate::color::Color, items: &[ChartDrawItem<'_>]) -> Result<()> {
        let prepared = self.inner.prepare(items)?;
        self.draw_prepared(clear, &prepared)
    }

    /// Draw renderer-owned charts, selecting resident data or an active exact
    /// streaming display independently for each chart.
    pub fn draw_registered(
        &mut self,
        clear: crate::color::Color,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> Result<()> {
        let prepared = self.inner.prepare_registered(items)?;
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
        items: &[ChartDrawItem<'_>],
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

    /// Alias for [`Self::first_frame_ready`].
    pub async fn warm_up(
        &mut self,
        clear: crate::color::Color,
        items: &[ChartDrawItem<'_>],
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
        // The bundle registers the device's lost callback and error handler
        // before figgy uses the device, once for this device.
        let gpu = RendererDevice::new(device, queue);
        let device = gpu.device().clone();
        let surface_config = observe_result(observer, "window", "configure", || {
            data_render::try_configure_surface(
                &surface,
                &adapter,
                &device,
                size.0.max(1),
                size.1.max(1),
            )
        })?;

        let caps = RendererDeviceCaps::from_device(&device);
        // Probing MSAA decides the sample count the renderer's pipelines are
        // compiled against, so it has to happen before the renderer exists.
        // The ledger is therefore created here and adopted by the renderer:
        // one ledger per device, this target included.
        // host-alloc: W1-a
        let gpu_ledger = Arc::new(GpuLedger::new());
        let preferred_sample_count = preferred_msaa_sample_count(caps, surface_config.format);
        let (target_sample_count, msaa_target) =
            observe_result(observer, "window", "figgy frame msaa target", || {
                match create_msaa_target(
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
                }
            })?;

        let inner = Renderer::try_new_with_sample_count_observed_async(
            gpu,
            surface_config.format,
            pool_capacity_bytes,
            target_sample_count,
            Arc::clone(&gpu_ledger),
            minimal,
            observer,
        )
        .await?;

        Ok(WindowedRenderer {
            inner,
            surface,
            surface_config,
            msaa_target,
            _instance: instance,
            _adapter: adapter,
        })
    }

    /// Blocking convenience wrapper around [`Self::for_window_async`]. Native
    /// only — on wasm, blocking the single thread would deadlock.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn for_window<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        pollster::block_on(Self::for_window_async(target, size, pool_capacity_bytes))
    }
}

impl<'w> std::ops::Deref for WindowedRenderer<'w> {
    type Target = Renderer;
    fn deref(&self) -> &Renderer {
        &self.inner
    }
}
