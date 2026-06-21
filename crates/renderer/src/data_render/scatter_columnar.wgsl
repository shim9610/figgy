// Columnar SDF scatter shader.
//
// slot 0: unit quad (per-vertex, ±1 NDC).
// slot 1: X column (per-instance, f32).
// slot 2: Y column (per-instance, f32).
//
// Marker shapes — style.shape_id is the stable code from mod.rs::shape_id().
// Existing ids 0..8 are kept compatible; newer scientific-plot symbols append
// after them. Shape decoding below maps those ids to compact SDF base shapes
// plus an open/filled flag.

// ───── BEGIN common block (SHADER_COMMON.md) ─────
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
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
// ───── END common block ─────
//
// Note: this shader inlines the `data_to_ndc` mapping inside `vs_main`
// rather than defining it as a function. The inline formula must match
// SHADER_COMMON.md §4 (4a / 4b are equivalent).

struct VsIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: f32,
    @location(2) y: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
};

// Quad half-extent in pixels = shape-specific axis extent + QUAD_MARGIN_PX.
// `point_radius_px` is the circle reference radius; non-circular shapes are
// normalized below so the same marker size has a similar visual weight.
// Outline
// strokes are centered on the shape contour, so they reach up to
// STROKE_HALF_PX plus the AA feather (~1 px) beyond the radius; without
// the margin the quad edge would clip them. fs_main must reconstruct
// pixel coordinates with this same expanded scale.
const QUAD_MARGIN_PX: f32 = 2.0;
// Half-width of the ~1.5 px contour stroke (outline shapes and Cross).
const STROKE_HALF_PX: f32 = 0.75;

const MARKER_PI: f32 = 3.14159265359;
const MARKER_HALF_PI: f32 = 1.57079632679;
const STAR_INNER_RATIO: f32 = 0.5;

fn shape_is_filled(shape_id: u32) -> bool {
    return (shape_id >= 5u && shape_id <= 8u) || shape_id >= 17u;
}

fn base_shape_id(shape_id: u32) -> u32 {
    switch shape_id {
        case 5u: { return 0u; }   // CircleFilled
        case 6u: { return 1u; }   // SquareFilled
        case 7u: { return 2u; }   // TriangleFilled
        case 8u: { return 3u; }   // DiamondFilled
        case 9u: { return 5u; }   // TriangleDown
        case 10u: { return 6u; }  // TriangleLeft
        case 11u: { return 7u; }  // TriangleRight
        case 12u: { return 8u; }  // Plus
        case 13u: { return 9u; }  // Pentagon
        case 14u: { return 10u; } // Hexagon
        case 15u: { return 11u; } // Octagon
        case 16u: { return 12u; } // Star
        case 17u: { return 5u; }  // TriangleDownFilled
        case 18u: { return 6u; }  // TriangleLeftFilled
        case 19u: { return 7u; }  // TriangleRightFilled
        case 20u: { return 8u; }  // PlusFilled
        case 21u: { return 4u; }  // CrossFilled
        case 22u: { return 9u; }  // PentagonFilled
        case 23u: { return 10u; } // HexagonFilled
        case 24u: { return 11u; } // OctagonFilled
        case 25u: { return 12u; } // StarFilled
        default: { return shape_id; }
    }
}

// Shape-local radius/half-extent relative to the circle reference radius.
// Filled polygon markers are area-matched to a circle of radius r.
fn visual_shape_radius(base: u32, r: f32) -> f32 {
    var scale = 1.0;
    switch base {
        case 1u: { scale = 0.88622695; }  // square sqrt(pi)/2
        case 2u, 5u, 6u, 7u: { scale = 1.55512030; } // triangles
        case 3u: { scale = 1.25331414; }  // diamond sqrt(pi/2)
        case 9u: { scale = 1.14913986; }  // pentagon
        case 10u: { scale = 1.09963611; } // hexagon
        case 11u: { scale = 1.05390737; } // octagon
        case 12u: { scale = 1.46285033; } // 5-point star, inner ratio 0.5
        default: {}
    }
    return r * scale;
}

