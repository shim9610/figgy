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
pub(crate) const SCATTER_SHADER: &str = crate::shader_assembly::wgsl_module!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
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
                ],
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
            depth_stencil: None,
            multisample: multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
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
    let mut bgls: Vec<Option<&wgpu::BindGroupLayout>> = vec![Some(transform_bgl), Some(style_bgl)];
    if let Some(t) = texture_bgl {
        bgls.push(Some(t));
    }
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
                ],
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
            depth_stencil: None,
            multisample: multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
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
}
