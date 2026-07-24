// Exact GPU point/segment picking over ColumnPool split-f32 values.
//
// This module deliberately uses pick_* names and its own layouts.  It is not
// part of the render-shader common block: the picker consumes the same public
// data representation, but owns a compute-only ABI and a persistent packed
// block BVH.

const PICK_BLOCK_POINTS: u32 = 64u;
const PICK_WORKGROUP_SIZE: u32 = 64u;
const PICK_F32_MAX: f32 = 0x1.fffffep+127;
const PICK_F32_EPSILON: f32 = 1.1920929e-7;

const PICK_NODE_LEAF: u32 = 1u;
const PICK_FLAG_SCATTER: u32 = 1u;
const PICK_FLAG_LINE: u32 = 2u;
const PICK_FLAG_STYLE_MAP: u32 = 4u;
const PICK_FLAG_STYLE_INDEX: u32 = 8u;

const PICK_STYLE_MASK_RADIUS: u32 = 2u;
const PICK_STYLE_MASK_SHAPE: u32 = 4u;

struct PickBvhNode {
    // Independent lane intervals are intentionally wider than an interval of
    // (hi + lo).  They remain conservative when the query subtracts split
    // axis bounds before adding the lanes (the precision-preserving linear
    // path used by rendering).
    x_hi_bounds: vec2<f32>,
    x_lo_bounds: vec2<f32>,
    y_hi_bounds: vec2<f32>,
    y_lo_bounds: vec2<f32>,
    // Leaf: first point and number of points owned by this leaf.
    // Internal: first child node and child count (one or two).
    first: u32,
    count: u32,
    kind: u32,
    valid: u32,
};

struct PickBuildParams {
    point_count: u32,
    x_base: u32,
    y_base: u32,
    input_start: u32,
    output_start: u32,
    input_count: u32,
    invocation_base: u32,
    include_line_boundary: u32,
};

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
    // (chart_width, chart_height, maximum pick distance, maximum primitive
    // center extent).  The final component is only a conservative BVH
    // expansion; exact radii/widths are resolved in leaves.
    chart_limits: vec4<f32>,
    // (base scatter radius, line half width, display scale, unused).
    scatter_line: vec4<f32>,
    // (point count, x f32-lane base, y f32-lane base, style-index lane base).
    data: vec4<u32>,
    // (flags, style count, override count, style-index logical length).
    style: vec4<u32>,
    // (root node, series order, base shape id, node count).
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

struct PickQueueState {
    head: atomic<u32>,
    tail: atomic<u32>,
    overflow: atomic<u32>,
    _pad: u32,
};

