fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    if is_log > 0.5 {
        return lv;
    }
    return mix(v, lv, is_log);
}

// The raw data t of `v` on an axis: its fraction of the range, in decades
// on a logarithmic one, before any cut remap. `prob` is the axis'
// probability code (`Transform.scale_prob`, #209): on a probability axis
// `min` / `max` hold `F⁻¹` of the bounds and the sample is placed at
// `prob_forward`, before the linear and log arithmetic, which stays as it
// was.
fn axis_data_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, prob: f32) -> f32 {
    let range = fma(1.0, max_hi - min_hi, max_lo - min_lo);
    if (prob > 0.5) {
        let prob_num = (prob_forward(v, prob) - min_hi) - min_lo;
        return prob_num / range;
    }
    let raw = v.x + v.y;
    let linear_num = fma(1.0, v.x - min_hi, v.y - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    if is_log > 0.5 {
        return log_num / range;
    }
    return mix(linear_num / range, log_num / range, is_log);
}

fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, prob: f32, panel_scale: f32, panel_offset: f32, cuts: AxisCuts) -> f32 {
    let data_t = axis_data_t(v, min_hi, max_hi, min_lo, max_lo, is_log, prob);
    return fma(cut_remap(cuts, data_t), panel_scale, panel_offset);
}

// Polar (Config.polar): the angle column in radians around the centre of the
// data area, the radius axis fraction between the inner ring and the outer
// radius. A radius below the axis start sits on the inner ring (data samples
// there are masked by `sample_valid`; a fill baseline is not). CPU twin:
// `cpu_twin::ndc`, test support.
fn polar_ndc(t: Transform, ty: f32, xv: vec2<f32>) -> vec2<f32> {
    let p = t.polar_angle;
    let e = t.polar_extent;
    let angle = p.y + p.z * (xv.x + xv.y);
    let rt = (ty - t.data_to_panel_offset.y) / t.data_to_panel_scale.y;
    let r = mix(p.w, 1.0, max(rt, 0.0));
    let center = (t.data_to_panel_offset + t.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    return center + vec2<f32>(cos(angle), sin(angle)) * r * e.w * t.pixel_to_ndc;
}

fn data_to_ndc(t: Transform, xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    let tx = axis_pair_to_t(xv, t.data_min.x, t.data_max.x, t.data_min_lo.x, t.data_max_lo.x, t.scale_log.x, t.scale_prob.x, t.data_to_panel_scale.x, t.data_to_panel_offset.x, cuts_of(t.breaks, t.break_info, 0u));
    let ty = axis_pair_to_t(yv, t.data_min.y, t.data_max.y, t.data_min_lo.y, t.data_max_lo.y, t.scale_log.y, t.scale_prob.y, t.data_to_panel_scale.y, t.data_to_panel_offset.y, cuts_of(t.breaks, t.break_info, 1u));
    if (t.polar_angle.x > 0.5) {
        return polar_ndc(t, ty, xv);
    }
    return vec2<f32>(tx, ty) * 2.0 - 1.0;
}

// Ternary (Config.ternary): a sample of three components placed at the
// triangle's corners weighted by them, `(x·L + y·T + c·R) / (x + y + c)`,
// the corners in NDC (`Transform.ternary_lt`, `ternary_r`). Normalizing only
// to place the sample is appearance (CLAUDE.md rule 1); nothing is stored.
// A projective map keeps lines straight, so a segment is its chord. Valid
// samples only (`ternary_sample_valid`). CPU twin: none; the pick splices
// this function.
fn ternary_ndc(t: Transform, xv: vec2<f32>, yv: vec2<f32>, cv: vec2<f32>) -> vec2<f32> {
    let x = xv.x + xv.y;
    let y = yv.x + yv.y;
    let c = cv.x + cv.y;
    return (t.ternary_lt.xy * x + t.ternary_lt.zw * y + t.ternary_r.xy * c) / (x + y + c);
}

// A baseline on axis `axis` (0 = x, 1 = y) in NDC: an area fill's base or a
// stem's start (#206). At or below zero on a logarithmic axis it has no
// position; masking it would delete the whole fill or stem, so it is
// clamped to the axis minimum (data_t = 0). This is the documented
// exception to the `sample_valid` rule. The other coordinate is a dummy 1.
fn axis_baseline_ndc(t: Transform, base: vec2<f32>, axis: u32) -> f32 {
    let value = base.x + base.y;
    let one = vec2<f32>(1.0, 0.0);
    // A probability axis (#209): a baseline at or past 0 or 1 has no
    // position either and is clamped to the axis end on its side (p ≤ 0 the
    // low end, p ≥ 1 the high end, which an inverted axis swaps).
    if (t.scale_prob[axis] > 0.5 && !prob_in_domain(base)) {
        let range = (t.data_max[axis] - t.data_min[axis]) + (t.data_max_lo[axis] - t.data_min_lo[axis]);
        let low = value < 0.5;
        let data_t = select(1.0, 0.0, low == (range > 0.0));
        return fma(data_t, t.data_to_panel_scale[axis], t.data_to_panel_offset[axis]) * 2.0 - 1.0;
    }
    if (axis == 0u) {
        if (t.scale_log.x != 0.0 && !(value > 0.0)) {
            return t.data_to_panel_offset.x * 2.0 - 1.0;
        }
        return data_to_ndc(t, base, one).x;
    }
    if (t.scale_log.y != 0.0 && !(value > 0.0)) {
        return t.data_to_panel_offset.y * 2.0 - 1.0;
    }
    return data_to_ndc(t, one, base).y;
}

// The inverse of `polar_ndc` (#208): at pixel offset `px` from the centre
// (`polar_clip_offset`, the draw's pixel frame), the angle value (in the turn
// `atan2` returns, before any wrap), the radius axis `t`, and their
// derivatives per pixel, which a polar contour needs to measure its stroke
// in pixels. It places given data, so it is projection, not data (rule 1).
// The centre itself has no angle: its derivatives are zero.
struct PolarUnproject {
    angle: f32,
    ty: f32,
    d_angle: vec2<f32>,
    d_ty: vec2<f32>,
};

fn polar_unproject(t: Transform, px: vec2<f32>) -> PolarUnproject {
    let p = t.polar_angle;
    let e = t.polar_extent;
    var out: PolarUnproject;
    let r2 = dot(px, px);
    let rho = sqrt(r2);
    // `polar_ndc` places radius fraction `rt` at `mix(inner, 1, rt) · outer`.
    let span = (1.0 - p.w) * e.w;
    out.angle = (atan2(px.y, px.x) - p.y) / p.z;
    out.ty = (rho - p.w * e.w) / span * t.data_to_panel_scale.y + t.data_to_panel_offset.y;
    out.d_angle = vec2<f32>(0.0, 0.0);
    out.d_ty = vec2<f32>(0.0, 0.0);
    if (rho > 0.0) {
        out.d_angle = vec2<f32>(-px.y, px.x) / (r2 * p.z);
        out.d_ty = px / rho * (t.data_to_panel_scale.y / span);
    }
    return out;
}

/// A data point in the draw's pixel frame: render pixels from the target's
/// centre, y up, as every vertex stage computes it. The exact picker
/// measures every primitive in this frame (SHADER_COMMON.md, the draw's
/// pixel frame).
fn data_to_px(t: Transform, xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    return data_to_ndc(t, xv, yv) / t.pixel_to_ndc;
}
