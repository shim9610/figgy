// ───── end shared line stroke ─────

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
// draw's y-up pixel frame and `arc_start` / `arc_end` the arc positions the
// vertex stage gives them (`has_arc` is false when the arc prefix is not
// bound: a streamed series). Zero exactly where `line_stroke_distance` is
// non-positive and the dash is on, which is where the fragment's coverage is
// at least one half.
fn pick_stroke_piece_distance(a_up: vec2<f32>, b_up: vec2<f32>, arc_start: f32, arc_end: f32, has_arc: bool) -> f32 {
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
            let w = clamp((along - a0) / (a1 - a0), 0.0, 1.0);
            let style_params = pick_query_params.transform.style_params[0];
            let seed = u32(style_params.z) ^ pick_query_params.styled.z;
            let arc0 = mix(arc_start, arc_end, t0);
            let arc1 = mix(arc_start, arc_end, t1);
            let d0 = sketch_line_offset(arc0, style_params.x, style_params.y, seed);
            let d1 = sketch_line_offset(arc1, style_params.x, style_params.y, seed);
            side = side - mix(d0, d1, w);
            let dist0 = arc0 + select(0.0, -half_w, k == 0u);
            let dist1 = arc1 + select(0.0, half_w, k + 1u == SKETCH_SUBDIV);
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
        dist_px = (arc_start - half_w) + (along + geom_half_w) * dist_slope;
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

fn pick_line_candidate(
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
    if !pick_polar_valid(ax, ay) || !pick_polar_valid(bx, by) {
        return pick_invalid_candidate(series_order);
    }
    // A step mode tests every axis-aligned piece of the segment and keeps the
    // nearest (lowest piece index on a tie); a linear segment is its single
    // piece and reproduces the straight-segment rule exactly.
    let mode = pick_line_mode();
    let pieces = pick_step_piece_count(mode);
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    // The cumulative arc prefix the line was drawn with, when bound: the
    // dash phase and the sketch wobble walk it.
    let has_arc = pick_variant(PICK_VARIANT_DASH | PICK_VARIANT_STYLED_LINE) && pick_query_params.line.y != 0u;
    var arc_a = 0.0;
    var arc_b = 0.0;
    if has_arc {
        arc_a = pick_arc[segment_index];
        arc_b = pick_arc[segment_index + 1u];
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
        let piece = pick_step_piece(ax, ay, bx, by, mode, k);
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
        // The draw's arc positions for this piece: it starts at the
        // segment's arc plus the pieces before it and the last piece ends at
        // the scanned prefix of B.
        let piece_prefix = prefix_length;
        let arc_start = arc_a + piece_prefix;
        let arc_end = select(arc_start + length, arc_b, k + 1u == pieces);
        prefix_length = prefix_length + length;
        total_length = total_length + length;
        // A zero-length piece (a riser between equal values, or a degenerate
        // linear segment) has no stroke to hit.
        if !(length > 1e-6) {
            continue;
        }
        let distance = pick_stroke_piece_distance(pick_flip(a_px), pick_flip(b_px), arc_start, arc_end, has_arc);
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
    // Projected endpoints are f32. Reciprocal/fused evaluation can put an
    // exact midpoint a rounding step on either side of 0.5 across backends.
    // Resolve that uncertainty by the same lower-index rule as the CPU picker.
    let midpoint_tolerance = min(PICK_F32_EPSILON * coordinate_scale / total_length, 0.25);
    let use_b = position > 0.5 + midpoint_tolerance;
    let point_index = select(segment_index, segment_index + 1u, use_b);
    return PickCandidate(
        1u,
        series_order,
        pick_source_row(point_index),
        1u,
        pick_source_row(segment_index),
        distance_sq,
        distance,
        0u,
    );
}
