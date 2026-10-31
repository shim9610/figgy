//! The region fill of a ternary series (`TernaryRegionStyle`): the closed
//! polygons its samples outline, filled by stencil-then-cover with the
//! even-odd rule into the panel composite's stencil attachment
//! (`surface_mesh::DEPTH_STENCIL_FORMAT`, #49, #55).
//!
//! Two pipelines over one assembled module: the stencil pass draws one
//! triangle per edge from the triangle's centre, inverting the stencil and
//! writing no colour; the cover pass draws the ternary triangle once where
//! the stencil is non-zero and resets it. The vertex stage reads the
//! series' x, y and c columns from the pool as storage (group 1), so no
//! vertex slot is bound.
//!
//! A ring's closing edge reads its first vertex's row from the ring-start
//! scan ([`scan_ring_starts`], #49): three compute entries of the same
//! module, in the arc scan's pattern, write each row's ring start once per
//! draw or pick, so no invocation walks its ring. That `u32` per row is the
//! only storage the fill adds.

/// The assembled region module.
pub const TERNARY_REGION_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/ternary_region.wgsl",
);

/// GPU twin: `ternary_region.wgsl::TernaryRegionParams`.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TernaryRegionParamsGpu {
    pub color_premul: [f32; 4],
    pub x_base: u32,
    pub y_base: u32,
    pub c_base: u32,
    pub count: u32,
}

const _: () = assert!(std::mem::size_of::<TernaryRegionParamsGpu>() == 32);

/// GPU twin: `ternary_region.wgsl::RingScanParams`.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RingScanParamsGpu {
    pub len: u32,
    pub x_base: u32,
    pub y_base: u32,
    pub c_base: u32,
}

const _: () = assert!(std::mem::size_of::<RingScanParamsGpu>() == 16);

/// Rows (or blocks) one scan workgroup covers: `RING_WG` in the shader.
pub const RING_SCAN_BLOCK: u32 = 256;

/// Group 1 of the region pipelines: the pool as storage, the params and the
/// ring starts.
pub fn create_ternary_region_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy ternary region bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
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

/// The ring-start scan's compute pipelines and their group 2 layout: the
/// pool, the level being written, the level above and the params.
pub struct RingScanPipelines {
    bgl: wgpu::BindGroupLayout,
    init: wgpu::ComputePipeline,
    scan_block: wgpu::ComputePipeline,
    apply: wgpu::ComputePipeline,
}

/// The ring-start scan's pipelines, from the region module `shader`.
pub(crate) fn create_ring_scan_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
) -> RingScanPipelines {
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
    let bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy ternary ring scan bgl"),
            entries: &[
                storage(0, true),
                storage(1, false),
                storage(2, false),
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
        },
    );
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy ternary ring scan layout"),
            bind_group_layouts: &[None, None, Some(&bgl)],
            immediate_size: 0,
        },
    );
    let pipeline = |entry: &str| {
        crate::gpu_compile::compute_pipeline(
            &device,
            &wgpu::ComputePipelineDescriptor {
                label: Some("figgy ternary ring scan pipeline"),
                layout: Some(&layout),
                module: shader,
                entry_point: Some(entry),
                compilation_options: if entry == "ring_scan_block" {
                    crate::gpu_compile::workgroup_options(&device)
                } else {
                    wgpu::PipelineCompilationOptions::default()
                },
                cache: None,
            },
        )
    };
    RingScanPipelines {
        init: pipeline("ring_init"),
        scan_block: pipeline("ring_scan_block"),
        apply: pipeline("ring_apply"),
        bgl,
    }
}

/// The workgroup grid covering `entries` in blocks of [`RING_SCAN_BLOCK`],
/// two-dimensional when one dimension's limit is too small.
fn ring_grid(entries: u32, max_per_dimension: u32) -> Option<(u32, u32)> {
    let blocks = entries.div_ceil(RING_SCAN_BLOCK).max(1);
    let x = blocks.min(max_per_dimension.max(1));
    let y = blocks.div_ceil(x);
    (y <= max_per_dimension).then_some((x, y))
}

