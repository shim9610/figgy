# 3D surfaces

A 3D chart draws a box in its data area, seen through a camera kept in the
chart's configuration. A heatmap on it draws as a surface over its grid, a
contour draws its isolines on the box floor, and a heatmap with contours
draws both. Set `Config.view_3d` to `Some(View3d::default())`. `view_3d:
None` (the default, omitted from JSON) is the flat chart, so older
documents are unchanged.

The data type is the heatmap's: the same `Heatmap` series draws flat on a
2D chart and as a surface on a 3D one. Switching between the two is one
configuration change.

## Input and first render

A surface reads the heatmap's own grid: `x_column` and `y_column` hold the
sample coordinates and `matrix` names one pool column per grid column, the
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
    renderer.add_column("x", &Column { data: xs.clone(), min: 0.0, max: 1.0 })?;
    renderer.add_column("y", &Column { data: xs.clone(), min: 0.0, max: 1.0 })?;
    let mut columns = Vec::new();
    for i in 0..n {
        let id = format!("z{i}");
        let z: Vec<f64> = xs
            .iter()
            .map(|y| (-((at(i) - 0.5).powi(2) + (y - 0.5).powi(2)) / 0.05).exp())
            .collect();
        renderer.add_column(&id, &Column { data: z, min: 0.0, max: 1.0 })?;
        columns.push(id);
    }

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect { x: 0, y: 0, width: 800, height: 640 });
    config.view_3d = Some(View3d::default());
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
        render_type: DataRenderType::Heatmap {
            matrix: MatrixRef {
                columns,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            },
            surface: None, // every surface option takes its default
        },
    }];
    let image = renderer.export_panel_rgba(&chart, &series, 1.0)?;
    std::fs::write("surface.png", renderer::encode_png(&image)?)?;
    Ok(())
}
```

Each quad between four neighbouring samples draws as two triangles. The
vertex stage reads every sample's x, y and height from the pool on every
draw. Positions, colours and, for a lit material, normals are computed
there and never stored. A quad with a NaN or unplaceable sample is not
drawn, as a NaN breaks a line, so a NaN leaves a hole in the surface.

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

`Config::view_matrix` is the one camera matrix. The GPU uniform, the box
grid and axes, the hit tests and the drags all read it.
`Config::project_3d` returns the pixel position and depth of a box point,
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

| Field | Default | Meaning |
|---|---|---|
| `color_matrix` | `None` | A second grid of the same shape to colour by; `None` colours by height |
| `wireframe` | `None` | `SurfaceWireframe { color, width }`: grid lines along the rows and columns, `width` in logical pixels, constant on screen |
| `material` | `Flat` | `Flat` is unlit; any other `Material` is lit with shading normals |
| `gloss`, `texture_strength`, `texture_scale` | 0.45, 0.2, 1 | The material's parameters |
| `emphasis_brightness` | 0.12 | The brightness the picked sample's patch gains |

A 3D chart draws only these three kinds, in the Precise draw style. It
rejects overlays, line offsets, the polar and ternary systems and streamed
sources, each with an explicit error. There is no limit on the grid's
size: every quad is drawn.

## Interaction

- **Hover and click.** A 1×1 ID pick renders what the cursor's pixel
  shows, with the depth test, and answers the grid sample nearest the
  fragment inside its quad: `PickedData::MatrixCell` with the sample's
  column and row. A sample hidden behind the surface is never answered.
  A floor contour answers its level, as on a flat chart.
- **Highlight.** The picked sample's patch, the halves of the quads around
  it nearest the sample, is outlined and brightened. It is drawn over
  anything in front of it, so a selection is visible from every side.
- **`step_selection`** walks the grid samples, as on a flat heatmap.
- **`selection_details()`** reports the sample's x, y and height, plus the
  colour grid's value when set.
- **Camera.** Dragging the data area orbits the camera
  (`Config::orbit_view`, half a degree per pixel). `Config::zoom_view`
  zooms it (the WASM kernel's `zoom_view(factor)`). The data area's edges
  still resize it.
- **Decoration drags.**
  - A box axis (`Element::Axis`) moves along its edge's outward screen
    normal (`line_offset`) and carries its ticks and labels.
  - Tick labels (`Element::AxisLabel`) move in the edge frame:
    `label_offset_x` along the edge and `label_offset_y` along its normal.
  - Titles (`Element::AxisTitle`) move in screen space.
- **Fit.** `auto_fit_layer_union` and the browser's `autoFitAll` also fit
  `right_y` to the union of the height columns' statistics. Nothing is
  read back.
- **Annotations.** Ratio and pixel anchors draw. A 3D chart has no
  separable value axes, so data anchors, reference lines and spans are
  skipped as `NoValueAxis`.
- **Subplots and export.** A subplot's `config.view_3d` makes that panel 3D
  beside flat ones. The chart exports to PNG at any scale, and `view_3d`
  serializes when on and is omitted when off.

## Drawing

The surface draws into the panel composite with a depth-stencil
attachment (`Depth24PlusStencil8`, at the target's sample count), created
only for a 3D panel. Floor contours draw first without writing depth, then
the surfaces with the depth test, so a translucent surface shows the
contours below it. The 2D pipelines and the shared frame and export passes
are unchanged.

Expected memory: 4 bytes per sample of the attachment, about 12 MB for an
1100 × 700 panel at 4x MSAA.

## Tests

```sh
cargo test --locked -p figgy-renderer --features serde,cpu-twin --test surface_render
```
