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

// ───── BEGIN common block (SHADER_COMMON.md) ─────
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
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
    style_params: array<vec4<f32>, 3>,
};  // 96 B (vec4 array at offset 48, stride 16 — alignment unchanged)

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
    // (sketch/milkyway/constellation) XOR it into their hash seeds so two series never
    // share a star/wobble pattern; precise entries never read it.
    series_salt: u32,
    _pad: u32,
    dash: array<vec4<f32>, 2>,
};

@group(1) @binding(0) var<uniform> style: Style;

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}

fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32) -> f32 {
    let raw = v.x + v.y;
    let linear_num = (v.x - min_hi) + (v.y - min_lo);
    let range = (max_hi - min_hi) + (max_lo - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    return mix(linear_num / range, log_num / range, is_log);
}

fn data_to_ndc(xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    let tx = axis_pair_to_t(xv, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x);
    let ty = axis_pair_to_t(yv, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y);
    return vec2<f32>(tx, ty) * 2.0 - 1.0;
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
};

const LINE_AA_EXTENT_PX: f32 = 0.5;

@vertex
fn vs_main(in: VsIn, arc: VsArc, @builtin(vertex_index) vid: u32) -> VsOut {
    let a_ndc = data_to_ndc(in.x_a, in.y_a);
    let b_ndc = data_to_ndc(in.x_b, in.y_b);

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

    let at_b = vid >= 2u;
    let on_pos = (vid & 1u) == 1u;
    let center_px = select(a_px, b_px, at_b);
    let side = select(-1.0, 1.0, on_pos);

    // Square caps: extend the quad by half_w along the segment direction on
    // both ends. Adjacent quads then overlap at the joint, which keeps the
    // stroke CONTINUOUS when segments are sub-pixel (dense / noisy data
    // flips the direction at every point; butt-ended quads degenerate into
    // disconnected slivers there — the "stippled line" artifact).
    let cap = select(-geom_half_w, geom_half_w, at_b);
    let corner_px = center_px + dir * cap + normal_px * geom_half_w * side;
    let corner_ndc = corner_px * transform.pixel_to_ndc;

    var out: VsOut;
    out.pos = vec4<f32>(corner_ndc, 0.0, 1.0);
    out.dist_px = select(arc.arc_a - half_w, arc.arc_b + half_w, at_b);
    out.along_px = select(-geom_half_w, len + geom_half_w, at_b);
    out.side_px = geom_half_w * side;
    out.segment_len_px = len;
    return out;
}

// Pattern scalar i (0..7): style.dash packs 8 sequential [on, off, ...]
// pixel lengths as two vec4s — dash[0].xyzw first, then dash[1].xyzw.
fn dash_scalar(i: u32) -> f32 {
    return style.dash[i / 4u][i % 4u];
}

fn stroke_alpha(in: VsOut) -> f32 {
    let half_w = max(style.line_width_px, 1.0) * 0.5;
    let p = vec2<f32>(in.along_px - in.segment_len_px * 0.5, in.side_px);
    let half_rect = vec2<f32>(in.segment_len_px * 0.5 + half_w, half_w);
    let q = abs(p) - half_rect;
    let outside = max(q, vec2<f32>(0.0, 0.0));
    let signed_dist = length(outside) + min(max(q.x, q.y), 0.0);
    let aa = max(fwidth(signed_dist), 0.5);
    return 1.0 - smoothstep(-aa, aa, signed_dist);
}

fn dash_alpha(in: VsOut) -> f32 {
    if style.dash_len == 0u {
        return 1.0;
    }
    // Clamp to the 8-scalar capacity so a bad CPU-side count cannot index
    // past the array.
    let n = min(style.dash_len, 8u);
    var period = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        period = period + max(dash_scalar(i), 0.0);
    }
    // Degenerate pattern (all spans zero/negative) — treat as solid.
    if period <= 0.0 {
        return 1.0;
    }
    // The square-cap extension can push dist_px slightly negative at the
    // polyline start — wrap into [0, period) with the sign-safe form.
    let phase = ((in.dist_px % period) + period) % period;
    var acc = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        let span = max(dash_scalar(i), 0.0);
        let start = acc;
        let end = acc + span;
        acc = end;
        if phase <= end || i == n - 1u {
            // Even spans are "on", odd spans are "off". Dash membership is
            // intentionally binary so the requested pattern length stays
            // stable; antialiasing is applied to the stroke outline itself.
            return select(0.0, 1.0, (i & 1u) == 0u);
        }
    }
    // phase == period can occur from float rounding; that point is the
    // start of the next repetition, which always opens with an "on" span.
    return 1.0;
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
    return line_fragment_color(in, style.color_premul);
}

