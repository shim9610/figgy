# figgy playground

An interactive debug bench for every figgy chart family. It is a standalone
Cargo workspace that only uses the public API of `crates/renderer`. It is not a
workspace member, not a renderer feature, and not part of any published package.

The scene catalogue (the synthetic data and the starting model of every scene)
is `crates/renderer/tests/support/playground_scenes.rs`, which the playground
includes with `#[path]`. The renderer's decoration tests
(`crates/renderer/tests/decoration_oracle.rs`) run over every scene in that
file, so a scene added there is checked in CI without a test of its own.

winit owns the window and the event loop. The app owns the wgpu surface, and
egui (`egui-winit` + `egui-wgpu`, no eframe) draws the controls on top.
Cartesian charts are painted straight into the surface pass. Radial charts
are rendered offscreen and shown as an egui image.

## Run

From the repository root:

```sh
cargo run --manifest-path tools/playground/Cargo.toml
```

With `--selftest` the playground opens no window, but it still needs a GPU
adapter:

```sh
cargo run --manifest-path tools/playground/Cargo.toml -- --selftest
```

It writes its output to `target/playground/selftest/` under the working
directory:

- the lines scene registered as a chart, through a sequence of overlay and
  subplot edits committed with `Renderer::set_chart_config`, a PNG after each
  step;
- the heatmap scene without its contour label boxes and level 0, as a PNG at
  2x;
