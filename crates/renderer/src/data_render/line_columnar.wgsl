// Columnar line shader — variable-thickness via quad extrusion plus analytic
// path-stroke coverage.
//
// One line segment = one instance, 4 vertices per instance (TriangleStrip).
// The X and Y columns are bound twice: the second binding starts one logical
// f32-pair value later, so instance `i` sees points [i] and [i+1] at once.
//
// Corner mapping per segment:
//   vid 0 : at A, −normal
//   vid 1 : at A, +normal
//   vid 2 : at B, −normal
//   vid 3 : at B, +normal
//
// The normal is computed in pixel space and converted back to NDC, so width
// stays constant across panels with different X/Y pixel ratios. The geometric
// quad is inflated by one AA pixel; the fragment stage evaluates the local
// square-cap stroke SDF so side edges, caps, and dash cuts get smooth coverage
// even when the render target itself is single-sample.
// Zero-length segments collapse the quad and become invisible.
//
// Dash patterns (Style.dash / Style.dash_len) are evaluated per fragment
// against the polyline's CUMULATIVE ARC LENGTH: slots 4/5 carry a GPU-computed
// prefix (pixels from the first point to point i), bound twice with a
// one-f32 shift like x/y, so the phase is exact and continuous across every
// joint regardless of curvature or sampling density. (A screen-position
// projection was tried first: its phase jumps by |position|·Δdirection at
// each joint, which shatters the pattern on curves.) Solid lines bind the X
// column as inert filler — the fragment stage never reads the varying when
// dash_len == 0.
//
// Step interpolation (Style.primitive_flags bits 2-3) draws every instance as
// 2 or 3 axis-aligned pieces instead of one straight segment. The strip then
// carries one 4-vertex quad per piece with two duplicated vertices between
// pieces (degenerate triangles), so the pipeline and the instance count stay
// the same and only the per-instance vertex count changes — see
// `strip_slot` and the CPU twin `line_strip_vertex_count` in mod.rs. The
// arc-length scan (line_arc.wgsl) and the GPU/CPU pickers walk the same
// pieces, so dash phase and picking stay on the stepped path.
//
// `vs_area` / `fs_area` fill the area under the line (to a baseline carried
// in Style.dash[1].xy) or between this line and a second y column bound in
// slots 4/5 (Style.primitive_flags bit 4). One instance per segment emits one
// trapezoid per step piece. It is a separate pipeline ("figgy line area
// pipeline") because its slots 4/5 are (hi, lo) pairs, not arc lengths.

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

struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
    point_radius_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
    shape_id: u32,
    dash_len: u32,
    // Per-series decorrelation salt (FNV-1a of series_id). Styled entries
    // (sketch/milkyway/constellation) XOR it into their hash seeds so two series never
    // share a star/wobble pattern; precise entries never read it.
    series_salt: u32,
    // Primitive-specific feature bits. Errorbar uses bit 0 for Y and bit 1
    // for X. Line entries read bits 2-3 as the step interpolation mode
    // (0 linear, 1 step-pre, 2 step-post, 3 step-mid) and the area entry
    // also reads bit 4 (fill to a series instead of a baseline); every
    // other primitive ignores this field.
    primitive_flags: u32,
    dash: array<vec4<f32>, 2>,
};

@group(1) @binding(0) var<uniform> style: Style;

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return select(v, lv, is_log > 0.5);
}

fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, panel_scale: f32, panel_offset: f32) -> f32 {
    let raw = v.x + v.y;
    let linear_num = fma(1.0, v.x - min_hi, v.y - min_lo);
    let range = fma(1.0, max_hi - min_hi, max_lo - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    let data_t = select(linear_num / range, log_num / range, is_log > 0.5);
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

struct VsIn {
    @location(0) x_a: vec2<f32>,
    @location(1) y_a: vec2<f32>,
    @location(2) x_b: vec2<f32>,
    @location(3) y_b: vec2<f32>,
};

// Line-only extra instance inputs (outside the common block): cumulative
// arc length at the segment's two endpoints, from the GPU-computed prefix.
struct VsArc {
    @location(4) arc_a: f32,
    @location(5) arc_b: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Arc length (px) from the polyline start: arc_a at the two A-side
    // corners, arc_b at the two B-side corners — interpolation yields the
    // per-fragment arc position the dash pattern walks.
    @location(0) dist_px: f32,
    // Local segment-space coordinates in pixels. The fragment stage treats
    // the whole segment stroke as one square-cap rectangle:
    //   along_px ∈ [-half_w, len + half_w], side_px ∈ [-half_w, half_w].
    @location(1) along_px: f32,
    @location(2) side_px: f32,
    @location(3) segment_len_px: f32,
    @location(4) polar_px: vec2<f32>,
    @location(5) @interpolate(flat) polar_limits: vec4<f32>,
    @location(6) @interpolate(flat) polar_sector: vec4<f32>,
};


// ───── shared line stroke (SHADER_COMMON.md §5.6; `shader_consistency`
// compares these function bodies and constants with the pick shader) ─────

// Solid strokes grow their quad by this anti-aliasing fringe on every side;
// dashed strokes keep the bare stroke so the measured dash lengths hold.
const LINE_AA_EXTENT_PX: f32 = 0.5;
// Subdivision count S per segment instance: the sketch pipeline draws
// 2*(S+1) strip vertices per instance. The CPU-side draw count lives in
// mod.rs (`LINE_SKETCH_VERTICES_PER_INSTANCE`) — keep the two in sync.
const SKETCH_SUBDIV: u32 = 8u;

/// Signed pixel distance from a stroke fragment to the edge of its piece's
/// square-capped rectangle. `along` runs from the piece start along its
/// direction, `side` across it; the rectangle is
/// `[-half_w, len + half_w] x [-half_w, half_w]`. The stroke covers a pixel
/// where this is below its anti-aliasing width.
fn line_stroke_distance(along: f32, side: f32, len: f32, half_w: f32) -> f32 {
    let p = vec2<f32>(along - len * 0.5, side);
    let half_rect = vec2<f32>(len * 0.5 + half_w, half_w);
    let q = abs(p) - half_rect;
    let outside = max(q, vec2<f32>(0.0, 0.0));
    return length(outside) + min(max(q.x, q.y), 0.0);
}

/// Whether the dash pattern is "on" at arc position `dist_px`. `dash` packs
/// up to eight sequential `[on, off, ...]` pixel lengths (`dash[0].xyzw`
/// first) and `count` says how many are used; zero is a solid line, and so is
/// a pattern whose spans are all zero or negative.
fn dash_pattern_on(dist_px: f32, dash: array<vec4<f32>, 2>, count: u32) -> bool {
    if count == 0u {
        return true;
    }
    // A local copy, so the spans can be indexed at run time on every
    // backend. Clamp to the 8-scalar capacity so a bad CPU-side count cannot
    // index past the array.
    var spans = dash;
    let n = min(count, 8u);
    var period = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        period = period + max(spans[i / 4u][i % 4u], 0.0);
    }
    if period <= 0.0 {
        return true;
    }
    // The square-cap extension can push dist_px slightly negative at the
    // polyline start — wrap into [0, period) with the sign-safe form.
    let phase = ((dist_px % period) + period) % period;
    var acc = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        let end = acc + max(spans[i / 4u][i % 4u], 0.0);
        acc = end;
        if phase <= end || i == n - 1u {
            // Even spans are "on", odd spans are "off". Dash membership is
            // intentionally binary so the requested pattern length stays
            // stable; antialiasing is applied to the stroke outline itself.
            return (i & 1u) == 0u;
        }
    }
    // phase == period can occur from float rounding; that point is the
    // start of the next repetition, which always opens with an "on" span.
    return true;
}

// Deliberately duplicated per data shader (scatter/line/errorbar/pick) and
// NOT in the SHADER_COMMON.md common block: line_arc.wgsl shares that block
// but has no use for noise.
fn sketch_hash01(i: u32, seed: u32) -> f32 {
    var h = (i * 0x9E3779B9u) ^ (seed * 0x85EBCA6Bu);
    h = (h ^ (h >> 16u)) * 0x45D9F3Bu;
    h = h ^ (h >> 16u);
    return f32(h) / 4294967296.0;
}

// [-1, 1], C1-continuous. Negative t is fine: the lattice index wraps in u32
// (floor(-0.3) → -1 → 0xFFFFFFFF, and +1u wraps back to 0), so continuity at
// integer boundaries is preserved.
fn sketch_noise(t: f32, seed: u32) -> f32 {
    let i = u32(i32(floor(t)));
    let f = fract(t);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(sketch_hash01(i, seed), sketch_hash01(i + 1u, seed), u) * 2.0 - 1.0;
}

/// The sketch line's perpendicular displacement at arc position `arc_px`:
/// `amplitude_px · noise(arc / wavelength)`. A wavelength <= 0 disables the
/// wobble — mirrors the CPU stroker's guard (sketch.rs) so GPU and deco
/// layers degrade identically.
fn sketch_line_offset(arc_px: f32, amplitude_px: f32, wavelength_px: f32, seed: u32) -> f32 {
    let wobble = max(amplitude_px, 0.0) * sketch_noise(arc_px / max(wavelength_px, 1e-6), seed);
    return select(0.0, wobble, wavelength_px > 0.0);
}

// ───── end shared line stroke ─────

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

const AREA_FILL_TO_SERIES: u32 = 16u;

fn line_step_mode() -> u32 {
    let step = (style.primitive_flags >> LINE_STEP_SHIFT) & LINE_STEP_MASK;
    // Polar arc pieces ride the transform (Config.polar), not the style.
    let arc = u32(max(transform.polar_extent.z, 0.0));
    return step | (arc << LINE_ARC_SHIFT);
}

// Summed pixel length of the pieces before piece `k` — where piece `k`
// starts along the segment's own arc.
fn step_prefix_px(in: VsIn, mode: u32, k: u32) -> f32 {
    var total = 0.0;
    for (var j = 0u; j < k; j = j + 1u) {
        total = total + step_piece_len_px(step_piece(in.x_a, in.y_a, in.x_b, in.y_b, mode, j));
    }
    return total;
}

// Strip position → (piece, local vertex) for an instance made of `pieces`
// strips of `per_piece` vertices each, joined by two duplicated vertices per
// joint: the last vertex of the piece and the first of the next, which fold
// the connecting triangles to zero area. Total = pieces · per_piece +
// 2 · (pieces − 1); a one-piece instance is exactly the plain strip. CPU
// twin: `line_strip_vertex_count` in data_render/mod.rs.
struct StripSlot {
    piece: u32,
    local: u32,
};

