// Input layer, grid form: `u` / `v` (and the colour value) are grids over the
// series' coordinate columns, declared like a heatmap's matrix.
//
// Instance `i` is cell `(col = i / rows, row = i % rows)` in constituent-column
// order. `col` walks the coordinate the columns run along (x for
// `ColumnsAreX`, y for `ColumnsAreY`) and `row` the other one. With `Centers`
// the arrow sits on the coordinate; with `Edges` it sits at the cell centre,
// the midpoint of two consecutive coordinates in data space, which is where
// the field's `sample_point_pair` puts a cell's value and what
// `selection_details` reports for an edge grid.
