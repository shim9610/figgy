//! The renderer's constructors.

use super::*;

impl Renderer {
    /// Initialize base Cartesian resources against the given device/queue pair.
    ///
    /// Creates the column pool, shared layouts/sampler, shader modules, unit
    /// quad and axis pipeline. Optional data/style and compute pipelines are
    /// lazy. Use [`Self::prewarm_all`] or detached preparation to install them
    /// before first use; construction does not prepare chart-specific data.
    ///
    /// `pool_capacity_bytes` is the requested total size of the GPU column
    /// pool (sum of all chart data). It is clamped to what the device can
    /// bind — the arc-scan compute binds the whole pool as one storage
    /// binding, so a request above `max_storage_buffer_binding_size` (or
    /// `max_buffer_size`) is reduced instead of panicking later; read the
    /// granted size back via [`Self::pool`]`().capacity()`. To get a larger
    /// pool, raise those limits when requesting the wgpu device (the defaults
    /// are only 128 MiB / 256 MiB). This initial slab is **not** subject to
    /// [`Self::set_memory_budget`] — the budget governs growth, and this size is
    /// the host's own declaration. `surface_format` is the final render
    /// target color format; the eager axis pipeline and any later lazy
    /// graphics pipelines are compiled against it.
    pub fn try_new(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
    ) -> Result<Self> {
        Self::try_new_with_sample_count(gpu, surface_format, pool_capacity_bytes, 1)
    }

