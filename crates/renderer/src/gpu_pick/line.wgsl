// Exact fragment: line strokes (SHADER_COMMON.md §5.6), their step pieces,
// dash phase and styled extrusion, and the segment candidate that measures
// them with the line's fill (`gpu_pick/area.wgsl`).

// The mode word of `step_piece_count` (common/line_step.wgsl): bits 0-1 the
// step mode, bits 4 and up the polar arc pieces per segment
// (`transform.polar_extent.z`), as `line_step_mode` in line_columnar.wgsl
// builds it.
fn pick_line_mode() -> u32 {
    let arc = u32(max(pick_query_params.transform.polar_extent.z, 0.0));
    return (pick_query_params.line.x & LINE_STEP_MASK) | (arc << LINE_ARC_SHIFT);
}

// The arc distances from `dist_px`, which lies in an "off" span of the dash
// pattern, back to the end of the previous "on" span (x) and forward to the
// start of the next one (y). The span walk is `dash_pattern_on`'s.
fn pick_dash_gap(dist_px: f32) -> vec2<f32> {
    var spans = pick_query_params.line_dash;
    let n = min(pick_query_params.styled.w, 8u);
    var period = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        period = period + max(spans[i / 4u][i % 4u], 0.0);
    }
    let phase = ((dist_px % period) + period) % period;
    var acc = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        let start = acc;
        let end = acc + max(spans[i / 4u][i % 4u], 0.0);
        acc = end;
        if phase <= end || i == n - 1u {
            return vec2<f32>(phase - start, end - phase);
        }
    }
    return vec2<f32>(0.0, 0.0);
}

