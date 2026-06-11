//! Public-facing wgpu renderer.
//!
//! `Renderer::try_new` builds every figgy GPU resource (pool, pipelines,
//! bind-group layouts, sampler, quad VB) in one call so users don't have to
//! wire `data_render::create_*_pipeline` / `ColumnPool::new` manually.
//!
//! Ownership: holds `Arc<wgpu::Device>` and `Arc<wgpu::Queue>` so the same
//! device can be shared with the host (egui_wgpu, iced_wgpu, etc.) without
//! lifetime gymnastics.
//!
//! `try_new` returns `Result<_, FiggyError>` so unsupported target formats and
//! device resource limits are reported before wgpu validation can abort.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::axis_render;
use crate::chart::Chart;
use crate::color::Color;
use crate::config::Config;
use crate::line::LineStylePreset;
use crate::data::ColumnSource;
use crate::data_config::{
    DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterStyleConfig,
    ErrorRef, SeriesConfig,
};
use crate::data_render::{
    self, AxisLayer, ColumnErrorBarDraw, ColumnHandle, ColumnId, ColumnLineLayer,
    ColumnPool, ColumnScatterLayer, DefragPolicy, PrimitiveStyle,
};
use crate::error::{FiggyError, Result};
use crate::layout::Rect;

/// A wgpu device/queue pair used by figgy.
///
/// wgpu does not expose a public parent-device id on `Queue`, so figgy cannot
/// prove at runtime that arbitrary handles came from the same logical device.
/// Passing them as one value keeps the pair together throughout the renderer
/// API and avoids accepting loose device/queue arguments at every call site.
#[derive(Clone)]
pub struct RendererDevice {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl RendererDevice {
    /// Bundle a wgpu device and queue that came from the same `request_device`
    /// result or host render state.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        Self { device, queue }
    }

    pub fn device(&self) -> &Arc<wgpu::Device> { &self.device }
    pub fn queue(&self) -> &Arc<wgpu::Queue> { &self.queue }
}

#[derive(Clone, Copy, Debug)]
struct RendererDeviceCaps {
    features: wgpu::Features,
    max_texture_dimension_2d: u32,
    max_buffer_size: u64,
}

impl RendererDeviceCaps {
    fn from_device(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        Self {
            features: device.features(),
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_buffer_size: limits.max_buffer_size,
        }
    }
}

/// Facade bundling every figgy GPU resource.
pub struct Renderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    caps: RendererDeviceCaps,
    pool: ColumnPool,

    // Bind group layouts (exposed so callers can build per-panel bind groups).
    texture_bgl: wgpu::BindGroupLayout,
    transform_bgl: wgpu::BindGroupLayout,
    style_bgl: wgpu::BindGroupLayout,

    // Pipelines.
    axis_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    scatter_pipeline: wgpu::RenderPipeline,
    errorbar_pipeline: wgpu::RenderPipeline,

    // Shared resources.
    sampler: wgpu::Sampler,
    quad_vb: wgpu::Buffer,

    /// Per-series GPU arc-scan state for dashed lines, keyed by series id.
    /// The prefix is re-dispatched on every draw that uses it (it depends on
    /// the data→pixel transform); buffers/bind groups are reused while the
    /// series layout (length, column offsets, pool generation) is stable.
    /// Interior mutability because `paint` takes `&self`.
    arc_cache: Mutex<HashMap<String, data_render::line_arc::ArcScratch>>,
    arc_pipelines: data_render::line_arc::ArcScanPipelines,

    surface_format: wgpu::TextureFormat,
}

fn validate_target_format(
    caps: RendererDeviceCaps,
    format: wgpu::TextureFormat,
) -> Result<()> {
    if !caps.features.contains(format.required_features()) {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format requires device features that were not enabled".into(),
        });
    }
    let features = format.guaranteed_format_features(caps.features);
    if !features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format is not guaranteed to support RENDER_ATTACHMENT".into(),
        });
    }
    if !features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "figgy pipelines require alpha blending into the target".into(),
        });
    }
    Ok(())
}

fn create_target_pipelines(
    device: &wgpu::Device,
    texture_bgl: &wgpu::BindGroupLayout,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
) {
    let axis_pipeline = data_render::create_fullscreen_textured_pipeline(
        device, texture_bgl, surface_format,
    );
    let line_pipeline = data_render::create_line_columnar_pipeline(
        device, transform_bgl, style_bgl, surface_format,
    );
    let scatter_pipeline = data_render::create_scatter_columnar_pipeline(
        device, transform_bgl, style_bgl, surface_format,
    );
    let errorbar_pipeline = data_render::create_errorbar_columnar_pipeline(
        device, transform_bgl, style_bgl, surface_format,
    );
    (axis_pipeline, line_pipeline, scatter_pipeline, errorbar_pipeline)
}

#[derive(Clone, Copy)]
struct TargetPipelineRefs<'a> {
    axis: &'a wgpu::RenderPipeline,
    line: &'a wgpu::RenderPipeline,
    scatter: &'a wgpu::RenderPipeline,
    errorbar: &'a wgpu::RenderPipeline,
}

fn validate_texture_extent(
    caps: RendererDeviceCaps,
    resource: &'static str,
    width: u32,
    height: u32,
) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(FiggyError::InvalidChartArea { width, height });
    }
    let max_dim = width.max(height);
    if max_dim > caps.max_texture_dimension_2d {
        return Err(FiggyError::GpuResourceLimit {
            resource,
            requested: max_dim as u64,
            limit: caps.max_texture_dimension_2d as u64,
        });
    }
    Ok(())
}

fn validate_buffer_size(
    caps: RendererDeviceCaps,
    resource: &'static str,
    size: u64,
) -> Result<()> {
    if size > caps.max_buffer_size {
        return Err(FiggyError::GpuResourceLimit {
            resource,
            requested: size,
            limit: caps.max_buffer_size,
        });
    }
    Ok(())
}

fn create_texture_checked(
    device: &wgpu::Device,
    desc: &wgpu::TextureDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Texture> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_texture(desc)))
        .map_err(|_| FiggyError::GpuResourceAllocationFailed {
            resource,
            reason: "wgpu Device::create_texture panicked".into(),
        })
}

fn create_buffer_checked(
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Buffer> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_buffer(desc)))
        .map_err(|_| FiggyError::GpuResourceAllocationFailed {
            resource,
            reason: "wgpu Device::create_buffer panicked".into(),
        })
}

impl Renderer {
    /// Initialize every figgy GPU resource against the given device/queue pair.
    ///
    /// `pool_capacity_bytes` is the total size of the GPU column pool
    /// (sum of all chart data). `surface_format` is the final render target
    /// color format; every graphics pipeline is compiled against it.
    pub fn try_new(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
    ) -> Result<Self> {
        let RendererDevice { device, queue } = gpu;
        let caps = RendererDeviceCaps::from_device(&device);
        validate_target_format(caps, surface_format)?;

        let pool = ColumnPool::new(&device, pool_capacity_bytes)?;

        let texture_bgl = data_render::create_texture_bind_group_layout(&device);
        let transform_bgl = data_render::create_scatter_transform_bind_group_layout(&device);
        let style_bgl = data_render::create_style_bind_group_layout(&device);

        let sampler = data_render::create_linear_sampler(&device);
        let quad_vb = data_render::create_unit_centered_quad_vertex_buffer(&device);

        let (axis_pipeline, line_pipeline, scatter_pipeline, errorbar_pipeline) =
            create_target_pipelines(
                &device,
                &texture_bgl,
                &transform_bgl,
                &style_bgl,
                surface_format,
            );
        let arc_pipelines = data_render::line_arc::create_arc_scan_pipelines(&device);

        Ok(Self {
            device,
            queue,
            caps,
            pool,
            texture_bgl,
            transform_bgl,
            style_bgl,
            axis_pipeline,
            line_pipeline,
            scatter_pipeline,
            errorbar_pipeline,
            sampler,
            quad_vb,
            arc_cache: Mutex::new(HashMap::new()),
            arc_pipelines,
            surface_format,
        })
    }

    // Handle / accessor methods.