fn rotate2(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let xv = maybe_log(in.x, transform.scale_log.x);
    let yv = maybe_log(in.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    let center_ndc = t * 2.0 - 1.0;
    let half_px =
        visual_shape_radius(base_shape_id(style.shape_id), style.point_radius_px) + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.local_pos = in.quad_pos;
    return out;
}

// Unsigned distance from `p` to segment `ab`. The denominator guard keeps
// a zero-length segment (radius 0) from yielding NaN — it degenerates to
// point distance instead.
fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    return length(pa - ba * h);
}

fn sd_box(p: vec2<f32>, b: vec2<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0);
}

fn regular_vertex(i: u32, n: u32, r: f32, rot: f32) -> vec2<f32> {
    let a = rot + f32(i) * (2.0 * MARKER_PI / f32(n));
    return vec2<f32>(cos(a), sin(a)) * r;
}

fn star_vertex(i: u32, r: f32) -> vec2<f32> {
    let outer = (i % 2u) == 0u;
    let rr = select(r * STAR_INNER_RATIO, r, outer);
    let a = MARKER_HALF_PI + f32(i) * MARKER_PI / 5.0;
    return vec2<f32>(cos(a), sin(a)) * rr;
}

fn sd_regular_polygon(p: vec2<f32>, n: u32, r: f32, rot: f32) -> f32 {
    var min_d = 1e20;
    var inside = false;
    var prev = regular_vertex(n - 1u, n, r, rot);
    for (var i = 0u; i < n; i = i + 1u) {
        let curr = regular_vertex(i, n, r, rot);
        min_d = min(min_d, sd_segment(p, prev, curr));
        if ((curr.y > p.y) != (prev.y > p.y)) {
            let x_at_y = (prev.x - curr.x) * (p.y - curr.y) / (prev.y - curr.y) + curr.x;
            if (p.x < x_at_y) {
                inside = !inside;
            }
        }
        prev = curr;
    }
    return select(min_d, -min_d, inside);
}

fn sd_star(p: vec2<f32>, r: f32) -> f32 {
    var min_d = 1e20;
    var inside = false;
    var prev = star_vertex(9u, r);
    for (var i = 0u; i < 10u; i = i + 1u) {
        let curr = star_vertex(i, r);
        min_d = min(min_d, sd_segment(p, prev, curr));
        if ((curr.y > p.y) != (prev.y > p.y)) {
            let x_at_y = (prev.x - curr.x) * (p.y - curr.y) / (prev.y - curr.y) + curr.x;
            if (p.x < x_at_y) {
                inside = !inside;
            }
        }
        prev = curr;
    }
    return select(min_d, -min_d, inside);
}

fn sd_plus_filled(p: vec2<f32>, r: f32) -> f32 {
    let arm = max(r * 0.32, STROKE_HALF_PX);
    let h = sd_box(p, vec2<f32>(r, arm));
    let v = sd_box(p, vec2<f32>(arm, r));
    return min(h, v);
}

// Exact SDF of an equilateral point-up triangle centered on the origin,
// vertices on the circle of radius `r` (same inscribed sizing as the
// diamond). `p_in` is y-up (NDC orientation), so the apex points up on
// screen. Adapted from iq's sdEquilateralTriangle.
fn sd_triangle(p_in: vec2<f32>, r: f32) -> f32 {
    let k = sqrt(3.0);
    let half_w = 0.5 * k * r; // half of the base edge
    var p = vec2<f32>(abs(p_in.x) - half_w, p_in.y + half_w / k);
    if (p.x + k * p.y > 0.0) {
        p = vec2<f32>(p.x - k * p.y, -k * p.x - p.y) * 0.5;
    }
    p.x -= clamp(p.x, -2.0 * half_w, 0.0);
    return -length(p) * sign(p.y);
}

