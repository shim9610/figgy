//! Bar series: pipelines, vertex counts and the bar and bar selection layers.

use super::*;

/// The assembled bar module: the histogram entries of `bar_columnar.wgsl`,
/// the category bar entries of its input fragment
/// (`common/category_bars.wgsl`, SHADER_COMMON.md §5.13), which read the
/// same override table and are shaded as flat bodies (`material.wgsl`,
/// `bounded_shading.wgsl`), and the box plot entries of its box plot input
/// fragment (`common/box_plot.wgsl`, §5.15), whose whiskers, caps and
/// median are the error bar's stroke quads (`errorbar_segment.wgsl`) and
/// whose mean and outliers are `marker.wgsl` shapes (with `ramp.wgsl` and
/// `sketch_noise.wgsl`, which that fragment reads), and the range bar
/// entries of its range bar input fragment (`common/range_bars.wgsl`,
/// §5.18), which share the box plot's slot layout and the category bars'
/// shading.
pub(crate) const BAR_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/data_pick.wgsl",
    "data_render/common/material.wgsl",
    "data_render/common/bounded_shading.wgsl",
    "data_render/common/category_bars.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/sketch_noise.wgsl",
    "data_render/common/marker.wgsl",
    "data_render/common/errorbar_segment.wgsl",
    "data_render/common/box_plot.wgsl",
    "data_render/common/range_bars.wgsl",
    "data_render/bar_columnar.wgsl",
    "data_render/common/contour_label_box.wgsl",
    "data_render/common/label_quad.wgsl",
    "data_render/common/value_label.wgsl",
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

// ───── Box plots ─────

/// Vertices of one box before its outliers: four whisker and cap quads, the
/// body, the median and the mean. Must match `box_plot.wgsl`'s
/// `BOX_PLOT_BASE_VERTICES`.
pub const BOX_PLOT_BASE_VERTICES: u32 = 42;
/// Vertices per quad of a box: one outlier marker adds one quad.
pub const BOX_PLOT_QUAD_VERTICES: u32 = 6;
/// Style slots and override rows per box (`box_plot.wgsl`'s
/// `BOX_PLOT_PARTS`): the body, the strokes and the markers.
pub const BOX_PLOT_PARTS: u32 = 3;
/// The body's part (`BOX_PART_BODY`).
pub(crate) const BOX_PART_BODY: u32 = 0;
/// Lane table entry of a box plot's outlier values
/// (`BOX_LANE_OUTLIER_VALUES`); entries 0..9 are the record's fields in
/// `BOX_PLOT_RECORD` order.
pub(crate) const BOX_LANE_OUTLIER_VALUES: u32 = 9;
/// The lane of the outlier offsets (`BOX_LANE_OUTLIER_OFFSETS`).
pub(crate) const BOX_LANE_OUTLIER_OFFSETS: u32 = 10;
/// The lane of the outlier group count (`BOX_LANE_OUTLIER_GROUPS`).
pub(crate) const BOX_LANE_OUTLIER_GROUPS: u32 = 11;
/// The lane of the longest outlier group (`BOX_LANE_MAX_GROUP`).
pub(crate) const BOX_LANE_MAX_GROUP: u32 = 12;
/// A lane with no column: an absent optional record field, or no outlier
/// column (`BOX_LANE_ABSENT`).
pub(crate) const BOX_LANE_ABSENT: u32 = u32::MAX;
/// Entries of a box plot's lane table: the record schema's fields and the
/// outlier column's four lanes, a fixed schema rather than a count of data.
pub(crate) const BOX_PLOT_LANES: usize = BOX_LANE_MAX_GROUP as usize + 1;

// ───── Range bars ─────

