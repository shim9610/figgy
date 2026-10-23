//! Windows DX12 workgroup initialization workaround.
//!
//! Naga's aggregate array zero becomes hundreds of static DXIL stores. WARP
//! spends seconds analyzing that block on first dispatch. These specific
//! shaders instead zero one element per invocation and synchronize before any
//! computation. Other shaders and backends retain wgpu's automatic zeroing.

pub(super) const EXPLICIT_ZERO: &str = "FIGGY_EXPLICIT_WORKGROUP_ZERO";

fn explicit_options() -> wgpu::PipelineCompilationOptions<'static> {
    wgpu::PipelineCompilationOptions {
        constants: &[(EXPLICIT_ZERO, 1.0)],
        zero_initialize_workgroup_memory: false,
    }
}

/// Only use with entries that call `figgy_zero_workgroup` before computation.
pub(crate) fn compilation_options(
    device: &wgpu::Device,
) -> wgpu::PipelineCompilationOptions<'static> {
    if cfg!(target_os = "windows") && device.adapter_info().backend == wgpu::Backend::Dx12 {
        explicit_options()
    } else {
        wgpu::PipelineCompilationOptions::default()
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct Case {
        source: &'static str,
        array: &'static str,
        width: u32,
        poison: &'static str,
        bits: &'static str,
    }

    const CASES: &[Case] = &[
        Case {
            source: crate::gpu_errorbar::ERRORBAR_FIT_SHADER,
            array: "shared_states",
            width: 64,
            poison: "SeriesState(AxisState(vec2<f32>(1.0), vec2<f32>(2.0), vec2<f32>(3.0)), AxisState(vec2<f32>(4.0), vec2<f32>(5.0), vec2<f32>(6.0)))",
            bits: "bitcast<u32>(v.x.minimum.x) | bitcast<u32>(v.x.minimum.y) | bitcast<u32>(v.x.maximum.x) | bitcast<u32>(v.x.maximum.y) | bitcast<u32>(v.x.minimum_positive.x) | bitcast<u32>(v.x.minimum_positive.y) | bitcast<u32>(v.y.minimum.x) | bitcast<u32>(v.y.minimum.y) | bitcast<u32>(v.y.maximum.x) | bitcast<u32>(v.y.maximum.y) | bitcast<u32>(v.y.minimum_positive.x) | bitcast<u32>(v.y.minimum_positive.y)",
        },
        Case {
            source: crate::gpu_pick::PickShader::MarkerLine.source(),
            array: "pick_shared_candidates",
            width: 64,
            poison: "PickCandidate(1u, 2u, 3u, 4u, 5u, 6.0, 7.0, 8u)",
            bits: "v.valid | v.series_order | v.point_index | v.primitive_kind | v.primitive_index | bitcast<u32>(v.distance_sq) | bitcast<u32>(v.distance_px) | v._pad",
        },
        Case {
            source: crate::data_render::BAR_SHADER,
            array: "data_pick_shared",
            width: 64,
            poison: "DataPickCandidate(1u, 2u, 3u, 4u, 5u, 6u, 7.0, 8u)",
            bits: "v.valid | v.paint_order | v.kind | v.index0 | v.index1 | v.index2 | bitcast<u32>(v.distance_px) | v.primitive_order",
        },
        Case {
            source: crate::data_render::line_arc::LINE_ARC_SHADER,
            array: "scan_shared",
            width: 256,
            poison: "23.0",
            bits: "bitcast<u32>(v)",
        },
    ];

    #[test]
    fn every_workgroup_entry_initializes_before_computation() {
        use wgpu::naga::{
            AddressSpace, Statement,
            valid::{Capabilities, ValidationFlags, Validator},
        };
        for case in CASES {
            let module = wgpu::naga::front::wgsl::parse_str(case.source).unwrap();
            let info = Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .unwrap();
            let init = module
                .functions
                .iter()
                .find(|(_, f)| f.name.as_deref() == Some("figgy_zero_workgroup"))
                .unwrap()
                .0;
            for (i, entry) in module.entry_points.iter().enumerate() {
                if !module.global_variables.iter().any(|(h, v)| {
                    v.space == AddressSpace::WorkGroup && !info.get_entry_point(i)[h].is_empty()
                }) {
                    continue;
                }
                // Argument expressions can precede the call; no memory effect,
                // branch, return or other function may precede initialization.
                let first_effect = entry
                    .function
                    .body
                    .iter()
                    .find(|s| !matches!(s, Statement::Emit(_)));
                assert!(
                    matches!(first_effect, Some(Statement::Call { function, .. }) if *function == init),
                    "{} starts before initialization",
                    entry.name
                );
                assert_eq!(entry.workgroup_size, [case.width, 1, 1]);
            }
        }
    }

    #[test]
    fn explicit_workgroup_zero_clears_every_field_and_synchronizes() {
        let (device, queue) = crate::data_render::shared_device().expect("required adapter");
        let selected = compilation_options(&device);
        assert_eq!(
            !selected.zero_initialize_workgroup_memory,
            cfg!(target_os = "windows") && device.adapter_info().backend == wgpu::Backend::Dx12
        );
        for case in CASES {
            let source = format!(
                r#"{}
@group(0) @binding(15) var<storage, read_write> figgy_zero_result: array<u32>;
@compute @workgroup_size({width})
fn figgy_zero_oracle(@builtin(local_invocation_index) lane: u32,
                     @builtin(workgroup_id) group: vec3<u32>) {{
    {array}[lane] = {poison};
    workgroupBarrier();
    figgy_zero_workgroup(lane);
    let v = {array}[(lane + 17u) % {width}u];
    figgy_zero_result[group.x * {width}u + lane] = {bits};
}}
"#,
                case.source,
                width = case.width,
                array = case.array,
                poison = case.poison,
                bits = case.bits
            );
            let started = Instant::now();
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("explicit workgroup zero oracle"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("explicit workgroup zero oracle"),
                layout: None,
                module: &shader,
                entry_point: Some("figgy_zero_oracle"),
                compilation_options: explicit_options(),
                cache: None,
            });
            let size = u64::from(case.width) * 4 * 4;
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 15,
                    resource: output.as_entire_binding(),
                }],
            });
            for repetition in 0..4 {
                queue.write_buffer(&output, 0, &vec![0xa5; size as usize]);
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.dispatch_workgroups(4, 1, 1);
                }
                encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
                queue.submit([encoder.finish()]);
                let (tx, rx) = std::sync::mpsc::channel();
                readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(Duration::from_secs(10)),
                    })
                    .unwrap();
                rx.recv().unwrap().unwrap();
                if repetition == 0 {
                    let elapsed = started.elapsed();
                    eprintln!(
                        "explicit zero {}: creation + first completion {elapsed:?}",
                        case.array
                    );
                    if device.adapter_info().backend == wgpu::Backend::Dx12 {
                        assert!(
                            elapsed < Duration::from_secs(5),
                            "{} first-use initialization stalled: {elapsed:?}",
                            case.array
                        );
                    }
                }
                {
                    let bytes = readback.slice(..).get_mapped_range().unwrap();
                    let words: &[u32] = bytemuck::cast_slice(&bytes);
                    assert!(
                        words.iter().all(|&v| v == 0),
                        "{} repetition={repetition}: zeroing or visibility failed",
                        case.array
                    );
                }
                readback.unmap();
            }
        }
    }
}