fn strip_slot(vid: u32, per_piece: u32) -> StripSlot {
    let block = vid / (per_piece + 2u);
    let r = vid % (per_piece + 2u);
    var slot: StripSlot;
    slot.piece = block;
    slot.local = r;
    if (r == per_piece) {
        slot.local = per_piece - 1u;
    }
    if (r == per_piece + 1u) {
        slot.piece = block + 1u;
        slot.local = 0u;
    }
    return slot;
}

// Bound solid-line geometry before extrusion and SDF interpolation. Hardware
// clipping an enormous quad loses its pixel-wide sides (and the distance to
// the near endpoint) at deep zoom. Keep offscreen endpoints connected by
// clipping the segment, never by rejecting its individual points.
fn clip_solid_segment(first: vec2<f32>, second: vec2<f32>, pad: vec2<f32>) -> mat2x2<f32> {
    var a = first;
    var b = second;
    for (var plane = 0u; plane < 4u; plane++) {
        let axis = plane / 2u;
        let upper = (plane & 1u) != 0u;
        let bound = select(-1.0 - pad[axis], 1.0 + pad[axis], upper);
        let a_out = select(a[axis] < bound, a[axis] > bound, upper);
        let b_out = select(b[axis] < bound, b[axis] > bound, upper);
        if (a_out && b_out) {
            // Degenerate, outside the viewport.
            return mat2x2<f32>(vec2<f32>(2.0), vec2<f32>(2.0));
        }
        if (a_out != b_out) {
            // Anchor interpolation at the INSIDE endpoint. a + t*(b-a)
            // with t nearly one cancels the visible position when a is far away.
            let inside = select(a, b, a_out);
            let outside = select(b, a, a_out);
            let t = (bound - inside[axis]) / (outside[axis] - inside[axis]);
            var intersection = inside + (outside - inside) * t;
            intersection[axis] = bound;
            if (a_out) { a = intersection; } else { b = intersection; }
        }
    }
    return mat2x2<f32>(a, b);
}

@vertex
fn vs_main(in: VsIn, arc: VsArc, @builtin(vertex_index) vid: u32) -> VsOut {
    if (!sample_valid(in.x_a, in.y_a) || !sample_valid(in.x_b, in.y_b)) {
        var masked: VsOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    // One quad per step piece; a linear line is the single piece a → b.
    let mode = line_step_mode();
    let slot = strip_slot(vid, 4u);
    let piece = step_piece(in.x_a, in.y_a, in.x_b, in.y_b, mode, slot.piece);
    let last_piece = slot.piece + 1u == step_piece_count(mode);
    var a_ndc = data_to_ndc(piece.x0, piece.y0);
    var b_ndc = data_to_ndc(piece.x1, piece.y1);
    if (style.dash_len == 0u) {
        let pad = abs(transform.pixel_to_ndc) * (max(style.line_width_px, 1.0) + 2.0);
        let clipped = clip_solid_segment(a_ndc, b_ndc, pad);
        a_ndc = clipped[0];
        b_ndc = clipped[1];
    }

    // Convert to pixel space so the direction vector can be normalized.
    let a_px = a_ndc / transform.pixel_to_ndc;
    let b_px = b_ndc / transform.pixel_to_ndc;

    let delta = b_px - a_px;
    let len = length(delta);
    // Zero-length guard — normal=0 makes corner=center, collapsing the quad.
    let dir = select(vec2<f32>(0.0, 0.0), delta / max(len, 1e-6), len > 1e-6);
    let normal_px = vec2<f32>(-dir.y, dir.x);

    let half_w = max(style.line_width_px, 1.0) * 0.5;
    // Dashed lines keep their historical geometry so the dash pattern's
    // measured on/off lengths do not shift. Solid strokes get an inflated
    // fringe that the fragment SDF turns into analytic AA.
    let aa_extent = select(LINE_AA_EXTENT_PX, 0.0, style.dash_len > 0u);
    let geom_half_w = half_w + aa_extent;

    let at_b = slot.local >= 2u;
    let on_pos = (slot.local & 1u) == 1u;
    let center_px = select(a_px, b_px, at_b);
    let side = select(-1.0, 1.0, on_pos);

    // Square caps: extend the quad by half_w along the segment direction on
    // both ends. Adjacent quads then overlap at the joint, which keeps the
    // stroke CONTINUOUS when segments are sub-pixel (dense / noisy data
    // flips the direction at every point; butt-ended quads degenerate into
    // disconnected slivers there — the "stippled line" artifact). Step
    // pieces get the same treatment at their corners.
    let cap = select(-geom_half_w, geom_half_w, at_b);
    let corner_px = center_px + dir * cap + normal_px * geom_half_w * side;
    let corner_ndc = corner_px * transform.pixel_to_ndc;

    // Dash phase: this piece starts at the segment's arc plus the pieces
    // before it and ends at the scanned prefix for B on the last piece, so
    // the phase is continuous through every corner and across the joint.
    let arc_start = arc.arc_a + step_prefix_px(in, mode, slot.piece);
    let arc_end = select(arc_start + len, arc.arc_b, last_piece);

    var out: VsOut;
    out.pos = vec4<f32>(corner_ndc, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.dist_px = select(arc_start - half_w, arc_end + half_w, at_b);
    out.along_px = select(-geom_half_w, len + geom_half_w, at_b);
    out.side_px = geom_half_w * side;
    out.segment_len_px = len;
    return out;
}

fn stroke_alpha(in: VsOut) -> f32 {
    let half_w = max(style.line_width_px, 1.0) * 0.5;
    let signed_dist = line_stroke_distance(in.along_px, in.side_px, in.segment_len_px, half_w);
    let aa = max(fwidth(signed_dist), 0.5);
    return 1.0 - smoothstep(-aa, aa, signed_dist);
}

fn dash_alpha(in: VsOut) -> f32 {
    return select(0.0, 1.0, dash_pattern_on(in.dist_px, style.dash, style.dash_len));
}

fn line_fragment_color(in: VsOut, color: vec4<f32>) -> vec4<f32> {
    let alpha = stroke_alpha(in) * dash_alpha(in);
    if alpha <= 0.0 {
        discard;
    }
    return color * alpha;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    return line_fragment_color(in, style.color_premul);
}

@fragment
fn fs_constellation_line(in: VsOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let a = clamp(transform.style_params[0].y, 0.0, 1.0);
    return line_fragment_color(in, vec4<f32>(style.color_premul.rgb * a, style.color_premul.a * a));
}

// ───────────────── area fill (NOT part of the common block) ─────────────────
// `vs_area` fills under the line or between two lines. Slots 0-3 are the
// line's own (x_a, y_a, x_b, y_b); slots 4/5 carry the second y column as
// (hi, lo) pairs with the same one-value shift (fill to a series), or the y
// column itself as inert filler (fill to a baseline, read from
// Style.dash[1].xy — the bar shader's baseline convention). Each step piece
// is one trapezoid (p0, q0, p1, q1): the line piece on top, its base piece
// below, both produced by the same `step_piece` so their x corners agree.
// Hardware clipping is enough here — there is no stroke SDF to protect.
struct VsAreaBase {
    @location(4) base_a: vec2<f32>,
    @location(5) base_b: vec2<f32>,
};

struct AreaOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) polar_px: vec2<f32>,
    @location(1) @interpolate(flat) polar_limits: vec4<f32>,
    @location(2) @interpolate(flat) polar_sector: vec4<f32>,
};

