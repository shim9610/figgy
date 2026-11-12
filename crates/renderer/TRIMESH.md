# Triangle meshes

A triangle mesh series draws a field over triangles the host gives: the
mesh of a finite element or finite volume solver with a pressure or a
temperature per node or per element, or scattered measurements the host
triangulated. It is matplotlib's `tripcolor`, `tricontour` and `trisurf`
in one series kind, `DataRenderType::TriMesh(TriMeshSeries)`
([#210](https://github.com/shim9610/figgy/issues/210)): a colour fill,
contour lines, or both, on a flat chart, and a surface in the box of a 3D
chart (`Config.view_3d`, [SURFACE.md](SURFACE.md)). The renderer never
triangulates and never resamples the mesh onto a lattice: a mesh with a
hole (flow around a cylinder) or a curved boundary draws as the host meshed
it.

The host supplies the vertex coordinates, the triangles, the values and, on
a 3D chart, the heights; the renderer places the vertices through the axes
or the camera, interpolates the given vertex values across each triangle,
draws the level set of that interpolant as the contour lines, maps the
values through the colourbar and shades a lit surface with each triangle's
own normal. The mesh lives in the GPU column pool.

## Input and first render

A mesh series names four pool columns: `x_column` and `y_column` are the
vertex coordinates, one row per vertex; `values.column` holds one value per
vertex or per triangle; and `triangles` names a triangle index column,
three vertex rows per triangle in triangle order, uploaded with
`Renderer::upsert_triangles`. The coordinate and value columns enter through
`ColumnSource` like any other column; the index column enters through
`TriangleSource` over a `Column<u32>` (or any `ColumnSource` whose values
are integers up to `u32::MAX`). A `Config.colorbar` is required, as for a
heatmap: it maps the values to colours.

```rust,no_run
use renderer::{
    Chart, Color, Renderer, SeriesConfig, TriangleSource,
    data::Column,
    data_config::{
        ContourConfig, DataLineStyleConfig, DataRenderType, FieldFillConfig, FillMode,
        LineInterpolation, Shading,
    },
    default,
    layout::{ChartArea, Rect},
    line::LineStylePreset,
    tri_mesh::{MeshValueLocation, MeshValues, TriMeshSeries},
};

fn draw(renderer: &mut Renderer) -> Result<(), Box<dyn std::error::Error>> {
    // The host's mesh: four vertices, two triangles, one value per vertex.
    renderer.add_column("x", &Column { data: vec![0.0, 1.0, 0.0, 1.0] })?;
    renderer.add_column("y", &Column { data: vec![0.0, 0.0, 1.0, 1.0] })?;
    renderer.add_column("p", &Column { data: vec![0.0, 1.0, 1.0, 2.0] })?;
    // Three vertex rows per triangle, in triangle order.
    renderer.upsert_triangles(
        "tri",
        &TriangleSource { indices: &Column { data: vec![0u32, 1, 3, 0, 3, 2] } },
    )?;

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect { x: 0, y: 0, width: 800, height: 640 });
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.min = -0.1;
        axis.max = 1.1;
        axis.major_spacing = 0.5;
    }
    let mut bar = default::default_colorbar_options();
    bar.axis.min = 0.0;
    bar.axis.max = 2.0;
    let mut contour = ContourConfig {
        levels: vec![],
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::from_rgb8(30, 30, 30),
            line_width: 1.0,
            fill: None,
            fill_color: None,
            interpolation: LineInterpolation::Linear,
        },
        per_level_color: None,
        labels: None, // contour labels are refused on a mesh
    };
    contour.set_nice_levels(&bar.axis, 4)?;
    config.colorbar = Some(bar);
    let chart = Chart::new(config).with_title("Pressure");

    let series = [SeriesConfig {
        series_id: "pressure".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None, // Some("p".into()) on a 3D chart: the vertex heights
        render_type: DataRenderType::TriMesh(TriMeshSeries {
            triangles: "tri".into(),
            values: MeshValues {
                column: "p".into(),
                at: MeshValueLocation::Vertices,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            }),
            contour: Some(contour),
            surface: None, // the look on a 3D chart; None draws SurfaceStyle::default()
        }),
    }];
    let image = renderer.export_rgba_blocking(renderer::ExportSource::chart(&chart, &series), renderer::ExportOptions::panel(1.0))?;
    std::fs::write("mesh.png", renderer::encode_png(&image)?)?;
    Ok(())
}
```

`upsert_triangles` replaces an index column of the same id and refuses a
count that is not a non-zero multiple of three, an index that is not an
integer in `0..=u32::MAX`, and a streamed id; `remove_triangles`
removes the column. The pool keeps the triangle count and the smallest and
largest index as scalar statistics. Registering
the series checks the largest index against the vertex count, the shortest
of the coordinate columns, the value column for vertex values and, on a 3D
chart, the height column, and refuses an index past it by naming the
triangle, the index and the column. A column shortened by a later upload is
drawn to what remains, dropping every triangle that names a vertex past it,
and `series_draw_info` reports `truncated`. The fit (`auto_fit_all`) covers
every registered vertex's coordinates, referenced by a triangle or not.

In the browser the same mesh is `register_triangles(id, Uint32Array)` and a
`"TriMesh"` series in the JSON schema
([SCHEMA.md](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md#trimesh-render_type),
[WASM.md](WASM.md)).

## Series

`TriMeshSeries { triangles, values, fill, contour, surface }`
([configuration reference](CONFIG.md#triangle-meshes)):

- `triangles` names the triangle index column.
- `values: MeshValues { column, at }` names the value column and says where
  its values sit: `MeshValueLocation::Vertices` (row `i` is vertex `i`'s
  value) or `Faces` (row `i` is triangle `i`'s). It is never inferred from
  the column lengths.
- `fill: Option<FieldFillConfig>` colours the triangles through the
  colourbar: `Continuous` or `Bands` (the band of the levels the value
  reaches). Vertex values are shaded `Interpolated`, linearly across each
  triangle, and face values `Flat`. The other pairings are refused: for a
  flat colour per triangle, upload face values; for an interpolated colour,
  upload vertex values.
- `contour: Option<ContourConfig>` draws contour lines over the fill, or on
  the box floor of a 3D chart. They need vertex values (a field constant on
  each triangle has cell boundaries, not isolines) and carry no labels.
- `surface: Option<SurfaceStyle>` is the look on a 3D chart, kept and
  unused on a flat one: a material, a wireframe and the surface knobs of a
  heatmap's surface, but no `color_matrix`, because the values colour the
  mesh. `None` draws `SurfaceStyle::default()`.

A series has a fill, contour lines or both, and a surface look only with a
fill. Polar, ternary and parallel charts refuse the kind, as they refuse
fields. On a 3D chart the series names its `z_column` and its fill is
continuous. `Config::validate` (through `TriMeshSeries::validate` and
`Config::validate_tri_mesh_series`) and series registration report every
rule by name. Streamed displays refuse the kind, and `upsert_triangles`
refuses a streamed id.

The legend symbol is the heatmap's swatch. Every mesh series of a chart is
coloured through the one colourbar; `Chart::auto_fit_colorbar_union` fits it
to the value columns' statistics.

## Drawing

The fill draws one instance per triangle, three vertices each, with no
vertex buffer: each vertex reads its triangle's three indices and the three
vertices' coordinates and values from the pool and places itself through
the axes, log, probability and broken axes included. A triangle that names a
vertex past the shortest vertex column or has a vertex the axes cannot
place (a NaN coordinate) is dropped, as a NaN breaks a line. Vertex values
blend barycentrically in screen space, so in axis `t` on a log axis, as a
heatmap's interpolation does; a non-finite vertex value paints its triangles
`nan_color`. The fragments are opaque, without coverage antialiasing: under MSAA
every sample of a shared edge is covered by exactly one triangle, so an
interior edge shows no background.

The contour lines are the level set of each triangle's linear interpolant,
drawn per fragment as the heatmap's are: the distance to a level is
`|f − L| / |∇f|` in output pixels, with the triangle's gradient from its
three projected vertices (an analytic flat value, never a derivative of the
raster), and the heatmap's level search over the colourbar's levels. The
interpolant is continuous across edges, so a stroke continues into the
neighbouring triangle, and at the mesh boundary half the stroke is clipped,
as at a heatmap's grid edge. A plateau (zero gradient) draws nothing.

Only the precise draw style draws a mesh, as it alone draws fields;
`draw_style_supports` answers `false` under the stylized styles.

## On a 3D chart

With `Config.view_3d` set ([SURFACE.md](SURFACE.md)) the mesh draws as a
surface over its triangles, a trisurf. The series names its height column
in `SeriesConfig.z_column`, read on `right_y` as a heatmap surface's heights
are; each triangle's three vertices are placed by `x_column`, `y_column`
and `z_column` through the camera, and the fill colours the triangle by
its values, interpolated for vertex values and flat for face values. The
height and the value may be the same column (a pressure drawn as a pressure
surface) or different ones (a temperature over a terrain). `surface` holds
the look: a lit material shades with the triangle's own normal, taken from
its three box points, and a wireframe outlines
the triangles. The contours draw on the box floor, where the level set of
each triangle's interpolant crosses it, with no labels.

`Chart::auto_fit_view_3d_z` spans the mesh's heights with the other 3D
series' `z_column`s; `Config::drawn_z_column` names the column. The camera
orbits with a data-area drag (`Config::orbit_view`) and zooms with
`Config::zoom_view`, and the box axes drag as on any 3D chart. `z_column:
None` on a 3D chart is refused at validation, as it is for a point series
in the box.

## Interaction

Every interaction of a Cartesian chart applies
([INTERACTION.md](INTERACTION.md)).

- **Hover and click.** On a flat chart `pick_chart_data` answers
  `PickedData::MeshTriangle { series_id, triangle_index, vertex_index,
  distance_px }`: the triangle under the cursor's pixel (distance 0) or
  within one output pixel of its edge, where the fill paints; where
  triangles overlap the later one answers, since it is drawn on top.
  `vertex_index` is the triangle's vertex nearest the cursor, a hover
  answer the selection does not keep. A contour stroke within the pick
  distance answers first, as `PickedData::ContourLevel` with the level and
  the triangle in `x_index` (`y_index` is 0). The pick walks every triangle
  on the GPU and reads the draw's own snapshot of the mesh, so it picks
  where it paints. On a 3D chart the 1×1 ID pick renders what the pixel
  shows with the depth test: the surface answers the triangle the pixel
  shows and the vertex nearest the fragment, a floor stroke its level with
  the triangle, and never a triangle hidden behind the surface.
- **The picked highlight.** The picked triangle is outlined inside its
  edges at the selection's `outline_width_px` in `highlight_color`; a
  picked level is redrawn over the whole mesh `contour_width_extra_px`
  wider. On a 3D chart the picked triangle is the surface's patch, outlined
  and brightened over anything in front of it.
- **`step_selection`** moves a picked triangle by `dx` over the triangles
  of its series (the triangle count, from the index column's statistics)
  and by `dy` across the chart's mesh series in series order; a contour
  level does not step.
- **`selection_details()`** reports the triangle's three vertex indices and
  their coordinates and values (and, on a 3D chart, their heights). The
  browser's details carry `index`, `vertices`, `x`, `y`, `values` (one per
  vertex; `value` for face values), `z` on a 3D chart and `at`.
- **Decorations and the plot area.** The title, legend, axes, tick labels,
  axis titles and colourbar drag as on any chart, and the plot area moves
  and resizes; a geometry edit changes the transform only and re-uploads
  nothing. On a 3D chart the data-area drag orbits the camera.
- **Annotations, overlays and subplots.** The annotation layer places data
  and ratio anchors over the mesh and draws above it; a mesh on an overlay
  `y_axis` draws and picks against that axis; a mesh in a subplot cell
  draws and picks in that cell. 3D meshes follow the 3D chart's rules:
  ratio anchors only, a camera per subplot, no overlays.
- **Export and documents.** PNG at any scale (`Config::scaled` scales the
  stroke, the outline and the MSAA), SVG and PDF with the mesh in the data
  image, and a serde round trip of the series and the `mesh_triangle`
  selection, both additive, so older documents parse and re-serialize
  unchanged.
- **SSOT invalidation.** A series edit (fill, contours, values, the surface
  look) rebuilds only the mesh's own scratch, and `upsert_triangles` or a
  vertex column upload moves the chart's data revision; the next frame
  draws the new mesh with no other upload.

## Cost and limits

Any number of triangles, vertices and mesh series can be drawn. A mesh
fails only at registration, when the pool cannot allocate the column or the
device cannot hold it. The draw and pick cost grows with the mesh: on
lavapipe (a software Vulkan adapter) a million triangles export at 1x in
about 0.9 s and pick in about 50 ms. A chart can hold more mesh series than
the 256-entry scratch cache, which evicts old entries and never refuses a
series. Index memory is 24 bytes per triangle (two lanes per index, as for
every pool column).

## Examples and tests

- The playground scenes `SceneKind::TriMesh` (potential flow past a
  cylinder on a graded annulus mesh, a mesh with a hole: the pressure
  coefficient per node, interpolated, with contour lines) and
  `SceneKind::TriSurface` (the same mesh as a matte, wireframed surface in
  a 3D box with floor contours) in `tests/support/playground_scenes.rs`.
- `tests/tri_mesh_render.rs`: the fill, the contours, the pick, the
  highlight, `step_selection`, registration and its refusals, the
  interactions, and the million-triangle draw and pick.
- `tests/surface_render.rs`: the 3D surface, its ID pick, highlight, floor
  contours, lit shading, z fit and height re-upload.
- `crates/web/tests/tri_mesh_browser.rs`: the browser's
  `register_triangles`, picks, details and steps, flat and in a 3D box.
