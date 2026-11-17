//! Compact single-lane upload (#317): staging sizes and the GPU expansion of
//! a compact column into the pool's `(hi, lo)` layout.
//!
//! A single-lane column (a [`ColumnLanes::Single`](crate::ColumnLanes)
//! source, or index lanes) is staged at 4 bytes per value. The upload copies
//! those bytes into one transient storage buffer per upload and dispatches
//! `column_expand.wgsl` once per column, which writes `(value, 0)` into the
//! column's pool region. Pair columns copy staging → pool as before. The
//! pool layout and every shader that reads it are unchanged.
//!
//! The expansion pipeline is created on the first compact upload and cached
//! on the pool, as the renderer creates its arc-scan pipelines on the first
//! dashed line that needs them. A device that cannot run it
//! ([`ExpandLimits::of`] is `None`) or a column it cannot bind in one window
//! ([`ExpandLimits::compacts`] is `false`) stages pairs, the existing path.

use super::*;

/// The workgroup width of `column_expand.wgsl`.
const WG: u32 = 64;

/// The label of the expansion compute pipeline.
pub(crate) const COLUMN_EXPAND_LABEL: &str = "figgy column expand pipeline";

/// The expansion module (`column_expand.wgsl`).
pub(crate) const COLUMN_EXPAND_SHADER: crate::shader_assembly::Assembled =
    crate::shader_assembly::assembled!("data_render/column_expand.wgsl");

#[cfg(test)]
thread_local! {
    /// Test hook: the workgroups per grid row, so a test can expand a column
    /// over several rows of a 2D grid without a column of millions.
    pub(super) static FORCED_GRID_X: std::cell::Cell<Option<u32>> =
        const { std::cell::Cell::new(None) };
}

/// What the device allows the expansion, read at the upload.
#[derive(Clone, Copy, Debug)]
pub(super) struct ExpandLimits {
    max_dim: u32,
    max_binding: u64,
}

impl ExpandLimits {
    /// `None` when the device cannot run the expansion: no compute, a
    /// workgroup narrower than 64, or a storage offset alignment that the
    /// pool's [`ALIGN`]-aligned regions do not meet. Such a device stages
    /// pairs.
    pub(super) fn of(device: &Device) -> Option<Self> {
        let limits = device.limits();
        let alignment = u64::from(limits.min_storage_buffer_offset_alignment);
        let runs = limits.max_compute_workgroups_per_dimension > 0
            && limits.max_compute_invocations_per_workgroup >= WG
            && limits.max_compute_workgroup_size_x >= WG
            && limits.max_storage_buffers_per_shader_stage >= 2
            && alignment > 0
            && ALIGN % alignment == 0;
        runs.then_some(Self {
            max_dim: limits.max_compute_workgroups_per_dimension,
            max_binding: limits.max_storage_buffer_binding_size,
        })
    }

    /// Limits for a pure test of the staging plan.
    #[cfg(test)]
    pub(super) fn for_test(max_dim: u32, max_binding: u64) -> Self {
        Self {
            max_dim,
            max_binding,
        }
    }

    /// Whether a single-lane column of `len_values` values is staged
    /// compact: its pool region fits one storage binding window and its
    /// workgroups fit one 2D grid. Otherwise it stages pairs. There is no
    /// cap: a column the expansion cannot take uploads the existing way.
    pub(super) fn compacts(self, len_values: usize) -> bool {
        let Some(region) = column_region_bytes(len_values) else {
            return false;
        };
        let dim = u64::from(self.max_dim);
        region <= self.max_binding && groups(region) <= dim.saturating_mul(dim)
    }
}

/// Whether a column is staged compact, and whether its source is held to the
/// single-lane contract, decided once per column at the upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StagedLanes {
    /// Staged at 4 bytes per value and expanded on the GPU.
    pub(super) compact: bool,
    /// The source declared [`ColumnLanes::Single`](crate::ColumnLanes): a
    /// non-zero second lane fails the upload, compact or not.
    pub(super) single: bool,
}

impl StagedLanes {
    /// The staging of a column of `len_values` values whose source declares
    /// `lanes`; `index` for index lanes, which are always single.
    pub(super) fn of(
        limits: Option<ExpandLimits>,
        lanes: crate::ColumnLanes,
        index: bool,
        len_values: usize,
    ) -> Self {
        let single = lanes == crate::ColumnLanes::Single && !index;
        let compact = (single || index) && limits.is_some_and(|limits| limits.compacts(len_values));
        Self { compact, single }
    }

    /// Bytes the source writes into staging: 4 per value compact, 8 as pairs.
    pub(super) fn raw_bytes(self, len_values: usize) -> u64 {
        if self.compact {
            (len_values as u64).saturating_mul(crate::data::COLUMN_SINGLE_LANE_BYTES as u64)
        } else {
            column_raw_bytes(len_values)
        }
    }

