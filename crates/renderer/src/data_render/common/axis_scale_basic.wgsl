// The basic twin of `axis_scale.wgsl` (#299): the same signatures and the
// same linear and logarithmic arithmetic, with no probability placement and
// no cut remap, for a renderer that has drawn no chart with either, so its
// pipelines do not compile them. A chart with a break or a probability axis
// is never drawn or picked through this fragment
// (`shader_assembly::ShaderSet`).

// The raw data t of `v` on a linear or logarithmic axis: its fraction of
// the range, in decades on a logarithmic one. `prob` is unread.
fn axis_data_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, prob: f32) -> f32 {
    let range = fma(1.0, max_hi - min_hi, max_lo - min_lo);
    let raw = v.x + v.y;
    let linear_num = fma(1.0, v.x - min_hi, v.y - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    if is_log > 0.5 {
        return log_num / range;
    }
    return mix(linear_num / range, log_num / range, is_log);
}

// `cuts` is unread: data t is drawn at t.
fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, prob: f32, panel_scale: f32, panel_offset: f32, cuts: AxisCuts) -> f32 {
    let data_t = axis_data_t(v, min_hi, max_hi, min_lo, max_lo, is_log, prob);
    return fma(data_t, panel_scale, panel_offset);
}
