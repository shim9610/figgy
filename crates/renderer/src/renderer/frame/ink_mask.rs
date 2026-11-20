//! The ink masks (#381): which pixels of each panel's data layer carry ink,
//! for the pick's hit test.

use super::*;

/// One panel's ink mask: a renderer-owned R8 texture, one sample,
/// panel-sized at the display scale, 1 where the panel's data layer (every
/// fill and body of its series, as the resident display draws them) has a
/// channel above 1/255 and 0 elsewhere. Coverage only, no colour.
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
/// ([`Renderer::remove_chart`]) and with
/// [`Renderer::release_unused_pool_memory`]; the next on-screen prepare
/// draws it again.
pub(in crate::renderer) struct InkMask {
    pub(in crate::renderer) chart: ChartId,
    /// The panel's index among the panels its chart draws as
    /// (`Config::panel_layers` order).
    pub(in crate::renderer) panel: usize,
    pub(in crate::renderer) key: ResidentDisplayKey,
    /// Where the panel was placed, in canvas pixels, at the prepare that
    /// last checked the mask.
    pub(in crate::renderer) panel_rect: Rect,
    /// `R8Unorm`, panel-sized; `COPY_SRC` for the pick's window read.
    pub(in crate::renderer) texture: TrackedTexture,
}

/// The mask's texel format.
pub(in crate::renderer) const INK_MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The pass that turns a panel's data layer into its ink mask.
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

    /// Record the pass that writes `mask` from the resolved data layer
    /// `layer` (the same size).
    pub(in crate::renderer) fn record(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        layer: &wgpu::TextureView,
        mask: &wgpu::TextureView,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy ink mask source"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(layer),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("figgy ink mask"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: mask,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
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
