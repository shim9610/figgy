//! P-00 GPU feasibility probe: replay the canonical arc scan with bounded
//! working buffers. This is not a streaming renderer or a portability proof.
#![cfg(not(target_arch = "wasm32"))]

use renderer::data_render::ScatterTransform;
use renderer::data_render::line_arc::create_arc_scan_pipelines;
use wgpu::util::DeviceExt;

const WG: u32 = 256;
const INPUT_POINTS: u32 = WG + 1; // one predecessor plus one complete leaf
const X_FLOATS: u32 = INPUT_POINTS * 2;
const CHECKS: [u32; 8] = [0, 255, 256, 257, 65_535, 65_536, 65_537, 65_538];
const CHUNK_N: u32 = 1030;
// Both 256- and 257-point chunk boundaries, including short last chunks.
const CHUNK_CHECKS: [u32; 22] = [
    0, 255, 256, 257, 258, 511, 512, 513, 514, 515, 767, 768, 769, 770, 771, 772, 1023, 1024, 1025,
    1027, 1028, 1029,
];
const EXTRA: &str = r#"
// Test-owned checkpoint gathering only; replay and canonical arc entries
// above are the repository's actual line_arc.wgsl entries.
@group(2) @binding(1) var<storage, read_write> gathered: array<u32>;

@compute @workgroup_size(256)
fn gather_checkpoint(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.len) { return; }
    let g = replay.global_start + i;
    let bits = bitcast<u32>(dst[params.start + i]);
    if (g == 0u) { gathered[0] = bits; }
    if (g == 255u) { gathered[1] = bits; }
    if (g == 256u) { gathered[2] = bits; }
    if (g == 257u) { gathered[3] = bits; }
    if (g == 65535u) { gathered[4] = bits; }
    if (g == 65536u) { gathered[5] = bits; }
    if (g == 65537u) { gathered[6] = bits; }
    if (g == 65538u) { gathered[7] = bits; }
}
"#;

fn test_shader() -> String {
    let mut source = format!(
        "{}\n{}",
        include_str!("../src/data_render/line_arc.wgsl"),
        EXTRA
    );
    source.push_str(
        "\n@compute @workgroup_size(256)\nfn gather_chunk_checkpoint(@builtin(global_invocation_id) gid: vec3<u32>) {\n\
         let i = gid.x; if (i >= params.len) { return; }\n\
         let g = replay.global_start + i;\n\
         let bits = bitcast<u32>(dst[params.start + i]);\n",
    );
    for (slot, index) in CHUNK_CHECKS.iter().enumerate() {
        source.push_str(&format!(
            "if (g == {index}u) {{ gathered[{slot}] = bits; }}\n"
        ));
    }
    source.push_str("}\n");
    source
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    bytes: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.max(4),
        usage,
        mapped_at_creation: false,
    })
}

fn initialized(
    device: &wgpu::Device,
    label: &str,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage,
    })
}

fn point(i: u32) -> ([f32; 2], [f32; 2]) {
    let x = 20.0 + i as f32 * 0.001 + ((i.wrapping_mul(37) % 11) as f32) * 0.0001;
    let y = if i % 193 == 37 {
        f32::NAN
    } else {
        5.0 + ((i.wrapping_mul(17) % 101) as f32) * 0.003
    };
    (
        [x, ((i.wrapping_mul(13) % 7) as f32) * 0.00001],
        [y, ((i.wrapping_mul(19) % 5) as f32) * 0.00002],
    )
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    transform_bg: wgpu::BindGroup,
    storage_bgl: wgpu::BindGroupLayout,
    replay_bgl: wgpu::BindGroupLayout,
    seg: wgpu::ComputePipeline,
    scan: wgpu::ComputePipeline,
    add: wgpu::ComputePipeline,
    apply_carry: wgpu::ComputePipeline,
    update_carry: wgpu::ComputePipeline,
    collect: wgpu::ComputePipeline,
    add_global: wgpu::ComputePipeline,
    gather: wgpu::ComputePipeline,
    chunk_gather: wgpu::ComputePipeline,
}

