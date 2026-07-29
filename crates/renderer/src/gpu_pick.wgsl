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

const PICK_STYLE_MASK_RADIUS: u32 = 2u;
const PICK_STYLE_MASK_SHAPE: u32 = 4u;

struct PickQueryTransform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    style_params: array<vec4<f32>, 3>,
};

struct PickQueryParams {
    transform: PickQueryTransform,
    // (cursor_x, cursor_y, chart_x, chart_y), all canvas pixels.
    cursor_chart: vec4<f32>,
    // (chart_width, chart_height, maximum pick distance, maximum scatter
    // center extent). Exact radii/widths are resolved after both gates.
    chart_limits: vec4<f32>,
    // (base scatter radius, line half width, display scale, direct-scan flag).
    scatter_line: vec4<f32>,
    // (point count, x f32-lane base, y f32-lane base, style-index lane base).
    data: vec4<u32>,
    // (flags, style count, override count, style-index logical length).
    style: vec4<u32>,
    // (gate-word count, series order, base shape id, dispatch X width).
    series: vec4<u32>,
};

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

var<workgroup> pick_shared_candidates: array<PickCandidate, PICK_WORKGROUP_SIZE>;

fn pick_is_finite(v: f32) -> bool {
    return v == v && abs(v) <= PICK_F32_MAX;
}

fn pick_read_pair(base: u32, point_index: u32) -> vec2<f32> {
    let word = base + point_index * 2u;
    return vec2<f32>(bitcast<f32>(pick_pool_words[word]), bitcast<f32>(pick_pool_words[word + 1u]));
}

fn pick_pair_is_valid(v: vec2<f32>) -> bool {
    return pick_is_finite(v.x) && pick_is_finite(v.y) && pick_is_finite(v.x + v.y);
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
    let linear_num = (v.x - min_hi) + (v.y - min_lo);
    let range = (max_hi - min_hi) + (max_lo - min_lo);
    let log_num = (pick_log10(raw) - min_hi) - min_lo;
    return mix(linear_num / range, log_num / range, pick_query_params.transform.scale_log[axis]);
}

fn pick_project_axis_pair(v: vec2<f32>, axis: u32) -> f32 {
    let t = pick_axis_pair_to_t(v, axis);
    if axis == 0u {
        return pick_query_params.cursor_chart.z + t * pick_query_params.chart_limits.x;
    }
    return pick_query_params.cursor_chart.w + (1.0 - t) * pick_query_params.chart_limits.y;
}

