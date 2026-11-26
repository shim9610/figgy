//! Error-bar series: pipelines and the error-bar draw.

use super::*;

/// The assembled error-bar module.
pub(crate) const ERRORBAR_SHADER_BY_SET: crate::shader_assembly::AssembledBySet = crate::shader_assembly::assembled_by_set!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    set,
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/common/grid_pair.wgsl",
    "data_render/common/arc_pair.wgsl",
    "data_render/common/sketch_noise.wgsl",
    "data_render/common/line_stroke.wgsl",
    "data_render/common/errorbar_segment.wgsl",
    "data_render/errorbar_columnar.wgsl",
);
/// The full assembly of [`ERRORBAR_SHADER_BY_SET`]: the module a chart with an axis
/// break, a probability axis or polar coordinates compiles.
pub(crate) const ERRORBAR_SHADER: crate::shader_assembly::Assembled = ERRORBAR_SHADER_BY_SET.full;

/// Bytes per value of every error-bar slot.
const ERRORBAR_SLOT_STRIDE: wgpu::BufferAddress =
    crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;

/// The attribute of each per-instance vertex slot the error-bar entries
/// read, in slot order: x, y, err_y_lo, err_y_hi, err_x_lo, err_x_hi, then
/// the mapped draw's f32 style index at 6.
const ERRORBAR_SLOT_ATTRIBUTES: [[wgpu::VertexAttribute; 1]; 7] = {
    let mut attrs = [[wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    }]; 7];
    let mut i = 0;
    while i < 7 {
        attrs[i][0].shader_location = i as u32;
        i += 1;
    }
    attrs[6][0].format = wgpu::VertexFormat::Float32;
    attrs
};

/// The first `slots` error-bar vertex slots.
fn errorbar_buffers<const SLOTS: usize>() -> [Option<wgpu::VertexBufferLayout<'static>>; SLOTS] {
    std::array::from_fn(|slot| {
        Some(wgpu::VertexBufferLayout {
            array_stride: ERRORBAR_SLOT_STRIDE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ERRORBAR_SLOT_ATTRIBUTES[slot],
        })
    })
}

/// The colour-mapped error bars (`vs_mapped` / `fs_mapped`): the precise
/// slots plus the style index at slot 6, the series' style map as group 2.
pub(crate) fn create_errorbar_mapped_pipeline(
    device: &wgpu::Device,
    cache: Option<&wgpu::PipelineCache>,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let layout = RenderPipelineSpec::layout(
        device,
        "figgy errorbar mapped layout",
        &[Some(transform_bgl), Some(style_bgl), Some(style_map_bgl)],
    );
    RenderPipelineSpec {
        label: "figgy errorbar mapped pipeline",
        layout: &layout,
        shader,
        entries: ("vs_mapped", "fs_mapped"),
        buffers: &errorbar_buffers::<7>(),
        topology: wgpu::PrimitiveTopology::TriangleList,
        depth_stencil: None,
        multisample: multisample_state(sample_count),
        target: blended_target(
            target_format,
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        ),
    }
    .create(device, cache)
}

/// An error-bar pipeline with `(vertex entry, fragment entry, label)` and
/// `blend`. Slots 0..5 are per-instance f32: x, y, err_y_lo, err_y_hi,
/// err_x_lo, err_x_hi. The precise draw (`vs_main` / `fs_main`) emits 36
/// vertices per instance on a `TriangleList`: six axis-aligned quads (Y
/// stem, caps @ y_lo/y_hi, X stem, caps @ x_lo/x_hi), expanded in the vertex
/// shader by half their pixel stroke width (`Style::line_width_px` for
/// stems, `cap_width_px` for caps; caps span ±`cap_half_px`). A direction
/// whose err columns sum to <= 0 collapses to zero-area quads, so its stem
/// and caps draw nothing. Styled variants (the sketch `vs_sketch`, the
/// constellation jets with their own fragment entry and additive blending)
/// share the layout and state, and a stem series (#206) draws `vs_stem` (or
/// `vs_stem_sketch`) into `fs_stem`, one quad per instance.
pub(crate) fn create_errorbar_pipeline(
    device: &wgpu::Device,
    cache: Option<&wgpu::PipelineCache>,
    shader: &wgpu::ShaderModule,
    [transform_bgl, style_bgl]: [&wgpu::BindGroupLayout; 2],
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    (vs_entry, fs_entry, label): (&str, &str, &str),
    blend: wgpu::BlendState,
) -> wgpu::RenderPipeline {
    let layout = RenderPipelineSpec::layout(
        device,
        "figgy errorbar columnar layout",
        &[Some(transform_bgl), Some(style_bgl)],
    );
    RenderPipelineSpec {
        label,
        layout: &layout,
        shader,
        entries: (vs_entry, fs_entry),
        buffers: &errorbar_buffers::<6>(),
        topology: wgpu::PrimitiveTopology::TriangleList,
        depth_stencil: None,
        multisample: multisample_state(sample_count),
        target: blended_target(target_format, blend),
    }
    .create(device, cache)
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