@fragment
fn fs_constellation_line(in: VsOut) -> @location(0) vec4<f32> {
    let a = clamp(transform.style_params[0].y, 0.0, 1.0);
    return line_fragment_color(in, vec4<f32>(style.color_premul.rgb * a, style.color_premul.a * a));
}

// ──────────────── sketch mode (NOT part of the common block) ────────────────
// Hand-drawn entry point — design SSoT: docs/SKETCH_DESIGN.md (§3 noise,
// §5b line). Selected as a separate pipeline variant; the precise entries
// above are never modified and never read the sketch Transform fields.

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

// Subdivision count S per segment instance: the sketch pipeline draws
// 2*(S+1) strip vertices per instance. The CPU-side draw count lives in
// mod.rs (`LINE_SKETCH_VERTICES_PER_INSTANCE`) — keep the two in sync.
const SKETCH_SUBDIV: u32 = 8u;

// Sketch line vertex stage (docs/SKETCH_DESIGN.md §5b): subdivide each
// segment into S spans (k = vid/2 ∈ 0..=S, side = vid%2, t = k/S), displace
// the midline perpendicularly by amplitude · noise(arc_px/wavelength, seed),
// then extrude ±half_w. Arc-length parameterization makes the displacement
// continuous across the shared endpoint of adjacent segments. The outer
// points keep vs_main's square-cap extension so joints stay seamless, and
// dash phase still comes from the same cumulative arc interpolation as vs_main, so
// dashed+sketch composes for free. Non-finite endpoints (NaN gaps, log of
// ≤ 0) propagate through mix() and clip, same as the precise path.
@vertex
fn vs_sketch(in: VsIn, arc: VsArc, @builtin(vertex_index) vid: u32) -> VsOut {
    let a_ndc = data_to_ndc(in.x_a, in.y_a);
    let b_ndc = data_to_ndc(in.x_b, in.y_b);

    let a_px = a_ndc / transform.pixel_to_ndc;
    let b_px = b_ndc / transform.pixel_to_ndc;

    let delta = b_px - a_px;
    let len = length(delta);
    let dir = select(vec2<f32>(0.0, 0.0), delta / max(len, 1e-6), len > 1e-6);
    let normal_px = vec2<f32>(-dir.y, dir.x);

    let half_w = max(style.line_width_px, 1.0) * 0.5;
    let aa_extent = select(LINE_AA_EXTENT_PX, 0.0, style.dash_len > 0u);
    let geom_half_w = half_w + aa_extent;

    let k = vid / 2u;
    let on_pos = (vid & 1u) == 1u;
    let t = f32(k) / f32(SKETCH_SUBDIV);
    let center_px = mix(a_px, b_px, t);
    let side = select(-1.0, 1.0, on_pos);

    // Square caps on the outer subdivision points only (same rule and same
    // rationale as vs_main).
    var cap = 0.0;
    if (k == 0u) { cap = -geom_half_w; }
    if (k == SKETCH_SUBDIV) { cap = geom_half_w; }

    let arc_at = mix(arc.arc_a, arc.arc_b, t);
    let amp = max(transform.style_params[0].x, 0.0);
    let wav = transform.style_params[0].y;
    let seed = u32(transform.style_params[0].z) ^ style.series_salt;
    let wobble = amp * sketch_noise(arc_at / max(wav, 1e-6), seed);
    // wavelength <= 0 disables the wobble — mirrors the CPU stroker's guard
    // (sketch.rs) so GPU and deco layers degrade identically.
    let disp = select(0.0, wobble, wav > 0.0);

    let corner_px = center_px + dir * cap + normal_px * (geom_half_w * side + disp);
    let corner_ndc = corner_px * transform.pixel_to_ndc;

    var out: VsOut;
    out.pos = vec4<f32>(corner_ndc, 0.0, 1.0);
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
// Star-chain line style — design SSoT: docs/CONSTELLATION_DESIGN.md.
// Two entries reuse the line pipeline's six instance slots:
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
};