fn area_baseline() -> vec2<f32> {
    return vec2<f32>(style.dash[1].x, style.dash[1].y);
}

// A baseline at or below zero on a logarithmic y axis has no position. The
// line's samples would be masked there; masking the baseline would delete the
// whole area instead, so it is clamped to the axis minimum (data_t = 0). This
// is the documented exception to the sample_valid rule.
fn area_baseline_ndc_y(base: vec2<f32>) -> f32 {
    let value = base.x + base.y;
    if (transform.scale_log.y != 0.0 && !(value > 0.0)) {
        return transform.data_to_panel_offset.y * 2.0 - 1.0;
    }
    return data_to_ndc(vec2<f32>(1.0, 0.0), base).y;
}

@vertex
fn vs_area(in: VsIn, base: VsAreaBase, @builtin(vertex_index) vid: u32) -> AreaOut {
    var masked: AreaOut;
    masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    if (!sample_valid(in.x_a, in.y_a) || !sample_valid(in.x_b, in.y_b)) {
        return masked;
    }
    let to_series = (style.primitive_flags & AREA_FILL_TO_SERIES) != 0u;
    if (to_series && (!sample_valid(in.x_a, base.base_a) || !sample_valid(in.x_b, base.base_b))) {
        // A gap in either line breaks the fill exactly where it breaks the
        // line.
        return masked;
    }
    let baseline = area_baseline();
    if (!to_series && (!sample_finite(baseline.x) || !sample_finite(baseline.y))) {
        return masked;
    }

    let mode = line_step_mode();
    let slot = strip_slot(vid, 4u);
    let top = step_piece(in.x_a, in.y_a, in.x_b, in.y_b, mode, slot.piece);
    // The base path steps with the same x corners: a baseline is constant, a
    // second series follows this series' interpolation.
    let base_ya = select(baseline, base.base_a, to_series);
    let base_yb = select(baseline, base.base_b, to_series);
    let bottom = step_piece(in.x_a, base_ya, in.x_b, base_yb, mode, slot.piece);

    let at_b = slot.local >= 2u;
    let on_base = (slot.local & 1u) == 1u;
    let x = select(top.x0, top.x1, at_b);
    let y_top = select(top.y0, top.y1, at_b);
    let y_base = select(bottom.y0, bottom.y1, at_b);
    var ndc = data_to_ndc(x, y_top);
    if (on_base) {
        // A polar baseline is a ring (the inner ring at or below the axis
        // start, which is the centre when `inner_radius_fraction` is 0), so
        // the base corner projects like a sample at the same angle.
        if (to_series || transform.polar_angle.x > 0.5) {
            ndc = data_to_ndc(x, y_base);
        } else {
            ndc.y = area_baseline_ndc_y(y_base);
        }
    }
    var out: AreaOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    return out;
}

@fragment
fn fs_area(in: AreaOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    return style.color_premul;
}

