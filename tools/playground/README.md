# figgy playground

An interactive debug bench for every figgy chart family. It is a standalone
Cargo workspace that only uses the public API of `crates/renderer`. It is not a
workspace member, not a renderer feature, and not part of any published package.

winit owns the window and the event loop. The app owns the wgpu surface, and
egui (`egui-winit` + `egui-wgpu`, no eframe) draws the controls on top.
Cartesian charts are painted straight into the surface pass. Radial,
categorical and box-plot charts are rendered offscreen and shown as an egui
image.

## Run

From the repository root:

```sh
cargo run --manifest-path tools/playground/Cargo.toml
```

`-- --selftest` runs headless instead. It applies a sequence of overlay and
subplot edits through the renderer registry, and exports each step and every
Cartesian scene to `target/playground/selftest/` under the working directory.
A rejected config fails the run.

## Scenes

The tabs at the top switch scenes.

- **Cartesian**: lines, areas and steps; scatter with error bars; histogram;
  heatmap with labelled contours; log axes; twin Y axes; subplots;
  annotations; a one-million-point line.
- **Bounded**: radial (pie and donut), categorical bars, box plot.

## Pointer

- **Left press**: select an axis, labels, title, legend, data area, colourbar
  or annotation, then drag it. A selected data area has eight resize handles.
  A click on a point, bar, cell or isoline picks it (Shift adds); the data
  area wins only once it is already selected.
- **Wheel**: zoom around the cursor (Shift: X only, Ctrl: Y only).
- **Right or middle drag**: pan.

Pan and zoom act on the active layer's chart: the parent, an overlay's Y axis,
or a subplot.

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
- **Series**: per render type (line, marker, error bar, histogram, field and
  contour). Contour labels can toggle the background box, font size, digits
  and level 0.
- **Picking, Annotations, Export (PNG), Diagnostics**: GPU memory and render
  warnings.
