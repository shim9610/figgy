// The exact entry (every exact module ends with it): the gated or direct
// candidate scan of one series, calling the variant's
// `pick_scatter_candidate` / `pick_line_candidate`.

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
        if (flags & PICK_FLAG_LINE) != 0u && point_index < pick_query_params.stream.z && pick_starts_segment(point_index) {
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
    var region = PICK_REGION_NONE;

    if word_index < pick_query_params.series.x {
        // A region fill: the crossings of the cursor's ray with the edges
        // starting at this word's points, every point an edge's start.
        if pick_region() {
            let first = word_index * PICK_GATE_WORD_POINTS;
            for (var bit = 0u; bit < PICK_GATE_WORD_POINTS; bit = bit + 1u) {
                let point_index = first + bit;
                if point_index >= pick_query_params.data.x {
                    break;
                }
                if pick_region_crossing(point_index) {
                    region = pick_region_fold(region, vec3<u32>(1u, point_index, series_order));
                }
            }
        }
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
                // The next point, or the first one for the closing segment of
                // a closed line (`pick_next_point`).
                let next = pick_next_point(point_index);
                let next_x = pick_read_pair(pick_query_params.data.y, next);
                let next_y = pick_read_pair(pick_query_params.data.z, next);
                let candidate =
                    pick_line_candidate(point_index, x, y, next_x, next_y);
                if pick_candidate_is_better(candidate, local_best) {
                    local_best = candidate;
                }
                cached_x = next_x;
                cached_y = next_y;
                cached_index = next;
                has_cached_point = true;
            } else {
                has_cached_point = false;
            }
        }
    }

    pick_shared_candidates[local_index] = local_best;
    pick_shared_region[local_index] = region;
    workgroupBarrier();
    for (var stride = PICK_WORKGROUP_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if local_index < stride {
            let candidate = pick_shared_candidates[local_index + stride];
            if pick_candidate_is_better(candidate, pick_shared_candidates[local_index]) {
                pick_shared_candidates[local_index] = candidate;
            }
            pick_shared_region[local_index] = pick_region_fold(
                pick_shared_region[local_index],
                pick_shared_region[local_index + stride],
            );
        }
        workgroupBarrier();
    }
    if local_index == 0u {
        if pick_region() {
            // The region entries follow the best candidates: the buffer
            // holds two per group.
            let groups = arrayLength(&pick_workgroup_candidates) / 2u;
            if group_index < groups {
                pick_workgroup_candidates[group_index] = pick_shared_candidates[0];
                let fold = pick_shared_region[0];
                pick_workgroup_candidates[groups + group_index] =
                    pick_region_entry(fold.x, fold.y, series_order);
            }
        } else if group_index < arrayLength(&pick_workgroup_candidates) {
            pick_workgroup_candidates[group_index] = pick_shared_candidates[0];
        }
    }
}

