// Where a value sits on its axis, with every scale an axis can have: linear,
// logarithmic, probability (#209) and the cut remap of a broken axis (#200).
// The full set of a data module splices it with `axis_cuts.wgsl` and
// `probability.wgsl`; `axis_scale_basic.wgsl` is its twin for a renderer
// that has drawn no chart with a break or a probability axis (#299), with
// the same signatures and the same linear and logarithmic arithmetic.

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
