#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use renderer::data_render::{create_instance, request_adapter_async, request_device_async};
use renderer::gpu_memory;
use wasm_bindgen_test::*;

// Exercise the product module without adding a public renderer API for tests.
#[path = "../../renderer/src/streaming_surface.rs"]
mod streaming_surface;
use streaming_surface::{StreamSurface, StreamSurfaceSpec, StreamTransfer};

wasm_bindgen_test_configure!(run_in_browser);

const WIDTH: u32 = 256;
const HEIGHT: u32 = 16;

// Independent fixture geometry; no SHADER_COMMON.md definitions are used.
const DRAW: &str = r#"
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}
@fragment fn prefix(@builtin(position) p: vec4<f32>, SAMPLE_ARG) -> @location(0) vec4<f32> {
    let x = u32(p.x);
    let y = u32(p.y);
    let a = f32((x * 19u + y * 31u + s * 53u) % 256u) / 255.0;
    let rgb = vec3<f32>(f32((x + s * 61u) % 256u),
        f32((x * 37u + y * 11u + s * 43u) % 256u),
        f32((x * 73u + y * 7u + s * 29u) % 256u)) / 255.0;
    return vec4(rgb * a, a);
}
@fragment fn suffix(@builtin(position) p: vec4<f32>, SAMPLE_ARG) -> @location(0) vec4<f32> {
    let x = u32(p.x);
    let y = u32(p.y);
    if ((x + y + s) % 5u == 0u) { discard; }
    let a = f32(1u + (x * 13u + y * 7u + s * 23u) % 253u) / 255.0;
    return vec4(vec3(0.8, 0.2, 0.6) * a, a);
}
"#;

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry: &str,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: None,
        vertex: wgpu::VertexState {
            module,
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
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: (entry == "suffix")
                    .then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn texture(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("browser direct reference"),
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
            | if samples == 1 {
                wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::empty()
            },
        view_formats: &[],
    })
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    pipelines: &[&wgpu::RenderPipeline],
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("browser reference/prefix"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: resolve,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    for pipeline in pipelines {
        pass.set_pipeline(pipeline);
        pass.draw(0..3, 0..1);
    }
}

fn copy_pixels(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("browser test-only readback"),
        size: u64::from(WIDTH * HEIGHT * 4),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        texture.size(),
    );
    buffer
}

async fn read_pixels(buffer: &wgpu::Buffer) -> Vec<u8> {
    let (tx, rx) = futures_channel::oneshot::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).expect("map receiver alive");
        });
    rx.await.expect("map callback").expect("WebGPU readback");
    let bytes = buffer
        .slice(..)
        .get_mapped_range()
        .expect("mapped pixels")
        .to_vec();
    buffer.unmap();
    bytes
}

#[wasm_bindgen_test(async)]
async fn chrome_stream_surface_matches_direct_render_all_formats_and_samples() {
    let instance = create_instance();
    let adapter = request_adapter_async(&instance)
        .await
        .expect("WebGPU adapter required; unavailable is not a passing test");
    let (device, queue) = request_device_async(&adapter).await.expect("WebGPU device");
    let ledger = Arc::new(gpu_memory::GpuLedger::new());
    let mut tested = 0;
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        for samples in [1, 4] {
            let source = if samples == 4 {
                DRAW.replace("SAMPLE_ARG", "@builtin(sample_index) s: u32")
            } else {
                DRAW.replace(", SAMPLE_ARG", "")
                    .replace("let x = u32(p.x);", "let s = 0u; let x = u32(p.x);")
            };
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("browser prefix/suffix fixture"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let prefix = pipeline(&device, &module, "prefix", format, samples);
            let suffix = pipeline(&device, &module, "suffix", format, samples);
            let transfer =
                StreamTransfer::new(&device, format, samples).expect("production transfer");
            let surface = StreamSurface::new(
                &device,
                &ledger,
                &transfer,
                StreamSurfaceSpec {
                    width: WIDTH,
                    height: HEIGHT,
                    format,
                    sample_count: samples,
                },
                16 * 1024 * 1024,
                0,
            )
            .expect("production surface");
            let direct = texture(&device, format, samples);
            let resolved = texture(&device, format, 1);
            let direct_view = direct.create_view(&Default::default());
            let resolved_view = resolved.create_view(&Default::default());
            let prefix_view = surface.prefix().create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            draw(
                &mut encoder,
                &direct_view,
                (samples > 1).then_some(&resolved_view),
                &[&prefix, &suffix],
            );
            draw(&mut encoder, &prefix_view, None, &[&prefix]);
            let expected = copy_pixels(
                &device,
                &mut encoder,
                if samples == 1 { &direct } else { &resolved },
            );
            surface
                .record_display(&transfer, &mut encoder, |_| {})
                .expect("prefix-only display");
            let before = copy_pixels(&device, &mut encoder, surface.resolved());
            surface
                .record_display(&transfer, &mut encoder, |pass| {
                    pass.set_pipeline(&suffix);
                    pass.draw(0..3, 0..1);
                })
                .expect("suffix display");
            let actual = copy_pixels(&device, &mut encoder, surface.resolved());
            surface
                .record_display(&transfer, &mut encoder, |pass| {
                    pass.set_pipeline(&suffix);
                    pass.draw(0..3, 0..1);
                })
                .expect("repeated suffix display");
            let repeated = copy_pixels(&device, &mut encoder, surface.resolved());
            queue.submit([encoder.finish()]);
            let expected = read_pixels(&expected).await;
            let before = read_pixels(&before).await;
            let actual = read_pixels(&actual).await;
            let repeated = read_pixels(&repeated).await;
            assert!(
                before.chunks_exact(4).any(|pixel| pixel != &before[..4]),
                "nonuniform fixture"
            );
            assert_ne!(
                before, expected,
                "{format:?} x{samples}: suffix must affect pixels"
            );
            assert_eq!(
                actual, expected,
                "{format:?} x{samples}: transferred suffix"
            );
            assert_eq!(
                repeated, expected,
                "{format:?} x{samples}: display must not contaminate prefix"
            );
            tested += 1;
        }
    }
    assert_eq!(
        tested, 8,
        "all browser format/sample combinations must execute"
    );
}
