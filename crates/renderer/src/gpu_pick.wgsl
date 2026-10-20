// Exact GPU point/segment picking over ColumnPool split-f32 values.
//
// This module deliberately uses pick_* names and its own layouts. It is not
// part of the render-shader common block: the picker consumes the same public
// data representation but owns a compute-only ABI.
//
// Small queries scan directly. Larger queries use two streaming gates instead
// of a traversal index. The X pass reads one axis and writes one scatter/line
// bit pair for every 32 logical indices. The Y pass narrows only those bits.
// The exact pass reads the surviving original indices and performs the
// production distance calculation.

const PICK_GATE_WORD_POINTS: u32 = 32u;
const PICK_WORKGROUP_SIZE: u32 = 64u;
const PICK_F32_MAX: f32 = 0x1.fffffep+127;
const PICK_F32_EPSILON: f32 = 1.1920929e-7;

const PICK_FLAG_SCATTER: u32 = 1u;
const PICK_FLAG_LINE: u32 = 2u;
const PICK_FLAG_STYLE_MAP: u32 = 4u;
const PICK_FLAG_STYLE_INDEX: u32 = 8u;
const PICK_FLAG_SIZE_MAP: u32 = 16u;
const PICK_FLAG_SIZE_AREA: u32 = 32u;
// A point vector series: its "markers" are the arrows the vector shader
// draws from each anchor.
const PICK_FLAG_VECTOR: u32 = 64u;
// `PickQueryParams.vector.w` bit 8: `VectorScale::Pixels` (else `Data`).
const PICK_VECTOR_SCALE_PIXELS: u32 = 256u;

const PICK_STYLE_MASK_RADIUS: u32 = 2u;
const PICK_STYLE_MASK_SHAPE: u32 = 4u;

struct PickQueryTransform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    data_to_panel_scale: vec2<f32>,
    data_to_panel_offset: vec2<f32>,
    style_params: array<vec4<f32>, 3>,
    polar_angle: vec4<f32>,
    polar_extent: vec4<f32>,
};

struct PickQueryParams {
    transform: PickQueryTransform,
    // (cursor_x, cursor_y, chart_x, chart_y), all canvas pixels.
    cursor_chart: vec4<f32>,
    // (chart_width, chart_height, maximum pick distance, maximum per-axis
    // reach of a drawn marker from its centre in canvas pixels, the quad
    // margin included). Exact radii/widths are resolved after both gates.
    chart_limits: vec4<f32>,
    // (base scatter radius, line half width, display scale, direct-scan flag).
    scatter_line: vec4<f32>,
    // (point count, x f32-lane base, y f32-lane base, style-index lane base).
    data: vec4<u32>,
    // (flags, style count, override count, style-index logical length).
    style: vec4<u32>,
    // (gate-word count, series order, base shape id, dispatch X width).
    series: vec4<u32>,
    // (global point offset, owned scatter count, owned segment count,
    //  optional original-row map word base; zero disables the map).
    stream: vec4<u32>,
    // (step interpolation mode: 0 linear, 1 step-pre, 2 step-post,
    //  3 step-mid; 1 when `pick_arc` holds this series' cumulative arc
    //  prefix; the rest reserved). The step mode mirrors the line shader's
    //  Style.primitive_flags bits 2-3 so a pick lands on the drawn path.
    line: vec4<u32>,
    // `point_size_map`: (size f32-lane base, size logical length, 0, 0).
    size_map: vec4<u32>,
    // (domain_min hi, domain_min lo, domain_max hi, domain_max lo).
    size_domain: vec4<f32>,
    // (min_px, max_px, 0, 0), logical pixels; the display scale is applied
    // with the other radii.
    size_px: vec4<f32>,
    // Line geometry in canvas pixels, the display scale applied: (stroke
    // half-width `max(width, 1) / 2`, milkyway half extent, per-axis gate
    // reach of the drawn stroke past its centreline, 0).
    line_px: vec4<f32>,
    // The line's dash pattern in canvas pixels: up to eight sequential
    // [on, off, ...] lengths, `styled.w` of them used (the line `Style.dash`).
    line_dash: array<vec4<f32>, 2>,
    // (marker draw mode, line draw mode, series salt of the styled hashes,
    //  dash count).
    styled: vec4<u32>,
    // A point vector series (`PICK_FLAG_VECTOR`): (u f32-lane base, v f32-lane
    // base, arrow count — the shortest column the vector shader reads —,
    // pivot | `PICK_VECTOR_SCALE_PIXELS`).
    vector: vec4<u32>,
    // (scale — the `Data` factor, or `px_per_unit` times the display scale —,
    //  shaft width, head length, head width), canvas pixels.
    vector_px: vec4<f32>,
    // (minimum length, 0, 0, 0), canvas pixels.
    vector_min: vec4<f32>,
};

const PICK_STEP_LINEAR: u32 = 0u;
const PICK_STEP_PRE: u32 = 1u;
const PICK_STEP_POST: u32 = 2u;
const PICK_STEP_MID: u32 = 3u;

