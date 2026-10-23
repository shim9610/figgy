// Columnar line shader — variable-thickness via quad extrusion plus analytic
// path-stroke coverage.
//
// One line segment = one instance, 4 vertices per instance (TriangleStrip).
// The X and Y columns are bound twice: the second binding starts one logical
// f32-pair value later, so instance `i` sees points [i] and [i+1] at once.
//
// Corner mapping per segment:
//   vid 0 : at A, −normal
//   vid 1 : at A, +normal
//   vid 2 : at B, −normal
//   vid 3 : at B, +normal
//
// The normal is computed in pixel space and converted back to NDC, so width
// stays constant across panels with different X/Y pixel ratios. The geometric
// quad is inflated by one AA pixel; the fragment stage evaluates the local
// square-cap stroke SDF so side edges, caps, and dash cuts get smooth coverage
// even when the render target itself is single-sample.
// Zero-length segments collapse the quad and become invisible.
//
// Dash patterns (Style.dash / Style.dash_len) are evaluated per fragment
// against the polyline's CUMULATIVE ARC LENGTH: slots 4/5 carry a GPU-computed
// prefix (pixels from the first point to point i), bound twice with a
// one-f32 shift like x/y, so the phase is exact and continuous across every
// joint regardless of curvature or sampling density. (A screen-position
// projection was tried first: its phase jumps by |position|·Δdirection at
// each joint, which shatters the pattern on curves.) Solid lines bind the X
// column as inert filler — the fragment stage never reads the varying when
// dash_len == 0.
//
// Step interpolation (Style.primitive_flags bits 2-3) draws every instance as
// 2 or 3 axis-aligned pieces instead of one straight segment. The strip then
// carries one 4-vertex quad per piece with two duplicated vertices between
// pieces (degenerate triangles), so the pipeline and the instance count stay
// the same and only the per-instance vertex count changes — see
// `strip_slot` and the CPU twin `line_strip_vertex_count` in mod.rs. The
// arc-length scan (line_arc.wgsl) and the GPU/CPU pickers walk the same
// pieces, so dash phase and picking stay on the stepped path.
//
// `vs_area` / `fs_area` fill the area under the line (to a baseline carried
// in Style.dash[1].xy) or between this line and a second y column bound in
// slots 4/5 (Style.primitive_flags bit 4). One instance per segment emits one
// trapezoid per step piece. It is a separate pipeline ("figgy line area
// pipeline") because its slots 4/5 are (hi, lo) pairs, not arc lengths.
