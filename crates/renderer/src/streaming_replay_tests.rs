use super::*;
use crate::renderer::streaming_request_tests::read_draw_target;
use crate::renderer::{Renderer, RendererDevice};

fn clear(r: &Renderer, s: &StreamSurface, color: wgpu::Color) {
    let view = s.prefix.create_view(&Default::default());
    let mut e = r.device.create_command_encoder(&Default::default());
    drop(e.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    }));
    r.queue.submit([e.finish()]);
}

fn rect(r: &Renderer, s: &StreamSurface, rgba: [f32; 4], rect: [u32; 4]) {
    let shader=crate::gpu_compile::shader_module(&r.device, wgpu::ShaderModuleDescriptor {
        label:Some("test translucent replay rectangle"),source:wgpu::ShaderSource::Wgsl(format!(r#"
        @vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {{
            let p=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.)); return vec4(p[i],0.,1.);
        }}
        @fragment fn fs()->@location(0) vec4<f32> {{ return vec4<f32>({},{},{},{}); }}
        "#,rgba[0],rgba[1],rgba[2],rgba[3]).into()),
    });
    let pipeline = crate::gpu_compile::render_pipeline(&r.device, &wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: s.spec.sample_count,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: s.spec.format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
    let view = s.prefix.create_view(&Default::default());
    let mut e = r.device.create_command_encoder(&Default::default());
    {
        let mut p = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        p.set_scissor_rect(rect[0], rect[1], rect[2], rect[3]);
        p.set_pipeline(&pipeline);
        p.draw(0..3, 0..1);
    }
    r.queue.submit([e.finish()]);
}

fn display(
    r: &Renderer,
    s: &StreamSurface,
    t: &StreamTransfer,
    bg: Option<wgpu::Color>,
    reveal: bool,
) -> Vec<u8> {
    let mut e = r.device.create_command_encoder(&Default::default());
    s.record_display_chunks(
        t,
        &mut e,
        bg,
        reveal.then_some([0, 0, s.spec.width, s.spec.height]),
        |_| {},
        |_| {},
        |_| {},
    )
    .unwrap();
    r.queue.submit([e.finish()]);
    read_draw_target(r, s.resolved())
}

#[test]
fn progressive_replay_selects_one_image_without_double_alpha_or_blank_suffix() {
    let _font = crate::text_render::FONT_REGISTRATION_TEST_LOCK.lock().unwrap();
    let (device, queue) = crate::data_render::shared_device().unwrap();
    for samples in [1, 4] {
        for opaque in [false, true] {
            for separate_grid in [false, true] {
                let mut r = Renderer::try_new_with_sample_count(
                    RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
                    wgpu::TextureFormat::Rgba8Unorm,
                    4096,
                    samples,
                )
                .unwrap();
                let spec = StreamSurfaceSpec {
                    width: 64,
                    height: 32,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    sample_count: samples,
                };
                let mut transfer = StreamTransfer::new(&r.device, spec.format, samples).unwrap();
                let mut surface =
                    StreamSurface::new(&r.device, &r.gpu_ledger, &transfer, spec, u64::MAX, 0)
                        .unwrap();
                let color = if opaque {
                    wgpu::Color::WHITE
                } else {
                    wgpu::Color::TRANSPARENT
                };
                let background = separate_grid.then_some(color);
                clear(
                    &r,
                    &surface,
                    wgpu::Color {
                        r: 0.0,
                        g: 0.5,
                        b: 0.0,
                        a: 0.5,
                    },
                );
                let preview = display(&r, &surface, &transfer, Some(color), false);
                let usage = r.gpu_memory_usage();
                let mut e = r.device.create_command_encoder(&Default::default());
                let pool = r.pool.gpu_bytes() + r.pool.retired_bytes();
                assert_eq!(
                    surface.begin_replay(
                        &r.device,
                        &r.gpu_ledger,
                        &mut transfer,
                        &mut e,
                        color,
                        |_| {},
                        usage.total_bytes() + replay_bytes(spec).unwrap() - 1,
                        pool
                    ),
                    Err(StreamSurfaceError::TooLarge)
                );
                assert!(!surface.has_replay());
                assert_eq!(
                    r.gpu_memory_usage(),
                    usage,
                    "budget rejection allocated resources"
                );
                surface
                    .begin_replay(
                        &r.device,
                        &r.gpu_ledger,
                        &mut transfer,
                        &mut e,
                        color,
                        |_| {},
                        u64::MAX,
                        pool,
                    )
                    .unwrap();
                r.queue.submit([e.finish()]);
                assert!(surface.has_replay());
                clear(
                    &r,
                    &surface,
                    if separate_grid {
                        wgpu::Color::TRANSPARENT
                    } else {
                        color
                    },
                );
                // The empty beginning retains the complete preview, including alpha.
                assert_eq!(display(&r, &surface, &transfer, background, true), preview);
                let cache_allocations = r.gpu_memory_usage();
                for (index, rgba) in [
                    [0.5, 0.0, 0.0, 0.5],
                    [0.0, 0.0, 0.5, 0.5],
                    [1.0, 1.0, 1.0, 1.0],
                ]
                .into_iter()
                .enumerate()
                {
                    rect(&r, &surface, rgba, [8, 8, 8, 8]);
                    let exact = display(&r, &surface, &transfer, background, false);
                    let shown = display(&r, &surface, &transfer, background, true);
                    let inside = (10 * 64 + 10) * 4;
                    let untouched = (24 * 64 + 56) * 4;
                    assert_eq!(
                        &shown[inside..inside + 4],
                        &exact[inside..inside + 4],
                        "new chunk was blended over preview"
                    );
                    assert_eq!(
                        &shown[untouched..untouched + 4],
                        &preview[untouched..untouched + 4],
                        "unprocessed part disappeared"
                    );
                    // The immediately adjacent pixel must retain preview. Even a
                    // one-pixel AA guard erased the unfinished line at the seam.
                    let frontier = (12 * 64 + 16) * 4;
                    assert_eq!(
                        &shown[frontier..frontier + 4],
                        &preview[frontier..frontier + 4],
                        "reveal erased preview ahead of the processed pixels"
                    );
                    for ((actual, a), b) in shown
                        .chunks_exact(4)
                        .zip(exact.chunks_exact(4))
                        .zip(preview.chunks_exact(4))
                    {
                        assert!(
                            actual == a || actual == b,
                            "a pixel combined two images: samples={samples},opaque={opaque},chunk={index}"
                        );
                    }
                    assert_eq!(
                        display(&r, &surface, &transfer, background, true),
                        shown,
                        "idle display compounded alpha"
                    );
                    for kind in [
                        GpuResourceKind::PanelTexture,
                        GpuResourceKind::MsaaTarget,
                        GpuResourceKind::StreamingUpload,
                    ] {
                        assert_eq!(
                            r.gpu_memory_usage().creations_of(kind),
                            cache_allocations.creations_of(kind),
                            "per-chunk allocation"
                        );
                    }
                }
                // Finishing removes all preview remnants. Dropping early (cancel) uses
                // the same charged ownership boundary for both snapshots and the mask.
                let expected = display(&r, &surface, &transfer, background, false);
                surface.finish_replay();
                assert_eq!(display(&r, &surface, &transfer, background, true), expected);
                drop(surface);
                r.end_gpu_frame();
                r.wait_idle();
                r.service_gpu_completions().unwrap();
                assert_eq!(
                    r.gpu_memory_usage()
                        .live_bytes_of(GpuResourceKind::StreamingUpload),
                    0
                );
                assert_eq!(
                    r.gpu_memory_usage()
                        .retired_bytes_of(GpuResourceKind::StreamingUpload),
                    0
                );
            }
        }
    }
}
