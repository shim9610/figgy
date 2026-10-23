/// The colour-map value as a ramp position, or a negative number when it
/// cannot be placed.
fn vector_ramp_t(zv: vec2<f32>) -> f32 {
    return ramp_t(zv, vector_params.z_min, vector_params.z_max, vector_flag(VECTOR_FLAG_LOG_Z));
}

/// The colourbar's ramp at `t`, through the shared `ramp_segment` /
/// `ramp_blend`; only the stop lookup is this fragment's.
fn vector_ramp(t: f32) -> vec4<f32> {
    let n = vector_params.stop_count;
    if (n == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    if (n == 1u) {
        return vector_stops[0];
    }
    let segment = ramp_segment(t, n);
    let index = u32(segment.x);
    return ramp_blend(vector_stops[index], vector_stops[index + 1u], segment.y);
}

/// Premultiplied arrow colour: the series colour, or the colour-map value
/// through the colourbar's ramp (`nan_color` when it cannot be placed).
fn vector_color(sample: VectorSample) -> vec4<f32> {
    if (!vector_flag(VECTOR_FLAG_COLOR_MAP)) {
        return style.color_premul;
    }
    let t = vector_ramp_t(sample.value);
    if (!sample.value_valid || t < 0.0) {
        return vector_params.nan_color_premul;
    }
    let c = vector_ramp(t);
    return vec4<f32>(c.rgb * c.a, c.a);
}
