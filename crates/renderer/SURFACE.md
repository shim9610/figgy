# 3D surfaces

A 3D chart draws a box in its data area, seen through a camera kept in the
chart's configuration. A heatmap on it draws as a surface over its grid, a
contour draws its isolines on the box floor, and a heatmap with contours
draws both. A triangle mesh (`TriMesh`) draws as a surface over its
triangles (a trisurf), its contours on the floor too. Set `Config.view_3d`
to `Some(View3d::default())`. `view_3d: None` (the default, omitted from
JSON) is the flat chart, so older documents are unchanged.

The data types are the flat ones: the same `Heatmap` series draws flat on a
2D chart and as a surface on a 3D one, and so does a `TriMesh` series.
Switching between the two is one configuration change.

## Input and first render

A surface reads the heatmap's own grid: `x_column` and `y_column` hold the
sample coordinates and `matrix` names one column per grid column, the
heights. Every column enters through `ColumnSource` like any other column.

```rust,no_run
use renderer::{
    Chart, ColorMap, Renderer, SeriesConfig,
    data::Column,
    data_config::{
        DataRenderType, FieldFillConfig, FillMode, GridLayout, MatrixOrientation, MatrixRef,
        Shading,
    },
    default,
    layout::{ChartArea, Rect},
    view3d::View3d,
};

fn draw(renderer: &mut Renderer) -> Result<(), Box<dyn std::error::Error>> {
    let n = 32;
    let at = |i: usize| i as f64 / (n - 1) as f64;
    let xs: Vec<f64> = (0..n).map(at).collect();
    renderer.add_column("x", &Column { data: xs.clone() })?;
    renderer.add_column("y", &Column { data: xs.clone() })?;
    let mut columns = Vec::new();
    for i in 0..n {
        let id = format!("z{i}");
        let z: Vec<f64> = xs
            .iter()
            .map(|y| (-((at(i) - 0.5).powi(2) + (y - 0.5).powi(2)) / 0.05).exp())
            .collect();
        renderer.add_column(&id, &Column { data: z })?;
        columns.push(id);
    }

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect { x: 0, y: 0, width: 800, height: 640 });
    config.set_view_3d(Some(View3d::default()));
    for axis in [&mut config.bottom_x, &mut config.left_y, &mut config.right_y] {
        axis.min = 0.0;
        axis.max = 1.0;
    }
    let mut bar = default::default_colorbar_options();
    bar.colormap = ColorMap::Viridis;
    bar.axis.min = 0.0;
    bar.axis.max = 1.0;
    config.colorbar = Some(bar);
    let chart = Chart::new(config).with_title("Peak");

    let series = [SeriesConfig {
        series_id: "peak".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Field(renderer::data_config::FieldSeries { matrix: MatrixRef {
                columns,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            }, fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            }), contour: None, surface: None }),
    }];
    let image = renderer.export_rgba_blocking(renderer::ExportSource::chart(&chart, &series), renderer::ExportOptions::panel(1.0))?;
    std::fs::write("surface.png", renderer::encode_png(&image)?)?;
    Ok(())
}
```

Each quad between four neighbouring samples draws as two triangles. A quad
with a NaN sample, or a sample its axes cannot place (such as a
non-positive value on a logarithmic axis), is not drawn, as a NaN breaks a
line, so a NaN leaves a hole in the surface.

## The box and the camera

The box's axes are x (`bottom_x`), y (`left_y`) and z (`right_y`, up). Each
axis is linear or logarithmic and not categorical; `top_x` stays in the
document but is not drawn.

`View3d` holds the camera:

| Field | Default | Meaning |
|---|---|---|
| `azimuth_degrees` | -60 | The turn around z, from +x towards +y; read modulo 360 |
| `elevation_degrees` | 30 | The height above the floor plane, `-90..=90` |
| `distance` | 3 | The eye distance in half-diagonals of the box, `1.2..=100` |
| `projection` | `Orthographic` | `Orthographic` or `Perspective` |
| `box_aspect` | `None` | The relative x : y : z lengths; `None` is `[1, 1, 0.75]` |

The box is fitted to the data area at the reference distance of 3. A
shorter distance magnifies and a longer one shrinks. With `Perspective`,
the distance alone sets the foreshortening; there is no field-of-view
option. The light is fixed relative to the camera, so a turn does not
change the shading.

`Config::view_matrix` returns the camera matrix the chart draws and picks
with. `Config::project_3d` returns the pixel position and depth of a box point,
which is how a host places its own overlay on the box.

