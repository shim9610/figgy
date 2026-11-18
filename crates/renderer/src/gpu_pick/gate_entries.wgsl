// Gate and reduction entries (the gate module: `core.wgsl` + this). The
// gates narrow the candidate points per axis; the reduction folds the
// per-series candidates. Neither reads the exact variant.

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

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_reduce_candidates(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = pick_invalid_candidate(0u);
    // A region series' entries (`PICK_KIND_REGION`) are folded beside the
    // candidates; the per-series reduction alone sees them.
    var region = PICK_REGION_NONE;
    let count = arrayLength(&pick_reduce_inputs);
    var index = local_index;
    while index < count {
        let candidate = pick_reduce_inputs[index];
        if candidate.primitive_kind == PICK_KIND_REGION {
            region = pick_region_fold(
                region,
                vec3<u32>(candidate.valid & 1u, candidate.point_index, candidate.series_order),
            );
        } else if pick_candidate_is_better(candidate, best) {
            best = candidate;
        }
        index = index + PICK_WORKGROUP_SIZE;
    }
    pick_shared_candidates[local_index] = best;
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
        var out = pick_shared_candidates[0];
        let fold = pick_shared_region[0];
        // An odd crossing count puts the cursor inside a region: a fill hit
        // at distance zero, which beats every farther candidate and loses
        // every tie to a marker or a stroke.
        if fold.x == 1u && fold.y != 0xffffffffu {
            let hit = pick_region_hit(fold.y, fold.z);
            if pick_candidate_is_better(hit, out) {
                out = hit;
            }
        }
        pick_reduce_output = out;
    }
}

