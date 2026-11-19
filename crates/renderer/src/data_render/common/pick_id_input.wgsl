// ───── pick ID input (#381) ─────
// What a vertex stage forwards to the pick ID output (`pick_id_output.wgsl`):
// the clip position and a flat `pick_id`, the series slot and the logical
// index of the primitive the vertex belongs to (a point, a segment, a bar,
// a cell, a triangle). `flat` keeps the integers exact: a primitive's
// fragments all carry its provoking vertex's id, never an interpolation.
//
// The vertex stage already has the instance or vertex index; this only
// forwards it. Nothing here reads data.

struct PickIdVarying {
    @builtin(position) position: vec4<f32>,
    // (series slot, primitive index).
    @location(0) @interpolate(flat) pick_id: vec2<u32>,
};

// A vertex of primitive `primitive` of series slot `series` at clip
// position `ndc` (z 0, w 1).
fn pick_id_varying(series: u32, primitive: u32, ndc: vec2<f32>) -> PickIdVarying {
    return PickIdVarying(vec4<f32>(ndc, 0.0, 1.0), vec2<u32>(series, primitive));
}

// A vertex of a primitive that draws nothing: outside the clip volume, so
// a point is clipped, and every vertex of a triangle at the same point, so
// it has no area; neither has a fragment.
fn pick_id_culled(series: u32, primitive: u32) -> PickIdVarying {
    return PickIdVarying(vec4<f32>(2.0, 2.0, 2.0, 1.0), vec2<u32>(series, primitive));
}
