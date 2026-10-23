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

/// A finite f32 by its exponent bits. Every ramp input is checked
/// explicitly: NaN and infinity propagation through arithmetic is not
/// portable in WGSL.
fn ramp_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

/// `zv` (a hi/lo pair) as a ramp position in `[0, 1]`, or a negative number
/// when it cannot be placed: a non-finite half or sum, a non-positive value
/// on a logarithmic bar, or a degenerate range. `z_min` / `z_max` are the
/// colourbar bounds as hi/lo pairs, already log10 when `log_z`.
///
/// CPU twin: `ColorBarOptions::normalized_z`. "Unplaceable" and "smallest"
/// are different facts, so the miss is reported instead of clamping to an
/// end.
fn ramp_t(zv: vec2<f32>, z_min: vec2<f32>, z_max: vec2<f32>, log_z: bool) -> f32 {
    // Both halves are checked before they are added.
    if (!ramp_finite(zv.x) || !ramp_finite(zv.y)) {
        return -1.0;
    }
    let raw = zv.x + zv.y;
    if (!ramp_finite(raw)) {
        return -1.0;
    }
    if (log_z && raw <= 0.0) {
        return -1.0;
    }
    let is_log = select(0.0, 1.0, log_z);
    // Same shape as `axis_pair_to_t`: the linear form keeps the hi/lo split so
    // a large absolute z still resolves small deltas.
    let linear_num = (zv.x - z_min.x) + (zv.y - z_min.y);
    let log_num = (maybe_log(raw, 1.0) - z_min.x) - z_min.y;
    let span = (z_max.x - z_min.x) + (z_max.y - z_min.y);
    if (!ramp_finite(linear_num) || !ramp_finite(log_num)
        || !ramp_finite(span) || span <= 0.0) {
        return -1.0;
    }
    var numerator = mix(linear_num, log_num, is_log);
    if is_log > 0.5 {
        numerator = log_num;
    }
    let normalized = numerator / span;
    if (!ramp_finite(numerator) || !ramp_finite(normalized)) {
        return -1.0;
    }
    return clamp(normalized, 0.0, 1.0);
}

/// Where `t` falls among `n >= 2` control points: `x` is the lower stop's
/// index as a float, `y` the blend towards the next. Twin of the index and
/// fraction `model::colormap::sample` computes, including the `n - 2` index
/// clamp that lets the last segment own `t = 1`.
fn ramp_segment(t: f32, n: u32) -> vec2<f32> {
    let tc = select(clamp(t, 0.0, 1.0), 0.0, !ramp_finite(t));
    let scaled = tc * f32(n - 1u);
    let index = min(u32(floor(scaled)), n - 2u);
    return vec2<f32>(f32(index), scaled - f32(index));
}

/// The control points `a` (the segment's lower stop) and `b` blended by
/// `frac`, with the exact-endpoint short circuits `colormap::sample` has.
/// `mix` is not used: `a + (b - a) * t` is what the CPU computes, and the two
/// paths have to agree at the ends.
fn ramp_blend(a: vec4<f32>, b: vec4<f32>, frac: f32) -> vec4<f32> {
    if (frac <= 0.0) {
        return a;
    }
    if (frac >= 1.0) {
        return b;
    }
    return a + (b - a) * frac;
}

/// The marker radius for a size-map value `sv` (a hi/lo pair) over the
/// domain `domain_min..domain_max` (hi/lo pairs) onto `lo..hi` pixels, by
/// marker area when `area`, else by radius. Negative when the value is not
/// finite: the point is hidden. Twin of `PointSizeMap::radius_px`.
fn size_map_radius(sv: vec2<f32>, domain_min: vec2<f32>, domain_max: vec2<f32>, lo: f32, hi: f32, area: bool) -> f32 {
    if (!ramp_finite(sv.x) || !ramp_finite(sv.y) || !ramp_finite(sv.x + sv.y)) {
        return -1.0;
    }
    let num = (sv.x - domain_min.x) + (sv.y - domain_min.y);
    let span = (domain_max.x - domain_min.x) + (domain_max.y - domain_min.y);
    let t = clamp(num / span, 0.0, 1.0);
    if (area) {
        return sqrt(lo * lo + (hi * hi - lo * lo) * t);
    }
    return lo + (hi - lo) * t;
}

/// The colour-map value as a ramp position, or a negative number when it
/// cannot be placed.
fn value_ramp_t(zv: vec2<f32>) -> f32 {
    return ramp_t(zv, point_value_map.z_min, point_value_map.z_max, value_map_flag(VALUE_MAP_LOG_Z));
}