## Series

- **`Heatmap`** draws a surface, coloured through the colourbar. Its
  `fill.shading` is honoured: `Interpolated` blends the colour across each
  quad and `Flat` gives each quad its corners' mean. `fill.opacity` makes
  the surface translucent. Banded fills are rejected on a 3D chart.
- **`Contour`** draws its isolines on the box floor, at their levels'
  colours and widths. Floor contours have no labels; a contour with
  `labels` is rejected on a 3D chart.
- **`HeatmapContour`** draws the surface and its contours on the floor.

`surface: Option<SurfaceStyle>` on `Heatmap` and `HeatmapContour` holds the
look; `None` (omitted from JSON) takes every default. On a flat chart the
field is kept and unused.

- **`TriMesh`** ([TRIMESH.md](TRIMESH.md)) draws a surface over its
  triangles when the series names its height column in
  `SeriesConfig.z_column` (read on `right_y`, as a surface's heights are):
  each triangle's three vertices are placed by `x_column`, `y_column` and
  `z_column`, and the fill colours the triangle by its values through the
  colourbar, interpolated across the triangle for vertex values and flat
  for face values. Its contours draw on the box floor, where its level set
  crosses the triangles. `TriMeshSeries.surface` holds the look, with the
  same fields as a heatmap's but no `color_matrix` (the values colour the
  mesh). The shading normal of a lit material is the triangle's own.
  `z_column: None` on a 3D chart is rejected,
  and a banded fill too; on a flat chart both the height column and the
  look are kept and unused, so a mesh switches between 2D and 3D with the
  view too. A triangle naming a vertex past the height column is not
  drawn, as one past the coordinate columns is not.

- **`Scatter`, `Line` and `ScatterLine`** draw points and trajectories in
  the box when the series names its height column in
  `SeriesConfig.z_column` (read on `right_y`, as the surface heights are).
  A marker is a camera-facing quad of `point_size` pixels on screen under
  either projection, at the depth of the front of a ball of its radius at
  the point; a line is a stroke of `line_width` pixels on screen between
  the projected samples, at the depth of the front of a tube of its half
  width, with its dashes measured along the projected chord. Both are
  depth tested against the surfaces and each other, so a mark on a surface
  stays visible, a marker covers its own stroke at the sample, and a mark
  behind a surface is hidden. A 3D line is
  linear (a step interpolation is rejected) without an area fill; a sample
  outside the box is not drawn and breaks the line, as a NaN does. Colour
  and size maps, per-point style tables and overrides apply as on a flat
  chart. `z_column: None` on a 3D chart is rejected; on a flat chart it is
  kept and unused, so a trajectory switches between 2D and 3D with the
  view too.
- **`Vector`** in the points form draws one arrow per row from
  `(x, y, z)` along `(u, v, w)`. The series names its anchor height
  in `SeriesConfig.z_column` and its z component in
  `VectorComponents::Points.w`; each missing column is rejected on a 3D
  chart, and on a flat chart both are kept and unused. The arrow is the
  flat chart's arrow, camera-facing and in logical pixels:
  `width_px`, `head_length_px` and `head_width_px` are constant on screen,
  and the arrow sits at the depth of the front of a tube of half its head
  width, so it is depth tested against the surfaces, the 3D marks and the
  other arrows.
  - `Data { factor }`: the tip is the anchor plus `factor · (u, v, w)` in
    each axis's data units, placed through the axes and the camera, so an
    arrow is foreshortened as it turns. An arrow whose anchor or tip leaves
    the box, or is non-positive on a logarithmic axis, is not drawn, as a 3D
    line drops a segment whose end leaves it.
  - `Pixels { px_per_unit }`: an arrow lying in the screen plane at its
    anchor is `px_per_unit · |(u, v, w)|` logical pixels long; other arrows
    are foreshortened by the camera. The length holds under `zoom_view`.
  - The pivot shifts along the projected axis: exact under the
    orthographic camera, a sub-pixel approximation under perspective.
  - A short arrow's head is scaled down, so an arrow pointing at the eye
    shrinks to its head, and below `min_length_px` it is not drawn.
  - `VectorStyle::fit_scale_3d` scales `hypot(max |u|, max |v|, max |w|)`,
    the longest vector the component ranges allow, to a target length. Colouring by speed takes a magnitude column the host uploads.
  - The grid form draws flat: it is rejected on a 3D chart (a host uploads
    lattice points as columns). Streamlines are rejected too.