impl Gpu {
    fn new() -> Self {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("P-00 requires a GPU adapter; no skipped experiment");
        assert!(adapter.limits().max_compute_workgroup_size_x >= WG);
        assert!(adapter.limits().max_compute_invocations_per_workgroup >= WG);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("P-00 requires a GPU device");
        let source = test_shader();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("P-00 source plus local replay kernels"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
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
        let transform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("P-00 transform"),
            entries: &[uniform(0)],
        });
        let storage_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("P-00 original storage"),
            entries: &[
                storage(0, true),
                storage(1, false),
                storage(2, false),
                uniform(3),
                storage(4, false),
            ],
        });
        let replay_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("P-00 test orchestration"),
            entries: &[storage(0, false), storage(1, false), uniform(2)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("P-00 arc layout"),
            bind_group_layouts: &[Some(&transform_bgl), Some(&storage_bgl), Some(&replay_bgl)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
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
        let transform_buf = initialized(
            &device,
            "P-00 transform bytes",
            bytemuck::bytes_of(&transform),
            wgpu::BufferUsages::UNIFORM,
        );
        let transform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("P-00 transform bind"),
            layout: &transform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buf.as_entire_binding(),
            }],
        });
        Self {
            seg: pipeline("seg_init"),
            scan: pipeline("scan_block"),
            add: pipeline("add_offsets"),
            apply_carry: pipeline("apply_carry"),
            update_carry: pipeline("update_carry"),
            collect: pipeline("collect_leaf_total"),
            add_global: pipeline("add_global_offset"),
            gather: pipeline("gather_checkpoint"),
            chunk_gather: pipeline("gather_chunk_checkpoint"),
            device,
            queue,
            transform_bg,
            storage_bgl,
            replay_bgl,
        }
    }

    fn params(&self, len: u32, x_base: u32, y_base: u32, start: u32) -> wgpu::Buffer {
        initialized(
            &self.device,
            "P-00 original ArcParams",
            bytemuck::cast_slice(&[len, x_base, y_base, start]),
            wgpu::BufferUsages::UNIFORM,
        )
    }

    fn storage(
        &self,
        pool: &wgpu::Buffer,
        dst: &wgpu::Buffer,
        sums: &wgpu::Buffer,
        params: &wgpu::Buffer,
        carry: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("P-00 original shader bindings"),
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

    fn replay(
        &self,
        top: &wgpu::Buffer,
        gather: &wgpu::Buffer,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("P-00 replay state"),
            layout: &self.replay_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: top.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: gather.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }

    fn submit(&self, encoder: wgpu::CommandEncoder) {
        self.queue.submit([encoder.finish()]);
    }

    fn read_u32(&self, source: &wgpu::Buffer, count: usize) -> Vec<u32> {
        let readback = buffer(
            &self.device,
            "P-00 readback",
            (count * 4) as u64,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("P-00 readback"),
            });
        encoder.copy_buffer_to_buffer(source, 0, &readback, 0, (count * 4) as u64);
        self.submit(encoder);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).expect("map send"));
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(60)),
            })
            .expect("GPU poll failed");
        rx.recv_timeout(std::time::Duration::from_secs(60))
            .expect("GPU map callback missing")
            .expect("GPU map failed");
        let bytes = readback
            .slice(..)
            .get_mapped_range()
            .expect("mapped range missing");
        let words = bytes
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        drop(bytes);
        readback.unmap();
        words
    }
}

