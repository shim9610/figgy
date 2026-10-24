//! Bar series: pipelines, vertex counts and the bar and bar selection layers.

use super::*;

/// The assembled bar module: the histogram entries of `bar_columnar.wgsl`
/// and the category bar entries of its input fragment
/// (`common/category_bars.wgsl`, SHADER_COMMON.md §5.13), which read the
/// same override table and are shaded as flat bodies (`material.wgsl`,
/// `bounded_shading.wgsl`).
pub(crate) const BAR_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/data_pick.wgsl",
    "data_render/common/material.wgsl",
    "data_render/common/bounded_shading.wgsl",
    "data_render/common/category_bars.wgsl",
    "data_render/bar_columnar.wgsl",
);

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

/// The selected bin's outline for a series drawn without a style map: the
/// bar layouts, groups 0 and 1 only (`vs_bar_selection`).
pub(crate) fn create_bar_selection_pipeline_with_sample_count(
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
        "vs_bar_selection",
        "figgy selected histogram bin pipeline",
    )
}

/// The selected bin's outline for a series drawn through its style map: the
/// mapped bar layout, group 2 = the series' bar style map
/// (`vs_bar_selection_mapped`).
pub(crate) fn create_bar_selection_mapped_pipeline_with_sample_count(
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
        "vs_bar_selection_mapped",
        "figgy selected mapped histogram bin pipeline",
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_bar_columnar_pipeline_with_entry(
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
    let mapped_layouts = [Some(transform_bgl), Some(group_one_bgl), group_two_bgl];
    let bind_group_layouts = if group_two_bgl.is_some() {
        &mapped_layouts[..]
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
/// same pool columns as the normal bar draw. `style_bg` is the selection's
/// bar `Style` (the highlight lanes, see `bar_columnar.wgsl`); `style_map_bg`
/// is the series' bar style map exactly when the bar draw binds it, and
/// selects the mapped outline entry.
///
/// A category bar's highlight (`category` set) is the same layer over the
/// category bar pipeline: it binds the series' data group as group 3 and no
/// vertex buffers, and `edges` / `values` are the series' columns, unread.
pub struct ColumnBarSelectionLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: wgpu::BindGroup,
    pub style_map_bg: Option<&'a wgpu::BindGroup>,
    pub pool_buffer: &'a wgpu::Buffer,
    pub edges: ColumnHandle,
    pub values: ColumnHandle,
    pub instance: u32,
    pub category: Option<CategoryBarData>,
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

// ───── Category bars ─────

/// Vertices per category bar instance: one quad as a TriangleList. Must match
/// `category_bars.wgsl`'s `CATEGORY_BAR_VERTICES`.
pub const CATEGORY_BAR_VERTICES: u32 = 6;

/// `CategoryBarParamsGpu::mode`: bars stand side by side.
pub(crate) const CATEGORY_BAR_MODE_GROUPED: u32 = 0;
/// `CategoryBarParamsGpu::mode`: bars stack.
pub(crate) const CATEGORY_BAR_MODE_STACKED: u32 = 1;
/// `CategoryBarParamsGpu::mode`: bars stack to the share of the category
/// total.
pub(crate) const CATEGORY_BAR_MODE_PERCENT: u32 = 2;

/// The group one category bar series belongs to and its layout: what the
/// category bar fragment reads beside `Transform`, `Style` and the override
/// table. Lane bases are f32 indices into the pool; nothing per bar is here.
///
/// GPU twin: `category_bars.wgsl::CategoryBarParams`, 112 B.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CategoryBarParamsGpu {
    /// Lane bases of the group's value columns in group order: member `k`
    /// is `values[k / 4][k % 4]`.
    pub values: [[u32; 4]; 4],
    /// Lane base of this series' category position column.
    pub positions: u32,
    /// Bars of this series: one per category.
    pub count: u32,
    /// This series' place in its group.
    pub member: u32,
    /// Series in the group.
    pub members: u32,
    /// One of the `CATEGORY_BAR_MODE_*` values.
    pub mode: u32,
    /// 1 when categories run along y.
    pub horizontal: u32,
    /// Render pixels between grouped bars.
    pub gap_px: f32,
    /// Fraction of a category slot the bars of the slot cover.
    pub group_width: f32,
    /// The series' corner radius in render pixels.
    pub corner_radius_px: f32,
    /// The series' emphasis brightness of a picked bar.
    pub emphasis: f32,
    /// Render pixels per logical pixel.
    pub scale: f32,
    /// The series salt, which seeds the material grain per bar.
    pub seed: u32,
}

const _: () = assert!(std::mem::size_of::<CategoryBarParamsGpu>() == 112);

/// Bind group layout of a category bar series' own data (group 3): the column
/// pool as read-only storage (binding 8) and its params (binding 9). Bindings
/// 0..6 of group 3 are the histogram envelope's in the same module. The pick
/// (`pick_category_bar`) binds the same group.
pub fn create_category_bar_data_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let visibility = wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE;
    crate::gpu_compile::bind_group_layout(
        device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy category bar data bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility,
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

/// What a category bar series draws with besides its style: its group 3,
/// its override table (group 2, resolved against its axis) and the charge
/// of its params buffer.
#[derive(Clone)]
pub struct CategoryBarData {
    pub bind_group: wgpu::BindGroup,
    pub style_map: wgpu::BindGroup,
    pub charge: crate::gpu_memory::SharedCharge,
}

/// Bind the pool and `params` into the category bar data layout, next to the
/// series' override table. The params buffer is owned by the bind group and
/// charged as a uniform.
pub fn create_category_bar_data_bind_group(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    params: &CategoryBarParamsGpu,
    style_map: BarStyleMap,
) -> CategoryBarData {
    use crate::gpu_memory::{ChargeTally, charged_buffer_init};
    let tally = ChargeTally::new();
    let params_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy category bar params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy category bar data bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 8,
                resource: pool_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: params_buf.as_entire_binding(),
            },
        ],
    });
    CategoryBarData {
        bind_group,
        style_map: style_map.bind_group,
        charge,
    }
}

/// A category bar pipeline: groups 0 transform, 1 style, 2 the series' bar
/// override table, 3 its data; no vertex buffers. `vs_category_bars` /
/// `fs_category_bars` draw the bars, `vs_category_bar_selection` /
/// `fs_category_bar_selection` the picked one's highlight.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_category_bar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    data_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    selection: bool,
) -> wgpu::RenderPipeline {
    let (vertex_entry, fragment_entry, label) = if selection {
        (
            "vs_category_bar_selection",
            "fs_category_bar_selection",
            "figgy selected category bar columnar pipeline",
        )
    } else {
        (
            "vs_category_bars",
            "fs_category_bars",
            "figgy category bar columnar pipeline",
        )
    };
    let layout = crate::gpu_compile::pipeline_layout(
        device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy category bar layout"),
            bind_group_layouts: &[
                Some(transform_bgl),
                Some(style_bgl),
                Some(style_map_bgl),
                Some(data_bgl),
            ],
            immediate_size: 0,
        },
    );
    crate::gpu_compile::render_pipeline(
        device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex_entry),
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

/// One category bar series: `count` instances of `CATEGORY_BAR_VERTICES`.
pub struct ColumnCategoryBarLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub data: CategoryBarData,
    pub count: u32,
}