// The hit distance of one drawn piece of a stroke: the line shader's own
// extrusion (square caps, `max(width, 1)`, the sketch displacement, the dash
// phase) inverted at the cursor. `a_up` / `b_up` are the piece's ends in the
// draw's y-up pixel frame, `origin` the segment's arc pair at A and
// `arc_start` / `arc_end` the piece's arc offsets from A the vertex stage
// gives them (SHADER_COMMON.md §5.6 arc pairs; `has_arc` is false when the
// arc prefix is not bound: a streamed series). Zero exactly where `line_stroke_distance` is
// non-positive and the dash is on, which is where the fragment's coverage is
// at least one half.
fn pick_stroke_piece_distance(a_up: vec2<f32>, b_up: vec2<f32>, origin: vec2<u32>, arc_start: f32, arc_end: f32, has_arc: bool) -> f32 {
    let delta = b_up - a_up;
    let len = length(delta);
    // The vertex stage collapses a zero-length piece to a point.
    if !(len > 1e-6) {
        return PICK_F32_MAX;
    }
    let dir = delta / len;
    let normal = vec2<f32>(-dir.y, dir.x);
    let rel = pick_flip(pick_query_params.cursor_chart.xy) - a_up;
    let along = dot(rel, dir);
    var side = dot(rel, normal);
    let mode = pick_query_params.styled.y;
    // A Constellation line drawn at zero opacity paints nothing (its
    // markers are stars, so the marker mode says which style this is).
    if pick_query_params.styled.x == PICK_MARKER_STAR && !(pick_query_params.transform.style_params[0].y > 0.0) {
        return PICK_F32_MAX;
    }
    let styled = pick_variant(PICK_VARIANT_STYLED_LINE);
    if styled && mode == PICK_LINE_RIBBON {
        // The ribbon strip spans one full ribbon width either side with
        // square caps of the same size; the stars scatter within the
        // envelope the CPU folds into the same half extent.
        return max(line_stroke_distance(along, side, len, pick_query_params.line_px.y), 0.0);
    }
    let half_w = pick_query_params.line_px.x;
    let dash_count = select(0u, pick_query_params.styled.w, pick_variant(PICK_VARIANT_DASH));
    let geom_half_w = half_w + select(LINE_AA_EXTENT_PX, 0.0, dash_count > 0u);
    // A's arc modulo the dash period, where the draw's `dist_px` starts.
    let phase = arc_dash_phase(origin, dash_period(pick_query_params.line_dash, dash_count));
    // The arc position at the cursor (the interpolated `dist_px`) and its
    // rate per pixel along the piece.
    var dist_px = 0.0;
    var dist_slope = 1.0;
    if styled && mode == PICK_LINE_SKETCH {
        if has_arc {
            // The strip's knots k = 0..S sit at `t_k * len` along the piece,
            // the outer two pushed out by the square cap, each displaced
            // along the normal by the wobble at its arc position.
            let s = f32(SKETCH_SUBDIV);
            let k = u32(clamp(floor(along / len * s), 0.0, s - 1.0));
            let t0 = f32(k) / s;
            let t1 = f32(k + 1u) / s;
            let a0 = select(t0 * len, -geom_half_w, k == 0u);
            let a1 = select(t1 * len, len + geom_half_w, k + 1u == SKETCH_SUBDIV);
            // Unclamped: past the strip's ends a multisampled fragment
            // extrapolates the end span's attributes to the pixel centre,
            // and the pick follows it (SHADER_COMMON.md §5.6, #114).
            let w = (along - a0) / (a1 - a0);
            let style_params = pick_query_params.transform.style_params[0];
            let seed = u32(style_params.z) ^ pick_query_params.styled.z;
            let arc0 = mix(arc_start, arc_end, t0);
            let arc1 = mix(arc_start, arc_end, t1);
            let d0 = sketch_line_offset_from(origin, arc0, style_params.x, style_params.y, seed);
            let d1 = sketch_line_offset_from(origin, arc1, style_params.x, style_params.y, seed);
            side = side - mix(d0, d1, w);
            let dist0 = phase + arc0 + select(0.0, -half_w, k == 0u);
            let dist1 = phase + arc1 + select(0.0, half_w, k + 1u == SKETCH_SUBDIV);
            dist_px = mix(dist0, dist1, w);
            dist_slope = (dist1 - dist0) / (a1 - a0);
        } else {
            // Without the arc prefix the wobble is unknown: the envelope of
            // every displacement it can take.
            side = max(abs(side) - max(pick_query_params.transform.style_params[0].x, 0.0), 0.0);
        }
    } else {
        // `vs_main` puts `arc_start - half_w` on the A-side corners and
        // `arc_end + half_w` on the B-side ones.
        let span = len + 2.0 * geom_half_w;
        dist_slope = (arc_end - arc_start + 2.0 * half_w) / span;
        dist_px = (phase + arc_start - half_w) + (along + geom_half_w) * dist_slope;
    }
    let stroke = max(line_stroke_distance(along, side, len, half_w), 0.0);
    if !pick_variant(PICK_VARIANT_DASH) || dash_count == 0u || !has_arc
        || dash_pattern_on(dist_px, pick_query_params.line_dash, dash_count) {
        return stroke;
    }
    // In a gap: the nearest dash of this piece on either side, along the
    // stroke, clipped to the stroke's rectangle.
    let gap = pick_dash_gap(dist_px) / max(abs(dist_slope), 1e-6);
    let across = max(abs(side) - half_w, 0.0);
    var best = PICK_F32_MAX;
    let before = along - gap.x;
    if before >= -half_w {
        let dx = along - min(before, len + half_w);
        best = min(best, sqrt(dx * dx + across * across));
    }
    let after = along + gap.y;
    if after <= len + half_w {
        let dx = max(after, -half_w) - along;
        best = min(best, sqrt(dx * dx + across * across));
    }
    return max(best, PICK_F32_EPSILON);
}

