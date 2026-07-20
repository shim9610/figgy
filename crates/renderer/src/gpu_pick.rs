//! Persistent GPU BVH picking for [`ColumnPool`](crate::data_render::ColumnPool).
//!
//! The BVH is an index-order block hierarchy.  A leaf owns at most 64 scatter
//! points and the same 64 adjacent line-segment starts; the endpoint at the
//! next block boundary is included in that leaf's bounds.  Leaf and parent
//! AABBs are built entirely by compute dispatches.  The CPU supplies only
//! column offsets/lengths, packed-level topology, series identity, and small
//! style metadata -- never per-value data.
//!
//! A query traverses the hierarchy with a per-series GPU queue.  Every
//! surviving leaf exact-tests all owned scatter points and line segments,
//! then a second compute pass reduces the per-series scalars with the current
//! picker tie rules.  [`GpuPickTicket`] owns its mapping buffer, device, and
//! identity snapshot, so it can outlive both the engine and the chart frame.

use std::fmt;
use std::sync::Arc;

use futures_channel::oneshot;
use wgpu::util::DeviceExt;

use crate::data_render::{
    ColumnHandle, ColumnId, ColumnPool, ScatterStyleMapMeta, ScatterStyleOverrideGpu,
    ScatterStyleSlotGpu, ScatterTransform,
};
use crate::pick::PickedPoint;

/// Logical point/segment starts exact-tested by one leaf.
pub const GPU_PICK_BLOCK_POINTS: u32 = 64;
/// Compute workgroup width used by build, traversal, and reduction.
pub const GPU_PICK_WORKGROUP_SIZE: u32 = 64;

const NODE_BYTES: u64 = 48;
const CANDIDATE_BYTES: u64 = 48;
const QUEUE_STATE_BYTES: u64 = 16;
const MAX_SHAPE_SCALE: f32 = 1.555_120_3;

const FLAG_SCATTER: u32 = 1;
const FLAG_LINE: u32 = 2;
const FLAG_STYLE_MAP: u32 = 4;
const FLAG_STYLE_INDEX: u32 = 8;
const STYLE_MASK_RADIUS: u32 = 2;
const STYLE_MASK_SHAPE: u32 = 4;

#[derive(Debug)]
pub enum GpuPickError {
    DeviceLimit {
        resource: &'static str,
        requested: u64,
        limit: u64,
    },
    AllocationFailed {
        resource: &'static str,
    },
    MissingColumn(ColumnId),
    StaleColumn {
        series_id: String,
        column_id: ColumnId,
    },
    EmptySeries(String),
    NoPickPrimitive(String),
    InvalidStyleMap {
        series_id: String,
        reason: &'static str,
    },
    TooManyValues {
        series_id: String,
        count: usize,
    },
    TraversalOverflow,
    MapChannelClosed,
    MapFailed(wgpu::BufferAsyncError),
    InvalidGpuResult,
    InvalidSeriesIndex {
        index: usize,
        len: usize,
    },
}

impl fmt::Display for GpuPickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceLimit {
                resource,
                requested,
                limit,
            } => write!(
                f,
                "GPU pick {resource} exceeds a device limit: requested {requested}, limit {limit}"
            ),
            Self::AllocationFailed { resource } => {
                write!(f, "GPU pick could not allocate {resource}")
            }
            Self::MissingColumn(id) => write!(f, "GPU pick column is missing: {id}"),
            Self::StaleColumn {
                series_id,
                column_id,
            } => write!(
                f,
                "GPU pick BVH for series {series_id:?} is stale because column {column_id:?} moved, was removed, or changed length"
            ),
            Self::EmptySeries(id) => write!(f, "GPU pick series {id:?} has no paired values"),
            Self::NoPickPrimitive(id) => {
                write!(
                    f,
                    "GPU pick series {id:?} has neither scatter nor line geometry"
                )
            }
            Self::InvalidStyleMap { series_id, reason } => {
                write!(
                    f,
                    "GPU pick style map for series {series_id:?} is invalid: {reason}"
                )
            }
            Self::TooManyValues { series_id, count } => write!(
                f,
                "GPU pick series {series_id:?} has {count} values; the WebGPU storage index limit is {}",
                u32::MAX
            ),
            Self::TraversalOverflow => write!(f, "GPU pick BVH traversal queue overflowed"),
            Self::MapChannelClosed => write!(f, "GPU pick readback callback was dropped"),
            Self::MapFailed(error) => write!(f, "GPU pick readback mapping failed: {error:?}"),
            Self::InvalidGpuResult => write!(f, "GPU pick returned an invalid scalar record"),
            Self::InvalidSeriesIndex { index, len } => write!(
                f,
                "GPU pick series index {index} is out of bounds for {len} registered series"
            ),
        }
    }
}

impl std::error::Error for GpuPickError {}

/// Exact per-point scatter-style inputs.
///
/// These are the small arrays already derived from `SeriesConfig` for the
/// precise render path.  They are uploaded once and cached with the series.
/// Passing `None` for [`GpuPickSeriesDescriptor::scatter`] disables scatter
/// picking.  Passing `style_map: None` selects the production picker's
/// non-mapped/base-style semantics (used by non-precise draw styles).
pub struct GpuPickScatterStyle<'a> {
    pub style_index_column: Option<ColumnId>,
    pub style_slots: &'a [ScatterStyleSlotGpu],
    pub style_overrides: &'a [ScatterStyleOverrideGpu],
    pub style_meta: ScatterStyleMapMeta,
}

/// Scatter inputs for one registered series.
pub struct GpuPickScatter<'a> {
    pub base_radius_px: f32,
    pub base_shape_id: u32,
    /// `Some` only when current precise-mode style mapping is active.
    pub style_map: Option<GpuPickScatterStyle<'a>>,
}

/// Registration-time metadata for a persistent series BVH.
pub struct GpuPickSeriesDescriptor<'a> {
    pub source_id: Option<String>,
    pub series_id: String,
    pub x_column: ColumnId,
    pub y_column: ColumnId,
    pub scatter: Option<GpuPickScatter<'a>>,
    /// Full production picker line width.  The engine applies `max(0) / 2`.
    pub line_width_px: Option<f32>,
}

