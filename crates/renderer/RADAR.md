# Radar charts

`RadarRenderer` draws radar (spider) charts: a few series compared over a
fixed set of categories, one spoke per category, each series a closed polygon
over one shared value axis. It is a bounded family like `RadialRenderer`,
`CategoricalRenderer` and `BoxPlotRenderer`: the whole chart is a small
`RadarChart` value, drawn by its own pipeline into an immutable frame, with no
GPU column pool, auto-fit job or stream replay.

## Input and first render

```rust,no_run
use renderer::radar::{RadarChart, RadarRenderer, RadarSeries};
use renderer::{Category, Color, RendererDevice};

fn draw(device: RendererDevice, format: wgpu::TextureFormat) -> Result<(), Box<dyn std::error::Error>> {
    let mut chart = RadarChart::default();
    chart.title = "Build quality".into();
    chart.categories = ["Speed", "Safety", "Size", "Docs", "Tests"]
        .iter()
        .map(|c| Category::new(c.to_lowercase(), *c))
        .collect();
    chart.series = vec![
        RadarSeries::new("v1", "Release 1", vec![6.0, 8.0, 4.0, 5.0, 7.0], Color::from_rgb8(60, 110, 200)),
        RadarSeries::new("v2", "Release 2", vec![8.0, 7.0, 6.0, 8.0, 9.0], Color::from_rgb8(220, 90, 70)),
    ];
    let mut renderer = RadarRenderer::new(device, format)?;
    let frame = renderer.prepare(&chart, (720, 520), 1.0)?;
    // In a render pass of a 720 x 520 attachment in `format`:
    // frame.draw(&mut pass);
    let _ = frame;
    Ok(())
}
```

- `categories[i]` owns spoke `i`; `series.values[i]` is the value on it. The
  spokes start at `style.start_angle_degrees` (-90, straight up) and follow
  `style.clockwise`. 3 to 64 categories and 1 to 16 series.
- The value axis is shared by every spoke. `value_range: None` fits the
  supplied values on a nice range that starts at zero (or below the smallest
  negative value) and ends on a ring; `Some([min, max])` fixes it. Values below
  the start sit at the centre, values above the end on the outer ring.
  `ring_spacing` chooses the ring interval; `None` picks a nice spacing for
  about five rings. Per-spoke scaling is the host's job: write normalized
  values and label the spokes accordingly.
- A non-finite value breaks the outline at that spoke, drops the series' fill
  and draws no marker there.
- `style.grid_shape` draws the rings as polygons through the spokes
  (`Polygon`, the spider web) or as circles. `grid: false` hides rings and
  spokes.
- Each series draws a fill (`RadarSeriesStyle::fill`, or the series colour at
  `style.fill_opacity`), an outline of `line_width_px` and a marker at every
  vertex (`Circle`, `Square` or `None`). Fills draw first for every series,
  then outlines, then markers, so a later series' fill never covers an earlier
  outline.
- `labels` writes the value at every vertex (`Values`) or at the vertices of
  the hovered or selected series only (`Emphasized`), formatted with
  `label_decimals` and `value_suffix`.

## Picking and selection

`RadarFrame::hit_test(position)` takes chart-local logical pixels and returns a
`RadarPick { target, part }`: a `Marker` hit names the vertex
(`RadarTarget { series_id, category_id: Some(..) }`), an `Outline` or `Fill`
hit names the series (`category_id: None`). Markers are tested first, then
outlines, then interiors, each in reverse draw order. Store the target in
`hovered` / `selected`: the series brightens by `style.emphasis_brightness`
and a picked vertex grows its marker. `RadarChart::value(&target)` reads the
value behind a vertex target.

## Decorations and annotations

The title, the value labels along the first spoke, the category labels at the
spoke ends (`DecorationId::SpokeLabel { category_id }`), the legend and the
data labels are measured decorations: `frame.decorations()` is the common
`HitMap` / `Draggable` contract, so they drag and reset like every other
bounded family's. The data area resizes the whole plot; the outer ring is the
largest circle that leaves room for the category labels inside it.

The annotation layer draws around the frame through `prepare_annotations`.
Anchors are ratios of the square around the outer ring; a data anchor has no
separable axis and is reported as a skip.

## Caching and export

`prepare` reuses its frame while every field the pixels depend on is unchanged
(`active_layer` and the annotation layer are not among them) and reuses the
label raster across hover and selection edits. `export_rgba_async` renders at
any scale and reads back straight-alpha RGBA8; `renderer::encode_png` encodes
it.

## Examples and tests

```bash
cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery
cargo run --locked -p figgy-renderer --features egui_demo --example radar_editor
cargo test --locked -p figgy-renderer --test radar_render
```
