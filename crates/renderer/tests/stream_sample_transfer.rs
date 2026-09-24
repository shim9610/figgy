//! P00 feasibility probe, not a production implementation or a portability proof.
//! Run with `cargo test -p renderer --test stream_sample_transfer -- --nocapture`.
//! No adapter/device/map failures are converted into successful skipped tests.
#![cfg(not(target_arch = "wasm32"))]

const WIDTH: u32 = 256;
const HEIGHT: u32 = 16;

// Independent probe shader: no SHADER_COMMON.md definitions are used.
const DRAW: &str = r#"
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}
fn decode(v: vec3<f32>) -> vec3<f32> {
    return select(pow((v + 0.055) / 1.055, vec3(2.4)), v / 12.92, v <= vec3(0.04045));
}
@fragment fn prefix(@builtin(position) p: vec4<f32>, SAMPLE_ARG) -> @location(0) vec4<f32> {
    let x = u32(p.x);
    let y = u32(p.y);
    // Odd multipliers permute all 256 codes in every channel for every row/sample.
    let codes = vec4<u32>((x + s * 61u + y * 17u) % 256u,
        (x * 37u + s * 43u + y * 11u) % 256u,
        (x * 73u + s * 29u + y * 7u) % 256u,
        (x * 19u + s * 53u + y * 31u) % 256u);
    var v = vec4<f32>(codes) / 255.0;
    if (IS_SRGB) { v = vec4(decode(v.rgb), v.a); }
    // First half stresses all representable codes, including non-premultiplied
    // values; second half also exercises physically valid premultiplied content.
    if (y >= 8u) { v = vec4(v.rgb * v.a, v.a); }
    return v;
}
@fragment fn suffix(@builtin(position) p: vec4<f32>, SAMPLE_ARG) -> @location(0) vec4<f32> {
    let x = u32(p.x);
    let y = u32(p.y);
    // Partial sample coverage plus varying, strictly non-opaque alpha.
    if ((x + y + s) % 5u == 0u) { discard; }
    let a = f32(1u + (x * 13u + y * 7u + s * 23u) % 253u) / 255.0;
    let rgb = vec3(f32((x * 3u + s * 17u) % 256u),
        f32((y * 41u + x) % 256u), f32((x * 7u + s * 31u) % 256u)) / 255.0;
    return vec4(rgb * a, a);
}
"#;

fn shader_source(samples: u32, srgb: bool) -> String {
    let source = DRAW.replace("IS_SRGB", if srgb { "true" } else { "false" });
    if samples > 1 {
        source.replace("SAMPLE_ARG", "@builtin(sample_index) s: u32")
    } else {
        source
            .replace(", SAMPLE_ARG", "")
            .replace("let x = u32(p.x);", "let s = 0u; let x = u32(p.x);")
    }
}

fn texture(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("P00 same-format attachment"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | if samples == 1 {
                wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::empty()
            },
        view_formats: &[],
    })
}

fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    entry: &str,
    format: wgpu::TextureFormat,
    samples: u32,
    blend: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: None,
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs"),
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
            module: shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    load: bool,
    draws: &[(&wgpu::RenderPipeline, Option<&wgpu::BindGroup>)],
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("P00 draw"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: resolve,
            ops: wgpu::Operations {
                load: if load {
                    wgpu::LoadOp::Load
                } else {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    for (pipeline, bind) in draws {
        pass.set_pipeline(pipeline);
        if let Some(bind) = bind {
            pass.set_bind_group(0, *bind, &[]);
        }
        pass.draw(0..3, 0..1);
    }
}

fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("GPU poll failed");
    rx.recv_timeout(std::time::Duration::from_secs(30))
        .expect("GPU map callback missing")
        .expect("GPU map failed");
    let bytes = buffer
        .slice(..)
        .get_mapped_range()
        .expect("mapped range failed")
        .to_vec();
    buffer.unmap();
    bytes
}

fn buffer(device: &wgpu::Device, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("P00 readback"),
        size,
        usage,
        mapped_at_creation: false,
    })
}

