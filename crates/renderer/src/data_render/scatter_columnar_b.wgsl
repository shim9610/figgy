struct VsIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: vec2<f32>,
    @location(2) y: vec2<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) polar_px: vec2<f32>,
    @location(2) @interpolate(flat) polar_limits: vec4<f32>,
    @location(3) @interpolate(flat) polar_sector: vec4<f32>,
};

// Quad half-extent in pixels = shape-specific axis extent + QUAD_MARGIN_PX.
// `point_radius_px` is the circle reference radius; non-circular shapes are
// normalized below so the same marker size has a similar visual weight.
// Outline
// strokes are centered on the shape contour, so they reach up to
// STROKE_HALF_PX plus the AA feather (~1 px) beyond the radius; without
// the margin the quad edge would clip them. fs_main must reconstruct
// pixel coordinates with this same expanded scale.

// ───── shared marker shape (SHADER_COMMON.md §5.5; `shader_consistency`
// compares these function bodies and constants with the pick shader) ─────
