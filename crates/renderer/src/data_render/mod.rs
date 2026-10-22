//! GPU data rendering via wgpu.
//!
//! wgpu owns the window surface; the CPU raster stack (tiny-skia + swash)
//! rasterises axes / titles / labels and uploads them as a texture. A single
//! render pass draws data primitives followed by the chrome texture overlay,
//! then presents.

use crate::color::Color;
use crate::config::Config;
use crate::data_config::ScatterShape;
use crate::layout::Rect;

use wgpu::util::DeviceExt;

mod surface;
pub use surface::*;
mod textured;
pub use textured::*;
mod transform;
pub use transform::*;
mod style;
pub use style::*;
mod value_map;
pub use value_map::*;
mod selection;
pub(crate) use selection::*;
mod line;
pub use line::*;
mod stylized;
pub use stylized::*;
mod scatter;
pub use scatter::*;
pub mod bar_envelope;
pub mod column_pool;
pub mod line_arc;
pub(crate) mod stream_field;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod stream_point_style_tests;
pub mod vector;
pub use column_pool::{
    AllocError, ColumnHandle, ColumnId, ColumnPool, ColumnSlot, DefragPolicy, FreeRegion,
    GpuAllocCtx, GpuBudget, GrowthPolicy,
};

// Instance, adapter, surface, and device setup.

// Texture upload, samplers, bind groups, and pipelines for the chrome overlay.

// Transform uniform (data-space -> NDC), shared by scatter / line / errorbar.
// Vertex buffers carry data-space values directly; resize/zoom only updates
// this uniform, never the instance data.

// WGSL mirror size guards. Field order and size must remain byte-identical to
// every shader common block before either CPU structure changes.
const _: () = assert!(std::mem::size_of::<ScatterTransform>() == 144);

// Primitive style uniform — color and per-primitive options.
// Bind groups: group(0) = transform (shared), group(1) = style (per primitive).

// Columnar pipelines backed by ColumnPool.
//
// Every column is uploaded once into a ColumnPool buffer; charts draw by
// binding `pool.buffer().slice(handle.byte_range())` into vertex slots.
// Shaders take a single column per slot; lengths and offsets are decided by
// the caller via the slice and draw range.

// Procedural planet-atlas bakes run once per style-set creation; results are
// cached on the GPU and never recomputed per frame.

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

/// Vertices per bar instance: five axis-aligned quads (fill + four border
/// edges) as a TriangleList. Must match `bar_columnar.wgsl`'s `BAR_SEGMENTS`.
pub const BAR_VERTICES_PER_INSTANCE: u32 = 30;
/// Four axis-aligned outline quads for one selected histogram bin.
/// Must match `vs_bar_selection` in `bar_columnar.wgsl`.
pub const BAR_SELECTION_VERTICES: u32 = 24;

/// Columnar bar pipeline. Slots (all per-instance): 0 = edge_lo, 1 = edge_hi,
/// 2 = value. The caller binds the edge column twice, the second time shifted
/// by one logical value, and picks which pool column is edges vs values from
/// the bar's orientation.
pub fn create_bar_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::bar(device);
    create_bar_columnar_pipeline_with_entry(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        None,
        target_format,
        1,
        "vs_main",
        "figgy bar columnar pipeline",
    )
}

pub(crate) fn create_bar_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_bar_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        style_bgl,
        None,
        target_format,
        sample_count,
        "vs_envelope_bars",
        "figgy bar columnar pipeline",
    )
}

pub fn create_bar_columnar_mapped_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::bar(device);
    create_bar_columnar_pipeline_with_entry(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        Some(style_map_bgl),
        target_format,
        1,
        "vs_mapped",
        "figgy mapped bar columnar pipeline",
    )
}

pub(crate) fn create_bar_columnar_mapped_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_bar_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        style_bgl,
        Some(style_map_bgl),
        target_format,
        sample_count,
        "vs_envelope_mapped_bars",
        "figgy mapped bar columnar pipeline",
    )
}

pub(crate) fn create_bar_selection_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    selection_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_bar_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        style_bgl,
        Some(selection_bgl),
        target_format,
        sample_count,
        "vs_bar_selection",
        "figgy selected histogram bin pipeline",
    )
}

#[allow(clippy::too_many_arguments)]
fn create_bar_columnar_pipeline_with_entry(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    group_one_bgl: &wgpu::BindGroupLayout,
    group_two_bgl: Option<&wgpu::BindGroupLayout>,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vertex_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    let base_layouts = [Some(transform_bgl), Some(group_one_bgl)];
    let selection_layouts = [Some(transform_bgl), Some(group_one_bgl), group_two_bgl];
    let bind_group_layouts = if group_two_bgl.is_some() {
        &selection_layouts[..]
    } else {
        &base_layouts[..]
    };
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy bar columnar layout"),
            bind_group_layouts,
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
    ];

    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex_entry),
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
                entry_point: Some("fs_main"),
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

/// Vertices per field draw. The field is **one** quad for the whole grid, not
/// one per cell — `field_columnar.wgsl`'s header has the two reasons (25M
/// instances at matrix scale, and MSAA seams between adjacent quads).
pub const FIELD_VERTICES: u32 = 6;

/// Where one constituent grid column lives in the pool.
///
/// GPU twin: `field_columnar.wgsl::GridColumn`. `base` is an **f32 lane** index,
/// matching the pool's `array<f32>` view — the same convention the arc scan and
/// the star pass use.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GridColumnGpu {
    pub base: u32,
    pub len: u32,
}

const _: () = assert!(std::mem::size_of::<GridColumnGpu>() == 8);

/// `FieldParams.flags` — the grid's declared shape, packed. Every one of these
/// comes from the *declaration*, never from the column lengths.
pub const FIELD_FLAG_COLUMNS_ARE_Y: u32 = 1;
pub const FIELD_FLAG_CENTERS: u32 = 2;
pub const FIELD_FLAG_INTERPOLATED: u32 = 4;
pub const FIELD_FLAG_BANDS: u32 = 8;
pub const FIELD_FLAG_LOG_Z: u32 = 16;

/// Contour lookup granularity. Each block fits in one shader `u32` candidate
/// mask, so the original declaration order can be restored without a second
/// per-fragment table or an O(level_count) scan.
pub(crate) const CONTOUR_LEVEL_BLOCK_SIZE: usize = 32;
pub(crate) const CONTOUR_LEVEL_BLOCK_COUNT: usize = 32;
pub(crate) const CONTOUR_LEVEL_LOOKUP_CAPACITY: usize =
    CONTOUR_LEVEL_BLOCK_SIZE * CONTOUR_LEVEL_BLOCK_COUNT;
const _: () = assert!(crate::data_config::MAX_CONTOUR_LEVELS == CONTOUR_LEVEL_LOOKUP_CAPACITY);

/// Per-32-level lookup bounds uploaded at field group-2 binding 6.
///
/// GPU twin: `field_columnar.wgsl::ContourLookupMetadata`, 8 B. Counts are
/// derived from the actual f32 search keys, after the declared f64 levels have
/// crossed the renderer upload boundary.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ContourLookupMetadataGpu {
    pub finite_count: u32,
    pub negative_infinity_count: u32,
}

const _: () = assert!(std::mem::size_of::<ContourLookupMetadataGpu>() == 8);

/// Everything the field shader needs beyond `Transform` and `Style`.
///
/// GPU twin: `field_columnar.wgsl::FieldParams`, 64 B. `z_min` / `z_max` are the
/// colourbar's bounds as the pool's `(hi, lo)` f32 pair, **already
/// log-transformed** when [`FIELD_FLAG_LOG_Z`] is set: the bounds are f64 on the
/// host, so taking the logarithm here keeps precision the shader would lose.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FieldParamsGpu {
    pub x_base: u32,
    pub y_base: u32,
    pub x_len: u32,
    pub y_len: u32,
    pub cols: u32,
    pub rows: u32,
    /// Contour levels, and for a `Bands` fill the bands' boundaries. Binding 2
    /// preserves declaration order; binding 3 carries a block-sorted lookup copy
    /// so fragments binary-search reachable levels without changing positional
    /// `level_index`, duplicate, colour, or source-over semantics.
    pub level_count: u32,
    pub stop_count: u32,
    pub flags: u32,
    pub opacity: f32,
    /// Contour stroke width in pixels, already scaled. Read by `fs_contour` and
    /// ignored by the fill.
    pub line_width_px: f32,
    /// Entries in the `level_colors` table.
    pub level_color_count: u32,
    pub z_min: [f32; 2],
    pub z_max: [f32; 2],
}