fn oracle(gpu: &Gpu, n: u32) -> Vec<u32> {
    let blocks = n.div_ceil(WG);
    let upper = blocks.div_ceil(WG);
    let mut source = vec![0.0f32; (n * 4) as usize];
    for i in 0..n {
        let (x, y) = point(i);
        source[(i * 2) as usize..(i * 2 + 2) as usize].copy_from_slice(&x);
        source[(n * 2 + i * 2) as usize..(n * 2 + i * 2 + 2) as usize].copy_from_slice(&y);
    }
    let pool = initialized(
        &gpu.device,
        "P-00 full input oracle",
        bytemuck::cast_slice(&source),
        wgpu::BufferUsages::STORAGE,
    );
    // Full-n output exists only in the resident oracle, never replay.
    let arc = buffer(
        &gpu.device,
        "P-00 full arc oracle",
        n as u64 * 4,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let sums0 = buffer(
        &gpu.device,
        "P-00 oracle sums0",
        blocks as u64 * 4,
        wgpu::BufferUsages::STORAGE,
    );
    let sums1 = buffer(
        &gpu.device,
        "P-00 oracle sums1",
        upper as u64 * 4,
        wgpu::BufferUsages::STORAGE,
    );
    let sink = buffer(
        &gpu.device,
        "P-00 oracle sink",
        4,
        wgpu::BufferUsages::STORAGE,
    );
    let carry = buffer(
        &gpu.device,
        "P-00 unused carry",
        4,
        wgpu::BufferUsages::STORAGE,
    );
    let p_arc = gpu.params(n, 0, n * 2, 0);
    let p_s0 = gpu.params(blocks, 0, 0, 0);
    let p_s1 = gpu.params(upper, 0, 0, 0);
    let bg_arc = gpu.storage(&pool, &arc, &sums0, &p_arc, &carry);
    let bg_s0 = gpu.storage(&pool, &sums0, &sums1, &p_s0, &carry);
    let bg_s1 = gpu.storage(&pool, &sums1, &sink, &p_s1, &carry);
    let dummy_gather = buffer(
        &gpu.device,
        "P-00 oracle unused gather",
        32,
        wgpu::BufferUsages::STORAGE,
    );
    let dummy_replay_params = gpu.params(0, 0, 0, 0);
    let dummy_replay = gpu.replay(&sums0, &dummy_gather, &dummy_replay_params);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("P-00 canonical oracle"),
        });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("P-00 original entry chain"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &gpu.transform_bg, &[]);
        pass.set_bind_group(1, &bg_arc, &[]);
        pass.set_bind_group(2, &dummy_replay, &[]);
        pass.set_pipeline(&gpu.seg);
        pass.dispatch_workgroups(blocks, 1, 1);
        pass.set_pipeline(&gpu.scan);
        pass.dispatch_workgroups(blocks, 1, 1);
        if blocks > 1 {
            pass.set_bind_group(1, &bg_s0, &[]);
            pass.dispatch_workgroups(upper, 1, 1);
            if upper > 1 {
                pass.set_bind_group(1, &bg_s1, &[]);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_pipeline(&gpu.add);
                pass.set_bind_group(1, &bg_s0, &[]);
                pass.dispatch_workgroups(upper, 1, 1);
            }
            pass.set_pipeline(&gpu.add);
            pass.set_bind_group(1, &bg_arc, &[]);
            pass.dispatch_workgroups(blocks, 1, 1);
        }
    }
    gpu.submit(encoder);
    // Readback is test validation only, not a renderer display path.
    gpu.read_u32(&arc, n as usize)
}