// The source point a hit on segment `segment_index` names, from `position`
// along its stepped path (0 at A, 1 at B): the nearer endpoint, B being the
// first point on a closed line's closing segment (`pick_next_point`).
// Projected endpoints are f32. Reciprocal/fused evaluation can put an exact
// midpoint a rounding step on either side of 0.5 across backends. Resolve
// that uncertainty by the same lower-index rule as the CPU picker: toward A.
fn pick_segment_endpoint(segment_index: u32, position: f32, coordinate_scale: f32, total_length: f32) -> u32 {
    let midpoint_tolerance = min(PICK_F32_EPSILON * coordinate_scale / total_length, 0.25);
    let use_b = position > 0.5 + midpoint_tolerance;
    return select(segment_index, pick_next_point(segment_index), use_b);
}

fn pick_stroke_candidate(
    segment_index: u32,
    ax: vec2<f32>,
    ay: vec2<f32>,
    bx: vec2<f32>,
    by: vec2<f32>,
) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    if !pick_coordinate_is_valid(ax, 0u) || !pick_coordinate_is_valid(ay, 1u) || !pick_coordinate_is_valid(bx, 0u) || !pick_coordinate_is_valid(by, 1u) {
        return pick_invalid_candidate(series_order);
    }
    let t = pick_query_params.transform;
    if !polar_sample_valid(t, ax, ay) || !polar_sample_valid(t, bx, by) {
        return pick_invalid_candidate(series_order);
    }
    // A step mode tests every axis-aligned piece of the segment and keeps the
    // nearest (lowest piece index on a tie); a linear segment is its single
    // piece and reproduces the straight-segment rule exactly.
    let mode = pick_line_mode();
    let pieces = step_piece_count(mode);
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    // The cumulative arc prefix the line was drawn with, when bound: the
    // dash phase and the sketch wobble walk it.
    let has_arc = pick_variant(PICK_VARIANT_DASH | PICK_VARIANT_STYLED_LINE) && pick_query_params.line.y != 0u;
    var arc_a = vec2<u32>(0u, 0u);
    var arc_b = vec2<u32>(0u, 0u);
    if has_arc {
        // A closed line is solid and binds no arc prefix; were one bound,
        // its closing segment would take the last point's arc at both ends
        // as the draw does.
        arc_a = pick_arc[segment_index];
        arc_b = select(arc_a, pick_arc[segment_index + 1u], segment_index + 1u < pick_query_params.data.x);
    }
    var coordinate_scale = 1.0;
    var total_length = 0.0;
    var prefix_length = 0.0;
    var best_distance = PICK_F32_MAX;
    var best_found = false;
    var best_t = 0.0;
    var best_prefix = 0.0;
    var best_length = 0.0;
    for (var k = 0u; k < pieces; k = k + 1u) {
        let piece = step_piece(ax, ay, bx, by, mode, k);
        let a_px = pick_project_pair(piece.x0, piece.y0);
        let b_px = pick_project_pair(piece.x1, piece.y1);
        if !pick_is_finite(a_px.x) || !pick_is_finite(a_px.y) || !pick_is_finite(b_px.x) || !pick_is_finite(b_px.y) {
            return pick_invalid_candidate(series_order);
        }
        coordinate_scale = max(coordinate_scale, max(max(abs(a_px.x), abs(a_px.y)), max(abs(b_px.x), abs(b_px.y))));
        let segment = b_px - a_px;
        let length_sq = fma(segment.x, segment.x, segment.y * segment.y);
        if !pick_is_finite(length_sq) {
            return pick_invalid_candidate(series_order);
        }
        let length = sqrt(length_sq);
        // The draw's arc offsets from A for this piece: it starts after the
        // pieces before it and the last piece ends at the scanned prefix of B.
        let piece_prefix = prefix_length;
        let arc_start = piece_prefix;
        let arc_end = select(arc_start + length, arc_delta(arc_b, arc_a), k + 1u == pieces);
        prefix_length = prefix_length + length;
        total_length = total_length + length;
        // A zero-length piece (a riser between equal values, or a degenerate
        // linear segment) has no stroke to hit.
        if !(length > 1e-6) {
            continue;
        }
        let distance = pick_stroke_piece_distance(pick_flip(a_px), pick_flip(b_px), arc_a, arc_start, arc_end, has_arc);
        if pick_is_finite(distance) && distance < best_distance {
            let cursor_delta = pick_query_params.cursor_chart.xy - a_px;
            best_distance = distance;
            best_found = true;
            best_t = clamp((cursor_delta.x * segment.x + cursor_delta.y * segment.y) / length_sq, 0.0, 1.0);
            best_prefix = piece_prefix;
            best_length = length;
        }
    }
    if !best_found {
        return pick_invalid_candidate(series_order);
    }
    // The polar fragment clip cuts the stroke like it cuts a marker.
    let distance = max(best_distance, pick_polar_clip_distance());
    let distance_sq = distance * distance;
    if !pick_is_finite(distance_sq) || distance_sq > maximum_sq {
        return pick_invalid_candidate(series_order);
    }
    // Where along the whole stepped path the hit lies, 0 at A and 1 at B. A
    // linear segment keeps t itself: `(t · len) / len` is not bit-exact.
    var position = best_t;
    if pieces > 1u {
        position = (best_prefix + best_t * best_length) / total_length;
    }
    let point_index = pick_segment_endpoint(segment_index, position, coordinate_scale, total_length);
    return PickCandidate(
        1u,
        series_order,
        pick_source_row(point_index),
        PICK_KIND_LINE,
        pick_source_row(segment_index),
        distance_sq,
        distance,
        0u,
    );
}

