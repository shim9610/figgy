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

use wgpu::util::DeviceExt;

use super::ScatterTransform;

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
        for (n, cap) in [(1u32, 1000u32), (1000, 1000), (1001, 1000), (2500, 1000), (3000, 1000)] {
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
}

/// Compute pipelines + bind group layouts, created once per `Renderer`.
pub struct ArcScanPipelines {
    transform_bgl: wgpu::BindGroupLayout,
    storage_bgl: wgpu::BindGroupLayout,
    star_args_bgl: wgpu::BindGroupLayout,
    seg_init: wgpu::ComputePipeline,
    scan_block: wgpu::ComputePipeline,
    add_offsets: wgpu::ComputePipeline,
    apply_carry: wgpu::ComputePipeline,
    update_carry: wgpu::ComputePipeline,
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
        bind_group_layouts: &[&transform_bgl, &storage_bgl],
        push_constant_ranges: &[],
    });
    // Same first two groups (compatible prefix keeps them bound), plus the
    // star args group.
    let star_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy star indirect layout"),
        bind_group_layouts: &[&transform_bgl, &storage_bgl, &star_args_bgl],
        push_constant_ranges: &[],
    });
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
        seg_init: pipeline(&layout, "seg_init"),
        scan_block: pipeline(&layout, "scan_block"),
        add_offsets: pipeline(&layout, "add_offsets"),
        apply_carry: pipeline(&layout, "apply_carry"),
        update_carry: pipeline(&layout, "update_carry"),
        star_indirect: pipeline(&star_layout, "star_indirect"),
        transform_bgl,
        storage_bgl,
        star_args_bgl,
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

/// Per-series GPU state: the arc buffer (consumed as vertex data by the line
/// pipeline), scan scratch, params, and bind groups. Rebuilt when the series'
/// length, column offsets, or pool generation change; the transform uniform
/// is rewritten on every use (it follows the live data→pixel mapping).
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
    n: u32,
    x_base: u32,
    y_base: u32,
    pool_generation: u32,
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
struct ArcParams {
    len: u32,
    x_base: u32,
    y_base: u32,
    start: u32,
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

impl ArcScratch {
    /// True when the cached state still matches the series' current layout.
    pub fn matches(&self, n: u32, x_base: u32, y_base: u32, pool_generation: u32) -> bool {
        self.n == n
            && self.x_base == x_base
            && self.y_base == y_base
            && self.pool_generation == pool_generation
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
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        device: &wgpu::Device,
        pipelines: &ArcScanPipelines,
        pool_buffer: &wgpu::Buffer,
        n: u32,
        x_base: u32,
        y_base: u32,
        pool_generation: u32,
        max_workgroups_per_dimension: u32,
        chunk_capacity_override: Option<u32>,
        star_data_bgl: Option<&wgpu::BindGroupLayout>,
    ) -> Option<Self> {
        let natural = chunk_capacity(max_workgroups_per_dimension);
        let cap = match chunk_capacity_override {
            Some(c) => u64::from(c).min(natural),
            None => natural,
        };
        let cap = u32::try_from(cap.min(u64::from(u32::MAX))).expect("min with u32::MAX");
        if cap == 0 {
            return None;
        }

        // First chunk is the widest; the shared sums scratch is sized for it.
        let len0 = n.min(cap);
        let b0_max = blocks(len0);
        let b1_max = blocks(b0_max);

        let storage_buf = |label: &str, len: u32, vertex: bool| {
            let mut usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
            if vertex {
                usage |= wgpu::BufferUsages::VERTEX;
            }
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(len.max(1)) * 4,
                usage,
                mapped_at_creation: false,
            })
        };
        let arc = Arc::new(storage_buf("figgy line arc prefix", n, true));
        let sums0 = storage_buf("figgy arc sums0", b0_max, false);
        let sums1 = storage_buf("figgy arc sums1", b1_max, false);
        // Block-sum sink for the final single-block scan of sums1.
        let sums2 = storage_buf("figgy arc sums2", 1, false);
        let carry_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("figgy arc carry"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params_buf = |label: &str, p: ArcParams| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::bytes_of(&p),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        };

        let transform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("figgy arc transform uniform"),
            size: std::mem::size_of::<ScatterTransform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg_transform = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy arc transform bg"),
            layout: &pipelines.transform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buf.as_entire_binding(),
            }],
        });

        let storage_bg = |label: &str, dst: &wgpu::Buffer, sums: &wgpu::Buffer, params: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &pipelines.storage_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: pool_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: dst.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: sums.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: carry_buf.as_entire_binding() },
                ],
            })
        };

        let mut chunks = Vec::new();
        let mut start = 0u32;
        while start < n {
            let len = (n - start).min(cap);
            let b0 = blocks(len);
            let b1 = blocks(b0);
            let p_main =
                params_buf("figgy arc params main", ArcParams { len, x_base, y_base, start });
            let p_s0 =
                params_buf("figgy arc params s0", ArcParams { len: b0, x_base: 0, y_base: 0, start: 0 });
            let p_s1 =
                params_buf("figgy arc params s1", ArcParams { len: b1, x_base: 0, y_base: 0, start: 0 });
            chunks.push(ChunkBinds {
                bg_arc: storage_bg("figgy arc bg(arc)", &arc, &sums0, &p_main),
                bg_s0: storage_bg("figgy arc bg(s0)", &sums0, &sums1, &p_s0),
                bg_s1: storage_bg("figgy arc bg(s1)", &sums1, &sums2, &p_s1),
                len,
            });
            start += len;
        }

        let star = star_data_bgl.map(|vs_bgl| {
            let indirect = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("figgy star indirect args"),
                size: 16,
                usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let kernel_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("figgy star indirect params"),
                size: std::mem::size_of::<StarIndirectParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let kernel_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy star args bg"),
                layout: &pipelines.star_args_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: indirect.as_entire_binding() },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: kernel_params_buf.as_entire_binding(),
                    },
                ],
            });
            let vs_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("figgy star vs params"),
                contents: bytemuck::bytes_of(&StarVsParams {
                    n_points: n,
                    x_base,
                    y_base,
                    _pad: 0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let vs_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy star vs bg"),
                layout: vs_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: arc.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: pool_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: vs_params.as_entire_binding() },
                ],
            });
            StarPass { indirect, vs_bg, kernel_bg, kernel_params_buf }
        });

        Some(Self {
            arc,
            transform_buf,
            carry_buf,
            bg_transform,
            chunks,
            star,
            n,
            x_base,
            y_base,
            pool_generation,
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
        if let (Some(star), Some(_)) = (self.star.as_ref(), star_pitch_px) {
            if let Some(last_chunk) = self.chunks.last() {
                pass.set_pipeline(&pipelines.star_indirect);
                pass.set_bind_group(1, &last_chunk.bg_arc, &[]);
                pass.set_bind_group(2, &star.kernel_bg, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
    }
}
