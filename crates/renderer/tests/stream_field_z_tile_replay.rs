//! Test-only P-00 Heatmap z/tile gate. `fs_main` is the resident colour oracle.
//! The candidate keeps GPU-only per-sample CellHit/z slots; a host fixture
//! supplies every declared column/row sequentially in one/two hi/lo-pair
//! tickets, without inspecting pixel state or selecting a candidate range.
//! The full grid/pool and readback are test-oracle resources only. This does
//! not implement or claim an integrated streamed Heatmap product path.
#![cfg(not(target_arch = "wasm32"))]

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

const WIDTH: u32 = 37;
const HEIGHT: u32 = 23;
const PANEL: (u32, u32, u32, u32) = (3, 2, 31, 19);
const MAX_SAMPLES: u32 = 4;
const STATE_BYTES: u64 = 64;

const ENTRIES: &str = r#"
struct ZState {
    x_index: u32, y_index: u32, x_frac: f32, y_frac: f32,
    hit: u32, valid_mask: u32,
    z00: vec2<f32>, z10: vec2<f32>, z01: vec2<f32>, z11: vec2<f32>,
    sample_marker: u32,
};
struct ZTicket { column: u32, row: u32, len: u32, samples: u32 };
@group(3) @binding(0) var<storage, read_write> z_states: array<ZState>;
@group(3) @binding(1) var<storage, read> z_chunk: array<f32>;
@group(3) @binding(2) var<uniform> z_ticket: ZTicket;

fn pixel_key(pos: vec4<f32>, sample_index: u32) -> u32 {
    return (sample_index * 23u + u32(pos.y)) * 37u + u32(pos.x);
}

@fragment fn fs_z_init(in: FieldOut, @builtin(sample_index) sample_index: u32)
    -> @location(0) vec4<f32> {
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along = select(field.cols, field.rows, columns_are_y);
    let across = select(field.rows, field.cols, columns_are_y);
    let x = locate(field.x_base, field.x_len, quad_count(along, LATTICE_QUADS),
        0u, in.axis_t.x, LATTICE_QUADS);
    let y = locate(field.y_base, field.y_len, quad_count(across, LATTICE_QUADS),
        1u, in.axis_t.y, LATTICE_QUADS);
    var s: ZState;
    s.x_index = x.index;
    s.y_index = y.index;
    s.x_frac = x.frac;
    s.y_frac = y.frac;
    s.hit = select(0u, 1u, x.hit && y.hit);
    s.valid_mask = 0u;
    s.z00 = vec2<f32>(0.0);
    s.z10 = vec2<f32>(0.0);
    s.z01 = vec2<f32>(0.0);
    s.z11 = vec2<f32>(0.0);
    s.sample_marker = sample_index + 1u;
    z_states[pixel_key(in.pos, sample_index)] = s;
    return vec4<f32>(0.0);
}

