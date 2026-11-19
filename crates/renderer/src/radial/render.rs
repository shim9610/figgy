//! The cache, the end of a frame and the raster exports.

use super::*;

impl RadialRenderer {
    /// Drop internal frame/annotation references. External frames stay valid.
    /// Retired allocation credits are processed through [`Self::end_frame`].
    pub fn clear_cache(&mut self) {
        self.cached = None;
        self.annotations_cached = None;
        self.annotation_layer.clear();
    }
    /// Call after submitting/discarding ALL recorded frames, including old
    /// snapshots. Completion credits are asynchronous, as in Renderer.
    pub fn end_frame(&self) {
        crate::renderer::retire_on_completion(self.gpu.ledger(), self.gpu.queue());
    }
    /// Render `chart` at logical `size` and `options.scale`, over
    /// `options.clear`, and read it back as straight-alpha RGBA8 (#160).
    /// Available on native and WASM; includes the model's current
    /// hover/selection. A radial chart has one panel, so `options.scope` does
    /// not change the image. [`Self::export_png`] encodes it.
    pub async fn export_rgba(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<RasterImage, RadialError> {
        self.export_rgba_with_background(chart, size, options, RadialExportBackground::Document)
            .await
    }

    /// [`Self::export_rgba`] over `background` instead of the chart's own
    /// (#313): transparent, or another colour. The override is the
    /// export's alone: the chart is read, never edited, and the frame
    /// [`Self::prepare`] returns for it keeps drawing the chart's
    /// background (the export binds its own copy of the frame's globals).
    pub async fn export_rgba_with_background(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
        background: RadialExportBackground,
    ) -> Result<RasterImage, RadialError> {
        let frame = self.prepare(chart, size, options.scale)?;
        let annotations = self.prepare_annotations_for(&frame, chart, options.scale)?;
        let bound = self.export_bind_group(&frame, background)?;
        let clear = export_clear(options.clear, background);
        self.render_rgba(frame.size, clear, |pass| {
            match &bound {
                Some((_, bind_group)) => frame.draw_bound(pass, bind_group),
                None => frame.draw(pass),
            }
            annotations.draw_overlay(pass);
        })
        .await
    }

    /// [`Self::export_rgba`], polling the device on this thread until the
    /// readback completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_rgba_blocking(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<RasterImage, RadialError> {
        self.export_rgba_with_background_blocking(
            chart,
            size,
            options,
            RadialExportBackground::Document,
        )
    }