// Extract each sample as raw f32 bits directly into storage, without a resolve
// or a second sRGB encode. This detects differences hidden by averaging.
fn extract(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    samples: u32,
) -> wgpu::Buffer {
    let ty = if samples > 1 {
        "texture_multisampled_2d<f32>"
    } else {
        "texture_2d<f32>"
    };
    let index = if samples > 1 { "i32(id.z)" } else { "0" };
    let source = format!(
        r#"
@group(0) @binding(0) var src: {ty};
@group(0) @binding(1) var<storage, read_write> out: array<vec4<u32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x < 256u && id.y < 16u && id.z < {samples}u) {{
        out[(id.z * 16u + id.y) * 256u + id.x] = bitcast<vec4<u32>>(textureLoad(src, vec2<i32>(id.xy), {index}));
    }}
}}"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("P00 sample extraction"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("P00 extraction"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let size = u64::from(WIDTH * HEIGHT * samples * 16);
    let storage = buffer(
        device,
        size,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let readback = buffer(
        device,
        size,
        wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    );
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("P00 extraction"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: storage.as_entire_binding(),
            },
        ],
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(WIDTH / 64, HEIGHT, samples);
    }
    encoder.copy_buffer_to_buffer(&storage, 0, &readback, 0, size);
    readback
}

fn resolved_bytes(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
) -> wgpu::Buffer {
    let readback = buffer(
        device,
        u64::from(WIDTH * HEIGHT * 4),
        wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    );
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        texture.size(),
    );
    readback
}

fn compare(
    device: &wgpu::Device,
    a: &wgpu::Buffer,
    b: &wgpu::Buffer,
    label: &str,
    failures: &mut Vec<String>,
) {
    let a = read(device, a);
    let b = read(device, b);
    let mismatches = a.iter().zip(&b).filter(|(a, b)| a != b).count();
    if mismatches != 0 {
        let first = a.iter().zip(&b).position(|(a, b)| a != b).unwrap();
        failures.push(format!(
            "{label}: {mismatches}/{} byte mismatches; first offset {first}: {} != {}",
            a.len(),
            a[first],
            b[first]
        ));
    }
    eprintln!(
        "{label}: compared {} bytes, {mismatches} mismatches",
        a.len()
    );
}

fn verify_fixture(
    device: &wgpu::Device,
    before: &wgpu::Buffer,
    after_suffix: &wgpu::Buffer,
    samples: u32,
    label: &str,
) {
    let prefix = read(device, before);
    let suffix = read(device, after_suffix);
    assert_ne!(prefix, suffix, "{label}: suffix must affect the image");
    for sample in 0..samples as usize {
        for row in 0..8 {
            for channel in 0..4 {
                let values: std::collections::HashSet<[u8; 4]> = (0..WIDTH as usize)
                    .map(|x| {
                        let offset = ((sample * HEIGHT as usize + row) * WIDTH as usize + x) * 16
                            + channel * 4;
                        prefix[offset..offset + 4].try_into().unwrap()
                    })
                    .collect();
                // An 8-bit channel has only 256 values: 256 distinct decoded
                // values proves the actual GPU fixture covers every stored code,
                // even for the sRGB encode performed while drawing the prefix.
                assert_eq!(
                    values.len(),
                    256,
                    "{label}: incomplete code sweep, sample {sample} row {row} channel {channel}"
                );
            }
        }
    }
    if samples > 1 {
        let stride = (WIDTH * HEIGHT * 16) as usize;
        for sample in 1..samples as usize {
            assert_ne!(
                &prefix[..stride],
                &prefix[sample * stride..(sample + 1) * stride],
                "{label}: sample {sample} must differ from sample 0"
            );
        }
    }
}

