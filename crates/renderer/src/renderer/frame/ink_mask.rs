//! The ink masks (#381): which series owns each pixel of each panel's data
//! layer, for the pick's hit test.

use super::*;

/// One panel's ink mask: a renderer-owned `R32Uint` texture, one sample,
/// panel-sized at the display scale. Each texel holds the slot of the
/// topmost series whose drawing (every fill and body, as the resident
/// display draws them) has a channel above 1/255 there, its index in
/// [`Self::owners`] plus one, and 0 where no series inks it. Ownership only,
/// no colour.
///
/// It is recorded series by series in the frame's order, every series'
/// fill and then every series' body, each drawn on its own with its own
/// pipelines; the resolve pass then writes its slot where it inked, so a
/// later pass overwrites an earlier one and a marker over a later series'
/// fill keeps the marker's series. A 3D panel keeps its depth
/// between the series, so a surface behind an earlier one leaves it the
/// owner. It costs 4 bytes per panel pixel, and recording it one panel
/// pass and one resolve pass per series (one more of each for a series
/// with a fill), on a display key change only.
///
/// Every panel of a registered chart has one, whatever its draw style:
/// each layer of a layered document (overlays, subplots, line offset
/// members) is its own panel. The on-screen prepare draws it again only when
/// its [`ResidentDisplayKey`] changes, which is the resident display's
/// invalidation; an export never touches it, and a pick only reads it.
///
/// A mask is keyed by its chart and panel, not by the view the chart is
/// drawn into: a chart shown in several views at once has the mask of its
/// most recent on-screen prepare. It goes with its chart
/// ([`Renderer::remove_chart`]) or a panel the chart no longer draws as; a
/// live chart's mask is in use, so
/// [`Renderer::release_unused_pool_memory`] keeps it.
pub(in crate::renderer) struct InkMask {
    pub(in crate::renderer) chart: ChartId,
    /// The panel's index among the panels its chart draws as
    /// (`Config::panel_layers` order).
    pub(in crate::renderer) panel: usize,
    pub(in crate::renderer) key: ResidentDisplayKey,
    /// Where the panel was placed, in canvas pixels, at the prepare that
    /// last checked the mask.
    pub(in crate::renderer) panel_rect: Rect,
    /// `R32Uint`, panel-sized; `COPY_SRC` for the pick's window read.
    pub(in crate::renderer) texture: TrackedTexture,
    /// The series each slot names, in drawing order: slot `k` is
    /// `owners[k - 1]`. One entry per series the panel draws, shared with
    /// the picks that read the mask.
    pub(in crate::renderer) owners: Arc<[crate::gpu_pick::InkSeries]>,
}

/// The mask's texel format: a series slot per texel, so a panel's series
/// count is not capped (rule 10).
pub(in crate::renderer) const INK_MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// The pass that writes one series' slot into a panel's ink mask where the
/// series inked the panel's data layer.
pub(in crate::renderer) struct InkMaskPipeline {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl InkMaskPipeline {
    pub(in crate::renderer) fn new(device: &wgpu::Device) -> Self {
        let shader = crate::gpu_compile::shader_module(
            device,
            wgpu::ShaderModuleDescriptor {
                label: Some("figgy ink mask shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("ink_mask.wgsl").into()),
            },
        );
        let layout = crate::gpu_compile::bind_group_layout(
            device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("figgy ink mask layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            },
        );
        let pipeline_layout = data_render::RenderPipelineSpec::layout(
            device,
            "figgy ink mask pipeline layout",
            &[Some(&layout)],
        );
        let pipeline = data_render::RenderPipelineSpec {
            label: "figgy ink mask pipeline",
            layout: &pipeline_layout,
            shader: &shader,
            entries: ("ink_mask_vs", "ink_mask_fs"),
            buffers: &[],
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth_stencil: None,
            multisample: data_render::multisample_state(1),
            target: wgpu::ColorTargetState {
                format: INK_MASK_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
        }
        .create(device);
        Self { layout, pipeline }
    }

    /// The bind group that reads the resolved data layer `layer`.
    pub(in crate::renderer) fn source(
        &self,
        device: &wgpu::Device,
        layer: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy ink mask source"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(layer),
            }],
        })
    }