/// Query state which integration can derive directly from `Config`.
#[derive(Debug, Clone, Copy)]
pub struct GpuPickQuery {
    pub transform: ScatterTransform,
    /// `(x, y, width, height)` in canvas pixels.  Width/height must be the
    /// same chart-area values used to build `transform`.
    pub chart_rect_px: [f32; 4],
    /// Current data-area clip in canvas pixels.  Like the CPU picker, cursor
    /// positions outside it return `None` before GPU work is submitted.
    pub data_area_px: Option<[f32; 4]>,
    pub canvas_position_px: [f32; 2],
    pub max_distance_px: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuPickSeriesId(u32);

impl GpuPickSeriesId {
    pub fn index(self) -> u32 {
        self.0
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PickBuildParamsGpu {
    point_count: u32,
    x_base: u32,
    y_base: u32,
    input_start: u32,
    output_start: u32,
    input_count: u32,
    invocation_base: u32,
    include_line_boundary: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PickQueryParamsGpu {
    transform: ScatterTransform,
    cursor_chart: [f32; 4],
    chart_limits: [f32; 4],
    scatter_line: [f32; 4],
    data: [u32; 4],
    style: [u32; 4],
    series: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct PickCandidateGpu {
    valid: u32,
    series_order: u32,
    point_index: u32,
    primitive_kind: u32,
    primitive_index: u32,
    data_x: f32,
    data_y: f32,
    distance_sq: f32,
    distance_px: f32,
    _pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<PickBuildParamsGpu>() == 32);
const _: () = assert!(std::mem::size_of::<PickQueryParamsGpu>() == 192);
const _: () = assert!(std::mem::size_of::<PickCandidateGpu>() == CANDIDATE_BYTES as usize);

#[derive(Clone)]
struct PickIdentity {
    source_id: Option<String>,
    series_id: String,
}

struct PickPipelines {
    build_bgl: wgpu::BindGroupLayout,
    query_data_bgl: wgpu::BindGroupLayout,
    query_work_bgl: wgpu::BindGroupLayout,
    reduce_bgl: wgpu::BindGroupLayout,
    build_leaves: wgpu::ComputePipeline,
    build_internal: wgpu::ComputePipeline,
    query: wgpu::ComputePipeline,
    reduce: wgpu::ComputePipeline,
}

struct PickSeriesGpu {
    identity: PickIdentity,
    x_column: ColumnId,
    y_column: ColumnId,
    style_index_column: Option<ColumnId>,
    x_handle: ColumnHandle,
    y_handle: ColumnHandle,
    style_index_handle: Option<ColumnHandle>,
    query_params: wgpu::Buffer,
    nodes: wgpu::Buffer,
    query_data_bg: wgpu::BindGroup,
    query_work_bg: wgpu::BindGroup,
    result: wgpu::Buffer,
    point_count: u32,
    node_count: u32,
    root_index: u32,
    flags: u32,
    base_radius_px: f32,
    base_shape_id: u32,
    line_half_width_px: f32,
    max_extent_px: f32,
    style_count: u32,
    override_count: u32,
    style_index_base: u32,
    style_index_len: u32,
}

struct ReduceResources {
    candidates: wgpu::Buffer,
    final_result: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Persistent exact GPU picking engine.
pub struct GpuPickEngine {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipelines: PickPipelines,
    series: Vec<PickSeriesGpu>,
    identities: Arc<[PickIdentity]>,
    reduce: ReduceResources,
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn create_pipelines(device: &wgpu::Device) -> PickPipelines {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figgy exact GPU pick shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("gpu_pick.wgsl").into()),
    });

    let build_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy GPU pick BVH build bgl"),
        entries: &[
            storage_entry(0, true),
            storage_entry(1, false),
            uniform_entry(2),
        ],
    });
    let query_data_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy GPU pick query data bgl"),
        entries: &[storage_entry(0, true), storage_entry(1, false)],
    });
    let query_work_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy GPU pick query work bgl"),
        entries: &[
            storage_entry(0, false),
            storage_entry(1, false),
            uniform_entry(2),
            storage_entry(3, true),
            storage_entry(4, true),
            storage_entry(5, false),
        ],
    });
    let reduce_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy GPU pick reduction bgl"),
        entries: &[storage_entry(3, true), storage_entry(4, false)],
    });

    let build_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy GPU pick BVH build layout"),
        bind_group_layouts: &[&build_bgl],
        push_constant_ranges: &[],
    });
    let query_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy GPU pick query layout"),
        bind_group_layouts: &[&query_data_bgl, &query_work_bgl],
        push_constant_ranges: &[],
    });
    let reduce_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy GPU pick reduction layout"),
        bind_group_layouts: &[&reduce_bgl],
        push_constant_ranges: &[],
    });
    let pipeline = |layout: &wgpu::PipelineLayout, entry: &str, label: &'static str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            module: &shader,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    };

    PickPipelines {
        build_leaves: pipeline(
            &build_layout,
            "pick_build_leaves",
            "figgy GPU pick leaf build pipeline",
        ),
        build_internal: pipeline(
            &build_layout,
            "pick_build_internal",
            "figgy GPU pick internal build pipeline",
        ),
        query: pipeline(
            &query_layout,
            "pick_query_bvh",
            "figgy GPU pick traversal pipeline",
        ),
        reduce: pipeline(
            &reduce_layout,
            "pick_reduce_series",
            "figgy GPU pick series reduction pipeline",
        ),
        build_bgl,
        query_data_bgl,
        query_work_bgl,
        reduce_bgl,
    }
}

fn create_buffer_checked(
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Buffer, GpuPickError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_buffer(desc)))
        .map_err(|_| GpuPickError::AllocationFailed { resource })
}

fn create_buffer_init_checked(
    device: &wgpu::Device,
    label: &'static str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
    resource: &'static str,
) -> Result<wgpu::Buffer, GpuPickError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage,
        })
    }))
    .map_err(|_| GpuPickError::AllocationFailed { resource })
}

fn nonnegative(v: f32) -> f32 {
    if !v.is_nan() && v > 0.0 { v } else { 0.0 }
}

fn base_shape_id(shape_id: u32) -> u32 {
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

fn shape_scale(shape_id: u32) -> f32 {
    match base_shape_id(shape_id) {
        1 => 0.886_226_95,
        2 | 5 | 6 | 7 => MAX_SHAPE_SCALE,
        3 => 1.253_314_1,
        9 => 1.149_139_9,
        10 => 1.099_636_1,
        11 => 1.053_907_4,
        12 => 1.462_850_3,
        _ => 1.0,
    }
}

fn gpu_meta_u32(v: f32) -> Option<u32> {
    if !v.is_finite() || !(0.0..4_294_967_296.0).contains(&v) || v.fract() != 0.0 {
        None
    } else {
        Some(v as u32)
    }
}

fn style_max_extent(series_id: &str, scatter: &GpuPickScatter<'_>) -> Result<f32, GpuPickError> {
    let mut maximum_radius = nonnegative(scatter.base_radius_px);
    let mut maximum_scale = shape_scale(scatter.base_shape_id);
    let Some(map) = scatter.style_map.as_ref() else {
        return Ok(maximum_radius * maximum_scale);
    };

    if map.style_meta.style_count as usize > map.style_slots.len() {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "style_count exceeds the supplied slot array",
        });
    }
    if map.style_meta.override_count as usize > map.style_overrides.len() {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "override_count exceeds the supplied override array",
        });
    }
    let has_index = map.style_meta.has_index != 0;
    if map.style_meta.has_index > 1 || has_index != map.style_index_column.is_some() {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "has_index and style_index_column disagree",
        });
    }

    let mut visit = |meta: [f32; 4]| -> Result<(), GpuPickError> {
        let mask = gpu_meta_u32(meta[2]).ok_or_else(|| GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "a style mask is not a finite integer",
        })?;
        if mask & STYLE_MASK_RADIUS != 0 {
            maximum_radius = maximum_radius.max(nonnegative(meta[0]));
        }
        if mask & STYLE_MASK_SHAPE != 0 {
            let shape = gpu_meta_u32(meta[1]).ok_or_else(|| GpuPickError::InvalidStyleMap {
                series_id: series_id.to_owned(),
                reason: "a mapped shape id is not a finite integer",
            })?;
            maximum_scale = maximum_scale.max(shape_scale(shape));
        }
        Ok(())
    };
    for slot in &map.style_slots[..map.style_meta.style_count as usize] {
        visit(slot.meta)?;
    }
    for override_ in &map.style_overrides[..map.style_meta.override_count as usize] {
        visit(override_.meta)?;
    }

    // Radius and shape changes may originate in different table/override
    // rows.  Their product is deliberately a cross-product upper bound; leaf
    // resolution still applies rows in exact production order.
    Ok(maximum_radius * maximum_scale)
}

fn lane_base(handle: ColumnHandle, series_id: &str) -> Result<u32, GpuPickError> {
    if !handle.offset.is_multiple_of(4) {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "column byte offset is not f32-aligned",
        });
    }
    u32::try_from(handle.offset / 4).map_err(|_| GpuPickError::DeviceLimit {
        resource: "column word offset",
        requested: handle.offset / 4,
        limit: u64::from(u32::MAX),
    })
}

fn packed_level_counts(point_count: u32) -> Vec<u32> {
    let mut levels = vec![point_count.div_ceil(GPU_PICK_BLOCK_POINTS)];
    while levels.last().copied().unwrap_or(1) > 1 {
        levels.push(levels.last().copied().unwrap_or(1).div_ceil(2));
    }
    levels
}

fn same_handle(a: ColumnHandle, b: ColumnHandle) -> bool {
    a.generation == b.generation
        && a.offset == b.offset
        && a.byte_size == b.byte_size
        && a.len_values == b.len_values
}

impl GpuPickEngine {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, GpuPickError> {
        let limits = device.limits();
        if limits.max_compute_workgroup_size_x < GPU_PICK_WORKGROUP_SIZE {
            return Err(GpuPickError::DeviceLimit {
                resource: "max_compute_workgroup_size_x",
                requested: u64::from(GPU_PICK_WORKGROUP_SIZE),
                limit: u64::from(limits.max_compute_workgroup_size_x),
            });
        }
        if limits.max_compute_invocations_per_workgroup < GPU_PICK_WORKGROUP_SIZE {
            return Err(GpuPickError::DeviceLimit {
                resource: "max_compute_invocations_per_workgroup",
                requested: u64::from(GPU_PICK_WORKGROUP_SIZE),
                limit: u64::from(limits.max_compute_invocations_per_workgroup),
            });
        }
        if limits.max_bind_groups < 2 {
            return Err(GpuPickError::DeviceLimit {
                resource: "max_bind_groups",
                requested: 2,
                limit: u64::from(limits.max_bind_groups),
            });
        }
        if limits.max_storage_buffers_per_shader_stage < 7 {
            return Err(GpuPickError::DeviceLimit {
                resource: "max_storage_buffers_per_shader_stage",
                requested: 7,
                limit: u64::from(limits.max_storage_buffers_per_shader_stage),
            });
        }

