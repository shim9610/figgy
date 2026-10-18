// Input layer, points form: `u` / `v` (and the colour value) are index-aligned
// columns, read at the instance index like the anchor columns.

/// The arrow sample for instance `i`, or an invalid sample past `count` or
/// where a component is not finite. A zero vector is a valid sample that
/// draws nothing.
fn vector_sample(i: u32) -> VectorSample {
    var sample: VectorSample;
    sample.origin_hi = vec3<f32>(0.0, 0.0, 0.0);
    sample.origin_lo = vec3<f32>(0.0, 0.0, 0.0);
    sample.vector = vec3<f32>(0.0, 0.0, 0.0);
    sample.value = vec2<f32>(0.0, 0.0);
    sample.valid = false;
    if (i >= vector_params.count) {
        return sample;
    }
    let x = vector_pool_pair(vector_params.x_base, i);
    let y = vector_pool_pair(vector_params.y_base, i);
    let u = vector_pool_pair(vector_params.u_base, i);
    let v = vector_pool_pair(vector_params.v_base, i);
    if (!vector_pair_finite(u) || !vector_pair_finite(v)) {
        return sample;
    }
    sample.origin_hi = vec3<f32>(x.x, y.x, 0.0);
    sample.origin_lo = vec3<f32>(x.y, y.y, 0.0);
    sample.vector = vec3<f32>(u.x + u.y, v.x + v.y, 0.0);
    if (vector_flag(VECTOR_FLAG_COLOR_MAP)) {
        sample.value = vector_pool_pair(vector_params.c_base, i);
    }
    sample.valid = true;
    return sample;
}
