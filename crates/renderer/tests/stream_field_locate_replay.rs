//! Test-only P-00 field gate. The resident `locate` is the oracle; the candidate
//! keeps only per-pixel GPU state and a one/two-pair GPU coordinate ticket.
//! Full-axis oracle storage and readback exist only inside this test.
#![cfg(not(target_arch = "wasm32"))]

use wgpu::util::DeviceExt;

const WIDTH: u32 = 37;
const HEIGHT: u32 = 23;
const PANEL: (u32, u32, u32, u32) = (3, 2, 31, 19);
const SWEEPS: usize = 12;

const ENTRIES: &str = r#"
fn pack_bits(bits: u32) -> vec4<f32> {
    return vec4<f32>(vec4<u32>(bits & 255u, (bits >> 8u) & 255u,
        (bits >> 16u) & 255u, (bits >> 24u) & 255u)) / 255.0;
}
struct PackedHit { @location(0) identity: vec4<f32>, @location(1) frac: vec4<f32> };
// One test-only exact equality query. Raster interpolation need not put the
// mathematical centre pixel at the exact f32 bit-pattern 0.5.
fn probe_t(in: FieldOut) -> f32 {
    if (u32(in.pos.x) == 18u && u32(in.pos.y) == 11u) { return 0.5; }
    return in.axis_t.x;
}
@fragment fn fs_locate_oracle(in: FieldOut) -> PackedHit {
    let count = quad_count(field.cols, LATTICE_QUADS);
    let hit = locate(field.x_base, field.x_len, count, 0u, probe_t(in), LATTICE_QUADS);
    let identity = hit.index | select(0u, 0x80000000u, hit.hit);
    return PackedHit(pack_bits(identity), pack_bits(bitcast<u32>(hit.frac)));
}

struct ReplayState {
    t_bits: u32, first_bits: u32, a_bits: u32, lo: u32,
    hi: u32, phase: u32, step: u32, ascending: u32,
    index: u32, frac_bits: u32, hit: u32, pad: u32,
};
struct ReplayTicket { start: u32, len: u32, n: u32, count: u32 };
@group(3) @binding(0) var<storage, read_write> states: array<ReplayState>;
@group(3) @binding(1) var<storage, read> coordinate_chunk: array<f32>;
@group(3) @binding(2) var<uniform> ticket: ReplayTicket;
@group(3) @binding(3) var<storage, read_write> replay_output: array<vec4<u32>>;

@fragment fn fs_replay_init(in: FieldOut) -> @location(0) vec4<f32> {
    let x = u32(in.pos.x);
    let y = u32(in.pos.y);
    let p = y * 37u + x;
    var s: ReplayState;
    s.t_bits = bitcast<u32>(probe_t(in));
    s.first_bits = 0u;
    s.a_bits = 0u;
    s.lo = 0u;
    s.hi = quad_count(field.cols, LATTICE_QUADS);
    s.phase = 0u;
    s.step = 0u;
    s.ascending = 0u;
    s.index = 0u;
    s.frac_bits = 0u;
    s.hit = 0u;
    s.pad = 0u;
    states[p] = s;
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}

fn chunk_has(k: u32) -> bool {
    return k >= ticket.start && k - ticket.start < ticket.len;
}
fn chunk_pair(k: u32) -> vec2<f32> {
    let j = (k - ticket.start) * 2u;
    return vec2<f32>(coordinate_chunk[j], coordinate_chunk[j + 1u]);
}
struct ReplayBoundary { available: bool, value: f32 };
// The exact resident pair arithmetic, but values come from a bounded ticket.
fn replay_boundary(k: u32) -> ReplayBoundary {
    var out: ReplayBoundary;
    out.available = false;
    out.value = 0.0;
    var pair = vec2<f32>(0.0, 0.0);
    if (lattice_is_samples(LATTICE_QUADS)) {
        if (field_flag(FIELD_CENTERS)) {
            if (!chunk_has(k)) { return out; }
            pair = chunk_pair(k);
        } else {
            let next = min(k + 1u, ticket.n - 1u);
            if (!chunk_has(k) || !chunk_has(next)) { return out; }
            pair = midpoint_grid_pair(chunk_pair(k), chunk_pair(next));
        }
    } else if (!field_flag(FIELD_CENTERS)) {
        if (!chunk_has(k)) { return out; }
        pair = chunk_pair(k);
    } else if (k == 0u) {
        let next = min(1u, ticket.n - 1u);
        if (!chunk_has(0u) || !chunk_has(next)) { return out; }
        let c0 = chunk_pair(0u);
        let c1 = chunk_pair(next);
        pair = add_grid_pairs(c0, scale_grid_pair(subtract_grid_pairs(c0, c1), 0.5));
    } else if (k >= ticket.n) {
        let last = ticket.n - 1u;
        let prev = max(ticket.n, 2u) - 2u;
        if (!chunk_has(last) || !chunk_has(prev)) { return out; }
        pair = add_grid_pairs(chunk_pair(last),
            scale_grid_pair(subtract_grid_pairs(chunk_pair(last), chunk_pair(prev)), 0.5));
    } else {
        if (!chunk_has(k - 1u) || !chunk_has(k)) { return out; }
        pair = midpoint_grid_pair(chunk_pair(k - 1u), chunk_pair(k));
    }
    out.available = true;
    out.value = axis_t(pair, 0u);
    return out;
}

