//! Scatter series: pipelines and the scatter and pick-ring layers.

use super::*;

/// Pixels the marker quad extends past the drawn shape, for anti-aliasing
/// and the outline.
pub(crate) const MARKER_QUAD_MARGIN_PX: f32 = 2.0;
/// `STROKE_HALF_PX`: half-width of the outline stroke.
pub(crate) const MARKER_STROKE_HALF_PX: f32 = 0.75;
/// Twin of `base_shape_id`.
pub(crate) fn marker_base_shape_id(shape_id: u32) -> u32 {
    match shape_id {
        5 => 0,
        6 => 1,
        7 => 2,
        8 => 3,
        9 | 17 => 5,
        10 | 18 => 6,
        11 | 19 => 7,
        12 | 20 => 8,
        13 | 22 => 9,
        14 | 23 => 10,
        15 | 24 => 11,
        16 | 25 => 12,
        21 => 4,
        other => other,
    }
}

/// The factor `visual_shape_radius` applies to the reference radius.
pub(crate) fn marker_visual_scale(base: u32) -> f32 {
    match base {
        1 => 0.886_226_95,
        2 | 5 | 6 | 7 => 1.555_120_3,
        3 => 1.253_314_1,
        9 => 1.149_139_9,
        10 => 1.099_636_1,
        11 => 1.053_907_4,
        12 => 1.462_850_3,
        _ => 1.0,
    }
}
/// The assembled scatter module.
pub(crate) const SCATTER_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/common/project_3d.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/sketch_noise.wgsl",
    "data_render/common/marker.wgsl",
    "data_render/scatter_columnar.wgsl",
);

/// Columnar SDF scatter pipeline.
/// slot 0 (per-vertex): unit quad, slot 1 (per-instance): X, slot 2: Y.
pub fn create_scatter_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::scatter(device);
    create_scatter_columnar_pipeline_with_sample_count(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        target_format,
        1,
    )
}

pub(crate) fn create_scatter_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_scatter_columnar_pipeline_full(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_main",
        "fs_main",
        None,
        "figgy scatter columnar pipeline",
    )
}

/// Precise scatter pipeline variant that resolves per-point style slots.
/// It keeps the common WGSL block untouched by using separate `vs_mapped` /
/// `fs_mapped` entry points and a fourth per-instance f32 vertex slot for
/// `point_style_index_column` when present. Sparse-only mappings bind the X
/// column into that slot as inert filler.
pub fn create_scatter_columnar_mapped_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    value_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::scatter(device);
    create_scatter_columnar_mapped_pipeline_with_entries(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        style_map_bgl,
        value_map_bgl,
        target_format,
        sample_count,
        "vs_mapped",
        "fs_mapped",
        "figgy scatter mapped pipeline",
    )
}

// Mapping adds index/table layouts to the shared scatter pipeline state, plus
// the value-map group (3) and the two value-column vertex slots (4, 5).
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_columnar_mapped_pipeline_with_entries(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    value_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_scatter_mapped_pipeline(
        device,
        shader,
        [transform_bgl, style_bgl, style_map_bgl, value_map_bgl],
        target_format,
        sample_count,
        (vs_entry, fs_entry, label),
        false,
        None,
    )
}

/// The mapped scatter pipeline of a ternary chart's entries
/// (`vs_ternary_mapped`, `vs_pick_ring_ternary_mapped`): the mapped layout
/// with the third component at slot 6.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_ternary_mapped_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    value_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_scatter_mapped_pipeline(
        device,
        shader,
        [transform_bgl, style_bgl, style_map_bgl, value_map_bgl],
        target_format,
        sample_count,
        (vs_entry, fs_entry, label),
        true,
        None,
    )
}

/// The mapped scatter pipeline, with the third component of a ternary
/// chart's entries (or a 3D chart's height) at slot 6 when `third`, and the
/// pass's depth state and target for a 3D chart's entries (`three_d`; its
/// group 3 is the combined value map and camera layout the caller passes).
#[allow(clippy::too_many_arguments)]
fn create_scatter_mapped_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    [transform_bgl, style_bgl, style_map_bgl, value_map_bgl]: [&wgpu::BindGroupLayout; 4],
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    (vs_entry, fs_entry, label): (&str, &str, &str),
    third: bool,
    three_d: Option<View3dPass>,
) -> wgpu::RenderPipeline {
    let (depth_stencil, color_target) = match three_d {
        Some(pass) => (pass.depth_stencil(), pass.color_target(target_format)),
        None => (
            None,
            wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            },
        ),
    };
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[
                Some(transform_bgl),
                Some(style_bgl),
                Some(style_map_bgl),
                Some(value_map_bgl),
            ],
            immediate_size: 0,
        },
    );

    let vec2_stride = (std::mem::size_of::<f32>() * 2) as wgpu::BufferAddress;
    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: vec2_stride,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 1,
                        }],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 2,
                        }],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 0,
                            shader_location: 3,
                        }],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 4,
                        }],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 5,
                        }],
                    }),
                    // slot 6: a ternary chart's third component.
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 6,
                        }],
                    }),
                ][..if third { 7 } else { 6 }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil,
            multisample: three_d.map_or_else(
                || multisample_state(sample_count),
                |pass| pass.multisample(sample_count),
            ),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(color_target)],
            }),
            multiview_mask: None,
            cache: None,
        },
    )
}

