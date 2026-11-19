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
/// shading, and the violin and raw point entries of its violin fragment
/// (`common/violin.wgsl`, §5.19), placed by the shared category slot
/// fragment (`common/category_jitter.wgsl`, §5.20) the point pick splices
/// too.
pub(crate) const BAR_SHADER_BY_SET: crate::shader_assembly::AssembledBySet = crate::shader_assembly::assembled_by_set!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    set,
    "data_render/common/projection.wgsl",
    "data_render/common/data_pick.wgsl",
    "data_render/common/material.wgsl",
    "data_render/common/bounded_shading.wgsl",
    "data_render/common/radial_sector.wgsl",
    "data_render/common/category_bars.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/sketch_noise.wgsl",
    "data_render/common/category_jitter.wgsl",
    "data_render/common/marker.wgsl",
    "data_render/common/errorbar_segment.wgsl",
    "data_render/common/box_plot.wgsl",
    "data_render/common/range_bars.wgsl",
    "data_render/common/range_links.wgsl",
    "data_render/common/violin.wgsl",
    "data_render/bar_columnar.wgsl",
    "data_render/common/contour_label_box.wgsl",
    "data_render/common/label_quad.wgsl",
    "data_render/common/value_label.wgsl",
);
/// The full assembly of [`BAR_SHADER_BY_SET`]: the module a chart with an axis
/// break, a probability axis or polar coordinates compiles.
pub(crate) const BAR_SHADER: crate::shader_assembly::Assembled = BAR_SHADER_BY_SET.full;

/// Vertices per bar instance: five axis-aligned quads (fill + four border
/// edges) as a TriangleList. Must match `bar_columnar.wgsl`'s `BAR_SEGMENTS`.
pub const BAR_VERTICES_PER_INSTANCE: u32 = 30;

/// Four axis-aligned outline quads for one selected histogram bin.
/// Must match `vs_bar_selection` in `bar_columnar.wgsl`.
pub const BAR_SELECTION_VERTICES: u32 = 24;

/// A histogram bar pipeline from `(vertex entry, label)` into `fs_main`.
/// Slots (all per-instance): 0 = edge_lo, 1 = edge_hi, 2 = value. The
/// caller binds the edge column twice, the second time shifted by one
/// logical value, and picks which pool column is edges vs values from the
/// bar's orientation. Groups: transform, style (`group_one_bgl`), and for
/// the mapped draw (`vs_envelope_mapped_bars`) and its selected bin's
/// outline (`vs_bar_selection_mapped`) the series' bar style map as group 2;
/// the precise draw is `vs_envelope_bars`, the outline `vs_bar_selection`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_bar_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    group_one_bgl: &wgpu::BindGroupLayout,
    group_two_bgl: Option<&wgpu::BindGroupLayout>,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    (vertex_entry, label): (&str, &str),
) -> wgpu::RenderPipeline {
    let layouts = [Some(transform_bgl), Some(group_one_bgl), group_two_bgl];
    let groups = if group_two_bgl.is_some() { 3 } else { 2 };
    let layout =
        RenderPipelineSpec::layout(device, "figgy bar columnar layout", &layouts[..groups]);
    const ATTRS: [[wgpu::VertexAttribute; 1]; 3] = {
        let mut attrs = [[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 0,
        }]; 3];
        let mut i = 0;
        while i < 3 {
            attrs[i][0].shader_location = i as u32;
            i += 1;
        }
        attrs
    };
    let buffers: [Option<wgpu::VertexBufferLayout<'_>>; 3] = std::array::from_fn(|slot| {
        Some(wgpu::VertexBufferLayout {
            array_stride: crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRS[slot],
        })
    });
    RenderPipelineSpec {
        label,
        layout: &layout,
        shader,
        entries: (vertex_entry, "fs_main"),
        buffers: &buffers,
        topology: wgpu::PrimitiveTopology::TriangleList,
        depth_stencil: None,
        multisample: multisample_state(sample_count),
        target: blended_target(
            target_format,
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        ),
    }
    .create(device)
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
    /// Instances from `instance` the highlight draws: one, or a violin's
    /// every segment (its vertex stage keeps the selected category's).
    pub instances: u32,
    pub category: Option<CategoryBarData>,
    /// Vertices per instance: `CATEGORY_BAR_VERTICES` for a bar or box,
    /// `RANGE_LINK_VERTICES` for a range link (#357), `BAR_SELECTION_VERTICES`
    /// for a histogram bin.
    pub vertices: u32,
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
/// The box's share of its slot as `f32` bits (`BOX_LANE_BOX_WIDTH`): 1 for
/// a plain box, a violin's `box_width`.
pub(crate) const BOX_LANE_BOX_WIDTH: u32 = 13;
/// The first of the violin curve lanes the value label clears
/// (`BOX_LANE_VIOLIN_CURVES`): each half's values and offsets lanes.
pub(crate) const BOX_LANE_VIOLIN_CURVES: u32 = 14;
/// Entries of a box plot's lane table: the record schema's fields, the
/// outlier column's four lanes, the box width and the two violin halves'
/// curve lanes, a fixed schema rather than a count of data.
pub(crate) const BOX_PLOT_LANES: usize = BOX_LANE_VIOLIN_CURVES as usize + 4;

