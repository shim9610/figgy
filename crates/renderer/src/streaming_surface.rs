//! Mutable prefix/display attachments, never an immutable prepared snapshot.

use std::sync::Arc;

use crate::gpu_memory::{GpuLedger, GpuResourceKind, TrackedTexture};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamSurfaceError {
    InvalidOptions,
    Overflow,
    TooLarge,
    AllocationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamSurfaceSpec {
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    pub sample_count: u32,
}

fn validate_format(format: wgpu::TextureFormat, samples: u32) -> Result<(), StreamSurfaceError> {
    if !matches!(
        format,
        wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8UnormSrgb
    ) || !matches!(samples, 1 | 4)
    {
        return Err(StreamSurfaceError::InvalidOptions);
    }
    Ok(())
}

impl StreamSurfaceSpec {
    pub(crate) fn charged_bytes(self) -> Result<u64, StreamSurfaceError> {
        validate_format(self.format, self.sample_count)?;
        if self.width == 0 || self.height == 0 {
            return Err(StreamSurfaceError::InvalidOptions);
        }
        u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|bytes| bytes.checked_mul(4))
            .and_then(|bytes| {
                bytes.checked_mul(
                    u64::from(self.sample_count) * 2 + u64::from(self.sample_count > 1),
                )
            })
            .ok_or(StreamSurfaceError::Overflow)
    }
}

/// Cached by the runtime for a format/sample-count pair, not per update.
pub(crate) struct StreamTransfer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    sample_count: u32,
}

impl StreamTransfer {
    pub(crate) fn matches(&self, format: wgpu::TextureFormat, sample_count: u32) -> bool {
        self.format == format && self.sample_count == sample_count
    }

    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Result<Self, StreamSurfaceError> {
        validate_format(format, sample_count)?;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("stream prefix sample transfer"),
                source: wgpu::ShaderSource::Wgsl(if sample_count > 1 {
                    include_str!("stream_sample_transfer_msaa.wgsl").into()
                } else {
                    include_str!("stream_sample_transfer.wgsl").into()
                }),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("stream prefix sample source"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: sample_count > 1,
                    },
                    count: None,
                }],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("stream prefix sample transfer"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("stream prefix sample transfer"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("transfer"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            Self {
                pipeline,
                layout,
                format,
                sample_count,
            }
        }))
        .map_err(|_| StreamSurfaceError::AllocationFailed)
    }
}

pub(crate) struct StreamSurface {
    spec: StreamSurfaceSpec,
    prefix: TrackedTexture,
    display: TrackedTexture,
    display_view: wgpu::TextureView,
    resolved: Option<TrackedTexture>,
    resolved_view: Option<wgpu::TextureView>,
    source: wgpu::BindGroup,
}

impl StreamSurface {
    /// `pool_bytes` includes ALL live and retired pool bytes outside the ledger.
    /// The caller checks adapter format capabilities before allocating targets.
    pub(crate) fn new(
        device: &wgpu::Device,
        ledger: &Arc<GpuLedger>,
        transfer: &StreamTransfer,
        spec: StreamSurfaceSpec,
        renderer_budget_bytes: u64,
        pool_bytes: u64,
    ) -> Result<Self, StreamSurfaceError> {
        let bytes = spec.charged_bytes()?;
        if transfer.format != spec.format || transfer.sample_count != spec.sample_count {
            return Err(StreamSurfaceError::InvalidOptions);
        }
        let limit = device.limits().max_texture_dimension_2d;
        if spec.width > limit || spec.height > limit {
            return Err(StreamSurfaceError::TooLarge);
        }
        let total = ledger
            .total_bytes()
            .checked_add(pool_bytes)
            .and_then(|total| total.checked_add(bytes))
            .ok_or(StreamSurfaceError::Overflow)?;
        if total > renderer_budget_bytes {
            return Err(StreamSurfaceError::TooLarge);
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let create = |label, samples| {
                // gpu-alloc: caller
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: spec.width,
                        height: spec.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: spec.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | if samples == 1 {
                            wgpu::TextureUsages::COPY_SRC
                        } else {
                            wgpu::TextureUsages::empty()
                        },
                    view_formats: &[],
                });
                TrackedTexture::new(
                    ledger,
                    if samples > 1 {
                        GpuResourceKind::MsaaTarget
                    } else {
                        GpuResourceKind::PanelTexture
                    },
                    texture,
                )
            };
            let prefix = create("stream prefix", spec.sample_count);
            let prefix_view = prefix.create_view(&Default::default());
            let display = create("stream display", spec.sample_count);
            let display_view = display.create_view(&Default::default());
            let resolved = (spec.sample_count > 1).then(|| create("stream display resolve", 1));
            let resolved_view = resolved
                .as_ref()
                .map(|texture| texture.create_view(&Default::default()));
            let source = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("stream prefix sample source"),
                layout: &transfer.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&prefix_view),
                }],
            });
            Self {
                spec,
                prefix,
                display,
                display_view,
                resolved,
                resolved_view,
                source,
            }
        }))
        .map_err(|_| StreamSurfaceError::AllocationFailed)
    }

    pub(crate) fn prefix(&self) -> &TrackedTexture {
        &self.prefix
    }

    pub(crate) fn resolved(&self) -> &TrackedTexture {
        self.resolved.as_ref().unwrap_or(&self.display)
    }

    /// P must have been initialized by the caller. Run only when content changes.
    /// Suffix is drawn after transfer, before the final resolve, and never into P.
    pub(crate) fn record_display(
        &self,
        transfer: &StreamTransfer,
        encoder: &mut wgpu::CommandEncoder,
        suffix: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> Result<(), StreamSurfaceError> {
        if transfer.format != self.spec.format || transfer.sample_count != self.spec.sample_count {
            return Err(StreamSurfaceError::InvalidOptions);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("stream prefix to display then suffix"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.display_view,
                depth_slice: None,
                resolve_target: self.resolved_view.as_ref(),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&transfer.pipeline);
        pass.set_bind_group(0, &self.source, &[]);
        pass.draw(0..3, 0..1);
        suffix(&mut pass);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_cost_is_constant_and_checked() {
        let mut spec = StreamSurfaceSpec {
            width: 256,
            height: 16,
            format: wgpu::TextureFormat::Rgba8Unorm,
            sample_count: 1,
        };
        assert_eq!(spec.charged_bytes(), Ok(256 * 16 * 4 * 2));
        spec.sample_count = 4;
        assert_eq!(spec.charged_bytes(), Ok(256 * 16 * 4 * 9));
        spec.width = u32::MAX;
        spec.height = u32::MAX;
        assert_eq!(spec.charged_bytes(), Err(StreamSurfaceError::Overflow));
        spec.width = 0;
        assert_eq!(
            spec.charged_bytes(),
            Err(StreamSurfaceError::InvalidOptions)
        );
        spec.width = 1;
        spec.sample_count = 2;
        assert_eq!(
            spec.charged_bytes(),
            Err(StreamSurfaceError::InvalidOptions)
        );
        spec.sample_count = 1;
        spec.format = wgpu::TextureFormat::Rgba16Float;
        assert_eq!(
            spec.charged_bytes(),
            Err(StreamSurfaceError::InvalidOptions)
        );
    }
}