impl FieldParamsGpu {
    /// Params for a field group that serves cell geometry only (a grid
    /// vector's selected-cell outline): the grid lattice and
    /// its flags are real; the z range, levels, stops and colours are unread
    /// by those passes and are left at neutral values.
    pub fn cell_geometry(
        x_base: u32,
        y_base: u32,
        x_len: u32,
        y_len: u32,
        cols: u32,
        rows: u32,
        flags: u32,
    ) -> Self {
        Self {
            x_base,
            y_base,
            x_len,
            y_len,
            cols,
            rows,
            level_count: 0,
            stop_count: 0,
            flags,
            opacity: 1.0,
            line_width_px: 0.0,
            level_color_count: 0,
            z_min: [0.0, 0.0],
            z_max: [1.0, 0.0],
        }
    }
}

const _: () = assert!(std::mem::size_of::<FieldParamsGpu>() == 64);

/// Bind group layout for the field's own data (group 2).
///
/// No textures: the colour ramp travels as its control points and the shader
/// reimplements `model::colormap::sample` over them, so the GPU field and the
/// CPU-drawn colourbar strip agree by construction instead of by resampling a
/// quantized LUT. The pool is bound whole in both stages — the vertex stage
/// needs nothing from it, but a single layout for one shader is simpler than two.
///
/// **Compute-visible too.** `contour_anchor.wgsl` reads the very same grid
/// through the very same `locate`/`grid_value` (they are one SSoT block), so it
/// binds this group rather than owning a second copy of the table. A label
/// therefore cannot land on a grid the lines were not drawn from.
pub fn create_field_data_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let render = wgpu::ShaderStages::VERTEX_FRAGMENT;
    let shared = render.union(wgpu::ShaderStages::COMPUTE);
    let storage = |binding, visibility| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    // Only what the anchor pass actually reads is compute-visible. A storage
    // binding counts against `max_storage_buffers_per_shader_stage` (8) in every
    // stage it is visible to, and the anchor pipeline binds four of its own — so
    // marking the ramp tables compute-visible for symmetry would put the layout
    // one over the limit on a conformant device. The colour stops alone are
    // compute-visible: the cell pick evaluates the colour the fill paints
    // (`field_paint`), which brings the anchor pass to exactly eight.
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy field data bgl"),
            entries: &[
                storage(0, shared), // column pool (coordinate + grid bases in the uniform)
                storage(1, shared), // per-constituent-column (base, len)
                storage(2, shared), // contour levels, in data units
                storage(3, shared), // colourmap control points
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: shared,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(5, render), // per-level contour stroke colour, premultiplied
                storage(6, wgpu::ShaderStages::FRAGMENT), // contour lookup block metadata
            ],
        },
    )
}

/// Bind the field's pool, tables and params into the layout above.
///
/// Every slice must be non-empty — a zero-sized storage binding is invalid, so
/// the caller pads. `FieldParams` carries the real counts.
/// The field's group-2 table contents, resolved from the grid declaration.
///
/// One argument instead of four: they are always built together from the same
/// declaration and always uploaded together, so passing them separately only
/// creates an order to get wrong. Each slice must be non-empty — a zero-sized
/// storage binding is invalid — and the real counts live in `params`.
pub struct FieldTables<'a> {
    pub grid: &'a [GridColumnGpu],
    pub levels: &'a [f32],
    /// Binding 3: `max(params.stop_count, 1)` ramp/padding entries followed by one
    /// search record per real contour level. In each 32-record block the finite
    /// keys form a sorted prefix; x is the level value and y stores the original
    /// level index as a normal numeric f32. The remaining records are zero.
    pub stops: &'a [[f32; 4]],
    /// Per-level contour stroke colour, **premultiplied**. A fill-only series
    /// pads it to one entry; `FieldParams.level_color_count` carries the truth.
    pub level_colors: &'a [[f32; 4]],
    /// Binding 6: one `[finite_count, negative_infinity_count]` record per
    /// 32-level block. An empty level declaration still has one zero record.
    pub lookup_metadata: &'a [ContourLookupMetadataGpu],
    pub params: &'a FieldParamsGpu,
}

/// The group-2 tables a field bind group needs beyond its grid and params
/// when it serves cell geometry only: a grid vector's selected-cell outline
/// runs the field pipeline, which reads the grid table
/// and `FieldParams` and nothing of the levels, stops, level colours or
/// lookup metadata. Those four bindings must still exist, so one padding set
/// (an empty level list, one stop, one colour, one zero lookup record) is
/// built once per renderer and shared by every such group, the way the
/// neutral value-map group is shared by mapped scatter series.
pub struct FieldCellTables {
    levels: wgpu::Buffer,
    stops: wgpu::Buffer,
    level_colors: wgpu::Buffer,
    lookup_metadata: wgpu::Buffer,
    _charge: crate::gpu_memory::SharedCharge,
}

pub fn create_field_cell_tables(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
) -> crate::Result<FieldCellTables> {
    use crate::gpu_memory::{ChargeTally, charged_buffer_init};
    let lookup = build_field_lookup_tables(&[[0.0; 4]], &[])?;
    let tally = ChargeTally::new();
    let storage = |label: &'static str, contents: &[u8]| {
        charged_buffer_init(
            &tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage: wgpu::BufferUsages::STORAGE,
            },
        )
    };
    let levels = storage(
        "figgy field cell levels (padding)",
        bytemuck::cast_slice(&[0.0f32]),
    );
    let stops = storage(
        "figgy field cell stops (padding)",
        bytemuck::cast_slice(&lookup.stops),
    );
    let level_colors = storage(
        "figgy field cell level colours (padding)",
        bytemuck::cast_slice(&[[0.0f32; 4]]),
    );
    let lookup_metadata = storage(
        "figgy field cell lookup metadata (padding)",
        bytemuck::cast_slice(&lookup.metadata),
    );
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::FieldTable,
    );
    Ok(FieldCellTables {
        levels,
        stops,
        level_colors,
        lookup_metadata,
        _charge: charge,
    })
}

/// A field bind group over an existing grid table buffer for cell geometry
/// only (see [`FieldCellTables`]): the grid and the pool are the caller's,
/// the padding tables are shared, and the params uniform is the one buffer
/// this group owns. `params` comes from [`FieldParamsGpu::cell_geometry`].
pub fn create_field_cell_bind_group(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    grid: &wgpu::Buffer,
    tables: &FieldCellTables,
    params: &FieldParamsGpu,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    use crate::gpu_memory::{ChargeTally, charged_buffer_init};
    let tally = ChargeTally::new();
    let param_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field cell params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::FieldTable,
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy field cell bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pool_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: grid.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: tables.levels.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: tables.stops.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: param_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: tables.level_colors.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: tables.lookup_metadata.as_entire_binding(),
            },
        ],
    });
    (bind_group, charge)
}

pub(crate) struct FieldLookupTables {
    pub stops: Vec<[f32; 4]>,
    pub metadata: Vec<ContourLookupMetadataGpu>,
}

