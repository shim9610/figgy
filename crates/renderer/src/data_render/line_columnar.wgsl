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
// `strip_slot` and the CPU twin `line_strip_vertex_count` in data_render/line.rs. The
// arc-length scan (line_arc.wgsl) and the GPU/CPU pickers walk the same
// pieces, so dash phase and picking stay on the stepped path.
//
// `vs_area` / `fs_area` fill the area under the line (to a baseline carried
// in Style.dash[1].xy) or between this line and a second y column bound in
// slots 4/5 (Style.primitive_flags bit 4). One instance per segment emits one
// trapezoid per step piece. It is a separate pipeline ("figgy line area
// pipeline") because its slots 4/5 are (hi, lo) pairs, not arc lengths.

struct VsIn {
    @location(0) x_a: vec2<f32>,
    @location(1) y_a: vec2<f32>,
    @location(2) x_b: vec2<f32>,
    @location(3) y_b: vec2<f32>,
};

// Line-only extra instance inputs: cumulative
// arc length at the segment's two endpoints, from the GPU-computed prefix,
// as arc pairs (SHADER_COMMON.md §5.6 arc pairs).
struct VsArc {
    @location(4) arc_a: vec2<u32>,
    @location(5) arc_b: vec2<u32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Dash position (px): A's arc modulo the dash period at the two A-side
    // corners, plus the segment's arc length at the two B-side corners —
    // interpolation yields the per-fragment position the dash pattern walks.
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
        total = total + step_piece_len_px(transform, step_piece(in.x_a, in.y_a, in.x_b, in.y_b, mode, j));
    }
    return total;
}

// Strip position → (piece, local vertex) for an instance made of `pieces`
// strips of `per_piece` vertices each, joined by two duplicated vertices per
// joint: the last vertex of the piece and the first of the next, which fold
// the connecting triangles to zero area. Total = pieces · per_piece +
// 2 · (pieces − 1); a one-piece instance is exactly the plain strip. CPU
// twin: `line_strip_vertex_count` in data_render/line.rs.
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
    return line_vertex(in, arc, vid);
}

