// The basic twin of `probability.wgsl` (#299): the signatures its callers
// use with no probability arithmetic, for a renderer that has drawn no chart
// with a probability axis, so its pipelines do not compile `F⁻¹`. A chart
// with a probability axis is never drawn or picked through this fragment
// (`shader_assembly::ShaderSet`); every call below sits behind a test
// of the axis' probability code, which is 0 on the charts drawn here.

// No sample has a position: a probability axis draws nothing here.
fn prob_in_domain(pair: vec2<f32>) -> bool {
    return false;
}

// Unreached with code 0.
fn prob_forward(pair: vec2<f32>, kind: f32) -> f32 {
    return 0.0;
}

// Unreached with code 0.
fn prob_slope(pair: vec2<f32>, kind: f32) -> f32 {
    return 1.0;
}

// Unreached with code 0.
fn prob_inverse(y: f32, kind: f32) -> vec2<f32> {
    return vec2<f32>(y, 0.0);
}