/// Build binding 3 and binding 6 from the same uploaded-f32 level slice.
///
/// Binding 3 keeps one record per declared level so block offsets remain source
/// positional. Each block compacts only its finite keys into a sorted prefix;
/// binding 6 is the bound that makes the zero padding unreachable. The source
/// `levels` remain untouched and continue to define every positional contract.
pub(crate) fn build_field_lookup_tables(
    stops: &[[f32; 4]],
    levels: &[f32],
) -> crate::Result<FieldLookupTables> {
    if levels.len() > CONTOUR_LEVEL_LOOKUP_CAPACITY {
        return Err(crate::FiggyError::StateAllocationFailed {
            resource: "contour level lookup",
            reason: format!(
                "{} levels exceed the {}-entry lookup capacity",
                levels.len(),
                CONTOUR_LEVEL_LOOKUP_CAPACITY
            ),
        });
    }

    let mut table = Vec::new();
    table
        .try_reserve_exact(stops.len().saturating_add(levels.len()))
        .map_err(|error| crate::FiggyError::StateAllocationFailed {
            resource: "field stop and contour lookup table",
            reason: error.to_string(),
        })?;
    table.extend_from_slice(stops);

    let block_count = levels.len().div_ceil(CONTOUR_LEVEL_BLOCK_SIZE);
    let mut metadata = Vec::new();
    metadata
        .try_reserve_exact(block_count.max(1))
        .map_err(|error| crate::FiggyError::StateAllocationFailed {
            resource: "contour lookup metadata table",
            reason: error.to_string(),
        })?;

    let mut order = [0u32; CONTOUR_LEVEL_BLOCK_SIZE];
    for block_start in (0..levels.len()).step_by(CONTOUR_LEVEL_BLOCK_SIZE) {
        let block_len = (levels.len() - block_start).min(CONTOUR_LEVEL_BLOCK_SIZE);
        let mut finite_count = 0usize;
        let mut negative_infinity_count = 0u32;
        for local in 0..block_len {
            let original = block_start + local;
            let value = levels[original];
            if value.is_finite() {
                order[finite_count] = u32::try_from(original).unwrap_or(u32::MAX);
                finite_count += 1;
            } else if value == f32::NEG_INFINITY {
                negative_infinity_count += 1;
            }
        }
        order[..finite_count].sort_unstable_by(|left, right| {
            let left_value = levels[*left as usize];
            let right_value = levels[*right as usize];
            left_value
                .partial_cmp(&right_value)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.cmp(right))
        });
        for original_index in &order[..finite_count] {
            table.push([
                levels[*original_index as usize],
                *original_index as f32,
                0.0,
                0.0,
            ]);
        }
        table.resize(table.len() + block_len - finite_count, [0.0; 4]);
        metadata.push(ContourLookupMetadataGpu {
            finite_count: u32::try_from(finite_count).unwrap_or(u32::MAX),
            negative_infinity_count,
        });
    }
    if metadata.is_empty() {
        metadata.push(ContourLookupMetadataGpu::default());
    }
    Ok(FieldLookupTables {
        stops: table,
        metadata,
    })
}

/// The six buffers end up owned only by the returned bind group, so no wrapper
/// can observe their lifetime. They are charged as one lump against the
/// [`crate::gpu_memory::GpuResourceKind::FieldTable`] row and the returned
/// [`crate::gpu_memory::SharedCharge`] credits
/// it back when the last clone of the layer dies — the same construction
/// `gpu_pick` uses for its bind-group-owned buffers. The grid table scales with
/// the column count, so leaving it uncharged would put a data-proportional term
/// outside the budget.
pub fn create_field_data_bind_group(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    tables: FieldTables<'_>,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    use crate::gpu_memory::{ChargeTally, charged_buffer_init};
    let FieldTables {
        grid,
        levels,
        stops,
        level_colors,
        lookup_metadata,
        params,
    } = tables;
    let tally = ChargeTally::new();
    let grid_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field grid table"),
            contents: bytemuck::cast_slice(grid),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let level_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field levels"),
            contents: bytemuck::cast_slice(levels),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let stop_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field ramp and contour lookup"),
            contents: bytemuck::cast_slice(stops),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let level_color_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field level colours"),
            contents: bytemuck::cast_slice(level_colors),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let lookup_metadata_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field contour lookup metadata"),
            contents: bytemuck::cast_slice(lookup_metadata),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let param_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::FieldTable,
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy field data bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pool_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: grid_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: level_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: stop_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: param_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: level_color_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: lookup_metadata_buf.as_entire_binding(),
            },
        ],
    });
    (bind_group, charge)
}

/// Columnar field pipeline. No vertex buffers — the quad is emitted from
/// `vertex_index` and every data read goes through group 2's storage bindings.
pub fn create_field_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::field(device);
    create_field_columnar_pipeline_with_sample_count(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        target_format,
        1,
    )
}

pub(crate) fn create_field_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        target_format,
        sample_count,
        "fs_main",
        "figgy field columnar pipeline",
    )
}

pub(crate) fn create_field_selection_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    selection_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        selection_bgl,
        field_bgl,
        target_format,
        sample_count,
        "fs_data_selection",
        "figgy selected field data pipeline",
    )
}

/// The selected-field highlight for a labelled contour: group 3 is the
/// series' label-gap group, which `fs_data_selection_labelled` reads.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_field_selection_labelled_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    selection_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    label_gap_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_optional_extra_layout(
        device,
        shader,
        transform_bgl,
        selection_bgl,
        field_bgl,
        Some(label_gap_bgl),
        target_format,
        sample_count,
        "fs_data_selection_labelled",
        "figgy selected labelled field data pipeline",
    )
}

/// The field quad against one fragment entry point.
///
/// `fs_main` fills; `fs_contour` draws the isolines as the level set of the same
/// interpolation. Same vertex shader, same layouts, same quad — which is what
/// makes a filled band and the line over it agree by construction rather than by
/// two implementations happening to round the same way.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_field_columnar_pipeline_with_entry(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    fragment_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_optional_extra_layout(
        device,
        shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        None,
        target_format,
        sample_count,
        fragment_entry,
        label,
    )
}

/// Labelled contours add group 3 containing the exact selected-anchor buffers.
/// No-label contours keep the original layout and entry point, so merely adding
/// contour support does not compile or bind label machinery.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_labelled_contour_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    label_gap_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_optional_extra_layout(
        device,
        shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        Some(label_gap_bgl),
        target_format,
        sample_count,
        "fs_contour_labelled",
        "figgy labelled field contour pipeline",
    )
}

#[allow(clippy::too_many_arguments)]
fn create_field_columnar_pipeline_with_optional_extra_layout(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    extra_bgl: Option<&wgpu::BindGroupLayout>,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    fragment_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    let layouts = [
        Some(transform_bgl),
        Some(style_bgl),
        Some(field_bgl),
        extra_bgl,
    ];
    let layout_count = if extra_bgl.is_some() { 4 } else { 3 };
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy field columnar layout"),
            bind_group_layouts: &layouts[..layout_count],
            immediate_size: 0,
        },
    );
    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
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
            depth_stencil: None,
            multisample: multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment_entry),
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

/// One columnar field (heatmap / band) series.
///
/// `field_bg` is group 2: the pool, the grid's `(base, len)` table, the contour
/// levels, the colourmap stops and `FieldParams`. It is built where the grid
/// declaration is resolved, so this layer carries no counts of its own — the
/// shader reads them from the uniform and the draw is always one quad.
pub struct ColumnFieldLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    /// Owned, not borrowed: the field's `Style` holds the *chart's*
    /// `colorbar.nan_color`, which the per-series style set does not know, so it
    /// is built where the grid declaration is resolved — the same reason the
    /// pick ring owns its style bind group.
    pub style_bg: wgpu::BindGroup,
    pub field_bg: wgpu::BindGroup,
    /// Keeps the group-2 buffers' lump charge alive for as long as anything can
    /// still draw with them.
    pub charge: crate::gpu_memory::SharedCharge,
    /// False when the declaration resolved to no cells; the draw is skipped
    /// rather than issued for an empty grid.
    pub drawable: bool,
}

/// One contour series' draw.
///
/// The **same quad as the field**, through `fs_contour`: the isolines are the
/// level set of the same bilinear interpolation, so there is no segment list, no
/// indirect draw and no per-level draw call. Per-level colour comes from the
/// group-2 `level_colors` table. Stroke coverage comes from the quadratic root
/// of the current cell's bilinear polynomial along the fragment's gradient
/// normal; it is not a claim of globally shortest Euclidean distance.
pub struct ColumnContourLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    /// group(1): the fallback colour for an unplaceable z. Unread by
    /// `fs_contour`, but the layout is shared with the fill.
    pub style_bg: wgpu::BindGroup,
    /// group(2): pool, grid table, levels, stops, params, level colours.
    pub field_bg: wgpu::BindGroup,
    /// Keeps the group-2 lump charge alive for as long as this layer can draw.
    pub charge: std::sync::Arc<crate::gpu_memory::GpuByteCharge>,
    /// False when the grid has no cell to interpolate across.
    pub drawable: bool,
    /// `None` when the series draws no labels.
    pub label: Option<ColumnContourLabel<'a>>,
}