// The precise stroke's vertex: `vs_main`, and `vs_main_mapped` under its
// colour value.
fn line_vertex(in: VsIn, arc: VsArc, vid: u32) -> VsOut {
    if (!sample_valid(transform, in.x_a, in.y_a) || !sample_valid(transform, in.x_b, in.y_b)) {
        var masked: VsOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    // One quad per step piece; a linear line is the single piece a → b.
    let mode = line_step_mode();
    let slot = strip_slot(vid, 4u);
    let piece = step_piece(in.x_a, in.y_a, in.x_b, in.y_b, mode, slot.piece);
    let last_piece = slot.piece + 1u == step_piece_count(mode);
    var a_ndc = data_to_ndc(transform, piece.x0, piece.y0);
    var b_ndc = data_to_ndc(transform, piece.x1, piece.y1);
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

    // Dash phase: this piece starts at A's arc (reduced modulo the dash
    // period) plus the pieces before it and ends at the scanned prefix for B
    // on the last piece, so the phase is continuous through every corner and
    // across the joint.
    let phase = arc_dash_phase(arc.arc_a, dash_period(style.dash, style.dash_len));
    let arc_start = phase + step_prefix_px(in, mode, slot.piece);
    let arc_end = select(arc_start + len, phase + arc_delta(arc.arc_b, arc.arc_a), last_piece);

    var out: VsOut;
    out.pos = vec4<f32>(corner_ndc, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
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

// ───────────── colour-mapped stroke ─────────────
// `vs_main_mapped` / `fs_main_mapped` draw the precise stroke of a
// streamline series with a colour column (SHADER_COMMON.md §5.6): the same
// geometry as `vs_main`, and the colour of the value interpolated along the
// segment through the colourbar ramp, which a colour-mapped marker or a
// heatmap cell of that value takes too. The value column is bound as slots
// 6/7 (hi/lo pairs, one value shifted like x/y) and the colourbar as
// group 3, the point value map (`PointValueMapParams`, §5.2); group 2 is
// left unread. Nothing per point is stored: the colour is computed on every
// draw.
struct VsValue {
    @location(6) value_a: vec2<f32>,
    @location(7) value_b: vec2<f32>,
};

struct MappedVsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) dist_px: f32,
    @location(1) along_px: f32,
    @location(2) side_px: f32,
    @location(3) segment_len_px: f32,
    @location(4) polar_px: vec2<f32>,
    @location(5) @interpolate(flat) polar_limits: vec4<f32>,
    @location(6) @interpolate(flat) polar_sector: vec4<f32>,
    // The colour value as a hi/lo pair, interpolated along the stroke.
    @location(7) value: vec2<f32>,
};

// The colourbar of a colour-mapped stroke. CPU twin:
// `data_render/value_map.rs::PointValueMapParamsGpu`, 64 B (the scatter
// shader's struct; this line module binds the same group).
struct PointValueMapParams {
    z_min: vec2<f32>,
    z_max: vec2<f32>,
    size_domain_min: vec2<f32>,
    size_domain_max: vec2<f32>,
    size_px: vec2<f32>,
    flags: u32,
    stop_count: u32,
    nan_color_premul: vec4<f32>,
};

const VALUE_MAP_COLOR: u32 = 1u;
const VALUE_MAP_LOG_Z: u32 = 4u;

@group(3) @binding(0) var<uniform> line_value_map: PointValueMapParams;
@group(3) @binding(1) var<storage, read> line_value_stops: array<vec4<f32>>;

// The colourbar's ramp at `t`, through the shared `ramp_segment` /
// `ramp_blend`; only the stop lookup is this shader's.
fn line_value_ramp(t: f32) -> vec4<f32> {
    let n = line_value_map.stop_count;
    if (n == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    if (n == 1u) {
        return line_value_stops[0];
    }
    let segment = ramp_segment(t, n);
    let index = u32(segment.x);
    return ramp_blend(line_value_stops[index], line_value_stops[index + 1u], segment.y);
}

// Premultiplied stroke colour for a colour value: the series colour without
// a colour map, `nan_color` where the ramp cannot place the value.
fn line_value_color(zv: vec2<f32>) -> vec4<f32> {
    if ((line_value_map.flags & VALUE_MAP_COLOR) == 0u) {
        return style.color_premul;
    }
    let t = ramp_t(zv, line_value_map.z_min, line_value_map.z_max, (line_value_map.flags & VALUE_MAP_LOG_Z) != 0u);
    if (t < 0.0) {
        return line_value_map.nan_color_premul;
    }
    let c = line_value_ramp(t);
    return vec4<f32>(c.rgb * c.a, c.a);
}

@vertex
fn vs_main_mapped(in: VsIn, arc: VsArc, value: VsValue, @builtin(vertex_index) vid: u32) -> MappedVsOut {
    let base = line_vertex(in, arc, vid);
    var out: MappedVsOut;
    out.pos = base.pos;
    out.dist_px = base.dist_px;
    out.along_px = base.along_px;
    out.side_px = base.side_px;
    out.segment_len_px = base.segment_len_px;
    out.polar_px = base.polar_px;
    out.polar_limits = base.polar_limits;
    out.polar_sector = base.polar_sector;
    // The corner's position along the unclipped segment, square cap and
    // viewport clip included, so the interpolated value is A's to B's
    // linearly along the drawn segment, as an arrowhead on it reads it.
    let a_px = data_to_px(transform, in.x_a, in.y_a);
    let b_px = data_to_px(transform, in.x_b, in.y_b);
    let d = b_px - a_px;
    let d2 = dot(d, d);
    let f = select(0.0, dot(base.pos.xy / transform.pixel_to_ndc - a_px, d) / d2, d2 > 1e-12);
    out.value = value.value_a + (value.value_b - value.value_a) * f;
    return out;
}

@fragment
fn fs_main_mapped(in: MappedVsOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    var stroke: VsOut;
    stroke.pos = in.pos;
    stroke.dist_px = in.dist_px;
    stroke.along_px = in.along_px;
    stroke.side_px = in.side_px;
    stroke.segment_len_px = in.segment_len_px;
    stroke.polar_px = in.polar_px;
    stroke.polar_limits = in.polar_limits;
    stroke.polar_sector = in.polar_sector;
    return line_fragment_color(stroke, line_value_color(in.value));
}

@fragment
fn fs_constellation_line(in: VsOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let a = clamp(transform.style_params[0].y, 0.0, 1.0);
    return line_fragment_color(in, vec4<f32>(style.color_premul.rgb * a, style.color_premul.a * a));
}

// ───────────────── area fill ─────────────────
// `vs_area` fills under the line or between two lines. Slots 0-3 are the
// line's own (x_a, y_a, x_b, y_b); slots 4/5 carry the second y column as
// (hi, lo) pairs with the same one-value shift (fill to a series), or the y
// column itself as inert filler (fill to a baseline, read from
// Style.dash[1].xy — the bar shader's baseline convention). Each step piece
// is one trapezoid, the shared fragment `common/area_piece.wgsl`, which the
// picker tests too. Hardware clipping is enough here — there is no stroke
// SDF to protect.
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

@vertex
fn vs_area(in: VsIn, base: VsAreaBase, @builtin(vertex_index) vid: u32) -> AreaOut {
    var masked: AreaOut;
    masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    let to_series = (style.primitive_flags & AREA_FILL_TO_SERIES) != 0u;
    let baseline = area_baseline();
    if (!area_segment_drawn(transform, in.x_a, in.y_a, in.x_b, in.y_b, base.base_a, base.base_b, to_series, baseline)) {
        return masked;
    }

    let mode = line_step_mode();
    let slot = strip_slot(vid, 4u);
    let piece = area_piece(in.x_a, in.y_a, in.x_b, in.y_b, base.base_a, base.base_b, to_series, baseline, mode, slot.piece);
    let at_b = slot.local >= 2u;
    let on_base = (slot.local & 1u) == 1u;
    var out: AreaOut;
    out.pos = vec4<f32>(area_corner_ndc(transform, piece, at_b, on_base, to_series), 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    return out;
}

@fragment
fn fs_area(in: AreaOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    return style.color_premul;
}

// ──────────────── sketch mode ────────────────
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
    if (!sample_valid(transform, in.x_a, in.y_a) || !sample_valid(transform, in.x_b, in.y_b)) {
        var masked: VsOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    // Each step piece is its own 2·(S+1) strip, joined like vs_main's quads.
    let mode = line_step_mode();
    let slot = strip_slot(vid, 2u * (SKETCH_SUBDIV + 1u));
    let piece = step_piece(in.x_a, in.y_a, in.x_b, in.y_b, mode, slot.piece);
    let last_piece = slot.piece + 1u == step_piece_count(mode);
    let a_ndc = data_to_ndc(transform, piece.x0, piece.y0);
    let b_ndc = data_to_ndc(transform, piece.x1, piece.y1);

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

    // The arc offset from A at this knot: the noise walks it from A's arc
    // pair, the dash from A's phase.
    let arc_start = step_prefix_px(in, mode, slot.piece);
    let arc_end = select(arc_start + len, arc_delta(arc.arc_b, arc.arc_a), last_piece);
    let offset_at = mix(arc_start, arc_end, t);
    let seed = u32(transform.style_params[0].z) ^ style.series_salt;
    let disp = sketch_line_offset_from(arc.arc_a, offset_at, transform.style_params[0].x, transform.style_params[0].y, seed);
    let arc_at = arc_dash_phase(arc.arc_a, dash_period(style.dash, style.dash_len)) + offset_at;

    let corner_px = center_px + dir * cap + normal_px * (geom_half_w * side + disp);
    let corner_ndc = corner_px * transform.pixel_to_ndc;

    var out: VsOut;
    out.pos = vec4<f32>(corner_ndc, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    out.dist_px = arc_at + select(0.0, cap, style.dash_len == 0u);
    if (style.dash_len > 0u) {
        out.dist_px = arc_at + select(0.0, -half_w, k == 0u) + select(0.0, half_w, k == SKETCH_SUBDIV);
    }
    out.along_px = t * len + cap;
    out.side_px = geom_half_w * side;
    out.segment_len_px = len;
    return out;
}

// ────────────── constellation mode ───────────
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
// CPU draw-count twin lives in data_render/stylized.rs: MILKYWAY_RIBBON_VERTICES =
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

// Local ribbon/temperature modulation, 0.35 .. 1.65, at arc `origin +
// offset_px` (an arc pair and a small offset, §5.6 arc pairs).
fn cons_clump(origin: vec2<u32>, offset_px: f32, seed: u32) -> f32 {
    let wavelength = CONS_CLUMP_WAVELENGTH_PX * cons_structure_scale();
    return 1.0 + 0.65 * arc_noise(origin, offset_px, wavelength, seed ^ 0xC10Du);
}

// Stellar population mix 0..1: 0 = old/warm (dense knots), 1 = young/hot.
fn cons_pop(origin: vec2<u32>, offset_px: f32, seed: u32) -> f32 {
    let wavelength = CONS_CLUMP_WAVELENGTH_PX * cons_structure_scale();
    return 0.5 + 0.5 * arc_noise(origin, offset_px, wavelength, seed ^ 0x090Bu);
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
    if (!sample_valid(transform, in.x_a, in.y_a) || !sample_valid(transform, in.x_b, in.y_b)) {
        var masked: RibbonOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    // Step pieces: one ribbon strip per piece, joined like vs_main's quads.
    let mode = line_step_mode();
    let slot = strip_slot(vid, 2u * (CONS_RIBBON_SUBDIV + 1u));
    let piece = step_piece(in.x_a, in.y_a, in.x_b, in.y_b, mode, slot.piece);
    let last_piece = slot.piece + 1u == step_piece_count(mode);
    let a_px = data_to_px(transform, piece.x0, piece.y0);
    let b_px = data_to_px(transform, piece.x1, piece.y1);
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
    // The arc offset from A at this knot.
    let arc_start = step_prefix_px(in, mode, slot.piece);
    let arc_end = select(arc_start + len, arc_delta(arc.arc_b, arc.arc_a), last_piece);
    let offset_px = mix(arc_start, arc_end, t);

    var out: RibbonOut;
    out.pos = vec4<f32>(corner_px * transform.pixel_to_ndc, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    out.cross_d = side;
    out.center_i = intensity_cfg * cons_clump(arc.arc_a, offset_px, seed);
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

// The star pass reads its arc slots through `common/arc_walk.wgsl` (group
// 3: the arc prefix, the pool and `StarVsParams`).

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
    // inside its stratum, as an exact arc pair (§5.6 arc pairs). Attributes
    // hash slot IDS (not positions), so the pattern upstream of a data edit
    // stays put.
    let arc_here = arc_slot_position(
        primary_id,
        0.5 + 0.8 * (cons_h(primary_id, 0x0701u, seed) - 0.5),
        pitch,
    );

    // Existence gate: expected stars per slot = density * pitch / 100.
    // Keep this independent of the clump field so the line is sewn uniformly
    // by arc length rather than looking denser around particular samples.
    let p_exist = clamp(density * pitch / 100.0, 0.0, 1.0);
    // A slot the existence gate drops draws nothing: skip its segment search
    // and arc arithmetic (most slots at the default density).
    if (!(cons_h(slot, 0x0E15u, seed) < p_exist)) {
        var dropped: StarOut;
        dropped.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return dropped;
    }
    var alive = arc_less(arc_here, total);

    // Binary search: greatest g with prefix[g] <= arc_here; segment (g, g+1).
    let lo = arc_walk_segment(n, arc_here);
    let seg_a = star_arc[lo];
    let seg_len = arc_delta(star_arc[lo + 1u], seg_a);
    // The slot's offset from the segment's start.
    let arc_from_a = arc_delta(arc_here, seg_a);
    // NaN-gap segments contribute zero prefix advance — a slot landing on a
    // flat span (boundary ties) dies instead of drawing inside a gap.
    if (seg_len <= 0.0) { alive = false; }
    let t = clamp(arc_from_a / max(seg_len, 1e-6), 0.0, 1.0);

    if (!sample_valid(transform, star_pool_pair(star_vsp.x_base, lo), star_pool_pair(star_vsp.y_base, lo))
        || !sample_valid(transform, star_pool_pair(star_vsp.x_base, lo + 1u), star_pool_pair(star_vsp.y_base, lo + 1u))) {
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
        var arc_in = arc_from_a;
        var k = 0u;
        loop {
            piece = step_piece(x_lo, y_lo, x_hi, y_hi, star_vsp.mode, k);
            let piece_len = step_piece_len_px(transform, piece);
            if (k + 1u >= pieces || arc_in <= piece_len) {
                t_piece = clamp(arc_in / max(piece_len, 1e-6), 0.0, 1.0);
                break;
            }
            arc_in = arc_in - piece_len;
            k = k + 1u;
        }
    }
    let a_px = data_to_px(transform, piece.x0, piece.y0);
    let b_px = data_to_px(transform, piece.x1, piece.y1);
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
    let pop = clamp(cons_pop(arc_here, 0.0, seed), 0.0, 1.0);
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
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
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
