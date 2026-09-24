//! GPU-only subpixel histogram envelope, prepared before painting.
use super::*;
use crate::gpu_memory::{
    ChargeTally, GpuLedger, GpuResourceKind, SharedCharge, charged_buffer, charged_buffer_init,
};
use crate::streaming::StreamError;
use std::sync::Arc;

pub(crate) struct Pipelines {
    compute: wgpu::ComputePipeline,
    render: wgpu::RenderPipeline,
    compute_layout: wgpu::BindGroupLayout,
    render_layout: wgpu::BindGroupLayout,
    map_layout: wgpu::BindGroupLayout,
    stream_reduce: wgpu::ComputePipeline,
    stream_merge: wgpu::ComputePipeline,
    stream_render: wgpu::RenderPipeline,
    stream_mapped_bars: wgpu::RenderPipeline,
    stream_reduce_layout: wgpu::BindGroupLayout,
    stream_merge_layout: wgpu::BindGroupLayout,
    stream_render_layout: wgpu::BindGroupLayout,
    stream_offset_layout: wgpu::BindGroupLayout,
}

#[derive(Clone)]
pub struct Snapshot {
    pipeline: wgpu::RenderPipeline,
    data: wgpu::BindGroup,
    map: wgpu::BindGroup,
    pixels: u32,
    _charge: SharedCharge,
}

#[derive(Clone)]
pub(crate) struct Persistent {
    pipeline: wgpu::RenderPipeline,
    data: wgpu::BindGroup,
    map: wgpu::BindGroup,
    winners: wgpu::Buffer,
    pixels: u32,
    map_bytes: u64,
    _charge: SharedCharge,
}

pub(crate) struct StreamChunk {
    mapped_bars: wgpu::RenderPipeline,
    offset_bg: wgpu::BindGroup,
    map: wgpu::BindGroup,
    count: u32,
    charged_bytes: u64,
    _charge: SharedCharge,
}