/// The ring starts of a region over `params.count` rows: for every row, the
/// row of the first vertex of its ring (`ternary_region.wgsl`, the ring-start
/// scan). The scan is recorded and submitted here, so it runs before any
/// later submission that reads the buffer. Every level is a buffer of its
/// own: the rows, then one entry per block of 256 below, up to one entry.
/// `None` on an adapter whose dispatch grid cannot cover the rows.
pub(crate) fn scan_ring_starts(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    pipelines: &RingScanPipelines,
    pool_buffer: &wgpu::Buffer,
    params: &TernaryRegionParamsGpu,
    max_workgroups_per_dimension: u32,
) -> Option<(wgpu::Buffer, crate::gpu_memory::SharedCharge)> {
    // The row level outlives the call and is charged while it is held. The
    // levels above and the params live only until the submission completes,
    // so their tally is not held.
    let tally = crate::gpu_memory::ChargeTally::new();
    let scratch = crate::gpu_memory::ChargeTally::new();
    // The level sizes: rows, then blocks of each level below, down to one.
    let mut sizes = [0u32; 5];
    let mut levels = 1;
    sizes[0] = params.count.max(1);
    while sizes[levels - 1] > 1 {
        sizes[levels] = sizes[levels - 1].div_ceil(RING_SCAN_BLOCK);
        levels += 1;
    }
    let buffer = |len: u32, tally: &crate::gpu_memory::ChargeTally| {
        crate::gpu_memory::charged_buffer(
            tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy ternary ring starts"),
                size: u64::from(len) * 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        )
    };
    // A single row is its own level: the scan above it writes nothing.
    let levels_buf: [Option<wgpu::Buffer>; 5] = std::array::from_fn(|k| {
        (k < levels.max(2)).then(|| buffer(sizes[k].max(1), if k == 0 { &tally } else { &scratch }))
    });
    let level = |k: usize| levels_buf[k].as_ref().expect("allocated level");
    let group = |len: u32, dst: &wgpu::Buffer, sums: &wgpu::Buffer| {
        let params_buf = crate::gpu_memory::charged_buffer_init(
            &scratch,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy ternary ring scan params"),
                contents: bytemuck::bytes_of(&RingScanParamsGpu {
                    len,
                    x_base: params.x_base,
                    y_base: params.y_base,
                    c_base: params.c_base,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy ternary ring scan bg"),
            layout: &pipelines.bgl,
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
                    resource: params_buf.as_entire_binding(),
                },
            ],
        })
    };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("figgy ternary ring scan"),
    });
    let recorded;
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("figgy ternary ring scan"),
            timestamp_writes: None,
        });
        let mut dispatch = |pipeline: &wgpu::ComputePipeline, len: u32, bg: &wgpu::BindGroup| {
            let (x, y) = ring_grid(len, max_workgroups_per_dimension)?;
            pass.set_pipeline(pipeline);
            pass.set_bind_group(2, bg, &[]);
            pass.dispatch_workgroups(x, y, 1);
            Some(())
        };
        let mut record = || {
            // Each row's candidate, then each level scanned into the next.
            let rows = params.count;
            if rows > 0 {
                dispatch(&pipelines.init, rows, &group(rows, level(0), level(1)))?;
            }
            for k in 0..levels.saturating_sub(1) {
                let len = if k == 0 { rows } else { sizes[k] };
                dispatch(
                    &pipelines.scan_block,
                    len,
                    &group(len, level(k), level(k + 1)),
                )?;
            }
            // The scanned levels folded downwards, the top one complete.
            for k in (1..levels.saturating_sub(1)).rev() {
                let len = if k == 1 { rows } else { sizes[k - 1] };
                dispatch(&pipelines.apply, len, &group(len, level(k - 1), level(k)))?;
            }
            Some(())
        };
        recorded = record();
    }
    recorded?;
    queue.submit([encoder.finish()]);
    let [rows, ..] = levels_buf;
    Some((
        rows.expect("the row level"),
        crate::gpu_memory::shared_charge(
            tally,
            ledger,
            crate::gpu_memory::GpuResourceKind::ArcScan,
        ),
    ))
}