    pub fn device(&self) -> &Arc<wgpu::Device> { &self.device }
    pub fn queue(&self) -> &Arc<wgpu::Queue> { &self.queue }
    pub fn pool(&self) -> &ColumnPool { &self.pool }
    pub fn surface_format(&self) -> wgpu::TextureFormat { self.surface_format }

    /// Rebuild render-target pipelines if the host swap-chain format changed.
    ///
    /// Column data, chart views, textures, bind groups, styles, and buffers do
    /// not depend on the final color attachment format, so embedders can call
    /// this instead of tearing down the whole renderer state.
    pub fn ensure_target_format(&mut self, surface_format: wgpu::TextureFormat) -> Result<bool> {
        if self.surface_format == surface_format {
            return Ok(false);
        }
        validate_target_format(self.caps, surface_format)?;
        let (axis_pipeline, line_pipeline, scatter_pipeline, errorbar_pipeline) =
            create_target_pipelines(
                &self.device,
                &self.texture_bgl,
                &self.transform_bgl,
                &self.style_bgl,
                surface_format,
            );
        self.axis_pipeline = axis_pipeline;
        self.line_pipeline = line_pipeline;
        self.scatter_pipeline = scatter_pipeline;
        self.errorbar_pipeline = errorbar_pipeline;
        self.surface_format = surface_format;
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
    pub fn wait_idle(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        }
    }

    pub fn texture_bind_group_layout(&self) -> &wgpu::BindGroupLayout { &self.texture_bgl }
    pub fn transform_bind_group_layout(&self) -> &wgpu::BindGroupLayout { &self.transform_bgl }
    pub fn style_bind_group_layout(&self) -> &wgpu::BindGroupLayout { &self.style_bgl }

    pub fn axis_pipeline(&self) -> &wgpu::RenderPipeline { &self.axis_pipeline }
    pub fn line_pipeline(&self) -> &wgpu::RenderPipeline { &self.line_pipeline }
    pub fn scatter_pipeline(&self) -> &wgpu::RenderPipeline { &self.scatter_pipeline }
    pub fn errorbar_pipeline(&self) -> &wgpu::RenderPipeline { &self.errorbar_pipeline }

    pub fn sampler(&self) -> &wgpu::Sampler { &self.sampler }
    pub fn quad_vertex_buffer(&self) -> &wgpu::Buffer { &self.quad_vb }

    // Column management (returns Result).

