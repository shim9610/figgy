//! Isolated Metal shader-supply experiment, not a production picker.
use std::{borrow::Cow, fs, path::Path, sync::mpsc};
use wgpu::util::DeviceExt;

const COUNT: usize = 64;
// These fixtures use exactly representable binary fractions. This subpixel
// bound detects destroyed hi/lo residuals; it is not a general picking tolerance.
const MAX_PIXEL_ERROR: f64 = 0.001;

fn definition(source: &str, kind: &str, name: &str) -> String {
    let needle = format!("{kind} {name}");
    let start = source.find(&needle).expect("production definition missing");
    let open = start + source[start..].find('{').unwrap();
    let mut depth = 0;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let end = open + offset + 1;
                    return format!(
                        "{}{}\n",
                        &source[start..end],
                        if kind == "struct" { ";" } else { "" }
                    );
                }
            }
            _ => {}
        }
    }
    panic!("unterminated production definition {name}")
}

fn generate(out: &Path, root: &Path) {
    fs::create_dir_all(out).unwrap();
    let production = fs::read_to_string(root.join("crates/renderer/src/gpu_pick.wgsl")).unwrap();
    let mut source = String::new();
    for name in ["PickQueryTransform", "PickQueryParams"] {
        source.push_str(&definition(&production, "struct", name));
    }
    source.push_str(
        r#"
@group(0) @binding(0) var<storage, read> inputs: array<vec4<f32>, 64>;
@group(0) @binding(1) var<storage, read_write> outputs: array<vec4<f32>, 64>;
@group(1) @binding(0) var<uniform> pick_query_params: PickQueryParams;
"#,
    );
    for name in [
        "pick_log10",
        "pick_axis_pair_to_t",
        "pick_project_axis_pair",
        "pick_project_pair",
    ] {
        source.push_str(&definition(&production, "fn", name));
    }
    source.push_str(
        r#"
@compute @workgroup_size(64)
fn precision_probe(@builtin(global_invocation_id) gid: vec3<u32>) {
    let v = inputs[gid.x];
    let p = pick_project_pair(v.xy, v.zw);
    outputs[gid.x] = vec4(p, pick_axis_pair_to_t(v.xy, 0u), pick_axis_pair_to_t(v.zw, 1u));
}
"#,
    );
    let module = naga::front::wgsl::parse_str(&source).unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
    // Match the explicit wgpu layout: two compute storage buffers in group 0,
    // followed by one compute uniform in group 1. No dynamic arrays, vertex
    // pulling, overrides or workgroup allocations are used by this probe.
    let mut resources = naga::back::msl::EntryPointResources::default();
    for (group, binding, slot, mutable) in [(0, 0, 0, false), (0, 1, 1, true), (1, 0, 2, false)] {
        resources.resources.insert(
            naga::ResourceBinding { group, binding },
            naga::back::msl::BindTarget {
                buffer: Some(slot),
                mutable,
                ..Default::default()
            },
        );
    }
    let mut options = naga::back::msl::Options {
        lang_version: (2, 3),
        fake_missing_bindings: false,
        ..Default::default()
    };
    options
        .per_entry_point_map
        .insert("precision_probe".into(), resources);
    let (msl, translated) = naga::back::msl::write_string(
        &module,
        &info,
        &options,
        &naga::back::msl::PipelineOptions {
            entry_point: Some((naga::ShaderStage::Compute, "precision_probe".into())),
            ..Default::default()
        },
    )
    .unwrap();
    let entry = translated.entry_point_names[0].as_ref().unwrap();
    fs::write(out.join("probe.wgsl"), source).unwrap();
    fs::write(out.join("probe.metal"), msl).unwrap();
    fs::write(out.join("entry.txt"), entry).unwrap();
    println!("Generated validated WGSL/MSL from production picker helpers; entry={entry}");
}

fn split(value: f64) -> [f32; 2] {
    let hi = value as f32;
    [hi, (value - f64::from(hi)) as f32]
}

struct Fixture {
    name: String,
    inputs: Vec<[f32; 4]>,
    params: Vec<f32>,
    expected: Vec<[f64; 4]>,
}

fn fixtures() -> Vec<Fixture> {
    let mut all = Vec::new();
    for epoch in [0.0, 1_700_000_000_000.0, -1_700_000_000_000.0] {
        for reversed in [false, true] {
            for (width, height, ox, oy) in [(100.0, 100.0, 0.0, 0.0), (3840.0, 2160.0, 13.0, 17.0)]
            {
                let min = split(epoch);
                let max = split(epoch + 1.0);
                // Exact ABI of PickQueryParams: 112-byte transform + seven vec4s.
                let mut params = vec![0.0_f32; 56];
                params[0] = min[0];
                params[2] = max[0];
                params[3] = 1.0;
                params[4] = min[1];
                params[6] = max[1];
                params[12] = if reversed { -1.0 } else { 1.0 };
                params[13] = 1.0;
                params[14] = if reversed { 1.0 } else { 0.0 };
                params[30] = ox as f32;
                params[31] = oy as f32;
                params[32] = width as f32;
                params[33] = height as f32;
                let mut inputs = Vec::new();
                let mut expected = Vec::new();
                for i in 0..COUNT {
                    let x = (i as f64 + 0.5) / COUNT as f64;
                    let y = 1.0 - x;
                    let xp = split(epoch + x);
                    let yp = split(y);
                    // Independent f64 oracle, never obtained from another GPU path.
                    let tx = if reversed { 1.0 - x } else { x };
                    inputs.push([xp[0], xp[1], yp[0], yp[1]]);
                    expected.push([ox + tx * width, oy + (1.0 - y) * height, tx, y]);
                }
                all.push(Fixture {
                    name: format!("epoch={epoch},reversed={reversed},viewport={width}x{height}"),
                    inputs,
                    params,
                    expected,
                });
            }
        }
    }
    all
}

