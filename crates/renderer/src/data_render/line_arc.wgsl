// Arc-length prefix for dashed lines — fully GPU-resident compute scan.
//
// The dash phase must advance with the polyline's cumulative PIXEL arc
// length, which depends on the data→pixel transform. This pipeline produces
// the per-point prefix directly on the GPU so the column data never has to
// come back to the CPU (the column pool keeps no CPU copies — that contract
// is load-bearing; do not reintroduce shadows to "simplify" this).
//
// Entry points, dispatched by `line_arc.rs`:
//   1. `seg_init`     — dst[start+i] = pixel length of segment (g-1, g) at
//                       global index g = start+i; 0 at g=0. Non-finite
//                       endpoints (NaN gaps, log of ≤0) contribute 0 so the
//                       phase stays continuous across gaps. A step mode
//                       (`params.mode`) sums the stepped pieces instead of
//                       the straight distance.
//   2. `scan_block`   — in-place per-256-block inclusive scan of the chunk,
//                       block totals written to `sums` (Hillis–Steele in
//                       workgroup memory).
//   3. `add_offsets`  — dst[start+i] += scanned_sums[block(i) - 1], block > 0.
//   4. `apply_carry`  — dst[start+i] += carry[0]: folds the running total of
//                       every previous chunk into this one.
//   5. `update_carry` — carry[0] = dst[start+len-1] (single thread), run
//                       AFTER apply_carry so the total stays cumulative.
//
// One chunk covers up to min(dispatch-limit × 256, 256³) points via the
// recursive scan (dst → sums0 → sums1); larger polylines are processed as
// SEQUENTIAL chunks in one encoder with the carry chain linking them — n is
// bounded only by pool memory, never by dispatch limits. The result buffer
// doubles as the line pipeline's vertex slots 4/5.

struct ArcParams {
    // Element count of this dispatch's window into `dst`.
    len: u32,
    // Element offsets of the X / Y columns inside the shared pool buffer
    // (seg_init only; scan/add ignore them).
    x_base: u32,
    y_base: u32,
    // First element of the window: kernels address dst[start + i]. Zero for
    // the sums buffers; the chunk offset for windows into the arc buffer.
    start: u32,
    // Step interpolation mode (LINE_STEP_*): seg_init sums the stepped
    // pieces' pixel lengths so the dash phase follows the drawn path.
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(1) @binding(0) var<storage, read> pool: array<f32>;
@group(1) @binding(1) var<storage, read_write> dst: array<f32>;
@group(1) @binding(2) var<storage, read_write> sums: array<f32>;
@group(1) @binding(3) var<uniform> params: ArcParams;
// Running total across chunks — one f32, zeroed before the first chunk.
@group(1) @binding(4) var<storage, read_write> carry: array<f32>;

const WG: u32 = 256u;

var<workgroup> scan_shared: array<f32, 256>;

// Windows DX12: bounded per-invocation zeroing avoids aggregate DXIL stores.
// Keep this override paired with gpu_compile::workgroup_options.
override FIGGY_EXPLICIT_WORKGROUP_ZERO: bool = false;

fn figgy_zero_workgroup(local_index: u32) {
    if FIGGY_EXPLICIT_WORKGROUP_ZERO {
        scan_shared[local_index] = f32();
        workgroupBarrier();
    }
}

fn point_valid(i: u32) -> bool {
    let xi = params.x_base + i * 2u;
    let yi = params.y_base + i * 2u;
    return sample_valid(transform, vec2<f32>(pool[xi], pool[xi + 1u]), vec2<f32>(pool[yi], pool[yi + 1u]));
}

fn point_pair(base: u32, i: u32) -> vec2<f32> {
    let j = base + i * 2u;
    return vec2<f32>(pool[j], pool[j + 1u]);
}

// Pixel length of the (possibly stepped) path from point i to point j: the
// straight distance for a linear polyline, the sum of the axis-aligned
// pieces for a step mode — exactly what the line vertex entries draw.
fn segment_px(i: u32, j: u32) -> f32 {
    let xa = point_pair(params.x_base, i);
    let ya = point_pair(params.y_base, i);
    let xb = point_pair(params.x_base, j);
    let yb = point_pair(params.y_base, j);
    var total = 0.0;
    let pieces = step_piece_count(params.mode);
    for (var k = 0u; k < pieces; k = k + 1u) {
        total = total + step_piece_len_px(transform, step_piece(xa, ya, xb, yb, params.mode, k));
    }
    return total;
}

@compute @workgroup_size(256)
fn seg_init(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.len) {
        return;
    }
    let g = params.start + i;
    if (g == 0u) {
        dst[0] = 0.0;
        return;
    }
    if (!point_valid(g - 1u) || !point_valid(g)) {
        dst[g] = 0.0;
        return;
    }
    let d = segment_px(g - 1u, g);
    // Finite guard: NaN fails `d == d`, infinities fail the magnitude check.
    dst[g] = select(0.0, d, d == d && d < 1e30);
}

