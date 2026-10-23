// The sketch errorbar uses the same six-quad construction as vs_main, but each
// quad end (A/B along the bar-length parameter) is displaced perpendicular to
// its stroke by
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

    let has_y = (style.primitive_flags & ERRORBAR_HAS_Y) != 0u;
    let has_x = (style.primitive_flags & ERRORBAR_HAS_X) != 0u;
    let dir_enabled = select(has_x, has_y, seg < 3u);
    if (!dir_enabled) {
        let anchor = data_to_ndc(in.x, in.y);
        return vec4<f32>(anchor, 0.0, 1.0);
    }

    let y_lo = pair_sub(in.y, in.err_y_lo);
    let y_hi = pair_sum(in.y, in.err_y_hi);
    let x_lo = pair_sub(in.x, in.err_x_lo);
    let x_hi = pair_sum(in.x, in.err_x_hi);

    var a_data: DataPoint;
    var b_data: DataPoint;
    var a_px = vec2<f32>(0.0, 0.0);
    var b_px = vec2<f32>(0.0, 0.0);
    var perp: vec2<f32>;
    var half_stroke: f32;

    if (seg == 0u) {
        a_data = DataPoint(in.x, y_lo);
        b_data = DataPoint(in.x, y_hi);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 1u) {
        a_data = DataPoint(in.x, y_lo);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 2u) {
        a_data = DataPoint(in.x, y_hi);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 3u) {
        a_data = DataPoint(x_lo, in.y);
        b_data = DataPoint(x_hi, in.y);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 4u) {
        a_data = DataPoint(x_lo, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -style.cap_half_px);
        b_px = vec2<f32>(0.0,  style.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.cap_width_px * 0.5;
    } else {
        a_data = DataPoint(x_hi, in.y);
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
    let seed = (u32(transform.style_params[0].z) ^ style.series_salt) + styled_point_index(inst);
    let lattice = f32(seg * 2u + select(1u, 0u, at_a));
    let disp = amp * sketch_noise(lattice, seed);

    let end_data = select_point(b_data, a_data, at_a);
    let end_px = select(b_px, a_px, at_a);
    let offset_px = end_px + perp * (side * half_stroke + disp);
    let ndc = point_ndc(end_data) + offset_px * transform.pixel_to_ndc;
    return vec4<f32>(ndc, 0.0, 1.0);
}

// ────────────── constellation mode (NOT part of the common block) ───────────
// Bipolar-jet errorbars render the error range as a glowing astrophysical jet:
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
    out.seed_inst = (u32(transform.style_params[0].w) ^ style.series_salt) + styled_point_index(inst);
    out.kind = select(1u, 0u, seg == 0u || seg == 3u);
    out.t_src = 0.5;
    out.local = vec2<f32>(0.0, 0.0);

    let has_y = (style.primitive_flags & ERRORBAR_HAS_Y) != 0u;
    let has_x = (style.primitive_flags & ERRORBAR_HAS_X) != 0u;
    let dir_enabled = select(has_x, has_y, seg < 3u);
    if (!dir_enabled) {
        let anchor = data_to_ndc(in.x, in.y);
        out.pos = vec4<f32>(anchor, 0.0, 1.0);
        return out;
    }

    let y_lo = pair_sub(in.y, in.err_y_lo);
    let y_hi = pair_sum(in.y, in.err_y_hi);
    let x_lo = pair_sub(in.x, in.err_x_lo);
    let x_hi = pair_sum(in.x, in.err_x_hi);

    var a_data: DataPoint;
    var b_data: DataPoint;
    var a_px = vec2<f32>(0.0, 0.0);
    var b_px = vec2<f32>(0.0, 0.0);
    var perp: vec2<f32>;
    var half_stroke: f32;

    let beam_half = max(style.line_width_px * 0.5, 1.2) * JET_BEAM_HALO;
    let knot_half = max(style.cap_half_px, 3.0) * JET_KNOT_HALO;

    if (seg == 0u) {
        a_data = DataPoint(in.x, y_lo);
        b_data = DataPoint(in.x, y_hi);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = beam_half;
        out.t_src = pair_value(in.err_y_lo) / max(pair_value(in.err_y_lo) + pair_value(in.err_y_hi), 1e-6);
    } else if (seg == 1u) {
        a_data = DataPoint(in.x, y_lo);
        b_data = a_data;
        a_px = vec2<f32>(-knot_half, 0.0);
        b_px = vec2<f32>(knot_half, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = knot_half;
    } else if (seg == 2u) {
        a_data = DataPoint(in.x, y_hi);
        b_data = a_data;
        a_px = vec2<f32>(-knot_half, 0.0);
        b_px = vec2<f32>(knot_half, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = knot_half;
    } else if (seg == 3u) {
        a_data = DataPoint(x_lo, in.y);
        b_data = DataPoint(x_hi, in.y);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = beam_half;
        out.t_src = pair_value(in.err_x_lo) / max(pair_value(in.err_x_lo) + pair_value(in.err_x_hi), 1e-6);
    } else if (seg == 4u) {
        a_data = DataPoint(x_lo, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -knot_half);
        b_px = vec2<f32>(0.0, knot_half);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = knot_half;
    } else {
        a_data = DataPoint(x_hi, in.y);
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

    let end_data = select_point(b_data, a_data, at_a);
    let end_px = select(b_px, a_px, at_a);
    let offset_px = end_px + perp * (side * half_stroke);
    let ndc = point_ndc(end_data) + offset_px * transform.pixel_to_ndc;

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
