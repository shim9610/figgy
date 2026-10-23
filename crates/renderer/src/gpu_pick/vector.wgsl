// Exact fragment: point vector arrows (SHADER_COMMON.md §5.8) as the
// scatter primitive of a vector series.

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
        PICK_KIND_MARKER,
        pick_source_row(point_index),
        distance_sq,
        hit_distance,
        0u,
    );
}


// A point vector series' scatter primitive: the arrows it draws.
fn pick_scatter_candidate(
    point_index: u32,
    x: vec2<f32>,
    y: vec2<f32>,
) -> PickCandidate {
    return pick_vector_candidate(point_index, x, y);
}