fn oracle_with_explicit_chunks(gpu: &Gpu, n: u32, cap: u32) -> (Vec<u32>, u32) {
    assert!((WG..=WG + 1).contains(&cap));
    let mut source = vec![0.0f32; (n * 4) as usize];
    for i in 0..n {
        let (x, y) = point(i);
        source[(i * 2) as usize..(i * 2 + 2) as usize].copy_from_slice(&x);
        source[(n * 2 + i * 2) as usize..(n * 2 + i * 2 + 2) as usize].copy_from_slice(&y);
    }
    let pool = initialized(
        &gpu.device,
        "P-00 cross-chunk resident input",
        bytemuck::cast_slice(&source),
        wgpu::BufferUsages::STORAGE,
    );
    // The resident oracle alone owns a full-n arc output.
    let arc = buffer(
        &gpu.device,
        "P-00 cross-chunk resident arc",
        n as u64 * 4,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let sums0 = buffer(
        &gpu.device,
        "P-00 resident chunk sums0",
        8,
        wgpu::BufferUsages::STORAGE,
    );
    let sums1 = buffer(
        &gpu.device,
        "P-00 resident chunk sums1",
        4,
        wgpu::BufferUsages::STORAGE,
    );
    let carry = buffer(
        &gpu.device,
        "P-00 resident running carry",
        4,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
    );
    gpu.queue.write_buffer(&carry, 0, &0f32.to_le_bytes());
    let dummy_gather = buffer(
        &gpu.device,
        "P-00 oracle dummy gather",
        4,
        wgpu::BufferUsages::STORAGE,
    );
    let dummy_params = gpu.params(0, 0, 0, 0);
    let dummy_replay = gpu.replay(&sums0, &dummy_gather, &dummy_params);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("P-00 resident explicit chunk oracle"),
        });
    let mut start = 0;
    while start < n {
        let len = (n - start).min(cap);
        let blocks = len.div_ceil(WG);
        let p_arc = gpu.params(len, 0, n * 2, start);
        let p_sums = gpu.params(blocks, 0, 0, 0);
        let bg_arc = gpu.storage(&pool, &arc, &sums0, &p_arc, &carry);
        let bg_sums = gpu.storage(&pool, &sums0, &sums1, &p_sums, &carry);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("P-00 exact original chunk chain"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &gpu.transform_bg, &[]);
            pass.set_bind_group(1, &bg_arc, &[]);
            pass.set_bind_group(2, &dummy_replay, &[]);
            pass.set_pipeline(&gpu.seg);
            pass.dispatch_workgroups(blocks, 1, 1);
            pass.set_pipeline(&gpu.scan);
            pass.dispatch_workgroups(blocks, 1, 1);
            if blocks > 1 {
                pass.set_bind_group(1, &bg_sums, &[]);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_pipeline(&gpu.add);
                pass.set_bind_group(1, &bg_arc, &[]);
                pass.dispatch_workgroups(blocks, 1, 1);
            }
            if start > 0 {
                pass.set_pipeline(&gpu.apply_carry);
                pass.set_bind_group(1, &bg_arc, &[]);
                pass.dispatch_workgroups(blocks, 1, 1);
            }
            if start + len < n {
                pass.set_pipeline(&gpu.update_carry);
                pass.set_bind_group(1, &bg_arc, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        start += len;
    }
    gpu.submit(encoder);
    let bits = gpu.read_u32(&arc, n as usize);
    let saved_carry = gpu.read_u32(&carry, 1)[0];
    (bits, saved_carry)
}

fn upload_leaf(gpu: &Gpu, pool: &wgpu::Buffer, global_start: u32, len: u32, io: u32) {
    let predecessor = u32::from(global_start != 0);
    let first = global_start - predecessor;
    let total = len + predecessor;
    let mut local = 0;
    while local < total {
        let batch = (total - local).min(io);
        let mut x = Vec::with_capacity((batch * 2) as usize);
        let mut y = Vec::with_capacity((batch * 2) as usize);
        for j in 0..batch {
            let (xp, yp) = point(first + local + j);
            x.extend(xp);
            y.extend(yp);
        }
        gpu.queue
            .write_buffer(pool, local as u64 * 8, bytemuck::cast_slice(&x));
        gpu.queue.write_buffer(
            pool,
            (X_FLOATS + local * 2) as u64 * 4,
            bytemuck::cast_slice(&y),
        );
        local += batch;
    }
}

fn replay(gpu: &Gpu, n: u32, io: u32) -> Vec<u32> {
    let blocks = n.div_ceil(WG);
    let upper = blocks.div_ceil(WG);
    assert!(
        blocks <= 257,
        "fixture deliberately stays within canonical 256^3 chunk"
    );
    let pool = buffer(
        &gpu.device,
        "P-00 bounded input leaf",
        INPUT_POINTS as u64 * 16,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    );
    let leaf = buffer(
        &gpu.device,
        "P-00 bounded 256-lane arc leaf",
        INPUT_POINTS as u64 * 4,
        wgpu::BufferUsages::STORAGE,
    );
    let leaf_total = buffer(
        &gpu.device,
        "P-00 leaf total",
        4,
        wgpu::BufferUsages::STORAGE,
    );
    let top = buffer(
        &gpu.device,
        "P-00 bounded canonical sums0",
        257 * 4,
        wgpu::BufferUsages::STORAGE,
    );
    let top_sums = buffer(
        &gpu.device,
        "P-00 bounded canonical sums1",
        2 * 4,
        wgpu::BufferUsages::STORAGE,
    );
    let sink = buffer(
        &gpu.device,
        "P-00 bounded top sink",
        4,
        wgpu::BufferUsages::STORAGE,
    );
    let carry = buffer(
        &gpu.device,
        "P-00 unused carry",
        4,
        wgpu::BufferUsages::STORAGE,
    );
    let gathered = initialized(
        &gpu.device,
        "P-00 bounded checkpoint gather",
        bytemuck::cast_slice(&[u32::MAX; 8]),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );

    // Pass 1: replay source into one 256-lane leaf, run the original kernels,
    // retain only its lane-255 total. No full-n arc/output buffer is allocated.
    for block in 0..blocks {
        let start = block * WG;
        let len = (n - start).min(WG);
        let local_start = u32::from(start != 0);
        upload_leaf(gpu, &pool, start, len, io);
        let p = gpu.params(len, 0, X_FLOATS, local_start);
        let rp = gpu.params(block, start, 0, 0);
        let bg = gpu.storage(&pool, &leaf, &leaf_total, &p, &carry);
        let test_bg = gpu.replay(&top, &gathered, &rp);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("P-00 leaf total replay"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("P-00 leaf total"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &gpu.transform_bg, &[]);
            pass.set_bind_group(1, &bg, &[]);
            pass.set_bind_group(2, &test_bg, &[]);
            pass.set_pipeline(&gpu.seg);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&gpu.scan);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&gpu.collect);
            pass.set_bind_group(2, &test_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        gpu.submit(encoder);
    }

    // The natural sums0 -> sums1 hierarchy, same original scan/add entries.
    if blocks > 1 {
        let p_top = gpu.params(blocks, 0, 0, 0);
        let p_upper = gpu.params(upper, 0, 0, 0);
        let bg_top = gpu.storage(&pool, &top, &top_sums, &p_top, &carry);
        let bg_upper = gpu.storage(&pool, &top_sums, &sink, &p_upper, &carry);
        let rp_top = gpu.params(0, 0, 0, 0);
        let test_bg = gpu.replay(&top, &gathered, &rp_top);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("P-00 upper original scan"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("P-00 upper levels"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &gpu.transform_bg, &[]);
            pass.set_bind_group(2, &test_bg, &[]);
            pass.set_pipeline(&gpu.scan);
            pass.set_bind_group(1, &bg_top, &[]);
            pass.dispatch_workgroups(upper, 1, 1);
            if upper > 1 {
                pass.set_bind_group(1, &bg_upper, &[]);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_pipeline(&gpu.add);
                pass.set_bind_group(1, &bg_top, &[]);
                pass.dispatch_workgroups(upper, 1, 1);
            }
        }
        gpu.submit(encoder);
    }

    // Pass 2: replay each leaf again, then add its *already scanned* sums0
    // predecessor, exactly once and after the leaf scan. Gather only requested
    // bits; scratch/input stay fixed-size for every fixture n.
    for block in 0..blocks {
        let start = block * WG;
        let len = (n - start).min(WG);
        let local_start = u32::from(start != 0);
        upload_leaf(gpu, &pool, start, len, io);
        let p = gpu.params(len, 0, X_FLOATS, local_start);
        let rp = gpu.params(block, start, 0, 0);
        let bg = gpu.storage(&pool, &leaf, &leaf_total, &p, &carry);
        let test_bg = gpu.replay(&top, &gathered, &rp);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("P-00 corrected leaf replay"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("P-00 corrected leaf"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &gpu.transform_bg, &[]);
            pass.set_bind_group(1, &bg, &[]);
            pass.set_bind_group(2, &test_bg, &[]);
            pass.set_pipeline(&gpu.seg);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&gpu.scan);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_bind_group(2, &test_bg, &[]);
            if block > 0 {
                pass.set_pipeline(&gpu.add_global);
                pass.dispatch_workgroups(1, 1, 1);
            }
            pass.set_pipeline(&gpu.gather);
            pass.dispatch_workgroups(1, 1, 1);
        }
        gpu.submit(encoder);
    }
    gpu.read_u32(&gathered, CHECKS.len())
}

