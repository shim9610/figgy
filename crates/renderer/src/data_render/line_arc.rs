//! GPU arc-length prefix scan for dashed lines (`line_arc.wgsl`).
//!
//! Produces, entirely on the GPU, the cumulative pixel arc length at every
//! polyline point — the dash phase input bound as the line pipeline's vertex
//! slots 4/5. The column pool keeps **no CPU copies** of data; this module is
//! what makes that contract hold while dashes still get exact phase.
//!
//! A single scan pass covers one *chunk* of up to
//! `min(dispatch_limit × 256, 256³)` points. Longer polylines are split into
//! sequential chunks recorded into the same encoder, linked by a one-element
//! `carry` buffer holding the running total — so `n` is bounded only by pool
//! memory, never by dispatch limits. No readback is involved at any size.
//!
//! Dispatch chain per chunk `k` (recorded into one encoder, submitted before
//! the host's render pass — queue order guarantees visibility):
//!
//! ```text
//! seg_init(arc[start..], len)                  per-point segment lengths
//! scan_block(arc[start..] → sums0, len)        256-block inclusive scans
//! if blocks(len) > 1:
//!     scan_block(sums0 → sums1, b0)
//!     if blocks(b0) > 1:
//!         scan_block(sums1 → sums2, b1)        b1 ≤ 256 ⇒ single block
//!         add_offsets(sums0 += sums1, b0)
//!     add_offsets(arc[start..] += sums0, len)
//! if k > 0:          apply_carry(arc[start..] += carry, len)
//! if k < last:       update_carry(carry = arc[start+len-1])
//! ```
//!
//! WebGPU guarantees writes from one dispatch are visible to later dispatches
//! in the same pass, which is what orders the scan chain and the carry hops.

use std::sync::Arc;

use crate::gpu_memory::{
    ChargeTally, GpuLedger, GpuResourceKind, SharedCharge, charged_buffer, charged_buffer_init,
    shared_charge,
};
use crate::init::{InitEvent, finished, observe_value, started};

use super::ScatterTransform;

const INIT_SCOPE: &str = "renderer.arc_scan";

/// Workgroup width of every kernel in `line_arc.wgsl`. Public so the
/// renderer can refuse downlevel adapters that cannot run 256-wide
/// workgroups instead of panicking at pipeline creation.
pub const WG: u32 = 256;

fn blocks(n: u32) -> u32 {
    n.div_ceil(WG)
}