struct PickScatterStyleSlot {
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct PickScatterStyleOverride {
    point_index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

// Thirty-two bytes, byte-for-byte with Rust's PickCandidateGpu. The public
// contract returns only identity/index/distance, so no reconstructed data value
// is carried through workgroup storage or readback.
struct PickCandidate {
    valid: u32,
    series_order: u32,
    point_index: u32,
    primitive_kind: u32, // scatter=0, line=1 (CPU traversal order)
    primitive_index: u32,
    distance_sq: f32,
    distance_px: f32,
    _pad: u32,
};

@group(0) @binding(0) var<storage, read> pick_pool_words: array<u32>;
// x=scatter bits, y=line-segment bits. One word pair covers 32 indices.
@group(0) @binding(1) var<storage, read_write> pick_gate_masks: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read_write> pick_workgroup_candidates: array<PickCandidate>;

// Reduction-only resources share group 0 but use bindings not reachable from
// the gate/exact entry points.
@group(0) @binding(3) var<storage, read> pick_reduce_inputs: array<PickCandidate>;
@group(0) @binding(4) var<storage, read_write> pick_reduce_output: PickCandidate;

@group(1) @binding(0) var<uniform> pick_query_params: PickQueryParams;
@group(1) @binding(1) var<storage, read> pick_style_slots: array<PickScatterStyleSlot>;
@group(1) @binding(2) var<storage, read> pick_style_overrides: array<PickScatterStyleOverride>;
@group(1) @binding(3) var<storage, read_write> pick_series_output: PickCandidate;

// The exact pass only: the line's cumulative arc length per point, the
// buffer the line shader's dash phase and sketch wobble were drawn with
// (`line_arc.wgsl`), or a one-value placeholder when `line.y` is zero.
@group(2) @binding(0) var<storage, read> pick_arc: array<f32>;

var<workgroup> pick_shared_candidates: array<PickCandidate, PICK_WORKGROUP_SIZE>;

// Windows DX12: bounded per-invocation zeroing avoids aggregate DXIL stores.
// Keep this override paired with gpu_compile::workgroup_options.
override FIGGY_EXPLICIT_WORKGROUP_ZERO: bool = false;

fn figgy_zero_workgroup(local_index: u32) {
    if FIGGY_EXPLICIT_WORKGROUP_ZERO {
        pick_shared_candidates[local_index] = PickCandidate();
        workgroupBarrier();
    }
}

fn pick_is_finite(v: f32) -> bool {
    return v == v && abs(v) <= PICK_F32_MAX;
}

fn pick_read_pair(base: u32, point_index: u32) -> vec2<f32> {
    let word = base + point_index * 2u;
    return vec2<f32>(bitcast<f32>(pick_pool_words[word]), bitcast<f32>(pick_pool_words[word + 1u]));
}

fn pick_source_row(local_index: u32) -> u32 {
    if pick_query_params.stream.w != 0u {
        return pick_pool_words[pick_query_params.stream.w + local_index];
    }
    return local_index + pick_query_params.stream.x;
}

fn pick_pair_is_valid(v: vec2<f32>) -> bool {
    return pick_is_finite(v.x) && pick_is_finite(v.y) && pick_is_finite(v.x + v.y);
}

fn pick_coordinate_is_valid(v: vec2<f32>, axis: u32) -> bool {
    return pick_pair_is_valid(v)
        && (pick_query_params.transform.scale_log[axis] == 0.0 || v.x + v.y > 0.0);
}

fn pick_log10(v: f32) -> f32 {
    return log(max(v, 1e-30)) / log(10.0);
}

fn pick_axis_pair_to_t(v: vec2<f32>, axis: u32) -> f32 {
    let raw = v.x + v.y;
    let min_hi = pick_query_params.transform.data_min[axis];
    let max_hi = pick_query_params.transform.data_max[axis];
    let min_lo = pick_query_params.transform.data_min_lo[axis];
    let max_lo = pick_query_params.transform.data_max_lo[axis];
    let linear_num = fma(1.0, v.x - min_hi, v.y - min_lo);
    let range = fma(1.0, max_hi - min_hi, max_lo - min_lo);
    let log_num = (pick_log10(raw) - min_hi) - min_lo;
    let data_t = mix(linear_num / range, log_num / range, pick_query_params.transform.scale_log[axis]);
    return fma(data_t, pick_query_params.transform.data_to_panel_scale[axis],
        pick_query_params.transform.data_to_panel_offset[axis]);
}

fn pick_project_axis_pair(v: vec2<f32>, axis: u32) -> f32 {
    let t = pick_axis_pair_to_t(v, axis);
    if axis == 0u {
        return pick_query_params.cursor_chart.z + t * pick_query_params.chart_limits.x;
    }
    return pick_query_params.cursor_chart.w + (1.0 - t) * pick_query_params.chart_limits.y;
}

fn pick_polar() -> bool {
    return pick_query_params.transform.polar_angle.x > 0.5;
}

// Twin of `polar_ndc` in the data shaders, in canvas pixels.
fn pick_project_pair_polar(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    let t = pick_query_params.transform;
    let ty = pick_axis_pair_to_t(y, 1u);
    let angle = t.polar_angle.y + t.polar_angle.z * (x.x + x.y);
    let rt = (ty - t.data_to_panel_offset.y) / t.data_to_panel_scale.y;
    let r = mix(t.polar_angle.w, 1.0, max(rt, 0.0));
    let center = (t.data_to_panel_offset + t.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    let ndc = center + vec2<f32>(cos(angle), sin(angle)) * r * t.polar_extent.w * t.pixel_to_ndc;
    return vec2<f32>(
        pick_query_params.cursor_chart.z + (ndc.x + 1.0) * 0.5 * pick_query_params.chart_limits.x,
        pick_query_params.cursor_chart.w + (1.0 - ndc.y) * 0.5 * pick_query_params.chart_limits.y,
    );
}

fn pick_project_pair(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    if pick_polar() {
        return pick_project_pair_polar(x, y);
    }
    return vec2<f32>(pick_project_axis_pair(x, 0u), pick_project_axis_pair(y, 1u));
}

// Twin of `polar_sample_valid`: a radius below the axis start is not drawn,
// so it is not picked. The sector, the outer ring and the hole cut the drawn
// primitive per fragment instead; `pick_polar_clip_distance` applies that
// clip at the cursor.
fn pick_polar_valid(x: vec2<f32>, y: vec2<f32>) -> bool {
    if !pick_polar() {
        return true;
    }
    let t = pick_query_params.transform;
    let ty = pick_axis_pair_to_t(y, 1u);
    let rt = (ty - t.data_to_panel_offset.y) / t.data_to_panel_scale.y;
    return rt >= 0.0;
}

// The polar fragment clip of the draw shaders (SHADER_COMMON.md, coordinate
// sample validity). `polar_fragment_visible` is copied byte-for-byte; the
// three inputs it takes are computed from this shader's transform exactly
// as `polar_clip_offset` / `polar_clip_limits` / `polar_clip_sector` compute
// them from the draw transform.
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

// The cursor's offset from the polar centre in pixels, y up: what
// `polar_clip_offset` interpolates to at the fragment under the cursor.
fn pick_polar_cursor_offset() -> vec2<f32> {
    let t = pick_query_params.transform;
    let cursor = pick_query_params.cursor_chart;
    let ndc = vec2<f32>(
        (cursor.x - cursor.z) / pick_query_params.chart_limits.x * 2.0 - 1.0,
        1.0 - (cursor.y - cursor.w) / pick_query_params.chart_limits.y * 2.0,
    );
    let center = (t.data_to_panel_offset + t.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    return (ndc - center) / t.pixel_to_ndc;
}

fn pick_polar_clip_limits() -> vec4<f32> {
    let p = pick_query_params.transform.polar_angle;
    let e = pick_query_params.transform.polar_extent;
    return vec4<f32>(p.x, e.w, e.w * p.w, select(0.0, 1.0, e.y > e.x));
}

fn pick_polar_clip_sector() -> vec4<f32> {
    let p = pick_query_params.transform.polar_angle;
    let e = pick_query_params.transform.polar_extent;
    return vec4<f32>(p.y, p.z, e.x, e.y);
}

// Distance from the ray at screen angle `angle` (from the polar centre) to
// `px`.
fn pick_polar_ray_distance(px: vec2<f32>, angle: f32) -> f32 {
    let dir = vec2<f32>(cos(angle), sin(angle));
    let along = dot(px, dir);
    if along <= 0.0 {
        return length(px);
    }
    return abs(px.x * dir.y - px.y * dir.x);
}

// Zero where the polar fragment clip keeps the fragment under the cursor,
// else a positive lower bound of the distance to the kept region: past the
// outer ring, inside the hole, or to the nearer sector edge. Every primitive
// a polar chart draws is cut by this clip, so a hit distance is never below
// it. Zero for a rectangular chart.
fn pick_polar_clip_distance() -> f32 {
    if !pick_polar() {
        return 0.0;
    }
    let px = pick_polar_cursor_offset();
    let limits = pick_polar_clip_limits();
    let sector = pick_polar_clip_sector();
    if polar_fragment_visible(px, limits, sector) {
        return 0.0;
    }
    let r = length(px);
    var d = max(r - (limits.y + 0.5), (limits.z - 0.5) - r);
    if limits.w >= 0.5 && !polar_fragment_visible(px, vec4<f32>(limits.x, PICK_F32_MAX, 0.0, limits.w), sector) {
        let start = sector.x + sector.y * sector.z;
        let end = sector.x + sector.y * sector.w;
        d = max(d, min(pick_polar_ray_distance(px, start), pick_polar_ray_distance(px, end)));
    }
    return max(d, PICK_F32_EPSILON);
}

fn pick_axis_cursor(axis: u32) -> f32 {
    return pick_query_params.cursor_chart[axis];
}

fn pick_conservative_gate_threshold(threshold: f32) -> f32 {
    // The exact pass remains authoritative. A tiny outward pad prevents an
    // axis gate from rejecting a boundary hit because sqrt/add/sub rounded in
    // a different direction.
    return threshold + max(abs(threshold), 1.0) * PICK_F32_EPSILON * 8.0;
}

// Polar projections are not separable per axis: the X gate passes every
// valid sample and the Y gate tests the full projection
// (`pick_polar_scatter_may_hit`); line segments go straight to the exact
// pass, whose arc pieces are the drawn path.
fn pick_scatter_projected_may_hit(projected: f32, axis: u32) -> bool {
    if pick_polar() {
        return true;
    }
    if !pick_is_finite(projected) {
        return false;
    }
    let threshold = pick_conservative_gate_threshold(
        pick_query_params.chart_limits.z + pick_query_params.chart_limits.w,
    );
    return abs(projected - pick_axis_cursor(axis)) <= threshold;
}

fn pick_polar_scatter_may_hit(point_index: u32, y: vec2<f32>) -> bool {
    let x = pick_read_pair(pick_query_params.data.y, point_index);
    if !pick_pair_is_valid(x) || !pick_polar_valid(x, y) {
        return false;
    }
    let projected = pick_project_pair_polar(x, y);
    if !pick_is_finite(projected.x) || !pick_is_finite(projected.y) {
        return false;
    }
    let threshold = pick_conservative_gate_threshold(
        pick_query_params.chart_limits.z + pick_query_params.chart_limits.w,
    );
    let delta = abs(projected - pick_query_params.cursor_chart.xy);
    return delta.x <= threshold && delta.y <= threshold;
}

fn pick_line_projected_may_hit(a_px: f32, b_px: f32, axis: u32) -> bool {
    if pick_polar() {
        return true;
    }
    if !pick_is_finite(a_px) || !pick_is_finite(b_px) {
        return false;
    }
    let threshold = pick_conservative_gate_threshold(
        pick_query_params.chart_limits.z + pick_query_params.line_px.z,
    );
    let cursor = pick_axis_cursor(axis);
    return cursor >= min(a_px, b_px) - threshold
        && cursor <= max(a_px, b_px) + threshold;
}

fn pick_linear_workgroup_id(workgroup_id: vec3<u32>) -> u32 {
    return workgroup_id.x + workgroup_id.y * pick_query_params.series.w;
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_gate_x(
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    let word_index =
        pick_linear_workgroup_id(workgroup_id) * PICK_WORKGROUP_SIZE + local_index;
    if word_index >= pick_query_params.series.x {
        return;
    }

    let flags = pick_query_params.style.x;
    let scatter_enabled = (flags & PICK_FLAG_SCATTER) != 0u;
    let line_enabled = (flags & PICK_FLAG_LINE) != 0u;
    let point_count = pick_query_params.data.x;
    let first = word_index * PICK_GATE_WORD_POINTS;
    var scatter_mask = 0u;
    var line_mask = 0u;
    var x = pick_read_pair(pick_query_params.data.y, first);
    for (var bit = 0u; bit < PICK_GATE_WORD_POINTS; bit = bit + 1u) {
        let point_index = first + bit;
        if point_index >= point_count {
            break;
        }
        let x_valid = pick_pair_is_valid(x);
        var x_px = 0.0;
        if x_valid {
            x_px = pick_project_axis_pair(x, 0u);
        }
        if scatter_enabled && point_index < pick_query_params.stream.y && x_valid && pick_scatter_projected_may_hit(x_px, 0u) {
            scatter_mask = scatter_mask | (1u << bit);
        }
        let has_next = point_index + 1u < point_count;
        let load_next = has_next && (line_enabled || bit + 1u < PICK_GATE_WORD_POINTS);
        if load_next {
            let next_x = pick_read_pair(pick_query_params.data.y, point_index + 1u);
            if line_enabled
                && point_index < pick_query_params.stream.z
                && x_valid
                && pick_pair_is_valid(next_x)
                && pick_line_projected_may_hit(
                    x_px,
                    pick_project_axis_pair(next_x, 0u),
                    0u,
                )
            {
                line_mask = line_mask | (1u << bit);
            }
            x = next_x;
        }
    }
    pick_gate_masks[word_index] = vec2<u32>(scatter_mask, line_mask);
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_gate_y(
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    let word_index =
        pick_linear_workgroup_id(workgroup_id) * PICK_WORKGROUP_SIZE + local_index;
    if word_index >= pick_query_params.series.x {
        return;
    }

    let point_count = pick_query_params.data.x;
    let first = word_index * PICK_GATE_WORD_POINTS;
    let input = pick_gate_masks[word_index];
    var scatter_mask = input.x;
    var line_mask = input.y;
    var cached_y = vec2<f32>(0.0);
    var cached_index = 0u;
    var has_cached_y = false;
    for (var bit = 0u; bit < PICK_GATE_WORD_POINTS; bit = bit + 1u) {
        let bit_mask = 1u << bit;
        if ((scatter_mask | line_mask) & bit_mask) == 0u {
            continue;
        }
        let point_index = first + bit;
        var y = vec2<f32>(0.0);
        if has_cached_y && cached_index == point_index {
            y = cached_y;
        } else {
            y = pick_read_pair(pick_query_params.data.z, point_index);
        }
        let y_valid = pick_pair_is_valid(y);
        var y_px = 0.0;
        if y_valid {
            y_px = pick_project_axis_pair(y, 1u);
        }
        if (scatter_mask & bit_mask) != 0u {
            if !y_valid
                || !pick_scatter_projected_may_hit(y_px, 1u)
                || (pick_polar() && !pick_polar_scatter_may_hit(point_index, y))
            {
                scatter_mask = scatter_mask & ~bit_mask;
            }
        }
        if (line_mask & bit_mask) != 0u {
            if point_index + 1u >= point_count {
                line_mask = line_mask & ~bit_mask;
            } else {
                let next_y = pick_read_pair(pick_query_params.data.z, point_index + 1u);
                cached_y = next_y;
                cached_index = point_index + 1u;
                has_cached_y = true;
                if !y_valid
                    || !pick_pair_is_valid(next_y)
                    || !pick_line_projected_may_hit(
                        y_px,
                        pick_project_axis_pair(next_y, 1u),
                        1u,
                    )
                {
                    line_mask = line_mask & ~bit_mask;
                }
            }
        } else {
            has_cached_y = false;
        }
    }
    pick_gate_masks[word_index] = vec2<u32>(scatter_mask, line_mask);
}

// ───── shared marker shape (SHADER_COMMON.md §5.5; `shader_consistency`
// compares these function bodies and constants with the scatter shader) ─────

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

fn pick_nonnegative(v: f32) -> f32 {
    return select(0.0, v, v == v && v > 0.0);
}

fn pick_valid_style_index(v: f32) -> bool {
    return pick_is_finite(v) && v >= 0.0 && v <= 16777216.0 && abs(v - round(v)) <= 0.001;
}

fn pick_apply_style(radius_in: f32, shape_in: u32, slot: PickScatterStyleSlot) -> vec2<f32> {
    let mask = u32(slot.params.z);
    var radius = radius_in;
    var shape = shape_in;
    if (mask & PICK_STYLE_MASK_RADIUS) != 0u {
        radius = pick_nonnegative(slot.params.x);
    }
    if (mask & PICK_STYLE_MASK_SHAPE) != 0u {
        shape = u32(pick_nonnegative(slot.params.y));
    }
    return vec2<f32>(radius, f32(shape));
}

// ───── shared size-map radius (SHADER_COMMON.md §5.4; `shader_consistency`
// compares these function bodies with the scatter shader) ─────

/// A finite f32 by its exponent bits. Every ramp input is checked
/// explicitly: NaN and infinity propagation through arithmetic is not
/// portable in WGSL.
fn ramp_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
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

// The marker radius a size value maps to, or a negative number when the value
// is not finite and the point is hidden.
fn pick_size_map_radius(point_index: u32) -> f32 {
    if point_index >= pick_query_params.size_map.y {
        return -1.0;
    }
    let value = pick_read_pair(pick_query_params.size_map.x, point_index);
    let domain = pick_query_params.size_domain;
    return size_map_radius(
        value,
        domain.xy,
        domain.zw,
        pick_query_params.size_px.x,
        pick_query_params.size_px.y,
        (pick_query_params.style.x & PICK_FLAG_SIZE_AREA) != 0u,
    );
}

// One marker as the scatter shader resolves it before drawing: the radius
// in canvas pixels (the display scale applied, like `point_radius_px`), the
// shape code, and whether the size map hides the point.
struct PickMarker {
    radius: f32,
    shape: u32,
    hidden: bool,
};

// Precedence per channel, as in `resolve_mapped_style`: sparse override >
// size map > style table slot > series base. A hidden point stays hidden
// unless an override sets its radius.
fn pick_resolve_scatter_marker(point_index: u32) -> PickMarker {
    var radius = pick_nonnegative(pick_query_params.scatter_line.x);
    var shape = pick_query_params.series.z;
    let flags = pick_query_params.style.x;
    var hidden = false;
    if (flags & PICK_FLAG_STYLE_MAP) != 0u {
        if (flags & PICK_FLAG_STYLE_INDEX) != 0u && point_index < pick_query_params.style.w {
            let index_word = pick_query_params.data.w + point_index * 2u;
            let style_index = bitcast<f32>(pick_pool_words[index_word]);
            if pick_valid_style_index(style_index) {
                let slot_index = u32(round(style_index));
                if slot_index < pick_query_params.style.y {
                    let resolved = pick_apply_style(radius, shape, pick_style_slots[slot_index]);
                    radius = resolved.x;
                    shape = u32(resolved.y);
                }
            }
        }
    }
    if (flags & PICK_FLAG_SIZE_MAP) != 0u {
        let mapped = pick_size_map_radius(point_index);
        if mapped < 0.0 {
            hidden = true;
        } else {
            radius = mapped;
        }
    }
    if (flags & PICK_FLAG_STYLE_MAP) != 0u {
        for (var i = 0u; i < pick_query_params.style.z; i = i + 1u) {
            let override_row = pick_style_overrides[i];
            if override_row.point_index == point_index + pick_query_params.stream.x {
                let slot = PickScatterStyleSlot(override_row.color_premul, override_row.params);
                let resolved = pick_apply_style(radius, shape, slot);
                if (u32(slot.params.z) & PICK_STYLE_MASK_RADIUS) != 0u {
                    hidden = false;
                }
                radius = resolved.x;
                shape = u32(resolved.y);
            }
        }
    }
    return PickMarker(radius * pick_query_params.scatter_line.z, shape, hidden);
}

// What a marker series' pipeline draws under the chart's draw style
// (`PickQueryParams.styled.x`).
const PICK_MARKER_PRECISE: u32 = 0u;
// `vs_sketch` / `fs_sketch`: the shape with a hashed contour wobble.
const PICK_MARKER_SKETCH: u32 = 1u;
// Milkyway `vs_planet` / `fs_planet`: a disc with a rim glow and a ring.
const PICK_MARKER_PLANET: u32 = 2u;
// Constellation `vs_constellation_star` / `fs_constellation_star`.
const PICK_MARKER_STAR: u32 = 3u;

// The global point index the styled entries hash (`styled_point_index`).
fn pick_styled_index(point_index: u32) -> u32 {
    return point_index + pick_query_params.stream.x;
}

// ───── shared styled markers (SHADER_COMMON.md §5.7; `shader_consistency`
// compares these function bodies and constants with the scatter shader) ─────

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

// A Milkyway planet's hit distance at `p_screen` (pixels from its centre, y
// up): `fs_planet`'s disc, the rim glow where it stays above
// `PLANET_ALPHA_FLOOR`, and the ring band between `RING_INNER` and
// `RING_OUTER`, all cut by the quad `vs_planet` emits. The band is the
// envelope of the baked ring texture, whose density is not a shader
// constant. `vs_planet` sizes the quad from the radius while `fs_planet`
// reconstructs positions from `max(radius, 1)`, so the fragment frame is
// scaled by their ratio.
fn pick_planet_distance(marker: PickMarker, p_screen: vec2<f32>, point_index: u32) -> f32 {
    let style_params = pick_query_params.transform.style_params;
    let r = marker.radius;
    let r_planet = max(r, 1.0);
    let quad = r * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX;
    let outside_quad = max(abs(p_screen.x), abs(p_screen.y)) - quad;
    let p = p_screen * ((r_planet * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX) / quad);
    let dist = length(p);
    let rim_gain = clamp(style_params[1].w, 0.0, 2.0);
    var d = dist - r_planet;
    if planet_rim_glow(dist, r_planet, rim_gain) > PLANET_ALPHA_FLOOR {
        d = min(d, 0.0);
    } else if rim_gain > PLANET_ALPHA_FLOOR {
        let reach = max(r_planet * 0.10, 1.0) * log(rim_gain / PLANET_ALPHA_FLOOR);
        d = min(d, dist - r_planet - reach);
    }
    let seed_inst = (u32(style_params[0].w) ^ pick_query_params.styled.z) + pick_styled_index(point_index);
    let q = planet_ring_frame(p, planet_ring_phi(pick_query_params.series.z, seed_inst));
    let rho = planet_ring_rho(q, r_planet);
    // A band distance in ring-plane radii is at least that many minor-axis
    // pixels away.
    d = min(d, max(RING_INNER - rho, rho - RING_OUTER) * r_planet * RING_INCL);
    return max(max(d, outside_quad), 0.0);
}

// The hit distance of a marker drawn at `center_px` for the cursor: the
// scatter shader's own coverage rule (`shape_distance` and `marker_edge`,
// SHADER_COMMON.md §5.5, with the sketch wobble of §5.7), evaluated where
// the fragment at the cursor would evaluate it. The marker's local frame is
// y-up like NDC while canvas pixels run down, hence the flip. Zero exactly
// where the fragment's coverage is at least one half; the anti-aliasing
// fringe beyond it is a sub-pixel positive distance. A Milkyway planet and
// a Constellation star pick on their drawn extent instead.
fn pick_marker_distance(marker: PickMarker, center_px: vec2<f32>, point_index: u32) -> f32 {
    let delta = pick_query_params.cursor_chart.xy - center_px;
    let p = vec2<f32>(delta.x, -delta.y);
    let mode = pick_query_params.styled.x;
    let style_params = pick_query_params.transform.style_params;
    if mode == PICK_MARKER_PLANET {
        return pick_planet_distance(marker, p, point_index);
    }
    if mode == PICK_MARKER_STAR {
        // A zero star opacity draws nothing. Otherwise the PSF sprite fades
        // to zero inside the disc its quad inscribes; the star's brightness
        // and tint are hashed per point and sampled from baked textures, so
        // that disc is the drawn envelope.
        if !(style_params[0].x > 0.0) {
            return PICK_F32_MAX;
        }
        let star = constellation_star_radius(marker.radius, pick_styled_index(point_index), pick_query_params.styled.z);
        return max(length(p) - (star * 4.0 + QUAD_MARGIN_PX), 0.0);
    }
    let filled = shape_is_filled(marker.shape);
    let base = base_shape_id(marker.shape);
    let visual_r = visual_shape_radius(base, marker.radius);
    var d: f32;
    if mode == PICK_MARKER_SKETCH {
        let wobble = sketch_marker_wobble_px(style_params[0].x, marker.radius);
        let seed = (u32(style_params[0].z) ^ pick_query_params.styled.z) + pick_styled_index(point_index);
        d = sketch_marker_distance(base, p, visual_r, filled, wobble, seed);
    } else {
        d = shape_distance(base, p, visual_r, filled);
    }
    return max(marker_edge(d, filled), 0.0);
}

fn pick_invalid_candidate(series_order: u32) -> PickCandidate {
    return PickCandidate(
        0u,
        series_order,
        0u,
        0u,
        0u,
        PICK_F32_MAX,
        PICK_F32_MAX,
        0u,
    );
}

fn pick_candidate_is_better(candidate: PickCandidate, incumbent: PickCandidate) -> bool {
    if candidate.valid == 0u {
        return false;
    }
    if incumbent.valid == 0u {
        return true;
    }
    if candidate.distance_sq != incumbent.distance_sq {
        return candidate.distance_sq < incumbent.distance_sq;
    }
    // An exact tie goes to whatever is painted on top. Series paint in
    // order, so a later series covers an earlier one; within a series the
    // markers paint over the line; and instances paint in index order, so a
    // higher marker or segment index covers a lower one.
    if candidate.series_order != incumbent.series_order {
        return candidate.series_order > incumbent.series_order;
    }
    if candidate.primitive_kind != incumbent.primitive_kind {
        return candidate.primitive_kind < incumbent.primitive_kind;
    }
    return candidate.primitive_index > incumbent.primitive_index;
}

// ───── shared arrow glyph (SHADER_COMMON.md §5.8; `shader_consistency`
// compares these function bodies, structs and constants with the vector
// shader's glyph and projection fragments) ─────

const ARROW_PIVOT_TAIL: u32 = 0u;
const ARROW_PIVOT_MIDDLE: u32 = 1u;
const ARROW_PIVOT_TIP: u32 = 2u;
// A shaft shorter than this (pixels) is no shaft. The head takes the whole
// length of an arrow shorter than the head; the fragment stage recomputes
// that length from the interpolated tail and tip, and the rounding must not
// turn the missing shaft into a round dot at the tail.
const ARROW_SHAFT_MIN_PX: f32 = 0.01;

/// Pixel-space arrow style. Sizes are already multiplied by the render scale.
struct ArrowStyle {
    width_px: f32,
    head_length_px: f32,
    head_width_px: f32,
    min_length_px: f32,
    pivot: u32,
};

/// An arrow placed on its anchor: tail and tip after the pivot shift, and the
/// head dimensions after scaling down for a short arrow.
struct ArrowPlacement {
    tail: vec2<f32>,
    tip: vec2<f32>,
    head_length_px: f32,
    head_width_px: f32,
    valid: bool,
};

fn arrow_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

/// Apply the pivot and the minimum length, and size the head to the arrow.
/// A zero-length arrow, one shorter than `min_length_px`, or one with a
/// non-finite end is not drawn.
fn arrow_place(tail: vec2<f32>, tip: vec2<f32>, style: ArrowStyle) -> ArrowPlacement {
    var out: ArrowPlacement;
    out.tail = tail;
    out.tip = tip;
    out.head_length_px = 0.0;
    out.head_width_px = 0.0;
    out.valid = false;
    let delta = tip - tail;
    let len = length(delta);
    if (!arrow_finite(len) || len <= 0.0 || len < style.min_length_px) {
        return out;
    }
    var shift = 0.0;
    if (style.pivot == ARROW_PIVOT_MIDDLE) {
        shift = -0.5;
    } else if (style.pivot == ARROW_PIVOT_TIP) {
        shift = -1.0;
    }
    out.tail = tail + delta * shift;
    out.tip = tip + delta * shift;
    let head_length = min(max(style.head_length_px, 0.0), len);
    out.head_length_px = head_length;
    if (style.head_length_px > 0.0) {
        out.head_width_px = max(style.head_width_px, 0.0) * (head_length / style.head_length_px);
    }
    out.valid = true;
    return out;
}

/// Unsigned distance from `p` to the segment `ab`; a zero-length segment is a
/// point.
fn arrow_sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    return length(pa - ba * h);
}

/// Signed distance from `p` to the triangle `p0 p1 p2` (negative inside).
fn arrow_sd_triangle(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    let e0 = p1 - p0;
    let e1 = p2 - p1;
    let e2 = p0 - p2;
    let v0 = p - p0;
    let v1 = p - p1;
    let v2 = p - p2;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / max(dot(e0, e0), 1e-12), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / max(dot(e1, e1), 1e-12), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / max(dot(e2, e2), 1e-12), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d = min(
        min(
            vec2<f32>(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x)),
            vec2<f32>(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x)),
        ),
        vec2<f32>(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x)),
    );
    return -sqrt(d.x) * sign(d.y);
}

/// Signed distance from pixel `p` to the placed arrow: the union of the
/// shaft (a segment `width_px` wide) and the head (a triangle). Negative
/// inside; `1e20` for an arrow with neither.
fn arrow_distance(p: vec2<f32>, placed: ArrowPlacement, width_px: f32) -> f32 {
    let delta = placed.tip - placed.tail;
    let len = length(delta);
    let dir = delta / max(len, 1e-12);
    let nrm = vec2<f32>(-dir.y, dir.x);
    let head_base = placed.tip - dir * placed.head_length_px;
    var distance = 1e20;
    // The shaft stops where the head begins. A shaft of zero length draws
    // nothing rather than a dot.
    if (len - placed.head_length_px > ARROW_SHAFT_MIN_PX && width_px > 0.0) {
        distance = min(distance, arrow_sd_segment(p, placed.tail, head_base) - width_px * 0.5);
    }
    if (placed.head_length_px > 0.0 && placed.head_width_px > 0.0) {
        let half = nrm * (placed.head_width_px * 0.5);
        distance = min(distance, arrow_sd_triangle(p, placed.tip, head_base + half, head_base - half));
    }
    return distance;
}

/// `pair + s` as a `(hi, lo)` pair: the error-free sum keeps the low part, so a
/// large-offset coordinate still resolves a small data-space step.
fn pair_add(pair: vec2<f32>, s: f32) -> vec2<f32> {
    let sum = fma(1.0, pair.x, s);
    let b_virtual = fma(1.0, sum, -pair.x);
    let a_virtual = fma(1.0, sum, -b_virtual);
    let b_roundoff = fma(1.0, s, -b_virtual);
    let a_roundoff = fma(1.0, pair.x, -a_virtual);
    return vec2<f32>(sum, fma(1.0, a_roundoff, b_roundoff) + pair.y);
}

/// Pixels per data unit along one axis whose range is `range` (the hi/lo
/// sum), at `pair`: the derivative of `axis_pair_to_t` mapped through NDC.
/// Exact on a linear axis; the local direction on a logarithmic one, the way
/// the contour labels orient themselves.
fn axis_px_per_unit_at(range: f32, panel_scale: f32, is_log: f32, pixel_to_ndc: f32, pair: vec2<f32>) -> f32 {
    let linear = 2.0 * panel_scale / (range * pixel_to_ndc);
    // d(log10 v)/dv = 1 / (v ln 10); `sample_valid` has already excluded v <= 0.
    let value = pair.x + pair.y;
    let log_slope = linear / (value * log(10.0));
    return mix(linear, log_slope, is_log);
}

// ───── end shared arrow glyph ─────

// `axis_px_per_unit` of the vector shader's projection fragment, on this
// shader's transform.
fn pick_axis_px_per_unit(axis: u32, pair: vec2<f32>) -> f32 {
    let t = pick_query_params.transform;
    let range = fma(1.0, t.data_max[axis] - t.data_min[axis], t.data_max_lo[axis] - t.data_min_lo[axis]);
    return axis_px_per_unit_at(range, t.data_to_panel_scale[axis], t.scale_log[axis], t.pixel_to_ndc[axis], pair);
}

// A point vector's candidate: the arrow the vector shader draws from anchor
// `(x, y)` with the sample's components — `vector_sample`, `project_arrow`
// and `arrow_place` in this shader's frame (canvas pixels with y flipped up,
// a translation of the draw's) — picked by `arrow_distance`, so the shaft
// and head pick where they are drawn, and an arrow that is not drawn (a
// non-finite or missing component, a zero vector, one shorter than the
// minimum length) does not pick at all.
fn pick_vector_candidate(point_index: u32, x: vec2<f32>, y: vec2<f32>) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    let vector = pick_query_params.vector;
    let px = pick_query_params.vector_px;
    if point_index >= vector.z {
        return pick_invalid_candidate(series_order);
    }
    let u = pick_read_pair(vector.x, point_index);
    let v = pick_read_pair(vector.y, point_index);
    if !pick_pair_is_valid(u) || !pick_pair_is_valid(v) {
        return pick_invalid_candidate(series_order);
    }
    if !pick_coordinate_is_valid(x, 0u) || !pick_coordinate_is_valid(y, 1u) {
        return pick_invalid_candidate(series_order);
    }
    let uu = u.x + u.y;
    let vv = v.x + v.y;
    let tail = pick_flip(pick_project_pair(x, y));
    var tip = tail;
    if (vector.w & PICK_VECTOR_SCALE_PIXELS) != 0u {
        let direction = vec2<f32>(pick_axis_px_per_unit(0u, x) * uu, pick_axis_px_per_unit(1u, y) * vv);
        let direction_len = length(direction);
        let len_px = px.x * length(vec2<f32>(uu, vv));
        if !pick_is_finite(direction_len) || !pick_is_finite(len_px) {
            return pick_invalid_candidate(series_order);
        }
        if direction_len > 0.0 && len_px != 0.0 {
            tip = tail + direction * (len_px / direction_len);
        }
    } else {
        let tx = pair_add(x, px.x * uu);
        let ty = pair_add(y, px.x * vv);
        if !pick_coordinate_is_valid(tx, 0u) || !pick_coordinate_is_valid(ty, 1u) {
            return pick_invalid_candidate(series_order);
        }
        tip = pick_flip(pick_project_pair(tx, ty));
    }
    if !pick_is_finite(tail.x) || !pick_is_finite(tail.y) || !pick_is_finite(tip.x) || !pick_is_finite(tip.y) {
        return pick_invalid_candidate(series_order);
    }
    let placed = arrow_place(tail, tip, ArrowStyle(px.y, px.z, px.w, pick_query_params.vector_min.x, vector.w & 255u));
    if !placed.valid {
        return pick_invalid_candidate(series_order);
    }
    let hit_distance = max(arrow_distance(pick_flip(pick_query_params.cursor_chart.xy), placed, px.y), 0.0);
    let distance_sq = hit_distance * hit_distance;
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    if !pick_is_finite(distance_sq) || distance_sq > maximum_sq {
        return pick_invalid_candidate(series_order);
    }
    return PickCandidate(
        1u,
        series_order,
        pick_source_row(point_index),
        0u,
        pick_source_row(point_index),
        distance_sq,
        hit_distance,
        0u,
    );
}

fn pick_scatter_candidate(
    point_index: u32,
    x: vec2<f32>,
    y: vec2<f32>,
) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    if (pick_query_params.style.x & PICK_FLAG_VECTOR) != 0u {
        return pick_vector_candidate(point_index, x, y);
    }
    // The vertex stage's `sample_valid`; the sector and the rings are the
    // fragment clip's, applied at the cursor below.
    if !pick_coordinate_is_valid(x, 0u) || !pick_coordinate_is_valid(y, 1u) || !pick_polar_valid(x, y) {
        return pick_invalid_candidate(series_order);
    }
    let marker = pick_resolve_scatter_marker(point_index);
    if marker.hidden || !pick_is_finite(marker.radius) {
        return pick_invalid_candidate(series_order);
    }
    let point_px = pick_project_pair(x, y);
    if !pick_is_finite(point_px.x) || !pick_is_finite(point_px.y) {
        return pick_invalid_candidate(series_order);
    }
    let hit_distance = max(pick_marker_distance(marker, point_px, point_index), pick_polar_clip_distance());
    let distance_sq = hit_distance * hit_distance;
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    if !pick_is_finite(distance_sq) || distance_sq > maximum_sq {
        return pick_invalid_candidate(series_order);
    }
    return PickCandidate(
        1u,
        series_order,
        pick_source_row(point_index),
        0u,
        pick_source_row(point_index),
        distance_sq,
        hit_distance,
        0u,
    );
}

