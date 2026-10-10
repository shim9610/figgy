//! Viewport-bounded second-pass reveal. The first-pass preview and the exact
//! background are captured without decorations. Exact data accumulates normally;
//! only changed pixels replace the preview, never blend with
//! it. Overlapping unfinished geometry in those pixels is still provisional.
use super::*;

pub(super) struct ReplayTransfer {
    compute_layout: wgpu::BindGroupLayout,
    restore_layout: wgpu::BindGroupLayout,
    compute: wgpu::ComputePipeline,
    restore: wgpu::RenderPipeline,
}

impl ReplayTransfer {
    pub(super) fn new(
        device: &wgpu::Device,
        spec: StreamSurfaceSpec,
    ) -> Result<Self, StreamSurfaceError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let multisampled = spec.sample_count > 1;
            let texture_type = if multisampled { "texture_multisampled_2d<f32>" } else { "texture_2d<f32>" };
            let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("stream replay tile mask"),
                source: wgpu::ShaderSource::Wgsl(format!(
                    "@group(0) @binding(0) var exact: {texture_type};\n@group(0) @binding(1) var baseline: {texture_type};\nconst SAMPLE_COUNT: u32 = {}u;\n{}",
                    spec.sample_count, include_str!("stream_replay_mask.wgsl")).into()),
            });
            let entry = if multisampled {
                "@fragment fn restore(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @location(0) vec4<f32> { return restore_pixel(vec2<i32>(p.xy), i32(s)); }"
            } else {
                "@fragment fn restore(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> { return restore_pixel(vec2<i32>(p.xy), 0); }"
            };
            let restore_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("stream replay preview restoration"),
                source: wgpu::ShaderSource::Wgsl(format!(
                    "@group(0) @binding(2) var preview: {texture_type};\n{}\n{entry}",
                    include_str!("stream_replay_restore.wgsl")).into()),
            });
            let texture = |binding, visibility| wgpu::BindGroupLayoutEntry {
                binding, visibility,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2, multisampled,
                }, count: None,
            };
            let storage = |visibility, read_only| wgpu::BindGroupLayoutEntry {
                binding: 3, visibility,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(4) }, count: None,
            };
            let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("stream replay mask"), entries: &[
                    texture(0,wgpu::ShaderStages::COMPUTE), texture(1,wgpu::ShaderStages::COMPUTE),
                    storage(wgpu::ShaderStages::COMPUTE,false),
                ],
            });
            let restore_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("stream replay restore"), entries: &[
                    texture(2,wgpu::ShaderStages::FRAGMENT), storage(wgpu::ShaderStages::FRAGMENT,true),
                ],
            });
            let layout = |bgl: &wgpu::BindGroupLayout| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("stream replay"), bind_group_layouts: &[Some(bgl)], immediate_size: 0,
            });
            let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("stream replay mask"), layout: Some(&layout(&compute_layout)),
                module: &compute_shader, entry_point: Some("mask"), compilation_options: Default::default(), cache: None,
            });
            let restore = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("stream replay restore"), layout: Some(&layout(&restore_layout)),
                vertex: wgpu::VertexState { module: &restore_shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(), depth_stencil: None,
                multisample: wgpu::MultisampleState { count: spec.sample_count, ..Default::default() },
                fragment: Some(wgpu::FragmentState { module: &restore_shader, entry_point: Some("restore"), compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: spec.format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
                multiview_mask: None, cache: None,
            });
            Self { compute_layout, restore_layout, compute, restore }
        })).map_err(|_| StreamSurfaceError::AllocationFailed)
    }
}

pub(super) struct ReplaySurface {
    preview: TrackedTexture,
    baseline: TrackedTexture,
    mask: TrackedBuffer,
    compute: wgpu::BindGroup,
    restore: wgpu::BindGroup,
}

fn replay_bytes(spec: StreamSurfaceSpec) -> Result<u64, StreamSurfaceError> {
    u64::from(spec.width)
        .checked_mul(u64::from(spec.height))
        .and_then(|n| n.checked_mul(8 * u64::from(spec.sample_count)))
        .and_then(|n| {
            n.checked_add(
                u64::from(spec.width.div_ceil(8)) * u64::from(spec.height.div_ceil(8)) * 8,
            )
        })
        .ok_or(StreamSurfaceError::Overflow)
}