impl Pipelines {
    pub(crate) fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        transform: &wgpu::BindGroupLayout,
        style: &wgpu::BindGroupLayout,
        map: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let make_layout = |compute: bool| {
            let visibility = if compute {
                wgpu::ShaderStages::COMPUTE
            } else {
                wgpu::ShaderStages::VERTEX
            };
            let buffer = |binding, ty| wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            };
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("histogram envelope data"),
                entries: &[
                    buffer(0, wgpu::BufferBindingType::Storage { read_only: true }),
                    buffer(1, wgpu::BufferBindingType::Uniform),
                    buffer(
                        if compute { 3 } else { 2 },
                        wgpu::BufferBindingType::Storage {
                            read_only: !compute,
                        },
                    ),
                ],
            })
        };
        let compute_layout = make_layout(true);
        let render_layout = make_layout(false);
        let layout = |last: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("histogram envelope pipeline layout"),
                bind_group_layouts: &[Some(transform), Some(style), Some(map), Some(last)],
                immediate_size: 0,
            })
        };
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("histogram pixel maxima"),
            layout: Some(&layout(&compute_layout)),
            module: shader,
            entry_point: Some("reduce_bar_envelope"),
            compilation_options: Default::default(),
            cache: None,
        });
        let render = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("histogram envelope fill"),
            layout: Some(&layout(&render_layout)),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_bar_envelope"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: multisample_state(samples),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_bar_envelope"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let stream_buffer = |binding, read_only, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let stream_uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let stream_layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let compute_stage = wgpu::ShaderStages::COMPUTE;
        let vertex_stage = wgpu::ShaderStages::VERTEX;
        let stream_reduce_layout = stream_layout(
            "stream histogram local winner data",
            &[
                stream_buffer(0, true, compute_stage),
                stream_uniform(1, compute_stage),
                stream_buffer(3, false, compute_stage),
                stream_uniform(6, compute_stage),
            ],
        );
        let stream_merge_layout = stream_layout(
            "stream histogram merge data",
            &[
                stream_buffer(0, true, compute_stage),
                stream_uniform(1, compute_stage),
                stream_buffer(3, false, compute_stage),
                stream_buffer(4, false, compute_stage),
                stream_uniform(6, compute_stage),
            ],
        );
        let stream_render_layout = stream_layout(
            "stream histogram persistent render data",
            &[stream_buffer(5, true, vertex_stage)],
        );
        let stream_offset_layout = stream_layout(
            "stream histogram full-bar offset",
            &[stream_uniform(6, vertex_stage)],
        );
        let stream_compute = |entry, data_layout: &wgpu::BindGroupLayout| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout(data_layout)),
                module: shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let stream_reduce = stream_compute("reduce_stream_bar_envelope", &stream_reduce_layout);
        let stream_merge = stream_compute("merge_stream_bar_envelope", &stream_merge_layout);
        let stream_render = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("stream histogram persistent envelope"),
            layout: Some(&layout(&stream_render_layout)),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_stream_bar_envelope"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: multisample_state(samples),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_bar_envelope"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let stream_mapped_bars = create_stream_mapped_bar_pipeline(
            device,
            shader,
            transform,
            style,
            map,
            &stream_offset_layout,
            format,
            samples,
        );
        Self {
            compute,
            render,
            compute_layout,
            render_layout,
            map_layout: map.clone(),
            stream_reduce,
            stream_merge,
            stream_render,
            stream_mapped_bars,
            stream_reduce_layout,
            stream_merge_layout,
            stream_render_layout,
            stream_offset_layout,
        }
    }

    pub(crate) fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &Arc<GpuLedger>,
        pool: &wgpu::Buffer,
        edges: ColumnHandle,
        values: ColumnHandle,
        pixels: u32,
        transform: &wgpu::BindGroup,
        style: &wgpu::BindGroup,
        style_map: Option<&wgpu::BindGroup>,
    ) -> Snapshot {
        let tally = ChargeTally::default();
        let pixels = pixels.max(1);
        let count = bar_instance_count(edges.len_values, values.len_values);
        let params = [
            (edges.byte_range().start / 8) as u32,
            (values.byte_range().start / 8) as u32,
            count,
            pixels,
        ];
        let uniform = charged_buffer_init(
            &tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("histogram envelope params"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let winners = charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("histogram envelope winners"),
                size: u64::from(pixels) * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
        );
        let bg = |layout: &wgpu::BindGroupLayout, binding| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("histogram envelope data"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: pool.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding,
                        resource: winners.as_entire_binding(),
                    },
                ],
            })
        };
        let compute_data = bg(&self.compute_layout, 3);
        let data = bg(&self.render_layout, 2);
        let map = style_map
            .cloned()
            .unwrap_or_else(|| create_bar_style_map(device, &self.map_layout, &[]).bind_group);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("histogram envelope preparation"),
        });
        if count != 0 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("histogram envelope reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, transform, &[]);
            pass.set_bind_group(1, style, &[]);
            pass.set_bind_group(2, &map, &[]);
            pass.set_bind_group(3, &compute_data, &[]);
            let groups = count.div_ceil(64);
            pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535), 1);
        }
        queue.submit([encoder.finish()]);
        Snapshot {
            pipeline: self.render.clone(),
            data,
            map,
            pixels,
            _charge: Arc::new(tally.into_charge(ledger, GpuResourceKind::Uniform)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_stream_chunk(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        ledger: &Arc<GpuLedger>,
        max_renderer_bytes: u64,
        pool_bytes: u64,
        persistent: &Persistent,
        work: &wgpu::Buffer,
        edges: ColumnHandle,
        values: ColumnHandle,
        global_bin_start: u32,
        transform: &wgpu::BindGroup,
        style: &wgpu::BindGroup,
    ) -> Result<StreamChunk, StreamError> {
        let count_usize = edges.len_values.saturating_sub(1).min(values.len_values);
        let count = u32::try_from(count_usize).map_err(|_| StreamError::TooLarge)?;
        if count > 0 {
            global_bin_start
                .checked_add(count - 1)
                .ok_or(StreamError::Overflow)?;
        }
        let edges_range = edges.byte_range();
        let values_range = values.byte_range();
        if edges_range.start % 8 != 0
            || values_range.start % 8 != 0
            || edges_range.end > work.size()
            || values_range.end > work.size()
        {
            return Err(StreamError::InvalidRange);
        }
        let edge_base = u32::try_from(edges_range.start / 8).map_err(|_| StreamError::TooLarge)?;
        let value_base =
            u32::try_from(values_range.start / 8).map_err(|_| StreamError::TooLarge)?;
        if count > 0 {
            edge_base.checked_add(count).ok_or(StreamError::TooLarge)?;
            value_base
                .checked_add(count - 1)
                .ok_or(StreamError::TooLarge)?;
        }
        let max_groups = device.limits().max_compute_workgroups_per_dimension;
        let reduce_groups = count.div_ceil(64);
        if reduce_groups.min(65535) > max_groups
            || reduce_groups.div_ceil(65535) > max_groups
            || persistent.pixels.div_ceil(64) > max_groups
        {
            return Err(StreamError::TooLarge);
        }
        let local_bytes = u64::from(persistent.pixels)
            .checked_mul(4)
            .and_then(|n| n.checked_add(32))
            .ok_or(StreamError::Overflow)?;
        if u64::from(persistent.pixels) * 4
            > u64::from(device.limits().max_storage_buffer_binding_size)
        {
            return Err(StreamError::TooLarge);
        }
        check_stream_budget(ledger, max_renderer_bytes, pool_bytes, local_bytes)?;
        let tally = ChargeTally::default();
        let params = [edge_base, value_base, count, persistent.pixels];
        let uniform = charged_buffer_init(
            &tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("stream histogram local params"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let offset = [global_bin_start, 0, 0, 0];
        let offset_uniform = charged_buffer_init(
            &tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("stream histogram global bin offset"),
                contents: bytemuck::cast_slice(&offset),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let local = charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("stream histogram local atomic winners"),
                size: u64::from(persistent.pixels) * 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        let reduce_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stream histogram local reduction data"),
            layout: &self.stream_reduce_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: work.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: local.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: offset_uniform.as_entire_binding(),
                },
            ],
        });
        let merge_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stream histogram serial merge data"),
            layout: &self.stream_merge_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: work.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: local.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: persistent.winners.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: offset_uniform.as_entire_binding(),
                },
            ],
        });
        let offset_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stream histogram mapped full-bin offset"),
            layout: &self.stream_offset_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 6,
                resource: offset_uniform.as_entire_binding(),
            }],
        });
        encoder.clear_buffer(&local, 0, None);
        if count > 0 {
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("stream histogram local reduction"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.stream_reduce);
                pass.set_bind_group(0, transform, &[]);
                pass.set_bind_group(1, style, &[]);
                pass.set_bind_group(2, &persistent.map, &[]);
                pass.set_bind_group(3, &reduce_bg, &[]);
                pass.dispatch_workgroups(
                    reduce_groups.min(65535),
                    reduce_groups.div_ceil(65535),
                    1,
                );
            }
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("stream histogram serial pixel merge"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.stream_merge);
                pass.set_bind_group(0, transform, &[]);
                pass.set_bind_group(1, style, &[]);
                pass.set_bind_group(2, &persistent.map, &[]);
                pass.set_bind_group(3, &merge_bg, &[]);
                pass.dispatch_workgroups(persistent.pixels.div_ceil(64), 1, 1);
            }
        }
        Ok(StreamChunk {
            mapped_bars: self.stream_mapped_bars.clone(),
            offset_bg,
            map: persistent.map.clone(),
            count,
            charged_bytes: local_bytes,
            _charge: Arc::new(tally.into_charge(ledger, GpuResourceKind::StreamingUpload)),
        })
    }
}