fn replay_with_explicit_chunks(gpu: &Gpu, n: u32, cap: u32, io: u32) -> (Vec<u32>, u32) {
    assert!((WG..=WG + 1).contains(&cap));
    let pool = buffer(
        &gpu.device,
        "P-00 cross-chunk bounded input leaf",
        INPUT_POINTS as u64 * 16,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    );
    let leaf = buffer(
        &gpu.device,
        "P-00 cross-chunk bounded arc leaf",
        INPUT_POINTS as u64 * 4,
        wgpu::BufferUsages::STORAGE,
    );
    let leaf_total = buffer(
        &gpu.device,
        "P-00 cross-chunk leaf total",
        4,
        wgpu::BufferUsages::STORAGE,
    );
    let top = buffer(
        &gpu.device,
        "P-00 cross-chunk sums0",
        8,
        wgpu::BufferUsages::STORAGE,
    );
    let top_sink = buffer(
        &gpu.device,
        "P-00 cross-chunk sums1",
        4,
        wgpu::BufferUsages::STORAGE,
    );
    let carry = buffer(
        &gpu.device,
        "P-00 cross-chunk bounded carry",
        4,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
    );
    gpu.queue.write_buffer(&carry, 0, &0f32.to_le_bytes());
    let gathered = initialized(
        &gpu.device,
        "P-00 cross-chunk bounded checkpoint gather",
        bytemuck::cast_slice(&[u32::MAX; CHUNK_CHECKS.len()]),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );

    let mut chunk_start = 0;
    while chunk_start < n {
        let chunk_len = (n - chunk_start).min(cap);
        let blocks = chunk_len.div_ceil(WG);
        assert!(blocks <= 2);

        // Reproduce each uncorrected 256-lane block total from source replay.
        // The carry is deliberately not folded into these totals.
        for block in 0..blocks {
            let start = chunk_start + block * WG;
            let len = (chunk_len - block * WG).min(WG);
            upload_leaf(gpu, &pool, start, len, io);
            let p = gpu.params(len, 0, X_FLOATS, u32::from(start != 0));
            let rp = gpu.params(block, start, 0, 0);
            let bg = gpu.storage(&pool, &leaf, &leaf_total, &p, &carry);
            let test_bg = gpu.replay(&top, &gathered, &rp);
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("P-00 cross-chunk replay block total"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("P-00 original leaf total entries"),
                    timestamp_writes: None,
                });
                pass.set_bind_group(0, &gpu.transform_bg, &[]);
                pass.set_bind_group(1, &bg, &[]);
                pass.set_bind_group(2, &test_bg, &[]);
                pass.set_pipeline(&gpu.seg);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_pipeline(&gpu.scan);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_pipeline(&gpu.collect);
                pass.dispatch_workgroups(1, 1, 1);
            }
            gpu.submit(encoder);
        }
        if blocks > 1 {
            let p_top = gpu.params(blocks, 0, 0, 0);
            let p_replay = gpu.params(0, 0, 0, 0);
            let bg_top = gpu.storage(&pool, &top, &top_sink, &p_top, &carry);
            let test_bg = gpu.replay(&top, &gathered, &p_replay);
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("P-00 cross-chunk sums0 scan"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("P-00 original sums0 scan"),
                    timestamp_writes: None,
                });
                pass.set_bind_group(0, &gpu.transform_bg, &[]);
                pass.set_bind_group(1, &bg_top, &[]);
                pass.set_bind_group(2, &test_bg, &[]);
                pass.set_pipeline(&gpu.scan);
                pass.dispatch_workgroups(1, 1, 1);
            }
            gpu.submit(encoder);
        }

        // Complete the block prefixes in original order: local scan, block
        // offset, prior chunk carry. Save only the last valid completed point
        // with the actual update_carry entry after apply_carry has run.
        for block in 0..blocks {
            let start = chunk_start + block * WG;
            let len = (chunk_len - block * WG).min(WG);
            upload_leaf(gpu, &pool, start, len, io);
            let p = gpu.params(len, 0, X_FLOATS, u32::from(start != 0));
            let rp = gpu.params(block, start, 0, 0);
            let bg = gpu.storage(&pool, &leaf, &leaf_total, &p, &carry);
            let test_bg = gpu.replay(&top, &gathered, &rp);
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("P-00 cross-chunk corrected leaf replay"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("P-00 corrected leaf/carry order"),
                    timestamp_writes: None,
                });
                pass.set_bind_group(0, &gpu.transform_bg, &[]);
                pass.set_bind_group(1, &bg, &[]);
                pass.set_bind_group(2, &test_bg, &[]);
                pass.set_pipeline(&gpu.seg);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_pipeline(&gpu.scan);
                pass.dispatch_workgroups(1, 1, 1);
                if block > 0 {
                    pass.set_pipeline(&gpu.add_global);
                    pass.dispatch_workgroups(1, 1, 1);
                }
                if chunk_start > 0 {
                    pass.set_pipeline(&gpu.apply_carry);
                    pass.dispatch_workgroups(1, 1, 1);
                }
                pass.set_pipeline(&gpu.chunk_gather);
                pass.dispatch_workgroups(1, 1, 1);
                if block + 1 == blocks && chunk_start + chunk_len < n {
                    pass.set_pipeline(&gpu.update_carry);
                    pass.dispatch_workgroups(1, 1, 1);
                }
            }
            gpu.submit(encoder);
        }
        chunk_start += chunk_len;
    }
    (
        gpu.read_u32(&gathered, CHUNK_CHECKS.len()),
        gpu.read_u32(&carry, 1)[0],
    )
}

