// The identity twin of `axis_cuts.wgsl` (#299): the same type and function
// signatures with no cut, for a renderer that has drawn no broken chart, so
// its pipelines do not compile the remap. A chart with a break is never
// drawn or picked through this fragment (`shader_assembly::AxisCutShaders`).

// One axis' cuts: none. `count_scale` is `(0, 1)`, as on an unbroken axis
// of `axis_cuts.wgsl`.
struct AxisCuts {
    count_scale: vec2<f32>,
};

fn no_cuts() -> AxisCuts {
    return AxisCuts(vec2<f32>(0.0, 1.0));
}

// Axis `axis` of a transform's break table: unread.
fn cuts_of(breaks: array<vec4<f32>, 6>, info: vec4<f32>, axis: u32) -> AxisCuts {
    return no_cuts();
}

// Where data t is drawn: at t.
fn cut_remap(c: AxisCuts, t: f32) -> f32 {
    return t;
}

// The inverse of `cut_remap`.
fn cut_unmap(c: AxisCuts, drawn: f32) -> f32 {
    return drawn;
}

// The scale of the piece data t lies in: 1.
fn cut_scale_at(c: AxisCuts, t: f32) -> f32 {
    return 1.0;
}