/// Which region pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum TernaryRegionPass {
    /// One triangle per edge inverting the stencil, no colour.
    Stencil,
    /// The ternary triangle where the stencil is non-zero, resetting it.
    Cover,
}

impl TernaryRegionPass {
    pub const ALL: [TernaryRegionPass; 2] = [TernaryRegionPass::Stencil, TernaryRegionPass::Cover];

    fn entries(self) -> (&'static str, &'static str) {
        match self {
            TernaryRegionPass::Stencil => ("vs_stencil", "fs_stencil"),
            TernaryRegionPass::Cover => ("vs_cover", "fs_cover"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TernaryRegionPass::Stencil => "figgy ternary region stencil pipeline",
            TernaryRegionPass::Cover => "figgy ternary region cover pipeline",
        }
    }

    /// The stencil state: the stencil pass inverts every covered sample
    /// (even-odd); the cover pass paints where the stencil is non-zero and
    /// clears it for the next series. Neither tests or writes depth.
    pub fn depth_stencil(self) -> wgpu::DepthStencilState {
        let face = match self {
            TernaryRegionPass::Stencil => wgpu::StencilFaceState {
                compare: wgpu::CompareFunction::Always,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op: wgpu::StencilOperation::Invert,
            },
            TernaryRegionPass::Cover => wgpu::StencilFaceState {
                compare: wgpu::CompareFunction::NotEqual,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op: wgpu::StencilOperation::Zero,
            },
        };
        wgpu::DepthStencilState {
            format: super::surface_mesh::DEPTH_STENCIL_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState {
                front: face,
                back: face,
                read_mask: 0xff,
                write_mask: 0xff,
            },
            bias: wgpu::DepthBiasState::default(),
        }
    }
}

/// One region pipeline over the composite's `target_format` and
/// `sample_count`.
pub(crate) fn create_ternary_region_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    region_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    pass: TernaryRegionPass,
) -> wgpu::RenderPipeline {
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy ternary region layout"),
            bind_group_layouts: &[Some(transform_bgl), Some(region_bgl)],
            immediate_size: 0,
        },
    );
    let (vertex, fragment) = pass.entries();
    let (blend, write_mask) = match pass {
        TernaryRegionPass::Stencil => (None, wgpu::ColorWrites::empty()),
        TernaryRegionPass::Cover => (
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            wgpu::ColorWrites::ALL,
        ),
    };
    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(pass.label()),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex),
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
            depth_stencil: Some(pass.depth_stencil()),
            multisample: super::multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend,
                    write_mask,
                })],
            }),
            multiview_mask: None,
            cache: None,
        },
    )
}

/// The region group of one series: the pool, its params and its ring
/// starts ([`scan_ring_starts`]), the params charged.
pub(crate) fn create_ternary_region_bind_group(
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    params: &TernaryRegionParamsGpu,
    ring_starts: &wgpu::Buffer,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    let tally = crate::gpu_memory::ChargeTally::new();
    let params_buf = crate::gpu_memory::charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy ternary region params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy ternary region bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pool_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: ring_starts.as_entire_binding(),
            },
        ],
    });
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    (bind_group, charge)
}

/// One series' region fill: its stencil and cover pipelines, the panel
/// transform, its group 1 and the vertex count to instance the edges over.
pub struct ColumnTernaryRegionLayer<'a> {
    pub stencil: &'a wgpu::RenderPipeline,
    pub cover: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub region_bg: wgpu::BindGroup,
    pub count: u32,
    pub charge: crate::gpu_memory::SharedCharge,
    /// The ring starts' charge, held while the group binds them.
    pub ring_charge: crate::gpu_memory::SharedCharge,
}