    /// Add a column to the pool. `AllocError` is converted to `FiggyError::Pool`.
    pub fn add_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        Ok(self.pool.add_column(id.into(), source, &self.device, &self.queue)?)
    }

    pub fn remove_column(&mut self, id: &str) -> bool {
        self.pool.remove_column(id)
    }

    /// Compact every live column to the start of the pool. `true` iff
    /// anything actually moved.
    pub fn defragment(&mut self) -> Result<bool> {
        Ok(self.pool.defragment(&self.device, &self.queue)?)
    }

    pub fn set_defrag_policy(&mut self, policy: DefragPolicy) {
        self.pool.defrag_policy = policy;
    }

    pub fn handle_for(&self, id: &str) -> Result<ColumnHandle> {
        self.pool
            .handle_for(id)
            .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })
    }

    pub fn is_valid_handle(&self, h: &ColumnHandle) -> bool {
        self.pool.is_valid_handle(h)
    }

    // Standalone path — figgy owns instance / adapter / device / queue / surface.
    //
    // For hosts where figgy renders the window directly (e.g. winit). Caller
    // writes zero lines of wgpu setup code.

    /// Build every GPU resource plus a surface and swap chain from a window
    /// handle. The returned [`WindowedRenderer`] manages the surface and
    /// expects `draw` to be called once per frame.
    ///
    /// `target` can be anything convertible to `wgpu::SurfaceTarget`
    /// (`Arc<winit::Window>`, a raw-handle wrapper, a web canvas, …). The
    /// figgy crate does not depend on winit types itself. On wasm, drive this
    /// from the host event loop (e.g. `wasm_bindgen_futures::spawn_local`).
    pub async fn for_window_async<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        let instance = data_render::create_instance();
        let surface = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            data_render::create_surface_for_window(&instance, target)
        }))
        .map_err(|_| FiggyError::SurfaceCreationFailed {
            reason: "wgpu surface creation panicked; check platform window/canvas/thread constraints"
                .into(),
        })?
        .map_err(|e| FiggyError::SurfaceCreationFailed { reason: format!("{e}") })?;
        let adapter = data_render::request_adapter_for_surface_async(&instance, &surface)
            .await
            .map_err(|_| FiggyError::AdapterUnavailable)?;
        let (device, queue) = data_render::request_device_async(&adapter)
            .await
            .map_err(|e| FiggyError::DeviceCreationFailed { reason: format!("{e}") })?;
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let surface_config = data_render::try_configure_surface(
            &surface, &adapter, &device, size.0.max(1), size.1.max(1),
        )?;

        let inner = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            surface_config.format,
            pool_capacity_bytes,
        )?;

        Ok(WindowedRenderer {
            inner,
            surface,
            surface_config,
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

    // Host-integration API (egui_wgpu / iced_wgpu / standalone).
    //
    // `Renderer` does NOT own the surface, swap chain, or event loop — the
    // host (egui, iced, a wgpu app) manages those. Per-frame flow:
    //   1. Host calls `surface.get_current_texture()` or enters its paint
    //      callback.
    //   2. If needed, the host invokes `update_transform` / `refresh_axis`
    //      after `chart.consume_data_dirty` / `consume_raster_dirty` — this
    //      is the "prepare" stage where `device`/`queue` are accessible.
    //   3. The host hands a `&mut RenderPass` to `renderer.paint(pass, items)`.
    //
    // egui pseudocode:
    // ```ignore
    // let cb = egui_wgpu::CallbackFn::new()
    //     .prepare(|device, queue, _enc, res| {
    //         let r: &mut Renderer = res.get_mut().unwrap();
    //         if chart.consume_data_dirty() { r.update_transform(&view, &chart); }
    //         if chart.consume_raster_dirty() { r.refresh_axis(&mut view, &chart, rect)?; }
    //         vec![]
    //     })
    //     .paint(|_info, pass, res| {
    //         let r: &Renderer = res.get().unwrap();
    //         r.paint(pass, &items).unwrap();
    //     });
    // ```

    /// Build a `ChartStyle` from `cfg.render_type`'s line/scatter/errorbar
    /// styles. Missing components default to BLACK / 1 px.
    pub fn create_style_for_series(&self, cfg: &SeriesConfig) -> ChartStyle {
        self.create_style_for_series_scaled(cfg, 1.0)
    }

    /// Scaled variant of `create_style_for_series` — every pixel size (line
    /// width, dash lengths, point radius, errorbar widths/cap) is multiplied
    /// by `scale` (used by the high-DPI export path).
    pub fn create_style_for_series_scaled(&self, cfg: &SeriesConfig, scale: f32) -> ChartStyle {
        let line = match extract_line(&cfg.render_type) {
            Some(l) => {
                let mut s =
                    PrimitiveStyle::from_color_with_width(l.line_color, l.line_width * scale);
                let pattern = l.line_style.pattern();
                // GPU dash capacity is 8 scalars (2 × vec4); presets max out at 6.
                for (i, len) in pattern.iter().take(8).enumerate() {
                    s.dash[i / 4][i % 4] = len * scale;
                }
                s.dash_len = pattern.len().min(8) as u32;
                s
            }
            None => PrimitiveStyle::from_color_with_width(Color::BLACK, 1.0),
        };
        let scatter = match extract_scatter(&cfg.render_type) {
            Some(sc) => {
                let mut s = PrimitiveStyle::from_color(sc.point_color);
                s.point_radius_px = sc.point_size * scale;
                s.shape_id = data_render::shape_id(&sc.point_shape);
                s
            }
            None => PrimitiveStyle::from_color(Color::BLACK),
        };
        let errorbar = match extract_errorbar_style(&cfg.render_type) {
            Some(e) => {
                let mut s = PrimitiveStyle::from_color_with_width(
                    e.error_bar_color,
                    e.error_bar_width * scale,
                );
                s.cap_half_px = e.error_bar_cap_size * scale;
                s.cap_width_px = e.cap_width * scale;
                s
            }
            None => PrimitiveStyle::from_color(Color::BLACK),
        };
        self.create_style_from_primitives(line, scatter, errorbar)
    }

    /// Shared tail of `create_style_for_series*`: allocate the three style
    /// uniform buffers and bind groups from fully-built values.
    fn create_style_from_primitives(
        &self,
        line: PrimitiveStyle,
        scatter: PrimitiveStyle,
        errorbar: PrimitiveStyle,
    ) -> ChartStyle {
        let dev = &self.device;
        let line_buf = data_render::create_style_uniform_buffer(dev, &line);
        let sc_buf = data_render::create_style_uniform_buffer(dev, &scatter);
        let eb_buf = data_render::create_style_uniform_buffer(dev, &errorbar);
        ChartStyle {
            line_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &line_buf),
            scatter_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &sc_buf),
            errorbar_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &eb_buf),
        }
    }

    /// Build the per-panel GPU resources: grid + decoration textures and
    /// bind groups, plus the transform uniform buffer + bind group.
    /// `panel_rect` is the panel's pixel rect in surface coordinates.
    /// `chart.config_mut().chart_area` must match `panel_rect` before this
    /// call so the axis raster is drawn correctly.
    pub fn create_chart_view(
        &self,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<ChartView> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);

        // Grid layer (drawn below data): raster + texture + bind group.
        let grid_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(), axis_render::AxisLayerKind::Grid,
        )?;
        let grid_tex = data_render::upload_rgba_texture(
            &self.device,
            &self.queue,
            self.caps.max_texture_dimension_2d,
            w,
            h,
            &grid_rgba,
        )?;
        let grid_view_t = grid_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let grid_bg = data_render::create_texture_bind_group(
            &self.device, &self.texture_bgl, &grid_view_t, &self.sampler,
        );

        // Decoration layer (drawn above data).
        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(), axis_render::AxisLayerKind::Decoration,
        )?;
        let dec_tex = data_render::upload_rgba_texture(
            &self.device,
            &self.queue,
            self.caps.max_texture_dimension_2d,
            w,
            h,
            &dec_rgba,
        )?;
        let dec_view_t = dec_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let dec_bg = data_render::create_texture_bind_group(
            &self.device, &self.texture_bgl, &dec_view_t, &self.sampler,
        );

        let t = data_render::scatter_transform_from_config(chart.config());
        let transform_buffer = data_render::create_scatter_transform_uniform_buffer(&self.device, &t);
        let transform_bg = data_render::create_scatter_transform_bind_group(
            &self.device, &self.transform_bgl, &transform_buffer,
        );

        Ok(ChartView {
            grid_texture: grid_tex,
            grid_bind_group: grid_bg,
            decoration_texture: dec_tex,
            decoration_bind_group: dec_bg,
            transform_buffer,
            transform_bg,
            panel_rect,
        })
    }

    /// When only the axis range (`AxisOptions { scale, min, max }`) changes,
    /// only the transform uniform buffer needs to be rewritten. No CPU
    /// raster, no texture rebuild — one GPU write.
    pub fn update_transform(&self, view: &ChartView, chart: &Chart) {
        let t = data_render::scatter_transform_from_config(chart.config());
        data_render::update_scatter_transform(&self.queue, &view.transform_buffer, &t);
    }

    /// Re-rasterize both grid and decoration textures. Updates in place via
    /// `write_texture` when the size matches; allocates new textures
    /// otherwise. Also refreshes the transform UB (chart_area feeds it).
    pub fn refresh_axis(
        &self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        self.refresh_axis_with_selection(view, chart, panel_rect, &[])
    }

    /// [`Self::refresh_axis`] plus selection highlight boxes composited into
    /// the decoration layer (selection draws above the data, never below).
    /// `selection` comes from `Selectable::selection_box` / `HitMap` and is in
    /// absolute surface coordinates.
    pub fn refresh_axis_with_selection(
        &self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selection: &[crate::select::SelectionBox],
    ) -> Result<()> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);

        let grid_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(), axis_render::AxisLayerKind::Grid,
        )?;
        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba_with_selection(
            chart.config(), axis_render::AxisLayerKind::Decoration, selection,
        )?;

        self.refresh_one_layer(
            &mut view.grid_texture, &mut view.grid_bind_group,
            &grid_rgba, w, h,
        )?;
        self.refresh_one_layer(
            &mut view.decoration_texture, &mut view.decoration_bind_group,
            &dec_rgba, w, h,
        )?;
        view.panel_rect = panel_rect;

        self.update_transform(view, chart);
        Ok(())
    }

    /// Update one texture in place or by reallocation. Used for both
    /// the grid and decoration layers.
    fn refresh_one_layer(
        &self,
        tex: &mut wgpu::Texture,
        bg: &mut wgpu::BindGroup,
        rgba: &[u8],
        w: u32, h: u32,
    ) -> Result<()> {
        let same_size = tex.width() == w && tex.height() == h;
        if same_size {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex, mip_level: 0,
                    origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h),
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        } else {
            let new_tex = data_render::upload_rgba_texture(
                &self.device,
                &self.queue,
                self.caps.max_texture_dimension_2d,
                w,
                h,
                rgba,
            )?;
            let new_view_t = new_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let new_bg = data_render::create_texture_bind_group(
                &self.device, &self.texture_bgl, &new_view_t, &self.sampler,
            );
            *tex = new_tex;
            *bg = new_bg;
        }
        Ok(())
    }

    /// Draw multiple chart panels into one RenderPass.
    ///
    /// Takes a pass already opened by the host via `begin_render_pass`.
    /// `Renderer` owns the pool buffer, pipelines, and bind groups, so no
    /// external resources are needed.
    ///
    /// Returns `FiggyError::UnknownColumn` if a series references an unknown
    /// column id (or a stale handle).
    ///
    /// `target_size` is the color attachment pixel size of the current pass;
    /// panel_rect / data_area are clamped to it because wgpu validation would
    /// otherwise abort. Pass the surface pixel size reported by the host
    /// (winit, egui CallbackInfo, iced shader::Primitive, …).
    pub fn paint(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        items: &[ChartDrawItem<'_>],
    ) -> Result<()> {
        let pipelines = TargetPipelineRefs {
            axis: &self.axis_pipeline,
            line: &self.line_pipeline,
            scatter: &self.scatter_pipeline,
            errorbar: &self.errorbar_pipeline,
        };
        self.paint_with_pipelines(pass, target_size, items, pipelines)
    }

    fn paint_with_pipelines<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        items: &[ChartDrawItem<'a>],
        pipelines: TargetPipelineRefs<'a>,
    ) -> Result<()> {
        for item in items {
            let panel_rect = item.view.panel_rect;
            let data_area = item
                .chart_config
                .data_area()
                .map(|da| da.0)
                .unwrap_or(panel_rect);

            // Bundle every series's primitives for the panel into one call.
            let series_list =
                self.build_series_layers(item.view, item.chart_config, item.series, pipelines)?;

            data_render::draw_chart_panel_columnar(
                pass,
                target_size,
                panel_rect,
                data_area,
                AxisLayer { pipeline: pipelines.axis, bind_group: &item.view.grid_bind_group },
                &series_list,
                AxisLayer { pipeline: pipelines.axis, bind_group: &item.view.decoration_bind_group },
            );
        }
        Ok(())
    }

    /// Convert one panel's `Series` list into `SeriesLayers` ready for
    /// drawing. The `config.render_type` enum variant decides which of
    /// line/scatter/errorbar each series needs; column ids are resolved to
    /// handles via the pool. `chart_config` supplies the data→pixel
    /// transform that dashed lines need for their arc-length prefix.
    fn build_series_layers<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &Config,
        series_specs: &[Series<'a>],
        pipelines: TargetPipelineRefs<'a>,
    ) -> Result<Vec<data_render::SeriesLayers<'a>>> {
        let pool = &self.pool;
        let lookup = |id: &ColumnId| -> Result<ColumnHandle> {
            pool.handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.clone() })
        };

        let mut out = Vec::with_capacity(series_specs.len());
        for series in series_specs {
            let cfg = series.config;
            let rt = &cfg.render_type;
            let x_h = lookup(&cfg.x_column)?;
            let y_h = lookup(&cfg.y_column)?;

            let line = if has_line(rt) {
                let dashed = extract_line(rt)
                    .is_some_and(|l| !matches!(l.line_style, LineStylePreset::Solid));
                let arc = if dashed {
                    self.ensure_arc_prefix(&cfg.series_id, &cfg.x_column, &cfg.y_column, chart_config)
                } else {
                    None
                };
                Some(ColumnLineLayer {
                    pipeline: pipelines.line,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.line_bg,
                    pool_buffer: pool.buffer(),
                    x: x_h, y: y_h,
                    arc,
                })
            } else { None };

            let scatter = if has_scatter(rt) {
                Some(ColumnScatterLayer {
                    pipeline: pipelines.scatter,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.scatter_bg,
                    quad_vb: &self.quad_vb,
                    pool_buffer: pool.buffer(),
                    x: x_h, y: y_h,
                })
            } else { None };

            let errorbar = match (extract_err_y(rt), extract_err_x(rt)) {
                (None, None) => None,
                (ey_opt, ex_opt) => {
                    let zero = self.zero_handle()?;
                    let (ey_lo, ey_hi) = match ey_opt {
                        Some(ErrorRef::Symmetric { column }) => {
                            let h = lookup(column)?; (h, h)
                        }
                        Some(ErrorRef::Asymmetric { lower, upper }) => {
                            (lookup(lower)?, lookup(upper)?)
                        }
                        None => (zero, zero),
                    };
                    let (ex_lo, ex_hi) = match ex_opt {
                        Some(ErrorRef::Symmetric { column }) => {
                            let h = lookup(column)?; (h, h)
                        }
                        Some(ErrorRef::Asymmetric { lower, upper }) => {
                            (lookup(lower)?, lookup(upper)?)
                        }
                        None => (zero, zero),
                    };
                    Some(ColumnErrorBarDraw {
                        pipeline: pipelines.errorbar,
                        transform_bg: &view.transform_bg,
                        style_bg: &series.style.errorbar_bg,
                        pool_buffer: pool.buffer(),
                        x: x_h, y: y_h,
                        err_y_lo: ey_lo, err_y_hi: ey_hi,
                        err_x_lo: ex_lo, err_x_hi: ex_hi,
                    })
                }
            };

            out.push(data_render::SeriesLayers { errorbar, line, scatter });
        }
        Ok(out)
    }

    /// Ensure the GPU arc-length prefix for one dashed line series and return
    /// the buffer slice info. The whole computation runs on the GPU
    /// (`line_arc.wgsl` compute scan over the pool columns) — the data never
    /// returns to the CPU, keeping the pool's no-CPU-copy contract intact.
    ///
    /// The scan is re-dispatched on every draw that needs it (the prefix
    /// depends on the data→pixel transform): a handful of tiny compute
    /// dispatches submitted before the host's render pass, which queue order
    /// then sequences correctly. Buffers/bind groups are cached per series
    /// and rebuilt only when the series layout changes.
    fn ensure_arc_prefix(
        &self,
        series_id: &str,
        x_id: &str,
        y_id: &str,
        chart_config: &Config,
    ) -> Option<(Arc<wgpu::Buffer>, u64)> {
        let x = self.pool.slot(x_id)?;
        let y = self.pool.slot(y_id)?;
        let n = x.len_values.min(y.len_values);
        if n < 2 {
            return None;
        }
        let n = u32::try_from(n).ok()?;
        // Pool offsets are 256-aligned bytes → exact f32 element indices.
        let x_base = u32::try_from(x.offset / 4).ok()?;
        let y_base = u32::try_from(y.offset / 4).ok()?;
        let generation = self.pool.generation();
        let t = data_render::scatter_transform_from_config(chart_config);

        let mut cache = self.arc_cache.lock().unwrap();
        // Runaway-churn backstop: ids of long-removed series would otherwise
        // pin GPU memory forever. Rebuilt on demand, so clearing is safe.
        if cache.len() > 256 {
            cache.clear();
        }
        let stale = cache
            .get(series_id)
            .is_none_or(|s| !s.matches(n, x_base, y_base, generation));
        if stale {
            cache.insert(
                series_id.to_string(),
                data_render::line_arc::ArcScratch::build(
                    &self.device,
                    &self.arc_pipelines,
                    self.pool.buffer(),
                    n,
                    x_base,
                    y_base,
                    generation,
                ),
            );
        }
        let scratch = cache.get(series_id).expect("just inserted");

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy line arc encoder"),
        });
        scratch.dispatch(&self.queue, &mut encoder, &self.arc_pipelines, &t);
        self.queue.submit(std::iter::once(encoder.finish()));

        Some((Arc::clone(&scratch.arc), u64::from(n) * 4))
    }

    /// Handle of the zero-filled column used to pad the unused dimension of
    /// asymmetric errorbars. Caller must pre-register `"__zero"` via
    /// `renderer.add_column("__zero", &zero_col)`. Auto-registering would
    /// require `&mut self`, which conflicts with `paint`'s `&self` signature.
    fn zero_handle(&self) -> Result<ColumnHandle> {
        self.pool.handle_for("__zero").ok_or_else(|| FiggyError::UnknownColumn {
            id: "__zero (zero-fill column for the unused dim of an errorbar series; \
                 pre-register via `renderer.add_column(\"__zero\", &zero_col)`)".into(),
        })
    }

    // Headless PNG export.

    /// Render one chart panel offscreen at `scale × original` pixel
    /// dimensions and return an RGBA buffer. Fonts, line widths, and margins
    /// are all scaled proportionally so the result is visually consistent.
    ///
    /// - `scale` is clamped to [`MIN_EXPORT_SCALE`] .. [`MAX_EXPORT_SCALE`].
    /// - Background is fully transparent (alpha 0).
    /// - Output size is `chart_area × scale`.
    /// - Encoding / saving the PNG is the caller's responsibility — use
    ///   [`encode_png`] or [`Self::export_panel_png_bytes_async`].
    ///
    /// Async because the GPU→CPU readback must yield to the browser on wasm;
    /// on native the await resolves immediately (the device is polled to
    /// completion inline). Native callers can use the blocking
    /// [`Self::export_panel_rgba`] wrapper instead.
    pub async fn export_panel_rgba_async(
        &self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        let scale = clamp_export_scale(scale);
        let orig = chart.config().chart_area.0;
        let w = ((orig.width as f32) * scale).round().max(1.0) as u32;
        let h = ((orig.height as f32) * scale).round().max(1.0) as u32;
        validate_texture_extent(self.caps, "figgy export target dimension", w, h)?;
        let export_format = wgpu::TextureFormat::Rgba8Unorm;
        validate_target_format(self.caps, export_format)?;
        let export_features = export_format.guaranteed_format_features(self.caps.features);
        if !export_features
            .allowed_usages
            .contains(wgpu::TextureUsages::COPY_SRC)
        {
            return Err(FiggyError::UnsupportedSurfaceFormat {
                format: export_format,
                reason: "figgy export target is not guaranteed to support COPY_SRC".into(),
            });
        }

        // 1) Scaled config with chart_area overridden to (0,0,w,h) → temp chart.
        let mut scaled_config = chart.config().scaled(scale);
        scaled_config.chart_area = crate::layout::ChartArea(Rect {
            x: 0, y: 0, width: w, height: h,
        });
        let scaled_chart = Chart::new(scaled_config);

        // 2) Temp ChartView with scaled axis textures.
        let view = self.create_chart_view(&scaled_chart, Rect { x: 0, y: 0, width: w, height: h })?;

        // 3) Series styles — line_width also scales (extracted from render_type).
        let scaled_styles: Vec<ChartStyle> = series.iter()
            .map(|cfg| self.create_style_for_series_scaled(cfg, scale))
            .collect();
        let series_objs: Vec<Series<'_>> = series.iter().zip(scaled_styles.iter())
            .map(|(cfg, st)| Series { config: cfg, style: st })
            .collect();

        // 4) Offscreen target.
        let target_desc = wgpu::TextureDescriptor {
            label: Some("figgy export target"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: export_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let target_tex = create_texture_checked(&self.device, &target_desc, "figgy export target")?;
        let target_view = target_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let (
            export_axis_pipeline,
            export_line_pipeline,
            export_scatter_pipeline,
            export_errorbar_pipeline,
        ) = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            export_format,
        );
        let export_pipelines = TargetPipelineRefs {
            axis: &export_axis_pipeline,
            line: &export_line_pipeline,
            scatter: &export_scatter_pipeline,
            errorbar: &export_errorbar_pipeline,
        };

        // 5) Readback buffer. Allocate at most the hardware limit and read rows
        // sequentially if the full image would exceed it.
        let unpadded_bpr_u64 = u64::from(w) * 4;
        let align = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let padded_bpr_u64 = ((unpadded_bpr_u64 + align - 1) / align) * align;
        if padded_bpr_u64 > u64::from(u32::MAX) {
            return Err(FiggyError::GpuResourceLimit {
                resource: "figgy export bytes_per_row",
                requested: padded_bpr_u64,
                limit: u64::from(u32::MAX),
            });
        }
        let unpadded_bpr = unpadded_bpr_u64 as u32;
        let padded_bpr = padded_bpr_u64 as u32;
        let max_rows = (self.caps.max_buffer_size / padded_bpr_u64).min(u64::from(h));
        if max_rows == 0 {
            return Err(FiggyError::GpuResourceLimit {
                resource: "figgy export readback row",
                requested: padded_bpr_u64,
                limit: self.caps.max_buffer_size,
            });
        }
        let mut rows_per_chunk = max_rows.min(u64::from(u32::MAX)) as u32;
        let readback = loop {
            let size = padded_bpr_u64 * u64::from(rows_per_chunk);
            validate_buffer_size(self.caps, "figgy export readback buffer", size)?;
            let desc = wgpu::BufferDescriptor {
                label: Some("figgy export readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            };
            match create_buffer_checked(&self.device, &desc, "figgy export readback buffer") {
                Ok(buffer) => break buffer,
                Err(e) if rows_per_chunk > 1 => {
                    rows_per_chunk = (rows_per_chunk / 2).max(1);
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        };

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy export encoder"),
        });

        // 6) Render pass — transparent clear + a single paint.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy export pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let items = [ChartDrawItem {
                view: &view,
                chart_config: scaled_chart.config(),
                series: &series_objs,
            }];
            self.paint_with_pipelines(&mut pass, (w, h), &items, export_pipelines)?;
        }
        self.queue.submit(std::iter::once(encoder.finish()));

        // 7) Texture → readback buffer in row chunks.
        let bgra = false;
        let rgba_len = u64::from(w)
            .checked_mul(u64::from(h))
            .and_then(|px| px.checked_mul(4))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(FiggyError::GpuResourceLimit {
                resource: "figgy export rgba output",
                requested: u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4),
                limit: usize::MAX as u64,
            })?;
        let mut rgba = vec![0u8; rgba_len];
        let mut y0 = 0;
        while y0 < h {
            let rows = rows_per_chunk.min(h - y0);
            let chunk_size = padded_bpr_u64 * u64::from(rows);
            let mut copy_encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy export copy encoder"),
            });
            copy_encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: y0, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bpr),
                        rows_per_image: Some(rows),
                    },
                },
                wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
            );
            self.queue.submit(std::iter::once(copy_encoder.finish()));

            let slice = readback.slice(..chunk_size);
            let (tx, rx) = futures_channel::oneshot::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
            // Native: drive the device to completion so the await below
            // resolves immediately. On wasm the browser polls the device and
            // the await yields to the JS event loop instead.
            #[cfg(not(target_arch = "wasm32"))]
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.await
                .map_err(|e| FiggyError::GpuResourceAllocationFailed {
                    resource: "figgy export readback mapping",
                    reason: format!("map_async sender dropped: {e}"),
                })?
                .map_err(|e| FiggyError::GpuResourceAllocationFailed {
                    resource: "figgy export readback mapping",
                    reason: format!("map_async: {e:?}"),
                })?;
            let mapped = slice.get_mapped_range();

            for local_y in 0..rows {
                let src_off = (local_y * padded_bpr) as usize;
                let dst_off = ((y0 + local_y) * unpadded_bpr) as usize;
                let row = &mapped[src_off..src_off + unpadded_bpr as usize];
                for i in 0..(w as usize) {
                    let p = i * 4;
                    let (b0, b1, b2, b3) = (row[p], row[p + 1], row[p + 2], row[p + 3]);
                    let (r, g, b, a) = if bgra { (b2, b1, b0, b3) } else { (b0, b1, b2, b3) };
                    let (or, og, ob) = if a == 0 || a == 255 {
                        (r, g, b)
                    } else {
                        let a_f = a as f32 / 255.0;
                        (
                            ((r as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                            ((g as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                            ((b as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                        )
                    };
                    rgba[dst_off + p] = or;
                    rgba[dst_off + p + 1] = og;
                    rgba[dst_off + p + 2] = ob;
                    rgba[dst_off + p + 3] = a;
                }
            }
            drop(mapped);
            readback.unmap();
            y0 += rows;
        }

        Ok(RasterImage { width: w, height: h, rgba })
    }

    /// Convenience wrapper: export panel RGBA, then encode PNG bytes in
    /// memory. Saving the bytes to disk is up to the caller.
    pub async fn export_panel_png_bytes_async(
        &self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let img = self.export_panel_rgba_async(chart, series, scale).await?;
        encode_png(&img)
    }

    /// Blocking convenience wrapper around [`Self::export_panel_rgba_async`].
    /// Native only — on wasm, await the async variant from the host's event
    /// loop instead.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_rgba(
        &self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        pollster::block_on(self.export_panel_rgba_async(chart, series, scale))
    }

    /// Blocking convenience wrapper around
    /// [`Self::export_panel_png_bytes_async`]. Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_png_bytes(
        &self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        pollster::block_on(self.export_panel_png_bytes_async(chart, series, scale))
    }
}

// Export result type, scale clamping, PNG encoding.

/// Lower bound for export scale. 1.0 is screen-equivalent (96 DPI baseline,
/// `scale = dpi / 96`); 0.25 corresponds to ~24 DPI (thumbnails).
pub const MIN_EXPORT_SCALE: f32 = 0.25;
/// Upper bound. 8.0 corresponds to ~768 DPI; we clamp instead of erroring
/// because anything bigger explodes memory and time
/// (an 8× export of a 1920×1080 panel is 15360×8640 RGBA ≈ 530 MB).
pub const MAX_EXPORT_SCALE: f32 = 8.0;

/// Convert DPI to a `scale` value relative to the 96 DPI baseline.
pub fn dpi_to_scale(dpi: f32) -> f32 {
    clamp_export_scale(dpi / 96.0)
}

/// Clamp `scale` to [`MIN_EXPORT_SCALE`] .. [`MAX_EXPORT_SCALE`].
pub fn clamp_export_scale(scale: f32) -> f32 {
    if scale.is_nan() || scale <= 0.0 {
        return MIN_EXPORT_SCALE;
    }
    scale.clamp(MIN_EXPORT_SCALE, MAX_EXPORT_SCALE)
}

/// In-memory RGBA8 image (straight alpha).
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, channel order R, G, B, A.
    pub rgba: Vec<u8>,
}

