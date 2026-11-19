// The segment entries of the pick ID pass (#381 prototype,
// `id_entries.wgsl`): a polyline's segment from a point to the next, gated
// as the point picker's X and Y gates keep a line segment, measured with the
// exact module's `pick_line_candidate`. Spliced after `id_entries.wgsl` in
// the modules whose kind has a line.

// Whether the gates would keep the segment from point `i` to `next`.
fn pick_id_segment_gate(i: u32, next: u32) -> bool {
    if pick_id_ungated() || next <= i {
        return true;
    }
    let ax = pick_read_pair(pick_query_params.data.y, i);
    let bx = pick_read_pair(pick_query_params.data.y, next);
    let ay = pick_read_pair(pick_query_params.data.z, i);
    let by = pick_read_pair(pick_query_params.data.z, next);
    if !pick_pair_is_valid(ax) || !pick_pair_is_valid(bx) || !pick_pair_is_valid(ay) || !pick_pair_is_valid(by) {
        return false;
    }
    return pick_line_projected_may_hit(pick_project_axis_pair(ax, 0u), pick_project_axis_pair(bx, 0u), 0u)
        && pick_line_projected_may_hit(pick_project_axis_pair(ay, 1u), pick_project_axis_pair(by, 1u), 1u);
}

@vertex
fn pick_id_vs_segments(@builtin(vertex_index) i: u32) -> PickIdVarying {
    let series = pick_query_params.series.y;
    if (pick_query_params.style.x & PICK_FLAG_LINE) == 0u
        || i >= pick_query_params.data.x
        || i >= pick_query_params.stream.z
        || !pick_starts_segment(i) {
        return pick_id_culled(series, i);
    }
    if !pick_id_segment_gate(i, pick_next_point(i)) {
        return pick_id_culled(series, i);
    }
    return pick_id_varying(series, i, PICK_ID_CENTRE);
}

@fragment
fn pick_id_fs_segments(input: PickIdVarying) -> PickIdOutput {
    let i = input.pick_id.y;
    let next = pick_next_point(i);
    let candidate = pick_line_candidate(
        i,
        pick_read_pair(pick_query_params.data.y, i),
        pick_read_pair(pick_query_params.data.z, i),
        pick_read_pair(pick_query_params.data.y, next),
        pick_read_pair(pick_query_params.data.z, next),
    );
    if candidate.valid == 0u {
        discard;
    }
    return pick_id_write(candidate);
}