@compute @workgroup_size(8, 8, 1)
fn cs_replay(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= 31u || id.y >= 19u) { return; }
    let p = (id.y + 2u) * 37u + (id.x + 3u);
    var s = states[p];
    if (s.phase >= 5u) { return; }
    let t = bitcast<f32>(s.t_bits);
    if (s.phase == 0u &&
        (ticket.count == 0u || ticket.n == 0u || !f32_is_finite(t))) {
        s.phase = 6u;
        states[p] = s;
        return;
    }
    if (s.phase == 2u && (s.hi - s.lo <= 1u || s.step == 32u)) {
        s.phase = 3u;
    }
    var k = 0u;
    switch s.phase {
        case 0u: { k = 0u; }
        case 1u: { k = ticket.count; }
        case 2u: { k = s.lo + (s.hi - s.lo) / 2u; }
        case 3u: { k = s.lo; }
        case 4u: { k = s.lo + 1u; }
        default: { states[p] = s; return; }
    }
    let boundary = replay_boundary(k);
    if (!boundary.available) { states[p] = s; return; }
    let v = boundary.value;
    if (!f32_is_finite(v)) { s.phase = 6u; states[p] = s; return; }
    switch s.phase {
        case 0u: { s.first_bits = bitcast<u32>(v); s.phase = 1u; }
        case 1u: {
            let first = bitcast<f32>(s.first_bits);
            if (t < min(first, v) || t > max(first, v)) {
                s.phase = 6u;
            } else {
                s.ascending = select(0u, 1u, v >= first);
                s.phase = 2u;
            }
        }
        case 2u: {
            // Test-only trace: distinct screen pixels can demand different
            // global mids, regardless of the current I/O ticket size.
            s.pad = s.pad | (1u << k);
            let before = select(v > t, v <= t, s.ascending != 0u);
            if (before) { s.lo = k; } else { s.hi = k; }
            s.step = s.step + 1u;
        }
        case 3u: { s.a_bits = bitcast<u32>(v); s.phase = 4u; }
        case 4u: {
            let a = bitcast<f32>(s.a_bits);
            let span = v - a;
            if (!f32_is_finite(span)) {
                s.phase = 6u;
            } else {
                var frac = 0.0;
                if (span != 0.0) {
                    frac = (t - a) / span;
                    if (!f32_is_finite(frac)) {
                        s.phase = 6u;
                        states[p] = s;
                        return;
                    }
                }
                s.index = s.lo;
                s.frac_bits = bitcast<u32>(clamp(frac, 0.0, 1.0));
                s.hit = 1u;
                s.phase = 5u;
            }
        }
        default: {}
    }
    states[p] = s;
}

@compute @workgroup_size(8, 8, 1)
fn cs_pack_replay(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= 31u || id.y >= 19u) { return; }
    let p = (id.y + 2u) * 37u + (id.x + 3u);
    let s = states[p];
    replay_output[p] = vec4<u32>(s.index | select(0u, 0x80000000u, s.hit != 0u),
        s.frac_bits, s.phase, s.pad);
}
"#;

#[derive(Clone, Copy)]
struct Fixture {
    name: &'static str,
    layout_centers: bool,
    interpolated: bool,
    descending: bool,
    log_axis: bool,
    values: [f32; 5],
}

