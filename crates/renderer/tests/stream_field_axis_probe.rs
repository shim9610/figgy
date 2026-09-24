//! Test-only P-00 gate: can a bounded 1x pixel-tile prepass capture the exact
//! `axis_t` that the resident field quad supplies at 1x and 4x MSAA?
//! This test changes no product shader or source data; failures are never skipped.
#![cfg(not(target_arch = "wasm32"))]

const WIDTH: u32 = 37;
const HEIGHT: u32 = 23;
const PANEL: (u32, u32, u32, u32) = (3, 2, 31, 19);
const TILES: [(u32, u32, u32, u32); 4] = [
    (3, 2, 14, 9),
    (17, 2, 17, 9),
    (3, 11, 14, 10),
    (17, 11, 17, 10),
];

const PROBE_ENTRIES: &str = r#"
fn pack_bits(bits: u32) -> vec4<f32> {
    let lanes = vec4<u32>(bits & 255u, (bits >> 8u) & 255u,
        (bits >> 16u) & 255u, (bits >> 24u) & 255u);
    return vec4<f32>(lanes) / 255.0;
}
struct AxisBits { @location(0) x: vec4<f32>, @location(1) y: vec4<f32> };
@fragment fn fs_axis_probe(in: FieldOut) -> AxisBits {
    return AxisBits(pack_bits(bitcast<u32>(in.axis_t.x)),
        pack_bits(bitcast<u32>(in.axis_t.y)));
}
@fragment fn fs_sample_fixture(in: FieldOut, @builtin(sample_index) sample: u32)
    -> AxisBits {
    return AxisBits(pack_bits(0xff00aa55u ^ ((sample + 1u) << 8u)),
        pack_bits(bitcast<u32>(in.axis_t.x)));
}
"#;

fn texture(device: &wgpu::Device, samples: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("field axis probe target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry: &str,
    samples: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: None,
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    views: (&wgpu::TextureView, &wgpu::TextureView),
    pipe: &wgpu::RenderPipeline,
    scissor: (u32, u32, u32, u32),
    clear: bool,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("field axis probe pass"),
        color_attachments: &[
            Some(wgpu::RenderPassColorAttachment {
                view: views.0,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            }),
            Some(wgpu::RenderPassColorAttachment {
                view: views.1,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            }),
        ],
        ..Default::default()
    });
    pass.set_viewport(
        PANEL.0 as f32,
        PANEL.1 as f32,
        PANEL.2 as f32,
        PANEL.3 as f32,
        0.0,
        1.0,
    );
    pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
    pass.set_pipeline(pipe);
    pass.draw(0..6, 0..1);
}

fn extract(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    sources: (&wgpu::Texture, &wgpu::Texture),
    samples: u32,
) -> wgpu::Buffer {
    let kind = if samples == 1 {
        "texture_2d<f32>"
    } else {
        "texture_multisampled_2d<f32>"
    };
    let sample = if samples == 1 { "0" } else { "i32(id.z)" };
    let code = format!(
        r#"
@group(0) @binding(0) var src_x: {kind};
@group(0) @binding(1) var src_y: {kind};
@group(0) @binding(2) var<storage, read_write> out: array<vec4<u32>>;
fn unpack_bits(value: vec4<f32>) -> u32 {{
    let b = vec4<u32>(round(value * 255.0));
    return b.x | (b.y << 8u) | (b.z << 16u) | (b.w << 24u);
}}
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x < {WIDTH}u && id.y < {HEIGHT}u && id.z < {samples}u) {{
        out[(id.z * {HEIGHT}u + id.y) * {WIDTH}u + id.x] =
            vec4<u32>(
                unpack_bits(textureLoad(src_x, vec2<i32>(id.xy), {sample})),
                unpack_bits(textureLoad(src_y, vec2<i32>(id.xy), {sample})),
                0x12345678u, 0x87654321u);
    }}
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("field axis per-sample extraction"),
        source: wgpu::ShaderSource::Wgsl(code.into()),
    });
    let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("field axis per-sample extraction"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let size = u64::from(WIDTH * HEIGHT * samples * 16);
    let storage = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("field axis sample storage"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("field axis sample readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let view_x = sources.0.create_view(&Default::default());
    let view_y = sources.1.create_view(&Default::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("field axis sample bind group"),
        layout: &pipe.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view_x),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view_y),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: storage.as_entire_binding(),
            },
        ],
    });
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipe);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), samples);
    }
    encoder.copy_buffer_to_buffer(&storage, 0, &readback, 0, size);
    readback
}

fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<[u32; 4]> {
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("field axis GPU poll failed");
    rx.recv_timeout(std::time::Duration::from_secs(30))
        .expect("field axis map callback missing")
        .expect("field axis map failed");
    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .expect("map range failed");
    let rows = mapped
        .chunks_exact(16)
        .map(|row| {
            std::array::from_fn(|lane| {
                u32::from_le_bytes(row[lane * 4..lane * 4 + 4].try_into().unwrap())
            })
        })
        .collect();
    drop(mapped);
    buffer.unmap();
    rows
}

