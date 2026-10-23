// ───── shared styled markers (SHADER_COMMON.md §5.7; `shader_consistency`
// compares these function bodies and constants with the pick shader) ─────

const SKETCH_TAU: f32 = 6.28318530718;
// Six noise periods around the contour keep marker wobble visually local.
const SKETCH_CONTOUR_WOBBLES: f32 = 6.0;
const CONSTELLATION_POINT_STAR_GAIN: f32 = 1.0;
// Ring radii relative to the planet radius, and the apparent inclination of
// the ring plane (minor/major axis ratio of the projected ellipse).
const RING_INNER: f32 = 1.45;
const RING_OUTER: f32 = 2.3;
const RING_INCL: f32 = 0.32;
// Quad half-extent multiplier — room for the ring + rim glow + AA.
const PLANET_QUAD_EXTENT: f32 = 2.5;
// A planet fragment whose composed alpha is at or below this is discarded.
const PLANET_ALPHA_FLOOR: f32 = 0.003;

/// A sketch marker's contour wobble amplitude in pixels: half the style's
/// amplitude, at most 0.35 of the marker radius.
fn sketch_marker_wobble_px(amplitude_px: f32, radius_px: f32) -> f32 {
    return min(max(amplitude_px, 0.0) * 0.5, radius_px * 0.35);
}

/// A sketch marker's signed contour distance: the precise `shape_distance`
/// perturbed with angle-parameterized noise, d' = d + k·noise(θ/τ·C), seeded
/// per marker so every point gets its own hand-drawn outline. The noise
/// lattice has a seam at θ = ±π; its magnitude is ≤ wobble_px (≤ 0.75 px at
/// default amplitude) and reads as part of the hand-drawn look.
fn sketch_marker_distance(base: u32, p: vec2<f32>, r: f32, filled: bool, wobble_px: f32, seed: u32) -> f32 {
    let theta = atan2(p.y, p.x); // [-π, π]
    return shape_distance(base, p, r, filled)
        + wobble_px * sketch_noise(theta / SKETCH_TAU * SKETCH_CONTOUR_WOBBLES, seed);
}

/// A constellation marker star's core radius in pixels: the marker radius
/// with a per-point size jitter hashed from the point index, at least half a
/// pixel. The star's quad (and its PSF sprite) spans four times this plus
/// `QUAD_MARGIN_PX`.
fn constellation_star_radius(radius_px: f32, index: u32, seed: u32) -> f32 {
    let size_jitter = 0.86 + 0.30 * sketch_hash01(index, seed ^ 0x51A7u);
    return max(radius_px * CONSTELLATION_POINT_STAR_GAIN * size_jitter, 0.5);
}

// ScatterShape code → ring position angle. The first legacy shapes get hand-
// picked angles; newer marker codes share a deterministic fallback.
fn cons_ring_angle(shape_id: u32) -> f32 {
    switch shape_id {
        case 0u: { return 0.0; }          // Circle
        case 1u: { return 0.52; }         // Square        (+30°)
        case 2u: { return 1.05; }         // Triangle      (+60°)
        case 3u: { return -0.52; }        // Diamond       (−30°)
        case 4u: { return -1.05; }        // Cross         (−60°)
        case 5u: { return 0.26; }         // CircleFilled  (+15°)
        case 6u: { return 0.79; }         // SquareFilled  (+45°)
        case 7u: { return -0.26; }        // TriangleFilled(−15°)
        case 8u: { return -0.79; }        // DiamondFilled (−45°)
        default: {
            return -1.2 + f32(shape_id % 11u) * 0.24;
        }
    }
}

/// A planet's ring position angle: the series shape's tilt plus a small
/// per-point jitter (real systems aren't aligned).
fn planet_ring_phi(shape_id: u32, seed_inst: u32) -> f32 {
    let h_jit = sketch_hash01(seed_inst, 0x717Au);
    return cons_ring_angle(shape_id) + (h_jit - 0.5) * 0.12;
}

/// `p` rotated into the ring frame of position angle `phi`.
fn planet_ring_frame(p: vec2<f32>, phi: f32) -> vec2<f32> {
    let cphi = cos(phi);
    let sphi = sin(phi);
    return vec2<f32>(p.x * cphi + p.y * sphi, -p.x * sphi + p.y * cphi);
}

/// The ring-plane radius of ring-frame point `q`, in planet radii: the ring
/// band is drawn where it lies between `RING_INNER` and `RING_OUTER`.
fn planet_ring_rho(q: vec2<f32>, r_planet: f32) -> f32 {
    return length(vec2<f32>(q.x, q.y / RING_INCL)) / r_planet;
}

