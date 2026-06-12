// Columnar errorbar shader.
//
// Six columns (x, y, err_y_lo, err_y_hi, err_x_lo, err_x_hi) bound as
// per-instance slots. Each instance produces 36 vertices on a TriangleList:
// six axis-aligned quads (6 vertices each) — Y stem, cap @ y_lo, cap @ y_hi,
// X stem, cap @ x_lo, cap @ x_hi. Stem endpoints live in data space; caps
// span ±style.cap_half_px (pixels) perpendicular to their stem. Each quad is
// expanded perpendicular to its own axis by half its stroke width in pixels
// (stems: style.line_width_px, caps: style.cap_width_px); pixel offsets are
// converted to NDC via transform.pixel_to_ndc.
//
// To draw only one direction, fill the unused err columns with zeros (or
// share a single zero-filled column): a direction with err_lo + err_hi <= 0
// collapses to zero-area quads, so neither its stem nor its caps rasterize.
//
// `scale_log.{x,y}` = 1.0 applies log10 to both the input coords and the
// computed err endpoints.

// ───── BEGIN common block (SHADER_COMMON.md) ─────
// WGSL has no import. The Transform/Style/binding/maybe_log/data_to_ndc
// definitions below are duplicated across scatter/line/errorbar shaders.
// To modify any of them, FIRST edit src/data_render/SHADER_COMMON.md,
// then mirror the change into every sibling shader. Do not edit only one
// file — silent drift here causes very hard-to-debug rendering bugs.
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    // Generic per-panel style parameter slots. Interpretation belongs to the
    // ACTIVE style's shader entries; the precise entries never read them.
    // sketch:        [0] = (amplitude_px, wavelength_px, seed(f32), 0)
    // constellation: [0] = (star_density, ribbon_width_px, ribbon_intensity,
    //                seed(f32)), [1] = (star_scale, spread_px, faint_bias, planet_rim),
    //                [2] = (structure_scale, 0, 0, 0) — multiplier on the
    //                style's px-denominated structure constants (clump
    //                wavelength, binary separation); keeps the star texture
    //                resolution-invariant under DPI/export scaling.
    style_params: array<vec4<f32>, 3>,
};  // 80 B (vec4 array at offset 32, stride 16 — alignment unchanged)

@group(0) @binding(0) var<uniform> transform: Transform;

struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
    point_radius_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
    shape_id: u32,
    dash_len: u32,
    // Per-series decorrelation salt (FNV-1a of series_id). Styled entries
    // (sketch/constellation) XOR it into their hash seeds so two series never
    // share a star/wobble pattern; precise entries never read it.
    series_salt: u32,
    _pad: u32,
    dash: array<vec4<f32>, 2>,
};

@group(1) @binding(0) var<uniform> style: Style;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) x: f32,
    @location(1) y: f32,
    @location(2) err_y_lo: f32,
    @location(3) err_y_hi: f32,
    @location(4) err_x_lo: f32,
    @location(5) err_x_hi: f32,
};

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

