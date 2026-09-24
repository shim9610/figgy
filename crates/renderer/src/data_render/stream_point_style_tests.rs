//! Product-entry GPU parity for bounded point/errorbar style identities.
//! Full fixture columns and readback are test-oracle resources only.

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::gpu_memory::{GpuLedger, GpuResourceKind};
    use std::sync::Arc;

    fn read_buffer(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
        let (tx, rx) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("streamed style GPU completion");
        rx.recv().unwrap().expect("streamed style readback map");
        let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        buffer.unmap();
        bytes
    }

    fn transform(params: [[f32; 4]; 3]) -> ScatterTransform {
        ScatterTransform {
            data_min: [1e9, 0.0],
            data_max: [1e9, 1.0],
            data_min_lo: [0.0, 0.0],
            data_max_lo: [8.0, 0.0],
            scale_log: [0.0; 2],
            pixel_to_ndc: [2.0 / 64.0; 2],
            data_to_panel_scale: [0.91, -0.88],
            data_to_panel_offset: [0.03, 0.94],
            style_params: params,
        }
    }

    #[test]
    fn streamed_style_global_index_preserves_all_u32_bits_and_uniform_ownership() {
        let (device, queue) = shared_device().expect("streamed style GPU adapter required");
        let transform_bgl = create_scatter_transform_bind_group_layout(&device);
        let output_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("streamed style identity probe output"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("streamed style identity probe"),
            bind_group_layouts: &[Some(&transform_bgl), Some(&output_bgl)],
            immediate_size: 0,
        });
        let probe = r#"