/// The colourbar's ramp at `t`, through the shared `ramp_segment` /
/// `ramp_blend`; only the stop lookup is this shader's.
fn value_ramp(t: f32) -> vec4<f32> {
    let n = point_value_map.stop_count;
    if (n == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    if (n == 1u) {
        return point_value_stops[0];
    }
    let segment = ramp_segment(t, n);
    let index = u32(segment.x);
    return ramp_blend(point_value_stops[index], point_value_stops[index + 1u], segment.y);
}

/// Premultiplied colour for a colour-map value; `nan_color` when unplaceable.
fn value_map_color(zv: vec2<f32>) -> vec4<f32> {
    let t = value_ramp_t(zv);
    if (t < 0.0) {
        return point_value_map.nan_color_premul;
    }
    let c = value_ramp(t);
    return vec4<f32>(c.rgb * c.a, c.a);
}

/// Marker radius for a size-map value, in pixels; negative hides the point.
fn value_map_radius(sv: vec2<f32>) -> f32 {
    return size_map_radius(
        sv,
        point_value_map.size_domain_min,
        point_value_map.size_domain_max,
        point_value_map.size_px.x,
        point_value_map.size_px.y,
        value_map_flag(VALUE_MAP_SIZE_AREA),
    );
}

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
    @location(4) polar_px: vec2<f32>,
    @location(5) @interpolate(flat) polar_limits: vec4<f32>,
    @location(6) @interpolate(flat) polar_sector: vec4<f32>,
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

// Precedence per channel: sparse override > continuous value map > style
// table slot > series base. A hidden point (non-finite size value) carries a
// negative radius; an override that sets the radius makes it visible again.
fn resolve_mapped_style(
    style_index: f32,
    inst: u32,
    color_value: vec2<f32>,
    size_value: vec2<f32>,
) -> ResolvedMappedStyle {
    var out = ResolvedMappedStyle(style.color_premul, style.point_radius_px, style.shape_id);
    let point_index = inst + scatter_style_meta._pad;

    if (scatter_style_meta.has_index != 0u && valid_style_index(style_index)) {
        let idx = u32(round(style_index));
        if (idx < scatter_style_meta.style_count) {
            out = apply_style_slot(out, scatter_style_slots[idx]);
        }
    }

    if (value_map_flag(VALUE_MAP_COLOR)) {
        out.color_premul = value_map_color(color_value);
    }
    if (value_map_flag(VALUE_MAP_SIZE)) {
        out.radius_px = value_map_radius(size_value);
    }

    for (var i = 0u; i < scatter_style_meta.override_count; i = i + 1u) {
        let ov = scatter_style_overrides[i];
        if (ov.point_index == point_index) {
            out = apply_style_slot(out, ScatterStyleSlot(ov.color_premul, ov.params));
        }
    }
    return out;
}

@vertex
fn vs_mapped(in: VsMappedIn, @builtin(instance_index) inst: u32) -> VsMappedOut {
    if (!sample_valid(in.x, in.y)) {
        var masked: VsMappedOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let resolved = resolve_mapped_style(in.style_index, inst, in.color_value, in.size_value);
    if (resolved.radius_px < 0.0) {
        var hidden: VsMappedOut;
        hidden.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return hidden;
    }
    let center_ndc = data_to_ndc(in.x, in.y);
    let half_px =
        visual_shape_radius(base_shape_id(resolved.shape_id), resolved.radius_px) + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsMappedOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.local_pos = in.quad_pos;
    out.color_premul = resolved.color_premul;
    out.radius_px = resolved.radius_px;
    out.shape_id = resolved.shape_id;
    return out;
}

@fragment
fn fs_mapped(in: VsMappedOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let filled = shape_is_filled(in.shape_id);
    let base = base_shape_id(in.shape_id);
    let visual_r = visual_shape_radius(base, in.radius_px);
    let p = in.local_pos * (visual_r + QUAD_MARGIN_PX);

    let d = shape_distance(base, p, visual_r, filled);
    let aa = fwidth(d);
    let edge = marker_edge(d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return in.color_premul * alpha;
}

// The picked ring's marker mode (`Style.primitive_flags` of the ring entry):
// the draw style whose marker it surrounds, numbered like the pick shader's
// `PICK_MARKER_*` (`gpu_pick.rs::GpuPickDrawStyle`).
const RING_MARKER_PRECISE: u32 = 0u;
const RING_MARKER_SKETCH: u32 = 1u;
const RING_MARKER_PLANET: u32 = 2u;
const RING_MARKER_STAR: u32 = 3u;

/// The farthest a precise marker of `shape_id` and radius `r` covers from its
/// centre: the circumradius of its contour (`shape_distance` at the visual
/// radius, sized like `visual_shape_radius`), plus the stroke half-width for
/// an outline or an open plus or cross. `marker_edge <= 0` lies within it.
fn marker_extent_px(shape_id: u32, r: f32) -> f32 {
    let filled = shape_is_filled(shape_id);
    let base = base_shape_id(shape_id);
    let rv = visual_shape_radius(base, max(r, 0.0));
    var extent = rv;
    switch base {
        case 1u: { extent = rv * sqrt(2.0); } // square corner
        case 4u: { // cross: the diagonal strokes, or the rotated filled plus
            if (filled) {
                let arm = max(rv * 0.32, STROKE_HALF_PX);
                extent = sqrt(rv * rv + arm * arm);
            } else {
                extent = rv * sqrt(2.0);
            }
        }
        case 8u: { // plus: the arm ends, or the filled arms' corners
            if (filled) {
                let arm = max(rv * 0.32, STROKE_HALF_PX);
                extent = sqrt(rv * rv + arm * arm);
            }
        }
        default: {} // circle, triangles, diamond, polygons, star: vertices on rv
    }
    return extent + select(STROKE_HALF_PX, 0.0, filled);
}

/// How far the marker the chart's draw style paints at instance `inst`
/// reaches from its centre, on the envelope the picker picks (SHADER_COMMON.md
/// §5.7): the precise contour, grown by the sketch wobble; a Milkyway
/// planet's disc, rim glow and ring band; a Constellation star's sprite disc.
fn ring_marker_extent_px(shape_id: u32, r: f32, inst: u32) -> f32 {
    let mode = style.primitive_flags & 3u;
    if (mode == RING_MARKER_STAR) {
        let star = constellation_star_radius(r, styled_point_index(inst), style.series_salt);
        return star * 4.0 + QUAD_MARGIN_PX;
    }
    if (mode == RING_MARKER_PLANET) {
        // In the planet's own frame (`fs_planet` scales the quad to
        // `r_planet`): the ring band's major axis, or the rim glow's reach
        // above `PLANET_ALPHA_FLOOR`, inside the quad's corner.
        let r_planet = max(r, 1.0);
        let rim_gain = clamp(transform.style_params[1].w, 0.0, 2.0);
        var reach = r_planet * RING_OUTER;
        if (rim_gain > PLANET_ALPHA_FLOOR) {
            reach = max(reach, r_planet + max(r_planet * 0.10, 1.0) * log(rim_gain / PLANET_ALPHA_FLOOR));
        }
        let quad = r * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX;
        let screen = reach * quad / (r_planet * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX);
        return min(screen, quad * sqrt(2.0));
    }
    let extent = marker_extent_px(shape_id, r);
    if (mode == RING_MARKER_SKETCH) {
        return extent + sketch_marker_wobble_px(transform.style_params[0].x, r);
    }
    return extent;
}

struct VsPickRingOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) radius_px: f32,
    @location(2) polar_px: vec2<f32>,
    @location(3) @interpolate(flat) polar_limits: vec4<f32>,
    @location(4) @interpolate(flat) polar_sector: vec4<f32>,
};

// The picked ring: a circle `radius_extra_px` (`Style.cap_half_px`) outside
// the marker the chart draws at that point (`ring_marker_extent_px` of the
// marker radius `Style.point_radius_px` and shape `Style.shape_id`, both at
// the render scale).
@vertex
fn vs_pick_ring(in: VsIn, @builtin(instance_index) inst: u32) -> VsPickRingOut {
    if (!sample_valid(in.x, in.y)) {
        var masked: VsPickRingOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(in.x, in.y);
    let radius = max(ring_marker_extent_px(style.shape_id, style.point_radius_px, inst) + style.cap_half_px, 0.0);
    let half_px = radius + max(style.line_width_px, 0.0) * 0.5 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPickRingOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.local_pos = in.quad_pos;
    out.radius_px = radius;
    return out;
}

@vertex
fn vs_pick_ring_mapped(in: VsMappedIn, @builtin(instance_index) inst: u32) -> VsPickRingOut {
    if (!sample_valid(in.x, in.y)) {
        var masked: VsPickRingOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let resolved = resolve_mapped_style(in.style_index, inst, in.color_value, in.size_value);
    if (resolved.radius_px < 0.0) {
        var hidden: VsPickRingOut;
        hidden.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return hidden;
    }
    let center_ndc = data_to_ndc(in.x, in.y);
    // The ring entries use style.cap_half_px as radius_extra_px. The
    // errorbar shader is not involved in this pipeline, so the slot is local
    // to this decoration entry.
    let radius = max(ring_marker_extent_px(resolved.shape_id, resolved.radius_px, inst) + style.cap_half_px, 0.0);
    let half_px = radius + max(style.line_width_px, 0.0) * 0.5 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPickRingOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.local_pos = in.quad_pos;
    out.radius_px = radius;
    return out;
}

@fragment
fn fs_pick_ring(in: VsPickRingOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
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
