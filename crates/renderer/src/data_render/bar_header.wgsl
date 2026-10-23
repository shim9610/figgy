// Columnar bar shader — histograms from host-binned data.
//
// Binning is the host's job. The renderer is handed two columns: bin `edges`
// (n + 1 values) and `counts` (n). The edge column is bound twice, the second
// time shifted by one logical value (COLUMN_VALUE_BYTES), so instance i sees
// `edges[i]`, `edges[i + 1]`, and `counts[i]` — the same trick the line shader
// uses for its segment endpoints. Instance count is decided by the caller as
// `min(edges - 1, counts)`; a length mismatch draws fewer bars, never garbage.
//
// `style.shape_id` picks which axis the edges run along, so one shader draws
// both orientations. The caller binds the edge column to slots 0/1 and the
// count column to slot 2 accordingly; nothing here infers the roles from the
// column lengths.
//
// Each instance is 5 axis-aligned quads = 30 vertices on a TriangleList:
// the fill plus the four border edges, which tile the bar exactly with no
// overlap (so a translucent fill never blends over its own border). Pixel
// dimensions — the inter-bar gap and the border width — are applied as NDC
// offsets via `transform.pixel_to_ndc`, the same construction
// `errorbar_columnar.wgsl` uses.
//
// Log axes need no special case: `maybe_log` already floors non-positive
// values at 1e-30, so a bar based at 0 on a logarithmic count axis extends
// past the bottom of the data area and is clipped by the scissor — which is
// what a bar with no lower bound looks like. Nothing produces NaN.