- every scene of the Cartesian group as it starts, at 1x and 2x, each as PNG,
  SVG and PDF (the monotonic scene's samples are lent to each export);
- the offset spectra with their group edited through the registry (a larger
  step, no skew, the stack reversed), a PNG after each edit, and a check that
  a member without a line is refused;
- the long-label scene after `layout::auto_layout`, committed the same way,
  at 1x and 2x, and a check that a second `auto_layout` changes nothing.

It prints `selftest passed` at the end. A rejected config, a failed export or
a failed check prints `selftest FAILED` with the reason and stops the run with
exit status 1.

## Scenes

The tabs at the top switch scenes. They come in two groups: **Cartesian**,
every scene the `Renderer` draws from a `Config`, and **Radial**, the scenes
drawn by `RadialRenderer`. The panel on the left names the scene and says what to try.
Below, the scenes of the Cartesian group are grouped by kind; the names are the
tab labels.

- **Lines, points and fields**
  - *Lines · areas · steps*: area fills to a baseline, a band between two
    series, the step modes.
  - *Scatter · error bars*: marker shapes, XY error bars, a line through the
    points.
  - *Scatter · value maps*: points coloured by a value column through the
    colourbar and sized by another (bubbles).
  - *Histogram*: pre-binned edges and counts, vertical or horizontal.
  - *Heatmap · contours*: an interpolated field, labelled isolines and a
    colourbar.
  - *Hexbin (hexagonal grid)*: host-binned counts on flat-topped hexagons.
  - *Image (RGB test card)*: three channel grids on one lattice, with markers
    over them.
  - *Vector field (quiver)*: arrows from (u, v) grids over a speed heatmap,
    plus pixel-scaled probe arrows.
  - *Streamlines (dipole)*: field lines the host integrates, drawn as
    NaN-separated polylines with arrowheads on a logarithmic colourbar.
  - *Stems (impulse response)* and *Lollipop (ranked)*: stems from a
    baseline, along y and along x on a categorical axis.
- **Axes, layers and layout**
  - *Log axes*: decade ticks and adaptive ticks under one decade.
  - *Axis units (raw counts in volts)*: raw channels printed in physical
    units (`AxisOptions.unit`).
  - *Category axis*: a label per category, staggered when they crowd.
  - *Broken axes (two peaks)*: ranges cut out of an axis, joined or drawn as
    panels.
  - *Probability paper (normal, Weibull)*: probability axes with the host's
    plotting positions and fitted lines.
  - *Long labels (auto layout)*: `0.00300` under the Y title, negative
    decimals, a logarithmic top axis, timestamps and a colourbar, in bands set
    by hand, which the "Auto layout" button beside "Reset view" fixes through
    `layout::auto_layout`. The button works on every scene of the Cartesian
    group; *Monotonic x* refuses a layout that changes the plot's width.
  - *Twin Y axes (overlay)* and *Subplots (linked X)*: an overlay layer with
    its own right axis, and subplots linked to the parent.
  - *Annotations*: reference lines, spans, text boxes and arrows to drag.
  - *Offset spectra (waterfall)*: twelve spectra stacked by
    `Config::line_offsets` with a skew along x, the first filled to its
    baseline and each labelled by an annotation at its start; a pick on a
    shifted trace reports its measured values.
- **Large data**
  - *Large data (resident)*: 5 million resident points at the start and up to
    100 million, as a line, markers or both; upload a new size, zoom, pan,
    pick or run the benchmark.
  - *Monotonic x (recorded channels)*: four channels of a 10 kHz recording,
    200 000 samples each, as monotonic-x lines in a chart registered with
    `Renderer::register_monotonic_chart`. The host lends the samples and the
    renderer keeps each pixel column's first, lowest, highest and last
    sample. This scene does not pan or zoom: its x range stays the one the
    scene's document sets, and the wheel and the pan drag do nothing. Resizing
    the window resizes the chart, and the renderer asks for the samples again
    at the new width. An edit that would change how x maps to pixels, such as
    a new x range or plot width, is refused. Pick a kept point and step with
    the arrows.
- **Categories and ranges**
  - *Categorical bars*: a categorical document opened as category bars on a
    categorical axis; switch grouped, stacked and percent in the chart's bar
    groups.
  - *Box plot*: a box plot document opened as box plot series, with notches,
    means and outliers.
  - *Split violins (box plot)* and *Raincloud (box plot)*: the host's density
    curves as violins, and half violins with jittered raw points.
  - *Waterfall (range bars)*, *Candlesticks (range bars)* and *Gantt (range
    bars)*: an energy balance with running totals and connectors, daily prices
    on a time axis, and tasks with a state and progress.
  - *Parallel coordinates*: one polyline per row across four axes with their
    own ranges.
- **Polar and ternary**
  - *Polar (angle, radius)*: a filled pattern and a marker rose with arc
    segments.
  - *Radar (spider)*: a radar document opened as polar radar series on a
    categorical angle axis.
  - *Rose (polar bars)*: a wind rose of stacked speed bands.
  - *Polar heatmap (radar)*: a weather radar sweep with labelled contours.
  - *Ternary (three components)*: alloy compositions, a dashed tie-line and a
    filled phase field with a hole.
- **3D and meshes**
  - *3D surface*: a heatmap's grid as a lit, wireframed surface, its contours
    on the floor.
  - *3D vector field (ABC flow)*: one arrow per lattice sample along
    (u, v, w).
  - *Triangle mesh (flow past a cylinder)*: the pressure coefficient on a
    graded annulus mesh, interpolated, with contour lines.
  - *3D triangle mesh surface*: the same mesh as a matte, wireframed surface.
- **Diagrams**
  - *Energy levels (He I)*, *Energy levels (Ar I)*, *Energy levels (Cu I)*,
    *Energy levels (U I)* and *Energy levels (Ar I, every level)*: Grotrian
    diagrams from NIST data.
  - *Feynman diagrams*: four processes whose internal vertices are laid out
    automatically; drag a vertex, bend a line, move a label.

The **Radial** group holds *Radial (pie / donut)* and *Sunburst (nested rings)*.

The lines scene's title and Y title, the twin-axes overlay title, a subplot
title, an annotation and the radar title are formulas written with
`RichText::from_tex`, so selecting, dragging and exporting math text can
be tried in every layer kind.

## Pointer

- **Left press**: select an axis, labels, title, legend, data area, colourbar
  or annotation, then drag it. A selected data area has eight resize handles.
  A click on a point, line, error bar, line fill, bar, cell or isoline picks
  it (Shift adds); the data area wins only once it is already selected.
- **Wheel**: zoom around the cursor (Shift: X only, Ctrl or Cmd on macOS: Y
  only).
- **Right or middle drag**: pan.

Wheel zoom and pan work in the scenes of the Cartesian group, except
*Monotonic x*, whose x range does not move. They act on the active layer's
chart: the parent, an overlay's Y axis, or a subplot. In the 3D scenes a drag
on the data area orbits the camera.

## Panel

- **Interaction**: pointer mode, active layer, reset, cursor readout.
- **Chart**: draw style, axis presets, colour cycle, titles, grid, log and
  inverted axes, legend, colourbar.
- **Layers**:
  - Add and remove overlays (an extra Y axis on the same plot) and subplots
    (separate charts with optional X/Y links, re-tiled on a grid).
  - Move each series between layers. Series joined by a band fill
    (`LineFill::ToSeries`) move together, because both ends of a band must be
    in the same chart.
- **Series**: per render type (line, marker, error bar, histogram, field,
  contour and vector). Lines set colour, width, dash, linear or step
  interpolation and a baseline fill. Vectors set arrow colour and size, pivot,
  scale mode and factor, and a colour map column for point vectors. Markers can
  take a colour map column (through the colourbar) and a size map column with
  its domain, pixel range and area/radius scale. Contour labels can toggle the
  background box, font size, digits and level 0.
- **Picking**: each pick lists the point's coordinates and, for a mapped
  series, the colour and size values from its value columns.
- **Annotations**: show or hide the layer, add vertical and horizontal
  reference lines, spans, text boxes and arrows, and edit or delete the
  selected item.
- **Export**: write the document PNG at a scale from 1x to 4x to
  `target/playground/<scene>.png` under the working directory.
- **Diagnostics**: GPU memory usage and render warnings.