/// Largest point count a single chunk's scan supports on a device with the
/// given per-dimension dispatch limit. Two constraints, both hard validation
/// errors if exceeded: the first scan dispatches `ceil(n/256)` workgroups
/// (≤ device limit), and the two-level block-sum chain needs
/// `ceil(n/256²) ≤ 256`. Larger series are handled by sequential chunks of
/// this size — this is a chunking granularity, not a capacity ceiling.
pub fn chunk_capacity(max_workgroups_per_dimension: u32) -> u64 {
    let by_dispatch = u64::from(max_workgroups_per_dimension) * u64::from(WG);
    let by_levels = u64::from(WG) * u64::from(WG) * u64::from(WG);
    by_dispatch.min(by_levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chunk granularity honors BOTH the dispatch limit and the fixed
    /// two-level scan depth — exceeding either inside one chunk would be a
    /// wgpu validation panic, so the split point must stay under both.
    #[test]
    fn chunk_capacity_respects_dispatch_and_level_limits() {
        // Spec-minimum dispatch limit: bound by dispatch count.
        assert_eq!(chunk_capacity(65_535), 65_535 * 256);
        // Huge dispatch limit: bound by the 256³ two-level scan depth.
        assert_eq!(chunk_capacity(u32::MAX), 256 * 256 * 256);
        // Degenerate adapter.
        assert_eq!(chunk_capacity(0), 0);
    }

    /// The chunk split covers every point exactly once and the per-chunk
    /// lengths never exceed the capacity that sized the shared scratch.
    #[test]
    fn chunk_layout_is_exact_and_bounded() {
        for (n, cap) in [
            (1u32, 1000u32),
            (1000, 1000),
            (1001, 1000),
            (2500, 1000),
            (3000, 1000),
        ] {
            let mut covered = 0u32;
            let mut start = 0u32;
            while start < n {
                let len = (n - start).min(cap);
                assert!(len >= 1 && len <= cap);
                covered += len;
                start += len;
            }
            assert_eq!(covered, n);
        }
    }

    #[test]
    fn replay_leaf_supply_ranges_include_only_the_global_predecessor() {
        let first = ArcReplayLeaf::in_chunk(0, 513, 0).unwrap();
        assert_eq!(
            (
                first.global_start,
                first.len,
                first.supply_start,
                first.supply_len,
                first.local_start
            ),
            (0, 256, 0, 256, 0)
        );
        let second = ArcReplayLeaf::in_chunk(0, 513, 1).unwrap();
        assert_eq!(
            (
                second.global_start,
                second.len,
                second.supply_start,
                second.supply_len,
                second.local_start
            ),
            (256, 256, 255, 257, 1)
        );
        let tail = ArcReplayLeaf::in_chunk(0, 513, 2).unwrap();
        assert_eq!(
            (
                tail.global_start,
                tail.len,
                tail.supply_start,
                tail.supply_len,
                tail.local_start
            ),
            (512, 1, 511, 2, 1)
        );
        let next_chunk = ArcReplayLeaf::in_chunk(513, 1, 0).unwrap();
        assert_eq!(
            (
                next_chunk.global_start,
                next_chunk.supply_start,
                next_chunk.local_start
            ),
            (513, 512, 1)
        );
        assert!(ArcReplayLeaf::in_chunk(0, 513, 3).is_none());
        assert!(ArcReplayLeaf::in_chunk(u32::MAX, 2, 0).is_none());
    }

    #[test]
    fn replay_chunk_preflight_checks_dispatch_depth_and_global_end() {
        assert_eq!(ArcReplayChunk::checked(0, 257, 2).unwrap().block_count(), 2);
        assert_eq!(
            ArcReplayChunk::checked(256, 257, 2)
                .unwrap()
                .leaf(1)
                .unwrap()
                .global_start,
            512
        );
        assert!(ArcReplayChunk::checked(0, 513, 2).is_none());
        assert!(ArcReplayChunk::checked(0, 1, 0).is_none());
        assert!(ArcReplayChunk::checked(u32::MAX, 1, 65_535).is_none());
        assert!(ArcReplayChunk::checked(0, 256 * 256 * 256 + 1, u32::MAX).is_none());
    }

    #[test]
    fn admission_buffer_plan_matches_build_layout() {
        let plan = buffer_plan(513, 2, None, true).unwrap();
        assert_eq!(plan.chunks, 2);
        assert_eq!(plan.first_level_blocks, 2);
        assert_eq!(plan.second_level_blocks, 1);
        assert_eq!(plan.buffer_count(), 15);

        let mut sizes = Vec::new();
        plan.visit_sizes(|bytes| sizes.push(bytes));
        assert_eq!(sizes.len(), plan.buffer_count());
        assert_eq!(sizes[0], 513 * 4);
        assert_eq!(&sizes[1..6], &[8, 4, 4, 4, 112]);
        assert_eq!(&sizes[6..12], &[16; 6]);
        assert_eq!(&sizes[12..], &[16, 8, 16]);

        assert!(buffer_plan(1, 0, None, false).is_none());
        assert!(buffer_plan(0, 65_535, None, false).is_none());
    }
}

/// Compute pipelines + bind group layouts, created once per `Renderer`.
pub struct ArcScanPipelines {
    transform_bgl: wgpu::BindGroupLayout,
    storage_bgl: wgpu::BindGroupLayout,
    replay_bgl: wgpu::BindGroupLayout,
    star_args_bgl: wgpu::BindGroupLayout,
    seg_init: wgpu::ComputePipeline,
    scan_block: wgpu::ComputePipeline,
    add_offsets: wgpu::ComputePipeline,
    apply_carry: wgpu::ComputePipeline,
    update_carry: wgpu::ComputePipeline,
    collect_leaf_total: wgpu::ComputePipeline,
    add_global_offset: wgpu::ComputePipeline,
    star_indirect: wgpu::ComputePipeline,
}

/// Candidate star slots per arc px = 1 / (this factor × structure_scale).
/// CPU twin of the star vertex shader's `cons_star_pitch` — the indirect
/// dispatch count and the VS slot mapping must agree on the pitch.
pub const STAR_SLOT_PITCH_FACTOR: f32 = 0.5;

/// Hard ceiling on candidate star slots per series — a render budget
/// backstop (≈12M quad vertices), far above any chart-scale arc, NOT a data
/// limit: the polyline itself stays unlimited-n via the chunked scan.
pub const STAR_MAX_SLOTS: u32 = 2_000_000;

pub fn create_arc_scan_pipelines(device: &wgpu::Device) -> ArcScanPipelines {
    let mut noop = |_| {};
    create_arc_scan_pipelines_observed(device, &mut noop)
}

pub fn create_arc_scan_pipelines_observed(
    device: &wgpu::Device,
    observer: &mut dyn FnMut(InitEvent),
) -> ArcScanPipelines {
    started(observer, INIT_SCOPE, "setup");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figgy line arc scan shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("line_arc.wgsl").into()),
    });

    let transform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy arc transform bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let storage_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy arc storage bgl"),
        entries: &[
            storage(0, true),  // pool (whole buffer; element bases in params)
            storage(1, false), // dst
            storage(2, false), // block sums
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            storage(4, false), // cross-chunk carry (1 element)
        ],
    });

    let replay_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy bounded arc replay bgl"),
        entries: &[
            storage(0, false), // canonical sums0 for this natural scan chunk
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    // Star indirect-args kernel: reads the scan result through the already
    // bound group(1) window (last chunk) and writes only its own group(2)
    // buffers — no aliased rebinding of the arc buffer.
    let star_args_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy star args bgl"),
        entries: &[
            storage(0, false), // DrawIndirect args (4 × u32)
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy arc scan layout"),
        bind_group_layouts: &[Some(&transform_bgl), Some(&storage_bgl)],
        immediate_size: 0,
    });
    let replay_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy bounded arc replay layout"),
        bind_group_layouts: &[Some(&transform_bgl), Some(&storage_bgl), Some(&replay_bgl)],
        immediate_size: 0,
    });
    // Same first two groups (compatible prefix keeps them bound), plus the
    // star args group.
    let star_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy star indirect layout"),
        bind_group_layouts: &[
            Some(&transform_bgl),
            Some(&storage_bgl),
            Some(&star_args_bgl),
        ],
        immediate_size: 0,
    });
    finished(observer, INIT_SCOPE, "setup");
    let pipeline = |layout: &wgpu::PipelineLayout, entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("figgy arc scan pipeline"),
            layout: Some(layout),
            module: &shader,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    };

    ArcScanPipelines {
        seg_init: observe_value(observer, INIT_SCOPE, "seg_init", || {
            pipeline(&layout, "seg_init")
        }),
        scan_block: observe_value(observer, INIT_SCOPE, "scan_block", || {
            pipeline(&layout, "scan_block")
        }),
        add_offsets: observe_value(observer, INIT_SCOPE, "add_offsets", || {
            pipeline(&layout, "add_offsets")
        }),
        apply_carry: observe_value(observer, INIT_SCOPE, "apply_carry", || {
            pipeline(&layout, "apply_carry")
        }),
        update_carry: observe_value(observer, INIT_SCOPE, "update_carry", || {
            pipeline(&layout, "update_carry")
        }),
        collect_leaf_total: observe_value(observer, INIT_SCOPE, "collect_leaf_total", || {
            pipeline(&replay_layout, "collect_leaf_total")
        }),
        add_global_offset: observe_value(observer, INIT_SCOPE, "add_global_offset", || {
            pipeline(&replay_layout, "add_global_offset")
        }),
        star_indirect: observe_value(observer, INIT_SCOPE, "star_indirect", || {
            pipeline(&star_layout, "star_indirect")
        }),
        transform_bgl,
        storage_bgl,
        replay_bgl,
        star_args_bgl,
    }
}

