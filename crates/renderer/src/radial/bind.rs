//! Frame sizes and the frame and export bind groups.

use super::*;

impl RadialRenderer {
    /// The physical size of `chart`'s frame at logical `size` and `scale`,
    /// after the checks every frame passes: a valid chart whose shown
    /// labels have text, a canvas of at least 160x160 at a scale of at least 0.5,
    /// inside the device's texture limit.
    pub(super) fn frame_size(
        &self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<(u32, u32), RadialError> {
        chart.validate().map_err(RadialError::Invalid)?;
        if !chart.labels_have_text() {
            return Err(RadialError::Invalid(
                crate::RuleKind::RadialLabelWithoutText,
            ));
        }
        if size.0 < 160 || size.1 < 160 || !scale.is_finite() || scale < 0.5 {
            return Err(RadialError::Invalid(crate::RuleKind::RadialCanvasSize));
        }
        let width = (size.0 as f64 * f64::from(scale)).round();
        let height = (size.1 as f64 * f64::from(scale)).round();
        let limit = f64::from(self.gpu.device().limits().max_texture_dimension_2d);
        if width > limit || height > limit {
            return Err(RadialError::Invalid(crate::RuleKind::RadialTextureLimit));
        }
        Ok((width as u32, height as u32))
    }

    /// The GPU inputs of one frame of layout `l`: its `globals`, its sectors
    /// and the bind group that reads them with the label texture `labels`.
    pub(super) fn bind_frame(
        &self,
        globals: &Globals,
        l: &layout::Layout,
        labels: &TrackedTexture,
    ) -> Result<(TrackedBuffer, TrackedBuffer, wgpu::BindGroup), RadialError> {
        let device = self.gpu.device();
        // gpu-alloc: Uniform
        let uniform = gpu_created(device, "radial globals", || {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("radial globals"),
                contents: bytemuck::bytes_of(globals),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        })?;
        let uniform = TrackedBuffer::new(self.gpu.ledger(), GpuResourceKind::Uniform, uniform);
        // gpu-alloc: FieldTable
        let sectors = gpu_created(device, "radial sectors", || {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("radial sectors"),
                contents: bytemuck::cast_slice(&l.sectors),
                usage: wgpu::BufferUsages::STORAGE,
            })
        })?;
        let sectors = TrackedBuffer::new(self.gpu.ledger(), GpuResourceKind::FieldTable, sectors);
        let bind_group = self.frame_bind_group(&uniform, &sectors, labels);
        Ok((uniform, sectors, bind_group))
    }

    /// The bind group of one frame: its globals, its sectors and its label
    /// texture.
    fn frame_bind_group(
        &self,
        uniform: &TrackedBuffer,
        sectors: &TrackedBuffer,
        labels: &TrackedTexture,
    ) -> wgpu::BindGroup {
        self.gpu
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("radial frame"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: sectors.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(
                            &labels.create_view(&Default::default()),
                        ),
                    },
                ],
            })
    }

    /// The bind group that draws `frame` over `background` instead of its
    /// chart's (#313): a uniform of the frame's globals with the background
    /// replaced, next to the frame's own sectors and label texture. The
    /// frame, the renderer's cache and the chart are not changed. `None`
    /// for [`RadialExportBackground::Document`], which draws the frame as
    /// it is.
    pub(super) fn export_bind_group(
        &self,
        frame: &RadialFrame,
        background: RadialExportBackground,
    ) -> Result<Option<(TrackedBuffer, wgpu::BindGroup)>, RadialError> {
        let color = match background {
            RadialExportBackground::Document => return Ok(None),
            RadialExportBackground::Transparent => crate::Color::new(0.0, 0.0, 0.0, 0.0),
            RadialExportBackground::Color(color) => color,
        };
        let components = [color.r, color.g, color.b, color.a];
        if !components
            .iter()
            .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
        {
            return Err(RadialError::Invalid(
                crate::RuleKind::RadialTextOrBackground,
            ));
        }
        self.admit(std::mem::size_of::<Globals>() as u64)?;
        let globals = Globals {
            background: components,
            ..frame.globals
        };
        let device = self.gpu.device();
        // gpu-alloc: Uniform
        let uniform = gpu_created(device, "radial export globals", || {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("radial export globals"),
                contents: bytemuck::bytes_of(&globals),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        })?;
        let uniform = TrackedBuffer::new(self.gpu.ledger(), GpuResourceKind::Uniform, uniform);
        let bind_group = self.frame_bind_group(&uniform, &frame.sectors, &frame._annotations);
        Ok(Some((uniform, bind_group)))
    }
}
