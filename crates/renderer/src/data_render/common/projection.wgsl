fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    if is_log > 0.5 {
        return lv;
    }
    return mix(v, lv, is_log);
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
    if (t.scale_log[axis] != 0.0 && !(value > 0.0)) {
        return t.data_to_panel_offset[axis] * 2.0 - 1.0;
    }
    // One `data_to_ndc` call for either axis: a software GPU compiler pays
    // for every inlined copy (#363).
    let on_y = axis != 0u;
    return data_to_ndc(t, select(base, one, on_y), select(one, base, on_y))[axis];
}

/// A data point in the draw's pixel frame: render pixels from the target's
/// centre, y up, as every vertex stage computes it. The exact picker
/// measures every primitive in this frame (SHADER_COMMON.md, the draw's
/// pixel frame).
fn data_to_px(t: Transform, xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    return data_to_ndc(t, xv, yv) / t.pixel_to_ndc;
}