    /// [`Self::export_rgba_with_background`], polling the device on this
    /// thread until the readback completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_rgba_with_background_blocking(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
        background: RadialExportBackground,
    ) -> Result<RasterImage, RadialError> {
        let frame = self.prepare(chart, size, options.scale)?;
        let annotations = self.prepare_annotations_for(&frame, chart, options.scale)?;
        let bound = self.export_bind_group(&frame, background)?;
        let clear = export_clear(options.clear, background);
        self.render_rgba_blocking(frame.size, clear, |pass| {
            match &bound {
                Some((_, bind_group)) => frame.draw_bound(pass, bind_group),
                None => frame.draw(pass),
            }
            annotations.draw_overlay(pass);
        })
    }

    /// [`Self::export_png`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_png_blocking(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<Vec<u8>, RadialError> {
        let image = self.export_rgba_blocking(chart, size, options)?;
        crate::encode_png(&image).map_err(radial_gpu_error)
    }

    /// [`Self::export_rgba`], encoded as PNG bytes in memory.
    pub async fn export_png(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<Vec<u8>, RadialError> {
        let image = self.export_rgba(chart, size, options).await?;
        crate::encode_png(&image).map_err(radial_gpu_error)
    }

    /// [`Self::export_rgba_with_background`], encoded as PNG bytes in memory.
    pub async fn export_png_with_background(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
        background: RadialExportBackground,
    ) -> Result<Vec<u8>, RadialError> {
        let image = self
            .export_rgba_with_background(chart, size, options, background)
            .await?;
        crate::encode_png(&image).map_err(radial_gpu_error)
    }

    /// Render `draw` into a cleared target of `width x height` physical
    /// pixels in the renderer's format and read it back as straight-alpha
    /// RGBA8 (linear, whatever the format's encoding).
    ///
    /// The export path every radial export goes through, with the same
    /// guards as [`crate::Renderer`]'s exports (#160): the device health is
    /// checked first, the work runs inside out-of-memory, validation and
    /// internal error scopes (native: around the whole render; browser:
    /// pushed before the first resource and popped before the readback), the
    /// completion of its transient resources is attached, and the health is
    /// checked again, so a GPU error during the export fails it.
    pub(super) async fn render_rgba(
        &self,
        size: (u32, u32),
        clear: crate::color::Color,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> Result<RasterImage, RadialError> {
        self.gpu.health().check().map_err(radial_gpu_error)?;
        #[cfg(not(target_arch = "wasm32"))]
        let result = crate::renderer::native_export_scoped(
            self.gpu.device(),
            self.render_rgba_inner(size, clear, draw),
            radial_gpu_error,
        )
        .await;
        #[cfg(target_arch = "wasm32")]
        let result = self.render_rgba_inner(size, clear, draw).await;
        self.render_rgba_end(result)
    }

    /// [`Self::render_rgba`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn render_rgba_blocking(
        &self,
        size: (u32, u32),
        clear: crate::color::Color,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> Result<RasterImage, RadialError> {
        self.gpu.health().check().map_err(radial_gpu_error)?;
        let result = crate::renderer::native_export_scoped_blocking(
            self.gpu.device(),
            || {
                let mut job = self.render_rgba_submit(size, clear, draw)?;
                job.readback
                    .wait_blocking()
                    .map_err(radial_readback_error)?;
                self.render_rgba_finish(job)
            },
            radial_gpu_error,
        );
        self.render_rgba_end(result)
    }

    /// The export's submission and readback have completed and its
    /// transient target and readback buffer are dropped: end the frame, and
    /// fail the export on a GPU error that reached the uncaptured-error
    /// handler meanwhile (a browser scope resolves asynchronously).
    fn render_rgba_end(
        &self,
        result: Result<RasterImage, RadialError>,
    ) -> Result<RasterImage, RadialError> {
        self.end_frame();
        let image = result?;
        self.gpu.health().check().map_err(radial_gpu_error)?;
        Ok(image)
    }

    async fn render_rgba_inner(
        &self,
        size: (u32, u32),
        clear: crate::color::Color,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> Result<RasterImage, RadialError> {
        let mut job = self.render_rgba_submit(size, clear, draw)?;
        #[cfg(target_arch = "wasm32")]
        if let Some(scopes) = job.scopes.take() {
            scopes
                .request_all()
                .finish()
                .await
                .map_err(radial_gpu_error)?;
        }
        job.readback.wait().await.map_err(radial_readback_error)?;
        self.render_rgba_finish(job)
    }

    /// The draw half of a radial export: render `draw` into a cleared
    /// target, copy it into the readback buffer and request the map.
    fn render_rgba_submit(
        &self,
        (width, height): (u32, u32),
        clear: crate::color::Color,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> Result<RadialRgbaJob, RadialError> {
        let stride = (u64::from(width) * 4).div_ceil(256) * 256;
        let bytes = stride * u64::from(height);
        if bytes > self.gpu.device().limits().max_buffer_size {
            return Err(RadialError::Invalid(crate::RuleKind::RadialReadbackLimit));
        }
        self.admit(u64::from(width) * u64::from(height) * 4 + bytes)?;
        let device = self.gpu.device();
        #[cfg(target_arch = "wasm32")]
        let browser_scopes = crate::renderer::BrowserErrorScopes::push(device, "radial export")
            .map_err(radial_gpu_error)?;
        // gpu-alloc: ExportTarget
        let target = crate::renderer::create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
                label: Some("radial export"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            "radial export target",
        )
        .map_err(radial_gpu_error)?;
        let target = TrackedTexture::new(self.gpu.ledger(), GpuResourceKind::ExportTarget, target);
        // gpu-alloc: Readback
        let buffer = gpu_created(device, "radial readback", || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("radial readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })?;
        let buffer = TrackedBuffer::new(self.gpu.ledger(), GpuResourceKind::Readback, buffer);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("radial export"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(clear.r),
                            g: f64::from(clear.g),
                            b: f64::from(clear.b),
                            a: f64::from(clear.a),
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue().submit([encoder.finish()]);
        let readback = GpuReadback::map(device, self.gpu.health(), &buffer, None);
        Ok(RadialRgbaJob {
            readback,
            size: (width, height),
            stride,
            _target: target,
            _buffer: buffer,
            #[cfg(target_arch = "wasm32")]
            scopes: Some(browser_scopes),
        })
    }

    /// After the job's wait: read the target back as straight-alpha RGBA8
    /// (linear, whatever the format's encoding).
    fn render_rgba_finish(&self, mut job: RadialRgbaJob) -> Result<RasterImage, RadialError> {
        let (width, height) = job.size;
        let stride = job.stride;
        let mut rgba = job
            .readback
            .finish(|data| {
                let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
                for row in data.chunks_exact(stride as usize) {
                    rgba.extend_from_slice(&row[..width as usize * 4]);
                }
                rgba
            })
            .map_err(radial_readback_error)?;
        for pixel in rgba.chunks_exact_mut(4) {
            if matches!(
                self.format,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
            ) {
                pixel.swap(0, 2);
            }
            if pixel[3] > 0 && pixel[3] < 255 {
                for i in 0..3 {
                    let alpha = f32::from(pixel[3]) / 255.0;
                    let mut c = f32::from(pixel[i]) / 255.0;
                    if self.format.is_srgb() {
                        c = if c <= 0.04045 {
                            c / 12.92
                        } else {
                            ((c + 0.055) / 1.055).powf(2.4)
                        };
                    }
                    c = (c / alpha).clamp(0.0, 1.0);
                    if self.format.is_srgb() {
                        c = if c <= 0.0031308 {
                            c * 12.92
                        } else {
                            1.055 * c.powf(1.0 / 2.4) - 0.055
                        };
                    }
                    pixel[i] = (c * 255.0).round() as u8;
                }
            }
        }
        Ok(RasterImage {
            width,
            height,
            rgba,
        })
    }
}
