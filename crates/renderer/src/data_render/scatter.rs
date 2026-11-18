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
pub(crate) const SCATTER_SHADER_BY_SET: crate::shader_assembly::AssembledBySet = crate::shader_assembly::assembled_by_set!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    set,
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/common/project_3d.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/sketch_noise.wgsl",
    "data_render/common/marker.wgsl",
    "data_render/scatter_columnar.wgsl",
);
/// The full assembly of [`SCATTER_SHADER_BY_SET`]: the module a chart with an axis
/// break, a probability axis or polar coordinates compiles.
pub(crate) const SCATTER_SHADER: crate::shader_assembly::Assembled = SCATTER_SHADER_BY_SET.full;

/// The mapped scatter pipeline over `[transform, style, style map, value
/// map]` with `(vertex entry, fragment entry, label)`: the precise draw
/// (`vs_mapped` / `fs_mapped`) and the picked ring's, which resolve per-point
/// style slots through separate entry points and a fourth per-instance f32
/// vertex slot for `point_style_index_column` when present (sparse-only
/// mappings bind the X column into that slot as inert filler), plus the
/// value-map group (3) and the two value-column vertex slots (4, 5). With
/// `third`, the third component of a ternary chart's entries
/// (`vs_ternary_mapped`, `vs_pick_ring_ternary_mapped`) or a 3D chart's
/// height at slot 6; with `three_d`, the pass's depth state and target for
/// a 3D chart's entries (`vs_3d_mapped`, `vs_3d_mapped_pick`,
/// `vs_pick_ring_3d_mapped`), whose group 3 is the combined value map and
/// camera layout (`surface_mesh::create_view3d_value_map_bind_group_layout`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_mapped_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    [transform_bgl, style_bgl, style_map_bgl, value_map_bgl]: [&wgpu::BindGroupLayout; 4],
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    (vs_entry, fs_entry, label): (&str, &str, &str),
    third: bool,
    three_d: Option<View3dPass>,
) -> wgpu::RenderPipeline {
    let layout = RenderPipelineSpec::layout(
        device,
        label,
        &[
            Some(transform_bgl),
            Some(style_bgl),
            Some(style_map_bgl),
            Some(value_map_bgl),
        ],
    );
    // Slot 3 is the f32 style index; slot 6 a ternary chart's third
    // component or a 3D chart's height.
    let buffers: [Option<wgpu::VertexBufferLayout<'_>>; 7] = std::array::from_fn(|slot| {
        Some(wgpu::VertexBufferLayout {
            array_stride: if slot == 0 {
                QUAD_VERTEX_STRIDE
            } else {
                crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress
            },
            step_mode: if slot == 0 {
                wgpu::VertexStepMode::Vertex
            } else {
                wgpu::VertexStepMode::Instance
            },
            attributes: &MAPPED_SCATTER_ATTRIBUTES[slot],
        })
    });
    point_spec(
        label,
        &layout,
        shader,
        (vs_entry, fs_entry),
        &buffers[..if third { 7 } else { 6 }],
        target_format,
        sample_count,
        three_d,
    )
    .create(device)
}

/// Bytes per vertex of the unit quad at slot 0.
const QUAD_VERTEX_STRIDE: wgpu::BufferAddress =
    (std::mem::size_of::<f32>() * 2) as wgpu::BufferAddress;

/// The attribute of each vertex slot a scatter pipeline reads, in slot
/// order: the unit quad, X, Y, then the ternary third component (or 3D
/// height) for the precise entries.
const SCATTER_ATTRIBUTES: [[wgpu::VertexAttribute; 1]; 4] = slot_attributes(None);

/// The mapped entries' slots: the precise three, the f32 style index at 3,
/// the two value columns and the third component at 6.
const MAPPED_SCATTER_ATTRIBUTES: [[wgpu::VertexAttribute; 1]; 7] = slot_attributes(Some(3));

/// One `Float32x2` attribute per slot at its own location, but `f32_slot`
/// a `Float32`.
const fn slot_attributes<const N: usize>(
    f32_slot: Option<usize>,
) -> [[wgpu::VertexAttribute; 1]; N] {
    let mut attrs = [[wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    }]; N];
    let mut i = 0;
    while i < N {
        attrs[i][0].shader_location = i as u32;
        if let Some(slot) = f32_slot
            && slot == i
        {
            attrs[i][0].format = wgpu::VertexFormat::Float32;
        }
        i += 1;
    }
    attrs
}

/// The spec of a point pipeline: premultiplied blending into the target
/// without depth, or a 3D pass's depth state, multisampling and target.
#[allow(clippy::too_many_arguments)]
fn point_spec<'a>(
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    shader: &'a wgpu::ShaderModule,
    entries: (&'a str, &'a str),
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    three_d: Option<View3dPass>,
) -> RenderPipelineSpec<'a> {
    RenderPipelineSpec {
        label,
        layout,
        shader,
        entries,
        buffers,
        topology: wgpu::PrimitiveTopology::TriangleStrip,
        depth_stencil: three_d.and_then(View3dPass::depth_stencil),
        multisample: three_d.map_or_else(
            || multisample_state(sample_count),
            |pass| pass.multisample(sample_count),
        ),
        target: match three_d {
            Some(pass) => pass.color_target(target_format),
            None => blended_target(
                target_format,
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            ),
        },
    }
}

/// The precise scatter pipeline over `[transform, style]` with `(vertex
/// entry, fragment entry, label)`: slot 0 the unit quad (per vertex), slots
/// 1 and 2 the X and Y columns (per instance). The precise draw is `vs_main`
/// / `fs_main`; the picked ring and the styled variants share its slots,
/// and the constellation and planet variants bind the style textures
/// (`texture_bgl`) as group 2. With `third`, the third component of a
/// ternary chart's entries (`vs_ternary`, `vs_pick_ring_ternary`) or a 3D
/// chart's height at slot 3; with `three_d`, a 3D chart's entries
/// (`vs_3d`, `vs_3d_pick`, `vs_pick_ring_3d`) with the camera group
/// (`surface_mesh::create_surface_data_bind_group_layout`) as group 3 and
/// the pass's depth state and target.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_pipeline(
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
    // The camera is group 3 whether or not the textures fill group 2.
    let bgls = [
        Some(transform_bgl),
        Some(style_bgl),
        texture_bgl,
        three_d.map(|(camera_bgl, _)| camera_bgl),
    ];
    let groups = match (texture_bgl, three_d) {
        (_, Some(_)) => 4,
        (Some(_), None) => 3,
        (None, None) => 2,
    };
    let layout =
        RenderPipelineSpec::layout(device, "figgy scatter columnar layout", &bgls[..groups]);
    let buffers: [Option<wgpu::VertexBufferLayout<'_>>; 4] = std::array::from_fn(|slot| {
        Some(wgpu::VertexBufferLayout {
            array_stride: if slot == 0 {
                QUAD_VERTEX_STRIDE
            } else {
                crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress
            },
            step_mode: if slot == 0 {
                wgpu::VertexStepMode::Vertex
            } else {
                wgpu::VertexStepMode::Instance
            },
            attributes: &SCATTER_ATTRIBUTES[slot],
        })
    });
    point_spec(
        label,
        &layout,
        shader,
        (vs_entry, fs_entry),
        &buffers[..if third { 4 } else { 3 }],
        target_format,
        sample_count,
        three_d.map(|(_, pass)| pass),
    )
    .create(device)
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