// Data-space corners of step piece `k` of the segment a → b. Same rules as
// `step_piece` in line_columnar.wgsl (data-space midpoint for step-mid), in
// this shader's own pick_* ABI; the CPU twin is `line_step::step_pieces`.
struct PickStepPiece {
    x0: vec2<f32>,
    y0: vec2<f32>,
    x1: vec2<f32>,
    y1: vec2<f32>,
};

// The mode word: bits 0-1 the step mode, bits 4 and up the polar arc
// pieces per segment (`transform.polar_extent.z`), as `step_piece_count` in
// line_columnar.wgsl reads it.
const PICK_ARC_SHIFT: u32 = 4u;

fn pick_line_mode() -> u32 {
    let arc = u32(max(pick_query_params.transform.polar_extent.z, 0.0));
    return (pick_query_params.line.x & 3u) | (arc << PICK_ARC_SHIFT);
}

fn pick_step_piece_count(mode: u32) -> u32 {
    let arc = mode >> PICK_ARC_SHIFT;
    if arc > 0u {
        return arc;
    }
    let step = mode & 3u;
    if step == PICK_STEP_PRE || step == PICK_STEP_POST {
        return 2u;
    }
    if step == PICK_STEP_MID {
        return 3u;
    }
    return 1u;
}

fn pick_step_piece(ax: vec2<f32>, ay: vec2<f32>, bx: vec2<f32>, by: vec2<f32>, mode: u32, k: u32) -> PickStepPiece {
    var p: PickStepPiece;
    p.x0 = ax;
    p.y0 = ay;
    p.x1 = bx;
    p.y1 = by;
    let arc = mode >> PICK_ARC_SHIFT;
    if arc > 0u {
        let t0 = f32(k) / f32(arc);
        let t1 = f32(k + 1u) / f32(arc);
        p.x0 = mix(ax, bx, t0);
        p.y0 = mix(ay, by, t0);
        p.x1 = mix(ax, bx, t1);
        p.y1 = mix(ay, by, t1);
        return p;
    }
    let step = mode & 3u;
    if step == PICK_STEP_PRE {
        if k == 0u { p.x1 = ax; } else { p.y0 = by; }
    } else if step == PICK_STEP_POST {
        if k == 0u { p.y1 = ay; } else { p.x0 = bx; }
    } else if step == PICK_STEP_MID {
        let xm = (ax + bx) * 0.5;
        if k == 0u {
            p.x1 = xm;
            p.y1 = ay;
        } else if k == 1u {
            p.x0 = xm;
            p.x1 = xm;
        } else {
            p.x0 = xm;
            p.y0 = by;
        }
    }
    return p;
}

