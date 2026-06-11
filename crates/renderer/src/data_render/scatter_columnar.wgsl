// Columnar SDF scatter shader.
//
// slot 0: unit quad (per-vertex, ±1 NDC).
// slot 1: X column (per-instance, f32).
// slot 2: Y column (per-instance, f32).
//
// Marker shapes — style.shape_id, ScatterShape declaration order
// (see mod.rs::shape_id()):
//   0 Circle   1 Square   2 Triangle   3 Diamond   4 Cross (×)
//   5 CircleFilled   6 SquareFilled   7 TriangleFilled   8 DiamondFilled
// 0..3 stroke a ~1.5 px outline on the shape contour, 4 strokes the two
// diagonals, 5..8 fill the interior. All shapes are evaluated as signed
// distances in pixel space (see fs_main).

// ───── BEGIN common block (SHADER_COMMON.md) ─────
// WGSL has no import. The Transform/Style/binding/maybe_log definitions
// below are duplicated across scatter/line/errorbar shaders. To modify
// any of them, FIRST edit src/data_render/SHADER_COMMON.md, then mirror
// the change into every sibling shader. Do not edit only one file —
// silent drift here causes very hard-to-debug rendering bugs.
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> transform: Transform;

struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
    point_radius_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
    shape_id: u32,
    dash_len: u32,
    _pad: vec2<f32>,
    dash: array<vec4<f32>, 2>,
};

@group(1) @binding(0) var<uniform> style: Style;

struct VsIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: f32,
    @location(2) y: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
};

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}
// ───── END common block ─────
//
// Note: this shader inlines the `data_to_ndc` mapping inside `vs_main`
// rather than defining it as a function. The inline formula must match
// SHADER_COMMON.md §4 (4a / 4b are equivalent).

// Quad half-extent in pixels = point_radius_px + QUAD_MARGIN_PX. Outline
// strokes are centered on the shape contour, so they reach up to
// STROKE_HALF_PX plus the AA feather (~1 px) beyond the radius; without
// the margin the quad edge would clip them. fs_main must reconstruct
// pixel coordinates with this same expanded scale.
const QUAD_MARGIN_PX: f32 = 2.0;
// Half-width of the ~1.5 px contour stroke (outline shapes and Cross).
const STROKE_HALF_PX: f32 = 0.75;

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let xv = maybe_log(in.x, transform.scale_log.x);
    let yv = maybe_log(in.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    let center_ndc = t * 2.0 - 1.0;
    let half_px = style.point_radius_px + QUAD_MARGIN_PX;
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

// Signed pixel distance from `p` to the contour of `base` shape
// (0 circle, 1 square, 2 triangle, 3 diamond, 4 cross). Circle and square
// use radius r directly; triangle and diamond put their vertices on the
// radius-r circle. The cross is the two diagonals of the square inscribed
// in that circle; its distance is unsigned (zero on the strokes), which
// is exactly what the contour-stroke alpha needs.
fn shape_distance(base: u32, p: vec2<f32>, r: f32) -> f32 {
    var d: f32;
    switch base {
        case 0u: { d = length(p) - r; }
        case 1u: { d = max(abs(p.x), abs(p.y)) - r; }
        case 2u: { d = sd_triangle(p, r); }
        case 3u: { d = abs(p.x) + abs(p.y) - r; }
        default: { // 4u: Cross
            let c = r * 0.70710678; // r / sqrt(2)
            let d1 = sd_segment(p, vec2<f32>(-c, -c), vec2<f32>(c, c));
            let d2 = sd_segment(p, vec2<f32>(-c, c), vec2<f32>(c, -c));
            d = min(d1, d2);
        }
    }
    return d;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let r = style.point_radius_px;
    // Pixel-space position — same expanded scale as the vs_main quad, so
    // one unit here is one screen pixel.
    let p = in.local_pos * (r + QUAD_MARGIN_PX);

    // shape_id 5..8 are the filled twins of 0..3; 4 (Cross) is stroke-only.
    let filled = style.shape_id >= 5u;
    let base = select(style.shape_id, style.shape_id - 5u, filled);

    let d = shape_distance(base, p, r);
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