/// The atmospheric rim glow's alpha at distance `dist` from the planet's
/// centre: a soft exponential falloff outside the disc.
fn planet_rim_glow(dist: f32, r_planet: f32, rim_gain: f32) -> f32 {
    let rim_d = max(dist - r_planet, 0.0);
    return rim_gain * exp(-rim_d / max(r_planet * 0.10, 1.0));
}

// ───── end shared styled markers ─────

// Sketch varyings. The fragment stage must not read `transform` (the shared
// transform bind group is vertex-only), so the vertex stage forwards the
// resolved per-marker seed and the clamped wobble amplitude as flat values.
struct VsSketchOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    // Global sketch seed + instance index — per-point shape variation.
    @location(1) @interpolate(flat) seed_inst: u32,
    // Contour wobble amplitude in px: min(amplitude·0.5, radius·0.35).
    @location(2) @interpolate(flat) wobble_px: f32,
    @location(3) polar_px: vec2<f32>,
    @location(4) @interpolate(flat) polar_limits: vec4<f32>,
    @location(5) @interpolate(flat) polar_sector: vec4<f32>,
};

// Same data→NDC placement as vs_main, with the quad grown by the wobble
// amplitude so the perturbed contour (and its stroke + AA feather) never
// clips at the quad edge. fs_sketch reconstructs pixel coordinates with the
// same expanded scale.
@vertex
fn vs_sketch(in: VsIn, @builtin(instance_index) inst: u32) -> VsSketchOut {
    if (!sample_valid(in.x, in.y)) {
        var masked: VsSketchOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(in.x, in.y);

    let wobble = sketch_marker_wobble_px(transform.style_params[0].x, style.point_radius_px);
    let half_px = visual_shape_radius(base_shape_id(style.shape_id), style.point_radius_px)
        + QUAD_MARGIN_PX
        + wobble;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsSketchOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.local_pos = in.quad_pos;
    out.seed_inst = (u32(transform.style_params[0].z) ^ style.series_salt) + styled_point_index(inst);
    out.wobble_px = wobble;
    return out;
}

// The sketch marker's contour is `sketch_marker_distance`'s: the precise
// shape perturbed by angle-parameterized noise, seeded per marker.
@fragment
fn fs_sketch(in: VsSketchOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let r = style.point_radius_px;
    let filled = shape_is_filled(style.shape_id);
    let base = base_shape_id(style.shape_id);
    let visual_r = visual_shape_radius(base, r);
    // Pixel-space position — same expanded scale as the vs_sketch quad.
    let p = in.local_pos * (visual_r + QUAD_MARGIN_PX + in.wobble_px);

    let d = sketch_marker_distance(base, p, visual_r, filled, in.wobble_px, in.seed_inst);

    let aa = fwidth(d);
    let edge = marker_edge(d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}

// ────────────── constellation mode (NOT part of the common block) ───────────
//   - The ring's POSITION ANGLE encodes the series: the existing
//     ScatterShape SSoT maps to a tilt (see cons_ring_angle) — no new
//     per-series fields.
//   - The planet surface samples a baked procedural atlas (2×2 archetypes,
//     picked per point); shading is sphere lambert + limb darkening.
//   - The series point_color appears ONLY as a thin atmospheric rim glow,
//     like the line ribbon: bodies stay physical, identity stays visible.
// Blending is premultiplied alpha (planets occlude the star field), unlike
// the additive line passes.

@group(2) @binding(0) var cons_psf_tex: texture_2d<f32>; // R=core, G=halo
@group(2) @binding(1) var cons_lut_tex: texture_2d<f32>; // 256x1 blackbody
@group(2) @binding(2) var cons_samp: sampler;
@group(2) @binding(3) var cons_planet_atlas: texture_2d<f32>;
@group(2) @binding(4) var cons_ring_tex: texture_2d<f32>;

struct VsConstellationStarOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec3<f32>,
    @location(2) brightness: f32,
    @location(3) polar_px: vec2<f32>,
    @location(4) @interpolate(flat) polar_limits: vec4<f32>,
    @location(5) @interpolate(flat) polar_sector: vec4<f32>,
};

@vertex
fn vs_constellation_star(in: VsIn, @builtin(instance_index) inst: u32) -> VsConstellationStarOut {
    if (!sample_valid(in.x, in.y)) {
        var masked: VsConstellationStarOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(in.x, in.y);

    let seed = style.series_salt;

    let global_inst = styled_point_index(inst);
    let u_b = sketch_hash01(global_inst, seed ^ 0x86A9u);
    let brightness = 0.22 + 0.78 * pow(u_b, 2.4);
    let star_radius = constellation_star_radius(style.point_radius_px, global_inst, seed);
    let half_px = star_radius * 4.0 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    let h_t = sketch_hash01(global_inst, seed ^ 0x7E47u);
    let t_norm = mix(0.08, 0.92, h_t);
    let tint = textureLoad(
        cons_lut_tex,
        vec2<i32>(i32(clamp(t_norm, 0.0, 1.0) * 255.0), 0),
        0,
    ).rgb;

    var out: VsConstellationStarOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.uv = in.quad_pos * 0.5 + vec2<f32>(0.5, 0.5);
    out.tint = tint;
    out.brightness = brightness;
    return out;
}

@fragment
fn fs_constellation_star(in: VsConstellationStarOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let s = textureSampleLevel(cons_psf_tex, cons_samp, in.uv, 0.0);
    let col = (vec3<f32>(1.0) * s.r + in.tint * s.g) * in.brightness;
    let a = clamp(max(col.r, max(col.g, col.b)), 0.0, 1.0);
    let marker_opacity = clamp(style.color_premul.a, 0.0, 1.0);
    let star_opacity = clamp(transform.style_params[0].x, 0.0, 1.0) * marker_opacity;
    let out_a = a * star_opacity;
    if (out_a <= 0.002) {
        discard;
    }
    return vec4<f32>(min(col, vec3<f32>(a)) * star_opacity, out_a);
}

struct VsPlanetOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) @interpolate(flat) seed_inst: u32,
    // Atmospheric rim-glow strength (MilkywayOptions.planet_rim) —
    // forwarded by the vertex stage; the transform group is vertex-only.
    @location(2) @interpolate(flat) rim_gain: f32,
    @location(3) polar_px: vec2<f32>,
    @location(4) @interpolate(flat) polar_limits: vec4<f32>,
    @location(5) @interpolate(flat) polar_sector: vec4<f32>,
};