// The arrowheads a streamline series draws on segment `segment_index`
// (SHADER_COMMON.md §5.3 path form, §5.8): the slots nearest the cursor's
// arc position on the segment, each placed by the draw's
// `path_arrow_on_segment`, projected in pixel mode one head long along the
// tangent through the axes' pixel-per-unit Jacobian and measured by the
// glyph's `arrow_place` / `arrow_distance`. A hit is the segment's line
// candidate naming its nearer endpoint, as the stroke's does; it is drawn
// under the stroke, so it only wins where the stroke is farther.
fn pick_path_arrow_candidate(
    segment_index: u32,
    ax: vec2<f32>,
    ay: vec2<f32>,
    bx: vec2<f32>,
    by: vec2<f32>,
) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    let spacing = pick_query_params.vector_min.y;
    let next = segment_index + 1u;
    if !(spacing > 0.0) || pick_query_params.line.y == 0u || next >= pick_query_params.data.x {
        return pick_invalid_candidate(series_order);
    }
    if !pick_coordinate_is_valid(ax, 0u) || !pick_coordinate_is_valid(ay, 1u) || !pick_coordinate_is_valid(bx, 0u) || !pick_coordinate_is_valid(by, 1u) {
        return pick_invalid_candidate(series_order);
    }
    let arc_a = pick_arc[segment_index];
    let arc_len = arc_delta(pick_arc[next], arc_a);
    if !(arc_len > 0.0) {
        return pick_invalid_candidate(series_order);
    }
    let a_px = pick_project_pair(ax, ay);
    let b_px = pick_project_pair(bx, by);
    let segment = b_px - a_px;
    let length_sq = dot(segment, segment);
    if !pick_is_finite(length_sq) || !(length_sq > 0.0) {
        return pick_invalid_candidate(series_order);
    }
    let cursor = pick_query_params.cursor_chart.xy;
    let position = clamp(dot(cursor - a_px, segment) / length_sq, 0.0, 1.0);
    // The cursor's arc pair on the segment and the slot it falls in.
    let cursor_arc = arc_add(arc_a, arc_from_px(position * arc_len));
    let px = pick_query_params.vector_px;
    let tr = pick_query_params.transform;
    let range_x = fma(1.0, tr.data_max.x - tr.data_min.x, tr.data_max_lo.x - tr.data_min_lo.x);
    let range_y = fma(1.0, tr.data_max.y - tr.data_min.y, tr.data_max_lo.y - tr.data_min_lo.y);
    let nearest = arc_cell(cursor_arc, spacing).index;
    var best = PICK_F32_MAX;
    for (var k = 0u; k < 3u; k = k + 1u) {
        if nearest + k < 1u {
            continue;
        }
        let slot = nearest + k - 1u;
        let slot_arc = arc_delta(arc_slot_position(slot, 0.5, spacing), arc_a);
        let head = path_arrow_on_segment(ax, ay, bx, by, 0.0, arc_len, slot_arc);
        if !head.valid {
            continue;
        }
        // `project_arrow`'s pixel mode at the head's point, in this
        // shader's y-up frame.
        let tail = pick_flip(pick_project_pair(head.x, head.y));
        let direction = vec2<f32>(
            axis_px_per_unit_at(range_x, tr.data_to_panel_scale.x, tr.scale_log.x, tr.pixel_to_ndc.x, head.x) * head.tangent.x,
            axis_px_per_unit_at(range_y, tr.data_to_panel_scale.y, tr.scale_log.y, tr.pixel_to_ndc.y, head.y) * head.tangent.y,
        );
        let direction_len = length(direction);
        let len_px = px.x * length(head.tangent);
        if !pick_is_finite(tail.x) || !pick_is_finite(tail.y) || !pick_is_finite(direction_len) || !(direction_len > 0.0) || !pick_is_finite(len_px) {
            continue;
        }
        let tip = tail + direction * (len_px / direction_len);
        let placed = arrow_place(tail, tip, ArrowStyle(0.0, px.z, px.w, 0.0, ARROW_PIVOT_MIDDLE));
        if !placed.valid {
            continue;
        }
        best = min(best, max(arrow_distance(pick_flip(cursor), placed, 0.0), 0.0));
    }
    let distance = max(best, pick_polar_clip_distance());
    let distance_sq = distance * distance;
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    if !pick_is_finite(distance_sq) || distance_sq > maximum_sq {
        return pick_invalid_candidate(series_order);
    }
    let coordinate_scale = max(max(max(abs(a_px.x), abs(a_px.y)), max(abs(b_px.x), abs(b_px.y))), 1.0);
    let point_index = pick_segment_endpoint(segment_index, position, coordinate_scale, sqrt(length_sq));
    return PickCandidate(
        1u,
        series_order,
        pick_source_row(point_index),
        PICK_KIND_LINE,
        pick_source_row(segment_index),
        distance_sq,
        distance,
        0u,
    );
}

