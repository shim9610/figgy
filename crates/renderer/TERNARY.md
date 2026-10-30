# Ternary plots

A ternary chart shows samples of three components that share a whole:
alloy compositions, soil textures, phase diagrams, vote shares. Set
`Config.ternary` to `Some(TernaryOptions {})` and draw `DataRenderType::Ternary`
series. The data area then holds an equilateral triangle whose corners are
the 100 % of each component. `ternary: None` (the default, omitted from
JSON) draws the rectangular chart, so older documents are unchanged.

## Input and first render

A ternary series names three ordinary pool columns: `x_column`, `y_column`
and `c_column`. Each enters through `ColumnSource` like any other column.
There is no record type and no third column on `SeriesConfig`; `c_column`
lives on the series kind.

```rust,no_run
use renderer::{
    Chart, Color, Renderer, ScatterShape, SeriesConfig, TernaryOptions,
    data::Column,
    data_config::{DataRenderType, DataScatterStyleConfig},
    default,
    layout::{ChartArea, Rect},
};

fn draw(renderer: &mut Renderer) -> Result<(), Box<dyn std::error::Error>> {
    // The host's components, unnormalized: no projection before upload.
    renderer.add_column("fe", &Column { data: vec![70.0, 20.0, 10.0], min: 10.0, max: 70.0 })?;
    renderer.add_column("ni", &Column { data: vec![20.0, 70.0, 20.0], min: 20.0, max: 70.0 })?;
    renderer.add_column("cr", &Column { data: vec![10.0, 10.0, 70.0], min: 10.0, max: 70.0 })?;

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect { x: 0, y: 0, width: 800, height: 640 });
    config.ternary = Some(TernaryOptions {});
    for axis in [&mut config.bottom_x, &mut config.left_y, &mut config.right_y] {
        axis.min = 0.0; // every axis spans the whole triangle
        axis.max = 100.0;
        axis.major_spacing = 20.0;
    }
    // The c axis is drawn like the other two.
    config.right_y.label_style.label_visible = true;
    let chart = Chart::new(config).with_title("Alloys");

    let series = [SeriesConfig {
        series_id: "alloys".into(),
        source_id: None,
        label: None,
        x_column: "fe".into(), // x: the bottom-left corner
        y_column: "ni".into(), // y: the top corner
        render_type: DataRenderType::Ternary {
            c_column: "cr".into(), // c: the bottom-right corner
            scatter: Some(DataScatterStyleConfig {
                point_color: Color::from_rgb8(40, 90, 200),
                point_shape: ScatterShape::CircleFilled,
                point_size: 6.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            }),
            line: None,
        },
    }];
    let image = renderer.export_panel_rgba(&chart, &series, 1.0)?;
    std::fs::write("ternary.png", renderer::encode_png(&image)?)?;
    Ok(())
}
```

A sample is placed at `(x·L + y·T + c·R) / (x + y + c)`: the triangle's
corners weighted by its components. Normalizing only to place a sample is
appearance: it is computed in the vertex stage on every draw and never
stored. Hover, selection and `selection_details()` report the original
components.

A sample with a negative or non-finite component, or a sum of zero or less,
is not drawn, and a line breaks on both of its sides, as at a NaN.

## The convention

One convention, with no option to change it:

| Component | Column | Axis | Edge | 100 % corner |
|---|---|---|---|---|
| x | `x_column` | `bottom_x` | bottom, from R to L | L, bottom-left |
| y | `y_column` | `left_y` | left, from L to T | T, top |
| c | `c_column` | `right_y` | right, from T to R | R, bottom-right |

Each axis increases clockwise towards its own corner. Lines of constant x
run parallel to the right edge, constant y is horizontal, and constant c
runs parallel to the left edge. `top_x` stays in the document but is not
drawn.

The three axes span the whole triangle: each has `min` 0 and the same
`max`, 1 or 100 (which only scales the labels). The axes must be linear,
not inverted and not categorical. These rules are checked where the chart
is drawn, as a polar chart's are. A fit (`auto_fit_layer_union`,
`auto_fit_x_extent`, the browser's `autoFitAll`) leaves the axes as they
are: their range is the triangle, not a data extent.

## Series

A `Ternary` series draws markers (`scatter`), a line (`line`) or both, and
names three distinct columns. The marker styles, style tables, overrides
and the colour and size maps work as on a scatter series. The line takes
every line style and dash pattern. It is linear: step modes are rejected.
It has no fill: `LineFill` fills towards a y value or a second series that
shares an x column, and neither means anything inside a triangle. Region
fills (a phase field, a soil class) are planned with the depth-stencil
attachment of #49.

A ternary chart draws only ternary series, in the Precise draw style. It
rejects overlays, line offsets, the polar system, subplot axis links to or
from it, and streamed sources, each with an explicit error.

## Layout

`Config::ternary_geometry()` returns the triangle: the largest one inside
the data area that leaves a band for each edge's ticks, labels and title,
centred with those bands. Every band term scales with `Config::scaled`, so
a 2x export is the 1x layout doubled. The GPU transform, the grid and the
axes, the hit regions and the drags all read this one triangle.

- **Grid.** At each tick of each axis, a line of constant value runs
  across the triangle. The x family takes the grid's x styles; the y and c
  families take its y styles.
- **Edges.** Each edge is its axis: the line, ticks along the edge's
  outward normal (outside, inside or both) and upright tick labels centred
  past the ticks. A corner shows only its own component's 100: the 0 of
  the edge that starts there would sit on it.
- **Titles.** Each axis title sits past its edge's labels at the edge's
  midpoint, parallel to the edge and reading left to right
  (`Config::ternary_title_placement`).

## Interaction

- **Hover and click.** Markers, strokes and dashes pick where they paint:
  the pick reads the c column from the pool and places the sample with the
  draw's own projection. A ternary series always takes the direct scan (one
  exact pass over its points), because the placement is not separable by
  axis. The result is `PickedData::Point` with the sample's index.
- **Highlight.** The picked point's ring is drawn at the marker's place and
  radius.
- **`step_selection`** walks the points; the count is the shortest of the
  three columns.
- **`selection_details()`** reports `x`, `y` and `c`, plus `color_value` or
  `size_value` when mapped.
- **Decoration drags.**
  - An edge (`Element::Axis`) moves along its outward normal by the drag's
    component on it, stored in `line_offset`, and carries its ticks and
    labels.
  - Tick labels (`Element::AxisLabel`) move in the edge frame:
    `label_offset_x` along the edge (from its 0 to its 100 % corner) and
    `label_offset_y` along the outward normal.
  - Titles (`Element::AxisTitle`) move in screen space.
  - The slanted edges and their labels hit on their drawn parts, not on a
    bounding box. The top axis is neither drawn nor hit.
  - The data area moves and resizes as on a rectangular chart, and the
    triangle refits.
- **Annotations.** Ratio and pixel anchors draw. A ternary chart has no
  separable value axes, so data anchors, reference lines and spans are
  skipped as `NoValueAxis`, as on a polar chart.
- **Subplots and export.** A subplot's `config.ternary` makes that panel
  ternary beside rectangular ones. The chart exports to PNG at any scale
  and to SVG and PDF, and `ternary` serializes as `{}` when on and is
  omitted when off.

## Examples and tests

The playground's Ternary scene (`tests/support/playground_scenes.rs`) draws
Fe/Ni/Cr alloy compositions coloured by hardness through a colourbar, with
a dashed tie-line. The decoration oracle sweeps it like every other scene.

```sh
cargo test --locked -p figgy-renderer --features serde,cpu-twin --test ternary_render
```