fn pick_project_pair(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(pick_project_axis_pair(x, 0u), pick_project_axis_pair(y, 1u));
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

fn pick_scatter_projected_may_hit(projected: f32, axis: u32) -> bool {
    if !pick_is_finite(projected) {
        return false;
    }
    let threshold = pick_conservative_gate_threshold(
        pick_query_params.chart_limits.z
        + pick_query_params.chart_limits.w * pick_query_params.scatter_line.z,
    );
    return abs(projected - pick_axis_cursor(axis)) <= threshold;
}

fn pick_line_projected_may_hit(a_px: f32, b_px: f32, axis: u32) -> bool {
    if !pick_is_finite(a_px) || !pick_is_finite(b_px) {
        return false;
    }
    let threshold = pick_conservative_gate_threshold(
        pick_query_params.chart_limits.z
        + pick_query_params.scatter_line.y * pick_query_params.scatter_line.z,
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
        if scatter_enabled && x_valid && pick_scatter_projected_may_hit(x_px, 0u) {
            scatter_mask = scatter_mask | (1u << bit);
        }
        let has_next = point_index + 1u < point_count;
        let load_next = has_next && (line_enabled || bit + 1u < PICK_GATE_WORD_POINTS);
        if load_next {
            let next_x = pick_read_pair(pick_query_params.data.y, point_index + 1u);
            if line_enabled
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
            if !y_valid || !pick_scatter_projected_may_hit(y_px, 1u) {
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

fn pick_base_shape_id(shape_id: u32) -> u32 {
    switch shape_id {
        case 5u: { return 0u; }
        case 6u: { return 1u; }
        case 7u: { return 2u; }
        case 8u: { return 3u; }
        case 9u: { return 5u; }
        case 10u: { return 6u; }
        case 11u: { return 7u; }
        case 12u: { return 8u; }
        case 13u: { return 9u; }
        case 14u: { return 10u; }
        case 15u: { return 11u; }
        case 16u: { return 12u; }
        case 17u: { return 5u; }
        case 18u: { return 6u; }
        case 19u: { return 7u; }
        case 20u: { return 8u; }
        case 21u: { return 4u; }
        case 22u: { return 9u; }
        case 23u: { return 10u; }
        case 24u: { return 11u; }
        case 25u: { return 12u; }
        default: { return shape_id; }
    }
}

fn pick_visual_shape_radius(shape_id: u32, radius: f32) -> f32 {
    let base = pick_base_shape_id(shape_id);
    var scale = 1.0;
    switch base {
        case 1u: { scale = 0.88622695; }
        case 2u, 5u, 6u, 7u: { scale = 1.55512030; }
        case 3u: { scale = 1.25331414; }
        case 9u: { scale = 1.14913986; }
        case 10u: { scale = 1.09963611; }
        case 11u: { scale = 1.05390737; }
        case 12u: { scale = 1.46285033; }
        default: {}
    }
    return radius * scale;
}

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

fn pick_resolve_scatter_radius(point_index: u32) -> f32 {
    var radius = pick_nonnegative(pick_query_params.scatter_line.x);
    var shape = pick_query_params.series.z;
    let flags = pick_query_params.style.x;
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
        for (var i = 0u; i < pick_query_params.style.z; i = i + 1u) {
            let override_row = pick_style_overrides[i];
            if override_row.point_index == point_index {
                let slot = PickScatterStyleSlot(override_row.color_premul, override_row.params);
                let resolved = pick_apply_style(radius, shape, slot);
                radius = resolved.x;
                shape = u32(resolved.y);
            }
        }
    }
    return pick_visual_shape_radius(shape, radius) * pick_query_params.scatter_line.z;
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
    // Current production picker replaces equal-distance candidates from a
    // later series, but retains the first primitive within one series.
    if candidate.series_order != incumbent.series_order {
        return candidate.series_order > incumbent.series_order;
    }
    if candidate.primitive_kind != incumbent.primitive_kind {
        return candidate.primitive_kind < incumbent.primitive_kind;
    }
    return candidate.primitive_index < incumbent.primitive_index;
}

fn pick_scatter_candidate(
    point_index: u32,
    x: vec2<f32>,
    y: vec2<f32>,
) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    if !pick_pair_is_valid(x) || !pick_pair_is_valid(y) {
        return pick_invalid_candidate(series_order);
    }
    let radius = pick_resolve_scatter_radius(point_index);
    if radius <= 0.0 {
        return pick_invalid_candidate(series_order);
    }
    let point_px = pick_project_pair(x, y);
    if !pick_is_finite(point_px.x) || !pick_is_finite(point_px.y) {
        return pick_invalid_candidate(series_order);
    }
    let delta = point_px - pick_query_params.cursor_chart.xy;
    let center_distance = sqrt(fma(delta.x, delta.x, delta.y * delta.y));
    let hit_distance = max(center_distance - radius, 0.0);
    let distance_sq = hit_distance * hit_distance;
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    if !pick_is_finite(distance_sq) || distance_sq > maximum_sq {
        return pick_invalid_candidate(series_order);
    }
    return PickCandidate(
        1u,
        series_order,
        point_index,
        0u,
        point_index,
        distance_sq,
        hit_distance,
        0u,
    );
}

fn pick_line_candidate(
    segment_index: u32,
    ax: vec2<f32>,
    ay: vec2<f32>,
    bx: vec2<f32>,
    by: vec2<f32>,
) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    if !pick_pair_is_valid(ax) || !pick_pair_is_valid(ay) || !pick_pair_is_valid(bx) || !pick_pair_is_valid(by) {
        return pick_invalid_candidate(series_order);
    }
    let a_px = pick_project_pair(ax, ay);
    let b_px = pick_project_pair(bx, by);
    if !pick_is_finite(a_px.x) || !pick_is_finite(a_px.y) || !pick_is_finite(b_px.x) || !pick_is_finite(b_px.y) {
        return pick_invalid_candidate(series_order);
    }
    let segment = b_px - a_px;
    let length_sq = fma(segment.x, segment.x, segment.y * segment.y);
    if !pick_is_finite(length_sq) || length_sq <= PICK_F32_EPSILON {
        return pick_invalid_candidate(series_order);
    }
    let cursor_delta = pick_query_params.cursor_chart.xy - a_px;
    let t = clamp((cursor_delta.x * segment.x + cursor_delta.y * segment.y) / length_sq, 0.0, 1.0);
    let closest = a_px + segment * t;
    let delta = closest - pick_query_params.cursor_chart.xy;
    let center_distance = sqrt(fma(delta.x, delta.x, delta.y * delta.y));
    if !pick_is_finite(center_distance) {
        return pick_invalid_candidate(series_order);
    }
    let line_half_width =
        pick_query_params.scatter_line.y * pick_query_params.scatter_line.z;
    let hit_distance = max(center_distance - line_half_width, 0.0);
    let distance_sq = hit_distance * hit_distance;
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    if !pick_is_finite(distance_sq) || distance_sq > maximum_sq {
        return pick_invalid_candidate(series_order);
    }
    let use_b = t > 0.5;
    let point_index = select(segment_index, segment_index + 1u, use_b);
    return PickCandidate(
        1u,
        series_order,
        point_index,
        1u,
        segment_index,
        distance_sq,
        sqrt(distance_sq),
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
        if (flags & PICK_FLAG_SCATTER) != 0u {
            scatter_mask = scatter_mask | (1u << bit);
        }
        if (flags & PICK_FLAG_LINE) != 0u && point_index + 1u < point_count {
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