/// Encode `img` as PNG bytes in memory. Saving to disk is the caller's job.
pub fn encode_png(img: &RasterImage) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| FiggyError::RasterWrapFailed { reason: format!("png header: {e}") })?;
        writer
            .write_image_data(&img.rgba)
            .map_err(|e| FiggyError::RasterWrapFailed { reason: format!("png write: {e}") })?;
    }
    Ok(buf)
}

// Public auxiliary types.

/// Per-panel GPU resources: separate grid + decoration textures (for layered
/// compositing) plus the transform uniform.
///
/// Grid (below data) and decoration (axis lines, labels — above data) are
/// rasterized to two textures so the GPU can composite in order:
/// `grid → data → decoration`.
pub struct ChartView {
    grid_texture: wgpu::Texture,
    grid_bind_group: wgpu::BindGroup,
    decoration_texture: wgpu::Texture,
    decoration_bind_group: wgpu::BindGroup,
    transform_buffer: wgpu::Buffer,
    transform_bg: wgpu::BindGroup,
    /// Panel pixel rect in surface coordinates.
    panel_rect: Rect,
}

impl ChartView {
    pub fn panel_rect(&self) -> Rect { self.panel_rect.clone() }
}

/// Bundle of style bind groups for line, scatter, and errorbar primitives.
/// Built via `Renderer::create_style_for_series*`. Multiple series in one
/// chart can share a single `ChartStyle` (`Series.style: &ChartStyle`).
///
/// The style uniform buffers live inside the bind groups — wgpu keeps bound
/// resources alive, so no named buffer fields are needed.
pub struct ChartStyle {
    line_bg: wgpu::BindGroup,
    scatter_bg: wgpu::BindGroup,
    errorbar_bg: wgpu::BindGroup,
}