| Field | Default | Meaning |
|---|---|---|
| `color_matrix` | `None` | A second grid of the same shape to colour by; `None` colours by height |
| `wireframe` | `None` | `SurfaceWireframe { color, width }`: grid lines along the rows and columns, `width` in logical pixels, constant on screen |
| `material` | `Flat` | `Flat` is unlit; any other `Material` is lit with shading normals |
| `gloss`, `texture_strength`, `texture_scale` | 0.45, 0.2, 1 | The material's parameters |
| `emphasis_brightness` | 0.12 | The brightness the picked sample's patch gains |

A 3D chart draws only these kinds, in the Precise draw style. It rejects
overlays, line offsets, the polar and ternary systems and streamed sources,
each with an explicit error. There is no limit on the grid's size, a mesh's
triangles or a series' rows: every quad, triangle, marker, segment and
arrow is drawn.

## Interaction

- **Hover and click.** Picking answers what the cursor's pixel shows, so
  hidden parts are never picked. On a surface it answers the grid sample
  nearest the cursor inside its quad: `PickedData::MatrixCell` with the sample's
  column and row. A mesh surface answers `PickedData::MeshTriangle` with the
  triangle the pixel shows and its vertex nearest the cursor.
  A floor contour answers its level, as on a flat chart (a mesh's names the
  triangle the stroke lies in as `x_index`). A marker answers
  `PickedData::Point` with its index, a stroke the index of its segment's
  nearer end (its dashes, not its gaps), an arrow its row where its shaft
  or head covers at least half the pixel; a mark behind a surface never
  answers.
- **Highlight.** The picked sample's patch, the halves of the quads around
  it nearest the sample, is outlined and brightened; a mesh's picked
  triangle is outlined and brightened whole. Either is drawn over anything
  in front of it, so a selection is visible from every side. A picked
  point is ringed at its projected place, over its occluders too; a picked
  arrow is ringed at its anchor at half its head width.
- **`step_selection`** walks the grid samples, as on a flat heatmap, a
  mesh's triangles, and a point series' rows.
- **`selection_details()`** reports the sample's x, y and height, plus the
  colour grid's value when set; a mesh triangle's vertex indices, their x,
  y, values and heights (`z`); a point's x, y and `z`; an arrow's x, y,
  `z`, `u`, `v` and `w`, and its colour value when colour mapped.
- **Camera.** Dragging the data area orbits the camera
  (`Config::orbit_view`, half a degree per pixel). `Config::zoom_view`
  zooms it (`zoom_view(factor)` in the browser wrapper). The data area's edges
  still resize it.
- **Decoration drags.**
  - A box axis (`Element::Axis`) moves along its edge's outward screen
    normal (`line_offset`) and carries its ticks and labels.
  - Tick labels (`Element::AxisLabel`) move in the edge frame:
    `label_offset_x` along the edge and `label_offset_y` along its normal.
  - Titles (`Element::AxisTitle`) move in screen space.
- **Fit.** `auto_fit_layer_union` and the browser's `autoFitAll` also fit
  `right_y` to the union of the height columns' statistics (a mesh's
  `z_column` among them). A `Data` arrow adds the bound
  `[z_min + f·w_min, z_max + f·w_max]` of its tips (ordered for a negative
  factor), from the same column ranges, which holds every tip.
- **Annotations.** Ratio and pixel anchors draw. A 3D chart has no
  separable value axes, so data anchors, reference lines and spans are
  skipped as `NoValueAxis`.
- **Subplots and export.** A subplot's `config.view_3d` makes that panel 3D
  beside flat ones. The chart exports to PNG at any scale, and `view_3d`
  serializes when on and is omitted when off.

## Draw order and cost

Floor contours draw first, then the surfaces (grid and mesh alike), so a
translucent surface shows the contours below it; then every series' 3D
arrows, lines and markers, depth tested against the surfaces and each other.
Under MSAA, the overlapping strokes and markers of a dense trajectory stay
solid.

- A mesh surface draws three vertices per triangle per pass, and picking
  redraws every triangle into a 1×1 target.
- An arrow draws six vertices, and picking redraws every arrow of the
  series into a 1×1 target, as a 3D scatter pick does.
- A 3D panel adds a depth buffer of 4 bytes per sample: about 12 MB for an
  1100 × 700 panel at 4x MSAA.

There is no limit on the triangle or arrow count; the cost grows with it.

## Tests

```sh
cargo test --locked -p figgy-renderer --features serde,cpu-twin --test surface_render
```