/// Two-entry convenience used by the sketch style (shared layout/state).
// This wrapper forwards both shader entries and the shared scatter state.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_columnar_pipeline_with_entries(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_scatter_columnar_pipeline_full(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        vs_entry,
        fs_entry,
        None,
        label,
    )
}

/// Entry-point-parameterized scatter pipeline builder. Styled variants share
/// the precise pipeline's three vertex slots; the constellation planet
/// variant additionally binds the style textures as group 2.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_columnar_pipeline_full(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    texture_bgl: Option<&wgpu::BindGroupLayout>,
    label: &str,
) -> wgpu::RenderPipeline {
    create_scatter_pipeline(
        device,
        shader,
        [transform_bgl, style_bgl],
        target_format,
        sample_count,
        (vs_entry, fs_entry, label),
        texture_bgl,
        false,
        None,
    )
}

/// Which target a 3D chart's point pipeline draws into (#132): the panel
/// composite with the depth test (`Draw`), the 1×1 ID pick target with the
/// depth test (`Pick`), or the frame's live highlight slot without depth
/// (`Live`, the picked ring).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View3dPass {
    Draw,
    Pick,
    Live,
}

impl View3dPass {
    pub(crate) fn depth_stencil(self) -> Option<wgpu::DepthStencilState> {
        match self {
            View3dPass::Draw | View3dPass::Pick => Some(super::surface_mesh::point_depth_stencil()),
            View3dPass::Live => None,
        }
    }

    /// The multisample state: the draw converts its coverage to sample
    /// coverage (alpha to coverage) on a multisampled target, so a mark's
    /// antialiased fringe writes depth only on the samples it covers and a
    /// dense trajectory stays solid instead of fading where an earlier
    /// fringe's depth hid the later marks; the pick (one sample) and the
    /// live ring (no depth) blend as every data draw.
    pub(crate) fn multisample(self, sample_count: u32) -> wgpu::MultisampleState {
        wgpu::MultisampleState {
            alpha_to_coverage_enabled: self == View3dPass::Draw && sample_count > 1,
            ..multisample_state(sample_count)
        }
    }

    /// The colour target: the integer ID target for the pick, else the
    /// given format blended as every data draw.
    pub(crate) fn color_target(self, target_format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
        match self {
            View3dPass::Pick => wgpu::ColorTargetState {
                format: super::surface_mesh::SURFACE_PICK_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
            View3dPass::Draw | View3dPass::Live => wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            },
        }
    }
}

/// The precise scatter pipeline of a 3D chart's entries (`vs_3d`,
/// `vs_3d_pick`, `vs_pick_ring_3d`): the precise layout with the height at
/// slot 3 and the camera group (`surface_mesh::create_surface_data_bind_group_layout`)
/// as group 3.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_view3d_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    camera_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    pass: View3dPass,
    vs_entry: &str,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_scatter_pipeline(
        device,
        shader,
        [transform_bgl, style_bgl],
        target_format,
        sample_count,
        (vs_entry, fs_entry, label),
        None,
        true,
        Some((camera_bgl, pass)),
    )
}

/// The mapped scatter pipeline of a 3D chart's entries (`vs_3d_mapped`,
/// `vs_3d_mapped_pick`, `vs_pick_ring_3d_mapped`): the mapped layout with
/// the height at slot 6 and the combined value map and camera group
/// (`surface_mesh::create_view3d_value_map_bind_group_layout`) as group 3.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_view3d_mapped_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    view3d_value_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    pass: View3dPass,
    vs_entry: &str,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_scatter_mapped_pipeline(
        device,
        shader,
        [
            transform_bgl,
            style_bgl,
            style_map_bgl,
            view3d_value_map_bgl,
        ],
        target_format,
        sample_count,
        (vs_entry, fs_entry, label),
        true,
        Some(pass),
    )
}

/// The precise scatter pipeline of a ternary chart's entries (`vs_ternary`,
/// `vs_pick_ring_ternary`): the precise layout with the third component at
/// slot 3.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_ternary_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_scatter_pipeline(
        device,
        shader,
        [transform_bgl, style_bgl],
        target_format,
        sample_count,
        (vs_entry, fs_entry, label),
        None,
        true,
        None,
    )
}