        let pipelines = create_pipelines(&device);
        let reduce = Self::create_reduce_resources(&device, &pipelines.reduce_bgl, 1)?;
        Ok(Self {
            device,
            queue,
            pipelines,
            series: Vec::new(),
            identities: Arc::from([]),
            reduce,
        })
    }

    fn refresh_identities(&mut self) {
        self.identities = self
            .series
            .iter()
            .map(|series| series.identity.clone())
            .collect::<Vec<_>>()
            .into();
    }

    fn create_reduce_resources(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        candidate_count: usize,
    ) -> Result<ReduceResources, GpuPickError> {
        let count = candidate_count.max(1) as u64;
        let candidate_size =
            count
                .checked_mul(CANDIDATE_BYTES)
                .ok_or(GpuPickError::DeviceLimit {
                    resource: "candidate buffer",
                    requested: u64::MAX,
                    limit: device.limits().max_buffer_size,
                })?;
        let candidates = create_buffer_checked(
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick per-series candidates"),
                size: candidate_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "per-series candidate buffer",
        )?;
        let final_result = create_buffer_checked(
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick final scalar"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "final pick scalar buffer",
        )?;
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy GPU pick reduction bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: candidates.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: final_result.as_entire_binding(),
                },
            ],
        });
        Ok(ReduceResources {
            candidates,
            final_result,
            bind_group,
        })
    }

    /// Build one persistent exact index from `pool`.
    ///
    /// Every later [`Self::pick`] must receive that same pool instance. Rebuild
    /// this series after any referenced column content replacement, including a
    /// remove+readd that reuses the same offset, length, and pool generation;
    /// a positional handle cannot distinguish that replacement by itself.
    pub fn add_series(
        &mut self,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let limits = self.device.limits();
        if pool.capacity() > u64::from(limits.max_storage_buffer_binding_size) {
            return Err(GpuPickError::DeviceLimit {
                resource: "column-pool storage binding",
                requested: pool.capacity(),
                limit: u64::from(limits.max_storage_buffer_binding_size),
            });
        }
        let x_handle = pool
            .handle_for(&descriptor.x_column)
            .ok_or_else(|| GpuPickError::MissingColumn(descriptor.x_column.clone()))?;
        let y_handle = pool
            .handle_for(&descriptor.y_column)
            .ok_or_else(|| GpuPickError::MissingColumn(descriptor.y_column.clone()))?;
        let paired_len = x_handle.len_values.min(y_handle.len_values);
        if paired_len == 0 {
            return Err(GpuPickError::EmptySeries(descriptor.series_id));
        }
        let point_count = u32::try_from(paired_len).map_err(|_| GpuPickError::TooManyValues {
            series_id: descriptor.series_id.clone(),
            count: paired_len,
        })?;

        let mut flags = 0u32;
        let mut base_radius_px = 0.0;
        let mut base_shape_id = 0u32;
        let mut max_extent_px = 0.0;
        let mut style_count = 0u32;
        let mut override_count = 0u32;
        let mut style_index_column = None;
        let mut style_index_handle = None;
        let mut style_index_base = 0u32;
        let mut style_index_len = 0u32;

        let dummy_slot = [ScatterStyleSlotGpu {
            color_premul: [0.0; 4],
            meta: [0.0; 4],
        }];
        let dummy_override = [ScatterStyleOverrideGpu {
            point_index: 0,
            _pad: [0; 3],
            color_premul: [0.0; 4],
            meta: [0.0; 4],
        }];
        let mut slot_bytes: &[u8] = bytemuck::cast_slice(&dummy_slot);
        let mut override_bytes: &[u8] = bytemuck::cast_slice(&dummy_override);

        if let Some(scatter) = descriptor.scatter.as_ref() {
            flags |= FLAG_SCATTER;
            base_radius_px = nonnegative(scatter.base_radius_px);
            base_shape_id = scatter.base_shape_id;
            max_extent_px = style_max_extent(&descriptor.series_id, scatter)?;
            if let Some(map) = scatter.style_map.as_ref() {
                flags |= FLAG_STYLE_MAP;
                style_count = map.style_meta.style_count;
                override_count = map.style_meta.override_count;
                if !map.style_slots.is_empty() {
                    slot_bytes = bytemuck::cast_slice(map.style_slots);
                }
                if !map.style_overrides.is_empty() {
                    override_bytes = bytemuck::cast_slice(map.style_overrides);
                }
                if let Some(column_id) = map.style_index_column.as_ref() {
                    flags |= FLAG_STYLE_INDEX;
                    let handle = pool
                        .handle_for(column_id)
                        .ok_or_else(|| GpuPickError::MissingColumn(column_id.clone()))?;
                    style_index_base = lane_base(handle, &descriptor.series_id)?;
                    style_index_len = u32::try_from(handle.len_values).unwrap_or(u32::MAX);
                    style_index_column = Some(column_id.clone());
                    style_index_handle = Some(handle);
                }
            }
        }

        let line_half_width_px = descriptor
            .line_width_px
            .map(|width| nonnegative(width) * 0.5)
            .unwrap_or(0.0);
        if descriptor.line_width_px.is_some() {
            flags |= FLAG_LINE;
            max_extent_px = max_extent_px.max(line_half_width_px);
        }
        if flags & (FLAG_SCATTER | FLAG_LINE) == 0 {
            return Err(GpuPickError::NoPickPrimitive(descriptor.series_id));
        }

        let x_base = lane_base(x_handle, &descriptor.series_id)?;
        let y_base = lane_base(y_handle, &descriptor.series_id)?;
        let levels = packed_level_counts(point_count);
        let node_count_u64: u64 = levels.iter().map(|&count| u64::from(count)).sum();
        let node_count = u32::try_from(node_count_u64).map_err(|_| GpuPickError::DeviceLimit {
            resource: "BVH node index",
            requested: node_count_u64,
            limit: u64::from(u32::MAX),
        })?;
        let node_bytes =
            node_count_u64
                .checked_mul(NODE_BYTES)
                .ok_or(GpuPickError::DeviceLimit {
                    resource: "BVH node buffer",
                    requested: u64::MAX,
                    limit: limits.max_buffer_size,
                })?;
        let storage_limit = u64::from(limits.max_storage_buffer_binding_size);
        if node_bytes > limits.max_buffer_size || node_bytes > storage_limit {
            return Err(GpuPickError::DeviceLimit {
                resource: "BVH node buffer",
                requested: node_bytes,
                limit: limits.max_buffer_size.min(storage_limit),
            });
        }
        let queue_bytes = node_count_u64
            .checked_mul(4)
            .ok_or(GpuPickError::DeviceLimit {
                resource: "BVH traversal queue",
                requested: u64::MAX,
                limit: limits.max_buffer_size,
            })?;

        let nodes = create_buffer_checked(
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick packed BVH"),
                size: node_bytes,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
            "packed BVH buffer",
        )?;
        let node_queue = create_buffer_checked(
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick traversal queue"),
                size: queue_bytes.max(4),
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
            "BVH traversal queue",
        )?;
        let queue_state = create_buffer_checked(
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick queue state"),
                size: QUEUE_STATE_BYTES,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
            "BVH queue state",
        )?;
        let query_params = create_buffer_checked(
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick query params"),
                size: std::mem::size_of::<PickQueryParamsGpu>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "query uniform",
        )?;
        let style_slots = create_buffer_init_checked(
            &self.device,
            "figgy GPU pick style slots",
            slot_bytes,
            wgpu::BufferUsages::STORAGE,
            "style slot buffer",
        )?;
        let style_overrides = create_buffer_init_checked(
            &self.device,
            "figgy GPU pick style overrides",
            override_bytes,
            wgpu::BufferUsages::STORAGE,
            "style override buffer",
        )?;
        let result = create_buffer_checked(
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick series scalar"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "series pick scalar",
        )?;

        struct BuildDispatch {
            internal: bool,
            bind_group: wgpu::BindGroup,
            workgroups: u32,
        }
        let max_groups = limits.max_compute_workgroups_per_dimension;
        let max_invocations = u64::from(max_groups) * u64::from(GPU_PICK_WORKGROUP_SIZE);
        if max_invocations == 0 {
            return Err(GpuPickError::DeviceLimit {
                resource: "max_compute_workgroups_per_dimension",
                requested: 1,
                limit: 0,
            });
        }
        let mut dispatches = Vec::new();
        let mut append_dispatches = |internal: bool,
                                     input_start: u32,
                                     output_start: u32,
                                     input_count: u32|
         -> Result<(), GpuPickError> {
            let output_count = if internal {
                input_count.div_ceil(2)
            } else {
                input_count
            };
            let mut invocation_base = 0u64;
            while invocation_base < u64::from(output_count) {
                let invocation_count =
                    (u64::from(output_count) - invocation_base).min(max_invocations);
                let workgroups =
                    u32::try_from(invocation_count.div_ceil(u64::from(GPU_PICK_WORKGROUP_SIZE)))
                        .map_err(|_| GpuPickError::DeviceLimit {
                            resource: "BVH build dispatch",
                            requested: invocation_count,
                            limit: max_invocations,
                        })?;
                let params = PickBuildParamsGpu {
                    point_count,
                    x_base,
                    y_base,
                    input_start,
                    output_start,
                    input_count,
                    invocation_base: invocation_base as u32,
                    include_line_boundary: u32::from(flags & FLAG_LINE != 0),
                };
                let params_buffer = create_buffer_init_checked(
                    &self.device,
                    "figgy GPU pick build params",
                    bytemuck::bytes_of(&params),
                    wgpu::BufferUsages::UNIFORM,
                    "BVH build uniform",
                )?;
                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("figgy GPU pick build bg"),
                    layout: &self.pipelines.build_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: pool.buffer().as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: nodes.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: params_buffer.as_entire_binding(),
                        },
                    ],
                });
                dispatches.push(BuildDispatch {
                    internal,
                    bind_group,
                    workgroups,
                });
                invocation_base += invocation_count;
            }
            Ok(())
        };

        append_dispatches(false, 0, 0, levels[0])?;
        let mut input_start = 0u32;
        let mut output_start = levels[0];
        for level in 0..levels.len().saturating_sub(1) {
            append_dispatches(true, input_start, output_start, levels[level])?;
            input_start = output_start;
            output_start += levels[level + 1];
        }
        let root_index = node_count - 1;

        let mut build_encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy GPU pick BVH build encoder"),
                });
        {
            let mut pass = build_encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick BVH build"),
                timestamp_writes: None,
            });
            for dispatch in &dispatches {
                pass.set_pipeline(if dispatch.internal {
                    &self.pipelines.build_internal
                } else {
                    &self.pipelines.build_leaves
                });
                pass.set_bind_group(0, &dispatch.bind_group, &[]);
                pass.dispatch_workgroups(dispatch.workgroups, 1, 1);
            }
        }
        self.queue.submit(std::iter::once(build_encoder.finish()));

        let query_data_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy GPU pick query data bg"),
            layout: &self.pipelines.query_data_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pool.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: nodes.as_entire_binding(),
                },
            ],
        });
        let query_work_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy GPU pick query work bg"),
            layout: &self.pipelines.query_work_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: node_queue.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: queue_state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: query_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: style_slots.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: style_overrides.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: result.as_entire_binding(),
                },
            ],
        });

        let next_count = self.series.len() + 1;
        let next_reduce =
            Self::create_reduce_resources(&self.device, &self.pipelines.reduce_bgl, next_count)?;
        let identity = PickIdentity {
            source_id: descriptor.source_id,
            series_id: descriptor.series_id,
        };
        let id = GpuPickSeriesId(self.series.len() as u32);
        self.series.push(PickSeriesGpu {
            identity: identity.clone(),
            x_column: descriptor.x_column,
            y_column: descriptor.y_column,
            style_index_column,
            x_handle,
            y_handle,
            style_index_handle,
            query_params,
            nodes,
            query_data_bg,
            query_work_bg,
            result,
            point_count,
            node_count,
            root_index,
            flags,
            base_radius_px,
            base_shape_id,
            line_half_width_px,
            max_extent_px,
            style_count,
            override_count,
            style_index_base,
            style_index_len,
        });
        self.reduce = next_reduce;
        self.refresh_identities();
        Ok(id)
    }

    /// Replace one registry slot without rebuilding any other series BVH.
    ///
    /// The replacement is built first, so a build/allocation error leaves the
    /// existing slot untouched. The returned id is the unchanged positional
    /// slot; later-series equal-distance precedence is therefore preserved.
    pub fn replace_series_at(
        &mut self,
        index: usize,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let len = self.series.len();
        if index >= len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        self.add_series(pool, descriptor)?;
        let replacement = self.series.pop().ok_or(GpuPickError::InvalidGpuResult)?;
        self.series[index] = replacement;
        // add_series allocated len + 1 reduction slots. Keeping that small
        // spare slot avoids a second allocation; pick() clears the complete
        // candidate buffer before copying live results.
        self.refresh_identities();
        Ok(GpuPickSeriesId(index as u32))
    }

    /// Insert one registry slot without rebuilding any existing series BVH.
    ///
    /// The new series is built completely at the tail before it is moved into
    /// position, so a build/allocation error leaves the registry and its tie
    /// order untouched. index equal to self.series.len() appends the series.
    pub fn insert_series_at(
        &mut self,
        index: usize,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let len = self.series.len();
        if index > len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        self.add_series(pool, descriptor)?;
        let inserted = self.series.pop().ok_or(GpuPickError::InvalidGpuResult)?;
        self.series.insert(index, inserted);
        self.refresh_identities();
        Ok(GpuPickSeriesId(index as u32))
    }

    /// Remove one positional registry slot without rebuilding surviving BVHs.
    /// Later slots shift left, matching their new production traversal order.
    pub fn remove_series_at(&mut self, index: usize) -> Result<(), GpuPickError> {
        let len = self.series.len();
        if index >= len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        self.series.remove(index);
        self.refresh_identities();
        Ok(())
    }

    /// Drop every registered series while retaining reusable pipelines and
    /// reduction storage.
    pub fn clear_series(&mut self) {
        self.series.clear();
        self.refresh_identities();
    }

    /// Rebind registered columns after an in-place [`ColumnPool::defragment`].
    ///
    /// Defragmentation changes the backing buffer, offsets, and generations,
    /// but preserves every value, so the value-space BVH nodes remain valid.
    /// This method verifies every referenced column still exists with exactly
    /// its registered logical length, recreates the whole-pool data bind
    /// groups, and updates all handles/bases without rebuilding any BVH.
    ///
    /// Use this only with the same `ColumnPool` after a relocation-only
    /// defragment. It is not valid after replacing column contents, even when
    /// ids and lengths happen to match; call replace_series_at/rebuild instead.
    pub fn rebind_columns(&mut self, pool: &ColumnPool) -> Result<(), GpuPickError> {
        let storage_limit = u64::from(self.device.limits().max_storage_buffer_binding_size);
        if pool.capacity() > storage_limit {
            return Err(GpuPickError::DeviceLimit {
                resource: "column-pool storage binding",
                requested: pool.capacity(),
                limit: storage_limit,
            });
        }

        struct RebindUpdate {
            x_handle: ColumnHandle,
            y_handle: ColumnHandle,
            style_index_handle: Option<ColumnHandle>,
            style_index_base: u32,
            style_index_len: u32,
            query_data_bg: wgpu::BindGroup,
        }

        let mut updates = Vec::with_capacity(self.series.len());
        for series in &self.series {
            let relocated =
                |id: &ColumnId, expected: ColumnHandle| -> Result<ColumnHandle, GpuPickError> {
                    let Some(current) = pool.handle_for(id) else {
                        return Err(GpuPickError::StaleColumn {
                            series_id: series.identity.series_id.clone(),
                            column_id: id.clone(),
                        });
                    };
                    if current.len_values != expected.len_values
                        || current.byte_size != expected.byte_size
                    {
                        return Err(GpuPickError::StaleColumn {
                            series_id: series.identity.series_id.clone(),
                            column_id: id.clone(),
                        });
                    }
                    Ok(current)
                };

            let x_handle = relocated(&series.x_column, series.x_handle)?;
            let y_handle = relocated(&series.y_column, series.y_handle)?;
            let _ = lane_base(x_handle, &series.identity.series_id)?;
            let _ = lane_base(y_handle, &series.identity.series_id)?;

            let (style_index_handle, style_index_base, style_index_len) = if let Some(id) =
                series.style_index_column.as_ref()
            {
                let expected = series
                    .style_index_handle
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                let current = relocated(id, expected)?;
                let base = lane_base(current, &series.identity.series_id)?;
                let len =
                    u32::try_from(current.len_values).map_err(|_| GpuPickError::TooManyValues {
                        series_id: series.identity.series_id.clone(),
                        count: current.len_values,
                    })?;
                (Some(current), base, len)
            } else {
                (None, 0, 0)
            };

            let query_data_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick rebound query data bg"),
                layout: &self.pipelines.query_data_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: pool.buffer().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: series.nodes.as_entire_binding(),
                    },
                ],
            });
            updates.push(RebindUpdate {
                x_handle,
                y_handle,
                style_index_handle,
                style_index_base,
                style_index_len,
                query_data_bg,
            });
        }

        for (series, update) in self.series.iter_mut().zip(updates) {
            series.x_handle = update.x_handle;
            series.y_handle = update.y_handle;
            series.style_index_handle = update.style_index_handle;
            series.style_index_base = update.style_index_base;
            series.style_index_len = update.style_index_len;
            series.query_data_bg = update.query_data_bg;
        }
        Ok(())
    }

    fn validate_series_columns(
        pool: &ColumnPool,
        series: &PickSeriesGpu,
    ) -> Result<(), GpuPickError> {
        for (id, expected) in [
            (&series.x_column, series.x_handle),
            (&series.y_column, series.y_handle),
        ] {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: series.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            };
            if !same_handle(current, expected) {
                return Err(GpuPickError::StaleColumn {
                    series_id: series.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            }
        }
        if let (Some(id), Some(expected)) = (
            series.style_index_column.as_ref(),
            series.style_index_handle,
        ) {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: series.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            };
            if !same_handle(current, expected) {
                return Err(GpuPickError::StaleColumn {
                    series_id: series.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            }
        }
        Ok(())
    }

    /// Submit one exact pick query and return an owned async readback ticket.
    pub fn pick(
        &self,
        pool: &ColumnPool,
        query: GpuPickQuery,
    ) -> Result<GpuPickTicket, GpuPickError> {
        let [cursor_x, cursor_y] = query.canvas_position_px;
        if !cursor_x.is_finite()
            || !cursor_y.is_finite()
            || !query.max_distance_px.is_finite()
            || query.max_distance_px < 0.0
        {
            return Ok(GpuPickTicket::ready_none());
        }
        let [chart_x, chart_y, chart_width, chart_height] = query.chart_rect_px;
        if !chart_x.is_finite()
            || !chart_y.is_finite()
            || !chart_width.is_finite()
            || !chart_height.is_finite()
            || chart_width <= 0.0
            || chart_height <= 0.0
        {
            return Ok(GpuPickTicket::ready_none());
        }
        if let Some([x, y, width, height]) = query.data_area_px {
            let x1 = x + width;
            let y1 = y + height;
            if !x.is_finite()
                || !y.is_finite()
                || !width.is_finite()
                || !height.is_finite()
                || cursor_x < x
                || cursor_x > x1
                || cursor_y < y
                || cursor_y > y1
            {
                return Ok(GpuPickTicket::ready_none());
            }
        }
        if self.series.is_empty() {
            return Ok(GpuPickTicket::ready_none());
        }

        for (series_order, series) in self.series.iter().enumerate() {
            Self::validate_series_columns(pool, series)?;
            let params = PickQueryParamsGpu {
                transform: query.transform,
                cursor_chart: [cursor_x, cursor_y, chart_x, chart_y],
                chart_limits: [
                    chart_width,
                    chart_height,
                    query.max_distance_px,
                    series.max_extent_px,
                ],
                scatter_line: [series.base_radius_px, series.line_half_width_px, 0.0, 0.0],
                data: [
                    series.point_count,
                    lane_base(series.x_handle, &series.identity.series_id)?,
                    lane_base(series.y_handle, &series.identity.series_id)?,
                    series.style_index_base,
                ],
                style: [
                    series.flags,
                    series.style_count,
                    series.override_count,
                    series.style_index_len,
                ],
                series: [
                    series.root_index,
                    series_order as u32,
                    series.base_shape_id,
                    series.node_count,
                ],
            };
            self.queue
                .write_buffer(&series.query_params, 0, bytemuck::bytes_of(&params));
        }

        let readback = create_buffer_checked(
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick detached readback"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "detached pick readback",
        )?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy exact GPU pick encoder"),
            });
        // Reduction storage may intentionally be larger than the live registry
        // after replace/remove. Clear every slot so a prior query's candidate
        // can never participate as a stale tail entry.
        encoder.clear_buffer(&self.reduce.candidates, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick traversal"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines.query);
            for series in &self.series {
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        for (index, series) in self.series.iter().enumerate() {
            encoder.copy_buffer_to_buffer(
                &series.result,
                0,
                &self.reduce.candidates,
                index as u64 * CANDIDATE_BYTES,
                CANDIDATE_BYTES,
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick cross-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines.reduce);
            pass.set_bind_group(0, &self.reduce.bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&self.reduce.final_result, 0, &readback, 0, CANDIDATE_BYTES);
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..CANDIDATE_BYTES);
        let (sender, receiver) = oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        Ok(GpuPickTicket {
            state: GpuPickTicketState::Pending {
                device: Arc::clone(&self.device),
                readback,
                receiver,
                identities: Arc::clone(&self.identities),
            },
        })
    }
}

enum GpuPickTicketState {
    ReadyNone,
    Pending {
        device: Arc<wgpu::Device>,
        readback: wgpu::Buffer,
        receiver: oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>,
        identities: Arc<[PickIdentity]>,
    },
}

/// Owned, detached asynchronous scalar readback.
pub struct GpuPickTicket {
    state: GpuPickTicketState,
}

impl GpuPickTicket {
    fn ready_none() -> Self {
        Self {
            state: GpuPickTicketState::ReadyNone,
        }
    }

    pub async fn resolve(self) -> Result<Option<PickedPoint>, GpuPickError> {
        let GpuPickTicketState::Pending {
            device: _device,
            readback,
            receiver,
            identities,
        } = self.state
        else {
            return Ok(None);
        };

        #[cfg(not(target_arch = "wasm32"))]
        let _ = _device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        receiver
            .await
            .map_err(|_| GpuPickError::MapChannelClosed)?
            .map_err(GpuPickError::MapFailed)?;
        let slice = readback.slice(..CANDIDATE_BYTES);
        let mapped = slice.get_mapped_range();
        let candidate = *bytemuck::from_bytes::<PickCandidateGpu>(&mapped);
        drop(mapped);
        readback.unmap();

        match candidate.valid {
            0 => Ok(None),
            2 => Err(GpuPickError::TraversalOverflow),
            1 => {
                let identity = identities
                    .get(candidate.series_order as usize)
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                if !candidate.data_x.is_finite()
                    || !candidate.data_y.is_finite()
                    || !candidate.distance_px.is_finite()
                {
                    return Err(GpuPickError::InvalidGpuResult);
                }
                Ok(Some(PickedPoint {
                    source_id: identity.source_id.clone(),
                    series_id: identity.series_id.clone(),
                    point_index: candidate.point_index as usize,
                    data_x: candidate.data_x,
                    data_y: candidate.data_y,
                    distance_px: candidate.distance_px,
                }))
            }
            _ => Err(GpuPickError::InvalidGpuResult),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::color::Color;
    use crate::config::{AxisScale, Config};
    use crate::data::{Column, split_f64_to_f32_pair};
    use crate::data_config::{
        DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig,
        DataScatterPointStyleOverride, DataScatterStyleConfig, ScatterShape, SeriesConfig,
    };
    use crate::layout::{ChartArea, Rect};
    use crate::line::LineStylePreset;
    use crate::pick::{PointColumnLookup, PointPickOptions, pick_nearest_point};

    struct CpuColumns(HashMap<ColumnId, Vec<f32>>);

    impl CpuColumns {
        fn new(entries: &[(&str, &[f32])]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|(id, values)| ((*id).to_owned(), values.to_vec()))
                    .collect(),
            )
        }
    }

    impl PointColumnLookup for CpuColumns {
        fn get_f32_column(&self, id: &ColumnId) -> Option<&[f32]> {
            self.0.get(id).map(Vec::as_slice)
        }
    }

    fn f32_column(values: Vec<f32>) -> Column<f32> {
        let min = values.iter().copied().fold(f32::INFINITY, f32::min);
        let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        Column {
            data: values,
            min,
            max,
        }
    }

    fn f64_column(values: Vec<f64>) -> Column<f64> {
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column {
            data: values,
            min,
            max,
        }
    }

    fn parity_config(width: u32, height: u32, x: (f64, f64), y: (f64, f64)) -> Config {
        let mut config = crate::default::default_config();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width,
            height,
        });
        config.bottom_x.min = x.0;
        config.bottom_x.max = x.1;
        config.left_y.min = y.0;
        config.left_y.max = y.1;
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
        config
    }

    fn query_from_config(
        config: &Config,
        canvas_position_px: [f32; 2],
        max_distance_px: f32,
    ) -> GpuPickQuery {
        let chart = config.chart_area.0;
        let data_area_px = config.data_area().ok().map(|area| {
            let rect = area.0;
            [
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ]
        });
        GpuPickQuery {
            transform: crate::data_render::scatter_transform_from_config(config),
            chart_rect_px: [
                chart.x as f32,
                chart.y as f32,
                chart.width as f32,
                chart.height as f32,
            ],
            data_area_px,
            canvas_position_px,
            max_distance_px,
        }
    }

    fn resolve_pick(
        engine: &GpuPickEngine,
        pool: &ColumnPool,
        query: GpuPickQuery,
    ) -> Option<PickedPoint> {
        pollster::block_on(engine.pick(pool, query).unwrap().resolve()).unwrap()
    }

    fn cpu_pick(
        config: &Config,
        series: &[SeriesConfig],
        columns: &CpuColumns,
        canvas_position_px: [f32; 2],
        max_distance_px: f32,
    ) -> Option<PickedPoint> {
        pick_nearest_point(
            config,
            series,
            columns,
            canvas_position_px[0],
            canvas_position_px[1],
            PointPickOptions { max_distance_px },
        )
    }

    fn assert_pick_parity(cpu: Option<PickedPoint>, gpu: Option<PickedPoint>) {
        match (cpu, gpu) {
            (None, None) => {}
            (Some(cpu), Some(gpu)) => {
                assert_eq!(gpu.source_id, cpu.source_id);
                assert_eq!(gpu.series_id, cpu.series_id);
                assert_eq!(gpu.point_index, cpu.point_index);
                assert_eq!(gpu.data_x.to_bits(), cpu.data_x.to_bits());
                assert_eq!(gpu.data_y.to_bits(), cpu.data_y.to_bits());
                assert!(
                    (gpu.distance_px - cpu.distance_px).abs() <= 0.001,
                    "GPU distance {} differs from CPU distance {}",
                    gpu.distance_px,
                    cpu.distance_px
                );
            }
            (cpu, gpu) => panic!("CPU/GPU picker mismatch: CPU={cpu:?}, GPU={gpu:?}"),
        }
    }

    fn scatter_series(
        series_id: &str,
        source_id: Option<&str>,
        x_column: &str,
        y_column: &str,
        scatter: DataScatterStyleConfig,
    ) -> SeriesConfig {
        SeriesConfig {
            series_id: series_id.to_owned(),
            source_id: source_id.map(str::to_owned),
            label: None,
            x_column: x_column.to_owned(),
            y_column: y_column.to_owned(),
            render_type: DataRenderType::Scatter { scatter },
        }
    }

    fn line_series(
        series_id: &str,
        source_id: Option<&str>,
        x_column: &str,
        y_column: &str,
        line_width: f32,
    ) -> SeriesConfig {
        SeriesConfig {
            series_id: series_id.to_owned(),
            source_id: source_id.map(str::to_owned),
            label: None,
            x_column: x_column.to_owned(),
            y_column: y_column.to_owned(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width,
                },
            },
        }
    }

    fn scatter_descriptor(
        series_id: &str,
        x_column: &str,
        y_column: &str,
    ) -> GpuPickSeriesDescriptor<'static> {
        GpuPickSeriesDescriptor {
            source_id: Some(format!("source-{series_id}")),
            series_id: series_id.to_owned(),
            x_column: x_column.to_owned(),
            y_column: y_column.to_owned(),
            scatter: Some(GpuPickScatter {
                base_radius_px: 2.0,
                base_shape_id: 0,
                style_map: None,
            }),
            line_width_px: None,
        }
    }

    fn test_query(canvas_position_px: [f32; 2]) -> GpuPickQuery {
        GpuPickQuery {
            transform: ScatterTransform {
                data_min: [0.0, 0.0],
                data_max: [10.0, 10.0],
                data_min_lo: [0.0; 2],
                data_max_lo: [0.0; 2],
                scale_log: [0.0; 2],
                pixel_to_ndc: [0.02; 2],
                style_params: [[0.0; 4]; 3],
            },
            chart_rect_px: [0.0, 0.0, 100.0, 100.0],
            data_area_px: Some([0.0, 0.0, 100.0, 100.0]),
            canvas_position_px,
            max_distance_px: 0.0,
        }
    }

    #[test]
    fn exact_gpu_scatter_style_mapping_matches_cpu_exhaustive_picker() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let config = parity_config(100, 100, (0.0, 10.0), (0.0, 10.0));
        let xs = [2.0, 5.0, 8.0];
        let ys = [5.0, 5.0, 5.0];
        let style_indices = [1.0, 0.0, 99.0];
        let cpu_columns = CpuColumns::new(&[
            ("parity-sx", &xs),
            ("parity-sy", &ys),
            ("parity-si", &style_indices),
        ]);

        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        for (id, values) in [
            ("parity-sx", xs.as_slice()),
            ("parity-sy", ys.as_slice()),
            ("parity-si", style_indices.as_slice()),
        ] {
            pool.add_column(id.into(), &f32_column(values.to_vec()), &device, &queue)
                .unwrap();
        }

        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::Circle,
            point_size: 1.0,
            point_style_table: Some(vec![
                DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::Square),
                    point_size: Some(2.0),
                },
                DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::Triangle),
                    point_size: Some(3.0),
                },
            ]),
            point_style_index_column: Some("parity-si".into()),
            point_style_overrides: Some(vec![DataScatterPointStyleOverride {
                index: 1,
                style: DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::Diamond),
                    point_size: Some(5.0),
                },
            }]),
        };
        let cpu_series = [scatter_series(
            "mapped",
            Some("mapped-source"),
            "parity-sx",
            "parity-sy",
            scatter,
        )];
        let slots = [
            ScatterStyleSlotGpu {
                color_premul: [0.0; 4],
                meta: [
                    2.0,
                    crate::data_render::shape_id(&ScatterShape::Square) as f32,
                    (STYLE_MASK_RADIUS | STYLE_MASK_SHAPE) as f32,
                    0.0,
                ],
            },
            ScatterStyleSlotGpu {
                color_premul: [0.0; 4],
                meta: [
                    3.0,
                    crate::data_render::shape_id(&ScatterShape::Triangle) as f32,
                    (STYLE_MASK_RADIUS | STYLE_MASK_SHAPE) as f32,
                    0.0,
                ],
            },
        ];
        let overrides = [ScatterStyleOverrideGpu {
            point_index: 1,
            _pad: [0; 3],
            color_premul: [0.0; 4],
            meta: [
                5.0,
                crate::data_render::shape_id(&ScatterShape::Diamond) as f32,
                (STYLE_MASK_RADIUS | STYLE_MASK_SHAPE) as f32,
                0.0,
            ],
        }];
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("mapped-source".into()),
                    series_id: "mapped".into(),
                    x_column: "parity-sx".into(),
                    y_column: "parity-sy".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 1.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::Circle),
                        style_map: Some(GpuPickScatterStyle {
                            style_index_column: Some("parity-si".into()),
                            style_slots: &slots,
                            style_overrides: &overrides,
                            style_meta: ScatterStyleMapMeta {
                                style_count: slots.len() as u32,
                                override_count: overrides.len() as u32,
                                has_index: 1,
                                _pad: 0,
                            },
                        }),
                    }),
                    line_width_px: None,
                },
            )
            .unwrap();

        for (cursor, expected_index) in [([24.5, 50.0], 0), ([56.0, 50.0], 1)] {
            let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
            let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
            assert_pick_parity(cpu.clone(), gpu);
            assert_eq!(cpu.unwrap().point_index, expected_index);
        }

        // An out-of-range style-table index falls back to the one-pixel base
        // circle; 1.25 px outside its center must therefore miss exactly.
        let cursor = [81.25, 50.0];
        let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
        assert_pick_parity(cpu, gpu);
    }

    #[test]
    fn exact_gpu_line_boundary_endpoint_matches_cpu_exhaustive_picker() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let config = parity_config(650, 100, (0.0, 65.0), (0.0, 10.0));
        let xs = (0..66).map(|value| value as f32).collect::<Vec<_>>();
        let ys = vec![5.0; xs.len()];
        let cpu_columns =
            CpuColumns::new(&[("parity-lx", xs.as_slice()), ("parity-ly", ys.as_slice())]);
        let cpu_series = [line_series(
            "boundary-line",
            Some("line-source"),
            "parity-lx",
            "parity-ly",
            0.0,
        )];

        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        pool.add_column("parity-lx".into(), &f32_column(xs), &device, &queue)
            .unwrap();
        pool.add_column("parity-ly".into(), &f32_column(ys), &device, &queue)
            .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("line-source".into()),
                    series_id: "boundary-line".into(),
                    x_column: "parity-lx".into(),
                    y_column: "parity-ly".into(),
                    scatter: None,
                    line_width_px: Some(0.0),
                },
            )
            .unwrap();

        // Segment start 63 belongs to the leaf ending at point 63 while its
        // endpoint 64 is in the next 64-point block.
        for (cursor, expected_index) in [([635.0, 50.0], 63), ([637.5, 50.0], 64)] {
            let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.01);
            let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.01));
            assert_pick_parity(cpu.clone(), gpu);
            assert_eq!(cpu.unwrap().point_index, expected_index);
        }
    }

    #[test]
    fn exact_gpu_log_inverted_and_nonfinite_semantics_match_cpu_picker() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut config = parity_config(120, 100, (1.0, 1_000.0), (0.0, 10.0));
        config.bottom_x.scale = AxisScale::Logarithmic;
        config.left_y.inverted = true;
        let xs = [f32::NAN, f32::INFINITY, 10.0, 100.0];
        let ys = [5.0, 5.0, 2.0, 8.0];
        let cpu_columns = CpuColumns::new(&[("parity-log-x", &xs), ("parity-log-y", &ys)]);
        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::Circle,
            point_size: 2.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
        };
        let cpu_series = [scatter_series(
            "log-inverted",
            None,
            "parity-log-x",
            "parity-log-y",
            scatter,
        )];

        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        pool.add_column(
            "parity-log-x".into(),
            &f32_column(xs.to_vec()),
            &device,
            &queue,
        )
        .unwrap();
        pool.add_column(
            "parity-log-y".into(),
            &f32_column(ys.to_vec()),
            &device,
            &queue,
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "log-inverted".into(),
                    x_column: "parity-log-x".into(),
                    y_column: "parity-log-y".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 2.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::Circle),
                        style_map: None,
                    }),
                    line_width_px: None,
                },
            )
            .unwrap();

        for (cursor, expected_index) in [([40.0, 20.0], 2), ([80.0, 80.0], 3)] {
            let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
            let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
            assert_pick_parity(cpu.clone(), gpu);
            assert_eq!(cpu.unwrap().point_index, expected_index);
        }

        for (cursor, max_distance) in [
            ([f32::NAN, 20.0], 1.0),
            ([40.0, f32::INFINITY], 1.0),
            ([40.0, 20.0], f32::NAN),
            ([40.0, 20.0], -1.0),
            ([-1.0, 20.0], 1.0),
        ] {
            let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, max_distance);
            let gpu = resolve_pick(
                &engine,
                &pool,
                query_from_config(&config, cursor, max_distance),
            );
            assert_pick_parity(cpu, gpu);
        }
    }

    #[test]
    fn exact_gpu_equal_distance_ties_match_cpu_series_and_primitive_order() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let config = parity_config(100, 100, (0.0, 10.0), (0.0, 10.0));
        let first_xs = [4.0, 6.0];
        let first_ys = [5.0, 5.0];
        let later_xs = [5.6];
        let later_ys = [5.0];
        let cpu_columns = CpuColumns::new(&[
            ("parity-tie-x0", &first_xs),
            ("parity-tie-y0", &first_ys),
            ("parity-tie-x1", &later_xs),
            ("parity-tie-y1", &later_ys),
        ]);
        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::Circle,
            point_size: 20.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
        };
        let line = DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 0.0,
        };
        let first_series = SeriesConfig {
            series_id: "first".into(),
            source_id: Some("first-source".into()),
            label: None,
            x_column: "parity-tie-x0".into(),
            y_column: "parity-tie-y0".into(),
            render_type: DataRenderType::ScatterLine { scatter, line },
        };
        let later_series = scatter_series(
            "later",
            Some("later-source"),
            "parity-tie-x1",
            "parity-tie-y1",
            DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_shape: ScatterShape::Circle,
                point_size: 1.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
            },
        );

        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        for (id, values) in [
            ("parity-tie-x0", first_xs.as_slice()),
            ("parity-tie-y0", first_ys.as_slice()),
            ("parity-tie-x1", later_xs.as_slice()),
            ("parity-tie-y1", later_ys.as_slice()),
        ] {
            pool.add_column(id.into(), &f32_column(values.to_vec()), &device, &queue)
                .unwrap();
        }
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("first-source".into()),
                    series_id: "first".into(),
                    x_column: "parity-tie-x0".into(),
                    y_column: "parity-tie-y0".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 20.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::Circle),
                        style_map: None,
                    }),
                    line_width_px: Some(0.0),
                },
            )
            .unwrap();

        // Both scatter points and the line segment have distance zero at this
        // cursor. CPU traversal keeps scatter point 0; GPU reduction must use
        // the same lower-index/scatter-before-line rule.
        let cursor = [56.0, 50.0];
        let cpu = cpu_pick(
            &config,
            std::slice::from_ref(&first_series),
            &cpu_columns,
            cursor,
            0.0,
        );
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
        assert_pick_parity(cpu.clone(), gpu);
        assert_eq!(cpu.unwrap().point_index, 0);

        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("later-source".into()),
                    series_id: "later".into(),
                    x_column: "parity-tie-x1".into(),
                    y_column: "parity-tie-y1".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 1.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::Circle),
                        style_map: None,
                    }),
                    line_width_px: None,
                },
            )
            .unwrap();
        let series = [first_series, later_series];
        let cpu = cpu_pick(&config, &series, &cpu_columns, cursor, 0.0);
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
        assert_pick_parity(cpu.clone(), gpu);
        assert_eq!(cpu.unwrap().series_id, "later");
    }

    #[test]
    fn exact_gpu_f64_residual_selects_point_but_returns_existing_f32_scalar() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let epoch = 1_700_000_000_000.0_f64;
        let xs = [epoch + 0.25, epoch + 0.75];
        let ys = [0.5_f64, 0.5];
        let (x0_hi, x0_lo) = split_f64_to_f32_pair(xs[0]);
        let (x1_hi, x1_lo) = split_f64_to_f32_pair(xs[1]);
        assert_eq!(x0_hi.to_bits(), x1_hi.to_bits());
        assert_ne!(x0_lo.to_bits(), x1_lo.to_bits());

        let config = parity_config(100, 100, (epoch, epoch + 1.0), (0.0, 1.0));
        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        pool.add_hilo_column(
            "parity-f64-x".into(),
            &f64_column(xs.to_vec()),
            &device,
            &queue,
        )
        .unwrap();
        pool.add_hilo_column(
            "parity-f64-y".into(),
            &f64_column(ys.to_vec()),
            &device,
            &queue,
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("f64-source".into()),
                    series_id: "f64-residual".into(),
                    x_column: "parity-f64-x".into(),
                    y_column: "parity-f64-y".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 1.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::Circle),
                        style_map: None,
                    }),
                    line_width_px: None,
                },
            )
            .unwrap();

        for (cursor, expected_index, expected_pair) in [
            ([25.0, 50.0], 0, (x0_hi, x0_lo)),
            ([75.0, 50.0], 1, (x1_hi, x1_lo)),
        ] {
            let picked =
                resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0)).unwrap();
            assert_eq!(picked.point_index, expected_index);
            let public_scalar: f32 = picked.data_x;
            assert_eq!(
                public_scalar.to_bits(),
                (expected_pair.0 + expected_pair.1).to_bits()
            );
        }
    }

    #[test]
    fn insertion_preserves_position_allows_tail_and_is_failure_atomic() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        pool.add_column("pick-ix".into(), &f32_column(vec![5.0]), &device, &queue)
            .unwrap();
        pool.add_column("pick-iy".into(), &f32_column(vec![5.0]), &device, &queue)
            .unwrap();

        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(&pool, scatter_descriptor("first", "pick-ix", "pick-iy"))
            .unwrap();
        engine
            .add_series(&pool, scatter_descriptor("last", "pick-ix", "pick-iy"))
            .unwrap();
        let inserted = engine
            .insert_series_at(1, &pool, scatter_descriptor("middle", "pick-ix", "pick-iy"))
            .unwrap();
        assert_eq!(inserted.index(), 1);

        let tied = pollster::block_on(
            engine
                .pick(&pool, test_query([50.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(tied.series_id, "last");
        engine.remove_series_at(2).unwrap();
        let middle = pollster::block_on(
            engine
                .pick(&pool, test_query([50.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(middle.series_id, "middle");

        assert!(matches!(
            engine.insert_series_at(
                0,
                &pool,
                scatter_descriptor("missing", "pick-missing", "pick-iy"),
            ),
            Err(GpuPickError::MissingColumn(_))
        ));
        assert!(matches!(
            engine.insert_series_at(
                3,
                &pool,
                scatter_descriptor("invalid", "pick-ix", "pick-iy"),
            ),
            Err(GpuPickError::InvalidSeriesIndex { index: 3, len: 2 })
        ));
        let unchanged = pollster::block_on(
            engine
                .pick(&pool, test_query([50.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(unchanged.series_id, "middle");

        let appended = engine
            .insert_series_at(2, &pool, scatter_descriptor("tail", "pick-ix", "pick-iy"))
            .unwrap();
        assert_eq!(appended.index(), 2);
        let tail = pollster::block_on(
            engine
                .pick(&pool, test_query([50.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(tail.series_id, "tail");
    }

    #[test]
    fn registry_mutation_preserves_order_and_clears_stale_reduction_slots() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        for (id, values) in [
            ("pick-ax", vec![2.0]),
            ("pick-ay", vec![5.0]),
            ("pick-bx", vec![8.0]),
            ("pick-by", vec![5.0]),
            ("pick-cx", vec![8.0]),
            ("pick-cy", vec![5.0]),
        ] {
            pool.add_column(id.into(), &f32_column(values), &device, &queue)
                .unwrap();
        }

        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(&pool, scatter_descriptor("a", "pick-ax", "pick-ay"))
            .unwrap();
        engine
            .add_series(&pool, scatter_descriptor("b", "pick-bx", "pick-by"))
            .unwrap();

        let replacement = engine
            .replace_series_at(0, &pool, scatter_descriptor("c", "pick-cx", "pick-cy"))
            .unwrap();
        assert_eq!(replacement.index(), 0);
        let tied = pollster::block_on(
            engine
                .pick(&pool, test_query([80.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(tied.series_id, "b");

        engine.remove_series_at(1).unwrap();
        let no_stale = pollster::block_on(
            engine
                .pick(&pool, test_query([20.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap();
        assert!(no_stale.is_none());

        let remaining = pollster::block_on(
            engine
                .pick(&pool, test_query([80.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(remaining.series_id, "c");
        assert_eq!(remaining.source_id.as_deref(), Some("source-c"));

        engine.clear_series();
        assert!(
            pollster::block_on(
                engine
                    .pick(&pool, test_query([80.0, 50.0]))
                    .unwrap()
                    .resolve()
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn relocation_rebind_keeps_bvh_and_refreshes_all_column_bases() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        pool.add_column("pick-hole".into(), &f32_column(vec![0.0]), &device, &queue)
            .unwrap();
        for (id, values) in [
            ("pick-rx", vec![4.0]),
            ("pick-ry", vec![6.0]),
            ("pick-rstyle", vec![1.0]),
        ] {
            pool.add_column(id.into(), &f32_column(values), &device, &queue)
                .unwrap();
        }
        let slots = [
            ScatterStyleSlotGpu {
                color_premul: [0.0; 4],
                meta: [0.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
            },
            ScatterStyleSlotGpu {
                color_premul: [0.0; 4],
                meta: [3.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
            },
        ];
        let mut engine = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "relocated".into(),
                    x_column: "pick-rx".into(),
                    y_column: "pick-ry".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 0.0,
                        base_shape_id: 0,
                        style_map: Some(GpuPickScatterStyle {
                            style_index_column: Some("pick-rstyle".into()),
                            style_slots: &slots,
                            style_overrides: &[],
                            style_meta: ScatterStyleMapMeta {
                                style_count: 2,
                                override_count: 0,
                                has_index: 1,
                                _pad: 0,
                            },
                        }),
                    }),
                    line_width_px: None,
                },
            )
            .unwrap();

        let before = pollster::block_on(
            engine
                .pick(&pool, test_query([40.0, 40.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(before.series_id, "relocated");

        assert!(pool.remove_column("pick-hole"));
        assert!(pool.defragment(&device, &queue).unwrap());
        assert!(matches!(
            engine.pick(&pool, test_query([40.0, 40.0])),
            Err(GpuPickError::StaleColumn { .. })
        ));
        engine.rebind_columns(&pool).unwrap();

        let after = pollster::block_on(
            engine
                .pick(&pool, test_query([40.0, 40.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(after.series_id, "relocated");
        assert_eq!([after.data_x, after.data_y], [4.0, 6.0]);
    }

    #[test]
    fn packed_levels_cover_every_leaf_and_end_in_one_root() {
        for n in [1, 63, 64, 65, 4096, 4097, 1_000_000] {
            let levels = packed_level_counts(n);
            assert_eq!(levels[0], n.div_ceil(GPU_PICK_BLOCK_POINTS));
            assert_eq!(levels.last(), Some(&1));
            for pair in levels.windows(2) {
                assert_eq!(pair[1], pair[0].div_ceil(2));
            }
            let nodes: u64 = levels.iter().map(|&v| u64::from(v)).sum();
            assert!(nodes >= u64::from(levels[0]));
        }
    }

    #[test]
    fn originating_leaf_owns_the_boundary_segment() {
        let n = 130u32;
        let mut starts = Vec::new();
        for first in (0..n).step_by(GPU_PICK_BLOCK_POINTS as usize) {
            let owned = GPU_PICK_BLOCK_POINTS.min(n - first);
            let segment_count = owned.min(n - 1 - first);
            starts.extend(first..first + segment_count);
        }
        assert_eq!(starts, (0..n - 1).collect::<Vec<_>>());
        assert!(starts.contains(&63));
        assert!(starts.contains(&64));
        assert!(starts.contains(&127));
    }

    #[test]
    fn independent_lane_bounds_conservatively_contain_split_linear_projection() {
        // Deliberately anti-correlated hi/lo lanes: bounding (hi + lo) alone
        // would lose the split subtraction guarantee around a large epoch.
        let values = [
            (1_700_000_000_000.0_f64 as f32, -65_535.75_f32),
            (1_700_000_000_000.0_f64 as f32, -65_535.0_f32),
            (1_700_000_131_072.0_f64 as f32, 0.125_f32),
            (1_700_000_131_072.0_f64 as f32, 0.875_f32),
        ];
        let min_hi = values.iter().map(|v| v.0).fold(f32::INFINITY, f32::min);
        let max_hi = values.iter().map(|v| v.0).fold(f32::NEG_INFINITY, f32::max);
        let min_lo = values.iter().map(|v| v.1).fold(f32::INFINITY, f32::min);
        let max_lo = values.iter().map(|v| v.1).fold(f32::NEG_INFINITY, f32::max);
        let axis_min_hi = min_hi;
        let axis_min_lo = min_lo - 1.0;
        let lower = (min_hi - axis_min_hi) + (min_lo - axis_min_lo);
        let upper = (max_hi - axis_min_hi) + (max_lo - axis_min_lo);
        for (hi, lo) in values {
            let numerator = (hi - axis_min_hi) + (lo - axis_min_lo);
            assert!(numerator >= lower && numerator <= upper);
        }
        // A negative (inverted) range reverses but does not invalidate the
        // interval after endpoint sorting.
        let inverted = [lower / -10.0, upper / -10.0];
        let inverted_lo = inverted[0].min(inverted[1]);
        let inverted_hi = inverted[0].max(inverted[1]);
        for (hi, lo) in values {
            let t = ((hi - axis_min_hi) + (lo - axis_min_lo)) / -10.0;
            assert!(t >= inverted_lo && t <= inverted_hi);
        }
    }

    #[test]
    fn style_cross_product_extent_is_conservative() {
        let scatter = GpuPickScatter {
            base_radius_px: 2.0,
            base_shape_id: 0,
            style_map: Some(GpuPickScatterStyle {
                style_index_column: None,
                style_slots: &[ScatterStyleSlotGpu {
                    color_premul: [0.0; 4],
                    meta: [10.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
                }],
                style_overrides: &[ScatterStyleOverrideGpu {
                    point_index: 7,
                    _pad: [0; 3],
                    color_premul: [0.0; 4],
                    meta: [0.0, 2.0, STYLE_MASK_SHAPE as f32, 0.0],
                }],
                style_meta: ScatterStyleMapMeta {
                    style_count: 1,
                    override_count: 1,
                    has_index: 0,
                    _pad: 0,
                },
            }),
        };
        let extent = style_max_extent("s", &scatter).unwrap();
        assert_eq!(extent, 10.0 * MAX_SHAPE_SCALE);
    }

    #[test]
    fn shader_pipelines_compile_when_a_gpu_is_available() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let engine = GpuPickEngine::new(device.clone(), queue);
        assert!(engine.is_ok());
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}
