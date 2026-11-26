//! Creating a radial renderer, and its memory budget.

use super::*;

impl RadialRenderer {
    /// Initialize browser pipelines through WebGPU's asynchronous compiler.
    /// Uses the checked production pipeline descriptors before publishing the
    /// wgpu-owned pipeline. Native hosts use the ordinary synchronous constructor.
    pub async fn try_new_async(
        gpu: RendererDevice,
        format: wgpu::TextureFormat,
    ) -> Result<Self, RadialError> {
        #[cfg(target_arch = "wasm32")]
        crate::browser_compile::render(
            gpu.device(),
            &[(
                RADIAL_SHADER.source,
                &[("radial surfaces", "radial surfaces")],
            )],
            format,
            1,
            None,
            &mut |_| {},
        )
        .await
        .map_err(RadialError::Gpu)?;
        #[cfg(target_arch = "wasm32")]
        crate::browser_compile::compute(
            gpu.device(),
            "renderer.radial_pick",
            RADIAL_PICK_SHADER.source,
            &[("radial pick", "pick")],
            &mut |_| {},
        )
        .await
        .map_err(RadialError::Gpu)?;
        Self::try_new(gpu, format)
    }

    /// Share the host device/queue. Supports RGBA8/BGRA8, linear or sRGB targets.
    /// Charges the bundle's ledger and spends its budget, which every renderer
    /// built from `gpu` shares (#160).
    pub fn try_new(gpu: RendererDevice, format: wgpu::TextureFormat) -> Result<Self, RadialError> {
        if !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            return Err(RadialError::Invalid(crate::RuleKind::RadialTargetFormat));
        }
        // A lost device refuses here (#469), as every later call would.
        gpu.health().check().map_err(radial_gpu_error)?;
        let device = gpu.device();
        let layout = crate::gpu_compile::bind_group_layout(
            &device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("radial chart"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(112),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                std::mem::size_of::<Sector>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            },
        );
        let pl = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("radial chart"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            },
        );
        let shader = crate::gpu_compile::shader_module(
            &device,
            wgpu::ShaderModuleDescriptor {
                label: Some("radial surfaces"),
                source: wgpu::ShaderSource::Wgsl(RADIAL_SHADER.source.into()),
            },
        );
        let pipeline = crate::gpu_compile::render_pipeline(
            &device,
            &wgpu::RenderPipelineDescriptor {
                label: Some("radial surfaces"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            },
        );
        // The pick module's own bindings: its query, the frame's sectors and
        // its result.
        let pick_layout = crate::gpu_compile::bind_group_layout(
            &device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("radial pick"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                                RadialPickQueryGpu,
                            >()
                                as u64),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                std::mem::size_of::<Sector>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(RADIAL_PICK_RESULT_BYTES),
                        },
                        count: None,
                    },
                ],
            },
        );
        let pick_pipeline_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("radial pick"),
                bind_group_layouts: &[Some(&pick_layout)],
                immediate_size: 0,
            },
        );
        let pick_shader = crate::gpu_compile::shader_module(
            &device,
            wgpu::ShaderModuleDescriptor {
                label: Some("radial pick"),
                source: wgpu::ShaderSource::Wgsl(RADIAL_PICK_SHADER.source.into()),
            },
        );
        let pick_pipeline = crate::gpu_compile::compute_pipeline(
            &device,
            &wgpu::ComputePipelineDescriptor {
                label: Some("radial pick"),
                layout: Some(&pick_pipeline_layout),
                module: &pick_shader,
                entry_point: Some("pick"),
                compilation_options: Default::default(),
                cache: None,
            },
        );
        Ok(Self {
            gpu,
            format,
            layout,
            pipeline,
            pick_layout,
            pick_pipeline,
            cached: None,
            annotations_cached: None,
            annotation_layer: crate::annotation::RadialAnnotationCache::new(format),
        })
    }
}

impl RadialRenderer {
    /// Accounted live resources plus resources awaiting submission
    /// completion, for every renderer built from this device bundle.
    pub fn gpu_memory_usage(&self) -> GpuMemoryUsage {
        self.gpu.ledger().snapshot()
    }
    /// The device bundle's memory budget, if any.
    pub fn memory_budget(&self) -> Option<u64> {
        self.gpu.ledger().budget()
    }
    /// Set the device bundle's memory budget, as
    /// [`crate::Renderer::set_memory_budget`] does: `None` leaves device
    /// limits as the only bound. Never evicts existing frames or resources.
    /// Returns the current total when the new ceiling is already below it.
    #[must_use = "a returned total means the new ceiling is already exceeded"]
    pub fn set_memory_budget(&mut self, budget: Option<u64>) -> Option<u64> {
        let ledger = self.gpu.ledger();
        ledger.set_budget(budget);
        let ceiling = budget?;
        let current = ledger.total_bytes();
        (current > ceiling).then_some(current)
    }
    pub(super) fn admit(&self, extra: u64) -> Result<(), RadialError> {
        let ledger = self.gpu.ledger();
        let needed = ledger.total_bytes().saturating_add(extra);
        let limit = ledger.budget_limit();
        if needed > limit {
            Err(RadialError::Budget { needed, limit })
        } else {
            Ok(())
        }
    }
}
