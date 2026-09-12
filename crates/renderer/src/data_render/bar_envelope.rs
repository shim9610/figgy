//! GPU-only subpixel histogram envelope, prepared before painting.
use super::*;
use crate::gpu_memory::{
    ChargeTally, GpuLedger, GpuResourceKind, SharedCharge, charged_buffer, charged_buffer_init,
};
use std::sync::Arc;

pub(crate) struct Pipelines {
    compute: wgpu::ComputePipeline,
    render: wgpu::RenderPipeline,
    compute_layout: wgpu::BindGroupLayout,
    render_layout: wgpu::BindGroupLayout,
    map_layout: wgpu::BindGroupLayout,
}

#[derive(Clone)]
pub struct Snapshot {
    pipeline: wgpu::RenderPipeline,
    data: wgpu::BindGroup,
    map: wgpu::BindGroup,
    pixels: u32,
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
        Self {
            compute,
            render,
            compute_layout,
            render_layout,
            map_layout: map.clone(),
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