/// One drawable series — `data_config::SeriesConfig` (declarative) plus a
/// `ChartStyle` (GPU uniform buffers). The unit of input to `Renderer::paint`.
pub struct Series<'a> {
    /// Column ids + render type + label. Pure declaration, no GPU state.
    pub config: &'a SeriesConfig,
    /// GPU style created by `Renderer::create_style_for_series`.
    pub style: &'a ChartStyle,
}

/// One chart panel to draw: view + chart config + series slice.
pub struct ChartDrawItem<'a> {
    pub view: &'a ChartView,
    pub chart_config: &'a crate::config::Config,
    pub series: &'a [Series<'a>],
}

// DataRenderType branching helpers — which layers are needed and which
// sub-style to extract.

fn has_line(rt: &DataRenderType) -> bool {
    matches!(
        rt,
        DataRenderType::Line { .. }
            | DataRenderType::ScatterLine { .. }
            | DataRenderType::LineScatterErrorbarX { .. }
            | DataRenderType::LineScatterErrorbarY { .. }
            | DataRenderType::LineScatterErrorbarXY { .. }
    )
}

fn has_scatter(rt: &DataRenderType) -> bool {
    !matches!(rt, DataRenderType::Line { .. })
}

fn extract_line(rt: &DataRenderType) -> Option<&DataLineStyleConfig> {
    match rt {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(line),
        _ => None,
    }
}