impl Snapshot {
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        transform: &wgpu::BindGroup,
        style: &wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, transform, &[]);
        pass.set_bind_group(1, style, &[]);
        pass.set_bind_group(2, &self.map, &[]);
        pass.set_bind_group(3, &self.data, &[]);
        pass.draw(0..6, 0..self.pixels);
    }
}

impl Persistent {
    pub(crate) fn new(
        pipelines: &Pipelines,
        device: &wgpu::Device,
        ledger: &Arc<GpuLedger>,
        pixels: u32,
        max_renderer_bytes: u64,
        pool_bytes: u64,
        style_map: Option<&wgpu::BindGroup>,
    ) -> Result<Self, StreamError> {
        let pixels = pixels.max(1);
        let bytes = u64::from(pixels)
            .checked_mul(16)
            .ok_or(StreamError::Overflow)?;
        let map_bytes = if style_map.is_none() {
            (std::mem::size_of::<BarStyleSlotGpu>()
                + std::mem::size_of::<BarStyleOverrideGpu>()
                + std::mem::size_of::<BarStyleMapMeta>()) as u64
        } else {
            0
        };
        if bytes > device.limits().max_buffer_size
            || bytes > u64::from(device.limits().max_storage_buffer_binding_size)
            || pixels.div_ceil(64) > device.limits().max_compute_workgroups_per_dimension
        {
            return Err(StreamError::TooLarge);
        }
        check_stream_budget(
            ledger,
            max_renderer_bytes,
            pool_bytes,
            bytes.checked_add(map_bytes).ok_or(StreamError::Overflow)?,
        )?;
        let tally = ChargeTally::default();
        let winners = charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("stream histogram persistent pixel winners"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | if cfg!(test) {
                        wgpu::BufferUsages::COPY_SRC
                    } else {
                        wgpu::BufferUsages::empty()
                    },
                mapped_at_creation: false,
            },
        );
        let data = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stream histogram persistent render data"),
            layout: &pipelines.stream_render_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 5,
                resource: winners.as_entire_binding(),
            }],
        });
        let map = style_map
            .cloned()
            .unwrap_or_else(|| create_bar_style_map(device, &pipelines.map_layout, &[]).bind_group);
        // The empty-map helper owns one padding style, one padding override and
        // one metadata uniform. This persistent owner charges their exact bytes.
        tally.add(map_bytes);
        Ok(Self {
            pipeline: pipelines.stream_render.clone(),
            data,
            map,
            winners,
            pixels,
            map_bytes,
            _charge: Arc::new(tally.into_charge(ledger, GpuResourceKind::StreamingUpload)),
        })
    }

    pub(crate) fn record_clear(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.winners, 0, None);
    }

    /// The caller chooses whether this is the current D overlay or the sole
    /// final P bake; neither draw mutates the stored winners.
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        transform: &wgpu::BindGroup,
        style: &wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, transform, &[]);
        pass.set_bind_group(1, style, &[]);
        pass.set_bind_group(2, &self.map, &[]);
        pass.set_bind_group(3, &self.data, &[]);
        pass.draw(0..6, 0..self.pixels);
    }

    pub(crate) fn charged_bytes(&self) -> u64 {
        u64::from(self.pixels) * 16 + self.map_bytes
    }
}