    /// Initialize the same base resources as [`Self::try_new`] and report
    /// timestamp-free startup stage boundaries to `observer`.
    pub fn try_new_observed(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self> {
        Self::try_new_with_sample_count_observed(
            gpu,
            surface_format,
            pool_capacity_bytes,
            1,
            observer,
        )
    }

    pub(in crate::renderer) fn try_new_with_sample_count(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        target_sample_count: u32,
    ) -> Result<Self> {
        let mut observer = |_| {};
        Self::try_new_with_sample_count_observed(
            gpu,
            surface_format,
            pool_capacity_bytes,
            target_sample_count,
            &mut observer,
        )
    }

    pub(in crate::renderer) fn try_new_with_sample_count_observed(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        target_sample_count: u32,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self> {
        let RendererDevice {
            device,
            queue,
            health,
            ledger: gpu_ledger,
        } = gpu;
        started(observer, "renderer", "capabilities");
        let caps = RendererDeviceCaps::from_device(&device);
        // Downlevel (GL-class) adapters report compute limits below what the
        // dashed-line arc scan needs; creating its pipelines there is a
        // validation panic. Refuse early with a real error instead.
        if caps.max_compute_invocations_per_workgroup < data_render::line_arc::WG
            || caps.max_compute_workgroups_per_dimension == 0
        {
            return Err(FiggyError::GpuResourceLimit {
                resource: "compute workgroup (figgy requires WebGPU-class compute; \
                           GL-downlevel adapters are not supported)",
                requested: u64::from(data_render::line_arc::WG),
                limit: u64::from(caps.max_compute_invocations_per_workgroup),
            });
        }
        validate_target_sample_count(caps, surface_format, target_sample_count)?;

        // Clamp the pool to what the arc-scan / star-pass storage bindings
        // can address — graceful fallback instead of a first-dashed-line
        // panic. The caller can read the granted size back via `pool()`.
        let pool_capacity = effective_pool_capacity(pool_capacity_bytes, caps);
        if pool_capacity < pool_capacity_bytes {
            eprintln!(
                "[figgy] pool capacity {pool_capacity_bytes} B exceeds device limits \
                 (max_storage_buffer_binding_size {}, max_buffer_size {}); \
                 clamped to {pool_capacity} B",
                caps.max_storage_buffer_binding_size, caps.max_buffer_size
            );
        }
        finished(observer, "renderer", "capabilities");
        let pool = observe_result(observer, "renderer", "figgy column pool", || {
            ColumnPool::new(pool_alloc_ctx(&device, &queue, &gpu_ledger), pool_capacity)
        })?;

        let texture_bgl = data_render::create_texture_bind_group_layout(&device);
        let transform_bgl = data_render::create_scatter_transform_bind_group_layout(&device);
        let style_bgl = data_render::create_style_bind_group_layout(&device);
        let per_point_style_map_bgl =
            data_render::create_per_point_style_map_bind_group_layout(&device);
        let point_value_map_bgl = data_render::create_point_value_map_bind_group_layout(&device);
        let (neutral_point_value_map_buffers, neutral_point_value_map_charge) =
            data_render::create_point_value_map_buffers(
                &device,
                &gpu_ledger,
                &data_render::PointValueMapParamsGpu::NEUTRAL,
                &[],
            )?;
        let neutral_point_value_map_bg = data_render::bind_point_value_map(
            &device,
            &point_value_map_bgl,
            &neutral_point_value_map_buffers,
        );
        let view3d_value_map_bgl =
            data_render::surface_mesh::create_view3d_value_map_bind_group_layout(&device);
        let data_selection_bgl = data_render::create_data_selection_bind_group_layout(&device);
        let star_data_bgl = data_render::create_star_data_bind_group_layout(&device);
        let field_bgl = data_render::create_field_data_bind_group_layout(&device);
        let surface_bgl = data_render::surface_mesh::create_surface_data_bind_group_layout(&device);
        let region_bgl =
            data_render::ternary_region::create_ternary_region_bind_group_layout(&device);
        let vector_bgl = data_render::vector::create_vector_data_bind_group_layout(&device);
        let category_bar_data_bgl =
            data_render::create_category_bar_data_bind_group_layout(&device);
        let parallel_bgl = data_render::create_parallel_data_bind_group_layout(&device);
        let value_label_bgl = data_render::create_value_label_bind_group_layout(&device);
        let field_cell_tables = data_render::create_field_cell_tables(&device, &gpu_ledger)?;

        let sampler = data_render::create_linear_sampler(&device);
        let quad_vb = TrackedBuffer::new(
            &gpu_ledger,
            GpuResourceKind::Uniform,
            data_render::create_unit_centered_quad_vertex_buffer(&device),
        );

        // Axis compiles here. Line/scatter/errorbar, mapped, pick-ring,
        // styled, and arc-scan pipelines compile on first prepare that needs
        // them. The errorbar extent engine requires explicit async preparation.
        let pipelines = create_target_pipelines_observed(
            &device,
            &texture_bgl,
            &transform_bgl,
            &style_bgl,
            &per_point_style_map_bgl,
            surface_format,
            target_sample_count,
            observer,
        );
        let renderer_identity = observe_result(observer, "renderer", "identity", || {
            issue_renderer_identity()
        })?;

        Ok(Self {
            device,
            queue,
            health,
            caps,
            pool,
            chart_states: HashMap::new(),
            streaming_sources: HashMap::new(),
            stream_revision_lineage: HashMap::new(),
            next_stream_source_fit_epoch: 0,
            stream_runtime: None,
            chart_order: Vec::new(),
            next_chart_id: 0,
            visual_revision: RenderRevision::initial(renderer_identity),
            renderer_identity,
            observed_font_generation: crate::text_render::font_generation(),
            pending_defrag: false,
            picker: RendererPicker::disabled(),
            errorbar_extent_engine: None,
            target_sample_count,
            target_pipeline_generation: 1,
            texture_bgl,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            point_value_map_bgl,
            neutral_point_value_map_bg,
            neutral_point_value_map_buffers,
            _neutral_point_value_map_charge: neutral_point_value_map_charge,
            view3d_value_map_bgl,
            data_selection_bgl,
            star_data_bgl,
            field_bgl,
            surface_bgl,
            region_bgl,
            vector_bgl,
            category_bar_data_bgl,
            parallel_bgl,
            value_label_bgl,
            field_cell_tables,
            pipelines,
            sampler,
            quad_vb,
            arc_cache: HashMap::new(),
            resident_displays: Vec::new(),
            resident_display_serial: 0,
            field_cache: HashMap::new(),
            value_map_cache: HashMap::new(),
            vector_cache: HashMap::new(),
            contour_label_pipelines: None,
            contour_cache: HashMap::new(),
            arc_pipelines: None,
            #[cfg(test)]
            arc_chunk_override: None,
            space_bg: None,
            surface_format,
            gpu_ledger,
            auto_resident_working_set_limit: None,
        })
    }

    pub(crate) async fn try_new_with_sample_count_observed_async(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        target_sample_count: u32,
        minimal: bool,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self> {
        let RendererDevice {
            device,
            queue,
            health,
            ledger: gpu_ledger,
        } = gpu;
        started(observer, "renderer", "capabilities");
        let caps = RendererDeviceCaps::from_device(&device);
        if caps.max_compute_invocations_per_workgroup < data_render::line_arc::WG
            || caps.max_compute_workgroups_per_dimension == 0
        {
            return Err(FiggyError::GpuResourceLimit {
                resource: "compute workgroup (figgy requires WebGPU-class compute; \
                           GL-downlevel adapters are not supported)",
                requested: u64::from(data_render::line_arc::WG),
                limit: u64::from(caps.max_compute_invocations_per_workgroup),
            });
        }
        validate_target_sample_count(caps, surface_format, target_sample_count)?;

        let pool_capacity = effective_pool_capacity(pool_capacity_bytes, caps);
        if pool_capacity < pool_capacity_bytes {
            eprintln!(
                "[figgy] pool capacity {pool_capacity_bytes} B exceeds device limits \
                 (max_storage_buffer_binding_size {}, max_buffer_size {}); \
                 clamped to {pool_capacity} B",
                caps.max_storage_buffer_binding_size, caps.max_buffer_size
            );
        }
        finished(observer, "renderer", "capabilities");
        crate::init::yield_init_frame().await;
        let pool = observe_result_async(observer, "renderer", "figgy column pool", || {
            ColumnPool::new(pool_alloc_ctx(&device, &queue, &gpu_ledger), pool_capacity)
        })
        .await?;

        let texture_bgl = data_render::create_texture_bind_group_layout(&device);
        let transform_bgl = data_render::create_scatter_transform_bind_group_layout(&device);
        let style_bgl = data_render::create_style_bind_group_layout(&device);
        let per_point_style_map_bgl =
            data_render::create_per_point_style_map_bind_group_layout(&device);
        let point_value_map_bgl = data_render::create_point_value_map_bind_group_layout(&device);
        let (neutral_point_value_map_buffers, neutral_point_value_map_charge) =
            data_render::create_point_value_map_buffers(
                &device,
                &gpu_ledger,
                &data_render::PointValueMapParamsGpu::NEUTRAL,
                &[],
            )?;
        let neutral_point_value_map_bg = data_render::bind_point_value_map(
            &device,
            &point_value_map_bgl,
            &neutral_point_value_map_buffers,
        );
        let view3d_value_map_bgl =
            data_render::surface_mesh::create_view3d_value_map_bind_group_layout(&device);
        let data_selection_bgl = data_render::create_data_selection_bind_group_layout(&device);
        let star_data_bgl = data_render::create_star_data_bind_group_layout(&device);
        let field_bgl = data_render::create_field_data_bind_group_layout(&device);
        let surface_bgl = data_render::surface_mesh::create_surface_data_bind_group_layout(&device);
        let region_bgl =
            data_render::ternary_region::create_ternary_region_bind_group_layout(&device);
        let vector_bgl = data_render::vector::create_vector_data_bind_group_layout(&device);
        let category_bar_data_bgl =
            data_render::create_category_bar_data_bind_group_layout(&device);
        let parallel_bgl = data_render::create_parallel_data_bind_group_layout(&device);
        let value_label_bgl = data_render::create_value_label_bind_group_layout(&device);
        let field_cell_tables = data_render::create_field_cell_tables(&device, &gpu_ledger)?;

        let sampler = data_render::create_linear_sampler(&device);
        let quad_vb = TrackedBuffer::new(
            &gpu_ledger,
            GpuResourceKind::Uniform,
            data_render::create_unit_centered_quad_vertex_buffer(&device),
        );

        let pipelines = create_target_pipelines_observed_async(
            &device,
            &texture_bgl,
            &transform_bgl,
            &style_bgl,
            &per_point_style_map_bgl,
            surface_format,
            target_sample_count,
            minimal,
            observer,
        )
        .await?;
        let renderer_identity = observe_result_async(observer, "renderer", "identity", || {
            issue_renderer_identity()
        })
        .await?;

        Ok(Self {
            device,
            queue,
            health,
            caps,
            pool,
            chart_states: HashMap::new(),
            streaming_sources: HashMap::new(),
            stream_revision_lineage: HashMap::new(),
            next_stream_source_fit_epoch: 0,
            stream_runtime: None,
            chart_order: Vec::new(),
            next_chart_id: 0,
            visual_revision: RenderRevision::initial(renderer_identity),
            renderer_identity,
            observed_font_generation: crate::text_render::font_generation(),
            pending_defrag: false,
            picker: RendererPicker::disabled(),
            errorbar_extent_engine: None,
            target_sample_count,
            target_pipeline_generation: 1,
            texture_bgl,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            point_value_map_bgl,
            neutral_point_value_map_bg,
            neutral_point_value_map_buffers,
            _neutral_point_value_map_charge: neutral_point_value_map_charge,
            view3d_value_map_bgl,
            data_selection_bgl,
            star_data_bgl,
            field_bgl,
            surface_bgl,
            region_bgl,
            vector_bgl,
            category_bar_data_bgl,
            parallel_bgl,
            value_label_bgl,
            field_cell_tables,
            pipelines,
            sampler,
            quad_vb,
            arc_cache: HashMap::new(),
            resident_displays: Vec::new(),
            resident_display_serial: 0,
            field_cache: HashMap::new(),
            value_map_cache: HashMap::new(),
            vector_cache: HashMap::new(),
            contour_label_pipelines: None,
            contour_cache: HashMap::new(),
            arc_pipelines: None,
            #[cfg(test)]
            arc_chunk_override: None,
            space_bg: None,
            surface_format,
            gpu_ledger,
            auto_resident_working_set_limit: None,
        })
    }
}