fn at(rows: &[[u32; 4]], x: u32, y: u32, sample: u32) -> [u32; 4] {
    rows[((sample * HEIGHT + y) * WIDTH + x) as usize]
}

#[test]
fn bounded_pixel_tile_axis_t_matches_resident_field_samples() {
    let instance = renderer::data_render::create_instance();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("field axis P-00 requires a GPU adapter, not a skipped test");
    eprintln!("field axis P-00 adapter: {:?}", adapter.get_info());
    let format_features = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm);
    assert!(
        format_features.allowed_usages.contains(
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        ) && format_features.flags.sample_count_supported(1)
            && format_features.flags.sample_count_supported(4),
        "Rgba8Unorm 1x/4x sample probe unsupported: {format_features:?}"
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
        .expect("field axis P-00 GPU device creation failed");
    let source = format!(
        "{}\n{}",
        include_str!("../src/data_render/field_columnar.wgsl"),
        PROBE_ENTRIES
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("resident field shader with test-only axis entries"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let tile_pipe = pipeline(&device, &shader, "fs_axis_probe", 1);
    let candidate_x = texture(&device, 1);
    let candidate_y = texture(&device, 1);
    let candidate_views = (
        candidate_x.create_view(&Default::default()),
        candidate_y.create_view(&Default::default()),
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    for (index, tile) in TILES.into_iter().enumerate() {
        draw(
            &mut encoder,
            (&candidate_views.0, &candidate_views.1),
            &tile_pipe,
            tile,
            index == 0,
        );
    }
    let candidate_read = extract(&device, &mut encoder, (&candidate_x, &candidate_y), 1);
    let mut resident_reads = Vec::new();
    for samples in [1, 4] {
        let target_x = texture(&device, samples);
        let target_y = texture(&device, samples);
        let views = (
            target_x.create_view(&Default::default()),
            target_y.create_view(&Default::default()),
        );
        let pipe = pipeline(&device, &shader, "fs_axis_probe", samples);
        draw(&mut encoder, (&views.0, &views.1), &pipe, PANEL, true);
        resident_reads.push((
            samples,
            extract(&device, &mut encoder, (&target_x, &target_y), samples),
        ));
        if samples == 4 {
            let fixture_x = texture(&device, 4);
            let fixture_y = texture(&device, 4);
            let fixture_views = (
                fixture_x.create_view(&Default::default()),
                fixture_y.create_view(&Default::default()),
            );
            let fixture_pipe = pipeline(&device, &shader, "fs_sample_fixture", 4);
            draw(
                &mut encoder,
                (&fixture_views.0, &fixture_views.1),
                &fixture_pipe,
                PANEL,
                true,
            );
            resident_reads.push((
                0,
                extract(&device, &mut encoder, (&fixture_x, &fixture_y), 4),
            ));
        }
    }
    queue.submit([encoder.finish()]);
    let candidate = read(&device, &candidate_read);
    let x_bits: std::collections::HashSet<u32> = (PANEL.0..PANEL.0 + PANEL.2)
        .map(|x| at(&candidate, x, PANEL.1 + PANEL.3 / 2, 0)[0])
        .collect();
    let y_bits: std::collections::HashSet<u32> = (PANEL.1..PANEL.1 + PANEL.3)
        .map(|y| at(&candidate, PANEL.0 + PANEL.2 / 2, y, 0)[1])
        .collect();
    assert!(
        x_bits.len() > 4 && y_bits.len() > 4,
        "axis_t fixture failed to vary across both screen axes"
    );
    let mut compared = 0usize;
    let mut saw_tile_edge = false;
    let mut saw_diagonal = false;
    for (samples, buffer) in resident_reads {
        let resident = read(&device, &buffer);
        if samples == 0 {
            for y in PANEL.1..PANEL.1 + PANEL.3 {
                for x in PANEL.0..PANEL.0 + PANEL.2 {
                    for sample in 0..4 {
                        assert_eq!(
                            at(&resident, x, y, sample)[0],
                            0xff00aa55u32 ^ ((sample + 1) << 8),
                            "sample fixture failed at ({x},{y}) sample {sample}"
                        );
                    }
                }
            }
            continue;
        }
        for y in PANEL.1..PANEL.1 + PANEL.3 {
            for x in PANEL.0..PANEL.0 + PANEL.2 {
                let expected = at(&candidate, x, y, 0);
                for sample in 0..samples {
                    let actual = at(&resident, x, y, sample);
                    assert_eq!(
                        actual, expected,
                        "axis_t mismatch at ({x},{y}), resident {samples}x sample {sample}: \
                         resident={actual:08x?} bounded-prepass={expected:08x?}"
                    );
                    compared += 1;
                }
                saw_tile_edge |= x == 16 || x == 17 || y == 10 || y == 11;
                saw_diagonal |= x - PANEL.0 == y - PANEL.1;
            }
        }
    }
    assert_eq!(compared, (PANEL.2 * PANEL.3 * 5) as usize);
    assert!(saw_tile_edge && saw_diagonal);
    eprintln!("field axis P-00 compared {compared} resident samples vs bounded pixel tiles");
}