// A line series' segment primitive: the stroke and, when the line has a fill
// (`PICK_VARIANT_AREA`, a non-zero filled segment count), the fill under the
// segment, and a streamline series' arrowheads on it
// (`PICK_VARIANT_PATH_ARROWS`, a non-zero spacing). The fill only picks
// inside it, at distance zero, and loses every tie
// (`pick_candidate_is_better`), so a stroke over it keeps picking; an
// arrowhead is the stroke's kind and segment, so it wins only where it is
// nearer than the stroke.
fn pick_line_candidate(
    segment_index: u32,
    ax: vec2<f32>,
    ay: vec2<f32>,
    bx: vec2<f32>,
    by: vec2<f32>,
) -> PickCandidate {
    var best = pick_stroke_candidate(segment_index, ax, ay, bx, by);
    if pick_variant(PICK_VARIANT_PATH_ARROWS) && pick_query_params.vector_min.y > 0.0 {
        let heads = pick_path_arrow_candidate(segment_index, ax, ay, bx, by);
        if pick_candidate_is_better(heads, best) {
            best = heads;
        }
    }
    if !pick_variant(PICK_VARIANT_AREA) || pick_query_params.area.y == 0u {
        return best;
    }
    let fill = pick_area_candidate(segment_index, ax, ay, bx, by);
    if pick_candidate_is_better(fill, best) {
        return fill;
    }
    return best;
}
