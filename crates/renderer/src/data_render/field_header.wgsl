// Columnar field shader — heatmaps and band (filled-contour) fields.
//
// The grid is a bundle of pool columns declared by `MatrixRef`: one column per
// x (or y) position, each holding that position's values along the other axis.
// Coordinates come from the ordinary `x_column` / `y_column`; nothing here is
// stored twice.
//
// **One quad for the whole field, not one per cell.** The cell a fragment
// belongs to is found per fragment by binary search over the coordinate
// columns. Two reasons, both decisive:
//
//   1. A 5000 x 5000 grid is 25M cells. As instances that is 150M vertices a
//      frame; as one quad it is 6, and the cost becomes the data area's pixel
//      count — which is what a heatmap's cost should be.
//   2. Adjacent per-cell quads seam under MSAA. Two quads meeting on an edge
//      resolve as `bg*(1-cA)*(1-cB) + ...`, so up to 25% of the background
//      shows through along every shared edge. One quad cannot seam against
//      itself.
//
// The search runs in axis `t` space (`axis_pair_to_t`), so logarithmic and
// inverted axes need no special case: the same monotone comparison works for
// all four combinations.
//
// Cell geometry follows `GridLayout`. `Edges` reads the coordinate columns as
// cell boundaries (n + 1 for n cells); `Centers` reads them as cell midpoints
// and derives boundaries as the midpoints between them, mirroring the outermost
// half-cells. A single centre has no neighbour to measure against, so its cell
// has zero width and draws nothing — the data does not say how wide it is, and
// inventing a width would be a number nobody wrote.
//
// `Shading::Flat` gives each cell its own colour from its own z.
// `Shading::Interpolated` interpolates z bilinearly between the four
// surrounding sample points, so the quads span point-to-point rather than
// cell-to-cell (matplotlib's `flat` vs `gouraud`).
//
// `FillMode::Bands` quantizes z into the intervals between the contour levels,
// which makes exactly the filled-contour bands the contour lines will be drawn
// over. `Continuous` uses the ramp directly.
//
// The colour ramp is the colourmap's own control points, and the mapping is a
// byte-for-byte reimplementation of `model::colormap::sample` — see `ramp()`.
// That is what makes the CPU-drawn colourbar strip and this shader agree: same
// stops, same formula, same `t`.
