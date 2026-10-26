//! PNG and RGBA export of charts and documents: scale limits, the export
//! render path and the per-panel draw storage it uses.

use super::*;

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
            .map_err(|e| FiggyError::RasterWrapFailed {
                reason: format!("png header: {e}"),
            })?;
        writer
            .write_image_data(&img.rgba)
            .map_err(|e| FiggyError::RasterWrapFailed {
                reason: format!("png write: {e}"),
            })?;
    }
    Ok(buf)
}

/// What an export renders: one panel or the whole layered document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExportScope {
    Panel,
    Document,
}

/// Owned storage for the internal panels of a prepare or export call: the
/// expanded configs and each panel's slice of the series, so the per-panel
/// `ChartDrawItem`s can borrow from one place. Expansion copies configuration
/// only; the series entries are the caller's references.
///
/// Only layered items are materialized here; a single-layer item is drawn
/// through the caller's own references, so a frame without overlays
/// allocates nothing for expansion.
pub(super) struct PanelDrawStorage<'a> {
    pub(super) configs: Vec<Config>,
    pub(super) series: Vec<Vec<Series<'a>>>,
    pub(super) views: Vec<&'a ChartView>,
    /// Panels per layered source item, in source order.
    pub(super) counts: Vec<usize>,
}

impl<'a> PanelDrawStorage<'a> {
    pub(super) fn expand(items: &[ChartDrawItem<'a>]) -> Result<Self> {
        let mut configs = Vec::new();
        let mut series = Vec::new();
        let mut views = Vec::new();
        let mut counts = Vec::new();
        for item in items {
            if !is_layered(item.chart_config) {
                continue;
            }
            let panels = expand_for_render(item.chart_config)?;
            let panel_views = layer_views(item.view, panels.len())?;
            let mut partition: Vec<Vec<Series<'a>>> = vec![Vec::new(); panels.len()];
            for entry in item.series {
                let layer = item.chart_config.layer_of_series(&entry.config.series_id);
                partition[panel_index(item.chart_config, panels.len(), layer)].push(*entry);
            }
            counts.push(panels.len());
            configs.extend(panels);
            series.extend(partition);
            views.extend(panel_views);
        }
        Ok(Self {
            configs,
            series,
            views,
            counts,
        })
    }

    /// The per-panel draw items of `source` (the items `expand` was given),
    /// in source order: a single-layer item as it is, a layered item as its
    /// expanded panels.
    pub(super) fn items<'s>(&'s self, source: &'s [ChartDrawItem<'a>]) -> Vec<ChartDrawItem<'s>> {
        let mut out = Vec::with_capacity(source.len() + self.configs.len());
        let mut counts = self.counts.iter();
        let mut next = 0;
        for item in source {
            if !is_layered(item.chart_config) {
                out.push(ChartDrawItem {
                    view: item.view,
                    chart_config: item.chart_config,
                    series: item.series,
                });
                continue;
            }
            let count = *counts.next().expect("one count per layered item");
            for index in next..next + count {
                out.push(ChartDrawItem {
                    view: self.views[index],
                    chart_config: &self.configs[index],
                    series: &self.series[index],
                });
            }
            next += count;
        }
        out
    }
}

impl PanelDrawStorage<'_> {
    /// Panels per source item, in source order (1 for a single-layer item).
    pub(super) fn group_sizes(&self, source: &[ChartDrawItem<'_>]) -> Vec<usize> {
        let mut counts = self.counts.iter();
        source
            .iter()
            .map(|item| {
                if !is_layered(item.chart_config) {
                    1
                } else {
                    *counts.next().expect("one count per layered item")
                }
            })
            .collect()
    }
}

/// What [`Renderer::prepare_export`] builds for one export: its panels'
/// prepared draws and the target they render into, with every resource
/// those draws read kept alive until the export ends.
pub(super) struct ExportFrame {
    /// The target size in physical pixels.
    pub(super) size: (u32, u32),
    /// One prepared draw per panel, in panel order.
    pub(super) items: Vec<PreparedItem>,
    target: TrackedTexture,
    msaa: Option<MsaaTarget>,
    _view: ChartView,
    _styles: Vec<ChartStyle>,
    _pipelines: TargetPipelines,
    #[cfg(target_arch = "wasm32")]
    browser_scopes: Option<BrowserErrorScopes>,
}