// A test-only identity probe: one physical MSAA sample must read its own
// storage slot, not a pixel-frequency value copied to all four samples.
@fragment fn fs_sample_slot_marker(in: FieldOut,
    @builtin(sample_index) sample_index: u32) -> @location(0) vec4<f32> {
    let s = z_states[pixel_key(in.pos, sample_index)];
    return vec4<f32>(f32(s.sample_marker) / 255.0, 0.0, 0.0, 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn cs_z_ticket(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= 31u || id.y >= 19u || id.z >= z_ticket.samples) { return; }
    let x = id.x + 3u;
    let y = id.y + 2u;
    let p = (id.z * 23u + y) * 37u + x;
    var s = z_states[p];
    if (s.hit == 0u) { return; }
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let c = select(s.x_index, s.y_index, columns_are_y);
    let r = select(s.y_index, s.x_index, columns_are_y);
    let need = select(1u, 4u, field_flag(FIELD_INTERPOLATED));
    for (var slot = 0u; slot < need; slot = slot + 1u) {
        let dc = select(0u, 1u, slot == 1u || slot == 3u);
        let dr = select(0u, 1u, slot >= 2u);
        let source_c = c + dc;
        let source_r = r + dr;
        if (source_c != z_ticket.column || source_r < z_ticket.row
            || source_r - z_ticket.row >= z_ticket.len) { continue; }
        let j = (source_r - z_ticket.row) * 2u;
        let value = vec2<f32>(z_chunk[j], z_chunk[j + 1u]);
        switch slot {
            case 0u: { s.z00 = value; }
            case 1u: { s.z10 = value; }
            case 2u: { s.z01 = value; }
            case 3u: { s.z11 = value; }
            default: {}
        }
        s.valid_mask = s.valid_mask | (1u << slot);
    }
    z_states[p] = s;
}

@fragment fn fs_z_final(in: FieldOut, @builtin(sample_index) sample_index: u32)
    -> @location(0) vec4<f32> {
    let clear = vec4<f32>(0.0);
    let s = z_states[pixel_key(in.pos, sample_index)];
    if (s.hit == 0u) { return clear; }
    let interpolated = field_flag(FIELD_INTERPOLATED);
    let need = select(1u, 15u, interpolated);
    if ((s.valid_mask & need) != need) { return style.color_premul; }
    var z = s.z00;
    if (interpolated) {
        if (!vec2_f32_is_finite(s.z00) || !vec2_f32_is_finite(s.z10)
            || !vec2_f32_is_finite(s.z01) || !vec2_f32_is_finite(s.z11)) {
            return style.color_premul;
        }
        let fc = select(s.x_frac, s.y_frac, field_flag(FIELD_COLUMNS_ARE_Y));
        let fr = select(s.y_frac, s.x_frac, field_flag(FIELD_COLUMNS_ARE_Y));
        let low = s.z00 + (s.z10 - s.z00) * fc;
        let high = s.z01 + (s.z11 - s.z01) * fc;
        if (!vec2_f32_is_finite(low) || !vec2_f32_is_finite(high)) {
            return style.color_premul;
        }
        z = low + (high - low) * fr;
        if (!vec2_f32_is_finite(z)) { return style.color_premul; }
    }
    let t = z_ramp_t(z);
    if (t < 0.0) { return style.color_premul; }
    let position = select(t, band_t(z.x + z.y), field_flag(FIELD_BANDS));
    let color = ramp(position);
    let alpha = color.a * field.opacity;
    return vec4<f32>(color.rgb * alpha, alpha);
}
"#;

#[derive(Clone, Copy)]
struct Fixture {
    centers: bool,
    interpolated: bool,
    columns_are_y: bool,
    ticket_pairs: u32,
}

fn fixtures() -> Vec<Fixture> {
    let mut cases = Vec::new();
    for centers in [false, true] {
        for interpolated in [false, true] {
            for columns_are_y in [false, true] {
                cases.push(Fixture {
                    centers,
                    interpolated,
                    columns_are_y,
                    ticket_pairs: if centers == interpolated { 1 } else { 2 },
                });
            }
        }
    }
    cases
}

fn init_buffer(
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

fn empty_buffer(
    device: &wgpu::Device,
    label: &str,
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

fn bind_buffers(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    entries: &[(u32, &wgpu::Buffer)],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("field z gate bindings"),
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

fn render_pipeline(
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
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn texture(device: &wgpu::Device, samples: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("field z gate target"),
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

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    pipe: &wgpu::RenderPipeline,
    target: &wgpu::TextureView,
    groups: &[(u32, &wgpu::BindGroup)],
    scissor: (u32, u32, u32, u32),
    clear: bool,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("field z gate draw"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
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
        })],
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
    for (index, group) in groups {
        pass.set_bind_group(*index, *group, &[]);
    }
    pass.draw(0..6, 0..1);
}

fn extract_samples(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::Texture,
    samples: u32,
) -> wgpu::Buffer {
    let texture_kind = if samples == 1 {
        "texture_2d<f32>"
    } else {
        "texture_multisampled_2d<f32>"
    };
    let sample_arg = if samples == 1 { "0" } else { "i32(id.z)" };
    let shader = format!(
        r#"
@group(0) @binding(0) var source: {texture_kind};
@group(0) @binding(1) var<storage, read_write> pixels: array<u32>;
@compute @workgroup_size(8, 8, 1) fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= {WIDTH}u || id.y >= {HEIGHT}u || id.z >= {samples}u) {{ return; }}
    let rgba = vec4<u32>(round(textureLoad(source, vec2<i32>(id.xy), {sample_arg}) * 255.0));
    pixels[(id.z * {HEIGHT}u + id.y) * {WIDTH}u + id.x] =
        rgba.x | (rgba.y << 8u) | (rgba.z << 16u) | (rgba.w << 24u);
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("field z gate sample extraction"),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("field z gate sample extraction"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let gpu = empty_buffer(
        device,
        "field z GPU pixels",
        u64::from(WIDTH * HEIGHT * samples * 4),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let cpu = empty_buffer(
        device,
        "field z test-only readback",
        u64::from(WIDTH * HEIGHT * samples * 4),
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    );
    let view = target.create_view(&Default::default());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("field z gate sample extraction binding"),
        layout: &pipe.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: gpu.as_entire_binding(),
            },
        ],
    });
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipe);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), samples);
    }
    encoder.copy_buffer_to_buffer(&gpu, 0, &cpu, 0, u64::from(WIDTH * HEIGHT * samples * 4));
    cpu
}