// Signed pixel distance from `p` to the contour of `base` shape. For open
// plus/cross, the distance is unsigned to the center strokes; the outline
// alpha path below turns that into a visible stroke.
fn shape_distance(base: u32, p: vec2<f32>, r: f32, filled: bool) -> f32 {
    var d: f32;
    switch base {
        case 0u: { d = length(p) - r; }
        case 1u: { d = max(abs(p.x), abs(p.y)) - r; }
        case 2u: { d = sd_triangle(p, r); }
        case 3u: { d = abs(p.x) + abs(p.y) - r; }
        case 4u: { // Cross / x
            let c = r;
            if (filled) {
                d = sd_plus_filled(rotate2(p, MARKER_PI * 0.25), c);
            } else {
                let d1 = sd_segment(p, vec2<f32>(-c, -c), vec2<f32>(c, c));
                let d2 = sd_segment(p, vec2<f32>(-c, c), vec2<f32>(c, -c));
                d = min(d1, d2);
            }
        }
        case 5u: { d = sd_triangle(-p, r); }
        case 6u: { d = sd_triangle(rotate2(p, -MARKER_HALF_PI), r); }
        case 7u: { d = sd_triangle(rotate2(p, MARKER_HALF_PI), r); }
        case 8u: { // Plus / +
            let c = r;
            if (filled) {
                d = sd_plus_filled(p, c);
            } else {
                let d1 = sd_segment(p, vec2<f32>(-c, 0.0), vec2<f32>(c, 0.0));
                let d2 = sd_segment(p, vec2<f32>(0.0, -c), vec2<f32>(0.0, c));
                d = min(d1, d2);
            }
        }
        case 9u: { d = sd_regular_polygon(p, 5u, r, MARKER_HALF_PI); }
        case 10u: { d = sd_regular_polygon(p, 6u, r, MARKER_HALF_PI); }
        case 11u: { d = sd_regular_polygon(p, 8u, r, MARKER_HALF_PI); }
        default: { // 12u: Star
            d = sd_star(p, r);
        }
    }
    return d;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let r = style.point_radius_px;
    let filled = shape_is_filled(style.shape_id);
    let base = base_shape_id(style.shape_id);
    let visual_r = visual_shape_radius(base, r);
    // Pixel-space position — same expanded scale as the vs_main quad, so
    // one unit here is one screen pixel.
    let p = in.local_pos * (visual_r + QUAD_MARGIN_PX);

    let d = shape_distance(base, p, visual_r, filled);
    let aa = fwidth(d);
    // Filled: cover d < 0. Outline/Cross: ~1.5 px stroke centered on the
    // d == 0 contour.
    let edge = select(abs(d) - STROKE_HALF_PX, d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}

// ──────────────── sketch mode (NOT part of the common block) ────────────────
// Hand-drawn entry points — design SSoT: docs/SKETCH_DESIGN.md (§3 noise,
// §5c scatter). Selected as a separate pipeline variant; the precise entries
// above are never modified and never read the sketch Transform fields.

// 1D value-noise pair — original formula: docs/SKETCH_DESIGN.md §3.
// Deliberately duplicated per data shader (scatter/line/errorbar) and NOT in
// the SHADER_COMMON.md common block: line_arc.wgsl shares that block but has
// no use for noise. Keep the three copies in sync with the design doc.
// Per-point style mapping (NOT part of the common block). The default precise
// path above remains the three-slot fast path; this entry is selected only for
// scatter series with a style table, style-index column, or sparse overrides.
struct VsMappedIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: f32,
    @location(2) y: f32,
    @location(3) style_index: f32,
};

struct ScatterStyleSlot {
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct ScatterStyleOverride {
    point_index: u32,
    // Keep this layout byte-for-byte with Rust's ScatterStyleOverrideGpu:
    // u32 + three scalar u32 pads, then two vec4<f32> fields (48 bytes).
    // A WGSL vec3<u32> would align to 16 bytes and shift color_premul.
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct ScatterStyleMapMeta {
    style_count: u32,
    override_count: u32,
    has_index: u32,
    _pad: u32,
};

@group(2) @binding(5) var<storage, read> scatter_style_slots: array<ScatterStyleSlot>;
@group(2) @binding(6) var<storage, read> scatter_style_overrides: array<ScatterStyleOverride>;
@group(2) @binding(7) var<uniform> scatter_style_meta: ScatterStyleMapMeta;

struct ResolvedMappedStyle {
    color_premul: vec4<f32>,
    radius_px: f32,
    shape_id: u32,
};

struct VsMappedOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) color_premul: vec4<f32>,
    @location(2) radius_px: f32,
    @location(3) @interpolate(flat) shape_id: u32,
};

