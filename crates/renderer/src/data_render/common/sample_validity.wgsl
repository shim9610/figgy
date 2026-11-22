// Mask source samples explicitly: NaN/Inf clipping is not portable in WGSL.
// Logarithmic coordinate domains exclude nonpositive samples, while bars and
// errorbar extents retain their separate baseline/clipping rules.
fn sample_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

// Ternary: every component finite and non-negative, their sum positive.
// Any other sample has no place in the triangle: it is not drawn and a line
// breaks there, so nothing needs a fragment clip.
fn ternary_sample_valid(xv: vec2<f32>, yv: vec2<f32>, cv: vec2<f32>) -> bool {
    let x = xv.x + xv.y;
    let y = yv.x + yv.y;
    let c = cv.x + cv.y;
    return sample_finite(xv.x) && sample_finite(xv.y)
        && sample_finite(yv.x) && sample_finite(yv.y)
        && sample_finite(cv.x) && sample_finite(cv.y)
        && sample_finite(x) && sample_finite(y) && sample_finite(c)
        && x >= 0.0 && y >= 0.0 && c >= 0.0 && x + y + c > 0.0;
}

fn sample_valid(t: Transform, xv: vec2<f32>, yv: vec2<f32>) -> bool {
    let x = xv.x + xv.y;
    let y = yv.x + yv.y;
    // Non-short-circuit `&` / `|`: every test is pure, so the result is
    // the `&&` chain's, but straight-line code. Software compilers
    // (SwiftShader) slow down sharply on the nested branches a long `&&`
    // chain becomes, once inlined into a picker loop (#363).
    var ok = sample_finite(xv.x) & sample_finite(xv.y);
    ok = ok & sample_finite(yv.x) & sample_finite(yv.y);
    ok = ok & sample_finite(x) & sample_finite(y);
    ok = ok & ((t.scale_log.x == 0.0) | (x > 0.0));
    ok = ok & ((t.scale_log.y == 0.0) | (y > 0.0));
    ok = ok & ((t.scale_prob.x < 0.5) | prob_in_domain(xv));
    ok = ok & ((t.scale_prob.y < 0.5) | prob_in_domain(yv));
    return ok & polar_sample_valid(t, xv, yv);
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