fn read_pixels(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u32> {
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).expect("field z map receiver missing");
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("field z GPU poll failed");
    rx.recv_timeout(std::time::Duration::from_secs(30))
        .expect("field z map callback missing")
        .expect("field z readback map failed");
    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .expect("field z mapped range missing");
    let result = mapped
        .chunks_exact(4)
        .map(|row| u32::from_le_bytes(row.try_into().expect("one RGBA pixel")))
        .collect();
    drop(mapped);
    buffer.unmap();
    result
}

fn sample_at(values: &[u32], x: u32, y: u32, sample: u32) -> u32 {
    values[((sample * HEIGHT + y) * WIDTH + x) as usize]
}

fn setup_fixture(
    fixture: Fixture,
) -> (
    renderer::data_render::ScatterTransform,
    renderer::data_render::PrimitiveStyle,
    renderer::data_render::FieldParamsGpu,
    Vec<[f32; 2]>,
    Vec<renderer::data_render::GridColumnGpu>,
    Vec<Vec<[f32; 2]>>,
) {
    let x_coords = [0.0f32, 1.0, 2.0, 3.0, 4.0];
    let y_coords = [0.0f32, 1.0, 2.0, 3.0];
    let x_cells = if fixture.centers {
        x_coords.len() as u32
    } else {
        x_coords.len() as u32 - 1
    };
    let y_cells = if fixture.centers {
        y_coords.len() as u32
    } else {
        y_coords.len() as u32 - 1
    };
    let cols = if fixture.columns_are_y {
        y_cells
    } else {
        x_cells
    };
    let rows = if fixture.columns_are_y {
        x_cells
    } else {
        y_cells
    };
    let mut pool = x_coords.iter().map(|&x| [x, 0.0]).collect::<Vec<_>>();
    let y_base = pool.len() as u32 * 2;
    pool.extend(y_coords.iter().map(|&y| [y, 0.0]));
    let mut grid = Vec::new();
    let mut columns = Vec::new();
    for c in 0..cols {
        let len = if c + 1 == cols { rows - 1 } else { rows };
        let mut source = Vec::new();
        for r in 0..len {
            let z = 0.1 + c as f32 * 0.17 + r as f32 * 0.09;
            // Every finite hi lane is identical; the low lane alone carries
            // the data. This is a valid f64 split around a large base where
            // one f32 ULP is much wider than the grid's variation.
            source.push([
                if c == 1 && r == 1 {
                    f32::NAN
                } else {
                    1_000_000_000.0
                },
                if c == 1 && r == 1 { 0.0 } else { z },
            ]);
        }
        grid.push(renderer::data_render::GridColumnGpu {
            base: pool.len() as u32 * 2,
            len,
        });
        pool.extend_from_slice(&source);
        columns.push(source);
    }
    let transform = renderer::data_render::ScatterTransform {
        data_min: [0.0, 0.0],
        data_max: [4.0, 3.0],
        data_min_lo: [0.0; 2],
        data_max_lo: [0.0; 2],
        scale_log: [0.0; 2],
        pixel_to_ndc: [2.0 / PANEL.2 as f32, 2.0 / PANEL.3 as f32],
        data_to_panel_scale: [1.0; 2],
        data_to_panel_offset: [0.0; 2],
        style_params: [[0.0; 4]; 3],
    };
    let mut style = renderer::data_render::PrimitiveStyle::zeroed();
    style.color_premul = [0.08, 0.13, 0.19, 0.45];
    let flags = u32::from(fixture.columns_are_y) * renderer::data_render::FIELD_FLAG_COLUMNS_ARE_Y
        | u32::from(fixture.centers) * renderer::data_render::FIELD_FLAG_CENTERS
        | u32::from(fixture.interpolated) * renderer::data_render::FIELD_FLAG_INTERPOLATED;
    let params = renderer::data_render::FieldParamsGpu {
        x_base: 0,
        y_base,
        x_len: x_coords.len() as u32,
        y_len: y_coords.len() as u32,
        cols,
        rows,
        level_count: 0,
        stop_count: 3,
        flags,
        opacity: 0.65,
        line_width_px: 0.0,
        level_color_count: 0,
        z_min: [1_000_000_000.0, 0.0],
        z_max: [1_000_000_000.0, 1.0],
    };
    (transform, style, params, pool, grid, columns)
}