fn extract_scatter(rt: &DataRenderType) -> Option<&DataScatterStyleConfig> {
    match rt {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(scatter),
        DataRenderType::Line { .. } => None,
    }
}

fn extract_errorbar_style(rt: &DataRenderType) -> Option<&DataErrorBarStyleConfig> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_style, .. }
        | DataRenderType::ScatterErrorbarY { err_style, .. }
        | DataRenderType::ScatterErrorbarXY { err_style, .. }
        | DataRenderType::LineScatterErrorbarX { err_style, .. }
        | DataRenderType::LineScatterErrorbarY { err_style, .. }
        | DataRenderType::LineScatterErrorbarXY { err_style, .. } => Some(err_style),
        _ => None,
    }
}

fn extract_err_y(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarY { err_y, .. }
        | DataRenderType::ScatterErrorbarXY { err_y, .. }
        | DataRenderType::LineScatterErrorbarY { err_y, .. }
        | DataRenderType::LineScatterErrorbarXY { err_y, .. } => Some(err_y),
        _ => None,
    }
}

fn extract_err_x(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_x, .. }
        | DataRenderType::ScatterErrorbarXY { err_x, .. }
        | DataRenderType::LineScatterErrorbarX { err_x, .. }
        | DataRenderType::LineScatterErrorbarXY { err_x, .. } => Some(err_x),
        _ => None,
    }
}

// WindowedRenderer — figgy owns the surface and swap chain.

/// Returned by `Renderer::for_window`. The caller never touches the surface,
/// surface config, encoder, or render pass — just calls `draw` per frame.
///
/// `Deref<Target = Renderer>` is implemented, so `add_column`,
/// `create_chart_view`, `create_style_for_series`, … are callable directly.
pub struct WindowedRenderer<'w> {
    inner: Renderer,
    surface: wgpu::Surface<'w>,
    surface_config: wgpu::SurfaceConfiguration,
    /// Instance and adapter must outlive the surface; keep them pinned here.
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
}

impl<'w> std::ops::Deref for WindowedRenderer<'w> {
    type Target = Renderer;
    fn deref(&self) -> &Renderer { &self.inner }
}

impl<'w> std::ops::DerefMut for WindowedRenderer<'w> {
    fn deref_mut(&mut self) -> &mut Renderer { &mut self.inner }
}

impl Drop for WindowedRenderer<'_> {
    fn drop(&mut self) {
        self.inner.wait_idle();
    }
}

