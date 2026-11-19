// The simple-geometry fragments of the pick ID pass (#381 prototype, the
// "per column kind" measure): a point is a disc of its resolved marker
// radius, a segment a capsule of the stroke's half width around the
// straight chord between its points. No marker shape, dash, step, styled
// stroke, error bar, area or arrow is modelled: these fragments measure how
// far a style-free kernel per column kind lands from the exact geometry
// (`id_entries.wgsl` holds the exact fragments).

fn pick_id_simple_write(kind: u32, i: u32, distance_px: f32) -> PickIdOutput {
    if !(distance_px <= pick_query_params.chart_limits.z) {
        discard;
    }
    return pick_id_output(
        kind,
        pick_query_params.series.y,
        pick_source_row(i),
        pick_source_row(i),
        distance_px,
        pick_query_params.chart_limits.z,
        false,
    );
}

@fragment
fn pick_id_fs_points_disc(input: PickIdVarying) -> PickIdOutput {
    let i = input.pick_id.y;
    let x = pick_read_pair(pick_query_params.data.y, i);
    let y = pick_read_pair(pick_query_params.data.z, i);
    if !pick_coordinate_is_valid(x, 0u) || !pick_coordinate_is_valid(y, 1u) {
        discard;
    }
    let marker = pick_resolve_scatter_marker(i);
    if marker.hidden {
        discard;
    }
    let center = pick_data_px(x, y);
    let distance = max(length(pick_cursor_px() - center) - marker.radius, 0.0);
    return pick_id_simple_write(PICK_KIND_MARKER, i, distance);
}

@fragment
fn pick_id_fs_segments_capsule(input: PickIdVarying) -> PickIdOutput {
    let i = input.pick_id.y;
    let next = pick_next_point(i);
    let ax = pick_read_pair(pick_query_params.data.y, i);
    let ay = pick_read_pair(pick_query_params.data.z, i);
    let bx = pick_read_pair(pick_query_params.data.y, next);
    let by = pick_read_pair(pick_query_params.data.z, next);
    if !pick_coordinate_is_valid(ax, 0u) || !pick_coordinate_is_valid(ay, 1u)
        || !pick_coordinate_is_valid(bx, 0u) || !pick_coordinate_is_valid(by, 1u) {
        discard;
    }
    let a = pick_data_px(ax, ay);
    let b = pick_data_px(bx, by);
    let p = pick_cursor_px();
    let ab = b - a;
    let len_sq = dot(ab, ab);
    var t = 0.0;
    if len_sq > 0.0 {
        t = clamp(dot(p - a, ab) / len_sq, 0.0, 1.0);
    }
    let distance = max(length(p - (a + ab * t)) - pick_query_params.line_px.x, 0.0);
    // The nearer end is the point the pick reports, as the exact line pick does.
    let point = select(i, next, t > 0.5);
    if !(distance <= pick_query_params.chart_limits.z) {
        discard;
    }
    return pick_id_output(
        PICK_KIND_LINE,
        pick_query_params.series.y,
        pick_source_row(point),
        pick_source_row(i),
        distance,
        pick_query_params.chart_limits.z,
        false,
    );
}