const STYLE_MASK_COLOR: u32 = 1u;
const STYLE_MASK_RADIUS: u32 = 2u;
const STYLE_MASK_SHAPE: u32 = 4u;

fn valid_style_index(v: f32) -> bool {
    return v >= 0.0 && v <= 16777216.0 && abs(v - round(v)) <= 0.001;
}

fn apply_style_slot(base: ResolvedMappedStyle, slot: ScatterStyleSlot) -> ResolvedMappedStyle {
    let mask = u32(slot.params.z);
    var out = base;
    if ((mask & STYLE_MASK_COLOR) != 0u) {
        out.color_premul = slot.color_premul;
    }
    if ((mask & STYLE_MASK_RADIUS) != 0u) {
        out.radius_px = max(slot.params.x, 0.0);
    }
    if ((mask & STYLE_MASK_SHAPE) != 0u) {
        out.shape_id = u32(max(slot.params.y, 0.0));
    }
    return out;
}

fn resolve_mapped_style(style_index: f32, inst: u32) -> ResolvedMappedStyle {
    var out = ResolvedMappedStyle(style.color_premul, style.point_radius_px, style.shape_id);

    if (scatter_style_meta.has_index != 0u && valid_style_index(style_index)) {
        let idx = u32(round(style_index));
        if (idx < scatter_style_meta.style_count) {
            out = apply_style_slot(out, scatter_style_slots[idx]);
        }
    }

    for (var i = 0u; i < scatter_style_meta.override_count; i = i + 1u) {
        let ov = scatter_style_overrides[i];
        if (ov.point_index == inst) {
            out = apply_style_slot(out, ScatterStyleSlot(ov.color_premul, ov.params));
        }
    }
    return out;
}

@vertex
fn vs_mapped(in: VsMappedIn, @builtin(instance_index) inst: u32) -> VsMappedOut {
    let resolved = resolve_mapped_style(in.style_index, inst);
    let xv = maybe_log(in.x, transform.scale_log.x);
    let yv = maybe_log(in.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    let center_ndc = t * 2.0 - 1.0;
    let half_px =
        visual_shape_radius(base_shape_id(resolved.shape_id), resolved.radius_px) + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsMappedOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.local_pos = in.quad_pos;
    out.color_premul = resolved.color_premul;
    out.radius_px = resolved.radius_px;
    out.shape_id = resolved.shape_id;
    return out;
}

@fragment
fn fs_mapped(in: VsMappedOut) -> @location(0) vec4<f32> {
    let filled = shape_is_filled(in.shape_id);
    let base = base_shape_id(in.shape_id);
    let visual_r = visual_shape_radius(base, in.radius_px);
    let p = in.local_pos * (visual_r + QUAD_MARGIN_PX);

    let d = shape_distance(base, p, visual_r, filled);
    let aa = fwidth(d);
    let edge = select(abs(d) - STROKE_HALF_PX, d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return in.color_premul * alpha;
}

struct VsPickRingOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) radius_px: f32,
};