/// The inline level labels for one contour series.
///
/// One quad per anchor, textured from the CPU-baked atlas. The serial
/// `anchor_select` pass writes the indirect instance count; the CPU neither
/// learns it nor needs to. A host override writes the same records and argument
/// quad, so there is exactly one draw path.
pub struct ColumnContourLabel<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    /// Immutable atlas plus this occurrence's explicit or exact-key automatic
    /// placement resources and their ledger charges.
    pub(crate) snapshot: crate::gpu_contour::ContourLabelSnapshot,
}

/// One columnar bar (histogram) series.
///
/// `edges` and `values` are already role-resolved by the caller from the bar's
/// orientation: `edges` is the bin-bound column, `values` the magnitudes. The
/// shader is told which screen axis the edges run along through
/// `Style.shape_id`.
pub struct ColumnBarLayer<'a> {
    pub envelope: Option<bar_envelope::Snapshot>,
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub style_map_bg: Option<&'a wgpu::BindGroup>,
    pub pool_buffer: &'a wgpu::Buffer,
    pub edges: ColumnHandle,
    pub values: ColumnHandle,
}

/// One selected histogram bin. The instance index is the only geometric
/// selection state: the vertex entry reads the exact edge/value pairs from the
/// same pool columns as the normal bar draw.
pub struct ColumnBarSelectionLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: wgpu::BindGroup,
    pub selection_bg: wgpu::BindGroup,
    pub selection_charge: crate::gpu_memory::SharedCharge,
    pub pool_buffer: &'a wgpu::Buffer,
    pub edges: ColumnHandle,
    pub values: ColumnHandle,
    pub instance: u32,
}

/// One selected matrix cell or contour level. `field_bg` is the exact group-2
/// snapshot used by the underlying field/contour draw, so the overlay cannot
/// drift onto a different lattice or level set.
pub struct ColumnFieldSelectionLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub selection_bg: wgpu::BindGroup,
    pub selection_charge: crate::gpu_memory::SharedCharge,
    pub field_bg: wgpu::BindGroup,
    pub charge: crate::gpu_memory::SharedCharge,
    pub drawable: bool,
}

/// One series' data primitives. A panel can hold multiple of these.
pub struct SeriesLayers<'a> {
    /// A line's area fill. Not drawn by `issue_series_data`: every series'
    /// fill is issued first by `issue_series_fill`, in series order, before
    /// any series body — see `draw_chart_panel_columnar`.
    pub area: Option<ColumnAreaLayer<'a>>,
    /// The field covers the whole grid, so it goes under everything else —
    /// see `issue_series_data` for the full order.
    pub field: Option<ColumnFieldLayer<'a>>,
    /// Bars are area primitives, so they are issued before the stroked ones.
    pub bar: Option<ColumnBarLayer<'a>>,
    /// Contour lines go over the field they describe and under the markers.
    pub contour: Option<ColumnContourLayer<'a>>,
    /// Arrows go over the field and contours they annotate, under the series'
    /// own stroked marks.
    pub vector: Option<vector::ColumnVectorLayer<'a>>,
    pub errorbar: Option<ColumnErrorBarDraw<'a>>,
    pub line: Option<ColumnLineLayer<'a>>,
    /// Constellation star pass over the same polyline as `line` — drawn
    /// right after it, before `scatter`. `None` everywhere else.
    pub line_extra: Option<ColumnStarLayer<'a>>,
    pub scatter: Option<ColumnScatterLayer<'a>>,
    pub selected_bars: Vec<ColumnBarSelectionLayer<'a>>,
    pub selected_fields: Vec<ColumnFieldSelectionLayer<'a>>,
    pub picked: Vec<ColumnPickRingLayer<'a>>,
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

/// Clamp a rect into `(0..target.0, 0..target.1)`. Returns `None` if the
/// clamped width or height is zero so callers can skip the draw entirely.
pub(crate) fn clamp_rect_to_target(r: Rect, target: (u32, u32)) -> Option<Rect> {
    let (tw, th) = target;
    let x0 = r.x.min(tw);
    let y0 = r.y.min(th);
    let x1 = r.x.saturating_add(r.width).min(tw);
    let y1 = r.y.saturating_add(r.height).min(th);
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 {
        None
    } else {
        Some(Rect {
            x: x0,
            y: y0,
            width: w,
            height: h,
        })
    }
}

/// Bars drawable from an `(edges, values)` pair.
///
/// `n` bins need `n + 1` edges, so the count is `min(edges - 1, values)` — and
/// a mismatch is **not** an error: the smallest common extent is drawn and the
/// caller reports the truncation. This is also why bars cannot
/// borrow the line draw call, which computes `min(x, y) - 1` and would drop the
/// last bar of a correctly-shaped histogram.
pub fn bar_instance_count(edge_values: usize, value_values: usize) -> u32 {
    edge_values.saturating_sub(1).min(value_values) as u32
}

/// Issue one series' area fill. Called for every series before any series
/// body (`draw_chart_panel_columnar`): the data pass is "every fill → every
/// body → every picked highlight", in SSOT series order with no sorting, so
/// a fill composites over earlier fills and under every line.
pub(crate) fn issue_series_fill(pass: &mut wgpu::RenderPass<'_>, series: &SeriesLayers<'_>) {
    let Some(a) = series.area.as_ref() else {
        return;
    };
    let count = a.x.len_values.min(a.y.len_values).min(a.base.len_values) as u32;
    if count < 2 {
        return;
    }
    pass.set_pipeline(a.pipeline);
    pass.set_bind_group(0, a.transform_bg, &[]);
    pass.set_bind_group(1, a.style_bg, &[]);
    let x_full = a.x.byte_range();
    let y_full = a.y.byte_range();
    let base_full = a.base.byte_range();
    let shift = crate::data::COLUMN_VALUE_BYTES as u64;
    pass.set_vertex_buffer(0, a.pool_buffer.slice(x_full.clone()));
    pass.set_vertex_buffer(1, a.pool_buffer.slice(y_full.clone()));
    pass.set_vertex_buffer(2, a.pool_buffer.slice((x_full.start + shift)..x_full.end));
    pass.set_vertex_buffer(3, a.pool_buffer.slice((y_full.start + shift)..y_full.end));
    // The second y column, one logical value apart like x/y, so instance i
    // sees base[i] and base[i + 1].
    pass.set_vertex_buffer(4, a.pool_buffer.slice(base_full.clone()));
    pass.set_vertex_buffer(
        5,
        a.pool_buffer
            .slice((base_full.start + shift)..base_full.end),
    );
    pass.draw(0..a.verts_per_instance, 0..(count - 1));
}

