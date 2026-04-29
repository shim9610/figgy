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
//! `try_new` returns `Result<_, FiggyError>` for forward compatibility — no
//! current sub-helper can fail, but keeping the signature fallible means
//! adding a fallible helper later won't break callers.

use std::sync::Arc;

use crate::axis_render;
use crate::chart::Chart;
use crate::color::Color;
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

/// Facade bundling every figgy GPU resource.
pub struct Renderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
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

    surface_format: wgpu::TextureFormat,
}

impl Renderer {
    /// Initialize every figgy GPU resource against the given device/queue.
    ///
    /// `pool_capacity_bytes` is the total size of the GPU column pool
    /// (sum of all chart data). `surface_format` is the final render target
    /// color format; every graphics pipeline is compiled against it.
    pub fn try_new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
    ) -> Result<Self> {
        let pool = ColumnPool::new(&device, pool_capacity_bytes);

        let texture_bgl = data_render::create_texture_bind_group_layout(&device);
        let transform_bgl = data_render::create_scatter_transform_bind_group_layout(&device);
        let style_bgl = data_render::create_style_bind_group_layout(&device);

        let sampler = data_render::create_linear_sampler(&device);
        let quad_vb = data_render::create_unit_centered_quad_vertex_buffer(&device);

        let axis_pipeline = data_render::create_fullscreen_textured_pipeline(
            &device, &texture_bgl, surface_format,
        );
        let line_pipeline = data_render::create_line_columnar_pipeline(
            &device, &transform_bgl, &style_bgl, surface_format,
        );
        let scatter_pipeline = data_render::create_scatter_columnar_pipeline(
            &device, &transform_bgl, &style_bgl, surface_format,
        );
        let errorbar_pipeline = data_render::create_errorbar_columnar_pipeline(
            &device, &transform_bgl, &style_bgl, surface_format,
        );

        Ok(Self {
            device,
            queue,
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
            surface_format,
        })
    }

    // Handle / accessor methods.

    pub fn device(&self) -> &Arc<wgpu::Device> { &self.device }
    pub fn queue(&self) -> &Arc<wgpu::Queue> { &self.queue }
    pub fn pool(&self) -> &ColumnPool { &self.pool }
    pub fn pool_mut(&mut self) -> &mut ColumnPool { &mut self.pool }
    pub fn surface_format(&self) -> wgpu::TextureFormat { self.surface_format }

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
    pub fn defragment(&mut self) -> bool {
        self.pool.defragment(&self.device, &self.queue)
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
    /// (`Arc<winit::Window>`, a raw-handle wrapper, …). The figgy crate does
    /// not depend on winit types itself.
    pub fn for_window<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        let instance = data_render::create_instance();
        let surface = data_render::create_surface_for_window(&instance, target)
            .map_err(|e| FiggyError::SurfaceCreationFailed { reason: format!("{e}") })?;
        let adapter = data_render::request_adapter_for_surface(&instance, &surface)
            .map_err(|_| FiggyError::AdapterUnavailable)?;
        let (device, queue) = data_render::request_device(&adapter)
            .map_err(|e| FiggyError::DeviceCreationFailed { reason: format!("{e}") })?;
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let surface_config = data_render::configure_surface(
            &surface, &adapter, &device, size.0.max(1), size.1.max(1),
        );

        let inner = Renderer::try_new(
            Arc::clone(&device),
            Arc::clone(&queue),
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

    /// Scaled variant of `create_style_for_series` — line width is multiplied
    /// by `scale` (used by the high-DPI export path).
    pub fn create_style_for_series_scaled(&self, cfg: &SeriesConfig, scale: f32) -> ChartStyle {
        let (line_c, line_w) = extract_line(&cfg.render_type)
            .map(|l| (l.line_color, l.line_width * scale))
            .unwrap_or((Color::BLACK, 1.0));
        let scatter_c = extract_scatter(&cfg.render_type)
            .map(|s| s.point_color)
            .unwrap_or(Color::BLACK);
        let err_c = extract_errorbar_style(&cfg.render_type)
            .map(|e| e.error_bar_color)
            .unwrap_or(Color::BLACK);
        self.create_style(line_c, line_w, scatter_c, err_c)
    }

    /// Lower-level: build a `ChartStyle` from explicit colors and line width,
    /// bypassing `DataRenderType`.
    pub fn create_style(
        &self,
        line_color: Color,
        line_width_px: f32,
        scatter_color: Color,
        errorbar_color: Color,
    ) -> ChartStyle {
        let dev = &self.device;
        let line_buf = data_render::create_style_uniform_buffer(
            dev, &PrimitiveStyle::from_color_with_width(line_color, line_width_px));
        // Scatter / errorbar use 1 px width as a placeholder; not used for stroke yet.
        let sc_buf = data_render::create_style_uniform_buffer(
            dev, &PrimitiveStyle::from_color_with_width(scatter_color, 1.0));
        let eb_buf = data_render::create_style_uniform_buffer(
            dev, &PrimitiveStyle::from_color_with_width(errorbar_color, 1.0));
        ChartStyle {
            line_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &line_buf),
            scatter_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &sc_buf),
            errorbar_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &eb_buf),
            line_buf,
            scatter_buf: sc_buf,
            errorbar_buf: eb_buf,
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
        let grid_tex = data_render::upload_rgba_texture(&self.device, &self.queue, w, h, &grid_rgba);
        let grid_view_t = grid_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let grid_bg = data_render::create_texture_bind_group(
            &self.device, &self.texture_bgl, &grid_view_t, &self.sampler,
        );

        // Decoration layer (drawn above data).
        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(), axis_render::AxisLayerKind::Decoration,
        )?;
        let dec_tex = data_render::upload_rgba_texture(&self.device, &self.queue, w, h, &dec_rgba);
        let dec_view_t = dec_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let dec_bg = data_render::create_texture_bind_group(
            &self.device, &self.texture_bgl, &dec_view_t, &self.sampler,
        );

        let t = data_render::scatter_transform_from_config(chart.config(), 5.0);
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
        let t = data_render::scatter_transform_from_config(chart.config(), 5.0);
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
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);

        let grid_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(), axis_render::AxisLayerKind::Grid,
        )?;
        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(), axis_render::AxisLayerKind::Decoration,
        )?;

        self.refresh_one_layer(
            &mut view.grid_texture, &mut view.grid_bind_group,
            &grid_rgba, w, h,
        );
        self.refresh_one_layer(
            &mut view.decoration_texture, &mut view.decoration_bind_group,
            &dec_rgba, w, h,
        );
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
    ) {
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
            let new_tex = data_render::upload_rgba_texture(&self.device, &self.queue, w, h, rgba);
            let new_view_t = new_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let new_bg = data_render::create_texture_bind_group(
                &self.device, &self.texture_bgl, &new_view_t, &self.sampler,
            );
            *tex = new_tex;
            *bg = new_bg;
        }
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
        for item in items {
            let panel_rect = item.view.panel_rect;
            let data_area = item
                .chart_config
                .data_area()
                .map(|da| da.0)
                .unwrap_or(panel_rect);

            // Bundle every series's primitives for the panel into one call.
            let series_list = self.build_series_layers(item.view, item.series)?;

            data_render::draw_chart_panel_columnar(
                pass,
                target_size,
                panel_rect,
                data_area,
                AxisLayer { pipeline: &self.axis_pipeline, bind_group: &item.view.grid_bind_group },
                &series_list,
                AxisLayer { pipeline: &self.axis_pipeline, bind_group: &item.view.decoration_bind_group },
            );
        }
        Ok(())
    }

    /// Convert one panel's `Series` list into `SeriesLayers` ready for
    /// drawing. The `config.render_type` enum variant decides which of
    /// line/scatter/errorbar each series needs; column ids are resolved to
    /// handles via the pool.
    fn build_series_layers<'a>(
        &'a self,
        view: &'a ChartView,
        series_specs: &[Series<'a>],
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
                Some(ColumnLineLayer {
                    pipeline: &self.line_pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.line_bg,
                    pool_buffer: pool.buffer(),
                    x: x_h, y: y_h,
                })
            } else { None };

            let scatter = if has_scatter(rt) {
                Some(ColumnScatterLayer {
                    pipeline: &self.scatter_pipeline,
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
                        pipeline: &self.errorbar_pipeline,
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
    ///   [`encode_png`] or [`Self::export_panel_png_bytes`].
    pub fn export_panel_rgba(
        &self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        let scale = clamp_export_scale(scale);
        let orig = chart.config().chart_area.0;
        let w = ((orig.width as f32) * scale).round().max(1.0) as u32;
        let h = ((orig.height as f32) * scale).round().max(1.0) as u32;

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
        let target_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("figgy export target"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.surface_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // 5) Readback buffer.
        let bpp = 4u32;
        let unpadded_bpr = w * bpp;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bpr = ((unpadded_bpr + align - 1) / align) * align;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("figgy export readback"),
            size: (padded_bpr as u64) * (h as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

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
            self.paint(&mut pass, (w, h), &items)?;
        }

        // 7) Texture → readback buffer.
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        // 8) Map + sync wait.
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        rx.recv()
            .expect("map_async sender dropped")
            .map_err(|e| FiggyError::DeviceCreationFailed { reason: format!("map_async: {e:?}") })?;
        let mapped = slice.get_mapped_range();

        // 9) tight RGBA + (BGRA→RGBA) + premul→straight.
        let bgra = matches!(
            self.surface_format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb,
        );
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            let src_off = (y * padded_bpr) as usize;
            let dst_off = (y * unpadded_bpr) as usize;
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

        Ok(RasterImage { width: w, height: h, rgba })
    }

    /// Convenience wrapper: export panel RGBA, then encode PNG bytes in
    /// memory. Saving the bytes to disk is up to the caller.
    pub fn export_panel_png_bytes(
        &self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let img = self.export_panel_rgba(chart, series, scale)?;
        encode_png(&img)
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

/// Bundle of style uniform buffers + bind groups for line, scatter, and
/// errorbar primitives. Built via `Renderer::create_style*`. Multiple series
/// in one chart can share a single `ChartStyle` (`Series.style: &ChartStyle`).
pub struct ChartStyle {
    line_buf: wgpu::Buffer,
    scatter_buf: wgpu::Buffer,
    errorbar_buf: wgpu::Buffer,
    line_bg: wgpu::BindGroup,
    scatter_bg: wgpu::BindGroup,
    errorbar_bg: wgpu::BindGroup,
}

impl ChartStyle {
    /// Change the line color via a single uniform write; bind group is reused.
    pub fn update_line_color(&self, queue: &wgpu::Queue, color: Color) {
        Self::write(queue, &self.line_buf, color);
    }
    pub fn update_scatter_color(&self, queue: &wgpu::Queue, color: Color) {
        Self::write(queue, &self.scatter_buf, color);
    }
    pub fn update_errorbar_color(&self, queue: &wgpu::Queue, color: Color) {
        Self::write(queue, &self.errorbar_buf, color);
    }
    fn write(queue: &wgpu::Queue, buf: &wgpu::Buffer, color: Color) {
        let s = PrimitiveStyle::from_color(color);
        queue.write_buffer(buf, 0, bytemuck::bytes_of(&s));
    }
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
/// `create_chart_view`, `create_style`, … are callable directly.
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
    pub fn resize(&mut self, w: u32, h: u32) {
        data_render::reconfigure_surface(
            &self.surface, self.inner.device(),
            &mut self.surface_config, w.max(1), h.max(1),
        );
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
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                // Swap chain invalid — resize on the next frame will recover.
                return Ok(());
            }
            Err(_) => return Ok(()),
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

    fn col_f64(idx: usize, data: Vec<f64>) -> Column<f64> {
        let min = data.iter().copied().fold(f64::INFINITY, f64::min);
        let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column { index: idx, data, min, max }
    }

    #[test]
    fn renderer_init_and_add_column() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            Arc::clone(&device),
            Arc::clone(&queue),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();

        let c = col_f64(0, (0..100).map(|i| i as f64).collect());
        let h = r.add_column("x", &c).unwrap();
        assert_eq!(h.len_values, 100);

        let h2 = r.handle_for("x").unwrap();
        assert!(r.is_valid_handle(&h2));

        // Unknown id → UnknownColumn.
        let res = r.handle_for("nope");
        assert!(matches!(res, Err(FiggyError::UnknownColumn { .. })));
    }
}