@vertex
fn vs_main(in: VsIn) -> @builtin(position) vec4<f32> {
    // segment: 0 Y-stem, 1 cap@y_lo, 2 cap@y_hi, 3 X-stem, 4 cap@x_lo,
    // 5 cap@x_hi.
    let seg = in.vi / 6u;

    // Zero-filled err columns disable a whole direction, caps included:
    // collapse its segments to the anchor so every triangle has zero area.
    let has_y = (in.err_y_lo + in.err_y_hi) > 0.0;
    let has_x = (in.err_x_lo + in.err_x_hi) > 0.0;
    let dir_enabled = select(has_x, has_y, seg < 3u);
    if (!dir_enabled) {
        let anchor = data_to_ndc(vec2<f32>(in.x, in.y));
        return vec4<f32>(anchor, 0.0, 1.0);
    }

    let y_lo = in.y - in.err_y_lo;
    let y_hi = in.y + in.err_y_hi;
    let x_lo = in.x - in.err_x_lo;
    let x_hi = in.x + in.err_x_hi;

    // Each segment is a quad between endpoints A and B: a data-space
    // position plus a pixel-space offset (caps run ±cap_half_px from a
    // single data point). `perp` is the pixel-space expansion axis; all
    // segments are axis-aligned, so no normalization is needed.
    var a_data: vec2<f32>;
    var b_data: vec2<f32>;
    var a_px = vec2<f32>(0.0, 0.0);
    var b_px = vec2<f32>(0.0, 0.0);
    var perp: vec2<f32>;
    var half_stroke: f32;

    if (seg == 0u) {
        // Y stem: vertical in data space, expand along X.
        a_data = vec2<f32>(in.x, y_lo);
        b_data = vec2<f32>(in.x, y_hi);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 1u) {
        // Cap @ y_lo: horizontal, expand along Y.
        a_data = vec2<f32>(in.x, y_lo);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 2u) {
        // Cap @ y_hi: horizontal, expand along Y.
        a_data = vec2<f32>(in.x, y_hi);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 3u) {
        // X stem: horizontal in data space, expand along Y.
        a_data = vec2<f32>(x_lo, in.y);
        b_data = vec2<f32>(x_hi, in.y);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 4u) {
        // Cap @ x_lo: vertical, expand along X.
        a_data = vec2<f32>(x_lo, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -style.cap_half_px);
        b_px = vec2<f32>(0.0,  style.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.cap_width_px * 0.5;
    } else {
        // Cap @ x_hi: vertical, expand along X.
        a_data = vec2<f32>(x_hi, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -style.cap_half_px);
        b_px = vec2<f32>(0.0,  style.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.cap_width_px * 0.5;
    }

    // Triangle-list corner map [0,1,2, 2,1,3] over corners
    // {0: A-, 1: A+, 2: B-, 3: B+} (-/+ = perpendicular half-stroke side).
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[in.vi % 6u];
    let at_a = corner < 2u;
    let side = select(1.0, -1.0, (corner & 1u) == 0u);

    let end_data = select(b_data, a_data, at_a);
    let end_px = select(b_px, a_px, at_a);
    let offset_px = end_px + perp * (side * half_stroke);
    let ndc = data_to_ndc(end_data) + offset_px * transform.pixel_to_ndc;
    return vec4<f32>(ndc, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return style.color_premul;
}

// ──────────────── sketch mode (NOT part of the common block) ────────────────
// Hand-drawn entry point — design SSoT: docs/SKETCH_DESIGN.md (§3 noise,
// §5d errorbar). Selected as a separate pipeline variant; the precise entries
// above are never modified and never read the sketch Transform fields.
// fs_main is shared (it reads no varyings).

// 1D value-noise pair — original formula: docs/SKETCH_DESIGN.md §3.
// Deliberately duplicated per data shader (scatter/line/errorbar) and NOT in
// the SHADER_COMMON.md common block: line_arc.wgsl shares that block but has
// no use for noise. Keep the three copies in sync with the design doc.
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

// Sketch errorbar vertex stage (docs/SKETCH_DESIGN.md §5d): same six-quad
// construction as vs_main, but each quad END (A/B, along the bar-length
// parameter) is displaced perpendicular to its stroke by
// amplitude · noise(2·seg + end, seed + instance_index). The integer lattice
// input samples the noise at lattice points, giving every one of the 12 quad
// ends an independent deterministic offset; both corners of an end share it,
// so strokes tilt without changing width. No subdivision — bars are short,
// corner perturbation suffices. A disabled direction stays collapsed to the
// anchor WITHOUT wobble (a displaced zero-area quad would gain area and
// rasterize).
@vertex
fn vs_sketch(in: VsIn, @builtin(instance_index) inst: u32) -> @builtin(position) vec4<f32> {
    // segment: 0 Y-stem, 1 cap@y_lo, 2 cap@y_hi, 3 X-stem, 4 cap@x_lo,
    // 5 cap@x_hi.
    let seg = in.vi / 6u;

    let has_y = (in.err_y_lo + in.err_y_hi) > 0.0;
    let has_x = (in.err_x_lo + in.err_x_hi) > 0.0;
    let dir_enabled = select(has_x, has_y, seg < 3u);
    if (!dir_enabled) {
        let anchor = data_to_ndc(vec2<f32>(in.x, in.y));
        return vec4<f32>(anchor, 0.0, 1.0);
    }

    let y_lo = in.y - in.err_y_lo;
    let y_hi = in.y + in.err_y_hi;
    let x_lo = in.x - in.err_x_lo;
    let x_hi = in.x + in.err_x_hi;

    var a_data: vec2<f32>;
    var b_data: vec2<f32>;
    var a_px = vec2<f32>(0.0, 0.0);
    var b_px = vec2<f32>(0.0, 0.0);
    var perp: vec2<f32>;
    var half_stroke: f32;

    if (seg == 0u) {
        a_data = vec2<f32>(in.x, y_lo);
        b_data = vec2<f32>(in.x, y_hi);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 1u) {
        a_data = vec2<f32>(in.x, y_lo);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 2u) {
        a_data = vec2<f32>(in.x, y_hi);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 3u) {
        a_data = vec2<f32>(x_lo, in.y);
        b_data = vec2<f32>(x_hi, in.y);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 4u) {
        a_data = vec2<f32>(x_lo, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -style.cap_half_px);
        b_px = vec2<f32>(0.0,  style.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.cap_width_px * 0.5;
    } else {
        a_data = vec2<f32>(x_hi, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -style.cap_half_px);
        b_px = vec2<f32>(0.0,  style.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.cap_width_px * 0.5;
    }

    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[in.vi % 6u];
    let at_a = corner < 2u;
    let side = select(1.0, -1.0, (corner & 1u) == 0u);

    let amp = max(transform.style_params[0].x, 0.0);
    let seed = (u32(transform.style_params[0].z) ^ style.series_salt) + inst;
    let lattice = f32(seg * 2u + select(1u, 0u, at_a));
    let disp = amp * sketch_noise(lattice, seed);

    let end_data = select(b_data, a_data, at_a);
    let end_px = select(b_px, a_px, at_a);
    let offset_px = end_px + perp * (side * half_stroke + disp);
    let ndc = data_to_ndc(end_data) + offset_px * transform.pixel_to_ndc;
    return vec4<f32>(ndc, 0.0, 1.0);
}

// ────────────── constellation mode (NOT part of the common block) ───────────
// Bipolar-jet errorbars — docs/CONSTELLATION_DESIGN.md. The error range
// renders as a glowing astrophysical jet (think Herbig-Haro flows): the
// stem quads become tapered beams brightest at the data point, and the cap
// quads become terminal shock KNOTS — diffuse radial glows that mark the
// exact interval bounds. Series color tints the plasma; the hot cores
// whiten. Rendered ADDITIVELY; geometry (and therefore the indicated range)
// is identical to the precise errorbar.

struct JetOut {
    @builtin(position) pos: vec4<f32>,
    // Beams: x = cross-strip [-1,1], y = longitudinal t in [0,1] (A→B).
    // Knots: full square local coords [-1,1]².
    @location(0) local: vec2<f32>,
    // 0 = beam, 1 = knot.
    @location(1) @interpolate(flat) kind: u32,
    // Beam only: the data point's position along the bar (0..1) — the jet
    // source the brightness tapers away from.
    @location(2) @interpolate(flat) t_src: f32,
    @location(3) @interpolate(flat) seed_inst: u32,
};

// Halo room multipliers — the glow needs quad area beyond the core stroke.
const JET_BEAM_HALO: f32 = 4.0;
const JET_KNOT_HALO: f32 = 1.7;

@vertex
fn vs_jet(in: VsIn, @builtin(instance_index) inst: u32) -> JetOut {
    let seg = in.vi / 6u;

    var out: JetOut;
    out.seed_inst = (u32(transform.style_params[0].w) ^ style.series_salt) + inst;
    out.kind = select(1u, 0u, seg == 0u || seg == 3u);
    out.t_src = 0.5;
    out.local = vec2<f32>(0.0, 0.0);

    let has_y = (in.err_y_lo + in.err_y_hi) > 0.0;
    let has_x = (in.err_x_lo + in.err_x_hi) > 0.0;
    let dir_enabled = select(has_x, has_y, seg < 3u);
    if (!dir_enabled) {
        let anchor = data_to_ndc(vec2<f32>(in.x, in.y));
        out.pos = vec4<f32>(anchor, 0.0, 1.0);
        return out;
    }

    let y_lo = in.y - in.err_y_lo;
    let y_hi = in.y + in.err_y_hi;
    let x_lo = in.x - in.err_x_lo;
    let x_hi = in.x + in.err_x_hi;

    var a_data: vec2<f32>;
    var b_data: vec2<f32>;
    var a_px = vec2<f32>(0.0, 0.0);
    var b_px = vec2<f32>(0.0, 0.0);
    var perp: vec2<f32>;
    var half_stroke: f32;

    let beam_half = max(style.line_width_px * 0.5, 1.2) * JET_BEAM_HALO;
    let knot_half = max(style.cap_half_px, 3.0) * JET_KNOT_HALO;

    if (seg == 0u) {
        a_data = vec2<f32>(in.x, y_lo);
        b_data = vec2<f32>(in.x, y_hi);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = beam_half;
        out.t_src = in.err_y_lo / max(in.err_y_lo + in.err_y_hi, 1e-6);
    } else if (seg == 1u) {
        a_data = vec2<f32>(in.x, y_lo);
        b_data = a_data;
        a_px = vec2<f32>(-knot_half, 0.0);
        b_px = vec2<f32>(knot_half, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = knot_half;
    } else if (seg == 2u) {
        a_data = vec2<f32>(in.x, y_hi);
        b_data = a_data;
        a_px = vec2<f32>(-knot_half, 0.0);
        b_px = vec2<f32>(knot_half, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = knot_half;
    } else if (seg == 3u) {
        a_data = vec2<f32>(x_lo, in.y);
        b_data = vec2<f32>(x_hi, in.y);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = beam_half;
        out.t_src = in.err_x_lo / max(in.err_x_lo + in.err_x_hi, 1e-6);
    } else if (seg == 4u) {
        a_data = vec2<f32>(x_lo, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -knot_half);
        b_px = vec2<f32>(0.0, knot_half);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = knot_half;
    } else {
        a_data = vec2<f32>(x_hi, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -knot_half);
        b_px = vec2<f32>(0.0, knot_half);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = knot_half;
    }

    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[in.vi % 6u];
    let at_a = corner < 2u;
    let side = select(1.0, -1.0, (corner & 1u) == 0u);

    let end_data = select(b_data, a_data, at_a);
    let end_px = select(b_px, a_px, at_a);
    let offset_px = end_px + perp * (side * half_stroke);
    let ndc = data_to_ndc(end_data) + offset_px * transform.pixel_to_ndc;

    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    // Beams: (cross side, longitudinal 0/1). Knots: square corner coords —
    // the along-axis corner sign doubles as the local x.
    let along = select(1.0, -1.0, at_a);
    out.local = select(
        vec2<f32>(along, side),
        vec2<f32>(side, select(1.0, 0.0, at_a)),
        out.kind == 0u,
    );
    return out;
}

@fragment
fn fs_jet(in: JetOut) -> @location(0) vec4<f32> {
    let tint = style.color_premul.rgb;
    var col = vec3<f32>(0.0);
    if (in.kind == 0u) {
        // Beam: gaussian cross-profile (FWHM ≈ the core stroke width inside
        // the 4× halo quad), tapering away from the jet source, with a soft
        // wisp modulation so the plasma reads organic.
        let cross = exp(-11.0 * in.local.x * in.local.x);
        let dn = abs(in.local.y - in.t_src) / max(max(in.t_src, 1.0 - in.t_src), 1e-3);
        let taper = mix(1.0, 0.30, clamp(dn, 0.0, 1.0));
        let wisp = 0.85 + 0.30 * sketch_noise(in.local.y * 6.0, in.seed_inst);
        let i = 0.6 * cross * taper * wisp;
        col = tint * i + vec3<f32>(1.0) * (0.18 * cross * cross * taper);
    } else {
        // Terminal shock knot: hot whitened core + tinted halo, clipped at
        // the quad edge.
        let r = length(in.local);
        let lim = 1.0 - smoothstep(0.85, 1.0, r);
        let core = exp(-5.5 * r * r);
        let halo = exp(-2.2 * r);
        col = (tint * (1.1 * core + 0.35 * halo) + vec3<f32>(0.95) * 0.55 * core * core) * lim;
    }
    let a = max(col.r, max(col.g, col.b));
    if (a <= 0.003) {
        discard;
    }
    return vec4<f32>(col, a);
}