#[test]
fn canonical_arc_bits_survive_bounded_gpu_replay() {
    let gpu = Gpu::new();
    // 65_539 includes both sides of 65_536 and a short final leaf; 258
    // exercises the first two-leaf transition; 1 checks the zero-length arc.
    for n in [1u32, 258, 65_539] {
        let expected = oracle(&gpu, n);
        assert_eq!(expected[0], 0.0f32.to_bits());
        if n > 1 {
            assert!(
                expected.iter().any(|b| *b != 0.0f32.to_bits()),
                "nontrivial arc fixture required"
            );
        }
        for io in [1u32, 255, 257] {
            let actual = replay(&gpu, n, io);
            let mut compared = 0;
            for (slot, &index) in CHECKS.iter().enumerate() {
                if index < n {
                    assert_ne!(
                        actual[slot],
                        u32::MAX,
                        "checkpoint {index} was never gathered (n={n}, io={io})"
                    );
                    assert_eq!(
                        actual[slot], expected[index as usize],
                        "f32 arc bits differ at global {index}, n={n}, IO chunk={io}"
                    );
                    compared += 1;
                }
            }
            assert!(compared >= 1, "vacuous arc parity run");
        }
    }
}

#[test]
fn product_bounded_replay_pipelines_compile() {
    let gpu = Gpu::new();
    let _pipelines = create_arc_scan_pipelines(&gpu.device);
}

