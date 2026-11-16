// The basic twin of `polar.wgsl` (#299): the same type and function
// signatures with no polar arithmetic, for a renderer that has drawn no
// polar chart, so its pipelines do not compile it. A polar chart is never
// drawn or picked through this fragment (`shader_assembly::ShaderSet`);
// each function answers for a rectangular chart and, where it can, gives a
// polar transform nothing to draw.

// Unreached on a rectangular chart (`data_to_ndc` tests the polar flag): a
// point outside the clip volume. It is computed from `ty`, not a constant:
// a compiler that sees a constant on the polar side of that test may fold
// the arithmetic that follows the rectangular side differently, which moved
// a box plot's points by an ulp on one driver.
fn polar_ndc(t: Transform, ty: f32, xv: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(abs(ty) + 2.0);
}

struct PolarUnproject {
    angle: f32,
    ty: f32,
    d_angle: vec2<f32>,
    d_ty: vec2<f32>,
};

// Unreached on a rectangular chart: zeros.
fn polar_unproject(t: Transform, px: vec2<f32>) -> PolarUnproject {
    return PolarUnproject(0.0, 0.0, vec2<f32>(0.0, 0.0), vec2<f32>(0.0, 0.0));
}

// Every sample of a rectangular chart, none of a polar one.
fn polar_sample_valid(t: Transform, xv: vec2<f32>, yv: vec2<f32>) -> bool {
    return t.polar_angle.x < 0.5;
}

// Every fragment of a rectangular chart, none of a polar one.
fn polar_fragment_visible(px: vec2<f32>, limits: vec4<f32>, sector: vec4<f32>) -> bool {
    return limits.x < 0.5;
}