// Forty-eight bytes, byte-for-byte with Rust's PickCandidateGpu.
struct PickCandidate {
    valid: u32,
    series_order: u32,
    point_index: u32,
    primitive_kind: u32, // scatter=0, line=1 (CPU traversal order)
    primitive_index: u32,
    data_x: f32,
    data_y: f32,
    distance_sq: f32,
    distance_px: f32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<storage, read> pick_pool_words: array<u32>;
@group(0) @binding(1) var<storage, read_write> pick_bvh_nodes: array<PickBvhNode>;
@group(0) @binding(2) var<uniform> pick_build_params: PickBuildParams;

// Reduction-only resources share group 0 but use bindings not reachable from
// the build/query entry points.
@group(0) @binding(3) var<storage, read> pick_reduce_inputs: array<PickCandidate>;
@group(0) @binding(4) var<storage, read_write> pick_reduce_output: PickCandidate;

@group(1) @binding(0) var<storage, read_write> pick_node_queue: array<u32>;
@group(1) @binding(1) var<storage, read_write> pick_queue_state: PickQueueState;
@group(1) @binding(2) var<uniform> pick_query_params: PickQueryParams;
@group(1) @binding(3) var<storage, read> pick_style_slots: array<PickScatterStyleSlot>;
@group(1) @binding(4) var<storage, read> pick_style_overrides: array<PickScatterStyleOverride>;
@group(1) @binding(5) var<storage, read_write> pick_series_output: PickCandidate;

var<workgroup> pick_round_begin: u32;
var<workgroup> pick_round_end: u32;
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

fn pick_empty_node(first: u32, count: u32, kind: u32) -> PickBvhNode {
    return PickBvhNode(
        vec2<f32>(PICK_F32_MAX, -PICK_F32_MAX),
        vec2<f32>(PICK_F32_MAX, -PICK_F32_MAX),
        vec2<f32>(PICK_F32_MAX, -PICK_F32_MAX),
        vec2<f32>(PICK_F32_MAX, -PICK_F32_MAX),
        first,
        count,
        kind,
        0u,
    );
}

fn pick_grow_node(node_in: PickBvhNode, x: vec2<f32>, y: vec2<f32>) -> PickBvhNode {
    var node = node_in;
    node.x_hi_bounds = vec2<f32>(min(node.x_hi_bounds.x, x.x), max(node.x_hi_bounds.y, x.x));
    node.x_lo_bounds = vec2<f32>(min(node.x_lo_bounds.x, x.y), max(node.x_lo_bounds.y, x.y));
    node.y_hi_bounds = vec2<f32>(min(node.y_hi_bounds.x, y.x), max(node.y_hi_bounds.y, y.x));
    node.y_lo_bounds = vec2<f32>(min(node.y_lo_bounds.x, y.y), max(node.y_lo_bounds.y, y.y));
    node.valid = 1u;
    return node;
}

fn pick_union_node(parent_in: PickBvhNode, child: PickBvhNode) -> PickBvhNode {
    if child.valid == 0u {
        return parent_in;
    }
    var parent = parent_in;
    parent.x_hi_bounds = vec2<f32>(min(parent.x_hi_bounds.x, child.x_hi_bounds.x), max(parent.x_hi_bounds.y, child.x_hi_bounds.y));
    parent.x_lo_bounds = vec2<f32>(min(parent.x_lo_bounds.x, child.x_lo_bounds.x), max(parent.x_lo_bounds.y, child.x_lo_bounds.y));
    parent.y_hi_bounds = vec2<f32>(min(parent.y_hi_bounds.x, child.y_hi_bounds.x), max(parent.y_hi_bounds.y, child.y_hi_bounds.y));
    parent.y_lo_bounds = vec2<f32>(min(parent.y_lo_bounds.x, child.y_lo_bounds.x), max(parent.y_lo_bounds.y, child.y_lo_bounds.y));
    parent.valid = 1u;
    return parent;
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_build_leaves(@builtin(global_invocation_id) gid: vec3<u32>) {
    let leaf_index = pick_build_params.invocation_base + gid.x;
    if leaf_index >= pick_build_params.input_count {
        return;
    }

    let first = leaf_index * PICK_BLOCK_POINTS;
    let owned_count = min(PICK_BLOCK_POINTS, pick_build_params.point_count - first);
    var scan_count = owned_count;
    if pick_build_params.include_line_boundary != 0u && first + scan_count < pick_build_params.point_count {
        scan_count = scan_count + 1u;
    }

    var node = pick_empty_node(first, owned_count, PICK_NODE_LEAF);
    for (var local = 0u; local < scan_count; local = local + 1u) {
        let point_index = first + local;
        let x = pick_read_pair(pick_build_params.x_base, point_index);
        let y = pick_read_pair(pick_build_params.y_base, point_index);
        if pick_pair_is_valid(x) && pick_pair_is_valid(y) {
            node = pick_grow_node(node, x, y);
        }
    }
    pick_bvh_nodes[pick_build_params.output_start + leaf_index] = node;
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_build_internal(@builtin(global_invocation_id) gid: vec3<u32>) {
    let parent_local = pick_build_params.invocation_base + gid.x;
    let parent_count = (pick_build_params.input_count + 1u) / 2u;
    if parent_local >= parent_count {
        return;
    }

    let first_child = pick_build_params.input_start + parent_local * 2u;
    let child_count = min(2u, pick_build_params.input_count - parent_local * 2u);
    var parent = pick_empty_node(first_child, child_count, 0u);
    parent = pick_union_node(parent, pick_bvh_nodes[first_child]);
    if child_count == 2u {
        parent = pick_union_node(parent, pick_bvh_nodes[first_child + 1u]);
    }
    pick_bvh_nodes[pick_build_params.output_start + parent_local] = parent;
}

fn pick_next_up_once(v: f32) -> f32 {
    if v != v || v >= PICK_F32_MAX {
        return v;
    }
    if v == 0.0 {
        return bitcast<f32>(1u);
    }
    let bits = bitcast<u32>(v);
    return bitcast<f32>(select(bits - 1u, bits + 1u, v > 0.0));
}

fn pick_next_down_once(v: f32) -> f32 {
    if v != v || v <= -PICK_F32_MAX {
        return v;
    }
    if v == 0.0 {
        return bitcast<f32>(0x80000001u);
    }
    let bits = bitcast<u32>(v);
    return bitcast<f32>(select(bits + 1u, bits - 1u, v > 0.0));
}

// Eight ULPs cover the WGSL transcendental accuracy allowance while keeping
// the hierarchy useful.  Every elementary interval operation below rounds
// outward as well, so no exact leaf candidate can be rejected by a parent.
fn pick_next_up(v_in: f32) -> f32 {
    var v = v_in;
    for (var i = 0u; i < 8u; i = i + 1u) {
        v = pick_next_up_once(v);
    }
    return v;
}

fn pick_next_down(v_in: f32) -> f32 {
    var v = v_in;
    for (var i = 0u; i < 8u; i = i + 1u) {
        v = pick_next_down_once(v);
    }
    return v;
}

struct PickInterval {
    lo: f32,
    hi: f32,
};

fn pick_interval(lo: f32, hi: f32) -> PickInterval {
    return PickInterval(min(lo, hi), max(lo, hi));
}

fn pick_interval_add(a: PickInterval, b: PickInterval) -> PickInterval {
    return PickInterval(pick_next_down(a.lo + b.lo), pick_next_up(a.hi + b.hi));
}

fn pick_interval_sub(a: PickInterval, b: PickInterval) -> PickInterval {
    return PickInterval(pick_next_down(a.lo - b.hi), pick_next_up(a.hi - b.lo));
}

fn pick_interval_mul_scalar(a: PickInterval, s: f32) -> PickInterval {
    let p0 = a.lo * s;
    let p1 = a.hi * s;
    return PickInterval(pick_next_down(min(p0, p1)), pick_next_up(max(p0, p1)));
}

fn pick_interval_div_scalar(a: PickInterval, s: f32) -> PickInterval {
    let p0 = a.lo / s;
    let p1 = a.hi / s;
    return PickInterval(pick_next_down(min(p0, p1)), pick_next_up(max(p0, p1)));
}

fn pick_log10(v: f32) -> f32 {
    return log(max(v, 1e-30)) / log(10.0);
}

fn pick_axis_t_interval(hi_bounds: vec2<f32>, lo_bounds: vec2<f32>, axis: u32) -> PickInterval {
    let hi = pick_interval(hi_bounds.x, hi_bounds.y);
    let lo = pick_interval(lo_bounds.x, lo_bounds.y);
    let min_hi = pick_query_params.transform.data_min[axis];
    let max_hi = pick_query_params.transform.data_max[axis];
    let min_lo = pick_query_params.transform.data_min_lo[axis];
    let max_lo = pick_query_params.transform.data_max_lo[axis];
    let range = (max_hi - min_hi) + (max_lo - min_lo);

    if pick_query_params.transform.scale_log[axis] >= 0.5 {
        let raw = pick_interval_add(hi, lo);
        let logged = PickInterval(
            pick_next_down(pick_log10(raw.lo)),
            pick_next_up(pick_log10(raw.hi)),
        );
        let numerator = pick_interval_sub(
            pick_interval_sub(logged, PickInterval(min_hi, min_hi)),
            PickInterval(min_lo, min_lo),
        );
        return pick_interval_div_scalar(numerator, range);
    }

    let hi_num = pick_interval_sub(hi, PickInterval(min_hi, min_hi));
    let lo_num = pick_interval_sub(lo, PickInterval(min_lo, min_lo));
    return pick_interval_div_scalar(pick_interval_add(hi_num, lo_num), range);
}

fn pick_axis_canvas_interval(t: PickInterval, axis: u32) -> PickInterval {
    let ndc = pick_interval_add(
        pick_interval_mul_scalar(t, 2.0),
        PickInterval(-1.0, -1.0),
    );
    if axis == 0u {
        let shifted = pick_interval_add(ndc, PickInterval(1.0, 1.0));
        let scaled = pick_interval_mul_scalar(
            pick_interval_mul_scalar(shifted, 0.5),
            pick_query_params.chart_limits.x,
        );
        return pick_interval_add(scaled, PickInterval(pick_query_params.cursor_chart.z, pick_query_params.cursor_chart.z));
    }
    let flipped = pick_interval_sub(PickInterval(1.0, 1.0), ndc);
    let scaled = pick_interval_mul_scalar(
        pick_interval_mul_scalar(flipped, 0.5),
        pick_query_params.chart_limits.y,
    );
    return pick_interval_add(scaled, PickInterval(pick_query_params.cursor_chart.w, pick_query_params.cursor_chart.w));
}

fn pick_node_may_hit(node: PickBvhNode) -> bool {
    if node.valid == 0u {
        return false;
    }
    let tx = pick_axis_t_interval(node.x_hi_bounds, node.x_lo_bounds, 0u);
    let ty = pick_axis_t_interval(node.y_hi_bounds, node.y_lo_bounds, 1u);
    let px = pick_axis_canvas_interval(tx, 0u);
    let py = pick_axis_canvas_interval(ty, 1u);

    // A non-finite interval means the transform itself is degenerate.  Do not
    // prune: exact leaf projection remains the authority.
    if !pick_is_finite(px.lo) || !pick_is_finite(px.hi) || !pick_is_finite(py.lo) || !pick_is_finite(py.hi) {
        return true;
    }

    let cursor = pick_query_params.cursor_chart.xy;
    let dx = max(max(px.lo - cursor.x, cursor.x - px.hi), 0.0);
    let dy = max(max(py.lo - cursor.y, cursor.y - py.hi), 0.0);
    let center_distance = sqrt(fma(dx, dx, dy * dy));
    let threshold = pick_next_up(
        pick_query_params.chart_limits.z
            + pick_query_params.chart_limits.w * pick_query_params.scatter_line.z,
    );
    return !pick_is_finite(center_distance) || center_distance <= threshold;
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

fn pick_project_pair(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    let tx = pick_axis_pair_to_t(x, 0u);
    let ty = pick_axis_pair_to_t(y, 1u);
    let ndc = vec2<f32>(tx, ty) * 2.0 - 1.0;
    return vec2<f32>(
        pick_query_params.cursor_chart.z + (ndc.x + 1.0) * 0.5 * pick_query_params.chart_limits.x,
        pick_query_params.cursor_chart.w + (1.0 - ndc.y) * 0.5 * pick_query_params.chart_limits.y,
    );
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
    return PickCandidate(0u, series_order, 0u, 0u, 0u, 0.0, 0.0, PICK_F32_MAX, PICK_F32_MAX, 0u, 0u, 0u);
}

fn pick_error_candidate(series_order: u32) -> PickCandidate {
    return PickCandidate(2u, series_order, 0u, 0u, 0u, 0.0, 0.0, 0.0, 0.0, 0u, 0u, 0u);
}

fn pick_candidate_is_better(candidate: PickCandidate, incumbent: PickCandidate) -> bool {
    if candidate.valid == 2u {
        return incumbent.valid != 2u;
    }
    if incumbent.valid == 2u {
        return false;
    }
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

fn pick_scatter_candidate(point_index: u32) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    let x = pick_read_pair(pick_query_params.data.y, point_index);
    let y = pick_read_pair(pick_query_params.data.z, point_index);
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
        x.x + x.y,
        y.x + y.y,
        distance_sq,
        hit_distance,
        0u,
        0u,
        0u,
    );
}

fn pick_line_candidate(segment_index: u32) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    let ax = pick_read_pair(pick_query_params.data.y, segment_index);
    let ay = pick_read_pair(pick_query_params.data.z, segment_index);
    let bx = pick_read_pair(pick_query_params.data.y, segment_index + 1u);
    let by = pick_read_pair(pick_query_params.data.z, segment_index + 1u);
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
    let data_x = select(ax.x + ax.y, bx.x + bx.y, use_b);
    let data_y = select(ay.x + ay.y, by.x + by.y, use_b);
    return PickCandidate(
        1u,
        series_order,
        point_index,
        1u,
        segment_index,
        data_x,
        data_y,
        distance_sq,
        sqrt(distance_sq),
        0u,
        0u,
        0u,
    );
}

fn pick_test_leaf(node: PickBvhNode, best_in: PickCandidate) -> PickCandidate {
    var best = best_in;
    let flags = pick_query_params.style.x;
    if (flags & PICK_FLAG_SCATTER) != 0u {
        for (var local = 0u; local < node.count; local = local + 1u) {
            let candidate = pick_scatter_candidate(node.first + local);
            if pick_candidate_is_better(candidate, best) {
                best = candidate;
            }
        }
    }
    if (flags & PICK_FLAG_LINE) != 0u && node.first < pick_query_params.data.x - 1u {
        let segment_count = min(node.count, pick_query_params.data.x - 1u - node.first);
        for (var local = 0u; local < segment_count; local = local + 1u) {
            let candidate = pick_line_candidate(node.first + local);
            if pick_candidate_is_better(candidate, best) {
                best = candidate;
            }
        }
    }
    return best;
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_query_bvh(@builtin(local_invocation_index) local_index: u32) {
    let series_order = pick_query_params.series.y;
    var local_best = pick_invalid_candidate(series_order);

    if local_index == 0u {
        atomicStore(&pick_queue_state.head, 0u);
        atomicStore(&pick_queue_state.tail, 1u);
        atomicStore(&pick_queue_state.overflow, 0u);
        pick_node_queue[0] = pick_query_params.series.x;
    }
    storageBarrier();
    workgroupBarrier();

    loop {
        if local_index == 0u {
            pick_round_begin = atomicLoad(&pick_queue_state.head);
            pick_round_end = atomicLoad(&pick_queue_state.tail);
            atomicStore(&pick_queue_state.head, pick_round_end);
        }
        workgroupBarrier();
        if pick_round_begin >= pick_round_end {
            break;
        }

        var queue_index = pick_round_begin + local_index;
        while queue_index < pick_round_end {
            let node_index = pick_node_queue[queue_index];
            let node = pick_bvh_nodes[node_index];
            if pick_node_may_hit(node) {
                if node.kind == PICK_NODE_LEAF {
                    local_best = pick_test_leaf(node, local_best);
                } else {
                    for (var child = 0u; child < node.count; child = child + 1u) {
                        let destination = atomicAdd(&pick_queue_state.tail, 1u);
                        if destination < pick_query_params.series.w {
                            pick_node_queue[destination] = node.first + child;
                        } else {
                            atomicStore(&pick_queue_state.overflow, 1u);
                        }
                    }
                }
            }
            queue_index = queue_index + PICK_WORKGROUP_SIZE;
        }
        storageBarrier();
        workgroupBarrier();
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

    if local_index == 0u {
        if atomicLoad(&pick_queue_state.overflow) != 0u {
            pick_series_output = pick_error_candidate(series_order);
        } else {
            pick_series_output = pick_shared_candidates[0];
        }
    }
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_reduce_series(@builtin(local_invocation_index) local_index: u32) {
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