@vertex
fn vs_ribbon(in: VsIn, arc: VsArc, @builtin(vertex_index) vid: u32) -> RibbonOut {
    let a_px = data_to_ndc(in.x_a, in.y_a) / transform.pixel_to_ndc;
    let b_px = data_to_ndc(in.x_b, in.y_b) / transform.pixel_to_ndc;
    let delta = b_px - a_px;
    let len = length(delta);
    let dir = select(vec2<f32>(0.0, 0.0), delta / max(len, 1e-6), len > 1e-6);
    let normal_px = vec2<f32>(-dir.y, dir.x);

    // Geometric half-extent = one FULL ribbon width: the gaussian profile in
    // fs_ribbon reaches ~6% of peak at the strip edge, so the geometric cut
    // is invisible under additive blending.
    let half_geom = max(transform.style_params[0].y, 1.0);

    let k = vid / 2u;
    let t = f32(k) / f32(CONS_RIBBON_SUBDIV);
    let side = select(-1.0, 1.0, (vid & 1u) == 1u);

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
    let arc_px = mix(arc.arc_a, arc.arc_b, t);

    var out: RibbonOut;
    out.pos = vec4<f32>(corner_px * transform.pixel_to_ndc, 0.0, 1.0);
    out.cross_d = side;
    out.center_i = intensity_cfg * cons_clump(arc_px, seed);
    out.cap_d = abs(cap) / max(half_geom, 1e-6);
    return out;
}

@fragment
fn fs_ribbon(in: RibbonOut) -> @location(0) vec4<f32> {
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
    _pad: u32,
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

    let a_px = data_to_ndc(star_pool_pair(star_vsp.x_base, lo), star_pool_pair(star_vsp.y_base, lo))
        / transform.pixel_to_ndc;
    let b_px = data_to_ndc(
        star_pool_pair(star_vsp.x_base, lo + 1u),
        star_pool_pair(star_vsp.y_base, lo + 1u),
    ) / transform.pixel_to_ndc;
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

    // Temperature: population mix by local density (§2.6), then blackbody
    // LUT. textureLoad — vertex stages have no implicit derivatives.
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

    var center = mix(a_px, b_px, t) + normal_px * off;
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
    out.uv = c * 0.5 + vec2<f32>(0.5, 0.5);
    out.tint = tint;
    out.brightness = b * star_brightness;
    return out;
}

@fragment
fn fs_stars(in: StarOut) -> @location(0) vec4<f32> {
    // Explicit LOD by repo rule (shader_consistency lint) — single-mip PSF.
    let s = textureSampleLevel(cons_psf_tex, cons_samp, in.uv, 0.0);
    // White saturated core + blackbody-tinted halo — star color lives in the
    // halo, exactly like a saturated sensor (§2.1).
    let col = (vec3<f32>(1.0) * s.r + in.tint * s.g) * in.brightness;
    let a = max(col.r, max(col.g, col.b));
    return vec4<f32>(col, a);
}