impl<'w> WindowedRenderer<'w> {
    pub fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    /// Reconfigure the swap chain after a window resize. The caller is then
    /// responsible for updating each chart's `chart_area` — figgy doesn't
    /// know your panel layout policy.
    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        data_render::reconfigure_surface(
            &self.surface, &self._adapter, self.inner.device(),
            &mut self.surface_config, w.max(1), h.max(1),
        )?;
        self.inner.ensure_target_format(self.surface_config.format)?;
        Ok(())
    }

    /// Draw one frame.
    ///
    /// Acquires the current swap-chain texture, builds an encoder + render
    /// pass, calls `paint(items)`, then submits and presents. The caller is
    /// only responsible for processing per-panel dirty flags
    /// (`chart.consume_*_dirty()` → `refresh_axis` / `update_transform`).
    pub fn draw(&mut self, clear: crate::color::Color, items: &[ChartDrawItem<'_>]) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(error @ (wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated)) => {
                // Let the caller decide whether to resize, retry, or exit.
                return Err(FiggyError::SurfaceAcquireFailed { error });
            }
            Err(error) => return Err(FiggyError::SurfaceAcquireFailed { error }),
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .inner
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy frame pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.inner.paint(
                &mut pass,
                (self.surface_config.width, self.surface_config.height),
                items,
            )?;
        }
        self.inner
            .queue()
            .submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;
    use crate::data_render::{create_instance, request_adapter, request_device};

    fn col_f64(data: Vec<f64>) -> Column<f64> {
        let min = data.iter().copied().fold(f64::INFINITY, f64::min);
        let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column { data, min, max }
    }

    /// Two line series on distinct column pairs must BOTH render — this
    /// mirrors the web demo (sine on x/y + rc on t/v) and guards the
    /// second-series draw path end to end via headless export.
    #[test]
    fn two_line_series_both_render() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let n = 512;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 * 6.28 / n as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
        let ts: Vec<f64> = (0..n).map(|i| i as f64 * 5.0 / n as f64).collect();
        let vs: Vec<f64> = ts.iter().map(|t| 1.0 - (-t).exp()).collect();
        r.add_column("x", &col_f64(xs)).unwrap();
        r.add_column("y", &col_f64(ys)).unwrap();
        r.add_column("t", &col_f64(ts)).unwrap();
        r.add_column("v", &col_f64(vs)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 640, height: 480 });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 6.5);
        chart.set_y_range(-1.2, 1.2);

        let line_cfg = |id: &str, x: &str, y: &str, color: Color| SeriesConfig {
            series_id: id.into(),
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: color,
                    line_width: 2.0,
                },
            },
        };
        let series = [
            line_cfg("sine", "x", "y", Color::BLACK),
            line_cfg("rc", "t", "v", Color::new(1.0, 0.0, 0.0, 1.0)),
        ];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (mut red, mut black) = (0usize, 0usize);
        for px in img.rgba.chunks_exact(4) {
            if px[3] > 16 {
                if px[0] > 120 && px[1] < 90 && px[2] < 90 { red += 1; }
                if px[0] < 90 && px[1] < 90 && px[2] < 90 { black += 1; }
            }
        }
        assert!(black > 300, "sine (black) curve missing: {black} px");
        assert!(red > 300, "rc (red) curve missing: {red} px");
    }

    /// The GPU arc-length scan must equal a sequential CPU reference for
    /// every dispatch shape: single block (n ≤ 256), one sums level, and two
    /// sums levels (n > 65 536). This is the correctness proof that lets the
    /// pool keep NO CPU copy of column data.
    #[test]
    fn gpu_arc_prefix_matches_cpu_reference() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 900, height: 600 });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-1.2, 1.2);

        for (case, n) in [("tiny", 3usize), ("one-level", 1000), ("two-level", 70_000)] {
            let xs: Vec<f64> = (0..n).map(|i| i as f64 / n as f64).collect();
            let ys: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin()).collect();
            let xid = format!("ax_{case}");
            let yid = format!("ay_{case}");
            r.add_column(&xid, &col_f64(xs.clone())).unwrap();
            r.add_column(&yid, &col_f64(ys.clone())).unwrap();

            let (arc_buf, len_bytes) = r
                .ensure_arc_prefix(&format!("s_{case}"), &xid, &yid, chart.config())
                .expect("arc prefix");

            // Read the GPU result back.
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: len_bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(&arc_buf, 0, &readback, 0, len_bytes);
            queue.submit(std::iter::once(enc.finish()));
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            let gpu: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();

            // Sequential CPU reference mirroring the shader math.
            let t = data_render::scatter_transform_from_config(chart.config());
            let px = |x: f64, y: f64| -> (f32, f32) {
                let nx = ((x as f32) - t.data_min[0]) / (t.data_max[0] - t.data_min[0]) * 2.0 - 1.0;
                let ny = ((y as f32) - t.data_min[1]) / (t.data_max[1] - t.data_min[1]) * 2.0 - 1.0;
                (nx / t.pixel_to_ndc[0], ny / t.pixel_to_ndc[1])
            };
            let mut acc = 0.0f32;
            let mut reference = vec![0.0f32];
            for i in 1..n {
                let a = px(xs[i - 1], ys[i - 1]);
                let b = px(xs[i], ys[i]);
                acc += ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                reference.push(acc);
            }

            assert_eq!(gpu.len(), n, "{case}: length");
            let total = reference.last().copied().unwrap_or(0.0).max(1.0);
            for (i, (g, c)) in gpu.iter().zip(reference.iter()).enumerate() {
                assert!(
                    (g - c).abs() <= total * 2e-4 + 0.05,
                    "{case}: arc[{i}] gpu={g} cpu={c} (total {total})"
                );
            }
        }
    }

    /// Log-axis auto-fit with zeros in the data must clamp the lower bound
    /// to the smallest POSITIVE value (via the pool's CPU shadow) instead of
    /// feeding log10 a non-positive min.
    #[test]
    fn log_auto_fit_clamps_to_smallest_positive() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();
        // Data spans 0..1000 with smallest positive 0.04.
        let vals = vec![0.0, 0.04, 1.0, 50.0, 1000.0];
        r.add_column("v", &col_f64(vals)).unwrap();

        let mut config = crate::default::default_config();
        config.left_y.scale = crate::config::AxisScale::Logarithmic;
        let mut chart = Chart::new(config);
        chart.auto_fit_y_union(r.pool(), &["v"], 0.0).unwrap();

        let min = chart.config().left_y.min;
        assert!(
            (min - 0.04).abs() < 1e-9,
            "log fit lower bound must be the smallest positive value, got {min}"
        );
        assert!(chart.config().left_y.max >= 1000.0);
    }

    /// `auto_fit_*` must leave a UNIFORM margin fraction on all four sides —
    /// measured in pixels, from the user's point of view. Data is a triangle
    /// wave touching its exact min/max so the ink bbox equals the data bbox.
    #[test]
    fn auto_fit_margins_are_uniform_on_all_sides() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let n = 400;
        let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
        // Triangle wave between exactly 10 and 90.
        let ys: Vec<f64> = (0..n)
            .map(|i| {
                let t = (i % 80) as f64 / 80.0;
                let tri = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
                10.0 + tri * 80.0
            })
            .collect();
        r.add_column("x", &col_f64(xs)).unwrap();
        r.add_column("y", &col_f64(ys)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 800, height: 600 });
        config.legend.visible = false;
        // Bare data area: no grid/tick ink inside to pollute the bbox.
        config.grid.show_major_x = false;
        config.grid.show_major_y = false;
        config.grid.show_minor_x = false;
        config.grid.show_minor_y = false;
        let mut chart = Chart::new(config);
        let pad = 0.05;
        chart.auto_fit_x_union(r.pool(), &["x"], pad).unwrap();
        chart.auto_fit_y_union(r.pool(), &["y"], pad).unwrap();

        let series = [SeriesConfig {
            series_id: "s".into(),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.0,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let da = chart.config().data_area().unwrap().0;

        let (mut min_x, mut max_x, mut min_y, mut max_y) = (usize::MAX, 0usize, usize::MAX, 0usize);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let p = &img.rgba[i..i + 4];
                if p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90 {
                    min_x = min_x.min(x); max_x = max_x.max(x);
                    min_y = min_y.min(y); max_y = max_y.max(y);
                }
            }
        }
        assert!(min_x < max_x, "no data ink found");

        // Expected margin fraction of the data_area extent on every side:
        // pad / (1 + 2·pad) of the full padded span.
        let expect = pad / (1.0 + 2.0 * pad);
        let close = |frac: f64, side: &str| {
            assert!(
                (frac - expect).abs() < 0.015,
                "{side} margin {frac:.3}, expected {expect:.3}"
            );
        };
        let daw = da.width as f64;
        let dah = da.height as f64;
        close((min_x as f64 - da.x as f64) / daw, "left");
        close((da.x as f64 + daw - 1.0 - max_x as f64) / daw, "right");
        close((min_y as f64 - da.y as f64) / dah, "top");
        close((da.y as f64 + dah - 1.0 - max_y as f64) / dah, "bottom");
    }

    /// A solid line stays CONTINUOUS no matter how dense the data is. With
    /// sub-pixel segments (here ~0.04 px each) naive per-segment quads
    /// degenerate into disconnected slivers — every x column the curve
    /// crosses must contain ink.
    #[test]
    fn dense_line_has_no_gaps() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let n = 50_000;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        // Decay curve like the report, plus measurement-style jitter: the
        // sub-pixel zig-zag flips the segment direction at every point,
        // which is what real noisy data does to the quad extrusion.
        let ys: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, x)| (-6.0 * x).exp() + if i % 2 == 0 { 0.0008 } else { -0.0008 })
            .collect();
        r.add_column("dx", &col_f64(xs)).unwrap();
        r.add_column("dy", &col_f64(ys)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 800, height: 400 });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-0.05, 1.05);

        let series = [SeriesConfig {
            series_id: "dense".into(),
            label: None,
            x_column: "dx".into(),
            y_column: "dy".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.5,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let col_has_red = |x: usize| {
            (0..h).any(|y| {
                let i = (y * w + x) * 4;
                let p = &img.rgba[i..i + 4];
                p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
            })
        };
        let first = (0..w).find(|&x| col_has_red(x)).expect("curve drew nothing");
        let last = (0..w).rev().find(|&x| col_has_red(x)).unwrap();
        let gaps: Vec<usize> = (first..=last).filter(|&x| !col_has_red(x)).collect();
        assert!(
            gaps.is_empty(),
            "dense solid line has {} gap columns between x={first} and x={last}: {:?}…",
            gaps.len(),
            &gaps[..gaps.len().min(20)]
        );

        // Continuity alone is not enough — a stippled line can still touch
        // every column. A 1.5 px stroke must average ≥ 1 ink pixel per
        // column along a near-horizontal curve.
        let red_per_col = |x: usize| {
            (0..h)
                .filter(|&y| {
                    let i = (y * w + x) * 4;
                    let p = &img.rgba[i..i + 4];
                    p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
                })
                .count()
        };
        let total: usize = (first..=last).map(red_per_col).sum();
        let density = total as f64 / (last - first + 1) as f64;
        assert!(
            density >= 1.0,
            "dense solid line is stippled: {density:.2} ink px per column (expected ≥ 1.0)"
        );
    }

    /// The square-cap fix must NOT change line semantics: NaN values still
    /// break the line (the caps may narrow a gap by ~line_width, never close
    /// a real one), and nothing assumes monotonic x — a path that doubles
    /// back stays continuous.
    #[test]
    fn caps_preserve_nan_breaks_and_nonmonotonic_paths() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 800, height: 400 });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-0.1, 1.1);

        let line_cfg = |id: &str, x: &str, y: &str| SeriesConfig {
            series_id: id.into(),
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.5,
                },
            },
        };
        let red_cols = |img: &crate::RasterImage| -> Vec<usize> {
            let (w, h) = (img.width as usize, img.height as usize);
            (0..w)
                .filter(|&x| {
                    (0..h).any(|y| {
                        let i = (y * w + x) * 4;
                        let p = &img.rgba[i..i + 4];
                        p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
                    })
                })
                .collect()
        };

        // 1) Dense noisy line with a NaN band: x ∈ [0.40, 0.45] → ~36 px of
        //    data area. The break must survive (gap may shrink by ~1.5 px of
        //    caps, so probe the gap's middle).
        let n = 20_000;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        let ys: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                if (0.40..0.45).contains(&x) {
                    f64::NAN
                } else {
                    0.5 + 0.3 * (x * 9.0).sin() + if i % 2 == 0 { 0.0008 } else { -0.0008 }
                }
            })
            .collect();
        r.add_column("gx", &col_f64(xs)).unwrap();
        r.add_column("gy", &col_f64(ys)).unwrap();

        let series = [line_cfg("gap", "gx", "gy")];
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let cols = red_cols(&img);
        let da = chart.config().data_area().unwrap().0;
        let to_px = |v: f64| da.x as f64 + v * da.width as f64;
        let (gap_a, gap_b) = (to_px(0.40), to_px(0.45));
        let mid_a = (gap_a + 4.0) as usize;
        let mid_b = (gap_b - 4.0) as usize;
        let leaked: Vec<&usize> =
            cols.iter().filter(|&&x| x >= mid_a && x <= mid_b).collect();
        assert!(
            leaked.is_empty(),
            "NaN break was bridged: ink at columns {leaked:?} inside the gap ({mid_a}..{mid_b})"
        );
        // Both sides of the gap still drew.
        assert!(cols.iter().any(|&x| (x as f64) < gap_a - 2.0), "left of gap missing");
        assert!(cols.iter().any(|&x| (x as f64) > gap_b + 2.0), "right of gap missing");

        // 2) Non-monotonic path: x sweeps 0→1 then back 1→0 at a higher y.
        //    Every column the path covers must have ink in BOTH passes.
        let m = 30_000;
        let xs2: Vec<f64> = (0..m)
            .map(|i| {
                let t = i as f64 / (m - 1) as f64;
                if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
            })
            .collect();
        let ys2: Vec<f64> = (0..m)
            .map(|i| {
                let t = i as f64 / (m - 1) as f64;
                let base = if t < 0.5 { 0.25 } else { 0.75 };
                base + if i % 2 == 0 { 0.0008 } else { -0.0008 }
            })
            .collect();
        r.add_column("mx", &col_f64(xs2)).unwrap();
        r.add_column("my", &col_f64(ys2)).unwrap();

        let series2 = [line_cfg("loop", "mx", "my")];
        let img2 = r.export_panel_rgba(&chart, &series2, 1.0).unwrap();
        let (w2, h2) = (img2.width as usize, img2.height as usize);
        let half = h2 / 2; // y=0.25 band renders below the midline, 0.75 above
        let band_has = |x: usize, top: bool| {
            let range = if top { 0..half } else { half..h2 };
            range.into_iter().any(|y| {
                let i = (y * w2 + x) * 4;
                let p = &img2.rgba[i..i + 4];
                p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
            })
        };
        let cols2 = red_cols(&img2);
        let (first2, last2) = (*cols2.first().unwrap(), *cols2.last().unwrap());
        let gaps2: Vec<usize> = (first2..=last2)
            .filter(|&x| !(band_has(x, true) && band_has(x, false)))
            .collect();
        assert!(
            gaps2.is_empty(),
            "non-monotonic path broke: {} columns missing one of the two passes: {:?}…",
            gaps2.len(),
            &gaps2[..gaps2.len().min(20)]
        );
    }

    /// Dash regularity on a CURVED line: the dash phase must advance with
    /// arc length, so on/off run lengths along the curve stay close to the
    /// pattern (Dash = [8, 4]). A phase that jumps at segment joints
    /// (per-segment restart, or screen-position projection) shatters the
    /// pattern into irregular fragments and fails the run statistics.
    #[test]
    fn dashed_curve_runs_match_pattern() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        // Worst case for joint phase errors: a SPARSE sine (large direction
        // change per segment) on a WIDE panel (fragments far from the origin
        // amplify any screen-position-derived phase).
        let n = 64;
        let ts: Vec<f64> = (0..n).map(|i| i as f64 * 6.28 / (n - 1) as f64).collect();
        let vs: Vec<f64> = ts.iter().map(|t| t.sin()).collect();
        r.add_column("t", &col_f64(ts)).unwrap();
        r.add_column("v", &col_f64(vs)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 1000, height: 600 });
        // Hide chrome that could add red-ish AA pixels; keep it minimal.
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 6.28);
        chart.set_y_range(-1.2, 1.2);

        let series = [SeriesConfig {
            series_id: "rc".into(),
            label: None,
            x_column: "t".into(),
            y_column: "v".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Dash, // [8, 4]
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 2.0,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let is_red = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            let p = &img.rgba[i..i + 4];
            p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
        };

        // Trace the x-monotonic curve column by column: ink centroid y per
        // column (None = gap), arc step ds = √(1 + dy²) per 1-px x step.
        // On/off run lengths in ARC pixels must then match the [8, 4]
        // pattern regardless of local slope.
        let col_y: Vec<Option<f64>> = (0..w)
            .map(|x| {
                let ys: Vec<usize> = (0..h).filter(|&y| is_red(x, y)).collect();
                if ys.is_empty() { None } else {
                    Some(ys.iter().sum::<usize>() as f64 / ys.len() as f64)
                }
            })
            .collect();
        let x0 = col_y.iter().position(|c| c.is_some()).expect("dashed curve drew nothing");
        let x1 = col_y.iter().rposition(|c| c.is_some()).unwrap();

        let mut runs_on: Vec<f64> = Vec::new();
        let mut runs_off: Vec<f64> = Vec::new();
        let mut cur_on = true;
        let mut run = 0.0f64;
        let mut last_y = col_y[x0].unwrap();
        for x in x0..=x1 {
            let (on, ds) = match col_y[x] {
                Some(y) => {
                    let ds = (1.0 + (y - last_y).powi(2)).sqrt();
                    last_y = y;
                    (true, ds)
                }
                // In a gap the curve continues invisibly — approximate the
                // arc step with the local slope we last saw.
                None => (false, 1.0),
            };
            if on == cur_on {
                run += ds;
            } else {
                if cur_on { runs_on.push(run) } else { runs_off.push(run) }
                cur_on = on;
                run = ds;
            }
        }
        if cur_on { runs_on.push(run) } else { runs_off.push(run) }

        let median = |v: &mut Vec<f64>| -> f64 {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if v.is_empty() { 0.0 } else { v[v.len() / 2] }
        };
        let needles = runs_on.iter().filter(|&&l| l <= 3.0).count();
        let n_on = runs_on.len();
        let med_on = median(&mut runs_on);
        let med_off = median(&mut runs_off);

        // Dash pattern is [8 on, 4 off] in arc px (line caps blur ±~2 px).
        assert!(
            (5.0..=12.0).contains(&med_on),
            "median on-run {med_on:.1} arc-px, expected ≈8 (runs: {runs_on:?})"
        );
        assert!(
            (2.0..=7.0).contains(&med_off),
            "median off-run {med_off:.1} arc-px, expected ≈4 (runs: {runs_off:?})"
        );
        assert!(
            needles * 5 <= n_on.max(1),
            "{needles}/{n_on} on-runs are needle fragments (≤3 arc-px): {runs_on:?}"
        );
    }

    #[test]
    fn renderer_init_and_add_column() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();

        let c = col_f64((0..100).map(|i| i as f64).collect());
        let h = r.add_column("x", &c).unwrap();
        assert_eq!(h.len_values, 100);

        let h2 = r.handle_for("x").unwrap();
        assert!(r.is_valid_handle(&h2));

        // Unknown id → UnknownColumn.
        let res = r.handle_for("nope");
        assert!(matches!(res, Err(FiggyError::UnknownColumn { .. })));
    }
}
