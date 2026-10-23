struct VsIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: vec2<f32>,
    @location(2) y: vec2<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) polar_px: vec2<f32>,
    @location(2) @interpolate(flat) polar_limits: vec4<f32>,
    @location(3) @interpolate(flat) polar_sector: vec4<f32>,
};

// Quad half-extent in pixels = shape-specific axis extent + QUAD_MARGIN_PX.
// `point_radius_px` is the circle reference radius; non-circular shapes are
// normalized below so the same marker size has a similar visual weight.
// Outline
// strokes are centered on the shape contour, so they reach up to
// STROKE_HALF_PX plus the AA feather (~1 px) beyond the radius; without
// the margin the quad edge would clip them. fs_main must reconstruct
// pixel coordinates with this same expanded scale.

// ───── shared marker shape (SHADER_COMMON.md §5.5; `shader_consistency`
// compares these function bodies and constants with the pick shader) ─────

// The marker quad's margin past the shape's visual radius, in pixels:
// outline strokes and their anti-aliasing reach past the contour, and
// nothing a marker draws lies outside `visual_shape_radius + QUAD_MARGIN_PX`
// on either axis.
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

/// The coverage edge of a marker at shape distance `d`: a filled marker
/// covers `d < 0`; an outline (and an open plus or cross) is a stroke of
/// half-width `STROKE_HALF_PX` centred on `d == 0`. A fragment draws where
/// this is below its anti-aliasing width, and the picker's hit distance is
/// `max(marker_edge(d, filled), 0)`.
fn marker_edge(d: f32, filled: bool) -> f32 {
    return select(abs(d) - STROKE_HALF_PX, d, filled);
}

// ───── end shared marker shape ─────

@vertex
fn vs_main(in: VsIn) -> VsOut {
    if (!sample_valid(in.x, in.y)) {
        var masked: VsOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(in.x, in.y);
    let half_px =
        visual_shape_radius(base_shape_id(style.shape_id), style.point_radius_px) + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.local_pos = in.quad_pos;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
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
    let edge = marker_edge(d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}

// ──────────────── sketch mode (NOT part of the common block) ────────────────
// Hand-drawn entry points selected as a separate pipeline variant. The precise
// entries above never read the sketch Transform fields.

// Deliberately duplicated per data shader (scatter/line/errorbar) and NOT in
// the SHADER_COMMON.md common block: line_arc.wgsl shares that block but has
// no use for noise. Keep these helpers byte-identical in all three shaders.
// Per-point style mapping (NOT part of the common block). The default precise
// path above remains the three-slot fast path; this entry is selected only for
// scatter series with a style table, style-index column, or sparse overrides.
struct VsMappedIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: vec2<f32>,
    @location(2) y: vec2<f32>,
    @location(3) style_index: f32,
    // Value columns of the continuous maps (`point_color_map` /
    // `point_size_map`), per-instance hi/lo pairs read straight from the
    // column pool. A series without the map binds its X column here as inert
    // filler; the flags in `point_value_map` say which of them are real.
    @location(4) color_value: vec2<f32>,
    @location(5) size_value: vec2<f32>,
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

// Continuous value maps (group 3, NOT part of the common block). The colour
// side mirrors the field shader: the colourbar's bounds (already log10 when
// `VALUE_MAP_LOG_Z` is set), its `nan_color` and its ramp control points, so a
// colour-mapped point and a heatmap cell with the same z draw the same colour.
// The size side is `PointSizeMap`: value domain as hi/lo pairs and the pixel
// range already multiplied by the export scale. CPU twin:
// `mod.rs::PointValueMapParamsGpu`, 64 B.
struct PointValueMapParams {
    z_min: vec2<f32>,
    z_max: vec2<f32>,
    size_domain_min: vec2<f32>,
    size_domain_max: vec2<f32>,
    // (min_px, max_px)
    size_px: vec2<f32>,
    flags: u32,
    stop_count: u32,
    nan_color_premul: vec4<f32>,
};

const VALUE_MAP_COLOR: u32 = 1u;
const VALUE_MAP_SIZE: u32 = 2u;
const VALUE_MAP_LOG_Z: u32 = 4u;
const VALUE_MAP_SIZE_AREA: u32 = 8u;

@group(3) @binding(0) var<uniform> point_value_map: PointValueMapParams;
@group(3) @binding(1) var<storage, read> point_value_stops: array<vec4<f32>>;

fn value_map_flag(bit: u32) -> bool {
    return (point_value_map.flags & bit) != 0u;
}


// ───── shared colour ramp and size-map radius (SHADER_COMMON.md §5.4;
// `shader_consistency` compares these function bodies with the field, vector
// and pick shaders) ─────
