// Polar coordinates (`Config.polar`): placing a sample around the centre,
// its inverse, and the two tests that keep a sample and a fragment inside
// the plot. The full set of a data module splices it; `polar_basic.wgsl`
// is its twin for a renderer that has drawn no polar chart (#299), with the
// same signatures.

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

// Polar: a radius below the axis start has no place on the plot (it would
// fold onto the inner ring), so the sample is not drawn and a line breaks
// there. Angles outside the sector and radii past the outer ring keep their
// projection and are cut by the fragment clip below.
fn polar_sample_valid(t: Transform, xv: vec2<f32>, yv: vec2<f32>) -> bool {
    if (t.polar_angle.x < 0.5) {
        return true;
    }
    // A polar radius: polar charts have no breaks.
    let ty = axis_pair_to_t(yv, t.data_min.y, t.data_max.y, t.data_min_lo.y, t.data_max_lo.y, t.scale_log.y, 0.0, t.data_to_panel_scale.y, t.data_to_panel_offset.y, no_cuts());
    let rt = (ty - t.data_to_panel_offset.y) / t.data_to_panel_scale.y;
    return rt >= 0.0;
}

// Whether the fragment at pixel offset `px` from the centre
// (`polar_clip_offset`) is inside the polar plot (`polar_clip_limits`,
// `polar_clip_sector`); every fragment of a rectangular chart is.
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
