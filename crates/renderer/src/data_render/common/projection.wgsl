fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    if is_log > 0.5 {
        return lv;
    }
    return mix(v, lv, is_log);
}

// One axis of a broken chart (#200): its cuts `(from, to, start, end)` in
// the axis' data t and `(count, piece scale)`. Count 0 is the identity.
// CPU twin: `layout::BreakMap`.
struct AxisCuts {
    k0: vec4<f32>,
    k1: vec4<f32>,
    k2: vec4<f32>,
    count_scale: vec2<f32>,
};

fn no_cuts() -> AxisCuts {
    return AxisCuts(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec2<f32>(0.0, 1.0));
}

// Axis `axis` (0 = x, 1 = y) of a transform's break table.
fn cuts_of(breaks: array<vec4<f32>, 6>, info: vec4<f32>, axis: u32) -> AxisCuts {
    if (axis == 0u) {
        return AxisCuts(breaks[0], breaks[1], breaks[2], vec2<f32>(info.x, info.z));
    }
    return AxisCuts(breaks[3], breaks[4], breaks[5], vec2<f32>(info.y, info.w));
}

// Where data t is drawn: linear within each piece at the piece scale, linear
// across each cut onto its gap, so the map is continuous, increasing and
// one-to-one. Written without a loop over the knots: a dynamic index into a
// value array is not portable.
fn cut_remap(c: AxisCuts, t: f32) -> f32 {
    let n = c.count_scale.x;
    if (n < 0.5) {
        return t;
    }
    let s = c.count_scale.y;
    if (t < c.k0.x) {
        return t * s;
    }
    if (t < c.k0.y) {
        return c.k0.z + (t - c.k0.x) / (c.k0.y - c.k0.x) * (c.k0.w - c.k0.z);
    }
    if (n < 1.5 || t < c.k1.x) {
        return c.k0.w + (t - c.k0.y) * s;
    }
    if (t < c.k1.y) {
        return c.k1.z + (t - c.k1.x) / (c.k1.y - c.k1.x) * (c.k1.w - c.k1.z);
    }
    if (n < 2.5 || t < c.k2.x) {
        return c.k1.w + (t - c.k1.y) * s;
    }
    if (t < c.k2.y) {
        return c.k2.z + (t - c.k2.x) / (c.k2.y - c.k2.x) * (c.k2.w - c.k2.z);
    }
    return c.k2.w + (t - c.k2.y) * s;
}

// The inverse of `cut_remap`: the data t drawn at `drawn`.
fn cut_unmap(c: AxisCuts, drawn: f32) -> f32 {
    let n = c.count_scale.x;
    if (n < 0.5) {
        return drawn;
    }
    let s = c.count_scale.y;
    if (drawn < c.k0.z) {
        return drawn / s;
    }
    if (drawn < c.k0.w) {
        return c.k0.x + (drawn - c.k0.z) / (c.k0.w - c.k0.z) * (c.k0.y - c.k0.x);
    }
    if (n < 1.5 || drawn < c.k1.z) {
        return c.k0.y + (drawn - c.k0.w) / s;
    }
    if (drawn < c.k1.w) {
        return c.k1.x + (drawn - c.k1.z) / (c.k1.w - c.k1.z) * (c.k1.y - c.k1.x);
    }
    if (n < 2.5 || drawn < c.k2.z) {
        return c.k1.y + (drawn - c.k1.w) / s;
    }
    if (drawn < c.k2.w) {
        return c.k2.x + (drawn - c.k2.z) / (c.k2.w - c.k2.z) * (c.k2.y - c.k2.x);
    }
    return c.k2.y + (drawn - c.k2.w) / s;
}

// The piece scale: drawn length per data-t length inside a piece, 1 on an
// unbroken axis. Derivatives (pixels per data unit) multiply by it.
fn cut_scale(c: AxisCuts) -> f32 {
    if (c.count_scale.x < 0.5) {
        return 1.0;
    }
    return c.count_scale.y;
}

fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, panel_scale: f32, panel_offset: f32, cuts: AxisCuts) -> f32 {
    let raw = v.x + v.y;
    let linear_num = fma(1.0, v.x - min_hi, v.y - min_lo);
    let range = fma(1.0, max_hi - min_hi, max_lo - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    if is_log > 0.5 {
        return fma(cut_remap(cuts, log_num / range), panel_scale, panel_offset);
    }
    let data_t = mix(linear_num / range, log_num / range, is_log);
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
    let tx = axis_pair_to_t(xv, t.data_min.x, t.data_max.x, t.data_min_lo.x, t.data_max_lo.x, t.scale_log.x, t.data_to_panel_scale.x, t.data_to_panel_offset.x, cuts_of(t.breaks, t.break_info, 0u));
    let ty = axis_pair_to_t(yv, t.data_min.y, t.data_max.y, t.data_min_lo.y, t.data_max_lo.y, t.scale_log.y, t.data_to_panel_scale.y, t.data_to_panel_offset.y, cuts_of(t.breaks, t.break_info, 1u));
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

/// A data point in the draw's pixel frame: render pixels from the target's
/// centre, y up, as every vertex stage computes it. The exact picker
/// measures every primitive in this frame (SHADER_COMMON.md, the draw's
/// pixel frame).
fn data_to_px(t: Transform, xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    return data_to_ndc(t, xv, yv) / t.pixel_to_ndc;
}
