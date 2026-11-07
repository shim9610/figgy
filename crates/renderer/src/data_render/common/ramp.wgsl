/// A finite f32 by its exponent bits. Every ramp input is checked
/// explicitly: NaN and infinity propagation through arithmetic is not
/// portable in WGSL.
fn ramp_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

/// `zv` (a hi/lo pair) as a ramp position in `[0, 1]`, or a negative number
/// when it cannot be placed: a non-finite half or sum, a non-positive value
/// on a logarithmic bar, or a degenerate range. `z_min` / `z_max` are the
/// colourbar bounds as hi/lo pairs, already log10 when `log_z`.
///
/// CPU twin: `ColorBarOptions::normalized_z`. "Unplaceable" and "smallest"
/// are different facts, so the miss is reported instead of clamping to an
/// end.
fn ramp_t(zv: vec2<f32>, z_min: vec2<f32>, z_max: vec2<f32>, log_z: bool) -> f32 {
    // Both halves are checked before they are added.
    if (!ramp_finite(zv.x) || !ramp_finite(zv.y)) {
        return -1.0;
    }
    let raw = zv.x + zv.y;
    if (!ramp_finite(raw)) {
        return -1.0;
    }
    if (log_z && raw <= 0.0) {
        return -1.0;
    }
    let is_log = select(0.0, 1.0, log_z);
    // Same shape as `axis_pair_to_t`: the linear form keeps the hi/lo split so
    // a large absolute z still resolves small deltas.
    let linear_num = (zv.x - z_min.x) + (zv.y - z_min.y);
    let log_num = (maybe_log(raw, 1.0) - z_min.x) - z_min.y;
    let span = (z_max.x - z_min.x) + (z_max.y - z_min.y);
    if (!ramp_finite(linear_num) || !ramp_finite(log_num)
        || !ramp_finite(span) || span <= 0.0) {
        return -1.0;
    }
    var numerator = mix(linear_num, log_num, is_log);
    if is_log > 0.5 {
        numerator = log_num;
    }
    let normalized = numerator / span;
    if (!ramp_finite(numerator) || !ramp_finite(normalized)) {
        return -1.0;
    }
    return clamp(normalized, 0.0, 1.0);
}

/// Where `t` falls among `n >= 2` control points: `x` is the lower stop's
/// index as a float, `y` the blend towards the next. Twin of the index and
/// fraction `model::colormap::sample` computes, including the `n - 2` index
/// clamp that lets the last segment own `t = 1`.
fn ramp_segment(t: f32, n: u32) -> vec2<f32> {
    let tc = select(clamp(t, 0.0, 1.0), 0.0, !ramp_finite(t));
    let scaled = tc * f32(n - 1u);
    let index = min(u32(floor(scaled)), n - 2u);
    return vec2<f32>(f32(index), scaled - f32(index));
}

/// The control points `a` (the segment's lower stop) and `b` blended by
/// `frac`, with the exact-endpoint short circuits `colormap::sample` has.
/// `mix` is not used: `a + (b - a) * t` is what the CPU computes, and the two
/// paths have to agree at the ends.
fn ramp_blend(a: vec4<f32>, b: vec4<f32>, frac: f32) -> vec4<f32> {
    if (frac <= 0.0) {
        return a;
    }
    if (frac >= 1.0) {
        return b;
    }
    return a + (b - a) * frac;
}