pub async fn create_arc_scan_pipelines_observed_async(
    device: &wgpu::Device,
    observer: &mut dyn FnMut(InitEvent),
) -> Result<ArcScanPipelines, String> {
    #[cfg(target_arch = "wasm32")]
    crate::init::prewarm_compute_entries_js(
        device,
        "line.arc.async",
        include_str!("line_arc.wgsl"),
        &[
            ("seg_init", "seg_init"),
            ("scan_block", "scan_block"),
            ("add_offsets", "add_offsets"),
            ("apply_carry", "apply_carry"),
            ("update_carry", "update_carry"),
            ("collect_leaf_total", "collect_leaf_total"),
            ("add_global_offset", "add_global_offset"),
            ("star_indirect", "star_indirect"),
        ],
        observer,
    )
    .await?;
    Ok(create_arc_scan_pipelines_observed(device, observer))
}

impl ArcScanPipelines {
    pub(crate) fn replay_transform_bind_group(
        &self,
        device: &wgpu::Device,
        transform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy bounded arc replay transform"),
            layout: &self.transform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform.as_entire_binding(),
            }],
        })
    }

    /// Caller-supplied bounded leaf/sums/carry buffers are charged and retained
    /// by the stream job, not by this shared pipeline bundle.
    pub(crate) fn replay_storage_bind_group(
        &self,
        device: &wgpu::Device,
        pool: &wgpu::Buffer,
        dst: &wgpu::Buffer,
        sums: &wgpu::Buffer,
        params: &wgpu::Buffer,
        carry: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy bounded arc replay storage"),
            layout: &self.storage_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pool.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: dst.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: sums.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: carry.as_entire_binding(),
                },
            ],
        })
    }

    pub(crate) fn replay_control_bind_group(
        &self,
        device: &wgpu::Device,
        canonical_sums0: &wgpu::Buffer,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy bounded arc replay control"),
            layout: &self.replay_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: canonical_sums0.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }

    pub(crate) fn record_replay_leaf_total<'a>(
        &'a self,
        pass: &mut wgpu::ComputePass<'a>,
        transform: &'a wgpu::BindGroup,
        storage: &'a wgpu::BindGroup,
        replay: &'a wgpu::BindGroup,
    ) {
        pass.set_bind_group(0, transform, &[]);
        pass.set_bind_group(1, storage, &[]);
        pass.set_bind_group(2, replay, &[]);
        pass.set_pipeline(&self.seg_init);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&self.scan_block);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&self.collect_leaf_total);
        pass.dispatch_workgroups(1, 1, 1);
    }

    /// Scan the canonical sums0 tree after every uncorrected leaf total has
    /// been collected. `top_storage` binds sums0→sums1, `upper_storage`
    /// sums1→sink, both with `ArcParams.start=0` and their own exact lengths.
    pub(crate) fn record_replay_top<'a>(
        &'a self,
        pass: &mut wgpu::ComputePass<'a>,
        transform: &'a wgpu::BindGroup,
        top_storage: &'a wgpu::BindGroup,
        upper_storage: &'a wgpu::BindGroup,
        chunk: ArcReplayChunk,
    ) {
        let block_count = chunk.block_count;
        if block_count <= 1 {
            return;
        }
        let upper_count = blocks(block_count);
        pass.set_bind_group(0, transform, &[]);
        pass.set_pipeline(&self.scan_block);
        pass.set_bind_group(1, top_storage, &[]);
        pass.dispatch_workgroups(upper_count, 1, 1);
        if upper_count > 1 {
            pass.set_bind_group(1, upper_storage, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.add_offsets);
            pass.set_bind_group(1, top_storage, &[]);
            pass.dispatch_workgroups(upper_count, 1, 1);
        }
    }

    pub(crate) fn record_replay_leaf_output<'a>(
        &'a self,
        pass: &mut wgpu::ComputePass<'a>,
        transform: &'a wgpu::BindGroup,
        storage: &'a wgpu::BindGroup,
        replay: &'a wgpu::BindGroup,
        leaf: ArcReplayLeaf,
        apply_chunk_carry: bool,
        save_chunk_carry: bool,
    ) {
        pass.set_bind_group(0, transform, &[]);
        pass.set_bind_group(1, storage, &[]);
        pass.set_bind_group(2, replay, &[]);
        pass.set_pipeline(&self.seg_init);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&self.scan_block);
        pass.dispatch_workgroups(1, 1, 1);
        if leaf.block > 0 {
            pass.set_pipeline(&self.add_global_offset);
            pass.dispatch_workgroups(1, 1, 1);
        }
        if apply_chunk_carry {
            pass.set_pipeline(&self.apply_carry);
            pass.dispatch_workgroups(1, 1, 1);
        }
        if save_chunk_carry {
            pass.set_pipeline(&self.update_carry);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }
}

