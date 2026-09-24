//! P-00 only: raster axis capture -> bounded global locate -> bounded Z replay
//! -> one alpha-over per completed tile. The candidate never binds resident
//! axes/grid storage. Only the final images are read back; source fixtures and
//! the full oracle pool are test-only. This is not a production executor.
#![cfg(not(target_arch = "wasm32"))]

use renderer;
#[path = "support/stream_field_fixture.rs"]
mod fixture;
use fixture::*;
const STATE_BYTES: u64 = 136;
const SCRATCH_BUDGET: u64 = 16 * 1024;
const TICKET_PAIRS: u32 = 2;
const RESIDENT: &str = include_str!("../src/data_render/field_columnar.wgsl");

const ENTRIES: &str = r#"
struct ReplayAxis {
    t: f32, first: f32, a: f32, lo: u32,
    hi: u32, phase: u32, steps: u32, ascending: u32,
    index: u32, frac: f32, hit: u32, pad: u32,
};
struct ReplayPixel {
    x: ReplayAxis, y: ReplayAxis,
    z: array<vec2<f32>, 4>, valid_mask: u32, sample_marker: u32,
};
struct ReplayTicket {
    start: u32, len: u32, n: u32, count: u32,
    axis: u32, column: u32, pad0: u32, pad1: u32,
};
struct ReplayTile {
    origin: vec2<u32>, extent: vec2<u32>,
    samples: u32, pad0: u32, pad1: u32, pad2: u32,
};
@group(3) @binding(0) var<storage, read_write> replay_pixels: array<ReplayPixel>;
@group(3) @binding(1) var<storage, read> replay_chunk: array<vec2<f32>>;
@group(3) @binding(2) var<uniform> replay_ticket: ReplayTicket;
@group(3) @binding(3) var<uniform> replay_tile: ReplayTile;

