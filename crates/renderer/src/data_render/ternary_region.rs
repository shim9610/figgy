//! The region fill of a ternary series (`TernaryRegionStyle`): the closed
//! polygons its samples outline, filled by stencil-then-cover with the
//! even-odd rule into the panel composite's stencil attachment
//! (`surface_mesh::DEPTH_STENCIL_FORMAT`, #49, #55).
//!
//! Two pipelines over one assembled module: the stencil pass draws one
//! triangle per edge from the triangle's centre, inverting the stencil and
//! writing no colour; the cover pass draws the ternary triangle once where
//! the stencil is non-zero and resets it. The vertex stage reads the
//! series' x, y and c columns from the pool as storage (group 1), so no
//! vertex slot is bound and nothing is stored per polygon.

/// The assembled region module.
pub const TERNARY_REGION_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/ternary_region.wgsl",
);

/// GPU twin: `ternary_region.wgsl::TernaryRegionParams`.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TernaryRegionParamsGpu {
    pub color_premul: [f32; 4],
    pub x_base: u32,
    pub y_base: u32,
    pub c_base: u32,
    pub count: u32,
}

const _: () = assert!(std::mem::size_of::<TernaryRegionParamsGpu>() == 32);

/// Group 1 of the region pipelines: the pool as storage and the params.
pub fn create_ternary_region_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy ternary region bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        },
    )
}

/// Which region pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum TernaryRegionPass {
    /// One triangle per edge inverting the stencil, no colour.
    Stencil,
    /// The ternary triangle where the stencil is non-zero, resetting it.
    Cover,
}

impl TernaryRegionPass {
    pub const ALL: [TernaryRegionPass; 2] = [TernaryRegionPass::Stencil, TernaryRegionPass::Cover];

    fn entries(self) -> (&'static str, &'static str) {
        match self {
            TernaryRegionPass::Stencil => ("vs_stencil", "fs_stencil"),
            TernaryRegionPass::Cover => ("vs_cover", "fs_cover"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TernaryRegionPass::Stencil => "figgy ternary region stencil pipeline",
            TernaryRegionPass::Cover => "figgy ternary region cover pipeline",
        }
    }

    /// The stencil state: the stencil pass inverts every covered sample
    /// (even-odd); the cover pass paints where the stencil is non-zero and
    /// clears it for the next series. Neither tests or writes depth.
    pub fn depth_stencil(self) -> wgpu::DepthStencilState {
        let face = match self {
            TernaryRegionPass::Stencil => wgpu::StencilFaceState {
                compare: wgpu::CompareFunction::Always,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op: wgpu::StencilOperation::Invert,
            },
            TernaryRegionPass::Cover => wgpu::StencilFaceState {
                compare: wgpu::CompareFunction::NotEqual,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op: wgpu::StencilOperation::Zero,
            },
        };
        wgpu::DepthStencilState {
            format: super::surface_mesh::DEPTH_STENCIL_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState {
                front: face,
                back: face,
                read_mask: 0xff,
                write_mask: 0xff,
            },
            bias: wgpu::DepthBiasState::default(),
        }
    }
}

/// One region pipeline over the composite's `target_format` and
/// `sample_count`.
pub(crate) fn create_ternary_region_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    region_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    pass: TernaryRegionPass,
) -> wgpu::RenderPipeline {
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy ternary region layout"),
            bind_group_layouts: &[Some(transform_bgl), Some(region_bgl)],
            immediate_size: 0,
        },
    );
    let (vertex, fragment) = pass.entries();
    let (blend, write_mask) = match pass {
        TernaryRegionPass::Stencil => (None, wgpu::ColorWrites::empty()),
        TernaryRegionPass::Cover => (
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            wgpu::ColorWrites::ALL,
        ),
    };
    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(pass.label()),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(pass.depth_stencil()),
            multisample: super::multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend,
                    write_mask,
                })],
            }),
            multiview_mask: None,
            cache: None,
        },
    )
}

/// The region group of one series: the pool and its params, charged.
pub(crate) fn create_ternary_region_bind_group(
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    params: &TernaryRegionParamsGpu,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    let tally = crate::gpu_memory::ChargeTally::new();
    let params_buf = crate::gpu_memory::charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy ternary region params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy ternary region bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pool_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params_buf.as_entire_binding(),
            },
        ],
    });
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    (bind_group, charge)
}

/// One series' region fill: its stencil and cover pipelines, the panel
/// transform, its group 1 and the vertex count to instance the edges over.
pub struct ColumnTernaryRegionLayer<'a> {
    pub stencil: &'a wgpu::RenderPipeline,
    pub cover: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub region_bg: wgpu::BindGroup,
    pub count: u32,
    pub charge: crate::gpu_memory::SharedCharge,
}

/// Draw `layer` into a pass with the stencil attachment: `count` edge
/// triangles (the closing edges included) into the stencil, then one cover.
/// The cost grows with the vertex count only: N triangles for N vertices.
pub fn issue_ternary_region_layer(
    pass: &mut wgpu::RenderPass<'_>,
    layer: &ColumnTernaryRegionLayer<'_>,
) {
    if layer.count < 3 {
        return;
    }
    pass.set_stencil_reference(0);
    pass.set_pipeline(layer.stencil);
    pass.set_bind_group(0, layer.transform_bg, &[]);
    pass.set_bind_group(1, &layer.region_bg, &[]);
    pass.draw(0..3, 0..layer.count);
    pass.set_pipeline(layer.cover);
    pass.draw(0..3, 0..1);
}