// ───── shared line stroke (SHADER_COMMON.md §5.6; `shader_consistency`
// compares these function bodies and constants with the line shader) ─────

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

// What a line series' pipeline draws under the chart's draw style
// (`PickQueryParams.styled.y`). Constellation lines are the precise quads.
const PICK_LINE_PRECISE: u32 = 0u;
// `vs_sketch`: a subdivided strip displaced by `sketch_line_offset`.
const PICK_LINE_SKETCH: u32 = 1u;
// Milkyway: the ribbon strip and the star field scattered along it.
const PICK_LINE_RIBBON: u32 = 2u;

// Canvas pixels (y down) into the draw shaders' pixel frame (y up).
fn pick_flip(v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(v.x, -v.y);
}

// The arc distances from `dist_px`, which lies in an "off" span of the dash
// pattern, back to the end of the previous "on" span (x) and forward to the
// start of the next one (y). The span walk is `dash_pattern_on`'s.
fn pick_dash_gap(dist_px: f32) -> vec2<f32> {
    var spans = pick_query_params.line_dash;
    let n = min(pick_query_params.styled.w, 8u);
    var period = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        period = period + max(spans[i / 4u][i % 4u], 0.0);
    }
    let phase = ((dist_px % period) + period) % period;
    var acc = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        let start = acc;
        let end = acc + max(spans[i / 4u][i % 4u], 0.0);
        acc = end;
        if phase <= end || i == n - 1u {
            return vec2<f32>(phase - start, end - phase);
        }
    }
    return vec2<f32>(0.0, 0.0);
}