#[test]
fn canonical_cross_chunk_carry_bits_survive_bounded_gpu_replay() {
    let gpu = Gpu::new();
    for cap in [WG, WG + 1] {
        assert!(CHUNK_N / cap >= 4, "multiple carry hops required");
        assert!(CHUNK_N % cap > 0, "short final chunk required");
        let (expected, oracle_carry) = oracle_with_explicit_chunks(&gpu, CHUNK_N, cap);
        let last_completed = ((CHUNK_N - 1) / cap) * cap - 1;
        assert_eq!(
            oracle_carry, expected[last_completed as usize],
            "original update_carry did not save the last valid completed arc"
        );
        assert_ne!(oracle_carry, 0.0f32.to_bits(), "nonzero carry required");
        assert_ne!(
            expected[(cap - 1) as usize],
            expected[cap as usize],
            "fixture must advance across the first chunk boundary"
        );
        for io in [1u32, 255, 257] {
            let (actual, replay_carry) = replay_with_explicit_chunks(&gpu, CHUNK_N, cap, io);
            assert_eq!(
                replay_carry, oracle_carry,
                "saved GPU carry bits differ, cap={cap}, IO={io}"
            );
            for (slot, &index) in CHUNK_CHECKS.iter().enumerate() {
                assert_ne!(
                    actual[slot],
                    u32::MAX,
                    "chunk checkpoint {index} not gathered"
                );
                assert_eq!(
                    actual[slot], expected[index as usize],
                    "cross-chunk f32 arc bits differ at {index}, cap={cap}, IO={io}"
                );
            }
        }
    }
}