/// One chunk's window: its bind groups carry the per-chunk params uniform
/// (len/start) alongside the shared arc/sums/carry buffers.
struct ChunkBinds {
    bg_arc: wgpu::BindGroup,
    bg_s0: wgpu::BindGroup,
    bg_s1: wgpu::BindGroup,
    len: u32,
}

/// One immutable GPU arc-scan result: the arc buffer (consumed as vertex data
/// by the line pipeline), scan scratch, params, and bind groups. A distinct
/// source/transform key gets a different `ArcScratch`; an existing result is
/// never rewritten after dispatch.
pub struct ArcScratch {
    pub arc: Arc<wgpu::Buffer>,
    transform_buf: wgpu::Buffer,
    carry_buf: wgpu::Buffer,
    // The params uniforms and sums buffers live inside the bind groups —
    // wgpu keeps bound resources alive, so only what dispatch() writes
    // (transform_buf, carry_buf) needs a named field.
    bg_transform: wgpu::BindGroup,
    chunks: Vec<ChunkBinds>,
    /// Constellation star pass state — built only for styles that draw the
    /// arc-driven star pass (`build`'s `star_data_bgl` argument).
    pub star: Option<StarPass>,
    /// Ledger charge for every buffer this scratch created. Most of them
    /// (chunk sums, params uniforms) live inside the bind groups above and
    /// have no named handle, so the charge is one lump taken at build time and
    /// credited back when the last cache or `PreparedFrame` shared owner drops.
    charge: SharedCharge,
}

/// Per-series GPU state of the constellation star pass: the DrawIndirect
/// args the scan-side kernel fills, the kernel's bind group, and the bind
/// group the star vertex shader reads (arc prefix + pool + offsets).
pub struct StarPass {
    pub indirect: wgpu::Buffer,
    pub vs_bg: wgpu::BindGroup,
    kernel_bg: wgpu::BindGroup,
    kernel_params_buf: wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ArcParams {
    pub(crate) len: u32,
    pub(crate) x_base: u32,
    pub(crate) y_base: u32,
    pub(crate) start: u32,
}

/// One 256-lane leaf of a canonical scan chunk. Its supply range includes the
/// predecessor only when the global first point is not zero. The caller owns
/// the chunk/leaf cursor and must submit each leaf before reusing its GPU work
/// buffer; an IO batch boundary does not change this layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArcReplayLeaf {
    pub(crate) block: u32,
    pub(crate) global_start: u32,
    pub(crate) len: u32,
    pub(crate) supply_start: u32,
    pub(crate) supply_len: u32,
    pub(crate) local_start: u32,
}

/// A checked natural scan chunk. The caller may narrow it below the adapter's
/// natural capacity but cannot exceed either the first dispatch or the fixed
/// 256³ hierarchy depth. IO supply chunks remain independent of this split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArcReplayChunk {
    pub(crate) start: u32,
    pub(crate) len: u32,
    block_count: u32,
}

impl ArcReplayChunk {
    pub(crate) fn checked(start: u32, len: u32, max_workgroups: u32) -> Option<Self> {
        if len == 0 || max_workgroups == 0 {
            return None;
        }
        start.checked_add(len)?;
        if u64::from(len) > chunk_capacity(max_workgroups) {
            return None;
        }
        let block_count = blocks(len);
        if block_count > max_workgroups || blocks(block_count) > WG {
            return None;
        }
        Some(Self {
            start,
            len,
            block_count,
        })
    }

    pub(crate) fn block_count(self) -> u32 {
        self.block_count
    }

    pub(crate) fn leaf(self, block: u32) -> Option<ArcReplayLeaf> {
        ArcReplayLeaf::in_chunk(self.start, self.len, block)
    }
}

impl ArcReplayLeaf {
    pub(crate) fn in_chunk(chunk_start: u32, chunk_len: u32, block: u32) -> Option<Self> {
        chunk_start.checked_add(chunk_len)?;
        let local = block.checked_mul(WG)?;
        let len = chunk_len.checked_sub(local)?.min(WG);
        if len == 0 {
            return None;
        }
        let global_start = chunk_start.checked_add(local)?;
        let predecessor = u32::from(global_start != 0);
        let supply_start = global_start.checked_sub(predecessor)?;
        let supply_len = len.checked_add(predecessor)?;
        Some(Self {
            block,
            global_start,
            len,
            supply_start,
            supply_len,
            local_start: predecessor,
        })
    }

    pub(crate) fn arc_params(self, x_base: u32, y_base: u32) -> ArcParams {
        ArcParams {
            len: self.len,
            x_base,
            y_base,
            start: self.local_start,
        }
    }