/// Vertices per range bar instance: the whisker, the body and the connector,
/// one quad each. Must match `range_bars.wgsl`'s `RANGE_BAR_VERTICES`.
pub const RANGE_BAR_VERTICES: u32 = 18;
/// Lane table entries of a range bar series (`RANGE_LANE_*`): the end,
/// start, low, high, colour and progress column lanes (`BOX_LANE_ABSENT`
/// for an absent column), the colour mode and the state style count, a
/// fixed schema rather than a count of data.
pub(crate) const RANGE_LANE_END: u32 = 0;
pub(crate) const RANGE_LANE_START: u32 = 1;
pub(crate) const RANGE_LANE_LOW: u32 = 2;
pub(crate) const RANGE_LANE_HIGH: u32 = 3;
pub(crate) const RANGE_LANE_COLOR: u32 = 4;
pub(crate) const RANGE_LANE_PROGRESS: u32 = 5;
pub(crate) const RANGE_LANE_COLOR_MODE: u32 = 6;
pub(crate) const RANGE_LANE_STATES: u32 = 7;
pub(crate) const RANGE_BAR_LANES: usize = RANGE_LANE_STATES as usize + 1;
/// `RANGE_LANE_COLOR_MODE` values (`RANGE_COLOR_*`).
pub(crate) const RANGE_COLOR_SINGLE: u32 = 0;
pub(crate) const RANGE_COLOR_DIRECTION: u32 = 1;
pub(crate) const RANGE_COLOR_STATE: u32 = 2;
/// A range bar series' style slots (`RANGE_SLOT_*`): the whisker, the
/// connector, the up, down and total fills, the progress, then one per
/// state from `RANGE_SLOT_STATES`.
pub(crate) const RANGE_SLOT_WHISKER: u32 = 0;
pub(crate) const RANGE_SLOT_CONNECTOR: u32 = 1;
pub(crate) const RANGE_SLOT_UP: u32 = 2;
pub(crate) const RANGE_SLOT_DOWN: u32 = 3;
pub(crate) const RANGE_SLOT_TOTAL: u32 = 4;
pub(crate) const RANGE_SLOT_PROGRESS: u32 = 5;
pub(crate) const RANGE_SLOT_STATES: u32 = 6;

/// `CategoryBarParamsGpu::mode`: bars stand side by side.
pub(crate) const CATEGORY_BAR_MODE_GROUPED: u32 = 0;
/// `CategoryBarParamsGpu::mode`: bars stack.
pub(crate) const CATEGORY_BAR_MODE_STACKED: u32 = 1;
/// `CategoryBarParamsGpu::mode`: bars stack to the share of the category
/// total.
pub(crate) const CATEGORY_BAR_MODE_PERCENT: u32 = 2;

/// The group one category bar series belongs to and its layout: what the
/// category bar fragment reads beside `Transform`, `Style`, the override
/// table and the series' lane table. Lane bases are f32 indices into the
/// pool; nothing per bar is here. The group's value column lanes are the
/// lane table's (group 3 binding 10), sized by the group, so no field here
/// bounds the number of series (#107 R3).
///
/// GPU twin: `category_bars.wgsl::CategoryBarParams`, 48 B.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CategoryBarParamsGpu {
    /// Lane base of this series' category position column.
    pub positions: u32,
    /// Bars of this series: one per category.
    pub count: u32,
    /// This series' place in its group.
    pub member: u32,
    /// Series in the group: the lane table's entries for a category bar
    /// series.
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
    /// Half a slot along the position axis in data units, as a `(hi, lo)`
    /// pair: `CATEGORY_SLOT_HALF` for category bars and boxes, half a
    /// range bar series' `slot_width` on a numeric axis.
    pub slot_half: [f32; 2],
    pub _reserved: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<CategoryBarParamsGpu>() == 64);

/// [`CategoryBarParamsGpu::slot_half`] of a slot that is one category.
pub(crate) const CATEGORY_SLOT_HALF: [f32; 2] = [0.5, 0.0];

/// Bind group layout of a category bar series' own data (group 3): the column
/// pool as read-only storage (binding 8), its params (binding 9) and its lane
/// table as read-only storage (binding 10, `array<u32>`). Bindings 0..6 of
/// group 3 are the histogram envelope's in the same module. The pick
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
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
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
/// of its params and lane table buffers.
#[derive(Clone)]
pub struct CategoryBarData {
    pub bind_group: wgpu::BindGroup,
    pub style_map: wgpu::BindGroup,
    pub charge: crate::gpu_memory::SharedCharge,
}

