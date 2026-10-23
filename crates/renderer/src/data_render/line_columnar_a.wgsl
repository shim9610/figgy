struct VsIn {
    @location(0) x_a: vec2<f32>,
    @location(1) y_a: vec2<f32>,
    @location(2) x_b: vec2<f32>,
    @location(3) y_b: vec2<f32>,
};

// Line-only extra instance inputs: cumulative
// arc length at the segment's two endpoints, from the GPU-computed prefix.
struct VsArc {
    @location(4) arc_a: f32,
    @location(5) arc_b: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Arc length (px) from the polyline start: arc_a at the two A-side
    // corners, arc_b at the two B-side corners — interpolation yields the
    // per-fragment arc position the dash pattern walks.
    @location(0) dist_px: f32,
    // Local segment-space coordinates in pixels. The fragment stage treats
    // the whole segment stroke as one square-cap rectangle:
    //   along_px ∈ [-half_w, len + half_w], side_px ∈ [-half_w, half_w].
    @location(1) along_px: f32,
    @location(2) side_px: f32,
    @location(3) segment_len_px: f32,
    @location(4) polar_px: vec2<f32>,
    @location(5) @interpolate(flat) polar_limits: vec4<f32>,
    @location(6) @interpolate(flat) polar_sector: vec4<f32>,
};


// ───── shared line stroke (common/line_stroke.wgsl) ─────