#[test]
fn same_format_per_sample_transfer_then_transparent_suffix_is_exact() {
    let instance = renderer::data_render::create_instance();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("P00 requires a GPU adapter; unavailable is NOT a passing experiment");
    eprintln!("P00 adapter: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("P00 GPU device creation failed");
    let mut failures = Vec::new();
    let mut tested = 0;
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        for samples in [1, 4] {
            let features = adapter.get_texture_format_features(format);
            if !features.allowed_usages.contains(
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ) || !features.flags.sample_count_supported(samples)
                || (samples > 1
                    && !features
                        .flags
                        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE))
            {
                eprintln!("SKIP unsupported {format:?} x{samples}: {features:?}");
                continue;
            }
            let label = format!("{format:?} x{samples}");
            let source = shader_source(samples, format.is_srgb());
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("P00 prefix/suffix"),
                source: wgpu::ShaderSource::Wgsl(source.clone().into()),
            });
            let prefix = pipeline(&device, &module, "prefix", format, samples, false);
            let suffix = pipeline(&device, &module, "suffix", format, samples, true);
            let transfer_fn = if samples > 1 {
                r#"
@group(0) @binding(0) var src: texture_multisampled_2d<f32>;
@fragment fn transfer(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @location(0) vec4<f32> {
    return textureLoad(src, vec2<i32>(p.xy), i32(s));
}"#
            } else {
                r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@fragment fn transfer(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(src, vec2<i32>(p.xy), 0);
}"#
            };
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("P00 transfer"),
                source: wgpu::ShaderSource::Wgsl((source + transfer_fn).into()),
            });
            let transfer = pipeline(&device, &module, "transfer", format, samples, false);
            let accumulated = texture(&device, format, samples);
            let direct = texture(&device, format, samples);
            let display = texture(&device, format, samples);
            let direct_resolved = texture(&device, format, 1);
            let display_resolved = texture(&device, format, 1);
            let av = accumulated.create_view(&Default::default());
            let dv = direct.create_view(&Default::default());
            let tv = display.create_view(&Default::default());
            let dr = direct_resolved.create_view(&Default::default());
            let tr = display_resolved.create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("P00 same-format source"),
                layout: &transfer.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&av),
                }],
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            draw(
                &mut encoder,
                &dv,
                (samples > 1).then_some(&dr),
                false,
                &[(&prefix, None), (&suffix, None)],
            );
            draw(&mut encoder, &av, None, false, &[(&prefix, None)]);
            draw(&mut encoder, &tv, None, false, &[(&transfer, Some(&bind))]);
            let before = extract(&device, &mut encoder, &av, samples);
            let after = extract(&device, &mut encoder, &tv, samples);
            draw(
                &mut encoder,
                &tv,
                (samples > 1).then_some(&tr),
                true,
                &[(&suffix, None)],
            );
            let direct_samples = extract(&device, &mut encoder, &dv, samples);
            let display_samples = extract(&device, &mut encoder, &tv, samples);
            let direct_bytes = resolved_bytes(
                &device,
                &mut encoder,
                if samples > 1 {
                    &direct_resolved
                } else {
                    &direct
                },
            );
            let display_bytes = resolved_bytes(
                &device,
                &mut encoder,
                if samples > 1 {
                    &display_resolved
                } else {
                    &display
                },
            );
            queue.submit([encoder.finish()]);
            verify_fixture(&device, &before, &display_samples, samples, &label);
            compare(
                &device,
                &before,
                &after,
                &format!("{label} prefix sample bits"),
                &mut failures,
            );
            compare(
                &device,
                &direct_samples,
                &display_samples,
                &format!("{label} suffix sample bits"),
                &mut failures,
            );
            compare(
                &device,
                &direct_bytes,
                &display_bytes,
                &format!("{label} resolved bytes"),
                &mut failures,
            );
            tested += 1;
        }
    }
    assert_eq!(
        tested, 8,
        "Required matrix incomplete; experiment is inconclusive (see SKIP diagnostics)"
    );
    eprintln!(
        "P00 executed {tested}/8 format/sample cases; 256 codes/channel in rows 0..8, premultiplied rows 8..16"
    );
    assert!(
        failures.is_empty(),
        "P00 exactness failed:\n{}",
        failures.join("\n")
    );
}
