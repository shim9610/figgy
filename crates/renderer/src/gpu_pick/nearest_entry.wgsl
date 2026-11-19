// The nearest-datum pick (#381 prototype, the maintainer's decision): the
// `PICK_NEAREST_K` data nearest to the cursor, with no pick reach, for every
// series whose data are (x, y) columns (markers, lines, point vectors, stems
// and error bars, each by its anchor points). Each datum is placed with the
// draw's own projection (`pick_data_px`, the shared transform and
// projection fragments) and measured by its screen distance to the cursor.
// There is no shape, stroke, dash or style.
//
// `PICK_NEAREST_K` is the result size the maintainer chose (#381), not a
// cap on any data count: every datum is measured, and fewer than K come
// back only when fewer exist.
//
// Two entries:
// - `pick_nearest_points`, per series, in the exact pass's layout (one
//   workgroup of `PICK_WORKGROUP_SIZE` invocations per
//   `PICK_GATE_WORD_POINTS` × that many points): each invocation keeps the
//   K nearest of its points, the workgroup merges them in shared memory
//   (distance and index only, 8 bytes each), and writes its K nearest to
//   `pick_workgroup_candidates[group × K ..]`.
// - `pick_nearest_merge`, once per pick, over every series' workgroup lists
//   (`pick_reduce_inputs`): the K best by `pick_candidate_is_better` (the
//   nearest; an exact tie to the series drawn later, then the later index,
//   both drawn on top), sorted, into `pick_workgroup_candidates[0 .. K]`.

const PICK_NEAREST_K: u32 = 10u;
const PICK_NEAREST_NONE: u32 = 0xffffffffu;

// (distance², index); no entry is (PICK_F32_MAX, PICK_NEAREST_NONE).
var<workgroup> pick_nearest_lists: array<array<vec2<u32>, PICK_NEAREST_K>, PICK_WORKGROUP_SIZE>;
// The merge pass's lists: indices into `pick_reduce_inputs`.
var<workgroup> pick_nearest_merged: array<array<u32, PICK_NEAREST_K>, PICK_WORKGROUP_SIZE>;

fn pick_nearest_empty() -> vec2<u32> {
    return vec2<u32>(bitcast<u32>(PICK_F32_MAX), PICK_NEAREST_NONE);
}

// Whether `a` comes before `b` within one series: nearer, then the later
// index (drawn on top). Distances are non-negative, so their bits order as
// the values do.
fn pick_nearest_before(a: vec2<u32>, b: vec2<u32>) -> bool {
    if a.y == PICK_NEAREST_NONE {
        return false;
    }
    if b.y == PICK_NEAREST_NONE {
        return true;
    }
    if a.x != b.x {
        return a.x < b.x;
    }
    return a.y > b.y;
}

