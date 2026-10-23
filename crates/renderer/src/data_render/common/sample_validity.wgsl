// Mask source samples explicitly: NaN/Inf clipping is not portable in WGSL.
// Logarithmic coordinate domains exclude nonpositive samples, while bars and
// errorbar extents retain their separate baseline/clipping rules.
fn sample_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

// Polar: a radius below the axis start has no place on the plot (it would
// fold onto the inner ring), so the sample is not drawn and a line breaks
// there. Angles outside the sector and radii past the outer ring keep their
// projection and are cut by the fragment clip below.
fn polar_sample_valid(t: Transform, xv: vec2<f32>, yv: vec2<f32>) -> bool {
    if (t.polar_angle.x < 0.5) {
        return true;
    }
    let ty = axis_pair_to_t(yv, t.data_min.y, t.data_max.y, t.data_min_lo.y, t.data_max_lo.y, t.scale_log.y, t.data_to_panel_scale.y, t.data_to_panel_offset.y);
    let rt = (ty - t.data_to_panel_offset.y) / t.data_to_panel_scale.y;
    return rt >= 0.0;
}

fn sample_valid(t: Transform, xv: vec2<f32>, yv: vec2<f32>) -> bool {
    let x = xv.x + xv.y;
    let y = yv.x + yv.y;
    return sample_finite(xv.x) && sample_finite(xv.y)
        && sample_finite(yv.x) && sample_finite(yv.y)
        && sample_finite(x) && sample_finite(y)
        && (t.scale_log.x == 0.0 || x > 0.0)
        && (t.scale_log.y == 0.0 || y > 0.0)
        && polar_sample_valid(t, xv, yv);
}

// Polar clip. The transform is vertex-visible only, so the vertex stage hands
// the fragment stage the fragment's offset from the centre in pixels
// (interpolated from the corner's NDC) and the bounds as flat varyings; the
// fragment discards outside the outer ring, inside the hole and outside the
// sector, cutting a primitive at the edge the way the data-area scissor cuts
// a rectangular chart. A rectangular chart passes a zero flag.
fn polar_clip_offset(t: Transform, ndc: vec2<f32>) -> vec2<f32> {
    let center = (t.data_to_panel_offset + t.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    return (ndc - center) / t.pixel_to_ndc;
}

// (flag, outer radius px, inner radius px, sector flag)
fn polar_clip_limits(t: Transform) -> vec4<f32> {
    let p = t.polar_angle;
    let e = t.polar_extent;
    return vec4<f32>(p.x, e.w, e.w * p.w, select(0.0, 1.0, e.y > e.x));
}

// (zero angle rad, radians per angle unit signed by the direction, sector
// start, sector end in angle units)
fn polar_clip_sector(t: Transform) -> vec4<f32> {
    let p = t.polar_angle;
    let e = t.polar_extent;
    return vec4<f32>(p.y, p.z, e.x, e.y);
}

fn polar_fragment_visible(px: vec2<f32>, limits: vec4<f32>, sector: vec4<f32>) -> bool {
    if (limits.x < 0.5) {
        return true;
    }
    let r = length(px);
    if (r > limits.y + 0.5 || r < limits.z - 0.5) {
        return false;
    }
    if (limits.w < 0.5) {
        return true;
    }
    // The data angle of the fragment, brought into the turn that starts at
    // the sector start.
    let turn = 6.283185307179586 / abs(sector.y);
    var a = (atan2(px.y, px.x) - sector.x) / sector.y;
    a = a - floor((a - sector.z) / turn) * turn;
    return a <= sector.w + 1e-6 * turn;
}
