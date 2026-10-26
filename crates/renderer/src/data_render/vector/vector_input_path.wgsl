// Input layer, path form: the arrowheads of a streamline series.
//
// Instance `i` is the slot at arc `(i + 0.5) * spacing` along the series'
// whole drawn path (the arc prefix of `line_arc.wgsl`, render pixels), the
// slot count the scan's `star_indirect` kernel writes into the indirect
// draw. `arc_walk_segment` finds the segment holding the slot (group 3,
// `common/arc_walk.wgsl`), `path_arrow_on_segment` (`common/path_arrow.wgsl`)
// its point and unit data-space tangent, and the colour value is
// interpolated between the segment's two samples the way the stroke's
// fragment interpolates it. The projection layer then runs in pixel mode
// with `scale = head length`, so the glyph's arrow is one head long and
// centred on its point (`pivot` middle): a head with no shaft.

/// The arrowhead sample for slot `i`, or an invalid sample for a slot past
/// the drawn arc, on a NaN gap or on a segment with an invalid sample.
fn vector_sample(i: u32) -> VectorSample {
    var sample: VectorSample;
    sample.origin_hi = vec3<f32>(0.0, 0.0, 0.0);
    sample.origin_lo = vec3<f32>(0.0, 0.0, 0.0);
    sample.vector = vec3<f32>(0.0, 0.0, 0.0);
    sample.value = vec2<f32>(0.0, 0.0);
    sample.value_valid = false;
    sample.valid = false;
    let n = star_vsp.n_points;
    let spacing = vector_params.spacing_px;
    if (n < 2u || !(spacing > 0.0)) {
        return sample;
    }
    let arc = (f32(i) + 0.5) * spacing;
    if (!(arc < star_arc[n - 1u])) {
        return sample;
    }
    let a = arc_walk_segment(n, arc);
    let b = a + 1u;
    // The coordinates and the colour column bound the drawn segments.
    if (b >= vector_params.count) {
        return sample;
    }
    let xa = vector_pool_pair(vector_params.x_base, a);
    let ya = vector_pool_pair(vector_params.y_base, a);
    let xb = vector_pool_pair(vector_params.x_base, b);
    let yb = vector_pool_pair(vector_params.y_base, b);
    if (!sample_valid(transform, xa, ya) || !sample_valid(transform, xb, yb)) {
        return sample;
    }
    let head = path_arrow_on_segment(xa, ya, xb, yb, star_arc[a], star_arc[b], arc);
    if (!head.valid) {
        return sample;
    }
    sample.origin_hi = vec3<f32>(head.x.x, head.y.x, 0.0);
    sample.origin_lo = vec3<f32>(head.x.y, head.y.y, 0.0);
    sample.vector = vec3<f32>(head.tangent, 0.0);
    if (vector_flag(VECTOR_FLAG_COLOR_MAP)) {
        let ca = vector_pool_pair(vector_params.c_base, a);
        let cb = vector_pool_pair(vector_params.c_base, b);
        sample.value = ca + (cb - ca) * head.t;
        sample.value_valid = true;
    }
    sample.valid = true;
    return sample;
}