/// Issue draw calls for one series' data primitives. The caller must have
/// already set the viewport (panel) and scissor (data_area).
pub(crate) fn issue_series_data(pass: &mut wgpu::RenderPass<'_>, series: &SeriesLayers<'_>) {
    // The field is the backdrop: it paints every cell of the grid, so bars,
    // contour lines and markers all have to land on top of it.
    if let Some(f) = series.field.as_ref().filter(|f| f.drawable) {
        pass.set_pipeline(f.pipeline);
        pass.set_bind_group(0, f.transform_bg, &[]);
        pass.set_bind_group(1, &f.style_bg, &[]);
        pass.set_bind_group(2, &f.field_bg, &[]);
        pass.draw(0..FIELD_VERTICES, 0..1);
    }

    // Bars next: they are filled areas, and the stroked primitives below must
    // land on top of them rather than under.
    if let Some(b) = series.bar.as_ref() {
        let count = bar_instance_count(b.edges.len_values, b.values.len_values);
        if count > 0 {
            pass.set_pipeline(b.pipeline);
            pass.set_bind_group(0, b.transform_bg, &[]);
            pass.set_bind_group(1, b.style_bg, &[]);
            if let Some(map) = b.style_map_bg {
                pass.set_bind_group(2, map, &[]);
            }
            let edges = b.edges.byte_range();
            // The same column one logical value along — instance i then sees
            // `edges[i]` and `edges[i + 1]`.
            let edges_next = (edges.start + crate::data::COLUMN_VALUE_BYTES as u64)..edges.end;
            pass.set_vertex_buffer(0, b.pool_buffer.slice(edges.clone()));
            pass.set_vertex_buffer(1, b.pool_buffer.slice(edges_next));
            pass.set_vertex_buffer(2, b.pool_buffer.slice(b.values.byte_range()));
            pass.draw(0..BAR_VERTICES_PER_INSTANCE, 0..count);
            if let Some(envelope) = &b.envelope {
                envelope.draw(pass, b.transform_bg, b.style_bg);
            }
        }
    }

    // Contour lines: over the field they describe, under everything stroked for
    // the data itself. One draw of the same quad the fill uses — `fs_contour`
    // finds every level in one fragment pass, so the cost is the data area's
    // pixels rather than the grid's cells.
    if let Some(c) = series.contour.as_ref().filter(|c| c.drawable) {
        pass.set_pipeline(c.pipeline);
        pass.set_bind_group(0, c.transform_bg, &[]);
        pass.set_bind_group(1, &c.style_bg, &[]);
        pass.set_bind_group(2, &c.field_bg, &[]);
        if let Some(label) = c.label.as_ref() {
            pass.set_bind_group(3, label.snapshot.label_gap_bind_group(), &[]);
        }
        pass.draw(0..FIELD_VERTICES, 0..1);
        // Labels over the lines they name, still inside the data pass — so the
        // `data_area` scissor trims them and the decoration layer (axes, legend,
        // colourbar) composites on top, which is the order a legend box covering
        // a label needs.
        if let Some(l) = c.label.as_ref() {
            pass.set_pipeline(l.pipeline);
            pass.set_bind_group(0, c.transform_bg, &[]);
            pass.set_bind_group(1, l.snapshot.bind_group(), &[]);
            pass.set_vertex_buffer(0, l.snapshot.anchors().slice(..));
            pass.draw_indirect(l.snapshot.indirect(), 0);
        }
    }

    // Arrows: over the field and contours, under errorbars, lines and markers.
    // No vertex buffers; `vs_main` reads sample `instance_index` from group 2.
    if let Some(v) = series.vector.as_ref().filter(|v| v.count > 0) {
        pass.set_pipeline(v.pipeline);
        pass.set_bind_group(0, v.transform_bg, &[]);
        pass.set_bind_group(1, &v.style_bg, &[]);
        pass.set_bind_group(2, &v.vector_bg, &[]);
        pass.draw(0..vector::ARROW_VERTICES, 0..v.count);
    }

    if let Some(eb) = series.errorbar.as_ref() {
        let count = [
            eb.x.len_values,
            eb.y.len_values,
            eb.err_y_lo.len_values,
            eb.err_y_hi.len_values,
            eb.err_x_lo.len_values,
            eb.err_x_hi.len_values,
        ]
        .into_iter()
        .min()
        .unwrap_or(0) as u32;
        if count > 0 {
            pass.set_pipeline(eb.pipeline);
            pass.set_bind_group(0, eb.transform_bg, &[]);
            pass.set_bind_group(1, eb.style_bg, &[]);
            if let Some(map) = eb.style_map_bg {
                pass.set_bind_group(2, map, &[]);
            }
            pass.set_vertex_buffer(0, eb.pool_buffer.slice(eb.x.byte_range()));
            pass.set_vertex_buffer(1, eb.pool_buffer.slice(eb.y.byte_range()));
            pass.set_vertex_buffer(2, eb.pool_buffer.slice(eb.err_y_lo.byte_range()));
            pass.set_vertex_buffer(3, eb.pool_buffer.slice(eb.err_y_hi.byte_range()));
            pass.set_vertex_buffer(4, eb.pool_buffer.slice(eb.err_x_lo.byte_range()));
            pass.set_vertex_buffer(5, eb.pool_buffer.slice(eb.err_x_hi.byte_range()));
            if eb.style_map_bg.is_some() {
                let style_index = eb.style_index.as_ref().unwrap_or(&eb.x);
                pass.set_vertex_buffer(6, eb.pool_buffer.slice(style_index.byte_range()));
            }
            // 36 vertices = 6 quads × 2 triangles (see errorbar_columnar.wgsl).
            pass.draw(0..36, 0..count);
        }
    }

    if let Some(l) = series.line.as_ref() {
        draw_line_layer(pass, l);
    }
    if let Some(s) = series.line_extra.as_ref() {
        // Constellation star pass: indirect draw — the instance count was
        // computed on the GPU from the polyline's total arc (line_arc.wgsl
        // star_indirect), so the CPU never sees, nor caps, the star count.
        pass.set_pipeline(s.pipeline);
        pass.set_bind_group(0, s.transform_bg, &[]);
        pass.set_bind_group(1, s.style_bg, &[]);
        pass.set_bind_group(2, s.texture_bg, &[]);
        pass.set_bind_group(3, s.star_bg, &[]);
        pass.draw_indirect(s.indirect, 0);
    }

    if let Some(s) = series.scatter.as_ref() {
        let count = s.x.len_values.min(s.y.len_values) as u32;
        if count > 0 {
            pass.set_pipeline(s.pipeline);
            pass.set_bind_group(0, s.transform_bg, &[]);
            pass.set_bind_group(1, s.style_bg, &[]);
            if let Some(map) = s.style_map_bg {
                pass.set_bind_group(2, map, &[]);
                pass.set_bind_group(
                    3,
                    s.value_map_bg
                        .expect("mapped scatter draw binds its value map group"),
                    &[],
                );
            } else if let Some(tex) = s.texture_bg {
                pass.set_bind_group(2, tex, &[]);
            }
            pass.set_vertex_buffer(0, s.quad_vb.slice(..));
            pass.set_vertex_buffer(1, s.pool_buffer.slice(s.x.byte_range()));
            pass.set_vertex_buffer(2, s.pool_buffer.slice(s.y.byte_range()));
            if s.style_map_bg.is_some() {
                let style_index = s.style_index.as_ref().unwrap_or(&s.x);
                pass.set_vertex_buffer(3, s.pool_buffer.slice(style_index.byte_range()));
                let color_value = s.color_value.as_ref().unwrap_or(&s.x);
                pass.set_vertex_buffer(4, s.pool_buffer.slice(color_value.byte_range()));
                let size_value = s.size_value.as_ref().unwrap_or(&s.x);
                pass.set_vertex_buffer(5, s.pool_buffer.slice(size_value.byte_range()));
            }
            pass.draw(0..4, 0..count);
        }
    }
}