fn fixtures() -> Vec<Fixture> {
    let mut result = Vec::new();
    for centers in [false, true] {
        for interpolated in [false, true] {
            for descending in [false, true] {
                result.push(Fixture {
                    name: "regular",
                    layout_centers: centers,
                    interpolated,
                    descending,
                    log_axis: false,
                    values: [0.0, 1.0, 2.0, 3.0, 4.0],
                });
            }
        }
    }
    result.extend([
        Fixture {
            name: "duplicate",
            layout_centers: false,
            interpolated: false,
            descending: false,
            log_axis: false,
            values: [0.0, 2.0, 2.0, 3.0, 4.0],
        },
        Fixture {
            name: "duplicate-2pair",
            layout_centers: false,
            interpolated: false,
            descending: false,
            log_axis: false,
            values: [0.0, 2.0, 2.0, 3.0, 4.0],
        },
        Fixture {
            name: "log",
            layout_centers: true,
            interpolated: true,
            descending: true,
            log_axis: true,
            values: [1.0, 2.0, 4.0, 8.0, 16.0],
        },
        Fixture {
            name: "nan-midpoint",
            layout_centers: false,
            interpolated: false,
            descending: false,
            log_axis: false,
            values: [0.0, 1.0, f32::NAN, 3.0, 4.0],
        },
    ]);
    result
}

fn buffer_init(
    device: &wgpu::Device,
    label: &'static str,
    data: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: data,
        usage,
    })
}

fn buffer(
    device: &wgpu::Device,
    label: &'static str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

fn texture(device: &wgpu::Device, samples: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("field locate oracle target"),
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
    targets: usize,
) -> wgpu::RenderPipeline {
    let color = Some(wgpu::ColorTargetState {
        format: wgpu::TextureFormat::Rgba8Unorm,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    });
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
            targets: &vec![color; targets],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn bind_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    entries: &[(u32, &wgpu::Buffer)],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("field locate test binding"),
        layout,
        entries: &entries
            .iter()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>(),
    })
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    pipe: &wgpu::RenderPipeline,
    views: &[&wgpu::TextureView],
    groups: &[(u32, &wgpu::BindGroup)],
) {
    let attachments: Vec<_> = views
        .iter()
        .map(|view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        })
        .collect();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("field locate test draw"),
        color_attachments: &attachments,
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
    pass.set_scissor_rect(PANEL.0, PANEL.1, PANEL.2, PANEL.3);
    pass.set_pipeline(pipe);
    for (index, group) in groups {
        pass.set_bind_group(*index, *group, &[]);
    }
    pass.draw(0..6, 0..1);
}

fn extract_oracle(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    textures: (&wgpu::Texture, &wgpu::Texture),
    samples: u32,
) -> wgpu::Buffer {
    let kind = if samples == 1 {
        "texture_2d<f32>"
    } else {
        "texture_multisampled_2d<f32>"
    };
    let sample = if samples == 1 { "0" } else { "i32(id.z)" };
    let shader = format!(
        r#"
@group(0) @binding(0) var identity: {kind};
@group(0) @binding(1) var frac: {kind};
@group(0) @binding(2) var<storage, read_write> out: array<vec4<u32>>;
fn unpack(v: vec4<f32>) -> u32 {{
    let b = vec4<u32>(round(v * 255.0));
    return b.x | (b.y << 8u) | (b.z << 16u) | (b.w << 24u);
}}
@compute @workgroup_size(8, 8, 1) fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x < {WIDTH}u && id.y < {HEIGHT}u && id.z < {samples}u) {{
        out[(id.z * {HEIGHT}u + id.y) * {WIDTH}u + id.x] = vec4<u32>(
            unpack(textureLoad(identity, vec2<i32>(id.xy), {sample})),
            unpack(textureLoad(frac, vec2<i32>(id.xy), {sample})), 0u, 0u);
    }}
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("field locate oracle extraction"),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("field locate oracle extraction"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let size = u64::from(WIDTH * HEIGHT * samples * 16);
    let output = buffer(
        device,
        "field oracle extraction",
        size,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let readback = buffer(
        device,
        "field oracle readback",
        size,
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    );
    let identity = textures.0.create_view(&Default::default());
    let frac = textures.1.create_view(&Default::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("field oracle extraction binding"),
        layout: &pipe.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&identity),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&frac),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output.as_entire_binding(),
            },
        ],
    });
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipe);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), samples);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
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
        .expect("field locate GPU poll failed");
    rx.recv_timeout(std::time::Duration::from_secs(30))
        .expect("field locate map callback missing")
        .expect("field locate map failed");
    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .expect("field locate mapped range failed");
    let result = mapped
        .chunks_exact(16)
        .map(|row| {
            std::array::from_fn(|lane| {
                u32::from_le_bytes(row[lane * 4..lane * 4 + 4].try_into().unwrap())
            })
        })
        .collect();
    drop(mapped);
    buffer.unmap();
    result
}

