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

// ───── BEGIN common block (SHADER_COMMON.md) ─────
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    data_to_panel_scale: vec2<f32>,
    data_to_panel_offset: vec2<f32>,
    // Generic per-panel style parameter slots. Interpretation belongs to the
    // ACTIVE style's shader entries; the precise entries never read them.
    // sketch:        [0] = (amplitude_px, wavelength_px, seed(f32), 0)
    // milkyway:      [0] = (star_density, ribbon_width_px, ribbon_intensity,
    //                seed(f32)), [1] = (star_scale, spread_px, faint_bias, planet_rim),
    //                [2] = (structure_scale, star_brightness, 0, 0) — multiplier on the
    //                style's px-denominated structure constants (clump
    //                wavelength, binary separation); keeps the star texture
    //                resolution-invariant under DPI/export scaling.
    // constellation: [0] = (star_opacity, line_opacity, 0, 0)
    // All styles reserve [2].z for the global point-base u32 BIT PATTERN.
    // Resident draws write zero; streamed point/errorbar draws bitcast it.
    style_params: array<vec4<f32>, 3>,
    // Polar coordinates (Config.polar); all zero in a rectangular chart.
    // polar_angle:  (flag, zero angle in radians, radians per angle unit
    //               signed by the direction, inner radius fraction)
    // polar_extent: (sector start, sector end in angle units, end <= start
    //               for the full circle; arc pieces per segment, 0 for
    //               chords; outer radius in pixels)
    polar_angle: vec4<f32>,
    polar_extent: vec4<f32>,
};  // 144 B (vec4 array at offset 64, stride 16; polar vec4s at 112 and 128)

@group(0) @binding(0) var<uniform> transform: Transform;

fn styled_point_index(local_index: u32) -> u32 {
    return bitcast<u32>(transform.style_params[2].z) + local_index;
}

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    if is_log > 0.5 {
        return lv;
    }
    return mix(v, lv, is_log);
}

fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, panel_scale: f32, panel_offset: f32) -> f32 {
    let raw = v.x + v.y;
    let linear_num = fma(1.0, v.x - min_hi, v.y - min_lo);
    let range = fma(1.0, max_hi - min_hi, max_lo - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    if is_log > 0.5 {
        return fma(log_num / range, panel_scale, panel_offset);
    }
    let data_t = mix(linear_num / range, log_num / range, is_log);
    return fma(data_t, panel_scale, panel_offset);
}

// Polar (Config.polar): the angle column in radians around the centre of the
// data area, the radius axis fraction between the inner ring and the outer
// radius. A radius below the axis start sits on the inner ring (data samples
// there are masked by `sample_valid`; a fill baseline is not). Twin of
// `polar_projection::ndc` on the CPU.
fn polar_ndc(ty: f32, xv: vec2<f32>) -> vec2<f32> {
    let p = transform.polar_angle;
    let e = transform.polar_extent;
    let angle = p.y + p.z * (xv.x + xv.y);
    let rt = (ty - transform.data_to_panel_offset.y) / transform.data_to_panel_scale.y;
    let r = mix(p.w, 1.0, max(rt, 0.0));
    let center = (transform.data_to_panel_offset + transform.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    return center + vec2<f32>(cos(angle), sin(angle)) * r * e.w * transform.pixel_to_ndc;
}

fn data_to_ndc(xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    let tx = axis_pair_to_t(xv, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x, transform.data_to_panel_scale.x, transform.data_to_panel_offset.x);
    let ty = axis_pair_to_t(yv, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, transform.data_to_panel_scale.y, transform.data_to_panel_offset.y);
    if (transform.polar_angle.x > 0.5) {
        return polar_ndc(ty, xv);
    }
    return vec2<f32>(tx, ty) * 2.0 - 1.0;
}

// Mask source samples explicitly: NaN/Inf clipping is not portable in WGSL.
// Logarithmic coordinate domains exclude nonpositive samples, while bars and
// errorbar extents retain their separate baseline/clipping rules.
fn sample_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

// Polar: a radius below the axis start has no place on the plot (it would
// fold onto the inner ring), so the sample is not drawn and a line breaks
// there. Angles outside the sector and radii past the outer ring keep their
// projection and are cut by the fragment clip below.
fn polar_sample_valid(xv: vec2<f32>, yv: vec2<f32>) -> bool {
    if (transform.polar_angle.x < 0.5) {
        return true;
    }
    let ty = axis_pair_to_t(yv, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, transform.data_to_panel_scale.y, transform.data_to_panel_offset.y);
    let rt = (ty - transform.data_to_panel_offset.y) / transform.data_to_panel_scale.y;
    return rt >= 0.0;
}

fn sample_valid(xv: vec2<f32>, yv: vec2<f32>) -> bool {
    let x = xv.x + xv.y;
    let y = yv.x + yv.y;
    return sample_finite(xv.x) && sample_finite(xv.y)
        && sample_finite(yv.x) && sample_finite(yv.y)
        && sample_finite(x) && sample_finite(y)
        && (transform.scale_log.x == 0.0 || x > 0.0)
        && (transform.scale_log.y == 0.0 || y > 0.0)
        && polar_sample_valid(xv, yv);
}

// Polar clip. The transform is vertex-visible only, so the vertex stage hands
// the fragment stage the fragment's offset from the centre in pixels
// (interpolated from the corner's NDC) and the bounds as flat varyings; the
// fragment discards outside the outer ring, inside the hole and outside the
// sector, cutting a primitive at the edge the way the data-area scissor cuts
// a rectangular chart. A rectangular chart passes a zero flag.
fn polar_clip_offset(ndc: vec2<f32>) -> vec2<f32> {
    let center = (transform.data_to_panel_offset + transform.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    return (ndc - center) / transform.pixel_to_ndc;
}

// (flag, outer radius px, inner radius px, sector flag)
fn polar_clip_limits() -> vec4<f32> {
    let p = transform.polar_angle;
    let e = transform.polar_extent;
    return vec4<f32>(p.x, e.w, e.w * p.w, select(0.0, 1.0, e.y > e.x));
}

// (zero angle rad, radians per angle unit signed by the direction, sector
// start, sector end in angle units)
fn polar_clip_sector() -> vec4<f32> {
    let p = transform.polar_angle;
    let e = transform.polar_extent;
    return vec4<f32>(p.y, p.z, e.x, e.y);
}

fn polar_fragment_visible(px: vec2<f32>, limits: vec4<f32>, sector: vec4<f32>) -> bool {
    if (limits.x < 0.5) {
        return true;
    }
    let r = length(px);
    if (r > limits.y + 0.5 || r < limits.z - 0.5) {
        return false;
    }
    if (limits.w < 0.5) {
        return true;
    }
    // The data angle of the fragment, brought into the turn that starts at
    // the sector start.
    let turn = 6.283185307179586 / abs(sector.y);
    var a = (atan2(px.y, px.x) - sector.x) / sector.y;
    a = a - floor((a - sector.z) / turn) * turn;
    return a <= sector.w + 1e-6 * turn;
}
// ───── END common block ─────

// ──────────── step interpolation (NOT part of the common block) ────────────
// A step mode draws every instance (segment a → b) as axis-aligned pieces.
// Piece corners live in DATA space so every consumer projects the same
// points: the line, area, sketch, ribbon and star entries here, the
// arc-length scan (line_arc.wgsl, `params.mode`) and the GPU picker
// (gpu_pick/line.wgsl, in its own pick_* ABI). `step_piece_count`, `step_piece`
// and `step_piece_len_px` must stay byte-identical between line_columnar.wgsl
// and line_arc.wgsl (shader_consistency checks); the CPU twin is
// `line_step::step_pieces` in the renderer crate.
const LINE_STEP_SHIFT: u32 = 2u;
const LINE_STEP_MASK: u32 = 3u;
const LINE_STEP_LINEAR: u32 = 0u;
const LINE_STEP_PRE: u32 = 1u;
const LINE_STEP_POST: u32 = 2u;
const LINE_STEP_MID: u32 = 3u;
const LINE_ARC_SHIFT: u32 = 4u;

// The mode word: bits 0-1 the step mode, bits 4 and up the polar arc pieces
// per segment (0 for chords). An arc overrides the step mode; the renderer
// rejects the two together.
fn step_piece_count(mode: u32) -> u32 {
    let arc = mode >> LINE_ARC_SHIFT;
    if (arc > 0u) {
        return arc;
    }
    let step = mode & LINE_STEP_MASK;
    if (step == LINE_STEP_PRE || step == LINE_STEP_POST) {
        return 2u;
    }
    if (step == LINE_STEP_MID) {
        return 3u;
    }
    return 1u;
}

// Data-space (hi, lo) corners of piece `k` of the stepped segment a → b.
struct StepPiece {
    x0: vec2<f32>,
    y0: vec2<f32>,
    x1: vec2<f32>,
    y1: vec2<f32>,
};

fn step_piece(xa: vec2<f32>, ya: vec2<f32>, xb: vec2<f32>, yb: vec2<f32>, mode: u32, k: u32) -> StepPiece {
    var p: StepPiece;
    p.x0 = xa;
    p.y0 = ya;
    p.x1 = xb;
    p.y1 = yb;
    let arc = mode >> LINE_ARC_SHIFT;
    if (arc > 0u) {
        // Polar arc: piece k of `arc` equal subdivisions of a -> b in data
        // space, so a constant radius follows its ring once projected.
        let t0 = f32(k) / f32(arc);
        let t1 = f32(k + 1u) / f32(arc);
        p.x0 = mix(xa, xb, t0);
        p.y0 = mix(ya, yb, t0);
        p.x1 = mix(xa, xb, t1);
        p.y1 = mix(ya, yb, t1);
        return p;
    }
    let step = mode & LINE_STEP_MASK;
    if (step == LINE_STEP_PRE) {
        // Riser at x_a, then the run at y_b.
        if (k == 0u) { p.x1 = xa; } else { p.y0 = yb; }
    } else if (step == LINE_STEP_POST) {
        // Run at y_a, then the riser at x_b.
        if (k == 0u) { p.y1 = ya; } else { p.x0 = xb; }
    } else if (step == LINE_STEP_MID) {
        // The riser sits at the DATA-space midpoint of the two x values, not
        // the pixel midpoint — the two differ on a logarithmic axis, and the
        // CPU picker places it in data units too.
        let xm = (xa + xb) * 0.5;
        if (k == 0u) {
            p.x1 = xm;
            p.y1 = ya;
        } else if (k == 1u) {
            p.x0 = xm;
            p.x1 = xm;
        } else {
            p.x0 = xm;
            p.y0 = yb;
        }
    }
    return p;
}

// Pixel length of one piece, with the arc scan's projection.
fn step_piece_len_px(p: StepPiece) -> f32 {
    let a_px = data_to_ndc(p.x0, p.y0) / transform.pixel_to_ndc;
    let b_px = data_to_ndc(p.x1, p.y1) / transform.pixel_to_ndc;
    return distance(a_px, b_px);
}
// ─────────────────────── end step interpolation ───────────────────────

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
    return sample_valid(vec2<f32>(pool[xi], pool[xi + 1u]), vec2<f32>(pool[yi], pool[yi + 1u]));
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
        total = total + step_piece_len_px(step_piece(xa, ya, xb, yb, params.mode, k));
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

// ── Constellation star-pass indirect args (NOT part of the common block) ──
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