/// The precise scatter pipeline, with the third component of a ternary
/// chart's entries (or a 3D chart's height) at slot 3 when `third`, and
/// the camera group as group 3 with the pass's depth state and target for
/// a 3D chart's entries (`three_d`).
#[allow(clippy::too_many_arguments)]
fn create_scatter_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    [transform_bgl, style_bgl]: [&wgpu::BindGroupLayout; 2],
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    (vs_entry, fs_entry, label): (&str, &str, &str),
    texture_bgl: Option<&wgpu::BindGroupLayout>,
    third: bool,
    three_d: Option<(&wgpu::BindGroupLayout, View3dPass)>,
) -> wgpu::RenderPipeline {
    let mut bgls: Vec<Option<&wgpu::BindGroupLayout>> = vec![Some(transform_bgl), Some(style_bgl)];
    if let Some(t) = texture_bgl {
        bgls.push(Some(t));
    }
    if let Some((camera_bgl, _)) = three_d {
        bgls.resize(3, None);
        bgls.push(Some(camera_bgl));
    }
    let (depth_stencil, color_target) = match three_d {
        Some((_, pass)) => (pass.depth_stencil(), pass.color_target(target_format)),
        None => (
            None,
            wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            },
        ),
    };
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy scatter columnar layout"),
            bind_group_layouts: &bgls,
            immediate_size: 0,
        },
    );

    let vec2_stride = (std::mem::size_of::<f32>() * 2) as wgpu::BufferAddress;
    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;

    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    // slot 0: unit quad (per-vertex, vec2)
                    Some(wgpu::VertexBufferLayout {
                        array_stride: vec2_stride,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }),
                    // slot 1: X column (per-instance, f32)
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 1,
                        }],
                    }),
                    // slot 2: Y column (per-instance, f32)
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 2,
                        }],
                    }),
                    // slot 3: a ternary chart's third component.
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 3,
                        }],
                    }),
                ][..if third { 4 } else { 3 }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil,
            multisample: three_d.map_or_else(
                || multisample_state(sample_count),
                |(_, pass)| pass.multisample(sample_count),
            ),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(color_target)],
            }),
            multiview_mask: None,
            cache: None,
        },
    )
}

pub struct ColumnScatterLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub style_map_bg: Option<&'a wgpu::BindGroup>,
    /// Group 3 of the mapped pipeline: the series' value maps, or the
    /// renderer's neutral params when it has none. Required whenever
    /// `style_map_bg` is bound; ignored by the plain pipeline.
    pub value_map_bg: Option<&'a wgpu::BindGroup>,
    pub quad_vb: &'a wgpu::Buffer,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    pub style_index: Option<ColumnHandle>,
    /// Value columns of `point_color_map` / `point_size_map`, bound as
    /// vertex slots 4 and 5 of the mapped pipeline (X as filler when absent).
    pub color_value: Option<ColumnHandle>,
    pub size_value: Option<ColumnHandle>,
    /// Style textures (group 2) when `pipeline`'s layout includes them —
    /// the constellation planet atlas/ring bind group. `None` otherwise.
    pub texture_bg: Option<&'a wgpu::BindGroup>,
    /// A ternary series' third component, or a 3D series' height (#132):
    /// `pipeline` is a ternary or 3D pipeline, the column bound at slot 3
    /// (precise) or 6 (mapped). `None` for every other series.
    pub third: Option<ColumnHandle>,
    /// A 3D series' camera group, bound as group 3 (#132): `pipeline` is a
    /// 3D pipeline, drawn into the panel composite with the depth test by
    /// `issue_series_surfaces`. `None` for every other series.
    pub view3d: Option<super::surface_mesh::View3dBinding>,
}

pub struct ColumnPickRingLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: wgpu::BindGroup,
    pub style_map_bg: Option<&'a wgpu::BindGroup>,
    /// See [`ColumnScatterLayer::value_map_bg`].
    pub value_map_bg: Option<&'a wgpu::BindGroup>,
    pub quad_vb: &'a wgpu::Buffer,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    pub style_index: Option<ColumnHandle>,
    pub color_value: Option<ColumnHandle>,
    pub size_value: Option<ColumnHandle>,
    pub instance: u32,
    /// See [`ColumnScatterLayer::third`].
    pub third: Option<ColumnHandle>,
    /// See [`ColumnScatterLayer::view3d`]: the ring of a 3D series, in the
    /// live slot through `vs_pick_ring_3d`.
    pub view3d: Option<super::surface_mesh::View3dBinding>,
}