impl StreamChunk {
    /// This is only needed for sparse mapped histograms. Unmapped full bins
    /// still use the resident bar pipeline with local work-buffer spans.
    pub(crate) fn draw_full_mapped_bars(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        work: &wgpu::Buffer,
        edges: ColumnHandle,
        values: ColumnHandle,
        transform: &wgpu::BindGroup,
        style: &wgpu::BindGroup,
    ) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.mapped_bars);
        pass.set_bind_group(0, transform, &[]);
        pass.set_bind_group(1, style, &[]);
        pass.set_bind_group(2, &self.map, &[]);
        pass.set_bind_group(3, &self.offset_bg, &[]);
        let edge_range = edges.byte_range();
        pass.set_vertex_buffer(0, work.slice(edge_range.clone()));
        pass.set_vertex_buffer(
            1,
            work.slice((edge_range.start + crate::data::COLUMN_VALUE_BYTES as u64)..edge_range.end),
        );
        pass.set_vertex_buffer(2, work.slice(values.byte_range()));
        pass.draw(0..BAR_VERTICES_PER_INSTANCE, 0..self.count);
    }

    pub(crate) fn charged_bytes(&self) -> u64 {
        self.charged_bytes
    }
}

fn check_stream_budget(
    ledger: &Arc<GpuLedger>,
    max_renderer_bytes: u64,
    pool_bytes: u64,
    additional: u64,
) -> Result<(), StreamError> {
    if pool_bytes
        .checked_add(ledger.total_bytes())
        .and_then(|n| n.checked_add(additional))
        .ok_or(StreamError::Overflow)?
        > max_renderer_bytes
    {
        return Err(StreamError::TooLarge);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_stream_mapped_bar_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform: &wgpu::BindGroupLayout,
    style: &wgpu::BindGroupLayout,
    map: &wgpu::BindGroupLayout,
    offset: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("stream histogram mapped full-bin pipeline layout"),
        bind_group_layouts: &[Some(transform), Some(style), Some(map), Some(offset)],
        immediate_size: 0,
    });
    let stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
    const ATTR0: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    }];
    const ATTR1: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 1,
    }];
    const ATTR2: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 2,
    }];
    let buffers = [
        Some(wgpu::VertexBufferLayout {
            array_stride: stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR0,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR1,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR2,
        }),
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("stream histogram mapped full bins"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_stream_envelope_mapped_bars"),
            compilation_options: Default::default(),
            buffers: &buffers,
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: multisample_state(samples),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    fn gpu() -> (wgpu::Device, wgpu::Queue) {
        let instance = create_instance();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("stream histogram GPU test requires an adapter");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("stream histogram GPU device creation failed")
    }

    fn wait(device: &wgpu::Device) {
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("stream histogram GPU submission failed");
    }

    fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
        let (tx, rx) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        wait(device);
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .expect("stream histogram GPU readback callback missing")
            .expect("stream histogram GPU readback failed");
        let bytes = buffer
            .slice(..)
            .get_mapped_range()
            .expect("stream histogram mapped range failed")
            .to_vec();
        buffer.unmap();
        bytes
    }

    fn setup(
        device: &wgpu::Device,
        pixels: u32,
        overrides: &[BarStyleOverrideGpu],
    ) -> (Pipelines, wgpu::BindGroup, wgpu::BindGroup, wgpu::BindGroup) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stream histogram test shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("bar_columnar.wgsl").into()),
        });
        let transform_bgl = create_scatter_transform_bind_group_layout(device);
        let style_bgl = create_style_bind_group_layout(device);
        let map_bgl = create_per_point_style_map_bind_group_layout(device);
        let pipelines = Pipelines::new(
            device,
            &shader,
            &transform_bgl,
            &style_bgl,
            &map_bgl,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
        );
        let transform = ScatterTransform {
            data_min: [0.0, 0.0],
            data_max: [pixels as f32, 10.0],
            data_min_lo: [0.0; 2],
            data_max_lo: [0.0; 2],
            scale_log: [0.0; 2],
            pixel_to_ndc: [2.0 / pixels as f32, 2.0 / 8.0],
            data_to_panel_scale: [1.0; 2],
            data_to_panel_offset: [0.0; 2],
            style_params: [[0.0; 4]; 3],
        };
        let transform_uniform = create_scatter_transform_uniform_buffer(device, &transform);
        let transform_bg =
            create_scatter_transform_bind_group(device, &transform_bgl, &transform_uniform);
        let mut bar_style = PrimitiveStyle::from_color(Color::new(0.0, 0.0, 1.0, 1.0));
        bar_style.shape_id = 0;
        bar_style.line_width_px = 0.0;
        bar_style.cap_half_px = 0.0;
        bar_style.cap_width_px = 1.0;
        bar_style.dash[0] = [0.0; 4];
        bar_style.dash[1] = [0.0; 4];
        let style_uniform = create_style_uniform_buffer(device, &bar_style);
        let style_bg = create_style_bind_group(device, &style_bgl, &style_uniform);
        let map_bg = create_bar_style_map(device, &map_bgl, overrides).bind_group;
        (pipelines, transform_bg, style_bg, map_bg)
    }

    fn target(device: &wgpu::Device) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("stream histogram style output"),
            size: wgpu::Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn rendered_colors(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Vec<[u8; 4]> {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stream histogram style readback"),
            size: 256 * 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(8),
                },
            },
            texture.size(),
        );
        queue.submit([encoder.finish()]);
        let bytes = read(device, &staging);
        let mut colors = Vec::with_capacity(64);
        for row in 0..8 {
            for column in 0..8 {
                let base = row * 256 + column * 4;
                colors.push(bytes[base..base + 4].try_into().unwrap());
            }
        }
        colors
    }

    #[test]
    fn persistent_winner_survives_reused_work_and_global_tie() {
        let (device, queue) = gpu();
        let (pipelines, transform, style, map) = setup(&device, 8, &[]);
        let ledger = Arc::new(GpuLedger::new());
        assert_eq!(
            Persistent::new(&pipelines, &device, &ledger, 8, 128, 1, Some(&map)).err(),
            Some(StreamError::TooLarge),
            "resident pool bytes must participate in the renderer cap"
        );
        let persistent =
            Persistent::new(&pipelines, &device, &ledger, 8, u64::MAX, 0, Some(&map)).unwrap();
        assert_eq!(persistent.charged_bytes(), 128);
        let empty_map_ledger = Arc::new(GpuLedger::new());
        assert_eq!(
            Persistent::new(&pipelines, &device, &empty_map_ledger, 8, 255, 0, None).err(),
            Some(StreamError::TooLarge),
            "the unmapped padding style map must be inside admission"
        );
        let empty_map =
            Persistent::new(&pipelines, &device, &empty_map_ledger, 8, 256, 0, None).unwrap();
        assert_eq!(empty_map.charged_bytes(), 256);
        assert_eq!(empty_map_ledger.total_bytes(), 256);
        drop(empty_map);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("stream histogram test clear"),
        });
        persistent.record_clear(&mut encoder);
        queue.submit([encoder.finish()]);
        wait(&device);
        let work = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stream histogram reused work"),
            size: 40,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let edges = ColumnHandle {
            generation: 0,
            offset: 0,
            byte_size: 24,
            len_values: 3,
        };
        let values = ColumnHandle {
            generation: 0,
            offset: 24,
            byte_size: 16,
            len_values: 2,
        };
        assert_eq!(
            pipelines
                .record_stream_chunk(
                    &device,
                    &mut device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None }),
                    &ledger,
                    192,
                    1,
                    &persistent,
                    &work,
                    edges,
                    values,
                    100,
                    &transform,
                    &style,
                )
                .err(),
            Some(StreamError::TooLarge),
            "local atomic and both uniforms must be inside the renderer cap"
        );
        let cases: [(u32, [[f32; 2]; 5], [u32; 4]); 4] = [
            (
                100u32,
                [[0.1, 0.0], [0.2, 0.0], [0.3, 0.0], [5.0, 0.0], [7.0, 0.0]],
                [1u32, 101, 7.0f32.to_bits(), 0],
            ),
            (
                102u32,
                [[0.3, 0.0], [0.4, 0.0], [0.5, 0.0], [7.0, 0.0], [6.0, 0.0]],
                [1u32, 101, 7.0f32.to_bits(), 0],
            ),
            (
                104u32,
                [[0.5, 0.0], [0.6, 0.0], [0.7, 0.0], [7.0, 0.25], [6.0, 0.0]],
                [1u32, 104, 7.0f32.to_bits(), 0.25f32.to_bits()],
            ),
            (
                106u32,
                [[0.5, 0.0], [0.6, 0.0], [0.7, 0.0], [8.0, 0.0], [0.0, 0.0]],
                [1u32, 106, 8.0f32.to_bits(), 0],
            ),
        ];
        for (bin_start, pairs, expected) in cases {
            queue.write_buffer(&work, 0, bytemuck::cast_slice(&pairs));
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("stream histogram test chunk"),
            });
            let chunk = pipelines
                .record_stream_chunk(
                    &device,
                    &mut encoder,
                    &ledger,
                    u64::MAX,
                    0,
                    &persistent,
                    &work,
                    edges,
                    values,
                    bin_start,
                    &transform,
                    &style,
                )
                .unwrap();
            assert_eq!(chunk.charged_bytes(), 8 * 4 + 32);
            queue.submit([encoder.finish()]);
            wait(&device);
            drop(chunk);
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("stream histogram test winner readback"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("stream histogram test read winner"),
            });
            encoder.copy_buffer_to_buffer(&persistent.winners, 0, &staging, 0, 16);
            queue.submit([encoder.finish()]);
            let got: [u32; 4] = bytemuck::cast_slice(&read(&device, &staging))[..4]
                .try_into()
                .unwrap();
            assert_eq!(got, expected);
        }
        assert_eq!(
            pipelines
                .record_stream_chunk(
                    &device,
                    &mut device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None }),
                    &ledger,
                    u64::MAX,
                    0,
                    &persistent,
                    &work,
                    edges,
                    values,
                    u32::MAX,
                    &transform,
                    &style,
                )
                .err(),
            Some(StreamError::Overflow)
        );
        let empty_edges = ColumnHandle {
            generation: 0,
            offset: 0,
            byte_size: 8,
            len_values: 1,
        };
        let empty_values = ColumnHandle {
            generation: 0,
            offset: 24,
            byte_size: 0,
            len_values: 0,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("stream histogram empty bin chunk"),
        });
        let empty = pipelines
            .record_stream_chunk(
                &device,
                &mut encoder,
                &ledger,
                u64::MAX,
                0,
                &persistent,
                &work,
                empty_edges,
                empty_values,
                u32::MAX,
                &transform,
                &style,
            )
            .unwrap();
        assert_eq!(empty.count, 0);
        queue.submit([encoder.finish()]);
        wait(&device);
        drop(empty);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("stream histogram reset after series"),
        });
        persistent.record_clear(&mut encoder);
        queue.submit([encoder.finish()]);
        wait(&device);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stream histogram cleared winner readback"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("stream histogram read cleared winner"),
        });
        encoder.copy_buffer_to_buffer(&persistent.winners, 0, &staging, 0, 16);
        queue.submit([encoder.finish()]);
        assert_eq!(read(&device, &staging), vec![0; 16]);
        queue.write_buffer(
            &work,
            0,
            bytemuck::cast_slice(&[
                [0.1f32, 0.0],
                [0.2, 0.0],
                [0.3, 0.0],
                [-3.0, 0.0],
                [-2.0, 0.0],
            ]),
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("stream histogram negative winner"),
        });
        let negative = pipelines
            .record_stream_chunk(
                &device,
                &mut encoder,
                &ledger,
                u64::MAX,
                0,
                &persistent,
                &work,
                edges,
                values,
                200,
                &transform,
                &style,
            )
            .unwrap();
        encoder.copy_buffer_to_buffer(&persistent.winners, 0, &staging, 0, 16);
        queue.submit([encoder.finish()]);
        drop(negative);
        let got: [u32; 4] = bytemuck::cast_slice(&read(&device, &staging))[..4]
            .try_into()
            .unwrap();
        assert_eq!(got, [1, 201, (-2.0f32).to_bits(), 0]);
    }

    #[test]
    fn mapped_full_bars_and_persistent_overlay_use_global_bin_styles() {
        let (device, queue) = gpu();
        let make_override = |bin_index, color| BarStyleOverrideGpu {
            bin_index,
            _pad: [0; 3],
            fill_color_premul: color,
            border_color_premul: [0.0; 4],
            params: [0.0, 0.0, 0.0, 1.0],
        };
        let overrides = [
            make_override(100, [0.0, 1.0, 0.0, 1.0]),
            make_override(101, [1.0, 0.0, 0.0, 1.0]),
        ];
        let (pipelines, transform, style, map) = setup(&device, 8, &overrides);
        let ledger = Arc::new(GpuLedger::new());
        let persistent =
            Persistent::new(&pipelines, &device, &ledger, 8, u64::MAX, 0, Some(&map)).unwrap();
        let work = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stream histogram mapped reused work"),
            size: 24,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let edges = ColumnHandle {
            generation: 0,
            offset: 0,
            byte_size: 16,
            len_values: 2,
        };
        let values = ColumnHandle {
            generation: 0,
            offset: 16,
            byte_size: 8,
            len_values: 1,
        };
        let full: [[f32; 2]; 3] = [[1.0, 0.0], [2.0, 0.0], [5.0, 0.0]];
        queue.write_buffer(&work, 0, bytemuck::cast_slice(&full));
        let texture = target(&device);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("stream histogram mapped full bin"),
        });
        persistent.record_clear(&mut encoder);
        let chunk = pipelines
            .record_stream_chunk(
                &device,
                &mut encoder,
                &ledger,
                u64::MAX,
                0,
                &persistent,
                &work,
                edges,
                values,
                100,
                &transform,
                &style,
            )
            .unwrap();
        {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stream histogram mapped full-bin draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            chunk.draw_full_mapped_bars(&mut pass, &work, edges, values, &transform, &style);
        }
        let colors = rendered_colors(&device, &queue, encoder, &texture);
        assert!(
            colors.contains(&[0, 255, 0, 255]),
            "global bin 100 green full bar absent"
        );
        assert!(
            !colors.contains(&[0, 0, 255, 255]),
            "local-index blue style leaked"
        );
        drop(chunk);
        let narrow: [[f32; 2]; 3] = [[0.1, 0.0], [0.2, 0.0], [5.0, 0.0]];
        queue.write_buffer(&work, 0, bytemuck::cast_slice(&narrow));
        let texture = target(&device);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("stream histogram persistent mapped overlay"),
        });
        let chunk = pipelines
            .record_stream_chunk(
                &device,
                &mut encoder,
                &ledger,
                u64::MAX,
                0,
                &persistent,
                &work,
                edges,
                values,
                101,
                &transform,
                &style,
            )
            .unwrap();
        {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stream histogram persistent mapped draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            persistent.draw(&mut pass, &transform, &style);
        }
        let colors = rendered_colors(&device, &queue, encoder, &texture);
        assert!(
            colors.contains(&[255, 0, 0, 255]),
            "global bin 101 red winner absent"
        );
        assert!(
            !colors.contains(&[0, 0, 255, 255]),
            "local-index blue winner style leaked"
        );
        drop(chunk);
    }
}