/// Draw `layer` into a pass with the stencil attachment: `count` edge
/// triangles (the closing edges included) into the stencil, then one cover.
/// The cost grows with the vertex count only: N triangles for N vertices.
pub fn issue_ternary_region_layer(
    pass: &mut wgpu::RenderPass<'_>,
    layer: &ColumnTernaryRegionLayer<'_>,
) {
    if layer.count < 3 {
        return;
    }
    pass.set_stencil_reference(0);
    pass.set_pipeline(layer.stencil);
    pass.set_bind_group(0, layer.transform_bg, &[]);
    pass.set_bind_group(1, &layer.region_bg, &[]);
    pass.draw(0..3, 0..layer.count);
    pass.set_pipeline(layer.cover);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test support: each row's ring start as the scan defines it, the row
    /// after the nearest invalid row at or before it.
    fn expected_starts(invalid: &[bool]) -> Vec<u32> {
        let mut start = 0u32;
        invalid
            .iter()
            .enumerate()
            .map(|(i, &bad)| {
                if bad {
                    start = i as u32 + 1;
                }
                start
            })
            .collect()
    }

    /// The scan over 70 000 rows (four levels: rows, 274 blocks, 2, 1) with
    /// invalid rows at and around block and level boundaries, by every kind
    /// `ternary_sample_valid` rejects, on a two-dimensional grid and on a
    /// grid one dimension wide enough: every row's start is the CPU rule's.
    #[test]
    fn the_ring_start_scan_finds_each_rows_ring_start_across_levels() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let n = 70_000usize;
        let mut invalid = vec![false; n];
        for i in [0, 3, 255, 256, 257, 511, 4_097, 65_535, 65_536, 69_999] {
            invalid[i] = true;
        }
        // A run of invalid rows, then one long ring to the end.
        for flag in &mut invalid[30_000..30_010] {
            *flag = true;
        }
        let mut lanes = vec![0f32; n * 6];
        for (i, &bad) in invalid.iter().enumerate() {
            let (x, y, c) = match (bad, i % 3) {
                (false, _) => (30.0, 30.0, 40.0),
                (true, 0) => (f32::NAN, 30.0, 40.0),
                (true, 1) => (30.0, -1.0, 40.0),
                (true, _) => (0.0, 0.0, 0.0),
            };
            lanes[i * 2] = x;
            lanes[n * 2 + i * 2] = y;
            lanes[n * 4 + i * 2] = c;
        }
        use wgpu::util::DeviceExt;
        let pool = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ring scan test pool"),
            contents: bytemuck::cast_slice(&lanes),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ring scan test"),
            source: wgpu::ShaderSource::Wgsl(TERNARY_REGION_SHADER.source.into()),
        });
        let pipelines = create_ring_scan_pipelines(&device, &module);
        let ledger = std::sync::Arc::new(crate::gpu_memory::GpuLedger::new());
        let params = TernaryRegionParamsGpu {
            color_premul: [0.0; 4],
            x_base: 0,
            y_base: (n * 2) as u32,
            c_base: (n * 4) as u32,
            count: n as u32,
        };
        let expected = expected_starts(&invalid);
        for max_per_dimension in [32, 65_535] {
            let (starts, _charge) = scan_ring_starts(
                &device,
                &queue,
                &ledger,
                &pipelines,
                &pool,
                &params,
                max_per_dimension,
            )
            .expect("the grid covers the rows");
            let bytes = (n * 4) as u64;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ring scan test readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&starts, 0, &readback, 0, bytes);
            queue.submit([encoder.finish()]);
            readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            });
            let got: Vec<u32> =
                bytemuck::cast_slice(&readback.slice(..).get_mapped_range().unwrap()).to_vec();
            for (i, (&g, &e)) in got.iter().zip(&expected).enumerate() {
                assert_eq!(
                    g, e,
                    "row {i} at {max_per_dimension} workgroups per dimension"
                );
            }
        }
        // The grid a dimension of 4 cannot cover is refused, not truncated.
        assert!(ring_grid(70_000, 4).is_none());
        assert_eq!(ring_grid(70_000, 32), Some((32, 9)));
        assert_eq!(ring_grid(0, 32), Some((1, 1)));
    }
}