@vertex
fn vs_pick_ring(in: VsIn) -> VsPickRingOut {
    let xv = maybe_log(in.x, transform.scale_log.x);
    let yv = maybe_log(in.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    let center_ndc = t * 2.0 - 1.0;
    let radius = max(style.point_radius_px, 0.0);
    let half_px = radius + max(style.line_width_px, 0.0) * 0.5 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPickRingOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.local_pos = in.quad_pos;
    out.radius_px = radius;
    return out;
}

@vertex
fn vs_pick_ring_mapped(in: VsMappedIn, @builtin(instance_index) inst: u32) -> VsPickRingOut {
    let resolved = resolve_mapped_style(in.style_index, inst);
    let xv = maybe_log(in.x, transform.scale_log.x);
    let yv = maybe_log(in.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    let center_ndc = t * 2.0 - 1.0;
    // Pick-ring mapped mode uses style.cap_half_px as radius_extra_px. The
    // errorbar shader is not involved in this pipeline, so the slot is local
    // to this decoration entry.
    let radius = max(resolved.radius_px + style.cap_half_px, 0.0);
    let half_px = radius + max(style.line_width_px, 0.0) * 0.5 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPickRingOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.local_pos = in.quad_pos;
    out.radius_px = radius;
    return out;
}

@fragment
fn fs_pick_ring(in: VsPickRingOut) -> @location(0) vec4<f32> {
    let radius = max(in.radius_px, 0.0);
    let stroke = max(style.line_width_px, 0.5);
    let half_px = radius + stroke * 0.5 + QUAD_MARGIN_PX;
    let d = abs(length(in.local_pos * half_px) - radius) - stroke * 0.5;
    let aa = max(fwidth(d), 0.5);
    let alpha = 1.0 - smoothstep(-aa, aa, d);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}

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
};

// Same data→NDC placement as vs_main, with the quad grown by the wobble
// amplitude so the perturbed contour (and its stroke + AA feather) never
// clips at the quad edge. fs_sketch reconstructs pixel coordinates with the
// same expanded scale.
@vertex
fn vs_sketch(in: VsIn, @builtin(instance_index) inst: u32) -> VsSketchOut {
    let xv = maybe_log(in.x, transform.scale_log.x);
    let yv = maybe_log(in.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    let center_ndc = t * 2.0 - 1.0;

    let amp = max(transform.style_params[0].x, 0.0);
    let wobble = min(amp * 0.5, style.point_radius_px * 0.35);
    let half_px = visual_shape_radius(base_shape_id(style.shape_id), style.point_radius_px)
        + QUAD_MARGIN_PX
        + wobble;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsSketchOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.local_pos = in.quad_pos;
    out.seed_inst = (u32(transform.style_params[0].z) ^ style.series_salt) + inst;
    out.wobble_px = wobble;
    return out;
}

const SKETCH_TAU: f32 = 6.28318530718;
// Wobble count around the marker contour (docs/SKETCH_DESIGN.md §5c: C ≈ 6).
const SKETCH_CONTOUR_WOBBLES: f32 = 6.0;

// Sketch fragment stage (docs/SKETCH_DESIGN.md §5c): perturb the signed
// contour distance with angle-parameterized noise — d' = d + k·noise(θ/τ·C),
// seeded per marker — so every point gets its own hand-drawn outline. The
// noise lattice has a seam at θ = ±π; its magnitude is ≤ wobble_px (≤ 0.75 px
// at default amplitude) and reads as part of the hand-drawn look.
@fragment
fn fs_sketch(in: VsSketchOut) -> @location(0) vec4<f32> {
    let r = style.point_radius_px;
    let filled = shape_is_filled(style.shape_id);
    let base = base_shape_id(style.shape_id);
    let visual_r = visual_shape_radius(base, r);
    // Pixel-space position — same expanded scale as the vs_sketch quad.
    let p = in.local_pos * (visual_r + QUAD_MARGIN_PX + in.wobble_px);

    let theta = atan2(p.y, p.x); // [-π, π]
    let d = shape_distance(base, p, visual_r, filled)
        + in.wobble_px * sketch_noise(theta / SKETCH_TAU * SKETCH_CONTOUR_WOBBLES, in.seed_inst);

    let aa = fwidth(d);
    let edge = select(abs(d) - STROKE_HALF_PX, d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}

// ────────────── constellation mode (NOT part of the common block) ───────────
// Ringed-planet markers — docs/CONSTELLATION_DESIGN.md Step 2.
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

const CONSTELLATION_POINT_STAR_GAIN: f32 = 1.0;

struct VsConstellationStarOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec3<f32>,
    @location(2) brightness: f32,
};

@vertex
fn vs_constellation_star(in: VsIn, @builtin(instance_index) inst: u32) -> VsConstellationStarOut {
    let xv = maybe_log(in.x, transform.scale_log.x);
    let yv = maybe_log(in.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    let center_ndc = t * 2.0 - 1.0;

    let seed = style.series_salt;

    let u_b = sketch_hash01(inst, seed ^ 0x86A9u);
    let brightness = 0.22 + 0.78 * pow(u_b, 2.4);
    let size_jitter = 0.86 + 0.30 * sketch_hash01(inst, seed ^ 0x51A7u);
    let star_radius = max(style.point_radius_px * CONSTELLATION_POINT_STAR_GAIN * size_jitter, 0.5);
    let half_px = star_radius * 4.0 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    let h_t = sketch_hash01(inst, seed ^ 0x7E47u);
    let t_norm = mix(0.08, 0.92, h_t);
    let tint = textureLoad(
        cons_lut_tex,
        vec2<i32>(i32(clamp(t_norm, 0.0, 1.0) * 255.0), 0),
        0,
    ).rgb;

    var out: VsConstellationStarOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.uv = in.quad_pos * 0.5 + vec2<f32>(0.5, 0.5);
    out.tint = tint;
    out.brightness = brightness;
    return out;
}

@fragment
fn fs_constellation_star(in: VsConstellationStarOut) -> @location(0) vec4<f32> {
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

// Ring radii relative to the planet radius, and the apparent inclination of
// the ring plane (minor/major axis ratio of the projected ellipse).
const RING_INNER: f32 = 1.45;
const RING_OUTER: f32 = 2.3;
const RING_INCL: f32 = 0.32;
// Quad half-extent multiplier — room for the ring + rim glow + AA.
const PLANET_QUAD_EXTENT: f32 = 2.5;

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

struct VsPlanetOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) @interpolate(flat) seed_inst: u32,
    // Atmospheric rim-glow strength (MilkywayOptions.planet_rim) —
    // forwarded by the vertex stage; the transform group is vertex-only.
    @location(2) @interpolate(flat) rim_gain: f32,
};

@vertex
fn vs_planet(in: VsIn, @builtin(instance_index) inst: u32) -> VsPlanetOut {
    let xv = maybe_log(in.x, transform.scale_log.x);
    let yv = maybe_log(in.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    let center_ndc = t * 2.0 - 1.0;

    let half_px = style.point_radius_px * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPlanetOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.local_pos = in.quad_pos;
    out.seed_inst = (u32(transform.style_params[0].w) ^ style.series_salt) + inst;
    out.rim_gain = clamp(transform.style_params[1].w, 0.0, 2.0);
    return out;
}

@fragment
fn fs_planet(in: VsPlanetOut) -> @location(0) vec4<f32> {
    let r_planet = max(style.point_radius_px, 1.0);
    let p = in.local_pos * (r_planet * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX);
    let dist = length(p);
    let aa = max(fwidth(dist), 0.5);

    // Per-point variation channels.
    let h_arch = sketch_hash01(in.seed_inst, 0xA2C4u);
    let h_spin = sketch_hash01(in.seed_inst, 0x591Eu);
    let h_jit = sketch_hash01(in.seed_inst, 0x717Au);

    // Ring frame: rotate into the ring plane. Position angle = series shape
    // mapping ± a small per-point jitter (real systems aren't aligned).
    let phi = cons_ring_angle(style.shape_id) + (h_jit - 0.5) * 0.12;
    let cphi = cos(phi);
    let sphi = sin(phi);
    let q = vec2<f32>(p.x * cphi + p.y * sphi, -p.x * sphi + p.y * cphi);

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
    let rho = length(vec2<f32>(q.x, q.y / RING_INCL)) / r_planet;
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
    let rim_d = max(dist - r_planet, 0.0);
    let rim = select(
        0.0,
        in.rim_gain * exp(-rim_d / max(r_planet * 0.10, 1.0)),
        dist > r_planet - aa,
    );
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

    if (a <= 0.003) {
        discard;
    }
    return vec4<f32>(rgb, a);
}
