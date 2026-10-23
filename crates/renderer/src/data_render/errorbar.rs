//! Error-bar series: pipelines and the error-bar draw.

use super::*;

/// The assembled error-bar module.
pub(crate) const ERRORBAR_SHADER: &str = crate::shader_assembly::wgsl_module!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/grid_pair.wgsl",
    "data_render/common/sketch_noise.wgsl",
    "data_render/errorbar_columnar.wgsl",
);

/// Columnar errorbar pipeline.
/// slots 0..5 are per-instance f32: x, y, err_y_lo, err_y_hi, err_x_lo,
/// err_x_hi. Each instance emits 36 vertices on a `TriangleList`: six
/// axis-aligned quads (Y stem, caps @ y_lo/y_hi, X stem, caps @ x_lo/x_hi),
/// expanded in the vertex shader by half their pixel stroke width
/// (`Style::line_width_px` for stems, `cap_width_px` for caps; caps span
/// ±`cap_half_px`). A direction whose err columns sum to <= 0 collapses to
/// zero-area quads, so its stem and caps draw nothing.
pub fn create_errorbar_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::errorbar(device);
    create_errorbar_columnar_pipeline_with_sample_count(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        target_format,
        1,
    )
}

pub(crate) fn create_errorbar_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_errorbar_columnar_pipeline_with_entries(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_main",
        "figgy errorbar columnar pipeline",
    )
}

pub fn create_errorbar_columnar_mapped_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::errorbar(device);
    create_errorbar_columnar_mapped_pipeline_from_shader(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        style_map_bgl,
        target_format,
        sample_count,
    )
}

pub(crate) fn create_errorbar_columnar_mapped_pipeline_from_shader(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy errorbar mapped layout"),
            bind_group_layouts: &[Some(transform_bgl), Some(style_bgl), Some(style_map_bgl)],
            immediate_size: 0,
        },
    );

    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
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
    const ATTR3: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 3,
    }];
    const ATTR4: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 4,
    }];
    const ATTR5: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 5,
    }];
    const ATTR6: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 0,
        shader_location: 6,
    }];
    let buffers = [
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR0,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR1,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR2,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR3,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR4,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR5,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR6,
        }),
    ];

    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some("figgy errorbar mapped pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_mapped"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &buffers,
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
            depth_stencil: None,
            multisample: multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_mapped"),
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

/// Entry-point-parameterized errorbar pipeline builder. Styled variants
/// (e.g. the sketch `vs_sketch` — fragment stage shared, vertex count
/// unchanged at 36 per instance) share the precise pipeline's layout and
/// state; the renderer's style table supplies the entry string.
// Styled error bars vary shader entries while retaining shared pipeline state.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_errorbar_columnar_pipeline_with_entries(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_errorbar_columnar_pipeline_full(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        vs_entry,
        "fs_main",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        label,
    )
}

/// Full-control errorbar builder — the constellation jet variant needs its
/// own fragment entry and additive blending.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_errorbar_columnar_pipeline_full(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    blend: wgpu::BlendState,
    label: &str,
) -> wgpu::RenderPipeline {
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy errorbar columnar layout"),
            bind_group_layouts: &[Some(transform_bgl), Some(style_bgl)],
            immediate_size: 0,
        },
    );

    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
    // Hold attributes in const arrays to avoid temporary-lifetime issues.
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
    const ATTR3: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 3,
    }];
    const ATTR4: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 4,
    }];
    const ATTR5: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 5,
    }];
    let buffers = [
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR0,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR1,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR2,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR3,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR4,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR5,
        }),
    ];

    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &buffers,
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
            depth_stencil: None,
            multisample: multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        },
    )
}

pub struct ColumnErrorBarDraw<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub style_map_bg: Option<&'a wgpu::BindGroup>,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    pub err_y_lo: ColumnHandle,
    pub err_y_hi: ColumnHandle,
    pub err_x_lo: ColumnHandle,
    pub err_x_hi: ColumnHandle,
    pub style_index: Option<ColumnHandle>,
}