fn at(rows: &[[u32; 4]], x: u32, y: u32, sample: u32) -> [u32; 4] {
    rows[((sample * HEIGHT + y) * WIDTH + x) as usize]
}

#[test]
fn bounded_gpu_axis_replay_matches_resident_global_locate() {
    let instance = renderer::data_render::create_instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("field locate P-00 requires a GPU adapter; no skip");
    eprintln!("field locate P-00 adapter: {:?}", adapter.get_info());
    let features = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm);
    assert!(
        features.allowed_usages.contains(
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
        ) && features.flags.sample_count_supported(1)
            && features.flags.sample_count_supported(4),
        "field locate 1x/4x probe unsupported: {features:?}"
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
        .expect("field locate P-00 device creation failed");
    let source = format!(
        "{}\n{}",
        include_str!("../src/data_render/field_columnar.wgsl"),
        ENTRIES
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("resident field plus test-only replay"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let oracle_pipes = [
        pipeline(&device, &module, "fs_locate_oracle", 1, 2),
        pipeline(&device, &module, "fs_locate_oracle", 4, 2),
    ];
    let init_pipe = pipeline(&device, &module, "fs_replay_init", 1, 1);
    let replay_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("bounded field coordinate replay"),
        layout: None,
        module: &module,
        entry_point: Some("cs_replay"),
        compilation_options: Default::default(),
        cache: None,
    });
    let pack_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("field replay result packing"),
        layout: None,
        module: &module,
        entry_point: Some("cs_pack_replay"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut total = 0usize;
    let mut nan_misses = 0usize;
    let mut duplicate_ties = 0usize;
    let mut descending_ties = 0usize;
    let mut duplicate_one_pair_result: Option<Vec<[u32; 4]>> = None;
    for fixture in fixtures() {
        let n = fixture.values.len() as u32;
        let cells = if fixture.layout_centers { n } else { n - 1 };
        let count = if fixture.interpolated {
            cells - 1
        } else {
            cells
        };
        let flags = u32::from(fixture.layout_centers) * renderer::data_render::FIELD_FLAG_CENTERS
            | u32::from(fixture.interpolated) * renderer::data_render::FIELD_FLAG_INTERPOLATED;
        let params = renderer::data_render::FieldParamsGpu {
            x_base: 0,
            y_base: 0,
            x_len: n,
            y_len: n,
            cols: cells,
            rows: 1,
            level_count: 0,
            stop_count: 0,
            flags,
            opacity: 1.0,
            line_width_px: 0.0,
            level_color_count: 0,
            z_min: [0.0; 2],
            z_max: [1.0, 0.0],
        };
        let upper = if fixture.log_axis {
            16.0f32.log10()
        } else {
            4.0
        };
        let transform = renderer::data_render::ScatterTransform {
            data_min: [0.0, 0.0],
            data_max: [upper, 1.0],
            data_min_lo: [0.0; 2],
            data_max_lo: [0.0; 2],
            scale_log: [if fixture.log_axis { 1.0 } else { 0.0 }, 0.0],
            pixel_to_ndc: [2.0 / PANEL.2 as f32, 2.0 / PANEL.3 as f32],
            data_to_panel_scale: [if fixture.descending { -1.0 } else { 1.0 }, 1.0],
            data_to_panel_offset: [if fixture.descending { 1.0 } else { 0.0 }, 0.0],
            style_params: [[0.0; 4]; 3],
        };
        let pairs: Vec<[f32; 2]> = fixture.values.iter().map(|value| [*value, 0.0]).collect();
        let full_pool = buffer_init(
            &device,
            "test-only full-axis oracle",
            bytemuck::cast_slice(&pairs),
            wgpu::BufferUsages::STORAGE,
        );
        let params_buffer = buffer_init(
            &device,
            "field replay params",
            bytemuck::bytes_of(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let transform_buffer = buffer_init(
            &device,
            "field replay transform",
            bytemuck::bytes_of(&transform),
            wgpu::BufferUsages::UNIFORM,
        );
        let state = buffer(
            &device,
            "bounded field pixel state",
            u64::from(WIDTH * HEIGHT * 48),
            wgpu::BufferUsages::STORAGE,
        );
        let chunk = buffer(
            &device,
            "bounded one/two-pair ticket",
            16,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let ticket_buffer = buffer(
            &device,
            "bounded ticket metadata",
            16,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let replay_result = buffer(
            &device,
            "field replay GPU result",
            u64::from(WIDTH * HEIGHT * 16),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let replay_readback = buffer(
            &device,
            "field replay test-only readback",
            u64::from(WIDTH * HEIGHT * 16),
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let init_group_2 = bind_buffer(
            &device,
            &init_pipe.get_bind_group_layout(2),
            &[(4, &params_buffer)],
        );
        let init_group_3 =
            bind_buffer(&device, &init_pipe.get_bind_group_layout(3), &[(0, &state)]);
        let replay_group_0 = bind_buffer(
            &device,
            &replay_pipe.get_bind_group_layout(0),
            &[(0, &transform_buffer)],
        );
        let replay_group_2 = bind_buffer(
            &device,
            &replay_pipe.get_bind_group_layout(2),
            &[(4, &params_buffer)],
        );
        let replay_group_3 = bind_buffer(
            &device,
            &replay_pipe.get_bind_group_layout(3),
            &[(0, &state), (1, &chunk), (2, &ticket_buffer)],
        );
        let pack_group_3 = bind_buffer(
            &device,
            &pack_pipe.get_bind_group_layout(3),
            &[(0, &state), (3, &replay_result)],
        );
        let init_target = texture(&device, 1);
        let init_view = init_target.create_view(&Default::default());
        let mut begin = device.create_command_encoder(&Default::default());
        draw(
            &mut begin,
            &init_pipe,
            &[&init_view],
            &[(2, &init_group_2), (3, &init_group_3)],
        );
        queue.submit([begin.finish()]);
        let ticket_len = if fixture.name == "duplicate-2pair"
            || fixture.layout_centers != fixture.interpolated
        {
            2
        } else {
            1
        };
        // Each sweep asks only one/two logical pairs at a time. No GPU state is
        // read by the host to choose these ranges; a full sequential replay is
        // deliberately used for sources without a range inclusion proof.
        for _ in 0..SWEEPS {
            for start in 0..n {
                if ticket_len == 2 && start + 1 == n {
                    break;
                }
                let len = ticket_len.min(n - start);
                let ticket = [start, len, n, count];
                queue.write_buffer(&ticket_buffer, 0, bytemuck::cast_slice(&ticket));
                queue.write_buffer(
                    &chunk,
                    0,
                    bytemuck::cast_slice(&pairs[start as usize..(start + len) as usize]),
                );
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&replay_pipe);
                    pass.set_bind_group(0, &replay_group_0, &[]);
                    pass.set_bind_group(2, &replay_group_2, &[]);
                    pass.set_bind_group(3, &replay_group_3, &[]);
                    pass.dispatch_workgroups(PANEL.2.div_ceil(8), PANEL.3.div_ceil(8), 1);
                }
                queue.submit([encoder.finish()]);
            }
        }
        let mut finish = device.create_command_encoder(&Default::default());
        {
            let mut pass = finish.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pack_pipe);
            pass.set_bind_group(3, &pack_group_3, &[]);
            pass.dispatch_workgroups(PANEL.2.div_ceil(8), PANEL.3.div_ceil(8), 1);
        }
        finish.copy_buffer_to_buffer(
            &replay_result,
            0,
            &replay_readback,
            0,
            u64::from(WIDTH * HEIGHT * 16),
        );
        let mut oracle_reads = Vec::new();
        for (index, samples) in [1, 4].into_iter().enumerate() {
            let identity = texture(&device, samples);
            let frac = texture(&device, samples);
            let views = (
                identity.create_view(&Default::default()),
                frac.create_view(&Default::default()),
            );
            let pipe = &oracle_pipes[index];
            let group_0 = bind_buffer(
                &device,
                &pipe.get_bind_group_layout(0),
                &[(0, &transform_buffer)],
            );
            let group_2 = bind_buffer(
                &device,
                &pipe.get_bind_group_layout(2),
                &[(0, &full_pool), (4, &params_buffer)],
            );
            draw(
                &mut finish,
                pipe,
                &[&views.0, &views.1],
                &[(0, &group_0), (2, &group_2)],
            );
            oracle_reads.push((
                samples,
                extract_oracle(&device, &mut finish, (&identity, &frac), samples),
            ));
        }
        queue.submit([finish.finish()]);
        let replay = read(&device, &replay_readback);
        if fixture.name == "duplicate" || fixture.name == "duplicate-2pair" {
            let middle = at(&replay, PANEL.0 + PANEL.2 / 2, PANEL.1 + PANEL.3 / 2, 0);
            assert_eq!(
                middle[0], 0x80000002,
                "ascending duplicate boundary tie did not choose global cell 2"
            );
            assert_eq!(middle[1], 0.0f32.to_bits());
            duplicate_ties += 1;
        }
        if fixture.name == "duplicate" {
            duplicate_one_pair_result = Some(replay.clone());
        } else if fixture.name == "duplicate-2pair" {
            assert_eq!(
                replay,
                duplicate_one_pair_result
                    .take()
                    .expect("one-pair replay missing"),
                "changing source ticket I/O size changed global locate state"
            );
        }
        if fixture.name == "regular"
            && !fixture.layout_centers
            && !fixture.interpolated
            && fixture.descending
        {
            let middle = at(&replay, PANEL.0 + PANEL.2 / 2, PANEL.1 + PANEL.3 / 2, 0);
            assert_eq!(
                middle[0], 0x80000001,
                "descending boundary tie did not choose global cell 1"
            );
            assert_eq!(middle[1], 1.0f32.to_bits());
            descending_ties += 1;
        }
        let mut fixture_hits = 0usize;
        let mut fixture_misses = 0usize;
        let mut global_mid_union = 0u32;
        for (samples, buffer) in oracle_reads {
            let oracle = read(&device, &buffer);
            for y in PANEL.1..PANEL.1 + PANEL.3 {
                for x in PANEL.0..PANEL.0 + PANEL.2 {
                    let candidate = at(&replay, x, y, 0);
                    global_mid_union |= candidate[3];
                    assert!(
                        candidate[2] == 5 || candidate[2] == 6,
                        "{} incomplete GPU replay at ({x},{y}): phase={}, mid-mask={}",
                        fixture.name,
                        candidate[2],
                        candidate[3]
                    );
                    for sample in 0..samples {
                        let expected = at(&oracle, x, y, sample);
                        assert_eq!(
                            &candidate[..2],
                            &expected[..2],
                            "{} centers={} interpolated={} descending={} log={} at ({x},{y}) {samples}x sample {sample}: replay={candidate:08x?} oracle={expected:08x?}",
                            fixture.name,
                            fixture.layout_centers,
                            fixture.interpolated,
                            fixture.descending,
                            fixture.log_axis
                        );
                        total += 1;
                        if candidate[0] & 0x80000000 != 0 {
                            fixture_hits += 1;
                        } else {
                            fixture_misses += 1;
                        }
                    }
                }
            }
        }
        if fixture.name == "nan-midpoint" {
            assert_eq!(fixture_hits, 0, "NaN midpoint must reject every sample");
            assert_eq!(fixture_misses, (PANEL.2 * PANEL.3 * 5) as usize);
            nan_misses += fixture_misses;
        }
        if fixture.name == "regular"
            && !fixture.layout_centers
            && !fixture.interpolated
            && !fixture.descending
        {
            assert_eq!(
                global_mid_union & 0b1110,
                0b1110,
                "different screen pixels did not request global mids 1, 2, and 3"
            );
        }
        eprintln!(
            "field locate P-00 {} centers={} interpolated={} descending={} log={}: hits={fixture_hits} misses={fixture_misses}",
            fixture.name,
            fixture.layout_centers,
            fixture.interpolated,
            fixture.descending,
            fixture.log_axis
        );
    }
    assert_eq!(total, fixtures().len() * (PANEL.2 * PANEL.3 * 5) as usize);
    assert!(
        nan_misses > 0,
        "NaN midpoint fixture did not expose a locate miss"
    );
    assert_eq!(duplicate_ties, 2);
    assert_eq!(descending_ties, 1);
    eprintln!("field locate P-00 compared {total} 1x/4x samples with bounded GPU axis replay");
}