impl Renderer {
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
    /// `Renderer::export_panel_rgba` wrapper instead (native only). `&mut self` for the same
    /// reason as [`Self::paint`]: the prepare phase (arc prefixes; the
    /// export's styled pipelines live in a per-call set instead).
    pub async fn export_panel_rgba_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        self.export_panel_rgba_with_clear_async(
            chart,
            series,
            scale,
            crate::color::Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        )
        .await
    }

    /// Render the whole document — the base layer and every overlay layer
    /// composited in order — offscreen at `scale × original` pixel
    /// dimensions and return an RGBA buffer. Same contract as
    /// [`Self::export_panel_rgba_async`] otherwise; for a document without
    /// overlays the two produce byte-identical images. The panel export of a
    /// layered document draws its base panel only (with the group data
    /// area), which is what a per-panel export means.
    pub async fn export_document_rgba_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        self.export_document_rgba_with_clear_async(
            chart,
            series,
            scale,
            crate::color::Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        )
        .await
    }

    /// [`Self::export_document_rgba_async`] with an explicit clear color.
    pub async fn export_document_rgba_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        self.export_rgba_with_clear_scoped(chart, series, scale, clear, ExportScope::Document)
            .await
    }

    /// Convenience wrapper: export the document, then encode PNG bytes.
    pub async fn export_document_png_bytes_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let img = self
            .export_document_rgba_async(chart, series, scale)
            .await?;
        encode_png(&img)
    }

    /// Convenience wrapper: export the document PNG with an explicit clear
    /// color.
    pub async fn export_document_png_bytes_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<Vec<u8>> {
        let img = self
            .export_document_rgba_with_clear_async(chart, series, scale, clear)
            .await?;
        encode_png(&img)
    }

    /// Blocking wrapper around [`Self::export_document_rgba_async`]. Native
    /// only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_document_rgba(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        pollster::block_on(self.export_document_rgba_async(chart, series, scale))
    }

    /// Blocking wrapper around [`Self::export_document_png_bytes_async`].
    /// Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_document_png_bytes(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        pollster::block_on(self.export_document_png_bytes_async(chart, series, scale))
    }

    /// Export a panel with an explicit clear color behind the chart.
    ///
    /// Wrapped in device error scopes. Export is where figgy asks for its
    /// largest transient resources — a target texture at `scale²`, an MSAA
    /// attachment at `scale² × samples`, and a readback buffer — and it is
    /// `async`, so every scope pop can actually be awaited here. Native uses
    /// wgpu's OOM guard; wasm uses short underlying `GPUDevice` OOM and internal
    /// scopes so internal/OOM classification and host scope boundaries remain
    /// explicit at the raw browser API boundary.
    /// Each browser scope pair is popped synchronously before the next await,
    /// so cancellation and a host sharing the device cannot cross its stack.
    /// The synchronous upload paths keep their `catch_unwind` wrapper instead:
    /// a future cannot be driven from them, and wasm cannot block a single
    /// thread to poll one.
    ///
    /// A device that reports OOM fails the export even when the encode
    /// succeeded — a partially-resident target renders garbage, and returning
    /// an image the caller would save is worse than an error.
    ///
    /// This method owns and completes its GPU submission, then reports a global
    /// submission boundary to the memory ledger. A split prepare/paint host must
    /// submit every command buffer it previously recorded from this renderer
    /// before starting an export; opaque host command buffers cannot be observed
    /// or retired selectively by the renderer.
    pub async fn export_panel_rgba_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        self.export_rgba_with_clear_scoped(chart, series, scale, clear, ExportScope::Panel)
            .await
    }

    pub(super) async fn export_rgba_with_clear_scoped(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
        scope: ExportScope,
    ) -> Result<RasterImage> {
        for item in series {
            self.require_resident_series(item)?;
        }
        #[cfg(target_arch = "wasm32")]
        let result = self
            .export_rgba_with_clear_inner(chart, series, scale, clear, scope)
            .await;

        #[cfg(not(target_arch = "wasm32"))]
        let result = {
            // wgpu 30's scope guard pops itself on drop, so an early return
            // inside the body cannot leave a scope on the device's stack for the
            // next caller to inherit.
            let scope_guard = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let rendered = self
                .export_rgba_with_clear_inner(chart, series, scale, clear, scope)
                .await;
            let out_of_memory = scope_guard.pop().await;
            match (rendered, out_of_memory) {
                (Ok(_), Some(error)) => Err(FiggyError::GpuResourceAllocationFailed {
                    resource: "figgy export target",
                    reason: format!("device reported out of memory during export: {error}"),
                }),
                (result, _) => result,
            }
        };
        // Every export submission and readback has completed and all transient
        // owners inside the inner future have been dropped.
        self.end_gpu_frame();
        result
    }

    pub(super) async fn export_rgba_with_clear_inner(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
        scope: ExportScope,
    ) -> Result<RasterImage> {
        let mut frame = self.prepare_export(chart, series, scale, scope)?;
        let panels = 0..frame.items.len();
        self.paint_export(&mut frame, panels, true, clear).await
    }

    /// The first half of an export: validate the series, scale and place
    /// the document at the origin of a `scale ×` target, build its views,
    /// styles and pipelines, and resolve every panel's prepared draw. Runs
    /// once per export; [`Self::paint_export`] renders from what it returns.
    /// Synchronous, so the browser error scopes it pushes are still open
    /// when the first paint pops them.
    pub(super) fn prepare_export(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        scope: ExportScope,
    ) -> Result<ExportFrame> {
        #[cfg(target_arch = "wasm32")]
        let browser_scopes = BrowserErrorScopes::push(&self.device, "figgy export target")?;

        for config in series {
            validate_contour_level_count(config.series_id.as_str(), &config.render_type)?;
            validate_line_fill(&self.pool, series.iter(), config)?;
            validate_polar_series(chart.config(), config)?;
        }
        validate_radar_rules(chart.config(), series)?;
        let scale = clamp_export_scale(scale);
        // A document export covers every area; a panel export the parent's.
        let orig = match scope {
            ExportScope::Document => chart.config().document_area(),
            ExportScope::Panel => chart.config().chart_area.0,
        };
        let w = ((orig.width as f32) * scale).round().max(1.0) as u32;
        let h = ((orig.height as f32) * scale).round().max(1.0) as u32;
        validate_texture_extent(self.caps, "figgy export target dimension", w, h)?;
        let export_format = wgpu::TextureFormat::Rgba8Unorm;
        let mut export_sample_count = preferred_msaa_sample_count(self.caps, export_format);
        validate_target_sample_count(self.caps, export_format, export_sample_count)?;
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

        // 1) Scaled config placed at the origin of a (0,0,w,h) target → temp
        // chart. Without subplots the chart area is the whole target. A panel
        // export places the parent's area at the origin.
        let target = Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        let scaled_config = match scope {
            ExportScope::Document => place_document(chart.config(), scale, target),
            ExportScope::Panel => {
                let mut panel = chart.config().scaled(scale);
                let parent = panel.chart_area.0;
                panel.translate_areas(-i64::from(parent.x), -i64::from(parent.y));
                panel.chart_area = crate::layout::ChartArea(target);
                panel
            }
        };
        let panel_rect = scaled_config.chart_area.0;
        let scaled_chart = Chart::new(scaled_config);

        // 2) Temp ChartView with scaled axis textures.
        let view = self.create_chart_view(&scaled_chart, panel_rect)?;

        // 3) Series styles — line_width also scales (extracted from render_type).
        let scaled_styles: Vec<ChartStyle> = series
            .iter()
            .map(|cfg| self.create_style_for_series_scaled_inner(cfg, scale))
            .collect::<Result<_>>()?;
        let series_objs: Vec<Series<'_>> = series
            .iter()
            .zip(scaled_styles.iter())
            .map(|(cfg, st)| Series {
                config: cfg,
                style: st,
            })
            .collect();
        let document = [ChartDrawItem {
            view: &view,
            chart_config: scaled_chart.config(),
            series: &series_objs,
        }];
        // One expansion path with `prepare`: the layered document becomes one
        // item per panel. A panel export keeps the base panel only.
        let storage = PanelDrawStorage::expand(&document)?;
        let mut items = storage.items(&document);
        let mut group_sizes = storage.group_sizes(&document);
        if scope == ExportScope::Panel {
            items.truncate(1);
            group_sizes = vec![1];
        }
        let items = items.as_slice();
        let mut contour_spacings = Vec::with_capacity(items.len());
        for item in items {
            let mut per_series = Vec::with_capacity(item.series.len());
            for series in item.series {
                let contour_enabled = effective_series_primitives(
                    &item.chart_config.draw_style,
                    &series.config.render_type,
                )
                .contour;
                per_series.push(preflight_contour_label_spacing(
                    series.config.series_id.as_str(),
                    &series.config.render_type,
                    contour_enabled,
                    scale,
                )?);
            }
            contour_spacings.push(per_series);
        }

        // 3.5) Per-item prepare phase (transform uniform write + arc-prefix
        // dispatch) — submitted before the render pass below, so queue order
        // sequences them.
        // 4) Offscreen target.
        let target_desc = wgpu::TextureDescriptor {
            label: Some("figgy export target"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: export_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        // Charged for the length of the export, so a concurrent pool growth
        // sees the export's bytes in its budget rather than after the fact.
        let target_tex = TrackedTexture::new(
            &self.gpu_ledger,
            GpuResourceKind::ExportTarget,
            create_texture_checked(&self.device, &target_desc, "figgy export target")?,
        );
        let msaa_export_target = match create_msaa_target(
            &self.device,
            &self.gpu_ledger,
            self.caps,
            "figgy export msaa target",
            w,
            h,
            export_format,
            export_sample_count,
        ) {
            Ok(target) => target,
            Err(_) if export_sample_count > 1 => {
                export_sample_count = 1;
                None
            }
            Err(e) => return Err(e),
        };
        // Per-call set against the export format. The same prepare-phase
        // ensure as `paint` fills the styled cache, so styled variants
        // compile only when this export actually draws with them.
        let mut export_target_pipelines = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            export_format,
            export_sample_count,
        );
        self.ensure_contour_label_pipelines(items);
        export_target_pipelines.ensure_precise_variants_for_items(
            &self.device,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            &self.point_value_map_bgl,
            &self.data_selection_bgl,
            &self.field_bgl,
            &self.vector_bgl,
            &self.category_bar_data_bgl,
            self.contour_label_pipelines.as_ref(),
            export_format,
            items,
        );
        export_target_pipelines.ensure_styles_for_items(
            &self.device,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            export_format,
            items,
        );
        // Export renders immediately rather than handing out a token, so the
        // receipts have nothing to be held for.
        let _ = self.ensure_field_scratches(items)?;
        self.ensure_value_map_scratches(items, scale)?;
        let _ = self.ensure_vector_scratches(items, scale)?;
        let (prepared_contours, _) =
            self.ensure_contour_scratches(items, scale, &contour_spacings)?;
        let prepared_arcs = self.prepare_arc_items(items)?;
        let (mut prepared_items, _) = self.resolve_prepared_items(
            items,
            &prepared_arcs,
            &prepared_contours,
            &export_target_pipelines,
        )?;
        lift_annotation_overlays(&mut prepared_items, &group_sizes);
        Ok(ExportFrame {
            size: (w, h),
            items: prepared_items,
            target: target_tex,
            msaa: msaa_export_target,
            _view: view,
            _styles: scaled_styles,
            _pipelines: export_target_pipelines,
            #[cfg(target_arch = "wasm32")]
            browser_scopes: Some(browser_scopes),
        })
    }

    /// The second half of an export: render `panels` of `frame` (a range of
    /// its panel indices, all of them for a full export) into its target,
    /// cleared to `clear`, and read the target back as straight-alpha RGBA.
    /// With `chrome` false only the data slot of each panel is drawn: the
    /// fills, bodies, picked highlights and value labels, without the grid,
    /// decoration and annotation textures. The PNG export paints every panel
    /// with the chrome; the vector export paints each panel group's data.
    pub(super) async fn paint_export(
        &self,
        frame: &mut ExportFrame,
        panels: std::ops::Range<usize>,
        chrome: bool,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        #[cfg(target_arch = "wasm32")]
        let browser_scopes = match frame.browser_scopes.take() {
            Some(scopes) => scopes,
            None => BrowserErrorScopes::push(&self.device, "figgy export target")?,
        };
        let (w, h) = frame.size;
        let target_tex = &frame.target;
        let target_view = target_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let (render_view, resolve_target, store) = match frame.msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&target_view), wgpu::StoreOp::Discard),
            None => (&target_view, None, wgpu::StoreOp::Store),
        };
        let prepared_items = &frame.items[panels];

        // 5) Readback buffer. Allocate at most the hardware limit and read rows
        // sequentially if the full image would exceed it.
        let unpadded_bpr_u64 = u64::from(w) * 4;
        let align = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let padded_bpr_u64 = unpadded_bpr_u64.div_ceil(align) * align;
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
                Ok(buffer) => {
                    break TrackedBuffer::new(&self.gpu_ledger, GpuResourceKind::Readback, buffer);
                }
                Err(e) if rows_per_chunk > 1 => {
                    rows_per_chunk = (rows_per_chunk / 2).max(1);
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy export encoder"),
            });

        // 6) Render pass — configured clear + a single paint.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy export pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: render_view,
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
            self.paint_prepared_items(&mut pass, (w, h), prepared_items, chrome)?;
        }
        self.queue.submit(std::iter::once(encoder.finish()));

        #[cfg(target_arch = "wasm32")]
        browser_scopes.request_all().finish().await?;

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
            #[cfg(target_arch = "wasm32")]
            let browser_scopes = BrowserErrorScopes::push(&self.device, "figgy export readback")?;

            let rows = rows_per_chunk.min(h - y0);
            let chunk_size = padded_bpr_u64 * u64::from(rows);
            let mut copy_encoder =
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("figgy export copy encoder"),
                    });
            copy_encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: target_tex,
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
                wgpu::Extent3d {
                    width: w,
                    height: rows,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit(std::iter::once(copy_encoder.finish()));

            let slice = readback.slice(..chunk_size);
            let (tx, rx) = futures_channel::oneshot::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });

            #[cfg(target_arch = "wasm32")]
            browser_scopes.request_all().finish().await?;

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
            let mapped =
                slice
                    .get_mapped_range()
                    .map_err(|e| FiggyError::GpuResourceAllocationFailed {
                        resource: "figgy export readback mapping",
                        reason: format!("get_mapped_range: {e:?}"),
                    })?;

            for local_y in 0..rows {
                let src_off = (local_y * padded_bpr) as usize;
                let dst_off = ((y0 + local_y) * unpadded_bpr) as usize;
                let row = &mapped[src_off..src_off + unpadded_bpr as usize];
                for i in 0..(w as usize) {
                    let p = i * 4;
                    let (b0, b1, b2, b3) = (row[p], row[p + 1], row[p + 2], row[p + 3]);
                    let (r, g, b, a) = if bgra {
                        (b2, b1, b0, b3)
                    } else {
                        (b0, b1, b2, b3)
                    };
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

        Ok(RasterImage {
            width: w,
            height: h,
            rgba,
        })
    }

    /// Convenience wrapper: export panel RGBA, then encode PNG bytes in
    /// memory. Saving the bytes to disk is up to the caller.
    pub async fn export_panel_png_bytes_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let img = self.export_panel_rgba_async(chart, series, scale).await?;
        encode_png(&img)
    }

    /// Convenience wrapper: export panel PNG with an explicit clear color.
    pub async fn export_panel_png_bytes_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<Vec<u8>> {
        let img = self
            .export_panel_rgba_with_clear_async(chart, series, scale, clear)
            .await?;
        encode_png(&img)
    }

    /// Blocking convenience wrapper around [`Self::export_panel_rgba_async`].
    /// Native only — on wasm, await the async variant from the host's event
    /// loop instead.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_rgba(
        &mut self,
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
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        pollster::block_on(self.export_panel_png_bytes_async(chart, series, scale))
    }
}
