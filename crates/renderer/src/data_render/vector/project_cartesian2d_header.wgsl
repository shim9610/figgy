// Projection layer: Cartesian 2D.
//
// One fragment of the assembled vector shader (`vector.rs`). It carries the
// common `Transform` / `Style` blocks, the hi/lo axis projection and the sample
// validity rule that every Cartesian data shader shares, and adds the arrow
// projection the entry calls: a `VectorSample` (hi/lo origin, vector,
// colour value) becomes an `ArrowPx` (tail and tip in chart-area pixels).
//
// Pixel space here is the chart area's, centred, y up, one unit per logical
// pixel: `px = ndc / transform.pixel_to_ndc`. The glyph layer works in this
// space and the entry converts back with `px_to_clip`.
//
// A later 3D projection fragment replaces this file alone: it has to provide
// the same `project_arrow` with the same `VectorSample` / `ArrowPx` types, and
// may ignore the hi/lo split once it has its own camera.