/// Draw one chart panel: grid → every series → decoration. The function
/// configures viewport and scissor itself, so callers only supply rects.
///
/// `target_size` is the pixel size of the current color attachment;
/// `panel_rect` / `data_area` are clamped to it to avoid wgpu validation
/// errors when a panel partially exits the surface.
pub(crate) fn issue_series_picked(pass: &mut wgpu::RenderPass<'_>, series: &SeriesLayers<'_>) {
    for selection in &series.selected_bars {
        pass.set_pipeline(selection.pipeline);
        pass.set_bind_group(0, selection.transform_bg, &[]);
        pass.set_bind_group(1, &selection.style_bg, &[]);
        pass.set_bind_group(2, &selection.selection_bg, &[]);
        let edges = selection.edges.byte_range();
        let edges_next = (edges.start + crate::data::COLUMN_VALUE_BYTES as u64)..edges.end;
        pass.set_vertex_buffer(0, selection.pool_buffer.slice(edges.clone()));
        pass.set_vertex_buffer(1, selection.pool_buffer.slice(edges_next));
        pass.set_vertex_buffer(
            2,
            selection.pool_buffer.slice(selection.values.byte_range()),
        );
        pass.draw(
            0..BAR_SELECTION_VERTICES,
            selection.instance..selection.instance + 1,
        );
    }

    for selection in series.selected_fields.iter().filter(|layer| layer.drawable) {
        pass.set_pipeline(selection.pipeline);
        pass.set_bind_group(0, selection.transform_bg, &[]);
        pass.set_bind_group(1, &selection.selection_bg, &[]);
        pass.set_bind_group(2, &selection.field_bg, &[]);
        // A labelled contour's highlight pipeline reads the label gaps the
        // contour draw leaves; the plain one ignores the extra group.
        if let Some(label) = series.contour.as_ref().and_then(|c| c.label.as_ref()) {
            pass.set_bind_group(3, label.snapshot.label_gap_bind_group(), &[]);
        }
        pass.draw(0..FIELD_VERTICES, 0..1);
    }

    for p in &series.picked {
        pass.set_pipeline(p.pipeline);
        pass.set_bind_group(0, p.transform_bg, &[]);
        pass.set_bind_group(1, &p.style_bg, &[]);
        if let Some(map) = p.style_map_bg {
            pass.set_bind_group(2, map, &[]);
            pass.set_bind_group(
                3,
                p.value_map_bg
                    .expect("mapped pick ring draw binds its value map group"),
                &[],
            );
        }
        pass.set_vertex_buffer(0, p.quad_vb.slice(..));
        pass.set_vertex_buffer(1, p.pool_buffer.slice(p.x.byte_range()));
        pass.set_vertex_buffer(2, p.pool_buffer.slice(p.y.byte_range()));
        if p.style_map_bg.is_some() {
            let style_index = p.style_index.as_ref().unwrap_or(&p.x);
            pass.set_vertex_buffer(3, p.pool_buffer.slice(style_index.byte_range()));
            let color_value = p.color_value.as_ref().unwrap_or(&p.x);
            pass.set_vertex_buffer(4, p.pool_buffer.slice(color_value.byte_range()));
            let size_value = p.size_value.as_ref().unwrap_or(&p.x);
            pass.set_vertex_buffer(5, p.pool_buffer.slice(size_value.byte_range()));
        }
        pass.draw(0..4, p.instance..p.instance + 1);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_chart_panel_columnar(
    pass: &mut wgpu::RenderPass<'_>,
    target_size: (u32, u32),
    panel_rect: Rect,
    data_area: Rect,
    grid: AxisLayer<'_>,
    annotation_underlay: Option<AxisLayer<'_>>,
    series_list: &[SeriesLayers<'_>],
    decoration: AxisLayer<'_>,
    annotation_overlay: Option<AxisLayer<'_>>,
) {
    draw_chart_panel_with_bodies(
        pass,
        target_size,
        panel_rect,
        data_area,
        grid,
        annotation_underlay,
        series_list,
        None,
        decoration,
        annotation_overlay,
    );
}

/// Every series' fill, then every body, each loop in SSOT series order: the
/// part of a panel a resident composite caches. The fill loop comes first by
/// construction so another series' fill can never cover this series' line.
pub(crate) fn issue_series_bodies(
    pass: &mut wgpu::RenderPass<'_>,
    series_list: &[SeriesLayers<'_>],
) {
    for s in series_list {
        issue_series_fill(pass, s);
    }
    for s in series_list {
        issue_series_data(pass, s);
    }
}

/// [`draw_chart_panel_columnar`], with the fills and bodies taken from
/// `cached_bodies` when given: a panel-sized texture of exactly what
/// [`issue_series_bodies`] draws. The picked highlights always come from
/// `series_list`, so a selection never needs the bodies drawn again.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_chart_panel_with_bodies(
    pass: &mut wgpu::RenderPass<'_>,
    target_size: (u32, u32),
    panel_rect: Rect,
    data_area: Rect,
    grid: AxisLayer<'_>,
    annotation_underlay: Option<AxisLayer<'_>>,
    series_list: &[SeriesLayers<'_>],
    cached_bodies: Option<AxisLayer<'_>>,
    decoration: AxisLayer<'_>,
    annotation_overlay: Option<AxisLayer<'_>>,
) {
    let Some(panel_clamped) = clamp_rect_to_target(panel_rect, target_size) else {
        return;
    };
    let Some(data_clamped) = clamp_rect_to_target(data_area, target_size) else {
        return;
    };

    pass.set_viewport(
        panel_clamped.x as f32,
        panel_clamped.y as f32,
        panel_clamped.width as f32,
        panel_clamped.height as f32,
        0.0,
        1.0,
    );

    // 1) Grid layer (under data).
    pass.set_scissor_rect(
        panel_clamped.x,
        panel_clamped.y,
        panel_clamped.width,
        panel_clamped.height,
    );
    pass.set_pipeline(grid.pipeline);
    pass.set_bind_group(0, grid.bind_group, &[]);
    pass.draw(0..3, 0..1);

    // 1b) Annotation underlay (spans): below the data, above the grid.
    if let Some(layer) = annotation_underlay {
        pass.set_pipeline(layer.pipeline);
        pass.set_bind_group(0, layer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // 2) Data primitives — scissor once to data_area, then issue every series:
    //    every fill, then every body, then every picked highlight, each loop
    //    in SSOT series order. The fill loop comes first by construction so
    //    another series' fill can never cover this series' line.
    pass.set_scissor_rect(
        data_clamped.x,
        data_clamped.y,
        data_clamped.width,
        data_clamped.height,
    );
    match cached_bodies {
        Some(layer) => {
            // The composite was clipped to the data area when it was drawn;
            // clipping its blit again would apply the data scissor twice at
            // the edge samples, which a direct draw applies once.
            pass.set_scissor_rect(
                panel_clamped.x,
                panel_clamped.y,
                panel_clamped.width,
                panel_clamped.height,
            );
            pass.set_pipeline(layer.pipeline);
            pass.set_bind_group(0, layer.bind_group, &[]);
            pass.draw(0..3, 0..1);
            pass.set_scissor_rect(
                data_clamped.x,
                data_clamped.y,
                data_clamped.width,
                data_clamped.height,
            );
        }
        None => issue_series_bodies(pass, series_list),
    }
    for s in series_list {
        issue_series_picked(pass, s);
    }

    // 3) Decoration layer (over data).
    pass.set_scissor_rect(
        panel_clamped.x,
        panel_clamped.y,
        panel_clamped.width,
        panel_clamped.height,
    );
    pass.set_pipeline(decoration.pipeline);
    pass.set_bind_group(0, decoration.bind_group, &[]);
    pass.draw(0..3, 0..1);

    // 4) Annotation overlay: topmost, above every decoration.
    if let Some(layer) = annotation_overlay {
        pass.set_pipeline(layer.pipeline);
        pass.set_bind_group(0, layer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

// Tests.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contour_lookup_uses_finite_prefixes_numeric_indices_and_block_metadata() {
        let mut levels: Vec<f32> = (0..35).map(|index| 100.0 - index as f32).collect();
        levels[2] = 50.0;
        levels[3] = 50.0;
        levels[4] = f32::NAN;
        levels[30] = f32::NEG_INFINITY;
        levels[32] = f32::NEG_INFINITY;
        levels[33] = f32::INFINITY;
        let ramp = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]];
        let lookup = build_field_lookup_tables(&ramp, &levels).unwrap();

        assert_eq!(&lookup.stops[..ramp.len()], &ramp);
        assert_eq!(lookup.stops.len(), ramp.len() + levels.len());
        assert_eq!(
            lookup.metadata,
            vec![
                ContourLookupMetadataGpu {
                    finite_count: 30,
                    negative_infinity_count: 1,
                },
                ContourLookupMetadataGpu {
                    finite_count: 1,
                    negative_infinity_count: 1,
                },
            ]
        );

        let search = &lookup.stops[ramp.len()..];
        let first_finite = &search[..lookup.metadata[0].finite_count as usize];
        assert!(first_finite.windows(2).all(|pair| pair[0][0] <= pair[1][0]));
        assert!(search[30..32].iter().all(|record| *record == [0.0; 4]));
        assert_eq!(search[32][0], levels[34]);
        assert_eq!(search[32][1], 34.0);
        assert!(search[33..35].iter().all(|record| *record == [0.0; 4]));

        let duplicate_indices: Vec<u32> = first_finite
            .iter()
            .filter(|record| record[0] == 50.0)
            .map(|record| record[1] as u32)
            .collect();
        assert_eq!(duplicate_indices, vec![2, 3]);
        for record in first_finite {
            let original = record[1] as usize;
            assert_eq!(record[1], original as f32);
            assert_eq!(record[0], levels[original]);
        }
    }

    #[test]
    fn contour_lookup_pads_empty_metadata_and_preserves_31_32_1023_indices() {
        let empty = build_field_lookup_tables(&[[0.0; 4]], &[]).unwrap();
        assert_eq!(empty.stops, vec![[0.0; 4]]);
        assert_eq!(empty.metadata, vec![ContourLookupMetadataGpu::default()]);

        let allowed = vec![0.0; CONTOUR_LEVEL_LOOKUP_CAPACITY];
        let lookup = build_field_lookup_tables(&[[0.0; 4]], &allowed).unwrap();
        assert_eq!(lookup.stops.len(), 1 + CONTOUR_LEVEL_LOOKUP_CAPACITY);
        assert_eq!(lookup.metadata.len(), CONTOUR_LEVEL_BLOCK_COUNT);
        let search = &lookup.stops[1..];
        for index in [31usize, 32, 1023] {
            let block = index / CONTOUR_LEVEL_BLOCK_SIZE;
            let start = block * CONTOUR_LEVEL_BLOCK_SIZE;
            let end = start + lookup.metadata[block].finite_count as usize;
            let record = search[start..end]
                .iter()
                .find(|record| record[1] == index as f32)
                .unwrap_or_else(|| panic!("numeric original index {index} was not preserved"));
            assert_eq!(record[1].to_bits(), (index as f32).to_bits());
        }

        let rejected = vec![0.0; CONTOUR_LEVEL_LOOKUP_CAPACITY + 1];
        assert!(matches!(
            build_field_lookup_tables(&[[0.0; 4]], &rejected),
            Err(crate::FiggyError::StateAllocationFailed {
                resource: "contour level lookup",
                ..
            })
        ));
    }

    #[test]
    fn contour_lookup_dimensions_match_the_model_and_shader() {
        assert_eq!(CONTOUR_LEVEL_BLOCK_SIZE, 32);
        assert_eq!(CONTOUR_LEVEL_BLOCK_COUNT, 32);
        assert_eq!(
            CONTOUR_LEVEL_LOOKUP_CAPACITY,
            crate::data_config::MAX_CONTOUR_LEVELS
        );

        let shader = include_str!("field_columnar.wgsl");
        assert!(shader.contains("const CONTOUR_LEVEL_BLOCK_SIZE: u32 = 32u;"));
        assert!(shader.contains("let original = firstTrailingBit(candidates);"));
        assert!(shader.contains("let original = u32(record.y) - start;"));
        assert!(shader.contains("contour_lookup_metadata[block].finite_count"));
        assert!(shader.contains("metadata.negative_infinity_count"));
        assert!(shader.contains("let search_lo = s.z_lo - margin;"));
        assert!(shader.contains("let search_hi = s.z_hi + margin;"));
        assert!(shader.contains("contour_lower_bound(start, count, search_lo)"));
        assert!(shader.contains("contour_upper_bound(start, count, search_hi)"));
        assert!(
            !shader.contains("for (var i = 0u; i < field.level_count"),
            "contour fragments must not return to a full level scan"
        );
    }

    #[test]
    fn field_table_exact_charge_includes_zero_level_lookup_metadata_padding() {
        let Some((device, _queue)) = shared_device() else {
            return;
        };
        let ledger = std::sync::Arc::new(crate::gpu_memory::GpuLedger::new());
        let layout = create_field_data_bind_group_layout(&device);
        let pool = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("field table charge test pool"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let lookup = build_field_lookup_tables(&[[0.0; 4]], &[]).unwrap();
        let params = <FieldParamsGpu as bytemuck::Zeroable>::zeroed();
        let (_bind_group, charge) = create_field_data_bind_group(
            &device,
            &ledger,
            &layout,
            &pool,
            FieldTables {
                grid: &[GridColumnGpu { base: 0, len: 0 }],
                levels: &[0.0],
                stops: &lookup.stops,
                level_colors: &[[0.0; 4]],
                lookup_metadata: &lookup.metadata,
                params: &params,
            },
        );

        let expected = std::mem::size_of::<GridColumnGpu>()
            + std::mem::size_of::<f32>()
            + std::mem::size_of::<[f32; 4]>()
            + std::mem::size_of::<[f32; 4]>()
            + std::mem::size_of::<ContourLookupMetadataGpu>()
            + std::mem::size_of::<FieldParamsGpu>();
        assert_eq!(expected, 116);
        assert_eq!(
            ledger
                .snapshot()
                .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            expected as u64
        );
        drop(charge);
        assert_eq!(
            ledger
                .snapshot()
                .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            0
        );
    }

    /// Instance creation must not panic, even on driver-less environments
    /// (it does not talk to any GPU yet).
    #[test]
    fn instance_creation_succeeds() {
        let _instance = create_instance();
    }

    /// Print adapter info; a missing adapter is a test failure.
    /// Mostly useful locally with `cargo test -- --nocapture`.
    #[test]
    fn adapter_request_prints_info_when_available() {
        let instance = create_instance();
        match request_adapter(&instance) {
            Ok(adapter) => {
                let info = adapter.get_info();
                println!("adapter name    : {}", info.name);
                println!("adapter backend : {:?}", info.backend);
                println!("adapter type    : {:?}", info.device_type);
                println!("adapter driver  : {} / {}", info.driver, info.driver_info);
            }
            Err(e) => {
                panic!("required GPU test: adapter initialization failed: {e}");
            }
        }
    }

    /// The constellation star picks on the disc its quad inscribes
    /// (SHADER_COMMON.md §5.7): the sprite is zero from 0.98 of the half
    /// width out, so even a bilinear tap a texel further stays inside it.
    #[test]
    fn psf_sprite_is_zero_outside_the_inscribed_disc() {
        let size = 128u32;
        let psf = bake_psf_rgba(size);
        let half = (size as f32 - 1.0) * 0.5;
        for y in 0..size {
            for x in 0..size {
                let dx = (x as f32 - half) / half;
                let dy = (y as f32 - half) / half;
                if (dx * dx + dy * dy).sqrt() >= 0.98 {
                    let i = ((y * size + x) * 4) as usize;
                    assert_eq!((psf[i], psf[i + 1]), (0, 0), "texel ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn psf_sprite_fades_to_zero_at_square_edges() {
        let psf = bake_psf_rgba(128);
        let px = |x: usize, y: usize, c: usize| psf[(y * 128 + x) * 4 + c];

        for i in 0..128 {
            assert_eq!(px(i, 0, 0), 0, "top edge core leaked at x={i}");
            assert_eq!(px(i, 0, 1), 0, "top edge halo leaked at x={i}");
            assert_eq!(px(i, 127, 0), 0, "bottom edge core leaked at x={i}");
            assert_eq!(px(i, 127, 1), 0, "bottom edge halo leaked at x={i}");
            assert_eq!(px(0, i, 0), 0, "left edge core leaked at y={i}");
            assert_eq!(px(0, i, 1), 0, "left edge halo leaked at y={i}");
            assert_eq!(px(127, i, 0), 0, "right edge core leaked at y={i}");
            assert_eq!(px(127, i, 1), 0, "right edge halo leaked at y={i}");
        }
    }

    #[test]
    fn log_transform_guards_manual_range_without_raising_tiny_positive_min() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
        }

        config.bottom_x.scale = crate::config::AxisScale::Logarithmic;
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 1000.0;
        let t = scatter_transform_from_config(&config);
        assert_eq!(t.scale_log[0], 1.0);
        assert!((t.data_min[0] + 12.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] - 3.0).abs() < 1.0e-6, "{:?}", t.data_max);

        config.bottom_x.min = 1.0e-15;
        config.bottom_x.max = 1.0e-12;
        let t = scatter_transform_from_config(&config);
        assert!((t.data_min[0] + 15.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] + 12.0).abs() < 1.0e-6, "{:?}", t.data_max);

        config.bottom_x.min = 10.0;
        config.bottom_x.max = 1.0;
        let t = scatter_transform_from_config(&config);
        assert!((t.data_min[0] - 1.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] - 2.0).abs() < 1.0e-6, "{:?}", t.data_max);
    }

    #[test]
    fn scatter_transform_inverted_axes_swap_final_ranges() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 160,
            height: 120,
        });
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 8.0;
            axis.major_tick_length = 0.0;
        }
        config.bottom_x.scale = crate::config::AxisScale::Logarithmic;
        config.bottom_x.min = 1.0;
        config.bottom_x.max = 100.0;
        config.left_y.min = -5.0;
        config.left_y.max = 5.0;

        let normal = scatter_transform_from_config(&config);
        config.bottom_x.inverted = true;
        config.left_y.inverted = true;
        let inverted = scatter_transform_from_config(&config);

        assert_eq!(inverted.scale_log, normal.scale_log);
        assert!((inverted.data_min[0] - normal.data_max[0]).abs() < 1.0e-6);
        assert!((inverted.data_max[0] - normal.data_min[0]).abs() < 1.0e-6);
        assert!((inverted.data_min[1] - normal.data_max[1]).abs() < 1.0e-6);
        assert!((inverted.data_max[1] - normal.data_min[1]).abs() < 1.0e-6);
        for v in inverted.data_min.into_iter().chain(inverted.data_max) {
            assert!(v.is_finite(), "{inverted:?}");
        }
    }

    #[test]
    fn gpu_transform_uses_auto_fit_ssot_bounds_bit_for_bit() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 17,
            y: 29,
            width: 900,
            height: 600,
        });
        let x_extent = crate::chart::FitExtent {
            min: 1_700_000_000_000.125,
            max: 1_700_000_000_009.875,
            min_positive: Some(1_700_000_000_000.125),
        };
        let y_extent = crate::chart::FitExtent {
            min: -3.25,
            max: 8.75,
            min_positive: Some(0.125),
        };
        crate::chart::apply_auto_fit_all(&mut config, &x_extent, &y_extent, 0.05);

        let transform = scatter_transform_from_config(&config);
        let x_min = crate::data::split_f64_to_f32_pair(config.bottom_x.min);
        let x_max = crate::data::split_f64_to_f32_pair(config.bottom_x.max);
        let y_min = crate::data::split_f64_to_f32_pair(config.left_y.min);
        let y_max = crate::data::split_f64_to_f32_pair(config.left_y.max);
        assert_eq!(
            [transform.data_min[0], transform.data_min_lo[0]],
            [x_min.0, x_min.1]
        );
        assert_eq!(
            [transform.data_max[0], transform.data_max_lo[0]],
            [x_max.0, x_max.1]
        );
        assert_eq!(
            [transform.data_min[1], transform.data_min_lo[1]],
            [y_min.0, y_min.1]
        );
        assert_eq!(
            [transform.data_max[1], transform.data_max_lo[1]],
            [y_max.0, y_max.1]
        );

        // Layout changes only the panel affine. They can no longer manufacture
        // a second, margin-extended axis range behind Config's back.
        let mut different_layout = config.clone();
        different_layout.bottom_x.out_margin += 40.0;
        different_layout.left_y.out_margin += 30.0;
        let moved = scatter_transform_from_config(&different_layout);
        assert_eq!(moved.data_min, transform.data_min);
        assert_eq!(moved.data_max, transform.data_max);
        assert_eq!(moved.data_min_lo, transform.data_min_lo);
        assert_eq!(moved.data_max_lo, transform.data_max_lo);
        assert_ne!(moved.data_to_panel_offset, transform.data_to_panel_offset);
        assert_ne!(moved.data_to_panel_scale, transform.data_to_panel_scale);
    }

    /// Smoke-test the texture-upload API path (no readback). Validation
    /// would surface here if the call shape were wrong.
    #[test]
    fn rgba_texture_upload_roundtrips_api() {
        let Some((device, queue)) = shared_device() else {
            println!("no adapter — skipping texture upload test");
            return;
        };

        // 2x2 checkerboard: 4 RGBA pixels = 16 bytes.
        let rgba: [u8; 16] = [
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 0, 255, // yellow
        ];
        let tex = upload_rgba_texture(
            &device,
            &queue,
            device.limits().max_texture_dimension_2d,
            2,
            2,
            &rgba,
        )
        .expect("upload texture");

        // Wait for the queued write_texture to complete; validation errors
        // surface during this poll.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });

        assert_eq!(tex.width(), 2);
        assert_eq!(tex.height(), 2);
        assert_eq!(tex.format(), wgpu::TextureFormat::Rgba8Unorm);
    }

    /// Compile the WGSL and build the fullscreen textured pipeline. Shader
    /// or layout mismatches would panic during creation.
    #[test]
    fn fullscreen_textured_pipeline_compiles_and_creates() {
        let Some((device, _queue)) = shared_device() else {
            println!("no adapter — skipping pipeline test");
            return;
        };

        let bgl = create_texture_bind_group_layout(&device);
        let pipeline = create_fullscreen_textured_pipeline(
            &device,
            &bgl,
            // Same non-sRGB format the surface picks at runtime.
            wgpu::TextureFormat::Bgra8Unorm,
        );

        let _ = pipeline;
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    #[test]
    fn streamed_point_style_map_reuses_rows_and_charges_only_meta() {
        let (device, _queue) = shared_device().expect("streamed point style map GPU adapter");
        let layout = create_scatter_style_map_bind_group_layout(&device);
        let map = create_scatter_style_map(
            &device,
            &layout,
            &[ScatterStyleSlotGpu {
                color_premul: [1.0, 0.0, 0.0, 1.0],
                meta: [3.0, 0.0, 1.0, 0.0],
            }],
            &[ScatterStyleOverrideGpu {
                point_index: 17,
                _pad: [0; 3],
                color_premul: [0.0, 1.0, 0.0, 1.0],
                meta: [3.0, 0.0, 1.0, 0.0],
            }],
            ScatterStyleMapMeta {
                style_count: 1,
                override_count: 1,
                has_index: 1,
                _pad: 0,
            },
        );
        assert_eq!(map.meta._pad, 0);
        let tally = crate::gpu_memory::ChargeTally::new();
        let streamed = map.stream_bind_group(&device, &layout, 17, &tally);
        assert_eq!(tally.bytes(), 16);
        drop(map);
        let _ = streamed;
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Compile the scatter WGSL entries added outside the common block for
    /// per-point style mapping and picked-point decoration. This catches WGSL
    /// syntax/layout drift without running a full render snapshot.
    #[test]
    fn scatter_mapped_and_pick_ring_pipelines_compile() {
        let Some((device, _queue)) = shared_device() else {
            println!("no adapter ??skipping scatter entry pipeline test");
            return;
        };
        let transform_bgl = create_scatter_transform_bind_group_layout(&device);
        let style_bgl = create_style_bind_group_layout(&device);
        let style_map_bgl = create_scatter_style_map_bind_group_layout(&device);
        let value_map_bgl = create_point_value_map_bind_group_layout(&device);
        let shaders = ShaderModules::new(&device);

        let mapped = create_scatter_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            &value_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
        );
        let picked = create_scatter_columnar_pipeline_with_entries(
            &device,
            &shaders.scatter,
            &transform_bgl,
            &style_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
            "vs_pick_ring",
            "fs_pick_ring",
            "figgy picked point ring pipeline test",
        );
        let picked_mapped = create_scatter_columnar_mapped_pipeline_with_entries(
            &device,
            &shaders.scatter,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            &value_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
            "vs_pick_ring_mapped",
            "fs_pick_ring",
            "figgy picked point mapped ring pipeline test",
        );
        let errorbar_mapped = create_errorbar_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
        );
        let bar_mapped = create_bar_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
        );

        let _ = (mapped, picked, picked_mapped, errorbar_mapped, bar_mapped);
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Wire up sampler + bind-group layout + bind group end-to-end; a
    /// slot-type mismatch would panic in `create_bind_group`.
    #[test]
    fn texture_sampler_bind_group_wires_up() {
        let Some((device, queue)) = shared_device() else {
            println!("no adapter — skipping bind group test");
            return;
        };

        let rgba: [u8; 16] = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let texture = upload_rgba_texture(
            &device,
            &queue,
            device.limits().max_texture_dimension_2d,
            2,
            2,
            &rgba,
        )
        .expect("upload texture");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = create_linear_sampler(&device);
        let layout = create_texture_bind_group_layout(&device);
        let _bind_group = create_texture_bind_group(&device, &layout, &view, &sampler);

        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Open a device + queue without a surface and print a few limits.
    /// Fails when no adapter is available.
    #[test]
    fn device_request_opens_device_and_queue() {
        let instance = create_instance();
        let adapter =
            request_adapter(&instance).expect("required GPU test: adapter initialization failed");
        match request_device(&adapter) {
            Ok((device, _queue)) => {
                let limits = device.limits();
                println!("device opened OK");
                println!(
                    "  max_texture_dim_2d     : {}",
                    limits.max_texture_dimension_2d
                );
                println!("  max_buffer_size        : {}", limits.max_buffer_size);
                println!("  max_bind_groups        : {}", limits.max_bind_groups);
            }
            Err(e) => panic!("request_device failed on available adapter: {e}"),
        }
    }
}