fn replay_key(local: vec2<u32>, sample_index: u32) -> u32 {
    return (sample_index * replay_tile.extent.y + local.y) * replay_tile.extent.x + local.x;
}
fn replay_fragment_key(pos: vec4<f32>, sample_index: u32) -> u32 {
    return replay_key(vec2<u32>(pos.xy) - replay_tile.origin, sample_index);
}
fn fresh_axis(t: f32, count: u32) -> ReplayAxis {
    return ReplayAxis(t, 0.0, 0.0, 0u, count, 0u, 0u, 0u, 0u, 0.0, 0u, 0u);
}
@fragment fn fs_replay_init(in: FieldOut, @builtin(sample_index) sample_index: u32)
    -> @location(0) vec4<f32> {
    var s: ReplayPixel;
    let cy = field_flag(FIELD_COLUMNS_ARE_Y);
    s.x = fresh_axis(in.axis_t.x, quad_count(select(field.cols, field.rows, cy), LATTICE_QUADS));
    s.y = fresh_axis(in.axis_t.y, quad_count(select(field.rows, field.cols, cy), LATTICE_QUADS));
    s.valid_mask = 0u;
    s.sample_marker = sample_index + 1u;
    replay_pixels[replay_fragment_key(in.pos, sample_index)] = s;
    return vec4<f32>(0.0);
}
fn replay_has(k: u32) -> bool {
    return k >= replay_ticket.start && k - replay_ticket.start < replay_ticket.len;
}
fn replay_pair(k: u32) -> vec2<f32> {
    return replay_chunk[k - replay_ticket.start];
}
struct ReplayBoundary { available: bool, value: f32 };
fn replay_boundary(k: u32) -> ReplayBoundary {
    var out = ReplayBoundary(false, 0.0);
    var pair = vec2<f32>(0.0);
    let n = replay_ticket.n;
    if (lattice_is_samples(LATTICE_QUADS)) {
        if (field_flag(FIELD_CENTERS)) {
            if (!replay_has(k)) { return out; }
            pair = replay_pair(k);
        } else {
            let next = min(k + 1u, n - 1u);
            if (!replay_has(k) || !replay_has(next)) { return out; }
            pair = midpoint_grid_pair(replay_pair(k), replay_pair(next));
        }
    } else if (!field_flag(FIELD_CENTERS)) {
        if (!replay_has(k)) { return out; }
        pair = replay_pair(k);
    } else if (k == 0u) {
        let next = min(1u, n - 1u);
        if (!replay_has(0u) || !replay_has(next)) { return out; }
        let c0 = replay_pair(0u);
        let c1 = replay_pair(next);
        pair = add_grid_pairs(c0, scale_grid_pair(subtract_grid_pairs(c0, c1), 0.5));
    } else if (k >= n) {
        let last = n - 1u;
        let prev = max(n, 2u) - 2u;
        if (!replay_has(last) || !replay_has(prev)) { return out; }
        pair = add_grid_pairs(replay_pair(last),
            scale_grid_pair(subtract_grid_pairs(replay_pair(last), replay_pair(prev)), 0.5));
    } else {
        if (!replay_has(k - 1u) || !replay_has(k)) { return out; }
        pair = midpoint_grid_pair(replay_pair(k - 1u), replay_pair(k));
    }
    return ReplayBoundary(true, axis_t(pair, replay_ticket.axis));
}
fn advance_axis(input: ReplayAxis) -> ReplayAxis {
    var s = input;
    if (s.phase >= 5u) { return s; }
    if (s.phase == 0u && (replay_ticket.count == 0u || replay_ticket.n == 0u
        || !f32_is_finite(s.t))) { s.phase = 6u; return s; }
    if (s.phase == 2u && (s.hi - s.lo <= 1u || s.steps == 32u)) { s.phase = 3u; }
    var k = 0u;
    switch s.phase {
        case 0u: { k = 0u; }
        case 1u: { k = replay_ticket.count; }
        case 2u: { k = s.lo + (s.hi - s.lo) / 2u; }
        case 3u: { k = s.lo; }
        case 4u: { k = s.lo + 1u; }
        default: { return s; }
    }
    let boundary = replay_boundary(k);
    if (!boundary.available) { return s; }
    let v = boundary.value;
    if (!f32_is_finite(v)) { s.phase = 6u; return s; }
    if (s.phase == 0u) {
        s.first = v;
        s.phase = 1u;
    } else if (s.phase == 1u) {
        if (s.t < min(s.first, v) || s.t > max(s.first, v)) { s.phase = 6u; }
        else { s.ascending = select(0u, 1u, v >= s.first); s.phase = 2u; }
    } else if (s.phase == 2u) {
        let before = select(v > s.t, v <= s.t, s.ascending != 0u);
        if (before) { s.lo = k; } else { s.hi = k; }
        s.steps = s.steps + 1u;
    } else if (s.phase == 3u) {
        s.a = v;
        s.phase = 4u;
    } else if (s.phase == 4u) {
        let span = v - s.a;
        if (!f32_is_finite(span)) { s.phase = 6u; return s; }
        var frac = 0.0;
        if (span != 0.0) {
            frac = (s.t - s.a) / span;
            if (!f32_is_finite(frac)) { s.phase = 6u; return s; }
        }
        s.index = s.lo;
        s.frac = clamp(frac, 0.0, 1.0);
        s.hit = 1u;
        s.phase = 5u;
    }
    return s;
}
@compute @workgroup_size(8, 8, 1)
fn cs_replay_axis(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= replay_tile.extent) || id.z >= replay_tile.samples) { return; }
    let p = replay_key(id.xy, id.z);
    var s = replay_pixels[p];
    if (replay_ticket.axis == 0u) { s.x = advance_axis(s.x); }
    else { s.y = advance_axis(s.y); }
    replay_pixels[p] = s;
}
@compute @workgroup_size(8, 8, 1)
fn cs_replay_z(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= replay_tile.extent) || id.z >= replay_tile.samples) { return; }
    let p = replay_key(id.xy, id.z);
    var s = replay_pixels[p];
    if (s.x.hit == 0u || s.y.hit == 0u) { return; }
    let cy = field_flag(FIELD_COLUMNS_ARE_Y);
    let c = select(s.x.index, s.y.index, cy);
    let r = select(s.y.index, s.x.index, cy);
    let need = select(1u, 4u, field_flag(FIELD_INTERPOLATED));
    for (var slot = 0u; slot < need; slot = slot + 1u) {
        let dc = slot & 1u;
        let dr = slot >> 1u;
        let row = r + dr;
        if (c + dc != replay_ticket.column || !replay_has(row)) { continue; }
        s.z[slot] = replay_pair(row);
        s.valid_mask = s.valid_mask | (1u << slot);
    }
    replay_pixels[p] = s;
}
fn replay_hit(s: ReplayAxis) -> CellHit {
    return CellHit(s.index, s.frac, s.hit != 0u);
}
fn replay_grid_value(s: ReplayPixel, c: u32, r: u32) -> GridValue {
    let cy = field_flag(FIELD_COLUMNS_ARE_Y);
    let base_c = select(s.x.index, s.y.index, cy);
    let base_r = select(s.y.index, s.x.index, cy);
    let slot = (r - base_r) * 2u + c - base_c;
    return GridValue(s.z[slot], (s.valid_mask & (1u << slot)) != 0u);
}
@fragment fn fs_nan_reference() -> @location(0) vec4<f32> {
    return style.color_premul;
}
@fragment fn fs_replay_final(in: FieldOut, @builtin(sample_index) sample_index: u32)
    -> @location(0) vec4<f32> {
    let s = replay_pixels[replay_fragment_key(in.pos, sample_index)];
    // An unfinished locate or wrong sample slot must fail the final-image gate,
    // rather than silently masquerading as a transparent field miss.
    if (s.x.phase < 5u || s.y.phase < 5u || s.sample_marker != sample_index + 1u) {
        return vec4<f32>(1.0, 0.0, 1.0, 1.0);
    }
    let clear = vec4<f32>(0.0);
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let x = replay_hit(s.x);
    let y = replay_hit(s.y);
    if (!x.hit || !y.hit) { return clear; }
"#;

fn shader_source() -> String {
    // Reuse the resident final colour arithmetic verbatim; only the four
    // source loads change. No hand-maintained second interpolation/ramp body.
    let main = RESIDENT.split("fn fs_main(in: FieldOut)").nth(1).unwrap();
    let tail = main
        .split("    let c = select(x.index, y.index, columns_are_y);")
        .nth(1)
        .unwrap()
        .split("\n}\n")
        .next()
        .unwrap();
    let tail = format!("    let c = select(x.index, y.index, columns_are_y);{tail}");
    assert_eq!(tail.matches("grid_value(").count(), 4);
    assert!(!tail.contains("locate("));
    format!(
        "{RESIDENT}\n{ENTRIES}{}\n}}",
        tail.replace("grid_value(", "replay_grid_value(s, ")
    )
}

// If bracket width is w > 1, either branch leaves at most ceil(w/2).
// Therefore ceil(log2(count)) binary transitions suffice for every u32 count.
// First/last boundaries and final a/b add four transitions. A complete sweep
// contains every adjacent pair, so every unfinished pixel makes at least one
// transition per sweep (or fails closed). No GPU-state readback is needed.
fn replay_sweeps(count: u32) -> u32 {
    if count == 0 {
        1
    } else {
        u32::BITS - (count - 1).leading_zeros() + 4
    }
}

#[test]
fn replay_sweep_bound_checks_halving_recurrence_and_u32_extremes() {
    let mut counts: Vec<u32> = (0..65_536).collect();
    for bit in 0..32 {
        let n = 1u32 << bit;
        counts.extend([n.saturating_sub(1), n, n.saturating_add(1)]);
    }
    counts.extend([u32::MAX - 1, u32::MAX]);
    for count in counts {
        let mut width = u64::from(count);
        let mut transitions = 0;
        while width > 1 {
            width = width.div_ceil(2);
            transitions += 1;
        }
        if count != 0 {
            assert_eq!(replay_sweeps(count), transitions + 4);
        }
        assert!(replay_sweeps(count) <= 36);
    }
    assert_eq!(replay_sweeps(u32::MAX), 36);
}

fn dispatch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipe: &wgpu::ComputePipeline,
    groups: &[(u32, &wgpu::BindGroup)],
    tile: (u32, u32, u32, u32),
    samples: u32,
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(pipe);
        for &(index, group) in groups {
            pass.set_bind_group(index, group, &[]);
        }
        pass.dispatch_workgroups(tile.2.div_ceil(8), tile.3.div_ceil(8), samples);
    }
    queue.submit([encoder.finish()]);
}

