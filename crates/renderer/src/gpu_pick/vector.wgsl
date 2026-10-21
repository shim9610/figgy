// Exact fragment: point vector arrows (SHADER_COMMON.md §5.8) as the
// scatter primitive of a vector series.

// ───── shared arrow glyph (SHADER_COMMON.md §5.8; `shader_consistency`
// compares these function bodies, structs and constants with the vector
// shader's glyph and projection fragments) ─────

const ARROW_PIVOT_TAIL: u32 = 0u;
const ARROW_PIVOT_MIDDLE: u32 = 1u;
const ARROW_PIVOT_TIP: u32 = 2u;
// A shaft shorter than this (pixels) is no shaft. The head takes the whole
// length of an arrow shorter than the head; the fragment stage recomputes
// that length from the pivot-shifted tail and tip, and the rounding must not
// turn the missing shaft into a round dot at the tail.
const ARROW_SHAFT_MIN_PX: f32 = 0.01;

/// Pixel-space arrow style. Sizes are already multiplied by the render scale.
struct ArrowStyle {
    width_px: f32,
    head_length_px: f32,
    head_width_px: f32,
    min_length_px: f32,
    pivot: u32,
};

/// An arrow placed on its anchor: tail and tip after the pivot shift, and the
/// head dimensions after scaling down for a short arrow.
struct ArrowPlacement {
    tail: vec2<f32>,
    tip: vec2<f32>,
    head_length_px: f32,
    head_width_px: f32,
    valid: bool,
};

fn arrow_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

/// Apply the pivot and the minimum length, and size the head to the arrow.
/// A zero-length arrow, one shorter than `min_length_px`, or one with a
/// non-finite end is not drawn.
fn arrow_place(tail: vec2<f32>, tip: vec2<f32>, style: ArrowStyle) -> ArrowPlacement {
    var out: ArrowPlacement;
    out.tail = tail;
    out.tip = tip;
    out.head_length_px = 0.0;
    out.head_width_px = 0.0;
    out.valid = false;
    let delta = tip - tail;
    let len = length(delta);
    if (!arrow_finite(len) || len <= 0.0 || len < style.min_length_px) {
        return out;
    }
    var shift = 0.0;
    if (style.pivot == ARROW_PIVOT_MIDDLE) {
        shift = -0.5;
    } else if (style.pivot == ARROW_PIVOT_TIP) {
        shift = -1.0;
    }
    out.tail = tail + delta * shift;
    out.tip = tip + delta * shift;
    let head_length = min(max(style.head_length_px, 0.0), len);
    out.head_length_px = head_length;
    if (style.head_length_px > 0.0) {
        out.head_width_px = max(style.head_width_px, 0.0) * (head_length / style.head_length_px);
    }
    out.valid = true;
    return out;
}

/// Unsigned distance from `p` to the segment `ab`; a zero-length segment is a
/// point.
fn arrow_sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    return length(pa - ba * h);
}

/// Signed distance from `p` to the triangle `p0 p1 p2` (negative inside).
fn arrow_sd_triangle(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    let e0 = p1 - p0;
    let e1 = p2 - p1;
    let e2 = p0 - p2;
    let v0 = p - p0;
    let v1 = p - p1;
    let v2 = p - p2;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / max(dot(e0, e0), 1e-12), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / max(dot(e1, e1), 1e-12), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / max(dot(e2, e2), 1e-12), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d = min(
        min(
            vec2<f32>(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x)),
            vec2<f32>(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x)),
        ),
        vec2<f32>(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x)),
    );
    return -sqrt(d.x) * sign(d.y);
}

/// Signed distance from pixel `p` to the placed arrow: the union of the
/// shaft (a segment `width_px` wide) and the head (a triangle). Negative
/// inside; `1e20` for an arrow with neither.
fn arrow_distance(p: vec2<f32>, placed: ArrowPlacement, width_px: f32) -> f32 {
    let delta = placed.tip - placed.tail;
    let len = length(delta);
    let dir = delta / max(len, 1e-12);
    let nrm = vec2<f32>(-dir.y, dir.x);
    let head_base = placed.tip - dir * placed.head_length_px;
    var distance = 1e20;
    // The shaft stops where the head begins. A shaft of zero length draws
    // nothing rather than a dot.
    if (len - placed.head_length_px > ARROW_SHAFT_MIN_PX && width_px > 0.0) {
        distance = min(distance, arrow_sd_segment(p, placed.tail, head_base) - width_px * 0.5);
    }
    if (placed.head_length_px > 0.0 && placed.head_width_px > 0.0) {
        let half = nrm * (placed.head_width_px * 0.5);
        distance = min(distance, arrow_sd_triangle(p, placed.tip, head_base + half, head_base - half));
    }
    return distance;
}

/// `pair + s` as a `(hi, lo)` pair: the error-free sum keeps the low part, so a
/// large-offset coordinate still resolves a small data-space step.
fn pair_add(pair: vec2<f32>, s: f32) -> vec2<f32> {
    let sum = fma(1.0, pair.x, s);
    let b_virtual = fma(1.0, sum, -pair.x);
    let a_virtual = fma(1.0, sum, -b_virtual);
    let b_roundoff = fma(1.0, s, -b_virtual);
    let a_roundoff = fma(1.0, pair.x, -a_virtual);
    return vec2<f32>(sum, fma(1.0, a_roundoff, b_roundoff) + pair.y);
}

/// Pixels per data unit along one axis whose range is `range` (the hi/lo
/// sum), at `pair`: the derivative of `axis_pair_to_t` mapped through NDC.
/// Exact on a linear axis; the local direction on a logarithmic one, the way
/// the contour labels orient themselves.
fn axis_px_per_unit_at(range: f32, panel_scale: f32, is_log: f32, pixel_to_ndc: f32, pair: vec2<f32>) -> f32 {
    let linear = 2.0 * panel_scale / (range * pixel_to_ndc);
    // d(log10 v)/dv = 1 / (v ln 10); `sample_valid` has already excluded v <= 0.
    let value = pair.x + pair.y;
    let log_slope = linear / (value * log(10.0));
    return mix(linear * (1.0 - is_log), log_slope, is_log);
}

// ───── end shared arrow glyph ─────

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
        0u,
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