// ───── Violins and raw points ─────

/// Vertices per violin segment and per raw point: one quad. Must match
/// `violin.wgsl`'s `VIOLIN_VERTICES`.
pub const VIOLIN_VERTICES: u32 = 6;
/// The violin draw's lane table (`VIOLIN_LANE_*`, `VIOLIN_HALF_*`): the
/// half count, the first half's segment count, then seven lanes per half
/// (its side, curve values, offsets, group count, value count, density
/// lane and density scale bits), a fixed schema rather than a count of
/// data.
pub(crate) const VIOLIN_LANE_HALVES: usize = 0;
pub(crate) const VIOLIN_LANE_FIRST_SEGMENTS: usize = 1;
pub(crate) const VIOLIN_HALF_BASE: usize = 2;
pub(crate) const VIOLIN_HALF_LANES: usize = 7;
pub(crate) const VIOLIN_LANES: usize = VIOLIN_HALF_BASE + 2 * VIOLIN_HALF_LANES;
/// `VIOLIN_HALF_SIDE` values (`VIOLIN_SIDE_*`).
pub(crate) const VIOLIN_SIDE_BOTH: u32 = 0;
pub(crate) const VIOLIN_SIDE_LOW: u32 = 1;
pub(crate) const VIOLIN_SIDE_HIGH: u32 = 2;
// The lane tables are indexed by these constants (#152): the record's
// fields take lanes `0..BOX_PLOT_RECORD.len()` before the outlier lanes,
// every named lane and both violin halves' curve lanes (two each) fit
// `BOX_PLOT_LANES`, and the violin table holds its two header lanes and two
// halves of `VIOLIN_HALF_LANES`, whose last lane is `base + 6`.
const _: () = assert!(super::BOX_PLOT_RECORD.len() <= BOX_LANE_OUTLIER_VALUES as usize);
const _: () = assert!(super::BOX_PLOT_RECORD.len() <= BOX_PLOT_LANES);
const _: () = assert!(
    BOX_LANE_OUTLIER_VALUES < BOX_LANE_OUTLIER_OFFSETS
        && BOX_LANE_OUTLIER_OFFSETS < BOX_LANE_OUTLIER_GROUPS
        && BOX_LANE_OUTLIER_GROUPS < BOX_LANE_MAX_GROUP
        && BOX_LANE_MAX_GROUP < BOX_LANE_BOX_WIDTH
        && BOX_LANE_BOX_WIDTH < BOX_LANE_VIOLIN_CURVES
);
const _: () = assert!(BOX_LANE_VIOLIN_CURVES as usize + 2 * 2 <= BOX_PLOT_LANES);
const _: () = assert!(BOX_LANE_ABSENT as usize >= BOX_PLOT_LANES);
const _: () =
    assert!(VIOLIN_LANE_HALVES < VIOLIN_HALF_BASE && VIOLIN_LANE_FIRST_SEGMENTS < VIOLIN_HALF_BASE);
const _: () = assert!(VIOLIN_HALF_LANES >= 7);
const _: () = assert!(VIOLIN_HALF_BASE + 2 * VIOLIN_HALF_LANES <= VIOLIN_LANES);

/// A raw point's marker shape code (`finish.w` of its style slot): the
/// `marker.wgsl` base shape id, plus this when filled.
pub(crate) const BOX_POINTS_FILLED: u32 = 100;