// The hit distance of one drawn piece of a stroke: the line shader's own
// extrusion (square caps, `max(width, 1)`, the sketch displacement, the dash
// phase) inverted at the cursor. `a_up` / `b_up` are the piece's ends in the
// draw's y-up pixel frame and `arc_start` / `arc_end` the arc positions the
// vertex stage gives them (`has_arc` is false when the arc prefix is not
// bound: a streamed series). Zero exactly where `line_stroke_distance` is
// non-positive and the dash is on, which is where the fragment's coverage is
// at least one half.
fn pick_stroke_piece_distance(a_up: vec2<f32>, b_up: vec2<f32>, arc_start: f32, arc_end: f32, has_arc: bool) -> f32 {
    let delta = b_up - a_up;
    let len = length(delta);
    // The vertex stage collapses a zero-length piece to a point.
    if !(len > 1e-6) {
        return PICK_F32_MAX;
    }
    let dir = delta / len;
    let normal = vec2<f32>(-dir.y, dir.x);
    let rel = pick_flip(pick_query_params.cursor_chart.xy) - a_up;
    let along = dot(rel, dir);
    var side = dot(rel, normal);
    let mode = pick_query_params.styled.y;
    // A Constellation line drawn at zero opacity paints nothing (its
    // markers are stars, so the marker mode says which style this is).
    if pick_query_params.styled.x == PICK_MARKER_STAR && !(pick_query_params.transform.style_params[0].y > 0.0) {
        return PICK_F32_MAX;
    }
    if mode == PICK_LINE_RIBBON {
        // The ribbon strip spans one full ribbon width either side with
        // square caps of the same size; the stars scatter within the
        // envelope the CPU folds into the same half extent.
        return max(line_stroke_distance(along, side, len, pick_query_params.line_px.y), 0.0);
    }
    let half_w = pick_query_params.line_px.x;
    let dash_count = pick_query_params.styled.w;
    let geom_half_w = half_w + select(LINE_AA_EXTENT_PX, 0.0, dash_count > 0u);
    // The arc position at the cursor (the interpolated `dist_px`) and its
    // rate per pixel along the piece.
    var dist_px = 0.0;
    var dist_slope = 1.0;
    if mode == PICK_LINE_SKETCH {
        if has_arc {
            // The strip's knots k = 0..S sit at `t_k * len` along the piece,
            // the outer two pushed out by the square cap, each displaced
            // along the normal by the wobble at its arc position.
            let s = f32(SKETCH_SUBDIV);
            let k = u32(clamp(floor(along / len * s), 0.0, s - 1.0));
            let t0 = f32(k) / s;
            let t1 = f32(k + 1u) / s;
            let a0 = select(t0 * len, -geom_half_w, k == 0u);
            let a1 = select(t1 * len, len + geom_half_w, k + 1u == SKETCH_SUBDIV);
            let w = clamp((along - a0) / (a1 - a0), 0.0, 1.0);
            let style_params = pick_query_params.transform.style_params[0];
            let seed = u32(style_params.z) ^ pick_query_params.styled.z;
            let arc0 = mix(arc_start, arc_end, t0);
            let arc1 = mix(arc_start, arc_end, t1);
            let d0 = sketch_line_offset(arc0, style_params.x, style_params.y, seed);
            let d1 = sketch_line_offset(arc1, style_params.x, style_params.y, seed);
            side = side - mix(d0, d1, w);
            let dist0 = arc0 + select(0.0, -half_w, k == 0u);
            let dist1 = arc1 + select(0.0, half_w, k + 1u == SKETCH_SUBDIV);
            dist_px = mix(dist0, dist1, w);
            dist_slope = (dist1 - dist0) / (a1 - a0);
        } else {
            // Without the arc prefix the wobble is unknown: the envelope of
            // every displacement it can take.
            side = max(abs(side) - max(pick_query_params.transform.style_params[0].x, 0.0), 0.0);
        }
    } else {
        // `vs_main` puts `arc_start - half_w` on the A-side corners and
        // `arc_end + half_w` on the B-side ones.
        let span = len + 2.0 * geom_half_w;
        dist_slope = (arc_end - arc_start + 2.0 * half_w) / span;
        dist_px = (arc_start - half_w) + (along + geom_half_w) * dist_slope;
    }
    let stroke = max(line_stroke_distance(along, side, len, half_w), 0.0);
    if dash_count == 0u || !has_arc || dash_pattern_on(dist_px, pick_query_params.line_dash, dash_count) {
        return stroke;
    }
    // In a gap: the nearest dash of this piece on either side, along the
    // stroke, clipped to the stroke's rectangle.
    let gap = pick_dash_gap(dist_px) / max(abs(dist_slope), 1e-6);
    let across = max(abs(side) - half_w, 0.0);
    var best = PICK_F32_MAX;
    let before = along - gap.x;
    if before >= -half_w {
        let dx = along - min(before, len + half_w);
        best = min(best, sqrt(dx * dx + across * across));
    }
    let after = along + gap.y;
    if after <= len + half_w {
        let dx = max(after, -half_w) - along;
        best = min(best, sqrt(dx * dx + across * across));
    }
    return max(best, PICK_F32_EPSILON);
}