fn buffer_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn evaluate(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    module: &wgpu::ShaderModule,
    entry: &str,
    name: &str,
) -> serde_json::Value {
    let data_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("probe data"),
        entries: &[
            buffer_entry(0, wgpu::BufferBindingType::Storage { read_only: true }),
            buffer_entry(1, wgpu::BufferBindingType::Storage { read_only: false }),
        ],
    });
    let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("probe params"),
        entries: &[buffer_entry(0, wgpu::BufferBindingType::Uniform)],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("probe explicit layout"),
        bind_group_layouts: &[Some(&data_layout), Some(&params_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(name),
        layout: Some(&layout),
        module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut records = Vec::new();
    let mut failures = 0;
    for fixture in fixtures() {
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("probe inputs"),
            contents: bytemuck::cast_slice(&fixture.inputs),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("probe uniform"),
            contents: bytemuck::cast_slice(&fixture.params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let size = (COUNT * 16) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe output"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let data = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &data_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let uniform = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &params_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &data, &[]);
            pass.set_bind_group(1, &uniform, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
        let submitted = queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let actual: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        let mut max_error = 0.0_f64;
        let mut bad = 0;
        for (got, expected) in actual.iter().zip(&fixture.expected) {
            let mut valid = true;
            for axis in 0..4 {
                // Convert normalized-coordinate error to the same pixel unit.
                let multiplier = match axis {
                    2 => fixture.params[32] as f64,
                    3 => fixture.params[33] as f64,
                    _ => 1.0,
                };
                let error = (got[axis] as f64 - expected[axis]).abs() * multiplier;
                if !error.is_finite() || error > MAX_PIXEL_ERROR {
                    valid = false;
                }
                max_error = max_error.max(error);
            }
            if !valid {
                bad += 1;
            }
        }
        let record = serde_json::json!({"fixture":fixture.name, "points":COUNT, "bad_points":bad, "max_pixel_error":max_error, "first_actual":actual[0], "first_expected":fixture.expected[0]});
        println!("{name}: {record}");
        records.push(record);
        failures += bad;
        drop(mapped);
        readback.unmap();
    }
    serde_json::json!({"variant":name, "bad_points":failures, "fixtures":records})
}

async fn run(out: &Path, metal: bool) {
    let backend = if metal {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: backend,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .expect("required native adapter missing");
    let features = if metal {
        wgpu::Features::PASSTHROUGH_SHADERS
    } else {
        wgpu::Features::empty()
    };
    assert!(
        adapter.features().contains(features),
        "required passthrough feature missing"
    );
    let info = adapter.get_info();
    println!("Actual adapter: {info:?}");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("Metal precision experiment"),
            required_features: features,
            ..Default::default()
        })
        .await
        .unwrap();
    let source = fs::read_to_string(out.join("probe.wgsl")).unwrap();
    let native = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("unmodified WGSL baseline"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let mut results = vec![evaluate(
        &device,
        &queue,
        &native,
        "precision_probe",
        "wgsl-default",
    )];
    if metal {
        let entry = fs::read_to_string(out.join("entry.txt")).unwrap();
        for variant in ["fast", "precise"] {
            let bytes = fs::read(out.join(format!("{variant}.metallib"))).unwrap();
            // SAFETY: locally generated validated shader, fixed-size 64-element
            // buffers, exactly 64 invocations, explicit verified resource slots.
            // Never accepts arbitrary user shader input. No workgroup memory or
            // runtime-sized arrays depend on absent passthrough reflection.
            let module = unsafe {
                device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                    label: Some(variant),
                    metallib: Some(Cow::Owned(bytes)),
                    entry_points: Cow::Owned(vec![wgpu::PassthroughShaderEntryPoint {
                        name: Cow::Borrowed(entry.trim()),
                        workgroup_size: (64, 1, 1),
                    }]),
                    ..Default::default()
                })
            };
            results.push(evaluate(&device, &queue, &module, entry.trim(), variant));
        }
    }
    let passed = results.last().unwrap()["bad_points"].as_u64().unwrap() == 0;
    let report = serde_json::json!({"scope":"production picker coordinate helpers only; not the full picker or renderer", "backend":format!("{:?}",info.backend), "adapter":info.name, "driver":info.driver, "driver_info":info.driver_info, "pixel_error_limit":MAX_PIXEL_ERROR, "precise_path_tested":metal, "required_variant_passed":passed, "results":results});
    fs::write(
        out.join(if metal {
            "metal-results.json"
        } else {
            "vulkan-control.json"
        }),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(
        passed,
        "required precision variant failed; see JSON evidence"
    );
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let mode = args
        .get(1)
        .expect("usage: generate OUT ROOT | metal OUT | vulkan OUT");
    let out = Path::new(args.get(2).expect("output directory required"));
    match mode.as_str() {
        "generate" => generate(
            out,
            Path::new(args.get(3).expect("repository root required")),
        ),
        "metal" => pollster::block_on(run(out, true)),
        "vulkan" => pollster::block_on(run(out, false)),
        _ => panic!("unknown mode"),
    }
}
