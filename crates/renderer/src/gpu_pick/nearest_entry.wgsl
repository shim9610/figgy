// The nearest-datum pick (#381: one nearest datum per series, no pick
// reach): one compute kernel for every series
// whose data are (x, y) columns (markers, lines, point vectors, stems and
// error bars anchored at their points). Each datum is placed by the
// projection fragment spliced before this one (`nearest_project_2d.wgsl`
// or `nearest_project_3d.wgsl`: `pick_nearest_offset`, the datum's screen
// offset from the cursor in pixels, through the draw's own shared
// projection); its length is the pick distance. There is no shape,
// stroke, dash or style, and no reach: every datum the draw places is a
// candidate.
//
// The layout is the exact pass's (`exact_entry.wgsl`): one workgroup of
// `PICK_WORKGROUP_SIZE` invocations per `PICK_GATE_WORD_POINTS` × that many
// points, the dispatch the registry already sized (`dispatch_x`,
// `dispatch_y`), and one candidate per workgroup in
// `pick_workgroup_candidates`, which the point picker's per-series and
// cross-series reductions (`pick_reduce_candidates`) fold: the nearest wins,
// an exact tie goes to the later series (drawn on top), then within a series
// to the later index.

fn pick_nearest_candidate(i: u32) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    let offset = pick_nearest_offset(i);
    if offset.z == 0.0 {
        return pick_invalid_candidate(series_order);
    }
    let d = offset.xy;
    let distance_sq = dot(d, d);
    if !pick_is_finite(distance_sq) {
        return pick_invalid_candidate(series_order);
    }
    let kind = select(PICK_KIND_LINE, PICK_KIND_MARKER, (pick_query_params.style.x & PICK_FLAG_SCATTER) != 0u);
    let row = pick_source_row(i);
    return PickCandidate(1u, series_order, row, kind, row, distance_sq, sqrt(distance_sq), 0u);
}

@compute @workgroup_size(PICK_WORKGROUP_SIZE)
fn pick_nearest_points(
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    figgy_zero_workgroup(local_index);
    let group_index = pick_linear_workgroup_id(workgroup_id);
    let word_index = group_index * PICK_WORKGROUP_SIZE + local_index;
    var local_best = pick_invalid_candidate(pick_query_params.series.y);
    if word_index < pick_query_params.series.x {
        let first = word_index * PICK_GATE_WORD_POINTS;
        let count = pick_query_params.data.x;
        for (var bit = 0u; bit < PICK_GATE_WORD_POINTS; bit = bit + 1u) {
            let i = first + bit;
            if i >= count {
                break;
            }
            let candidate = pick_nearest_candidate(i);
            if pick_candidate_is_better(candidate, local_best) {
                local_best = candidate;
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
    if local_index == 0u {
        if pick_region() {
            // A region series' buffer holds a region entry per group after
            // the candidates (`exact_entry.wgsl`); this kernel has none, so
            // it overwrites what an exact pick left there.
            let groups = arrayLength(&pick_workgroup_candidates) / 2u;
            if group_index < groups {
                pick_workgroup_candidates[group_index] = pick_shared_candidates[0];
                pick_workgroup_candidates[groups + group_index] =
                    pick_invalid_candidate(pick_query_params.series.y);
            }
        } else if group_index < arrayLength(&pick_workgroup_candidates) {
            pick_workgroup_candidates[group_index] = pick_shared_candidates[0];
        }
    }
}