// ───── Range bars ─────

/// Vertices per range bar instance: the whisker, the body and the connector,
/// one quad each. Must match `range_bars.wgsl`'s `RANGE_BAR_VERTICES`.
pub const RANGE_BAR_VERTICES: u32 = 18;
/// Lane table entries of a range bar series (`RANGE_LANE_*`): the end,
/// start, low, high, colour and progress column lanes (`BOX_LANE_ABSENT`
/// for an absent column), the colour mode and the state style (or row
/// colour) count, a
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
/// A range link series' lanes past its bars' (#357, `range_links.wgsl`):
/// the predecessor and successor row columns, the link count, and the
/// stroke width, head and lead-out in render pixels as f32 bits.
pub(crate) const RANGE_LANE_LINK_FROM: u32 = 8;
pub(crate) const RANGE_LANE_LINK_TO: u32 = 9;
pub(crate) const RANGE_LANE_LINK_COUNT: u32 = 10;
pub(crate) const RANGE_LANE_LINK_WIDTH: u32 = 11;
pub(crate) const RANGE_LANE_LINK_HEAD: u32 = 12;
pub(crate) const RANGE_LANE_LINK_LEAD: u32 = 13;
pub(crate) const RANGE_LINK_LANES: usize = RANGE_LANE_LINK_LEAD as usize + 1;
/// Vertices per link: five segment quads and the head's quad. Must match
/// `range_links.wgsl`'s `RANGE_LINK_VERTICES`.
pub const RANGE_LINK_VERTICES: u32 = 36;
/// `RANGE_LANE_COLOR_MODE` values (`RANGE_COLOR_*`).
pub(crate) const RANGE_COLOR_SINGLE: u32 = 0;
pub(crate) const RANGE_COLOR_DIRECTION: u32 = 1;
pub(crate) const RANGE_COLOR_STATE: u32 = 2;
pub(crate) const RANGE_COLOR_ROW: u32 = 3;
/// A range bar series' style slots (`RANGE_SLOT_*`): the whisker, the
/// connector, the up, down and total fills, the progress, then one per
/// state, or one per row colour, from `RANGE_SLOT_STATES`.
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
/// lane table's (group 3 binding 10), sized by the group, so any number of
/// series fits.
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
    /// 1 on a polar chart: the bars are wedges of the angle axis along the
    /// radius (a rose, #133), built by `category_wedge`.
    pub wedge: u32,
    pub _reserved: u32,
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
/// bar group's members, a box plot's 13 record and outlier lanes, the
/// value columns of a radar label group's members). Entries `write` does not push read as zero; pushes
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
    )?;
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
    )?;
    let written = {
        let mut view = lane_buf.slice(..).get_mapped_range_mut().map_err(|error| {
            crate::FiggyError::GpuValidation {
                resource: "category lane table",
                reason: error.to_string(),
            }
        })?;
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

/// A pipeline of the bar module's category axis entries with `(vertex
/// entry, fragment entry, label)`: groups 0 transform, 1 style (a value
/// label's group for the value labels), 2 the series' bar style map, 3 its
/// data; no vertex buffers. The category bars (`vs_category_bars` /
/// `fs_category_bars`), box plots (`vs_box_plot`), range bars
/// (`vs_range_bars`), violins (`vs_violin`), a box plot's raw points
/// (`vs_box_points`) and value labels (`vs_value_label`) draw through it,
/// each with a `_selection` pair for the picked one's highlight.
pub(crate) fn create_category_axis_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    [transform_bgl, style_bgl, style_map_bgl, data_bgl]: [&wgpu::BindGroupLayout; 4],
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    (vertex_entry, fragment_entry, label): (&str, &str, &str),
) -> wgpu::RenderPipeline {
    let layout = RenderPipelineSpec::layout(
        device,
        "figgy category bar layout",
        &[
            Some(transform_bgl),
            Some(style_bgl),
            Some(style_map_bgl),
            Some(data_bgl),
        ],
    );
    RenderPipelineSpec {
        label,
        layout: &layout,
        shader,
        entries: (vertex_entry, fragment_entry),
        buffers: &[],
        topology: wgpu::PrimitiveTopology::TriangleList,
        depth_stencil: None,
        multisample: multisample_state(sample_count),
        target: blended_target(
            target_format,
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        ),
    }
    .create(device)
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