    /// Record the pass that writes `slot` into `mask` wherever the data
    /// layer `source` reads (the same size) has ink, after clearing the mask
    /// to 0 (no ink) when `clear`. `slot` 0 only clears.
    pub(in crate::renderer) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::BindGroup,
        mask: &wgpu::TextureView,
        slot: u32,
        clear: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("figgy ink mask"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: mask,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        if slot == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, source, &[]);
        // The slot is the instance index, so the pass needs no buffer.
        pass.draw(0..3, slot..slot + 1);
    }
}

#[cfg(test)]
impl Renderer {
    /// Show registered chart `id` as `request` sees it and run one
    /// on-screen prepare, so its pick reads drawn ink (#381): letterboxed
    /// onto the request's display panel, or as stored when the panel is the
    /// stored document at 1x. Test support.
    pub(crate) fn shown_for_pick(&mut self, id: ChartId, request: &GpuPickRequest) -> &mut Self {
        let panel = request.display_panel_px;
        let document = self.chart_config(id).unwrap().document_area();
        if request.display_scale != 1.0 || panel != document {
            assert_eq!((panel.x, panel.y), (0, 0), "a display panel at the origin");
            self.set_chart_display(id, Some((panel.width, panel.height)))
                .unwrap();
        }
        let (config, rect, scale) = self.chart_display_config(id).unwrap();
        let chart = Chart::new(config.into_owned());
        let mut view = self.create_chart_view_at(&chart, rect, scale).unwrap();
        self.prepare_registered(&mut [RegisteredChartDrawItem {
            chart_id: id,
            view: &mut view,
        }])
        .unwrap();
        self
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// The resolve pass writes a series slot past any 8 or 16 bit range
    /// (rule 10), only where the data layer has a channel above 1/255, and a
    /// later series overwrites an earlier one where both ink: the mask holds
    /// `R32Uint` slots, so a panel's series count is not capped.
    #[test]
    fn the_resolve_pass_writes_any_slot_where_the_series_inked() {
        let (device, queue) = crate::test_gpu::device();
        let pipeline = InkMaskPipeline::new(&device);
        let layer = |texels: [[u8; 4]; 4]| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 4,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                texels.as_flattened(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(16),
                    rows_per_image: None,
                },
                texture.size(),
            );
            texture
        };
        // The first series inks texels 0 and 1, the second 1 and 2 (texel
        // 2 only by its alpha, 2/255); texel 3 carries 1/255, no ink.
        let first = layer([[255, 0, 0, 255], [0, 0, 255, 255], [0; 4], [1, 1, 1, 1]]);
        let second = layer([[0; 4], [255, 255, 255, 255], [0, 0, 0, 2], [1, 1, 1, 1]]);
        let mask = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 4,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: INK_MASK_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mask_view = mask.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (low, high) = (70_001u32, u32::MAX - 1);
        let mut encoder = device.create_command_encoder(&Default::default());
        for (texture, slot, clear) in [(&first, low, true), (&second, high, false)] {
            let source = pipeline.source(&device, &texture.create_view(&Default::default()));
            pipeline.record(&mut encoder, &source, &mask_view, slot, clear);
        }
        encoder.copy_texture_to_buffer(
            mask.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            mask.size(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device completes the mask");
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .expect("the readback maps");
        let slots: Vec<u32> = mapped[..16]
            .chunks_exact(4)
            .map(|texel| u32::from_le_bytes([texel[0], texel[1], texel[2], texel[3]]))
            .collect();
        assert_eq!(slots, [low, high, high, 0]);
    }
}