@group(1) @binding(8) var<storage, read_write> identity_probe: array<vec4<u32>>;
@compute @workgroup_size(4)
fn probe_identity(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = styled_point_index(gid.x);
    identity_probe[gid.x] = vec4<u32>(index,
        bitcast<u32>(sketch_hash01(index, 0x12345678u)),
        bitcast<u32>(transform.style_params[2].x),
        bitcast<u32>(transform.style_params[2].y));
}
"#;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("streamed style identity output"),
            size: 64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("streamed style identity readback"),
            size: 64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let output_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("streamed style identity output"),
            layout: &output_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 8,
                resource: output.as_entire_binding(),
            }],
        });
        let base_transform = transform([[1.0, 2.0, 3.0, 4.0], [5.0; 4], [1.25, 0.75, 0.0, 0.0]]);
        let original = bytemuck::bytes_of(&base_transform).to_vec();
        let ledger = Arc::new(GpuLedger::new());
        let bases = [
            0,
            1,
            (1 << 24) + 1,
            0x1234_5678,
            0x7f80_0000,
            0x7fc1_2345,
            0x8000_0001,
            u32::MAX - 3,
        ];
        let snapshots: Vec<_> = bases
            .iter()
            .map(|base| {
                create_stream_point_transform_bind_group(
                    &ledger,
                    &device,
                    &transform_bgl,
                    &base_transform,
                    *base,
                )
            })
            .collect();
        assert_eq!(bytemuck::bytes_of(&base_transform), &original);
        assert_eq!(
            ledger.snapshot().live_bytes_of(GpuResourceKind::Uniform),
            bases.len() as u64 * STREAM_POINT_TRANSFORM_BYTES
        );
        for source in [
            include_str!("scatter_columnar.wgsl"),
            include_str!("errorbar_columnar.wgsl"),
        ] {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("streamed style product identity probe"),
                source: wgpu::ShaderSource::Wgsl(format!("{source}\n{probe}").into()),
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("streamed style identity probe"),
                layout: Some(&layout),
                module: &shader,
                entry_point: Some("probe_identity"),
                compilation_options: Default::default(),
                cache: None,
            });
            for (&base, (bg, _charge)) in bases.iter().zip(&snapshots) {
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, bg, &[]);
                    pass.set_bind_group(1, &output_bg, &[]);
                    pass.dispatch_workgroups(1, 1, 1);
                }
                encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 64);
                queue.submit([encoder.finish()]);
                let bytes = read_buffer(&device, &readback);
                let values: &[u32] = bytemuck::cast_slice(&bytes);
                for local in 0..4u32 {
                    let index = base + local;
                    let mut hash =
                        index.wrapping_mul(0x9e37_79b9) ^ 0x1234_5678u32.wrapping_mul(0x85eb_ca6b);
                    hash = (hash ^ (hash >> 16)).wrapping_mul(0x045d_9f3b);
                    hash ^= hash >> 16;
                    let expected_hash = (hash as f32 / 4294967296.0).to_bits();
                    assert_eq!(
                        &values[local as usize * 4..local as usize * 4 + 4],
                        &[index, expected_hash, 1.25f32.to_bits(), 0.75f32.to_bits()],
                        "base={base:#x} local={local}"
                    );
                }
            }
        }
        drop(snapshots);
        assert_eq!(ledger.snapshot().live_bytes(), 0);
        assert_eq!(
            ledger.snapshot().retired_bytes(),
            bases.len() as u64 * STREAM_POINT_TRANSFORM_BYTES
        );
    }

    struct DrawCase<'a> {
        name: &'static str,
        pipeline: &'a wgpu::RenderPipeline,
        texture_bg: Option<&'a wgpu::BindGroup>,
        errorbar: bool,
        transform: ScatterTransform,
    }

    fn render_case(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bg: &wgpu::BindGroup,
        case: &DrawCase<'_>,
        samples: u32,
        chunk_size: usize,
        include_global_base: bool,
    ) -> Vec<u8> {
        const N: usize = 19;
        let mut values = Vec::<[f32; 2]>::new();
        values.extend((0..N).map(|i| {
            if i == 11 {
                [f32::NAN, 0.0]
            } else {
                [1e9, 0.6 + (i % 9) as f32 * 0.7]
            }
        }));
        values.extend((0..N).map(|i| [0.2 + (i % 5) as f32 * 0.12, 2f32.powi(-26)]));
        values.extend((0..N).map(|i| [0.05 + (i % 4) as f32 * 0.012, 2f32.powi(-30)]));
        let pool = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("streamed style oracle columns"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let quad = create_unit_centered_quad_vertex_buffer(device);
        let texture = |sample_count| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("streamed style parity target"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | if sample_count == 1 {
                        wgpu::TextureUsages::COPY_SRC
                    } else {
                        wgpu::TextureUsages::empty()
                    },
                view_formats: &[],
            })
        };
        let target = texture(samples);
        let target_view = target.create_view(&Default::default());
        let resolved = (samples > 1).then(|| texture(1));
        let resolved_view = resolved
            .as_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let ledger = Arc::new(GpuLedger::new());
        let chunks: Vec<_> = (0..N)
            .step_by(chunk_size)
            .map(|start| {
                let len = chunk_size.min(N - start);
                let (bg, charge) = create_stream_point_transform_bind_group(
                    &ledger,
                    device,
                    transform_bgl,
                    &case.transform,
                    if include_global_base { start as u32 } else { 0 },
                );
                (start, len, bg, charge)
            })
            .collect();
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("streamed style parity"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: resolved_view.as_ref(),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(case.pipeline);
            pass.set_bind_group(1, style_bg, &[]);
            if let Some(bg) = case.texture_bg {
                pass.set_bind_group(2, bg, &[]);
            }
            for (start, len, bg, _) in &chunks {
                pass.set_bind_group(0, bg, &[]);
                let range = |column: usize| {
                    ((column * N + start) as u64 * 8)..((column * N + start + len) as u64 * 8)
                };
                if case.errorbar {
                    pass.set_vertex_buffer(0, pool.slice(range(0)));
                    pass.set_vertex_buffer(1, pool.slice(range(1)));
                    for slot in 2..6 {
                        pass.set_vertex_buffer(slot, pool.slice(range(2)));
                    }
                    pass.draw(0..36, 0..*len as u32);
                } else {
                    pass.set_vertex_buffer(0, quad.slice(..));
                    pass.set_vertex_buffer(1, pool.slice(range(0)));
                    pass.set_vertex_buffer(2, pool.slice(range(1)));
                    pass.draw(0..4, 0..*len as u32);
                }
            }
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("streamed style parity readback"),
            size: 64 * 64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: resolved.as_ref().unwrap_or(&target),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        read_buffer(device, &readback)
    }

    #[test]
    fn streamed_style_point_and_errorbar_entries_match_resident_pixels() {
        let (device, queue) = shared_device().expect("streamed style GPU adapter required");
        let transform_bgl = create_scatter_transform_bind_group_layout(&device);
        let style_bgl = create_style_bind_group_layout(&device);
        let star_bgl = create_star_data_bind_group_layout(&device);
        let shaders = ShaderModules::new(&device);
        let mut style = PrimitiveStyle::from_color_with_width(
            Color {
                r: 0.3,
                g: 0.7,
                b: 0.9,
                a: 0.72,
            },
            1.7,
        );
        style.point_radius_px = 5.5;
        style.cap_half_px = 3.2;
        style.cap_width_px = 1.7;
        style.shape_id = shape_id(&ScatterShape::CircleFilled);
        style.series_salt = 0x9876_1234;
        style.primitive_flags = 3;
        let style_buffer = create_style_uniform_buffer(&device, &style);
        let style_bg = create_style_bind_group(&device, &style_bgl, &style_buffer);
        let sketch = transform([[2.1, 13.7, 123.0, 0.0], [0.0; 4], [0.0; 4]]);
        let milkyway = transform([
            [16.0, 8.0, 0.7, 127.0],
            [1.4, 4.0, 1.1, 0.6],
            [1.0, 0.9, 0.0, 0.0],
        ]);
        let constellation = transform([[0.8, 0.4, 0.0, 0.0], [0.0; 4], [0.0; 4]]);
        for samples in [1, 4] {
            let scatter = create_scatter_columnar_pipeline_with_entries(
                &device,
                &shaders.scatter,
                &transform_bgl,
                &style_bgl,
                wgpu::TextureFormat::Rgba8Unorm,
                samples,
                "vs_sketch",
                "fs_sketch",
                "stream sketch scatter",
            );
            let errorbar = create_errorbar_columnar_pipeline_with_entries(
                &device,
                &shaders.errorbar,
                &transform_bgl,
                &style_bgl,
                wgpu::TextureFormat::Rgba8Unorm,
                samples,
                "vs_sketch",
                "stream sketch errorbar",
            );
            let milky = create_milkyway_set(
                &device,
                &queue,
                &shaders,
                &transform_bgl,
                &style_bgl,
                &star_bgl,
                wgpu::TextureFormat::Rgba8Unorm,
                samples,
            );
            let cons = create_point_constellation_set(
                &device,
                &queue,
                &shaders,
                &transform_bgl,
                &style_bgl,
                wgpu::TextureFormat::Rgba8Unorm,
                samples,
            );
            let cases = [
                DrawCase {
                    name: "sketch scatter",
                    pipeline: &scatter,
                    texture_bg: None,
                    errorbar: false,
                    transform: sketch,
                },
                DrawCase {
                    name: "sketch errorbar",
                    pipeline: &errorbar,
                    texture_bg: None,
                    errorbar: true,
                    transform: sketch,
                },
                DrawCase {
                    name: "milkyway planet",
                    pipeline: &milky.planets,
                    texture_bg: Some(&milky.star_tex_bg),
                    errorbar: false,
                    transform: milkyway,
                },
                DrawCase {
                    name: "milkyway jet",
                    pipeline: &milky.jets,
                    texture_bg: None,
                    errorbar: true,
                    transform: milkyway,
                },
                DrawCase {
                    name: "constellation point",
                    pipeline: &cons.stars,
                    texture_bg: Some(&cons.star_tex_bg),
                    errorbar: false,
                    transform: constellation,
                },
            ];
            for case in &cases {
                let oracle = render_case(
                    &device,
                    &queue,
                    &transform_bgl,
                    &style_bg,
                    case,
                    samples,
                    19,
                    false,
                );
                assert!(
                    oracle.iter().any(|value| *value != 0),
                    "{} is vacuous",
                    case.name
                );
                let wrong = render_case(
                    &device,
                    &queue,
                    &transform_bgl,
                    &style_bg,
                    case,
                    samples,
                    3,
                    false,
                );
                assert_ne!(
                    oracle, wrong,
                    "{} fixture did not exercise global identity",
                    case.name
                );
                for chunk in [1, 2, 3, 7] {
                    let actual = render_case(
                        &device,
                        &queue,
                        &transform_bgl,
                        &style_bg,
                        case,
                        samples,
                        chunk,
                        true,
                    );
                    assert_eq!(oracle, actual, "{} {samples}x chunk={chunk}", case.name);
                }
            }
        }
    }
}