@compute @workgroup_size(256)
fn scan_block(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) lid_v: vec3<u32>,
    @builtin(workgroup_id) wid: vec3<u32>,
) {
    figgy_zero_workgroup(lid_v.x);
    let i = gid.x;
    let lid = lid_v.x;
    var v = 0.0;
    if (i < params.len) {
        v = dst[params.start + i];
    }
    scan_shared[lid] = v;
    workgroupBarrier();

    var offset = 1u;
    loop {
        if (offset >= WG) {
            break;
        }
        var add = 0.0;
        if (lid >= offset) {
            add = scan_shared[lid - offset];
        }
        workgroupBarrier();
        scan_shared[lid] = scan_shared[lid] + add;
        workgroupBarrier();
        offset = offset << 1u;
    }

    if (i < params.len) {
        dst[params.start + i] = scan_shared[lid];
    }
    if (lid == WG - 1u) {
        sums[wid.x] = scan_shared[lid];
    }
}

@compute @workgroup_size(256)
fn add_offsets(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(workgroup_id) wid: vec3<u32>,
) {
    let i = gid.x;
    if (i >= params.len || wid.x == 0u) {
        return;
    }
    dst[params.start + i] = dst[params.start + i] + sums[wid.x - 1u];
}

@compute @workgroup_size(256)
fn apply_carry(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.len) {
        return;
    }
    dst[params.start + i] = dst[params.start + i] + carry[0];
}

@compute @workgroup_size(256)
fn update_carry(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x == 0u) {
        carry[0] = dst[params.start + params.len - 1u];
    }
}

// Bounded replay orchestration. The host cursor owns the canonical chunk and
// 256-point leaf ranges; these kernels do not infer ranges from IO chunking.
// A leaf is scanned with the entries above, its lane-255 total is copied to
// the canonical sums0 slot, and on replay the already scanned predecessor is
// added in the same order as `add_offsets` before any cross-chunk carry.
struct ArcReplayParams {
    block: u32,
    global_start: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(2) @binding(0) var<storage, read_write> replay_sums0: array<f32>;
@group(2) @binding(2) var<uniform> replay: ArcReplayParams;

@compute @workgroup_size(1)
fn collect_leaf_total() {
    replay_sums0[replay.block] = sums[0];
}

@compute @workgroup_size(256)
fn add_global_offset(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.len || replay.block == 0u) {
        return;
    }
    dst[params.start + i] = dst[params.start + i] + replay_sums0[replay.block - 1u];
}

// ── Constellation star-pass indirect args ──
// After the scan, dst[start+len-1] of the LAST chunk is the polyline's total
// pixel arc. One thread converts it into DrawIndirect args for the
// arc-driven star pass: candidate star slots are budgeted by TOTAL arc (one
// slot per `slot_pitch_px`), not per segment — which is what makes star
// density independent of how densely the data samples the line (the old
// per-segment quad budget saturated on sparse polylines). Dispatched with
// group(1) re-bound to the last chunk so `dst`/`params` window the tail.
struct StarIndirectParams {
    // Arc px per candidate star slot — must equal the value the star vertex
    // shader derives (0.5 · structure_scale; see line_columnar.wgsl
    // cons_star_pitch), or slot positions and the dispatch count disagree.
    slot_pitch_px: f32,
    // Runaway backstop for degenerate pitch / pathological arc lengths.
    max_slots: u32,
};

@group(2) @binding(0) var<storage, read_write> star_args: array<u32>;
@group(2) @binding(1) var<uniform> star_params: StarIndirectParams;

@compute @workgroup_size(1)
fn star_indirect() {
    let total = dst[params.start + params.len - 1u];
    var slots = 0u;
    if (total > 0.0 && total < 1e30 && star_params.slot_pitch_px > 0.0) {
        slots = min(u32(ceil(total / star_params.slot_pitch_px)), star_params.max_slots);
    }
    star_args[0] = 6u;    // vertex_count: one quad per slot
    star_args[1] = slots; // instance_count: candidate star slots
    star_args[2] = 0u;    // first_vertex
    star_args[3] = 0u;    // first_instance
}