impl StreamSurface {
    /// Optional enhancement: budget failure retains the old atomic handoff.
    /// No buffers/textures scale with the source length or the number of chunks.
    pub(crate) fn begin_replay(
        &mut self,
        device: &wgpu::Device,
        ledger: &Arc<GpuLedger>,
        transfer: &mut StreamTransfer,
        encoder: &mut wgpu::CommandEncoder,
        clear: wgpu::Color,
        mut background: impl FnMut(&mut wgpu::RenderPass<'_>),
        budget: u64,
        pool_bytes: u64,
    ) -> Result<(), StreamSurfaceError> {
        let bytes = replay_bytes(self.spec)?;
        if ledger
            .total_bytes()
            .checked_add(pool_bytes)
            .and_then(|n| n.checked_add(bytes))
            .filter(|n| *n <= budget)
            .is_none()
        {
            return Err(StreamSurfaceError::TooLarge);
        }
        if transfer.replay.is_none() {
            transfer.replay = Some(ReplayTransfer::new(device, self.spec)?);
        }
        let pipelines = transfer.replay.as_ref().unwrap();
        let replay = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let create = |label| {
                // gpu-alloc: caller
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: self.spec.width,
                        height: self.spec.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: self.spec.sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.spec.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                TrackedTexture::new(
                    ledger,
                    if self.spec.sample_count > 1 {
                        GpuResourceKind::MsaaTarget
                    } else {
                        GpuResourceKind::PanelTexture
                    },
                    texture,
                )
            };
            let preview = create("stream replay retained preview");
            let baseline = create("stream replay clean background");
            // gpu-alloc: StreamingUpload
            let mask = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("stream replay tile visibility"),
                size: u64::from(self.spec.width.div_ceil(8))
                    * u64::from(self.spec.height.div_ceil(8))
                    * 8,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mask = TrackedBuffer::new(ledger, GpuResourceKind::StreamingUpload, mask);
            let compute = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("stream replay comparison"),
                layout: &pipelines.compute_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.display_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &baseline.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: mask.as_entire_binding(),
                    },
                ],
            });
            let restore = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("stream replay preview"),
                layout: &pipelines.restore_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(
                            &preview.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: mask.as_entire_binding(),
                    },
                ],
            });
            ReplaySurface {
                preview,
                baseline,
                mask,
                compute,
                restore,
            }
        }))
        .map_err(|_| StreamSurfaceError::AllocationFailed)?;
        for (texture, include_data) in [(&replay.baseline, false), (&replay.preview, true)] {
            let view = texture.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stream replay snapshot without labels"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            background(&mut pass);
            if include_data {
                pass.set_viewport(
                    0.0,
                    0.0,
                    self.spec.width as f32,
                    self.spec.height as f32,
                    0.0,
                    1.0,
                );
                pass.set_scissor_rect(0, 0, self.spec.width, self.spec.height);
                pass.set_pipeline(&transfer.overlay_pipeline);
                pass.set_bind_group(0, &self.source, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        encoder.clear_buffer(&replay.mask, 0, None);
        self.replay = Some(replay);
        Ok(())
    }

    pub(crate) fn has_replay(&self) -> bool {
        self.replay.is_some()
    }
    pub(crate) fn finish_replay(&mut self) {
        self.replay = None;
    }

    pub(super) fn record_replay_restore(
        &self,
        transfer: &StreamTransfer,
        encoder: &mut wgpu::CommandEncoder,
        clip: [u32; 4],
        suffix: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) {
        let replay = self.replay.as_ref().unwrap();
        let pipelines = transfer.replay.as_ref().unwrap();
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("stream replay reveal pixels"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.compute);
            pass.set_bind_group(0, &replay.compute, &[]);
            pass.dispatch_workgroups(self.spec.width.div_ceil(8), self.spec.height.div_ceil(8), 1);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("stream replay retain unfinished pixels then labels"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.display_view,
                depth_slice: None,
                resolve_target: self.resolved_view.as_ref(),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        if clip[2] > 0 && clip[3] > 0 {
            pass.set_scissor_rect(clip[0], clip[1], clip[2], clip[3]);
            pass.set_pipeline(&pipelines.restore);
            pass.set_bind_group(0, &replay.restore, &[]);
            pass.draw(0..3, 0..1);
        }
        suffix(&mut pass);
    }
}

#[cfg(test)]
#[path = "streaming_replay_tests.rs"]
mod tests;