fn pick_line_candidate(
    segment_index: u32,
    ax: vec2<f32>,
    ay: vec2<f32>,
    bx: vec2<f32>,
    by: vec2<f32>,
) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    if !pick_coordinate_is_valid(ax, 0u) || !pick_coordinate_is_valid(ay, 1u) || !pick_coordinate_is_valid(bx, 0u) || !pick_coordinate_is_valid(by, 1u) {
        return pick_invalid_candidate(series_order);
    }
    if !pick_polar_valid(ax, ay) || !pick_polar_valid(bx, by) {
        return pick_invalid_candidate(series_order);
    }
    // A step mode tests every axis-aligned piece of the segment and keeps the
    // nearest (lowest piece index on a tie); a linear segment is its single
    // piece and reproduces the straight-segment rule exactly.
    let mode = pick_line_mode();
    let pieces = pick_step_piece_count(mode);
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    // The cumulative arc prefix the line was drawn with, when bound: the
    // dash phase and the sketch wobble walk it.
    let has_arc = pick_query_params.line.y != 0u;
    var arc_a = 0.0;
    var arc_b = 0.0;
    if has_arc {
        arc_a = pick_arc[segment_index];
        arc_b = pick_arc[segment_index + 1u];
    }
    var coordinate_scale = 1.0;
    var total_length = 0.0;
    var prefix_length = 0.0;
    var best_distance = PICK_F32_MAX;
    var best_found = false;
    var best_t = 0.0;
    var best_prefix = 0.0;
    var best_length = 0.0;
    for (var k = 0u; k < pieces; k = k + 1u) {
        let piece = pick_step_piece(ax, ay, bx, by, mode, k);
        let a_px = pick_project_pair(piece.x0, piece.y0);
        let b_px = pick_project_pair(piece.x1, piece.y1);
        if !pick_is_finite(a_px.x) || !pick_is_finite(a_px.y) || !pick_is_finite(b_px.x) || !pick_is_finite(b_px.y) {
            return pick_invalid_candidate(series_order);
        }
        coordinate_scale = max(coordinate_scale, max(max(abs(a_px.x), abs(a_px.y)), max(abs(b_px.x), abs(b_px.y))));
        let segment = b_px - a_px;
        let length_sq = fma(segment.x, segment.x, segment.y * segment.y);
        if !pick_is_finite(length_sq) {
            return pick_invalid_candidate(series_order);
        }
        let length = sqrt(length_sq);
        // The draw's arc positions for this piece: it starts at the
        // segment's arc plus the pieces before it and the last piece ends at
        // the scanned prefix of B.
        let piece_prefix = prefix_length;
        let arc_start = arc_a + piece_prefix;
        let arc_end = select(arc_start + length, arc_b, k + 1u == pieces);
        prefix_length = prefix_length + length;
        total_length = total_length + length;
        // A zero-length piece (a riser between equal values, or a degenerate
        // linear segment) has no stroke to hit.
        if !(length > 1e-6) {
            continue;
        }
        let distance = pick_stroke_piece_distance(pick_flip(a_px), pick_flip(b_px), arc_start, arc_end, has_arc);
        if pick_is_finite(distance) && distance < best_distance {
            let cursor_delta = pick_query_params.cursor_chart.xy - a_px;
            best_distance = distance;
            best_found = true;
            best_t = clamp((cursor_delta.x * segment.x + cursor_delta.y * segment.y) / length_sq, 0.0, 1.0);
            best_prefix = piece_prefix;
            best_length = length;
        }
    }
    if !best_found {
        return pick_invalid_candidate(series_order);
    }
    // The polar fragment clip cuts the stroke like it cuts a marker.
    let distance = max(best_distance, pick_polar_clip_distance());
    let distance_sq = distance * distance;
    if !pick_is_finite(distance_sq) || distance_sq > maximum_sq {
        return pick_invalid_candidate(series_order);
    }
    // Where along the whole stepped path the hit lies, 0 at A and 1 at B. A
    // linear segment keeps t itself: `(t · len) / len` is not bit-exact.
    var position = best_t;
    if pieces > 1u {
        position = (best_prefix + best_t * best_length) / total_length;
    }
    // Projected endpoints are f32. Reciprocal/fused evaluation can put an
    // exact midpoint a rounding step on either side of 0.5 across backends.
    // Resolve that uncertainty by the same lower-index rule as the CPU picker.
    let midpoint_tolerance = min(PICK_F32_EPSILON * coordinate_scale / total_length, 0.25);
    let use_b = position > 0.5 + midpoint_tolerance;
    let point_index = select(segment_index, segment_index + 1u, use_b);
    return PickCandidate(
        1u,
        series_order,
        pick_source_row(point_index),
        1u,
        pick_source_row(segment_index),
        distance_sq,
        distance,
        0u,
    );
}