#[test]
fn bounded_gpu_heatmap_z_tickets_match_resident_colour_samples_and_tile_seam() {
    let instance = renderer::data_render::create_instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("field z P-00 requires a native GPU adapter; no skip");
    eprintln!("field z P-00 adapter: {:?}", adapter.get_info());
    let support = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm);
    assert!(
        support.allowed_usages.contains(
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
        ) && support.flags.sample_count_supported(1)
            && support.flags.sample_count_supported(4),
        "field z P-00 1x/4x Rgba8Unorm unsupported: {support:?}"
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
        .expect("field z P-00 device creation failed");
    let source = format!(
        "{}\n{}",
        include_str!("../src/data_render/field_columnar.wgsl"),
        ENTRIES
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("resident field shader plus test-only bounded z replay"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let ticket_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("field z bounded ticket replay"),
        layout: None,
        module: &module,
        entry_point: Some("cs_z_ticket"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut compared = 0usize;
    let mut finite_colors = 0usize;
    let mut missing_colors = 0usize;
    for fixture in fixtures() {
        let (transform, style, params, full_pairs, grid, columns) = setup_fixture(fixture);
        let transform_buf = init_buffer(
            &device,
            "field z transform",
            bytemuck::bytes_of(&transform),
            wgpu::BufferUsages::UNIFORM,
        );
        let style_buf = init_buffer(
            &device,
            "field z nan style",
            bytemuck::bytes_of(&style),
            wgpu::BufferUsages::UNIFORM,
        );
        let params_buf = init_buffer(
            &device,
            "field z params",
            bytemuck::bytes_of(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let full_pool = init_buffer(
            &device,
            "field z test-only full grid oracle",
            bytemuck::cast_slice(&full_pairs),
            wgpu::BufferUsages::STORAGE,
        );
        let axis_end_pairs = (params.y_base / 2 + params.y_len) as usize;
        let axis_only = init_buffer(
            &device,
            "field z candidate resident-axis-only probe",
            bytemuck::cast_slice(&full_pairs[..axis_end_pairs]),
            wgpu::BufferUsages::STORAGE,
        );
        let grid_buf = init_buffer(
            &device,
            "field z test-only grid metadata oracle",
            bytemuck::cast_slice(&grid),
            wgpu::BufferUsages::STORAGE,
        );
        let stops = [
            [0.9f32, 0.1, 0.15, 0.55],
            [0.15, 0.8, 0.2, 0.9],
            [0.1, 0.2, 0.95, 0.3],
        ];
        let stops_buf = init_buffer(
            &device,
            "field z ramp stops",
            bytemuck::cast_slice(&stops),
            wgpu::BufferUsages::STORAGE,
        );
        let contour_metadata = init_buffer(
            &device,
            "field z empty band metadata",
            bytemuck::cast_slice(&[0u32, 0u32]),
            wgpu::BufferUsages::STORAGE,
        );
        let state = empty_buffer(
            &device,
            "field z per-sample GPU state",
            STATE_BYTES * u64::from(WIDTH * HEIGHT * MAX_SAMPLES),
            wgpu::BufferUsages::STORAGE,
        );
        let chunk = empty_buffer(
            &device,
            "field z one/two pair bounded chunk",
            16,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let ticket_buf = empty_buffer(
            &device,
            "field z bounded ticket metadata",
            16,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        for samples in [1, 4] {
            let oracle_pipe = render_pipeline(&device, &module, "fs_main", samples);
            let init_pipe = render_pipeline(&device, &module, "fs_z_init", samples);
            let final_pipe = render_pipeline(&device, &module, "fs_z_final", samples);
            let oracle_g0 = bind_buffers(
                &device,
                &oracle_pipe.get_bind_group_layout(0),
                &[(0, &transform_buf)],
            );
            let oracle_g1 = bind_buffers(
                &device,
                &oracle_pipe.get_bind_group_layout(1),
                &[(0, &style_buf)],
            );
            let oracle_g2 = bind_buffers(
                &device,
                &oracle_pipe.get_bind_group_layout(2),
                &[
                    (0, &full_pool),
                    (1, &grid_buf),
                    (3, &stops_buf),
                    (4, &params_buf),
                    (6, &contour_metadata),
                ],
            );
            let init_g0 = bind_buffers(
                &device,
                &init_pipe.get_bind_group_layout(0),
                &[(0, &transform_buf)],
            );
            let init_g2 = bind_buffers(
                &device,
                &init_pipe.get_bind_group_layout(2),
                &[(0, &axis_only), (4, &params_buf)],
            );
            let init_g3 =
                bind_buffers(&device, &init_pipe.get_bind_group_layout(3), &[(0, &state)]);
            let ticket_g2 = bind_buffers(
                &device,
                &ticket_pipe.get_bind_group_layout(2),
                &[(4, &params_buf)],
            );
            let ticket_g3 = bind_buffers(
                &device,
                &ticket_pipe.get_bind_group_layout(3),
                &[(0, &state), (1, &chunk), (2, &ticket_buf)],
            );
            let final_g1 = bind_buffers(
                &device,
                &final_pipe.get_bind_group_layout(1),
                &[(0, &style_buf)],
            );
            let final_g2 = bind_buffers(
                &device,
                &final_pipe.get_bind_group_layout(2),
                &[(3, &stops_buf), (4, &params_buf), (6, &contour_metadata)],
            );
            let final_g3 = bind_buffers(
                &device,
                &final_pipe.get_bind_group_layout(3),
                &[(0, &state)],
            );
            let init_target = texture(&device, samples);
            let init_view = init_target.create_view(&Default::default());
            let mut begin = device.create_command_encoder(&Default::default());
            draw(
                &mut begin,
                &init_pipe,
                &init_view,
                &[(0, &init_g0), (2, &init_g2), (3, &init_g3)],
                PANEL,
                true,
            );
            queue.submit([begin.finish()]);

            if samples == 4 && !fixture.centers && !fixture.interpolated && !fixture.columns_are_y {
                let marker_pipe = render_pipeline(&device, &module, "fs_sample_slot_marker", 4);
                let marker_g3 = bind_buffers(
                    &device,
                    &marker_pipe.get_bind_group_layout(3),
                    &[(0, &state)],
                );
                let marker_target = texture(&device, 4);
                let marker_view = marker_target.create_view(&Default::default());
                let mut marker_encoder = device.create_command_encoder(&Default::default());
                draw(
                    &mut marker_encoder,
                    &marker_pipe,
                    &marker_view,
                    &[(3, &marker_g3)],
                    PANEL,
                    true,
                );
                let marker_readback =
                    extract_samples(&device, &mut marker_encoder, &marker_target, 4);
                queue.submit([marker_encoder.finish()]);
                let markers = read_pixels(&device, &marker_readback);
                let probe_x = PANEL.0 + PANEL.2 / 2;
                let probe_y = PANEL.1 + PANEL.3 / 2;
                let mut distinct = std::collections::HashSet::new();
                for sample in 0..4 {
                    let actual = sample_at(&markers, probe_x, probe_y, sample);
                    let expected = 0xff00_0000u32 | (sample + 1);
                    assert_eq!(
                        actual, expected,
                        "4x storage slot/sample identity mismatch at ({probe_x},{probe_y}) sample {sample}"
                    );
                    distinct.insert(actual);
                }
                assert_eq!(
                    distinct.len(),
                    4,
                    "4x sample marker probe did not reach four distinct GPU state slots"
                );
            }

            // The ticket order is source order. No host readback, spatial
            // candidate selection, or per-pixel cursor drives this replay.
            for (c, source) in columns.iter().enumerate() {
                for row in (0..source.len()).step_by(fixture.ticket_pairs as usize) {
                    let len = fixture.ticket_pairs.min(source.len() as u32 - row as u32);
                    let ticket = [c as u32, row as u32, len, samples];
                    queue.write_buffer(&ticket_buf, 0, bytemuck::cast_slice(&ticket));
                    queue.write_buffer(
                        &chunk,
                        0,
                        bytemuck::cast_slice(&source[row..row + len as usize]),
                    );
                    let mut encoder = device.create_command_encoder(&Default::default());
                    {
                        let mut pass = encoder.begin_compute_pass(&Default::default());
                        pass.set_pipeline(&ticket_pipe);
                        pass.set_bind_group(2, &ticket_g2, &[]);
                        pass.set_bind_group(3, &ticket_g3, &[]);
                        pass.dispatch_workgroups(PANEL.2.div_ceil(8), PANEL.3.div_ceil(8), samples);
                    }
                    queue.submit([encoder.finish()]);
                }
            }
            let oracle = texture(&device, samples);
            let oracle_view = oracle.create_view(&Default::default());
            let candidate = texture(&device, samples);
            let candidate_view = candidate.create_view(&Default::default());
            let seam_x = PANEL.0 + PANEL.2 / 2;
            let mut finish = device.create_command_encoder(&Default::default());
            draw(
                &mut finish,
                &oracle_pipe,
                &oracle_view,
                &[(0, &oracle_g0), (1, &oracle_g1), (2, &oracle_g2)],
                PANEL,
                true,
            );
            draw(
                &mut finish,
                &final_pipe,
                &candidate_view,
                &[(1, &final_g1), (2, &final_g2), (3, &final_g3)],
                (PANEL.0, PANEL.1, seam_x - PANEL.0, PANEL.3),
                true,
            );
            draw(
                &mut finish,
                &final_pipe,
                &candidate_view,
                &[(1, &final_g1), (2, &final_g2), (3, &final_g3)],
                (seam_x, PANEL.1, PANEL.0 + PANEL.2 - seam_x, PANEL.3),
                false,
            );
            let expected_buf = extract_samples(&device, &mut finish, &oracle, samples);
            let actual_buf = extract_samples(&device, &mut finish, &candidate, samples);
            queue.submit([finish.finish()]);
            let expected = read_pixels(&device, &expected_buf);
            let actual = read_pixels(&device, &actual_buf);
            let nan = style.color_premul.map(|v| (v * 255.0).round() as u32);
            let nan_bits = nan[0] | nan[1] << 8 | nan[2] << 16 | nan[3] << 24;
            let mut fixture_nan = 0usize;
            let mut fixture_finite = 0usize;
            let mut finite_colors_seen = std::collections::HashSet::new();
            for y in PANEL.1..PANEL.1 + PANEL.3 {
                for x in PANEL.0..PANEL.0 + PANEL.2 {
                    for sample in 0..samples {
                        let exp = sample_at(&expected, x, y, sample);
                        let got = sample_at(&actual, x, y, sample);
                        assert_eq!(
                            got,
                            exp,
                            "Heatmap z tile mismatch centers={} interpolated={} columns_are_y={} ticket_pairs={} {samples}x sample {sample} at ({x},{y}): candidate={got:08x}, resident={exp:08x}",
                            fixture.centers,
                            fixture.interpolated,
                            fixture.columns_are_y,
                            fixture.ticket_pairs
                        );
                        compared += 1;
                        if got == nan_bits {
                            fixture_nan += 1;
                        } else if got != 0 {
                            fixture_finite += 1;
                            finite_colors_seen.insert(got);
                        }
                    }
                }
            }
            assert!(
                fixture_nan > 0,
                "NaN/short-row sentinel never painted: centers={} interpolated={} columns_are_y={} {samples}x",
                fixture.centers,
                fixture.interpolated,
                fixture.columns_are_y
            );
            assert!(
                fixture_finite > 0,
                "finite ramp never painted: centers={} interpolated={} columns_are_y={} {samples}x",
                fixture.centers,
                fixture.interpolated,
                fixture.columns_are_y
            );
            assert!(
                finite_colors_seen.len() > 1,
                "the low-only hi/lo z variation did not affect finite ramp pixels"
            );
            missing_colors += fixture_nan;
            finite_colors += fixture_finite;
            eprintln!(
                "field z gate centers={} interpolated={} columns_are_y={} {}x: finite={fixture_finite} nan_or_short={fixture_nan}",
                fixture.centers, fixture.interpolated, fixture.columns_are_y, samples
            );
        }
    }
    assert_eq!(
        compared,
        fixtures().len() * (PANEL.2 * PANEL.3 * 5) as usize
    );
    assert!(finite_colors > 0 && missing_colors > 0);
    eprintln!(
        "field z P-00 matched {compared} exact 1x/4x RGBA samples, including a two-tile alpha seam"
    );
}
