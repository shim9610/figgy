# figgy playground

An interactive debug bench for every figgy chart family. It is a standalone
Cargo workspace that only uses the public API of `crates/renderer`. It is not a
workspace member, not a renderer feature, and not part of any published package.

The scene catalogue (the synthetic data and the starting model of every scene)
is `crates/renderer/tests/support/playground_scenes.rs`, which the playground
includes with `#[path]`. The renderer's decoration oracle
(`crates/renderer/tests/decoration_oracle.rs`) sweeps the same file, so a
scene added there is checked in CI without a test of its own.

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

It registers the lines scene as a chart, applies a sequence of overlay and
subplot edits with `Renderer::set_chart_config`, and writes a PNG of each step,
and of every Cartesian scene at 1x and 2x, to `target/playground/selftest/`
under the working directory. A rejected config or a failed export stops the run
with exit status 1.

## Scenes

The tabs at the top switch scenes.

- **Cartesian**: lines, areas and steps; scatter with error bars; scatter with
  colour and size value maps; histogram;
  heatmap with labelled contours; vector field (quiver); log axes; twin Y axes; subplots;
  polar coordinates (an angle axis and a radius axis);
  annotations; a one-million-point line; categorical bars and box plots
  (documents opened as Cartesian series on a categorical axis); a radar
  (spider) chart (a document opened as polar radar series on a categorical
  angle axis).
- **Radial**: the pie and donut chart (the DataLess chart, its own renderer).

The lines scene's title and Y title, the twin-axes overlay title, a subplot
title, an annotation and the radar title are formulas written with
`RichText::from_tex` (#45), so selecting, dragging and exporting math text can
be tried in every layer kind.

## Pointer

- **Left press**: select an axis, labels, title, legend, data area, colourbar
  or annotation, then drag it. A selected data area has eight resize handles.
  A click on a point, line, error bar, line fill, bar, cell or isoline picks
  it (Shift adds); the data area wins only once it is already selected.
- **Wheel**: zoom around the cursor (Shift: X only, Ctrl or Cmd on macOS: Y
  only).
- **Right or middle drag**: pan.

Wheel zoom and pan work in Cartesian scenes only. They act on the active
layer's chart: the parent, an overlay's Y axis, or a subplot.

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
  series, the colour and size values read back from the GPU column.
- **Annotations**: show or hide the layer, add vertical and horizontal
  reference lines, spans, text boxes and arrows, and edit or delete the
  selected item.
- **Export**: write the document PNG at a scale from 1x to 4x to
  `target/playground/<scene>.png` under the working directory.
- **Diagnostics**: GPU memory usage and render warnings.
