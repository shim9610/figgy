//! The SVG and PDF exports and their recorded parts.

use super::*;

impl RadialRenderer {
    /// The vector export of `chart` at logical `size` and `scale` into
    /// `out` (#48): the frame's sectors and background as one image, from a
    /// frame laid out without labels and built uncached; then the title, the
    /// labels with their leader lines and the split connectors from the
    /// layout's own label emit, and the annotation overlay, as vector
    /// content. Every inside label is drawn above every sector at its lifted
    /// place, and no selection box is drawn.
    pub(crate) async fn write_vector(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
        clear: crate::color::Color,
        out: &mut dyn crate::vector_export::VectorOut,
    ) -> Result<(), RadialError> {
        let frame = self.vector_frame(chart, size, scale)?;
        let image = self
            .render_rgba(frame.size, clear, |pass| {
                self.draw_vector_frame(pass, &frame)
            })
            .await?;
        self.write_vector_rest(chart, size, scale, &frame, image, out)
    }

    /// [`Self::write_vector`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn write_vector_blocking(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
        clear: crate::color::Color,
        out: &mut dyn crate::vector_export::VectorOut,
    ) -> Result<(), RadialError> {
        let frame = self.vector_frame(chart, size, scale)?;
        let image = self.render_rgba_blocking(frame.size, clear, |pass| {
            self.draw_vector_frame(pass, &frame)
        })?;
        self.write_vector_rest(chart, size, scale, &frame, image, out)
    }

    /// The vector export's frame: laid out without labels, bound to a
    /// zeroed label texture.
    fn vector_frame(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<RadialVectorFrame, RadialError> {
        // A lost device refuses here, before any buffer is created: wgpu's
        // `create_buffer_init` panics on a destroyed device (#469), as in
        // `prepare`.
        self.gpu.health().check().map_err(radial_gpu_error)?;
        let (width, height) = self.frame_size(chart, size, scale)?;
        let count = chart.node_count();
        self.admit(
            u64::from(width) * u64::from(height) * 4
                + 112
                + count as u64 * std::mem::size_of::<Sector>() as u64,
        )?;
        let l = layout::layout(chart, size, scale, layout::Labels::None)?;
        // A texture starts zeroed: the frame draws no label.
        // gpu-alloc: PanelTexture
        let labels = crate::renderer::create_texture_checked(
            self.gpu.device(),
            &wgpu::TextureDescriptor {
                label: Some("radial vector export without labels"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            "radial export label texture",
        )
        .map_err(radial_gpu_error)?;
        let labels = TrackedTexture::new(self.gpu.ledger(), GpuResourceKind::PanelTexture, labels);
        let globals = frame_globals(chart, &l, (width, height), self.format);
        let (uniform, sectors, bind_group) = self.bind_frame(&globals, &l, &labels)?;
        Ok(RadialVectorFrame {
            size: (width, height),
            layout: l,
            bind_group,
            _labels: labels,
            _uniform: uniform,
            _sectors: sectors,
        })
    }

    fn draw_vector_frame(&self, pass: &mut wgpu::RenderPass<'_>, frame: &RadialVectorFrame) {
        let (width, height) = frame.size;
        pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, width, height);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &frame.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// After the frame image: write it, then the title, labels, leader
    /// lines, split connectors and annotation overlay as vector content.
    fn write_vector_rest(
        &self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
        frame: &RadialVectorFrame,
        image: RasterImage,
        out: &mut dyn crate::vector_export::VectorOut,
    ) -> Result<(), RadialError> {
        let (width, height) = frame.size;
        let l = &frame.layout;
        let failed = |e: crate::FiggyError| RadialError::Gpu(e.to_string());
        out.image([0, 0], &image).map_err(failed)?;
        drop(image);
        let canvas = crate::layout::Rect {
            x: 0,
            y: 0,
            width,
            height,
        };
        out.push_clip(canvas);
        let mut sink = crate::vector_export::VectorSink::new(
            out,
            crate::sketch::DecoStroker::Precise,
            crate::text_render::FontPolicy::Standard,
            [0.0, 0.0],
            scale,
            canvas,
        );
        let placed = layout::layout(chart, size, scale, layout::Labels::Into(&mut sink));
        if placed.is_ok() && chart.annotations.visible {
            // The overlay the frame's annotation layer draws
            // (`RadialAnnotationCache`): the radial space has no value axes,
            // so nothing goes below the slices.
            let request = crate::annotation::RasterRequest {
                layer: &chart.annotations,
                space: l.space,
                origin: [0.0, 0.0],
                size,
                scale,
                selected: None,
                // An export draws no selection and no snap guide (#440).
                guides: crate::layout::SnapGuides::NONE,
                policy: crate::text_render::FontPolicy::Standard,
                markers: None,
            };
            crate::annotation::emit_annotation_layer(
                &request,
                &mut crate::vector_export::SlotFilter {
                    slot: crate::sink::Slot::Above,
                    inner: &mut sink,
                },
                &mut |_| {},
            );
        }
        let error = sink.error.take();
        drop(sink);
        out.pop_clip();
        placed?;
        error.map_or(Ok(()), |e| Err(failed(e)))
    }
}

impl RadialRenderer {
    /// Export the chart as an SVG document at logical `size` and
    /// `options.scale`, with the same checks as [`Self::prepare`]: the slices
    /// and background (`options.clear`) as one image at `scale ×`
    /// resolution, the title, the labels with their leader lines, the split
    /// connectors and the annotation layer as vector content, glyphs as
    /// outlines. The SVG's `viewBox` is in physical pixels and its size the
    /// logical size in points (1 px = 0.75 pt). Every inside label is drawn
    /// above every slice, at its place in the PNG, and no selection box is
    /// drawn.
    pub async fn export_svg(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<String, RadialError> {
        self.export_vector::<crate::vector_export::SvgOut>(chart, size, options)
            .await
    }

    /// [`Self::export_svg`] as a one-page PDF: the page is the logical size
    /// in points, the image a deflated RGB image with a soft mask.
    pub async fn export_pdf(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<Vec<u8>, RadialError> {
        self.export_vector::<crate::vector_export::PdfOut>(chart, size, options)
            .await
    }

    /// [`Self::export_svg`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_svg_blocking(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<String, RadialError> {
        self.export_vector_blocking::<crate::vector_export::SvgOut>(chart, size, options)
    }

    /// [`Self::export_pdf`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_pdf_blocking(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<Vec<u8>, RadialError> {
        self.export_vector_blocking::<crate::vector_export::PdfOut>(chart, size, options)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn export_vector_blocking<F: crate::vector_export::VectorFile>(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<F::Output, RadialError> {
        let physical = self.frame_size(chart, size, options.scale)?;
        let mut out = F::create(physical, crate::vector_export::page_points(size));
        self.write_vector_blocking(chart, size, options.scale, options.clear, &mut out)?;
        out.finish_file()
            .map_err(|e| RadialError::Gpu(e.to_string()))
    }

    async fn export_vector<F: crate::vector_export::VectorFile>(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: crate::ExportOptions,
    ) -> Result<F::Output, RadialError> {
        let physical = self.frame_size(chart, size, options.scale)?;
        let mut out = F::create(physical, crate::vector_export::page_points(size));
        self.write_vector(chart, size, options.scale, options.clear, &mut out)
            .await?;
        out.finish_file()
            .map_err(|e| RadialError::Gpu(e.to_string()))
    }
}

/// Every part [`RadialRenderer::export_svg`] and
/// [`RadialRenderer::export_pdf`] write for `chart` at logical `size`
/// and `scale`, in writing order, recorded through
/// [`crate::sink::Parts`] from the same label emit: the frame image, then
/// the title, labels, leader lines, split connectors and the annotation
/// overlay. Test support (`cpu-twin`).
#[cfg(feature = "cpu-twin")]
#[doc(hidden)]
pub fn written_vector_parts(
    chart: &RadialChart,
    size: (u32, u32),
    scale: f32,
    out: &mut dyn FnMut(crate::WrittenPart),
) -> Result<(), RadialError> {
    // `layout::layout` reads a validated chart, as every other caller's.
    chart.validate().map_err(RadialError::Invalid)?;
    let canvas = crate::layout::Rect {
        x: 0,
        y: 0,
        width: (size.0 as f64 * f64::from(scale)).round() as u32,
        height: (size.1 as f64 * f64::from(scale)).round() as u32,
    };
    out(crate::WrittenPart::Image(canvas));
    let mut twin = crate::vector_export::PartsTwin::new(
        out,
        crate::text_render::FontPolicy::Standard,
        [0.0, 0.0],
        scale,
        canvas,
    );
    let l = layout::layout(chart, size, scale, layout::Labels::Into(&mut twin))?;
    if chart.annotations.visible {
        let request = crate::annotation::RasterRequest {
            layer: &chart.annotations,
            space: l.space,
            origin: [0.0, 0.0],
            size,
            scale,
            selected: None,
            // An export draws no selection and no snap guide (#440).
            guides: crate::layout::SnapGuides::NONE,
            policy: crate::text_render::FontPolicy::Standard,
            markers: None,
        };
        crate::annotation::emit_annotation_layer(
            &request,
            &mut crate::vector_export::SlotFilter {
                slot: crate::sink::Slot::Above,
                inner: &mut twin,
            },
            &mut |_| {},
        );
    }
    Ok(())
}
