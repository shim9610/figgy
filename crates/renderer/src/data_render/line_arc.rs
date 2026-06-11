//! GPU arc-length prefix scan for dashed lines (`line_arc.wgsl`).
//!
//! Produces, entirely on the GPU, the cumulative pixel arc length at every
//! polyline point — the dash phase input bound as the line pipeline's vertex
//! slots 4/5. The column pool keeps **no CPU copies** of data; this module is
//! what makes that contract hold while dashes still get exact phase.
//!
//! Dispatch chain per dashed series (recorded into one encoder, submitted
//! before the host's render pass — queue order guarantees visibility):
//!
//! ```text
//! seg_init(arc, n)                       per-point segment lengths
//! scan_block(arc → sums0, n)             256-block inclusive scans
//! if blocks(n) > 1:
//!     scan_block(sums0 → sums1, b0)
//!     if blocks(b0) > 1:
//!         scan_block(sums1 → sums2, b1)  b1 ≤ 256 ⇒ single block
//!         add_offsets(sums0 += sums1, b0)
//!     add_offsets(arc += sums0, n)
//! ```
//!
//! Supports `n ≤ 256³` (≈16.7M points) — far above the pool's capacity.

use std::sync::Arc;

use wgpu::util::DeviceExt;

use super::ScatterTransform;

const WG: u32 = 256;

fn blocks(n: u32) -> u32 {
    n.div_ceil(WG)
}

/// Compute pipelines + bind group layouts, created once per `Renderer`.
pub struct ArcScanPipelines {
    transform_bgl: wgpu::BindGroupLayout,
    storage_bgl: wgpu::BindGroupLayout,
    seg_init: wgpu::ComputePipeline,
    scan_block: wgpu::ComputePipeline,
    add_offsets: wgpu::ComputePipeline,
}

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
            storage(1, false), // target
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
        ],
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy arc scan layout"),
        bind_group_layouts: &[&transform_bgl, &storage_bgl],
        push_constant_ranges: &[],
    });
    let pipeline = |entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("figgy arc scan pipeline"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    };

    ArcScanPipelines {
        transform_bgl,
        storage_bgl,
        seg_init: pipeline("seg_init"),
        scan_block: pipeline("scan_block"),
        add_offsets: pipeline("add_offsets"),
    }
}

/// Per-series GPU state: the arc buffer (consumed as vertex data by the line
/// pipeline), scan scratch, params, and bind groups. Rebuilt when the series'
/// length, column offsets, or pool generation change; the transform uniform
/// is rewritten on every use (it follows the live data→pixel mapping).
pub struct ArcScratch {
    pub arc: Arc<wgpu::Buffer>,
    transform_buf: wgpu::Buffer,
    // The params uniforms and sums buffers live inside the bind groups —
    // wgpu keeps bound resources alive, so only what dispatch() writes
    // (transform_buf) needs a named field.
    bg_transform: wgpu::BindGroup,
    bg_arc: wgpu::BindGroup,
    bg_s0: wgpu::BindGroup,
    bg_s1: wgpu::BindGroup,
    n: u32,
    x_base: u32,
    y_base: u32,
    pool_generation: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ArcParams {
    len: u32,
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

    pub fn build(
        device: &wgpu::Device,
        pipelines: &ArcScanPipelines,
        pool_buffer: &wgpu::Buffer,
        n: u32,
        x_base: u32,
        y_base: u32,
        pool_generation: u32,
    ) -> Self {
        let b0 = blocks(n);
        let b1 = blocks(b0);
        assert!(b1 <= WG, "arc scan supports up to {} points", WG * WG * WG);

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
        let sums0 = storage_buf("figgy arc sums0", b0, false);
        let sums1 = storage_buf("figgy arc sums1", b1, false);
        // Block-sum sink for the final single-block scan of sums1.
        let sums2 = storage_buf("figgy arc sums2", 1, false);

        let params_buf = |label: &str, p: ArcParams| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::bytes_of(&p),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        };
        let p_main = params_buf("figgy arc params main", ArcParams { len: n, x_base, y_base, _pad: 0 });
        let p_s0 = params_buf("figgy arc params s0", ArcParams { len: b0, x_base: 0, y_base: 0, _pad: 0 });
        let p_s1 = params_buf("figgy arc params s1", ArcParams { len: b1, x_base: 0, y_base: 0, _pad: 0 });

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

        let storage_bg = |label: &str, target: &wgpu::Buffer, sums: &wgpu::Buffer, params: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &pipelines.storage_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: pool_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: target.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: sums.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: params.as_entire_binding() },
                ],
            })
        };
        let bg_arc = storage_bg("figgy arc bg(arc)", &arc, &sums0, &p_main);
        let bg_s0 = storage_bg("figgy arc bg(s0)", &sums0, &sums1, &p_s0);
        let bg_s1 = storage_bg("figgy arc bg(s1)", &sums1, &sums2, &p_s1);

        Self {
            arc,
            transform_buf,
            bg_transform,
            bg_arc,
            bg_s0,
            bg_s1,
            n,
            x_base,
            y_base,
            pool_generation,
        }
    }

    /// Write the current transform and record the full scan chain. The caller
    /// submits the encoder; queue order makes the result visible to any
    /// later-submitted render pass that reads `self.arc` as vertex data.
    pub fn dispatch(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &ArcScanPipelines,
        transform: &ScatterTransform,
    ) {
        queue.write_buffer(&self.transform_buf, 0, bytemuck::bytes_of(transform));

        let n = self.n;
        let b0 = blocks(n);
        let b1 = blocks(b0);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("figgy line arc scan"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &self.bg_transform, &[]);

        pass.set_pipeline(&pipelines.seg_init);
        pass.set_bind_group(1, &self.bg_arc, &[]);
        pass.dispatch_workgroups(b0, 1, 1);

        pass.set_pipeline(&pipelines.scan_block);
        pass.dispatch_workgroups(b0, 1, 1);

        if b0 > 1 {
            pass.set_bind_group(1, &self.bg_s0, &[]);
            pass.dispatch_workgroups(b1, 1, 1);

            if b1 > 1 {
                pass.set_bind_group(1, &self.bg_s1, &[]);
                pass.dispatch_workgroups(1, 1, 1);

                pass.set_pipeline(&pipelines.add_offsets);
                pass.set_bind_group(1, &self.bg_s0, &[]);
                pass.dispatch_workgroups(b1, 1, 1);
            }

            pass.set_pipeline(&pipelines.add_offsets);
            pass.set_bind_group(1, &self.bg_arc, &[]);
            pass.dispatch_workgroups(b0, 1, 1);
        }
    }
}