    pub(crate) fn replay_params(self) -> ArcReplayParams {
        ArcReplayParams {
            block: self.block,
            global_start: self.global_start,
            _pad0: 0,
            _pad1: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ArcReplayParams {
    pub(crate) block: u32,
    pub(crate) global_start: u32,
    pub(crate) _pad0: u32,
    pub(crate) _pad1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StarIndirectParams {
    slot_pitch_px: f32,
    max_slots: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StarVsParams {
    n_points: u32,
    x_base: u32,
    y_base: u32,
    _pad: u32,
}

/// Allocation-only twin of [`ArcScratch::build`]. Resident admission uses
/// this plan before any buffer exists, so its sizes must stay in the same
/// module as the allocator rather than being re-derived by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArcBufferPlan {
    n: u32,
    chunk_capacity: u32,
    first_level_blocks: u32,
    second_level_blocks: u32,
    chunks: u32,
    star: bool,
}

impl ArcBufferPlan {
    pub(crate) fn buffer_count(self) -> usize {
        6 + self.chunks as usize * 3 + if self.star { 3 } else { 0 }
    }

    pub(crate) fn visit_sizes(self, mut visit: impl FnMut(u64)) {
        // arc, sums0, sums1, sums2, carry, transform
        visit(u64::from(self.n.max(1)) * 4);
        visit(u64::from(self.first_level_blocks.max(1)) * 4);
        visit(u64::from(self.second_level_blocks.max(1)) * 4);
        visit(4);
        visit(4);
        visit(std::mem::size_of::<ScatterTransform>() as u64);

        // Each scan chunk owns main/sums0/sums1 ArcParams uniforms.
        for _ in 0..self.chunks {
            visit(std::mem::size_of::<ArcParams>() as u64);
            visit(std::mem::size_of::<ArcParams>() as u64);
            visit(std::mem::size_of::<ArcParams>() as u64);
        }
        if self.star {
            visit(16); // DrawIndirect args
            visit(std::mem::size_of::<StarIndirectParams>() as u64);
            visit(std::mem::size_of::<StarVsParams>() as u64);
        }
    }
}

pub(crate) fn buffer_plan(
    n: u32,
    max_workgroups_per_dimension: u32,
    chunk_capacity_override: Option<u32>,
    star: bool,
) -> Option<ArcBufferPlan> {
    let natural = chunk_capacity(max_workgroups_per_dimension);
    let cap = match chunk_capacity_override {
        Some(cap) => u64::from(cap).min(natural),
        None => natural,
    };
    let cap = u32::try_from(cap.min(u64::from(u32::MAX))).ok()?;
    if n == 0 || cap == 0 {
        return None;
    }
    let first_chunk_len = n.min(cap);
    Some(ArcBufferPlan {
        n,
        chunk_capacity: cap,
        first_level_blocks: blocks(first_chunk_len),
        second_level_blocks: blocks(blocks(first_chunk_len)),
        chunks: n.div_ceil(cap),
        star,
    })
}

impl ArcScratch {
    /// Accounting lifetime shared with prepared tokens that clone this
    /// result's GPU handles.
    pub fn charge(&self) -> SharedCharge {
        Arc::clone(&self.charge)
    }

    /// `None` only on an adapter whose dispatch limit is zero — already
    /// rejected by renderer construction, kept as a defensive guard. Any
    /// real `n` is supported: series longer than one chunk's capacity scan
    /// as sequential chunks linked by the carry buffer.
    ///
    /// `chunk_capacity_override` narrows the chunk size below the device's
    /// natural `chunk_capacity(...)` — tests use it to exercise the
    /// multi-chunk carry path with small `n`.
    /// `star_data_bgl`: pass the renderer's star-data layout to also build
    /// the constellation star pass (indirect args + the VS bind group);
    /// `None` for styles without it.
    // Prepared frames share this immutable result with the cache. A different
    // dispatch key builds different buffers; this scratch is never rewritten.
    #[cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        device: &wgpu::Device,
        ledger: &Arc<GpuLedger>,
        pipelines: &ArcScanPipelines,
        pool_buffer: &wgpu::Buffer,
        n: u32,
        x_base: u32,
        y_base: u32,
        max_workgroups_per_dimension: u32,
        chunk_capacity_override: Option<u32>,
        star_data_bgl: Option<&wgpu::BindGroupLayout>,
    ) -> Option<Self> {
        let plan = buffer_plan(
            n,
            max_workgroups_per_dimension,
            chunk_capacity_override,
            star_data_bgl.is_some(),
        )?;
        let cap = plan.chunk_capacity;

        // First chunk is the widest; the shared sums scratch is sized for it.
        let len0 = n.min(cap);
        let b0_max = blocks(len0);
        let b1_max = blocks(b0_max);
        debug_assert_eq!(plan.first_level_blocks, b0_max);
        debug_assert_eq!(plan.second_level_blocks, b1_max);

        // Every buffer below is created through `charged_buffer*`, which tallies
        // the size the device is handed. Nothing here can allocate without
        // charging, and no size is written twice.
        let tally = ChargeTally::new();
        let storage_buf = |label: &str, len: u32, vertex: bool| {
            let mut usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
            if vertex {
                usage |= wgpu::BufferUsages::VERTEX;
            }
            // gpu-alloc: ArcScan
            charged_buffer(
                &tally,
                device,
                &wgpu::BufferDescriptor {
                    label: Some(label),
                    size: u64::from(len.max(1)) * 4,
                    usage,
                    mapped_at_creation: false,
                },
            )
        };
        let arc = Arc::new(storage_buf("figgy line arc prefix", n, true));
        let sums0 = storage_buf("figgy arc sums0", b0_max, false);
        let sums1 = storage_buf("figgy arc sums1", b1_max, false);
        // Block-sum sink for the final single-block scan of sums1.
        let sums2 = storage_buf("figgy arc sums2", 1, false);
        // gpu-alloc: ArcScan
        let carry_buf = charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy arc carry"),
                size: 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        let params_buf = |label: &str, p: ArcParams| {
            // gpu-alloc: ArcScan
            charged_buffer_init(
                &tally,
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::bytes_of(&p),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            )
        };

        // gpu-alloc: ArcScan
        let transform_buf = charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy arc transform uniform"),
                size: std::mem::size_of::<ScatterTransform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        let bg_transform = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy arc transform bg"),
            layout: &pipelines.transform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buf.as_entire_binding(),
            }],
        });