    /// Bytes the column takes in staging for a pool region of
    /// `region_bytes`: half the region compact (the region's padding
    /// included, as zeros), the region as pairs.
    pub(super) fn staged_bytes(self, region_bytes: u64) -> u64 {
        if self.compact {
            region_bytes / 2
        } else {
            region_bytes
        }
    }

    /// Bytes the column takes in the upload's transient storage buffer: its
    /// compact half region, aligned so the next column's window starts at a
    /// storage offset. Zero for a pair column.
    pub(super) fn transient_bytes(self, region_bytes: u64) -> u64 {
        if self.compact {
            align_up(region_bytes / 2, ALIGN)
        } else {
            0
        }
    }
}

/// Workgroups a region of `region_bytes` needs: one invocation per value.
fn groups(region_bytes: u64) -> u64 {
    (region_bytes / COLUMN_VALUE_BYTES as u64).div_ceil(u64::from(WG))
}

/// The upload's transient storage buffer: compact staging is copied into it
/// and the expansion reads it. Charged to the ledger like staging.
pub(super) fn create_transient(
    device: &Device,
    ledger: &Arc<GpuLedger>,
    bytes: u64,
) -> Result<TrackedBuffer, AllocError> {
    create_buffer_checked(
        device,
        ledger,
        &BufferDescriptor {
            label: Some("figgy column expand source"),
            size: bytes,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        },
        "column expand source buffer",
    )
}

/// The expansion pipeline, cached on the pool once created. Clones share
/// the pipeline.
#[derive(Clone)]
pub(crate) struct ColumnExpand {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl ColumnExpand {
    pub(crate) fn new(device: &Device) -> Result<Self, AllocError> {
        crate::renderer::gpu_checked(device, "column expand pipeline", || {
            let module = crate::gpu_compile::shader_module(
                device,
                wgpu::ShaderModuleDescriptor {
                    label: Some("figgy column expand shader"),
                    source: wgpu::ShaderSource::Wgsl(COLUMN_EXPAND_SHADER.source.into()),
                },
            );
            let pipeline = crate::gpu_compile::compute_pipeline(
                device,
                &wgpu::ComputePipelineDescriptor {
                    label: Some(COLUMN_EXPAND_LABEL),
                    layout: None,
                    module: &module,
                    entry_point: Some("expand_single"),
                    compilation_options: Default::default(),
                    cache: None,
                },
            );
            let layout = pipeline.get_bind_group_layout(0);
            Self { pipeline, layout }
        })
        .map_err(|error| pool_gpu_error("column expand pipeline", error))
    }

    /// The cached pipeline, created on the first call.
    pub(super) fn ensure<'a>(
        cache: &'a mut Option<Self>,
        device: &Device,
    ) -> Result<&'a Self, AllocError> {
        if cache.is_none() {
            *cache = Some(Self::new(device)?);
        }
        cache.as_ref().ok_or(AllocError::Internal {
            invariant: "the column expand pipeline was just created",
        })
    }

    /// Expand one compact column: `src[src_offset..]` holds its
    /// `region_bytes / 2` compact bytes, and `dst[dst_offset..]` is its pool
    /// region of `region_bytes`. Both windows start at the column, so the
    /// region starts at lane 0; `dst_offset` is [`ALIGN`]-aligned like every
    /// pool region and `src_offset` like every transient slot, which meets
    /// the device's storage offset alignment ([`ExpandLimits::of`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch(
        &self,
        device: &Device,
        pass: &mut wgpu::ComputePass<'_>,
        limits: ExpandLimits,
        src: &Buffer,
        src_offset: u64,
        dst: &Buffer,
        dst_offset: u64,
        region_bytes: u64,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy column expand"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: window(src, src_offset, region_bytes / 2),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: window(dst, dst_offset, region_bytes),
                },
            ],
        });
        let (grid_x, grid_y) = grid(groups(region_bytes), limits.max_dim);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(grid_x, grid_y, 1);
    }
}

/// A storage binding of `size` bytes of `buffer` from `offset`.
fn window(buffer: &Buffer, offset: u64, size: u64) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset,
        size: std::num::NonZeroU64::new(size),
    })
}

/// The 2D grid for `groups` workgroups: rows of at most `max_dim`
/// workgroups (or the test hook's width), as many rows as needed.
/// [`ExpandLimits::compacts`] keeps the rows within `max_dim` as well.
fn grid(groups: u64, max_dim: u32) -> (u32, u32) {
    #[cfg(test)]
    let max_dim = FORCED_GRID_X
        .with(std::cell::Cell::get)
        .map_or(max_dim, |forced| forced.clamp(1, max_dim));
    let grid_x = groups.clamp(1, u64::from(max_dim));
    let grid_y = groups.div_ceil(grid_x).max(1);
    (
        u32::try_from(grid_x).unwrap_or(u32::MAX),
        u32::try_from(grid_y).unwrap_or(u32::MAX),
    )
}