// ──────────────── sketch mode (NOT part of the common block) ────────────────
// Hand-drawn entry point selected as a separate pipeline variant. The precise
// entries above never read the sketch Transform fields.


// Subdivide each sketch line segment into S spans (k = vid/2 ∈ 0..=S,
// side = vid%2, t = k/S), displace the midline perpendicularly by
// amplitude · noise(arc_px/wavelength, seed),
// then extrude ±half_w. Arc-length parameterization makes the displacement
// continuous across the shared endpoint of adjacent segments. The outer
// points keep vs_main's square-cap extension so joints stay seamless, and
// dash phase still comes from the same cumulative arc interpolation as vs_main, so
// dashed+sketch composes for free. Invalid endpoints are explicitly masked
// before extrusion, as in the precise path.
@vertex
fn vs_sketch(in: VsIn, arc: VsArc, @builtin(vertex_index) vid: u32) -> VsOut {
    if (!sample_valid(in.x_a, in.y_a) || !sample_valid(in.x_b, in.y_b)) {
        var masked: VsOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    // Each step piece is its own 2·(S+1) strip, joined like vs_main's quads.
    let mode = line_step_mode();
    let slot = strip_slot(vid, 2u * (SKETCH_SUBDIV + 1u));
    let piece = step_piece(in.x_a, in.y_a, in.x_b, in.y_b, mode, slot.piece);
    let last_piece = slot.piece + 1u == step_piece_count(mode);
    let a_ndc = data_to_ndc(piece.x0, piece.y0);
    let b_ndc = data_to_ndc(piece.x1, piece.y1);

    let a_px = a_ndc / transform.pixel_to_ndc;
    let b_px = b_ndc / transform.pixel_to_ndc;

    let delta = b_px - a_px;
    let len = length(delta);
    let dir = select(vec2<f32>(0.0, 0.0), delta / max(len, 1e-6), len > 1e-6);
    let normal_px = vec2<f32>(-dir.y, dir.x);

    let half_w = max(style.line_width_px, 1.0) * 0.5;
    let aa_extent = select(LINE_AA_EXTENT_PX, 0.0, style.dash_len > 0u);
    let geom_half_w = half_w + aa_extent;

    let k = slot.local / 2u;
    let on_pos = (slot.local & 1u) == 1u;
    let t = f32(k) / f32(SKETCH_SUBDIV);
    let center_px = mix(a_px, b_px, t);
    let side = select(-1.0, 1.0, on_pos);

    // Square caps on the outer subdivision points only (same rule and same
    // rationale as vs_main).
    var cap = 0.0;
    if (k == 0u) { cap = -geom_half_w; }
    if (k == SKETCH_SUBDIV) { cap = geom_half_w; }

    let arc_start = arc.arc_a + step_prefix_px(in, mode, slot.piece);
    let arc_end = select(arc_start + len, arc.arc_b, last_piece);
    let arc_at = mix(arc_start, arc_end, t);
    let seed = u32(transform.style_params[0].z) ^ style.series_salt;
    let disp = sketch_line_offset(arc_at, transform.style_params[0].x, transform.style_params[0].y, seed);

    let corner_px = center_px + dir * cap + normal_px * (geom_half_w * side + disp);
    let corner_ndc = corner_px * transform.pixel_to_ndc;

    var out: VsOut;
    out.pos = vec4<f32>(corner_ndc, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.dist_px = arc_at + select(0.0, cap, style.dash_len == 0u);
    if (style.dash_len > 0u) {
        out.dist_px = arc_at + select(0.0, -half_w, k == 0u) + select(0.0, half_w, k == SKETCH_SUBDIV);
    }
    out.along_px = t * len + cap;
    out.side_px = geom_half_w * side;
    out.segment_len_px = len;
    return out;
}

// ────────────── constellation mode (NOT part of the common block) ───────────
// Star-chain line style. Two entries reuse the line pipeline's six instance
// slots:
//   vs_ribbon/fs_ribbon — the unresolved-starlight haze (series-colored
//                         nebula band; this is what separates two series),
//   vs_stars/fs_stars   — individual stars scattered along the arc.
// Both render ADDITIVELY onto a dark backdrop; alpha is written as the max
// channel so straight-alpha consumers (the PNG export) unpremultiply sanely.
// Every star attribute derives from (arc length, seed) hashes in-shader —
// the column pool keeps no CPU copies and nothing here changes that.

// Style-texture bindings (group 2). Only the constellation entries reference
// them; pipelines built for the other entries omit this bind group layout.
@group(2) @binding(0) var cons_psf_tex: texture_2d<f32>; // R=core, G=halo
@group(2) @binding(1) var cons_lut_tex: texture_2d<f32>; // 256×1 blackbody
@group(2) @binding(2) var cons_samp: sampler;

// Arc wavelength (px) of the slow "clump" modulation shared by ribbon
// brightness and population temperature. Star counts stay uniform per arc
// length so data-sampling density does not look like extra stars.
const CONS_CLUMP_WAVELENGTH_PX: f32 = 90.0;
// CPU draw-count twin lives in mod.rs: MILKYWAY_RIBBON_VERTICES =
// 2·(SUBDIV+1). The star pass has no fixed vertex count — it draws via
// DrawIndirect args the arc-scan kernel fills (line_arc.wgsl star_indirect).
const CONS_RIBBON_SUBDIV: u32 = 8u;

// Resolution-invariance factor for px-denominated structure constants
// (style_params[2].x = structure_scale, 1.0 live / export scale on export).
// Without it a 2× export halves the clump wavelength relative to the data
// and the chains read as a different, busier texture.
fn cons_structure_scale() -> f32 {
    return max(transform.style_params[2].x, 1e-3);
}

// Local ribbon/temperature modulation, 0.35 .. 1.65.
fn cons_clump(arc_px: f32, seed: u32) -> f32 {
    let wavelength = CONS_CLUMP_WAVELENGTH_PX * cons_structure_scale();
    return 1.0 + 0.65 * sketch_noise(arc_px / wavelength, seed ^ 0xC10Du);
}

// Stellar population mix 0..1: 0 = old/warm (dense knots), 1 = young/hot.
fn cons_pop(arc_px: f32, seed: u32) -> f32 {
    let wavelength = CONS_CLUMP_WAVELENGTH_PX * cons_structure_scale();
    return 0.5 + 0.5 * sketch_noise(arc_px / wavelength, seed ^ 0x090Bu);
}

struct RibbonOut {
    @builtin(position) pos: vec4<f32>,
    // Cross-strip coordinate in [-1, 1] (edge = one full ribbon width from
    // the centerline — see the geometric-width note in vs_ribbon).
    @location(0) cross_d: f32,
    // Centerline intensity (config intensity × clump noise), computed in the
    // vertex stage: the shared transform bind group is vertex-visible only,
    // and the clump wavelength (90 px) is far above the subdivision spacing,
    // so linear interpolation across the strip is lossless in practice.
    @location(1) center_i: f32,
    // Longitudinal cap coordinate: 0 over the segment body, →1 at the tip of
    // the square-cap extension. Fades the cap so true polyline ends finish
    // softly; at interior joints the neighbor's full-intensity body wins
    // under the ribbon's MAX blending.
    @location(2) cap_d: f32,
    @location(3) polar_px: vec2<f32>,
    @location(4) @interpolate(flat) polar_limits: vec4<f32>,
    @location(5) @interpolate(flat) polar_sector: vec4<f32>,
};

@vertex
fn vs_ribbon(in: VsIn, arc: VsArc, @builtin(vertex_index) vid: u32) -> RibbonOut {
    if (!sample_valid(in.x_a, in.y_a) || !sample_valid(in.x_b, in.y_b)) {
        var masked: RibbonOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    // Step pieces: one ribbon strip per piece, joined like vs_main's quads.
    let mode = line_step_mode();
    let slot = strip_slot(vid, 2u * (CONS_RIBBON_SUBDIV + 1u));
    let piece = step_piece(in.x_a, in.y_a, in.x_b, in.y_b, mode, slot.piece);
    let last_piece = slot.piece + 1u == step_piece_count(mode);
    let a_px = data_to_ndc(piece.x0, piece.y0) / transform.pixel_to_ndc;
    let b_px = data_to_ndc(piece.x1, piece.y1) / transform.pixel_to_ndc;
    let delta = b_px - a_px;
    let len = length(delta);
    let dir = select(vec2<f32>(0.0, 0.0), delta / max(len, 1e-6), len > 1e-6);
    let normal_px = vec2<f32>(-dir.y, dir.x);

    // Geometric half-extent = one FULL ribbon width: the gaussian profile in
    // fs_ribbon reaches ~6% of peak at the strip edge, so the geometric cut
    // is invisible under additive blending.
    let half_geom = max(transform.style_params[0].y, 1.0);

    let k = slot.local / 2u;
    let t = f32(k) / f32(CONS_RIBBON_SUBDIV);
    let side = select(-1.0, 1.0, (slot.local & 1u) == 1u);

    // Square-cap extension on both strip ends, like vs_main: adjacent
    // segments then overlap at every joint, sealing the outer-bend wedge
    // gaps a butt-ended strip leaves on curves. The ribbon pipeline blends
    // with MAX (not ADD), so the overlap cannot double-brighten.
    var cap = 0.0;
    if (k == 0u) { cap = -half_geom; }
    if (k == CONS_RIBBON_SUBDIV) { cap = half_geom; }

    let center_px = mix(a_px, b_px, t) + dir * cap;
    let corner_px = center_px + normal_px * half_geom * side;

    let seed = u32(transform.style_params[0].w) ^ style.series_salt;
    let intensity_cfg = max(transform.style_params[0].z, 0.0);
    let arc_start = arc.arc_a + step_prefix_px(in, mode, slot.piece);
    let arc_end = select(arc_start + len, arc.arc_b, last_piece);
    let arc_px = mix(arc_start, arc_end, t);

    var out: RibbonOut;
    out.pos = vec4<f32>(corner_px * transform.pixel_to_ndc, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.cross_d = side;
    out.center_i = intensity_cfg * cons_clump(arc_px, seed);
    out.cap_d = abs(cap) / max(half_geom, 1e-6);
    return out;
}

@fragment
fn fs_ribbon(in: RibbonOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    // FWHM = ribbon_width_px: cross_d is in strip units where the edge sits
    // one full width out, so exp(-2.77 d²) halves at d = 0.5 (= width/2).
    // The same falloff runs along the cap extension so free polyline ends
    // fade out instead of cutting square.
    let gauss = exp(-2.77 * (in.cross_d * in.cross_d + in.cap_d * in.cap_d));
    let i = in.center_i * gauss;
    return vec4<f32>(style.color_premul.rgb * i, i);
}

struct StarOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Blackbody tint and power-law brightness — constant across the star's
    // quad (flat-ish via plain interpolation of equal corner values).
    @location(1) tint: vec3<f32>,
    @location(2) brightness: f32,
    @location(3) polar_px: vec2<f32>,
    @location(4) @interpolate(flat) polar_limits: vec4<f32>,
    @location(5) @interpolate(flat) polar_sector: vec4<f32>,
};

// Per-star hash channel: one lattice index per candidate slot, salted per
// attribute so channels are independent.
fn cons_h(id: u32, salt: u32, seed: u32) -> f32 {
    return sketch_hash01(id, seed ^ salt);
}

// Arc-driven star pass data (group 3). The star pass binds NO vertex
// buffers: each instance is one candidate slot along the polyline's TOTAL
// arc, and the VS locates its segment by binary search over the arc-length
// prefix, fetching endpoints straight from the column pool. Star density is
// therefore independent of how densely the data samples the line — the old
// per-segment quad budget saturated on sparse polylines (a single long
// segment capped at 24 stars no matter the requested density).
struct StarVsParams {
    n_points: u32,
    x_base: u32,
    y_base: u32,
    // Step interpolation mode of the polyline (LINE_STEP_*), so a star's arc
    // position lands on the same stepped piece the ribbon and scan used.
    mode: u32,
};
@group(3) @binding(0) var<storage, read> star_arc: array<f32>;
@group(3) @binding(1) var<storage, read> star_pool: array<f32>;
@group(3) @binding(2) var<uniform> star_vsp: StarVsParams;

fn star_pool_pair(base: u32, i: u32) -> vec2<f32> {
    let j = base + i * 2u;
    return vec2<f32>(star_pool[j], star_pool[j + 1u]);
}

// Candidate slot pitch in arc px. CPU twin: line_arc.rs
// STAR_SLOT_PITCH_FACTOR sizes the indirect dispatch with the same formula —
// the two must agree or slot positions and the dispatch count diverge.
// Pitch scales with structure_scale, so the slot grid (and with it the
// whole star pattern) is resolution-invariant, and density changes only
// flip the existence of FIXED candidates instead of reshuffling them.
fn cons_star_pitch() -> f32 {
    return max(0.5 * cons_structure_scale(), 1e-3);
}

@vertex
fn vs_stars(
    @builtin(vertex_index) vid: u32,
    @builtin(instance_index) slot: u32,
) -> StarOut {
    let corner = vid % 6u;

    let density = max(transform.style_params[0].x, 0.0); // stars / 100 px arc
    let seed = u32(transform.style_params[0].w) ^ style.series_salt;
    let star_scale = max(transform.style_params[1].x, 0.0);
    let spread = max(transform.style_params[1].y, 0.0);
    let star_brightness = clamp(transform.style_params[2].y, 0.0, 8.0);
    let pitch = cons_star_pitch();

    let n = star_vsp.n_points;
    let total = star_arc[n - 1u];

    // ~22% of slots re-anchor beside the PREVIOUS slot's star as a dim close
    // companion — re-deriving the primary's hashes is deterministic and free,
    // and binary pairs are a strong realism cue.
    let is_companion = cons_h(slot, 0x0B17u, seed) < 0.22 && slot > 0u;
    let primary_id = select(slot, slot - 1u, is_companion);

    // Stratified arc position: one candidate per `pitch` px of arc, jittered
    // inside its stratum. Attributes hash slot IDS (not positions), so the
    // pattern upstream of a data edit stays put.
    let arc_here =
        (f32(primary_id) + 0.5 + 0.8 * (cons_h(primary_id, 0x0701u, seed) - 0.5)) * pitch;

    // Existence gate: expected stars per slot = density * pitch / 100.
    // Keep this independent of the clump field so the line is sewn uniformly
    // by arc length rather than looking denser around particular samples.
    let p_exist = clamp(density * pitch / 100.0, 0.0, 1.0);
    var alive = cons_h(slot, 0x0E15u, seed) < p_exist && arc_here < total;

    // Binary search: greatest g with prefix[g] <= arc_here; segment (g, g+1).
    var lo = 0u;
    var hi = n - 1u;
    loop {
        if (hi - lo <= 1u) { break; }
        let mid = (lo + hi) >> 1u;
        if (star_arc[mid] <= arc_here) { lo = mid; } else { hi = mid; }
    }
    let seg_a = star_arc[lo];
    let seg_len = star_arc[lo + 1u] - seg_a;
    // NaN-gap segments contribute zero prefix advance — a slot landing on a
    // flat span (boundary ties) dies instead of drawing inside a gap.
    if (seg_len <= 0.0) { alive = false; }
    let t = clamp((arc_here - seg_a) / max(seg_len, 1e-6), 0.0, 1.0);

    if (!sample_valid(star_pool_pair(star_vsp.x_base, lo), star_pool_pair(star_vsp.y_base, lo))
        || !sample_valid(star_pool_pair(star_vsp.x_base, lo + 1u), star_pool_pair(star_vsp.y_base, lo + 1u))) {
        var masked: StarOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }

    // Walk the segment's step pieces to the one holding `arc_here`; a linear
    // polyline has the single piece lo → lo + 1 and keeps t as computed.
    let x_lo = star_pool_pair(star_vsp.x_base, lo);
    let y_lo = star_pool_pair(star_vsp.y_base, lo);
    let x_hi = star_pool_pair(star_vsp.x_base, lo + 1u);
    let y_hi = star_pool_pair(star_vsp.y_base, lo + 1u);
    var piece = step_piece(x_lo, y_lo, x_hi, y_hi, star_vsp.mode, 0u);
    var t_piece = t;
    let pieces = step_piece_count(star_vsp.mode);
    if (pieces > 1u) {
        var arc_in = arc_here - seg_a;
        var k = 0u;
        loop {
            piece = step_piece(x_lo, y_lo, x_hi, y_hi, star_vsp.mode, k);
            let piece_len = step_piece_len_px(piece);
            if (k + 1u >= pieces || arc_in <= piece_len) {
                t_piece = clamp(arc_in / max(piece_len, 1e-6), 0.0, 1.0);
                break;
            }
            arc_in = arc_in - piece_len;
            k = k + 1u;
        }
    }
    let a_px = data_to_ndc(piece.x0, piece.y0) / transform.pixel_to_ndc;
    let b_px = data_to_ndc(piece.x1, piece.y1) / transform.pixel_to_ndc;
    let delta = b_px - a_px;
    let len = length(delta);
    let dir = select(vec2<f32>(0.0, 0.0), delta / max(len, 1e-6), len > 1e-6);
    let normal_px = vec2<f32>(-dir.y, dir.x);
    // Also kills non-finite endpoints (NaN comparisons are false).
    if (!(len > 1e-6)) { alive = false; }

    // Perpendicular scatter: sum of two uniforms → triangular ≈ gaussian-ish.
    let g1 = cons_h(primary_id, 0x0FF1u, seed);
    let g2 = cons_h(primary_id, 0x0FF2u, seed);
    let off = (g1 + g2 - 1.0) * spread * 1.7;

    // Brightness power law — most stars faint, rare bright anchors. The
    // exponent (faint_bias, style_params[1].z) is the luminosity-function
    // slope: higher = more faint dust per anchor.
    let faint_bias = clamp(transform.style_params[1].z, 0.5, 24.0);
    let u_b = cons_h(primary_id, 0x86A9u, seed);
    var b = 0.12 + 0.88 * pow(u_b, faint_bias);
    if (is_companion) {
        b = b * 0.35;
    }

    // Temperature is a population mix driven by local density, then mapped
    // through the blackbody LUT. Vertex stages use textureLoad because they
    // have no implicit derivatives.
    let pop = clamp(cons_pop(arc_here, seed), 0.0, 1.0);
    let h_t = cons_h(primary_id, 0x7E47u, seed);
    let t_norm = mix(mix(0.04, 0.30, h_t), mix(0.45, 0.95, h_t), pop);
    let tint = textureLoad(
        cons_lut_tex,
        vec2<i32>(i32(clamp(t_norm, 0.0, 1.0) * 255.0), 0),
        0,
    ).rgb;

    // Star radius (px): brighter → bigger (PSF wings cross the saturation
    // threshold further out). The quad leaves 4× room for the halo.
    let r_star = star_scale * (0.9 + 3.4 * pow(b, 0.7));
    let quad_half = r_star * 4.0;

    var center = mix(a_px, b_px, t_piece) + normal_px * off;
    if (is_companion) {
        let ang = cons_h(slot, 0x0A46u, seed) * 6.2831853;
        // px-denominated structure constant — scales with the resolution
        // factor so binary pairs keep their separation relative to the stars.
        let sep = (2.0 + 2.0 * cons_h(slot, 0x0D15u, seed)) * cons_structure_scale();
        center = center + vec2<f32>(cos(ang), sin(ang)) * sep;
    }

    var c: vec2<f32>;
    switch corner {
        case 0u, 3u: { c = vec2<f32>(-1.0, -1.0); }
        case 1u: { c = vec2<f32>(1.0, -1.0); }
        case 2u, 4u: { c = vec2<f32>(1.0, 1.0); }
        default: { c = vec2<f32>(-1.0, 1.0); }
    }
    // Dead slots collapse to zero area.
    let q = quad_half * select(0.0, 1.0, alive);
    let corner_px = center + c * q;

    var out: StarOut;
    out.pos = vec4<f32>(corner_px * transform.pixel_to_ndc, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.uv = c * 0.5 + vec2<f32>(0.5, 0.5);
    out.tint = tint;
    out.brightness = b * star_brightness;
    return out;
}

@fragment
fn fs_stars(in: StarOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    // Explicit LOD by repo rule (shader_consistency lint) — single-mip PSF.
    let s = textureSampleLevel(cons_psf_tex, cons_samp, in.uv, 0.0);
    // Keep the saturated core white and carry star color in the
    // blackbody-tinted halo.
    let col = (vec3<f32>(1.0) * s.r + in.tint * s.g) * in.brightness;
    let a = max(col.r, max(col.g, col.b));
    return vec4<f32>(col, a);
}