        let storage_bg =
            |label: &str, dst: &wgpu::Buffer, sums: &wgpu::Buffer, params: &wgpu::Buffer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &pipelines.storage_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: pool_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: dst.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: sums.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: carry_buf.as_entire_binding(),
                        },
                    ],
                })
            };

        let mut chunks = Vec::new();
        let mut start = 0u32;
        while start < n {
            let len = (n - start).min(cap);
            let b0 = blocks(len);
            let b1 = blocks(b0);
            let p_main = params_buf(
                "figgy arc params main",
                ArcParams {
                    len,
                    x_base,
                    y_base,
                    start,
                },
            );
            let p_s0 = params_buf(
                "figgy arc params s0",
                ArcParams {
                    len: b0,
                    x_base: 0,
                    y_base: 0,
                    start: 0,
                },
            );
            let p_s1 = params_buf(
                "figgy arc params s1",
                ArcParams {
                    len: b1,
                    x_base: 0,
                    y_base: 0,
                    start: 0,
                },
            );
            chunks.push(ChunkBinds {
                bg_arc: storage_bg("figgy arc bg(arc)", &arc, &sums0, &p_main),
                bg_s0: storage_bg("figgy arc bg(s0)", &sums0, &sums1, &p_s0),
                bg_s1: storage_bg("figgy arc bg(s1)", &sums1, &sums2, &p_s1),
                len,
            });
            start += len;
        }

        let star = star_data_bgl.map(|vs_bgl| {
            // gpu-alloc: ArcScan
            let indirect = charged_buffer(
                &tally,
                device,
                &wgpu::BufferDescriptor {
                    label: Some("figgy star indirect args"),
                    size: 16,
                    usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                },
            );
            // gpu-alloc: ArcScan
            let kernel_params_buf = charged_buffer(
                &tally,
                device,
                &wgpu::BufferDescriptor {
                    label: Some("figgy star indirect params"),
                    size: std::mem::size_of::<StarIndirectParams>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            let kernel_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy star args bg"),
                layout: &pipelines.star_args_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: indirect.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: kernel_params_buf.as_entire_binding(),
                    },
                ],
            });
            // gpu-alloc: ArcScan
            let vs_params = charged_buffer_init(
                &tally,
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("figgy star vs params"),
                    contents: bytemuck::bytes_of(&StarVsParams {
                        n_points: n,
                        x_base,
                        y_base,
                        _pad: 0,
                    }),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            );
            let vs_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy star vs bg"),
                layout: vs_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: arc.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: pool_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: vs_params.as_entire_binding(),
                    },
                ],
            });
            StarPass {
                indirect,
                vs_bg,
                kernel_bg,
                kernel_params_buf,
            }
        });

        Some(Self {
            arc,
            transform_buf,
            carry_buf,
            bg_transform,
            chunks,
            star,
            charge: shared_charge(tally, ledger, GpuResourceKind::ArcScan),
        })
    }

    /// Write the current transform and record the full scan chain — every
    /// chunk in sequence, carry linking them. The caller submits the encoder;
    /// queue order makes the result visible to any later-submitted render
    /// pass that reads `self.arc` as vertex data.
    /// `star_pitch_px`: when the constellation star pass is built, the
    /// candidate-slot pitch (`STAR_SLOT_PITCH_FACTOR × structure_scale`) —
    /// the indirect-args kernel runs after the scan with it. Ignored when
    /// the scratch has no star pass.
    pub fn dispatch(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &ArcScanPipelines,
        transform: &ScatterTransform,
        star_pitch_px: Option<f32>,
    ) {
        queue.write_buffer(&self.transform_buf, 0, bytemuck::bytes_of(transform));
        if self.chunks.len() > 1 {
            // Reset the running total; write_buffer lands before this
            // encoder's commands at submit time.
            queue.write_buffer(&self.carry_buf, 0, &0f32.to_le_bytes());
        }
        if let (Some(star), Some(pitch)) = (self.star.as_ref(), star_pitch_px) {
            queue.write_buffer(
                &star.kernel_params_buf,
                0,
                bytemuck::bytes_of(&StarIndirectParams {
                    slot_pitch_px: pitch.max(1e-3),
                    max_slots: STAR_MAX_SLOTS,
                }),
            );
        }

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("figgy line arc scan"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &self.bg_transform, &[]);

        let Some(last) = self.chunks.len().checked_sub(1) else {
            return; // unreachable: build() always produces ≥1 chunk for n ≥ 2
        };
        for (k, chunk) in self.chunks.iter().enumerate() {
            let b0 = blocks(chunk.len);
            let b1 = blocks(b0);

            pass.set_pipeline(&pipelines.seg_init);
            pass.set_bind_group(1, &chunk.bg_arc, &[]);
            pass.dispatch_workgroups(b0, 1, 1);

            pass.set_pipeline(&pipelines.scan_block);
            pass.dispatch_workgroups(b0, 1, 1);

            if b0 > 1 {
                pass.set_bind_group(1, &chunk.bg_s0, &[]);
                pass.dispatch_workgroups(b1, 1, 1);

                if b1 > 1 {
                    pass.set_bind_group(1, &chunk.bg_s1, &[]);
                    pass.dispatch_workgroups(1, 1, 1);

                    pass.set_pipeline(&pipelines.add_offsets);
                    pass.set_bind_group(1, &chunk.bg_s0, &[]);
                    pass.dispatch_workgroups(b1, 1, 1);
                }

                pass.set_pipeline(&pipelines.add_offsets);
                pass.set_bind_group(1, &chunk.bg_arc, &[]);
                pass.dispatch_workgroups(b0, 1, 1);
            }

            if k > 0 {
                pass.set_pipeline(&pipelines.apply_carry);
                pass.set_bind_group(1, &chunk.bg_arc, &[]);
                pass.dispatch_workgroups(b0, 1, 1);
            }
            if k < last {
                pass.set_pipeline(&pipelines.update_carry);
                pass.set_bind_group(1, &chunk.bg_arc, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }

        // Constellation star pass: convert the completed prefix's total arc
        // into DrawIndirect args. Group(1) re-binds the LAST chunk so the
        // kernel's `dst[start+len-1]` reads the full-polyline total; the
        // kernel touches the arc buffer only through that already-tracked
        // binding (no aliased rebind).
        if let (Some(star), Some(_)) = (self.star.as_ref(), star_pitch_px)
            && let Some(last_chunk) = self.chunks.last()
        {
            pass.set_pipeline(&pipelines.star_indirect);
            pass.set_bind_group(1, &last_chunk.bg_arc, &[]);
            pass.set_bind_group(2, &star.kernel_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod replay_product_tests {
    use super::*;

    fn buffer(device: &wgpu::Device, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        charged_buffer(
            &ChargeTally::new(),
            device,
            &wgpu::BufferDescriptor {
                label: Some("arc product replay GPU test"),
                size: size.max(4),
                usage,
                mapped_at_creation: false,
            },
        )
    }

    fn init<T: bytemuck::Pod>(device: &wgpu::Device, value: &T) -> wgpu::Buffer {
        charged_buffer_init(
            &ChargeTally::new(),
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("arc product replay params"),
                contents: bytemuck::bytes_of(value),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        )
    }

    fn submit(device: &wgpu::Device, queue: &wgpu::Queue, encoder: wgpu::CommandEncoder) {
        let _ = device;
        queue.submit([encoder.finish()]);
    }

    fn read_bits(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &wgpu::Buffer,
        n: u32,
    ) -> Vec<u32> {
        let bytes = u64::from(n) * 4;
        let readback = buffer(
            device,
            bytes,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("arc replay readback"),
        });
        encoder.copy_buffer_to_buffer(source, 0, &readback, 0, bytes);
        submit(device, queue, encoder);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).expect("GPU map callback send");
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(60)),
            })
            .expect("GPU poll");
        rx.recv_timeout(std::time::Duration::from_secs(60))
            .expect("GPU map callback")
            .expect("GPU map");
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .expect("GPU mapped bytes");
        let bits = mapped
            .chunks_exact(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("u32 word")))
            .collect();
        drop(mapped);
        readback.unmap();
        bits
    }

    fn upload_leaf(
        queue: &wgpu::Queue,
        pool: &wgpu::Buffer,
        x: &[f32],
        y: &[f32],
        leaf: ArcReplayLeaf,
    ) {
        let start = leaf.supply_start as usize * 2;
        let end = start + leaf.supply_len as usize * 2;
        queue.write_buffer(pool, 0, bytemuck::cast_slice(&x[start..end]));
        queue.write_buffer(pool, 257 * 8, bytemuck::cast_slice(&y[start..end]));
    }

    #[test]
    fn product_bind_and_record_helpers_preserve_256_257_and_carry_bits() {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("arc replay requires GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("arc replay requires GPU device");
        let max_workgroups = adapter.limits().max_compute_workgroups_per_dimension;
        assert!(adapter.limits().max_compute_workgroup_size_x >= WG);
        assert!(adapter.limits().max_compute_invocations_per_workgroup >= WG);
        let pipelines = create_arc_scan_pipelines(&device);
        let transform = ScatterTransform {
            data_min: [0.0, 0.0],
            data_max: [100.0, 100.0],
            data_min_lo: [0.0; 2],
            data_max_lo: [0.0; 2],
            scale_log: [0.0; 2],
            pixel_to_ndc: [0.002, 0.003],
            data_to_panel_scale: [1.0; 2],
            data_to_panel_offset: [0.0; 2],
            style_params: [[0.0; 4]; 3],
        };
        let transform_buf = init(&device, &transform);
        let transform_bg = pipelines.replay_transform_bind_group(&device, &transform_buf);
        for (n, cap) in [
            (256u32, 256u32),
            (257, 257),
            (514, 256),
            (514, 257),
            (65_539, 65_539), // 257 sums0 leaves exercise the upper scan/add branch.
        ] {
            let mut x = vec![0.0f32; n as usize * 2];
            let mut y = vec![0.0f32; n as usize * 2];
            for i in 0..n {
                let j = i as usize * 2;
                x[j] = 20.0 + i as f32 * 0.001 + (i % 11) as f32 * 0.0001;
                x[j + 1] = (i % 7) as f32 * 0.00001;
                y[j] = if i % 193 == 37 {
                    f32::NAN
                } else {
                    5.0 + (i * 17 % 101) as f32 * 0.003
                };
                y[j + 1] = (i % 5) as f32 * 0.00002;
            }
            let mut resident_data = x.clone();
            resident_data.extend_from_slice(&y);
            let resident_pool = charged_buffer_init(
                &ChargeTally::new(),
                &device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("arc replay resident oracle pool"),
                    contents: bytemuck::cast_slice(&resident_data),
                    usage: wgpu::BufferUsages::STORAGE,
                },
            );
            let ledger = Arc::new(GpuLedger::new());
            let resident = ArcScratch::build(
                &device,
                &ledger,
                &pipelines,
                &resident_pool,
                n,
                0,
                n * 2,
                max_workgroups,
                Some(cap),
                None,
            )
            .expect("resident oracle build");
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("arc resident oracle"),
            });
            resident.dispatch(&queue, &mut encoder, &pipelines, &transform, None);
            submit(&device, &queue, encoder);
            let expected = read_bits(&device, &queue, &resident.arc, n);
            assert_eq!(expected[0], 0.0f32.to_bits());
            assert!(
                expected.iter().any(|bits| *bits != 0.0f32.to_bits()),
                "nonzero arc fixture required"
            );

            let leaf_pool = buffer(
                &device,
                257 * 16,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            let leaf_arc = buffer(
                &device,
                257 * 4,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            );
            let leaf_sums = buffer(&device, 4, wgpu::BufferUsages::STORAGE);
            let carry = buffer(
                &device,
                4,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            let output = buffer(
                &device,
                u64::from(n) * 4,
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            );
            queue.write_buffer(&carry, 0, &0f32.to_le_bytes());
            let mut chunk_start = 0u32;
            while chunk_start < n {
                let chunk_len = (n - chunk_start).min(cap);
                let chunk = ArcReplayChunk::checked(chunk_start, chunk_len, max_workgroups)
                    .expect("checked canonical chunk");
                let top = buffer(
                    &device,
                    u64::from(chunk.block_count()) * 4,
                    wgpu::BufferUsages::STORAGE,
                );
                let top_sums = buffer(
                    &device,
                    u64::from(blocks(chunk.block_count())) * 4,
                    wgpu::BufferUsages::STORAGE,
                );
                let sink = buffer(&device, 4, wgpu::BufferUsages::STORAGE);
                for block in 0..chunk.block_count() {
                    let leaf = chunk.leaf(block).expect("checked leaf");
                    upload_leaf(&queue, &leaf_pool, &x, &y, leaf);
                    let arc_params = init(&device, &leaf.arc_params(0, 257 * 2));
                    let replay_params = init(&device, &leaf.replay_params());
                    let storage = pipelines.replay_storage_bind_group(
                        &device,
                        &leaf_pool,
                        &leaf_arc,
                        &leaf_sums,
                        &arc_params,
                        &carry,
                    );
                    let control =
                        pipelines.replay_control_bind_group(&device, &top, &replay_params);
                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("arc product collect leaf"),
                        });
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("arc product collect"),
                            timestamp_writes: None,
                        });
                        pipelines.record_replay_leaf_total(
                            &mut pass,
                            &transform_bg,
                            &storage,
                            &control,
                        );
                    }
                    submit(&device, &queue, encoder);
                }
                if chunk.block_count() > 1 {
                    let top_params = init(
                        &device,
                        &ArcParams {
                            len: chunk.block_count(),
                            x_base: 0,
                            y_base: 0,
                            start: 0,
                        },
                    );
                    let upper_params = init(
                        &device,
                        &ArcParams {
                            len: blocks(chunk.block_count()),
                            x_base: 0,
                            y_base: 0,
                            start: 0,
                        },
                    );
                    let top_storage = pipelines.replay_storage_bind_group(
                        &device,
                        &leaf_pool,
                        &top,
                        &top_sums,
                        &top_params,
                        &carry,
                    );
                    let upper_storage = pipelines.replay_storage_bind_group(
                        &device,
                        &leaf_pool,
                        &top_sums,
                        &sink,
                        &upper_params,
                        &carry,
                    );
                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("arc product canonical top"),
                        });
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("arc product top"),
                            timestamp_writes: None,
                        });
                        pipelines.record_replay_top(
                            &mut pass,
                            &transform_bg,
                            &top_storage,
                            &upper_storage,
                            chunk,
                        );
                    }
                    submit(&device, &queue, encoder);
                }
                for block in 0..chunk.block_count() {
                    let leaf = chunk.leaf(block).expect("checked replay leaf");
                    upload_leaf(&queue, &leaf_pool, &x, &y, leaf);
                    let arc_params = init(&device, &leaf.arc_params(0, 257 * 2));
                    let replay_params = init(&device, &leaf.replay_params());
                    let storage = pipelines.replay_storage_bind_group(
                        &device,
                        &leaf_pool,
                        &leaf_arc,
                        &leaf_sums,
                        &arc_params,
                        &carry,
                    );
                    let control =
                        pipelines.replay_control_bind_group(&device, &top, &replay_params);
                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("arc product replay leaf"),
                        });
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("arc product replay"),
                            timestamp_writes: None,
                        });
                        pipelines.record_replay_leaf_output(
                            &mut pass,
                            &transform_bg,
                            &storage,
                            &control,
                            leaf,
                            chunk_start != 0,
                            block + 1 == chunk.block_count() && chunk_start + chunk_len < n,
                        );
                    }
                    encoder.copy_buffer_to_buffer(
                        &leaf_arc,
                        u64::from(leaf.local_start) * 4,
                        &output,
                        u64::from(leaf.global_start) * 4,
                        u64::from(leaf.len) * 4,
                    );
                    submit(&device, &queue, encoder);
                }
                chunk_start += chunk_len;
            }
            let actual = read_bits(&device, &queue, &output, n);
            assert_eq!(
                actual, expected,
                "product bind/record helpers changed arc bits, n={n}, canonical cap={cap}"
            );
            if n > cap {
                assert_ne!(
                    actual[(cap - 1) as usize],
                    actual[cap as usize],
                    "carry fixture must cross a nonzero boundary"
                );
            }
        }
    }
}