// Datum `i`'s squared screen distance to the cursor, or none when the
// draw places nothing there.
fn pick_nearest_entry(i: u32) -> vec2<u32> {
    let x = pick_read_pair(pick_query_params.data.y, i);
    let y = pick_read_pair(pick_query_params.data.z, i);
    var p = vec2<f32>(0.0);
    if pick_ternary() {
        if !pick_ternary_valid(i, x, y) {
            return pick_nearest_empty();
        }
        p = pick_ternary_px(i, x, y);
    } else {
        if !pick_coordinate_is_valid(x, 0u) || !pick_coordinate_is_valid(y, 1u)
            || !polar_sample_valid(pick_query_params.transform, x, y) {
            return pick_nearest_empty();
        }
        p = pick_data_px(x, y);
    }
    let d = pick_cursor_px() - p;
    let distance_sq = dot(d, d);
    if !pick_is_finite(distance_sq) {
        return pick_nearest_empty();
    }
    return vec2<u32>(bitcast<u32>(distance_sq), i);
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_nearest_points(
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    let group_index = pick_linear_workgroup_id(workgroup_id);
    let word_index = group_index * PICK_WORKGROUP_SIZE + local_index;
    var list: array<vec2<u32>, PICK_NEAREST_K>;
    for (var k = 0u; k < PICK_NEAREST_K; k = k + 1u) {
        list[k] = pick_nearest_empty();
    }
    if word_index < pick_query_params.series.x {
        let first = word_index * PICK_GATE_WORD_POINTS;
        let count = pick_query_params.data.x;
        for (var bit = 0u; bit < PICK_GATE_WORD_POINTS; bit = bit + 1u) {
            let i = first + bit;
            if i >= count {
                break;
            }
            var entry = pick_nearest_entry(i);
            if !pick_nearest_before(entry, list[PICK_NEAREST_K - 1u]) {
                continue;
            }
            // Insertion into the sorted list: carry the entry down.
            for (var k = 0u; k < PICK_NEAREST_K; k = k + 1u) {
                if pick_nearest_before(entry, list[k]) {
                    let moved = list[k];
                    list[k] = entry;
                    entry = moved;
                }
            }
        }
    }
    pick_nearest_lists[local_index] = list;
    workgroupBarrier();
    // Tree merge of two sorted lists into the lower slot.
    for (var stride = PICK_WORKGROUP_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if local_index < stride {
            let a = pick_nearest_lists[local_index];
            let b = pick_nearest_lists[local_index + stride];
            var merged: array<vec2<u32>, PICK_NEAREST_K>;
            var ia = 0u;
            var ib = 0u;
            for (var k = 0u; k < PICK_NEAREST_K; k = k + 1u) {
                if ib >= PICK_NEAREST_K || (ia < PICK_NEAREST_K && !pick_nearest_before(b[ib], a[ia])) {
                    merged[k] = a[ia];
                    ia = ia + 1u;
                } else {
                    merged[k] = b[ib];
                    ib = ib + 1u;
                }
            }
            pick_nearest_lists[local_index] = merged;
        }
        workgroupBarrier();
    }
    if local_index == 0u {
        let base = group_index * PICK_NEAREST_K;
        let kind = select(PICK_KIND_LINE, PICK_KIND_MARKER, (pick_query_params.style.x & PICK_FLAG_SCATTER) != 0u);
        for (var k = 0u; k < PICK_NEAREST_K; k = k + 1u) {
            if base + k >= arrayLength(&pick_workgroup_candidates) {
                break;
            }
            let entry = pick_nearest_lists[0][k];
            var candidate = pick_invalid_candidate(pick_query_params.series.y);
            if entry.y != PICK_NEAREST_NONE {
                let distance_sq = bitcast<f32>(entry.x);
                let row = pick_source_row(entry.y);
                candidate = PickCandidate(1u, pick_query_params.series.y, row, kind, row, distance_sq, sqrt(distance_sq), 0u);
            }
            pick_workgroup_candidates[base + k] = candidate;
        }
    }
}

// Whether input `a` comes before input `b` (indices into
// `pick_reduce_inputs`; none sorts last).
fn pick_nearest_input_before(a: u32, b: u32) -> bool {
    if a == PICK_NEAREST_NONE {
        return false;
    }
    if b == PICK_NEAREST_NONE {
        return pick_reduce_inputs[a].valid == 1u;
    }
    return pick_candidate_is_better(pick_reduce_inputs[a], pick_reduce_inputs[b]);
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_nearest_merge(@builtin(local_invocation_index) local_index: u32) {
    var list: array<u32, PICK_NEAREST_K>;
    for (var k = 0u; k < PICK_NEAREST_K; k = k + 1u) {
        list[k] = PICK_NEAREST_NONE;
    }
    let count = arrayLength(&pick_reduce_inputs);
    for (var index = local_index; index < count; index = index + PICK_WORKGROUP_SIZE) {
        var entry = index;
        if !pick_nearest_input_before(entry, list[PICK_NEAREST_K - 1u]) {
            continue;
        }
        for (var k = 0u; k < PICK_NEAREST_K; k = k + 1u) {
            if pick_nearest_input_before(entry, list[k]) {
                let moved = list[k];
                list[k] = entry;
                entry = moved;
            }
        }
    }
    pick_nearest_merged[local_index] = list;
    workgroupBarrier();
    for (var stride = PICK_WORKGROUP_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if local_index < stride {
            let a = pick_nearest_merged[local_index];
            let b = pick_nearest_merged[local_index + stride];
            var merged: array<u32, PICK_NEAREST_K>;
            var ia = 0u;
            var ib = 0u;
            for (var k = 0u; k < PICK_NEAREST_K; k = k + 1u) {
                if ib >= PICK_NEAREST_K || (ia < PICK_NEAREST_K && !pick_nearest_input_before(b[ib], a[ia])) {
                    merged[k] = a[ia];
                    ia = ia + 1u;
                } else {
                    merged[k] = b[ib];
                    ib = ib + 1u;
                }
            }
            pick_nearest_merged[local_index] = merged;
        }
        workgroupBarrier();
    }
    if local_index == 0u {
        for (var k = 0u; k < PICK_NEAREST_K; k = k + 1u) {
            if k >= arrayLength(&pick_workgroup_candidates) {
                break;
            }
            let index = pick_nearest_merged[0][k];
            var candidate = pick_invalid_candidate(0u);
            if index != PICK_NEAREST_NONE {
                candidate = pick_reduce_inputs[index];
            }
            pick_workgroup_candidates[k] = candidate;
        }
    }
}