@vertex
fn vs_planet(in: VsIn, @builtin(instance_index) inst: u32) -> VsPlanetOut {
    if (!sample_valid(in.x, in.y)) {
        var masked: VsPlanetOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(in.x, in.y);

    let half_px = style.point_radius_px * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPlanetOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.local_pos = in.quad_pos;
    out.seed_inst = (u32(transform.style_params[0].w) ^ style.series_salt) + styled_point_index(inst);
    out.rim_gain = clamp(transform.style_params[1].w, 0.0, 2.0);
    return out;
}

@fragment
fn fs_planet(in: VsPlanetOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let r_planet = max(style.point_radius_px, 1.0);
    let p = in.local_pos * (r_planet * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX);
    let dist = length(p);
    let aa = max(fwidth(dist), 0.5);

    // Per-point variation channels.
    let h_arch = sketch_hash01(in.seed_inst, 0xA2C4u);
    let h_spin = sketch_hash01(in.seed_inst, 0x591Eu);

    // Ring frame: rotate into the ring plane. Position angle = series shape
    // mapping ± a small per-point jitter (real systems aren't aligned).
    let phi = planet_ring_phi(style.shape_id, in.seed_inst);
    let cphi = cos(phi);
    let sphi = sin(phi);
    let q = planet_ring_frame(p, phi);

    // ── Physically consistent ring/planet frame ──
    // The ring lies in the planet's EQUATORIAL plane. With the projected
    // ellipse ratio RING_INCL = sin(i), the spin pole is
    //   P  = (0, cos i, sin i)   (screen-up in ring frame, tipped toward
    //                             the viewer — the north pole is visible)
    // and the equatorial basis is
    //   e1 = (1, 0, 0),  e2 = P × e1 = (0, sin i, −cos i).
    // e2's far hemisphere projects to +q.y, so the UPPER ring half is the
    // far (occluded) one and the lower half passes in front — and surface
    // latitude derives from the same pole, which is what makes the bands
    // bow with exactly the ring's curvature family (the Saturn look).
    let sin_i = RING_INCL;
    let cos_i = sqrt(1.0 - sin_i * sin_i);

    // Projected ring ellipse: radial coordinate in ring-plane units.
    let rho = planet_ring_rho(q, r_planet);
    let in_band = rho > RING_INNER && rho < RING_OUTER;
    var ring_a = 0.0;
    var ring_rgb = vec3<f32>(0.0);
    if (in_band) {
        let u = (rho - RING_INNER) / (RING_OUTER - RING_INNER);
        // textureSampleLevel, NOT textureSample: this branch is non-uniform
        // (fragment-position dependent), and browser WGSL (Tint) hard-rejects
        // implicit-derivative sampling there — the whole module fails to
        // compile, which blanks every scatter draw on wasm. The bake
        // textures are single-mip, so explicit LOD 0 is pixel-identical.
        let s = textureSampleLevel(cons_ring_tex, cons_samp, vec2<f32>(u, 0.5), 0.0);
        // Edge AA along the band borders.
        let band_aa = smoothstep(RING_INNER, RING_INNER + 0.06, rho)
            * (1.0 - smoothstep(RING_OUTER - 0.06, RING_OUTER, rho));
        ring_a = s.a * band_aa;
        ring_rgb = s.rgb;

        // Planet shadow on the ring — the REAL model: the shadow is the
        // planet's anti-light cylinder cutting the ring plane, not "the
        // whole back half" (a binary front/back gate puts a hard seam where
        // the ring crosses the major axis). Reconstruct the ring point in
        // 3D (ring plane: depth z = −u2·cos i with u2 = q.y / sin i), put
        // the light in the same rotated frame, and shade smoothly inside
        // the cylinder behind the body.
        let ring_p3 = vec3<f32>(q.x, q.y, -q.y * cos_i / max(sin_i, 1e-4)) / r_planet;
        let l0 = normalize(vec3<f32>(-0.55, 0.5, 0.62));
        let lq = vec3<f32>(
            l0.x * cphi + l0.y * sphi,
            -l0.x * sphi + l0.y * cphi,
            l0.z,
        );
        let t_axis = dot(ring_p3, lq);
        let d_perp = length(ring_p3 - lq * t_axis);
        let umbra = (1.0 - smoothstep(0.92, 1.18, d_perp))
            * smoothstep(0.05, 0.45, -t_axis);
        ring_rgb = ring_rgb * (1.0 - 0.8 * umbra);
    }
    // Lower half (q.y < 0) passes in FRONT of the planet, upper half behind.
    let ring_front = q.y < 0.0;

    // Planet disc + sphere shading.
    let disc = 1.0 - smoothstep(r_planet - aa, r_planet + aa, dist);
    var planet_rgb = vec3<f32>(0.0);
    if (disc > 0.0) {
        let pr = min(dist / r_planet, 0.9999);
        let z = sqrt(1.0 - pr * pr);
        let n = vec3<f32>(p.x / r_planet, p.y / r_planet, z);
        // Light from the upper-left, slightly toward the viewer.
        let l = normalize(vec3<f32>(-0.55, 0.5, 0.62));
        let diff = max(dot(n, l), 0.0);
        let limb = pow(max(z, 0.0), 0.45);

        // Sphere point in the ring frame (q.x, q.y, z toward viewer).
        let nq = vec3<f32>(q.x / r_planet, q.y / r_planet, z);
        // Latitude about the ring-plane pole; longitude in the equatorial
        // basis, spun per point.
        let lat = asin(clamp(nq.y * cos_i + nq.z * sin_i, -1.0, 1.0));
        let lon = atan2(nq.y * sin_i - nq.z * cos_i, nq.x);
        let arch = u32(h_arch * 4.0) % 4u;
        let tile = vec2<f32>(f32(arch % 2u), f32(arch / 2u));
        let inner_uv = vec2<f32>(
            fract(lon / 6.2831853 + 0.5 + h_spin),
            lat / 3.1415927 + 0.5,
        ) * 0.94 + vec2<f32>(0.03, 0.03);
        let uv = (tile + inner_uv) * 0.5;
        // Explicit LOD for the same Tint uniformity rule as the ring sample.
        let albedo = textureSampleLevel(cons_planet_atlas, cons_samp, uv, 0.0).rgb;

        planet_rgb = albedo * (0.18 + 0.88 * diff) * limb;
    }

    // Atmospheric rim glow — the series color's only appearance. Soft
    // exponential falloff outside the disc, not a hard shell.
    let rim = select(0.0, planet_rim_glow(dist, r_planet, in.rim_gain), dist > r_planet - aa);
    let rim_rgb = style.color_premul.rgb * rim;

    // Compose, premultiplied: back ring (occluded by the disc) → planet →
    // front ring over the planet → rim glow outside.
    var rgb = vec3<f32>(0.0);
    var a = 0.0;
    if (!ring_front) {
        let behind_a = ring_a * (1.0 - disc);
        rgb = ring_rgb * behind_a;
        a = behind_a;
    }
    rgb = rgb * (1.0 - disc) + planet_rgb * disc;
    a = a * (1.0 - disc) + disc;
    if (ring_front) {
        rgb = rgb * (1.0 - ring_a) + ring_rgb * ring_a;
        a = a * (1.0 - ring_a) + ring_a;
    }
    rgb = rgb + rim_rgb * (1.0 - a);
    a = a + rim * (1.0 - a);

    if (a <= PLANET_ALPHA_FLOOR) {
        discard;
    }
    return vec4<f32>(rgb, a);
}