/// Bind the pool, `params` and the series' lane table into the category bar
/// data layout, next to the series' override table.
///
/// The lane table holds `lanes` entries: `write` pushes them in order and
/// they go straight into the mapping of the table's buffer at creation, so
/// nothing is staged on the CPU and the table is sized by what it holds (a
/// bar group's members, a box plot's 13 record and outlier lanes, a radar
/// label's one value column). Entries `write` does not push read as zero; pushes
/// past `lanes` are dropped. An error from `write` is returned as is. The
/// buffers are owned by the bind group and charged as a uniform.
#[allow(clippy::too_many_arguments)]
pub fn create_category_bar_data_bind_group(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    params: &CategoryBarParamsGpu,
    lanes: usize,
    write: impl FnOnce(&mut dyn FnMut(u32)) -> crate::Result<()>,
    style_map: BarStyleMap,
) -> crate::Result<CategoryBarData> {
    use crate::gpu_memory::{ChargeTally, charged_buffer, charged_buffer_init};
    const LANE: usize = std::mem::size_of::<u32>();
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
    let lane_bytes = (lanes.max(1) as u64).checked_mul(LANE as u64).ok_or(
        crate::FiggyError::GpuResourceLimit {
            resource: "category lane table",
            requested: lanes as u64,
            limit: u64::MAX / LANE as u64,
        },
    )?;
    let lane_buf = charged_buffer(
        &tally,
        device,
        &wgpu::BufferDescriptor {
            label: Some("figgy category lane table"),
            size: lane_bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        },
    );
    let written = {
        let mut view = lane_buf
            .slice(..)
            .get_mapped_range_mut()
            .expect("the lane table is mapped at creation");
        let mut written = 0usize;
        let result = write(&mut |lane| {
            if written < lanes {
                let at = written * LANE;
                view.slice(at..at + LANE)
                    .copy_from_slice(bytemuck::bytes_of(&lane));
                written += 1;
            }
        });
        result.map(|()| written)
    };
    lane_buf.unmap();
    written?;
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
            wgpu::BindGroupEntry {
                binding: 10,
                resource: lane_buf.as_entire_binding(),
            },
        ],
    });
    Ok(CategoryBarData {
        bind_group,
        style_map: style_map.bind_group,
        charge,
    })
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
    let entries = if selection {
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
    create_category_axis_pipeline(
        device,
        shader,
        [transform_bgl, style_bgl, style_map_bgl, data_bgl],
        target_format,
        sample_count,
        entries,
    )
}

/// A box plot pipeline, over the category bar pipeline's layout (groups 0
/// transform, 1 style, 2 the series' bar style map, 3 its data; no vertex
/// buffers): `vs_box_plot` / `fs_box_plot` draw the boxes,
/// `vs_box_plot_selection` / `fs_box_plot_selection` the picked one's
/// highlight.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_box_plot_pipeline_with_sample_count(
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
    let entries = if selection {
        (
            "vs_box_plot_selection",
            "fs_box_plot_selection",
            "figgy selected box plot columnar pipeline",
        )
    } else {
        (
            "vs_box_plot",
            "fs_box_plot",
            "figgy box plot columnar pipeline",
        )
    };
    create_category_axis_pipeline(
        device,
        shader,
        [transform_bgl, style_bgl, style_map_bgl, data_bgl],
        target_format,
        sample_count,
        entries,
    )
}

/// A range bar pipeline, over the category bar pipeline's layout (groups 0
/// transform, 1 style, 2 the series' bar style map, 3 its data; no vertex
/// buffers): `vs_range_bars` / `fs_range_bars` draw the bars,
/// `vs_range_bar_selection` / `fs_range_bar_selection` the picked one's
/// highlight.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_range_bar_pipeline_with_sample_count(
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
    let entries = if selection {
        (
            "vs_range_bar_selection",
            "fs_range_bar_selection",
            "figgy selected range bar columnar pipeline",
        )
    } else {
        (
            "vs_range_bars",
            "fs_range_bars",
            "figgy range bar columnar pipeline",
        )
    };
    create_category_axis_pipeline(
        device,
        shader,
        [transform_bgl, style_bgl, style_map_bgl, data_bgl],
        target_format,
        sample_count,
        entries,
    )
}

/// A pipeline of the bar module's category axis entries (category bars,
/// box plots, range bars, value labels) with `(vertex entry, fragment
/// entry, label)`.
pub(super) fn create_category_axis_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    [transform_bgl, style_bgl, style_map_bgl, data_bgl]: [&wgpu::BindGroupLayout; 4],
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    (vertex_entry, fragment_entry, label): (&str, &str, &str),
) -> wgpu::RenderPipeline {
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

/// One category bar series (`count` instances of `CATEGORY_BAR_VERTICES`),
/// one box plot series (`count` instances of `vertices`: the box's
/// strokes, body and markers, `BOX_PLOT_BASE_VERTICES` plus six per outlier
/// of the longest group) or one range bar series (`count` instances of
/// `RANGE_BAR_VERTICES`).
pub struct ColumnCategoryBarLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub data: CategoryBarData,
    pub count: u32,
    /// Vertices per instance.
    pub vertices: u32,
}
