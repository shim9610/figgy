# Fields and maps

[← Gallery](../../../README.md#gallery) · [Lines, areas and stems](lines.md) · [Bars and distributions](distributions.md) · [Ranges and timelines](ranges.md) · **Fields and maps** · [3D](3d.md) · [Polar and radial](polar.md) · [Science diagrams](science.md) · [Layouts and annotations](layouts.md) · [Styles and visual concepts](styles.md)

Click an image to open the example code that draws it. All data is synthetic;
the images are native renderer output.

## Heatmap with contours

<a href="../examples/heatmap_contours.rs"><img src="../assets/gallery-contours.png" alt="Heatmap with labelled contours" width="760"></a>

An interpolated matrix fill, labelled isolines with line gaps, a colourbar.

**Code:** [heatmap_contours.rs](../examples/heatmap_contours.rs), `contours`

**Run:** `cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours`

## Hexbin

<a href="../examples/heatmap_contours.rs"><img src="../assets/gallery-hexbin.png" alt="Hexbin of taxi pickups" width="760"></a>

100,000 taxi pickups the host bins into counts on a hexagonal lattice (`GridLayout::Hexagonal`), on a log colourbar; empty cells are NaN.

**Code:** [heatmap_contours.rs](../examples/heatmap_contours.rs), `hexbin`

**Run:** `cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours`

## Colour image

<a href="../examples/heatmap_contours.rs"><img src="../assets/gallery-imshow.png" alt="Planetary nebula as an RGB image" width="760"></a>

A planetary nebula as one `RgbImage`: three host-composited channel grids on one lattice of square pixels, interpolated, with field stars.

**Code:** [heatmap_contours.rs](../examples/heatmap_contours.rs), `imshow`

**Run:** `cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours`

## Triangle mesh

<a href="../examples/trimesh.rs"><img src="../assets/gallery-trimesh.png" alt="Stress around a hole on a triangle mesh" width="760"></a>

Von Mises stress around a hole in a plate under tension (Kirsch's solution) on the host's graded mesh: interpolated fill, contour lines and the mesh edges as a faint line series.

**Code:** [trimesh.rs](../examples/trimesh.rs), `plate`

**Run:** `cargo run --locked -p figgy-renderer --example trimesh -- target/trimesh`

## Wind field

<a href="../examples/vector_field.rs"><img src="../assets/gallery-wind.png" alt="Wind field over a heatmap" width="760"></a>

Grid vectors from two matrices over a speed heatmap.

**Code:** [vector_field.rs](../examples/vector_field.rs), `wind`

**Run:** `cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field`

## Surface currents

<a href="../examples/vector_field.rs"><img src="../assets/gallery-currents.png" alt="Surface currents" width="760"></a>

Point vectors coloured by a speed column, scaled with `fit_scale`.

**Code:** [vector_field.rs](../examples/vector_field.rs), `currents`

**Run:** `cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field`

## Colour-mapped points

<a href="../examples/value_maps.rs"><img src="../assets/gallery-colormap.png" alt="Colour-mapped particles" width="760"></a>

`point_color_map`: points coloured by an age column through the colourbar.

**Code:** [value_maps.rs](../examples/value_maps.rs), `colormap_scatter`

**Run:** `cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps`

## Bubble chart

<a href="../examples/value_maps.rs"><img src="../assets/gallery-bubbles.png" alt="Bubble chart" width="760"></a>

`point_size_map`: marker area follows a population column.

**Code:** [value_maps.rs](../examples/value_maps.rs), `bubbles`

**Run:** `cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps`