#[test]
fn bounded_heatmap_axis_and_z_replay_matches_resident_alpha_samples() {
    let instance = renderer::data_render::create_instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("P-00 requires GPU; no skip");
    eprintln!("bounded field replay adapter: {:?}", adapter.get_info());
    let support = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm);
    assert!(support.flags.sample_count_supported(1) && support.flags.sample_count_supported(4));
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("resident field plus bounded replay proof"),
        source: wgpu::ShaderSource::Wgsl(shader_source().into()),
    });
    let axis_pipe = compute_pipeline(&device, &shader, "cs_replay_axis");
    let z_pipe = compute_pipeline(&device, &shader, "cs_replay_z");
    let mut compared = 0;
    for centers in [false, true] {
        for interpolated in [false, true] {
            for columns_are_y in [false, true] {
                let case = Fixture {
                    centers,
                    interpolated,
                    columns_are_y,
                    log: centers != interpolated,
                    inverted: columns_are_y,
                };
                let data = fixture_data(case);
                let transform = buffer(
                    &device,
                    bytemuck::bytes_of(&data.transform),
                    wgpu::BufferUsages::UNIFORM,
                );
                let style = buffer(
                    &device,
                    bytemuck::bytes_of(&data.style),
                    wgpu::BufferUsages::UNIFORM,
                );
                let params = buffer(
                    &device,
                    bytemuck::bytes_of(&data.params),
                    wgpu::BufferUsages::UNIFORM,
                );
                let oracle_pool = buffer(
                    &device,
                    bytemuck::cast_slice(&data.oracle_pool),
                    wgpu::BufferUsages::STORAGE,
                );
                let oracle_grid = buffer(
                    &device,
                    bytemuck::cast_slice(&data.oracle_grid),
                    wgpu::BufferUsages::STORAGE,
                );
                let stops = [
                    [0.9f32, 0.1, 0.15, 0.55],
                    [0.15, 0.8, 0.2, 0.9],
                    [0.1, 0.2, 0.95, 0.3],
                ];
                let stops = buffer(
                    &device,
                    bytemuck::cast_slice(&stops),
                    wgpu::BufferUsages::STORAGE,
                );
                let metadata = buffer(
                    &device,
                    bytemuck::cast_slice(&[0u32; 2]),
                    wgpu::BufferUsages::STORAGE,
                );
                for samples in [1, 4] {
                    let oracle_pipe = render_pipeline(&device, &shader, "fs_main", samples, false);
                    let init_pipe =
                        render_pipeline(&device, &shader, "fs_replay_init", samples, true);
                    let final_pipe =
                        render_pipeline(&device, &shader, "fs_replay_final", samples, false);
                    let nan_pipe =
                        render_pipeline(&device, &shader, "fs_nan_reference", samples, false);
                    // Include all candidate fixed buffers, not just the pixel array.
                    let fixed = transform.size()
                        + style.size()
                        + params.size()
                        + stops.size()
                        + metadata.size()
                        + 16
                        + 32
                        + 32;
                    let available = (SCRATCH_BUDGET - fixed)
                        .min(u64::from(device.limits().max_storage_buffer_binding_size));
                    let pixels =
                        u32::try_from(available / STATE_BYTES / u64::from(samples)).unwrap();
                    assert!(pixels > 0 && pixels < PANEL.2 * PANEL.3);
                    let mut tile_w = 1u32;
                    while tile_w < PANEL.2 && (tile_w + 1).pow(2) <= pixels {
                        tile_w += 1;
                    }
                    let tile_h = (pixels / tile_w).min(PANEL.3);
                    let state_bytes = STATE_BYTES * u64::from(tile_w * tile_h * samples);
                    assert!(state_bytes + fixed <= SCRATCH_BUDGET);
                    let state = empty(&device, state_bytes, wgpu::BufferUsages::STORAGE);
                    let chunk = empty(
                        &device,
                        16,
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    );
                    let ticket = empty(
                        &device,
                        32,
                        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    );
                    let tile_buf = empty(
                        &device,
                        32,
                        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    );
                    let oracle_g0 = bindings(
                        &device,
                        &oracle_pipe.get_bind_group_layout(0),
                        &[(0, &transform)],
                    );
                    let oracle_g1 = bindings(
                        &device,
                        &oracle_pipe.get_bind_group_layout(1),
                        &[(0, &style)],
                    );
                    let oracle_g2 = bindings(
                        &device,
                        &oracle_pipe.get_bind_group_layout(2),
                        &[
                            (0, &oracle_pool),
                            (1, &oracle_grid),
                            (3, &stops),
                            (4, &params),
                            (6, &metadata),
                        ],
                    );
                    // Candidate groups have no field_pool/grid binding. Auto-layout
                    // validates this claim for every entry's reachable call graph.
                    let init_g2 = bindings(
                        &device,
                        &init_pipe.get_bind_group_layout(2),
                        &[(4, &params)],
                    );
                    let init_g3 = bindings(
                        &device,
                        &init_pipe.get_bind_group_layout(3),
                        &[(0, &state), (3, &tile_buf)],
                    );
                    let axis_g0 = bindings(
                        &device,
                        &axis_pipe.get_bind_group_layout(0),
                        &[(0, &transform)],
                    );
                    let axis_g2 = bindings(
                        &device,
                        &axis_pipe.get_bind_group_layout(2),
                        &[(4, &params)],
                    );
                    let axis_g3 = bindings(
                        &device,
                        &axis_pipe.get_bind_group_layout(3),
                        &[(0, &state), (1, &chunk), (2, &ticket), (3, &tile_buf)],
                    );
                    let z_g2 = bindings(&device, &z_pipe.get_bind_group_layout(2), &[(4, &params)]);
                    let z_g3 = bindings(
                        &device,
                        &z_pipe.get_bind_group_layout(3),
                        &[(0, &state), (1, &chunk), (2, &ticket), (3, &tile_buf)],
                    );
                    let final_g1 = bindings(
                        &device,
                        &final_pipe.get_bind_group_layout(1),
                        &[(0, &style)],
                    );
                    let final_g2 = bindings(
                        &device,
                        &final_pipe.get_bind_group_layout(2),
                        &[(3, &stops), (4, &params), (6, &metadata)],
                    );
                    let final_g3 = bindings(
                        &device,
                        &final_pipe.get_bind_group_layout(3),
                        &[(0, &state), (3, &tile_buf)],
                    );
                    let oracle = texture(&device, samples);
                    let candidate = texture(&device, samples);
                    let nan_reference = texture(&device, samples);
                    let oracle_view = oracle.create_view(&Default::default());
                    let candidate_view = candidate.create_view(&Default::default());
                    let nan_view = nan_reference.create_view(&Default::default());
                    let nan_group =
                        bindings(&device, &nan_pipe.get_bind_group_layout(1), &[(0, &style)]);
                    let mut first = true;
                    let mut tile_count = 0;
                    for y in (PANEL.1..PANEL.1 + PANEL.3).step_by(tile_h as usize) {
                        for x in (PANEL.0..PANEL.0 + PANEL.2).step_by(tile_w as usize) {
                            let tile = (
                                x,
                                y,
                                tile_w.min(PANEL.0 + PANEL.2 - x),
                                tile_h.min(PANEL.1 + PANEL.3 - y),
                            );
                            queue.write_buffer(
                                &tile_buf,
                                0,
                                bytemuck::cast_slice(&[x, y, tile.2, tile.3, samples, 0, 0, 0]),
                            );
                            let mut encoder = device.create_command_encoder(&Default::default());
                            draw(
                                &mut encoder,
                                &init_pipe,
                                &candidate_view,
                                &[(2, &init_g2), (3, &init_g3)],
                                tile,
                                first,
                            );
                            queue.submit([encoder.finish()]);
                            first = false;
                            for axis in 0..2 {
                                let source = &data.axes[axis];
                                let cells = if (axis == 0) == !case.columns_are_y {
                                    data.params.cols
                                } else {
                                    data.params.rows
                                };
                                let count = cells.saturating_sub(u32::from(case.interpolated));
                                for _ in 0..replay_sweeps(count) {
                                    // Sliding two-pair windows include every required
                                    // midpoint/extrapolated boundary, including n=1.
                                    for start in 0..source.len().saturating_sub(1).max(1) {
                                        let len = TICKET_PAIRS.min((source.len() - start) as u32);
                                        queue.write_buffer(
                                            &ticket,
                                            0,
                                            bytemuck::cast_slice(&[
                                                start as u32,
                                                len,
                                                source.len() as u32,
                                                count,
                                                axis as u32,
                                                0,
                                                0,
                                                0,
                                            ]),
                                        );
                                        queue.write_buffer(
                                            &chunk,
                                            0,
                                            bytemuck::cast_slice(
                                                &source[start..start + len as usize],
                                            ),
                                        );
                                        dispatch(
                                            &device,
                                            &queue,
                                            &axis_pipe,
                                            &[(0, &axis_g0), (2, &axis_g2), (3, &axis_g3)],
                                            tile,
                                            samples,
                                        );
                                    }
                                }
                            }
                            for (column, &id) in data.declarations[..data.params.cols as usize]
                                .iter()
                                .enumerate()
                            {
                                let source = &data.sources[id];
                                for start in (0..source.len()).step_by(TICKET_PAIRS as usize) {
                                    let len = TICKET_PAIRS.min((source.len() - start) as u32);
                                    queue.write_buffer(
                                        &ticket,
                                        0,
                                        bytemuck::cast_slice(&[
                                            start as u32,
                                            len,
                                            0,
                                            0,
                                            0,
                                            column as u32,
                                            0,
                                            0,
                                        ]),
                                    );
                                    queue.write_buffer(
                                        &chunk,
                                        0,
                                        bytemuck::cast_slice(&source[start..start + len as usize]),
                                    );
                                    dispatch(
                                        &device,
                                        &queue,
                                        &z_pipe,
                                        &[(2, &z_g2), (3, &z_g3)],
                                        tile,
                                        samples,
                                    );
                                }
                            }
                            let mut encoder = device.create_command_encoder(&Default::default());
                            draw(
                                &mut encoder,
                                &final_pipe,
                                &candidate_view,
                                &[(1, &final_g1), (2, &final_g2), (3, &final_g3)],
                                tile,
                                false,
                            );
                            queue.submit([encoder.finish()]);
                            tile_count += 1;
                        }
                    }
                    assert!(tile_count > 1);
                    let mut encoder = device.create_command_encoder(&Default::default());
                    draw(
                        &mut encoder,
                        &oracle_pipe,
                        &oracle_view,
                        &[(0, &oracle_g0), (1, &oracle_g1), (2, &oracle_g2)],
                        PANEL,
                        true,
                    );
                    draw(
                        &mut encoder,
                        &nan_pipe,
                        &nan_view,
                        &[(1, &nan_group)],
                        PANEL,
                        true,
                    );
                    let expected = final_image(&device, &mut encoder, &oracle, samples);
                    let actual = final_image(&device, &mut encoder, &candidate, samples);
                    let nan_image = final_image(&device, &mut encoder, &nan_reference, samples);
                    queue.submit([encoder.finish()]);
                    let expected = read_final_image(&device, &expected);
                    let actual = read_final_image(&device, &actual);
                    let nan_image = read_final_image(&device, &nan_image);
                    for (i, (&expected, &actual)) in expected.iter().zip(&actual).enumerate() {
                        assert_eq!(
                            actual, expected,
                            "{case:?}, {samples}x, pixel/sample offset {i}, tile {tile_w}x{tile_h}"
                        );
                        compared += 1;
                    }
                    let colors = actual
                        .iter()
                        .copied()
                        .collect::<std::collections::HashSet<_>>();
                    // GPU blending/UNORM conversion is the oracle here too;
                    // a CPU reimplementation may round differently. The
                    // reference is another final image, never replay state.
                    let nan_color = nan_image[(PANEL.1 * WIDTH + PANEL.0) as usize];
                    assert!(
                        colors.contains(&nan_color),
                        "NaN branch never painted: {case:?}"
                    );
                    assert!(
                        colors.len() > 3,
                        "finite low-lane ramp must vary beyond background/NaN"
                    );
                    eprintln!(
                        "bounded Heatmap {case:?} {samples}x: {tile_count} tiles, {} candidate bytes, {} colors",
                        state_bytes + fixed,
                        colors.len()
                    );
                }
            }
        }
    }
    assert_eq!(compared, (WIDTH * HEIGHT * 5 * 8) as usize);
}