fn pick_direct_masks(word_index: u32) -> vec2<u32> {
    let flags = pick_query_params.style.x;
    let point_count = pick_query_params.data.x;
    let first = word_index * PICK_GATE_WORD_POINTS;
    var scatter_mask = 0u;
    var line_mask = 0u;
    for (var bit = 0u; bit < PICK_GATE_WORD_POINTS; bit = bit + 1u) {
        let point_index = first + bit;
        if point_index >= point_count {
            break;
        }
        if (flags & PICK_FLAG_SCATTER) != 0u && point_index < pick_query_params.stream.y {
            scatter_mask = scatter_mask | (1u << bit);
        }
        if (flags & PICK_FLAG_LINE) != 0u && point_index < pick_query_params.stream.z && point_index + 1u < point_count {
            line_mask = line_mask | (1u << bit);
        }
    }
    return vec2<u32>(scatter_mask, line_mask);
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_exact_candidates(
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    figgy_zero_workgroup(local_index);
    let group_index = pick_linear_workgroup_id(workgroup_id);
    let word_index = group_index * PICK_WORKGROUP_SIZE + local_index;
    let series_order = pick_query_params.series.y;
    var local_best = pick_invalid_candidate(series_order);

    if word_index < pick_query_params.series.x {
        var masks = vec2<u32>(0u);
        if pick_query_params.scatter_line.w >= 0.5 {
            masks = pick_direct_masks(word_index);
        } else {
            masks = pick_gate_masks[word_index];
        }
        let first = word_index * PICK_GATE_WORD_POINTS;
        var cached_x = vec2<f32>(0.0);
        var cached_y = vec2<f32>(0.0);
        var cached_index = 0u;
        var has_cached_point = false;
        for (var bit = 0u; bit < PICK_GATE_WORD_POINTS; bit = bit + 1u) {
            let bit_mask = 1u << bit;
            if ((masks.x | masks.y) & bit_mask) == 0u {
                continue;
            }
            let point_index = first + bit;
            var x = vec2<f32>(0.0);
            var y = vec2<f32>(0.0);
            if has_cached_point && cached_index == point_index {
                x = cached_x;
                y = cached_y;
            } else {
                x = pick_read_pair(pick_query_params.data.y, point_index);
                y = pick_read_pair(pick_query_params.data.z, point_index);
            }
            if (masks.x & bit_mask) != 0u {
                let candidate = pick_scatter_candidate(point_index, x, y);
                if pick_candidate_is_better(candidate, local_best) {
                    local_best = candidate;
                }
            }
            if (masks.y & bit_mask) != 0u {
                let next_x =
                    pick_read_pair(pick_query_params.data.y, point_index + 1u);
                let next_y =
                    pick_read_pair(pick_query_params.data.z, point_index + 1u);
                let candidate =
                    pick_line_candidate(point_index, x, y, next_x, next_y);
                if pick_candidate_is_better(candidate, local_best) {
                    local_best = candidate;
                }
                cached_x = next_x;
                cached_y = next_y;
                cached_index = point_index + 1u;
                has_cached_point = true;
            } else {
                has_cached_point = false;
            }
        }
    }

    pick_shared_candidates[local_index] = local_best;
    workgroupBarrier();
    for (var stride = PICK_WORKGROUP_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if local_index < stride {
            let candidate = pick_shared_candidates[local_index + stride];
            if pick_candidate_is_better(candidate, pick_shared_candidates[local_index]) {
                pick_shared_candidates[local_index] = candidate;
            }
        }
        workgroupBarrier();
    }
    if local_index == 0u && group_index < arrayLength(&pick_workgroup_candidates) {
        pick_workgroup_candidates[group_index] = pick_shared_candidates[0];
    }
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_reduce_candidates(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = pick_invalid_candidate(0u);
    let count = arrayLength(&pick_reduce_inputs);
    var index = local_index;
    while index < count {
        let candidate = pick_reduce_inputs[index];
        if pick_candidate_is_better(candidate, best) {
            best = candidate;
        }
        index = index + PICK_WORKGROUP_SIZE;
    }
    pick_shared_candidates[local_index] = best;
    workgroupBarrier();
    for (var stride = PICK_WORKGROUP_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if local_index < stride {
            let candidate = pick_shared_candidates[local_index + stride];
            if pick_candidate_is_better(candidate, pick_shared_candidates[local_index]) {
                pick_shared_candidates[local_index] = candidate;
            }
        }
        workgroupBarrier();
    }
    if local_index == 0u {
        pick_reduce_output = pick_shared_candidates[0];
    }
}
