fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    if is_log > 0.5 {
        return lv;
    }
    return mix(v, lv, is_log);
}

fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, panel_scale: f32, panel_offset: f32) -> f32 {
    let raw = v.x + v.y;
    let linear_num = fma(1.0, v.x - min_hi, v.y - min_lo);
    let range = fma(1.0, max_hi - min_hi, max_lo - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    if is_log > 0.5 {
        return fma(log_num / range, panel_scale, panel_offset);
    }
    let data_t = mix(linear_num / range, log_num / range, is_log);
    return fma(data_t, panel_scale, panel_offset);
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
    let tx = axis_pair_to_t(xv, t.data_min.x, t.data_max.x, t.data_min_lo.x, t.data_max_lo.x, t.scale_log.x, t.data_to_panel_scale.x, t.data_to_panel_offset.x);
    let ty = axis_pair_to_t(yv, t.data_min.y, t.data_max.y, t.data_min_lo.y, t.data_max_lo.y, t.scale_log.y, t.data_to_panel_scale.y, t.data_to_panel_offset.y);
    if (t.polar_angle.x > 0.5) {
        return polar_ndc(t, ty, xv);
    }
    return vec2<f32>(tx, ty) * 2.0 - 1.0;
}
