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
//                       phase stays continuous across gaps.
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

// ───── BEGIN common block (SHADER_COMMON.md) ─────
// WGSL has no import. The Transform/maybe_log/data_to_ndc definitions below
// are duplicated across the data shaders. To modify any of them, FIRST edit
// src/data_render/SHADER_COMMON.md, then mirror the change into every
// sibling shader. Do not edit only one file.
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    sketch_amp_wave: vec2<f32>,
    sketch_seed: vec2<f32>,
};

@group(0) @binding(0) var<uniform> transform: Transform;

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}

fn data_to_ndc(v: vec2<f32>) -> vec2<f32> {
    let xv = maybe_log(v.x, transform.scale_log.x);
    let yv = maybe_log(v.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    return t * 2.0 - 1.0;
}
// ───── END common block ─────

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
};

@group(1) @binding(0) var<storage, read> pool: array<f32>;
@group(1) @binding(1) var<storage, read_write> dst: array<f32>;
@group(1) @binding(2) var<storage, read_write> sums: array<f32>;
@group(1) @binding(3) var<uniform> params: ArcParams;
// Running total across chunks — one f32, zeroed before the first chunk.
@group(1) @binding(4) var<storage, read_write> carry: array<f32>;

const WG: u32 = 256u;

var<workgroup> scan_shared: array<f32, 256>;

fn point_px(i: u32) -> vec2<f32> {
    let p = vec2<f32>(pool[params.x_base + i], pool[params.y_base + i]);
    return data_to_ndc(p) / transform.pixel_to_ndc;
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
    let d = distance(point_px(g - 1u), point_px(g));
    // Finite guard: NaN fails `d == d`, infinities fail the magnitude check.
    dst[g] = select(0.0, d, d == d && d < 1e30);
}

@compute @workgroup_size(256)
fn scan_block(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) lid_v: vec3<u32>,
    @builtin(workgroup_id) wid: vec3<u32>,
) {
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
