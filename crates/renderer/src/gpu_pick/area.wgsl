// Exact fragment: a line's area fill (SHADER_COMMON.md §2.2), measured on
// the trapezoids `common/area_piece.wgsl` builds for `vs_area`, as part of
// the series' line primitive.

fn pick_cross(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

// Whether triangle (a, b, c) of non-zero area covers `p`, its edges
// included, in either winding: the area pipeline culls no face.
fn pick_triangle_covers(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> bool {
    let area = pick_cross(b - a, c - a);
    if !(abs(area) > 0.0) {
        return false;
    }
    let w0 = pick_cross(b - a, p - a);
    let w1 = pick_cross(c - b, p - b);
    let w2 = pick_cross(a - c, p - c);
    if area > 0.0 {
        return w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0;
    }
    return w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0;
}

// The fill under segment `segment_index` (A = `ax`, `ay`, B = `bx`, `by`):
// a hit at distance zero where the cursor lies in one of the two triangles
// a piece's trapezoid is drawn with, and nothing anywhere else (a fill has
// no stroke to come near). The polar fragment clip cuts it like every
// primitive. The hit names the segment's nearer endpoint by the line rule,
// the position of the cursor's projection onto the line's own pieces along
// the stepped path.
fn pick_area_candidate(
    segment_index: u32,
    ax: vec2<f32>,
    ay: vec2<f32>,
    bx: vec2<f32>,
    by: vec2<f32>,
) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    let area = pick_query_params.area;
    if segment_index >= area.y {
        return pick_invalid_candidate(series_order);
    }
    let t = pick_query_params.transform;
    let to_series = area.z != 0u;
    var base_a = vec2<f32>(0.0);
    var base_b = vec2<f32>(0.0);
    if to_series {
        base_a = pick_read_pair(area.x, segment_index);
        base_b = pick_read_pair(area.x, pick_next_point(segment_index));
    }
    let baseline = pick_query_params.area_baseline.xy;
    if !area_segment_drawn(t, ax, ay, bx, by, base_a, base_b, to_series, baseline)
        || pick_polar_clip_distance() > 0.0 {
        return pick_invalid_candidate(series_order);
    }
    let cursor = pick_query_params.cursor_chart.xy;
    let mode = pick_line_mode();
    let pieces = step_piece_count(mode);
    var coordinate_scale = 1.0;
    var total_length = 0.0;
    var found = false;
    var hit_prefix = 0.0;
    var hit_t = 0.0;
    var hit_length = 0.0;
    for (var k = 0u; k < pieces; k = k + 1u) {
        let piece = area_piece(ax, ay, bx, by, base_a, base_b, to_series, baseline, mode, k);
        let top_a = pick_ndc_to_canvas(area_corner_ndc(t, piece, false, false, to_series));
        let base_a_px = pick_ndc_to_canvas(area_corner_ndc(t, piece, false, true, to_series));
        let top_b = pick_ndc_to_canvas(area_corner_ndc(t, piece, true, false, to_series));
        let base_b_px = pick_ndc_to_canvas(area_corner_ndc(t, piece, true, true, to_series));
        let top = top_b - top_a;
        let length_sq = fma(top.x, top.x, top.y * top.y);
        let length = sqrt(length_sq);
        if !pick_is_finite(length) {
            return pick_invalid_candidate(series_order);
        }
        coordinate_scale = max(coordinate_scale, max(max(abs(top_a.x), abs(top_a.y)), max(abs(top_b.x), abs(top_b.y))));
        if !found && (pick_triangle_covers(cursor, top_a, base_a_px, top_b)
            || pick_triangle_covers(cursor, base_a_px, top_b, base_b_px)) {
            found = true;
            hit_prefix = total_length;
            hit_length = length;
            if length_sq > 0.0 {
                let delta = cursor - top_a;
                hit_t = clamp((delta.x * top.x + delta.y * top.y) / length_sq, 0.0, 1.0);
            }
        }
        total_length = total_length + length;
    }
    if !found {
        return pick_invalid_candidate(series_order);
    }
    var position = hit_t;
    if pieces > 1u && total_length > 0.0 {
        position = (hit_prefix + hit_t * hit_length) / total_length;
    }
    let point_index = pick_segment_endpoint(segment_index, position, coordinate_scale, total_length);
    return PickCandidate(
        1u,
        series_order,
        pick_source_row(point_index),
        PICK_KIND_FILL,
        pick_source_row(segment_index),
        0.0,
        0.0,
        0u,
    );
}
