# Rust API contracts

## Package names and features

The Cargo packages are `figgy-renderer` and `figgy-model`, and their Rust
library names are `renderer` and `model`: code imports `renderer::...` and
`model::...`, and the dependency names its package with the `package` key:

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.18.1" }
```

A crate that uses the model types without the renderer depends on
`model = { package = "figgy-model", version = "0.12.1" }`. To build against a
local checkout, replace `version` with a `path` to `crates/renderer` or
`crates/model`.

The renderer re-exports model types. Enable its `serde` feature to serialize
those types; consumers do not need to name `figgy-model` merely to enable serde.
`egui_demo` enables optional GUI example dependencies. Default features do not
require a windowing or GUI framework. The `figgy` WASM wrapper, figgy's browser
product, is a separate consumer and has `publish = false` for the Rust registry.

Rust 1.99 is the declared support floor (`rust-version`), not a claim that
earlier toolchains cannot compile the code. CI tests with the toolchain pinned
in the source repository's `rust-toolchain.toml` (1.99.0). Direct host
device/queue/render-pass sharing requires wgpu 30; a host on another wgpu major
must adapt or upgrade its integration.

For a complete headless Cartesian program, run the shipped `line` example
(`cargo run --locked -p figgy-renderer --example line -- line.png` in a source
checkout). The source repository's [example index](https://github.com/shim9610/figgy/blob/master/crates/renderer/examples/README.md)
maps each chart to its input, output and command. Gallery images are repository
assets; they are not runtime resources required by the crate.

macOS / Metal support is **experimental and not yet stable**: physical Apple GPU
coverage is outstanding. The source repository's opt-in precompiled Metal shader
probe is not a Cargo feature, does not change the default renderer and is not a
precision guarantee; applications need no dependency patch or precompiled shader.
Tested revisions and results are in the
[backend status](https://github.com/shim9610/figgy/blob/master/ci/STATUS.md).

## Entry points and input contracts

| Renderer | Input | Frame preparation |
| --- | --- | --- |
| `Renderer` | `Config`, ordered `SeriesConfig`, registered numeric columns | `prepare` or `prepare_registered` |
| `RadialRenderer` | `RadialChart`, bounded slice values | `prepare` |

The bounded radial chart carries every value in the chart model, up to fixed
limits such as 64 slices. Its API is separate from `ColumnSource` and
streaming, and it does not calculate raw-sample statistics. Limits, missing
values, style inheritance and picking identities are documented in its module.
Radial targets use group/index positions, so a host that reorders slices must
update interaction targets too.

A polar plot is not a renderer of its own: `Config.polar = Some(PolarOptions)`
makes `Renderer` draw the same columns and `SeriesConfig` in (angle, radius),
with `bottom_x` as the angle axis and `left_y` as the radius axis. One turn
always spans 360° (or 2π) from `zero_angle_degrees`: the angle mapping ignores
`bottom_x.min` / `max`, which only choose the angle ticks, spokes and labels
drawn. Preparation, picking, export and subplots use the same calls as a
rectangular chart. Decorations follow polar rules: the outer ring registers no
hit region, and the radius ray turns around the centre when dragged.
Annotations draw only ratio-anchored text and arrows; reference lines, spans
and data-anchored items are skipped and reported as
`RenderWarning::AnnotationSkipped`.
Scatter, line and scatter-line series are drawn, including fills and steps.
Histograms, fields, contours, error bars, vectors, overlay layers, a
logarithmic or inverted angle axis, an arc interpolation on a step line and
streamed sources are rejected with explicit errors. See the
[polar guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/POLAR.md).

A radar chart is a polar chart with a categorical angle axis
(`bottom_x.categories`) and `DataRenderType::Radar` series: closed outlines
through the category spokes, with an optional fill and vertex markers, picked
by vertex as `PickedData::Point`. A legacy `RadarChart` document opens through
`Config::from_radar`, which returns the `Config`, the series and the columns to
upload (category positions and one value column per series) borrowed from the
document. Radar series reject streamed sources. See the
[radar guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/RADAR.md).

`ColumnSource` uploads each value as one `f32`, stored as an `(f32, 0)` pair;
`HiLoColumnSource` uploads `(hi, lo)` `f32` pairs that preserve sub-f32 deltas
for large coordinates such as Unix timestamps. Upload with `Renderer::add_column`,
`add_columns` or `upsert_column`, and with `add_hilo_column` or
`upsert_hilo_column` for hi/lo sources. Native upload avoids an intermediate CPU
conversion vector but still copies staging data to GPU storage. It is not a
promise of zero driver copies. Use the hi/lo path explicitly when precision
requires it.

Streamed columns (`StreamColumn`) declare `StreamReplay::Sequential` or
`RandomAccess`. `Unavailable` is rejected: the renderer keeps a bounded working
set, so exact redraws, zooms and queries read the source again. Every replay
must return the same encoded values for the same source revision. The host owns
and retains the replayable source for the operations it enables.

Automatic streaming renders the registered range unless explicitly asked to fit.
Use `request_stream_auto_fit(chart_id, padding)` for the next automatic execution
when the initial range is unknown, or when the user requests a fresh fit. Do not
call it for every chunk, repaint, tab switch or aspect-ratio change. Manual streaming
and resident drawing do not consume that request. Initial fit previews can be
approximate. In `StreamingStatus`, `status` reaches `StreamingState::Complete`
with the final render, while `AllSubmitted` only means that every input primitive
has been submitted; `auto_fit_pending` stays true until a requested fit is final.

## Direct SSOT edits and convenience helpers

`Config` is the single source of truth (SSOT). For precise control, start from
the current config and change only the fields you intend to change. Direct
edits do not run range fitting, choose tick spacing or enlarge the title band.
For example, change an axis's `min` and `max` to pan or zoom while preserving
its `major_spacing`, `minor_count` and label format. Update both axes of a pair
only when they should share the new range; they remain independent fields.

`Chart` convenience helpers perform the related adjustments documented by each
method. They are useful for automatic setup and fitting, and are not all simple
field setters:

| Operation | Config changes |
| --- | --- |
| `config_mut()` / `with_axis_range_change(...)` / `with_decoration_change(...)` | Only the fields the caller writes. `config_mut` and `with_axis_range_change` also set the chart's `data_dirty()` and `raster_dirty()` flags; `with_decoration_change` sets only `raster_dirty()`. |
| `set_x_range(...)` / `set_y_range(...)` | Set both axes in the pair and choose major spacing, minor count, label format and precision from the bottom/left axis's scale. The spacing is `tick::axis_major_spacing`: 0.2, 0.5 or 1 × 10^k of the span on a linear axis (5 to 10 major intervals), and on a logarithmic axis 1 decade per major tick up to 6 decades, 2 up to 12, then `ceil(decades / 6)`. On a linear axis an existing timestamp format is kept and any other format, including a polar `Degrees` or `Radians` format, becomes `Decimal`. On a polar chart `set_x_range` only chooses the angle ticks, spokes and labels drawn. |
| `auto_fit_x*` / `auto_fit_y*` / `auto_fit_colorbar*` | Derive a range from data or supplied extents with padding, then apply the associated tick and label adjustments. These and `set_x_range` / `set_y_range` leave a categorical axis (`categories` set) unchanged. |
| `with_title(...)` | Replace the title text and grow its band when the measured text is taller than the current band. The band never shrinks. |
| `with_x_title(...)` / `with_y_title(...)` | Replace the bottom/left axis title text; these helpers do not fit margins. |

Fit padding is an explicit caller argument: `0.05` adds 5% on each side of a
linear range, while `0.0` adds no padding. The renderer does not impose a global
5% default; a binding or UI may choose its own convenience default.

To change only the X range while keeping the current tick settings:

```rust
use renderer::{Chart, default::default_config};

let mut chart = Chart::new(default_config());
let ticks = chart.config().bottom_x.major_spacing;
chart.with_axis_range_change(|cfg| {
    for axis in [&mut cfg.bottom_x, &mut cfg.top_x] {
        axis.min = 20.0;
        axis.max = 80.0;
    }
});
assert_eq!(chart.config().bottom_x.major_spacing, ticks);
```

Use `with_axis_range_change` for range/scale edits, `with_decoration_change` for
decoration edits, or `config_mut` for general edits. For a title change that must
keep the exact margin, assign `cfg.chart_title.text.segments` inside a decoration
edit instead of calling `with_title`. Renderer-owned charts require submitting
the edited config through `Renderer::set_chart_config`; changing a detached
copy does not update the registered chart.

Keeping tick settings does not keep ticks at the same screen coordinates or
guarantee the same number of visible labels: those depend on the new range and
available space. Raster safety limits can still substitute unusable tick settings
without rewriting the config; read `ChartView::render_warnings()` after refresh.
Numeric tick labels may use a coarser display stride to avoid overlaps, without
changing tick/grid positions or the config. Number format and requested precision
are controlled by `label_style.format` and `label_style.significant_digits`,
including colorbar labels; unusually long labels still need explicit margin space.
Calendar-based timestamp ticks follow their own layout policy. The renderer
still validates a directly edited config when it registers, refreshes, prepares
or exports the chart. After any edit, refresh the derived view as described below.

A Cartesian axis becomes categorical when its `categories` is set: with `n`
categories it maps the data range `[-0.5, n - 0.5]`, category `i` sits at
x (or y) `i`, every category has a major tick (and a grid line when that
axis' major grid is on), and the tick label is `categories[i].label` in the
axis' `label_style`, thinned like numeric labels when neighbours crowd. Upload
the data of a categorical axis as category indices `0..n`. `min`, `max`,
`major_spacing`, `minor_count` and `label_style.format` are ignored there,
and `set_x_range` / `set_y_range`, the `auto_fit_*` helpers and view states
leave the axis unchanged. A categorical axis must be linear; `inverted` walks
its categories from the far end (the first category at the top of a `left_y`),
with 1 to 64 categories whose ids are unique; polar axes and the colourbar
axis cannot be categorical yet, and streamed series are rejected on a chart
with a categorical axis.

Categorical bars are `DataRenderType::CategoryBars` series over the
categorical axis of their direction (`bottom_x` for vertical bars, `left_y`
for horizontal ones), laid out by `Config.bar_groups` (grouped, stacked or
percent, the group width and the gap). Each reads a positions column
`0..n` and a values column of one value per category; NaN is a missing
bar. `AxisOptions::cross_at` puts the category axis line at a value of the
value axis. `auto_fit_layer_union` fits the value axis to what the bars
draw; a stacked group's extent is measured by the pool when its value
columns are uploaded together with `add_columns`. A bar is picked as
`PickedData::HistogramBin` with the category row, and selected through
`PickedDataRef::HistogramBin`. `Config::from_categorical` opens a legacy
`CategoricalChart` document as such a chart; see the
[bar guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/CATEGORICAL.md).

Box plots are `DataRenderType::BoxPlot` series over the same categorical
axes and grouped `Config.bar_groups`. Each reads a positions column `0..n`, a
record of box statistics (`Renderer::upsert_record` with the
`BOX_PLOT_RECORD` schema: median, quartiles, whiskers and the optional mean,
count and median CI, one row per category) and, optionally, a ragged column of
outliers (`Renderer::upsert_ragged`, one group per category). The renderer
does not compute statistics or check their order; the drawing clamps.
`auto_fit_layer_union` fits the value axis to the records' and outliers'
statistics. Every part of a box (body, whiskers, caps, median, mean,
outliers) picks the box as `PickedData::HistogramBin` with the category row.
`Config::from_boxplot` opens a legacy `BoxPlotChart` document as such a
chart; see the
[box plot guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/BOXPLOT.md).

Category bar, box plot and radar series draw value labels: the strings of
`labels.text` (SSOT configuration written at edit time by `format_labels`,
`format_box_plot_labels` or `format_radar_labels`; the renderer never
formats a value), baked into a label atlas with the contour labels' text
stack at the render scale and placed from the pool on every draw at the bar
end, past the top of the box or outside the vertex, moved by
`labels.offsets`. A label is picked by `pick_chart_data` as
`PickedData::ValueLabel` (series and category), selected through
`PickedDataRef::ValueLabel` (its frame is outlined) and dragged with
`SeriesConfig::drag_value_label`; its id for hosts is
`data_config::value_label_id(series_id, index)`. See Value labels in the
[interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md).

```rust
use renderer::config::Category;
use renderer::default::default_config;

let mut cfg = default_config();
cfg.bottom_x.categories = Some(vec![
    Category::new("north", "North"),
    Category::new("south", "South"),
]);
assert_eq!(cfg.bottom_x.data_range(), (-0.5, 1.5));
assert_eq!(cfg.bottom_x.validate_categories(), Ok(()));
```

## Native edit, pick and export workflow

1. Create one `RendererDevice` from the host's matching wgpu device/queue handles.
   Use `Renderer::try_new` for Cartesian charts or the bounded renderer's `new`.
   Window helpers are optional; a headless application does not need a window.
2. For Cartesian charts, upload columns, build `Chart`/`SeriesConfig`, create a
   `ChartView` and a `ChartStyle` for each series, then use `prepare` +
   `paint_prepared`, or the panel export helpers. Renderer-managed chart
   IDs instead use `register_chart`, `set_chart_config`/`set_chart_series` and
   `prepare_registered`; editing an unrelated local `Config` does not edit that ID.
   For the bounded radial chart, edit the `RadialChart` passed to `prepare`. Normal edits do not require `clear_cache`.
3. Hit-test the frame currently displayed. Bounded data uses `frame.hit_test`;
   the radial chart picks its slices on the GPU (`frame.pick(p)` returns a
   ticket, `resolve()` reads the slice back). Bounded titles/axes/labels/
   legends/plot areas use `frame.decorations()`; the radial frame's is a
   `HitMap<RadialChart>` queried with the chart it was prepared from
   (`frame.decoration_at(&chart, p)`). All take chart-local logical pixels. Cartesian decorations use
   `HitMap::for_active_layer(&config)`, or `ChartView::hit_test_active_layer` to
   include annotations, in the pixels of the config the view was refreshed with.
   Cartesian data picks address a registered chart ID: `Renderer::pick_chart` /
   `pick_chart_data` take a `GpuPickRequest` with a physical canvas position,
   the physical display panel, the display scale and `max_distance_px`. Do not
   mix these coordinate conventions.
4. Store the target in the model and edit its values/styles or decoration layout.
   For Cartesian charts, refresh the view's changed axes/decorations and rebuild
   affected caller-owned styles as described below, then prepare, draw and submit.
   Bounded renderers update their derived resources in `prepare`.
   A stored frame is not a live view of the model.
   For a resize that may fail layout validation, prepare a trial model before
   replacing the displayed model. The host requests frames and advances animations.
5. Export the same model state to RGBA/PNG. Selection and hover emphasis are model
   state; clearing them or exporting another copy changes the output. For bounded
   charts, hide only decoration handles by clearing `decorations.selected` in an
   export clone while preserving data selection, offsets and `data_area`.
   Cartesian exports never draw decoration selection boxes. The caller stores
   PNG bytes.
6. Release obsolete frame references. Custom render-pass hosts call
   `end_gpu_frame` / `end_frame` after submitting or discarding their commands.
   Cartesian window drawing and panel export handle their own submission boundary;
   submit pending external commands before invoking those helpers. Native hosts
   poll the device to process completion callbacks. Retained frames retain resources.

### Cartesian edits: update the derived inputs before preparing

`ChartView` holds rasterized axes, grid and decorations. `ChartStyle` holds GPU
style data derived from a `SeriesConfig`. Editing the config does not update
either object automatically in the caller-owned `prepare` path.

| Change | Required work before the next `prepare` |
| --- | --- |
| Axis range, ticks, grid, title, legend, decoration layout or panel size | Call `refresh_axis` with the current chart and panel rectangle. Use `refresh_axis_with_selection(view, chart, panel_rect, Some(element_id))` when displaying a selection: the renderer draws the box from the element's own placement, and `selection_box(chart, element_id)` returns the same box for host UI. |
| Series appearance, mapped styles or series identity | Recreate the affected `ChartStyle` with `create_style_for_series`, or its scaled variant at the display scale. Refresh decorations too if labels or legend appearance changed. |
| Column values | Finish the upload/upsert before preparing. Refresh axes/decorations if the edit also changes their config. |
| Annotation items or annotation selection only | Call `refresh_annotations(view, chart, panel_rect, selected)`; it leaves the axis and decoration textures alone. A `refresh_axis*` call redoes the annotation layer too. Toggling `annotations.visible` only skips the composite and needs no raster refresh. |
| Renderer-owned chart ID | Commit edits with `set_chart_config` / `set_chart_series`, refresh the caller-owned view from that registered config, then call `prepare_registered`. This path maintains the series-style cache itself. |

`prepare` updates transforms and derived GPU data; it does **not** rerasterize
the view or recreate the `ChartStyle` values supplied by the caller.
`update_transform` alone also leaves axis labels and the grid unchanged.
Use the chart's dirty state or `ChartRenderStamp::needs_raster_since` to decide
when to refresh; unchanged frames can reuse the existing view and styles.

In that refresh branch, call `refresh_axis` and then read
`view.render_warnings()`. It lists, as `RenderWarning`s, the annotations the new
raster skipped and the tick settings it could not draw as written: for example a
`major_spacing` drawn at automatic spacing or a clamped `minor_count` (limits in the
[configuration reference](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md)).
`create_chart_view` and every successful refresh re-derive the list from the
chart they rasterized, so it changes on dirty-driven raster refreshes and never
per frame. To report only changes, compare it with the list read after the
previous refresh; a NaN `requested` value compares unequal to itself.

This helper shows the caller-owned path after a series-style edit at scale 1.
It conservatively refreshes decorations and rebuilds every style. A host can
rebuild only the affected styles; an axis-only edit can reuse them all.
Submit or discard commands using the previous frame **before** calling it.

```rust,no_run
use renderer::{
    Chart, ChartDrawItem, ChartStyle, ChartView, FiggyError, PreparedFrame,
    Renderer, Series, SeriesConfig,
};
use renderer::layout::Rect;

fn prepare_after_series_edit(
    renderer: &mut Renderer,
    chart: &Chart,
    view: &mut ChartView,
    configs: &[SeriesConfig],
    panel: Rect,
) -> Result<(Vec<ChartStyle>, PreparedFrame), FiggyError> {
    let styles = configs.iter()
        .map(|config| renderer.create_style_for_series(config))
        .collect::<Result<Vec<_>, _>>()?;
    renderer.refresh_axis(view, chart, panel)?;
    let series: Vec<_> = configs.iter().zip(&styles)
        .map(|(config, style)| Series { config, style })
        .collect();
    let frame = renderer.prepare(&[ChartDrawItem {
        view,
        chart_config: chart.config(),
        series: &series,
    }])?;
    // Store the styles for later frames, paint this token, then submit.
    Ok((styles, frame))
}
```

Panel export helpers build their own scaled view and styles from the supplied
configs. A correct PNG does not establish that a separately displayed view has
been refreshed. Apply the same model edits to both paths. A `Config` with
`overlays` is exported as a whole by the `export_document_*` helpers (every
layer composited in order, every layer's decoration above every layer's data). A document with `subplots` (more charts, each with
its own axes) exports the union of every chart area (`Config::document_area`),
and a display fits that union with `display_config_for_surface`. The
`export_panel_*` helpers render only the parent chart's own base layer in its
`chart_area`; the two are byte-identical for a document without overlays or
subplots. See the
[overlay layer contract](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#overlay-layers) and
[subplots](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#subplots).

### Bounded chart example

This bounded example accepts the host GPU pair, selects from a displayed frame,
changes a slice color and exports the resulting model. It is compiled as a doctest;
the `radial_editor`
example (`--features egui_demo`) demonstrates render-pass submission and pointer
events.

```rust,no_run
use renderer::{Color, RadialChart, RadialRenderer, RadialSlice, RendererDevice};

async fn edit_and_export(
    gpu: RendererDevice,
    pointer: [f32; 2], // chart-local logical pixels
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut renderer = RadialRenderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm)?;
    let mut chart = RadialChart {
        slices: vec![
            RadialSlice::new("A", 30.0, Color::from_rgb8(40, 140, 200)),
            RadialSlice::new("B", 70.0, Color::from_rgb8(220, 160, 60)),
        ],
        ..Default::default()
    };
    // The drawn label strings are written at edit time.
    chart.format_labels();
    let displayed = renderer.prepare(&chart, (800, 600), 1.0)?;
    // The radial chart picks on the GPU: request, then resolve.
    chart.interaction.selected = displayed.pick(pointer)?.resolve().await?;
    chart.slices[0].color = Color::from_rgb8(50, 180, 150);
    // Export prepares the edited model at the requested resolution (1600 x 1200).
    let image = renderer.export_rgba_async(&chart, (800, 600), 2.0).await?;
    drop(displayed);
    renderer.end_frame();
    Ok(renderer::encode_png(&image)?)
}
```

## Errors

Cartesian operations return `FiggyError`; the radial renderer uses
`RadialError`. `RadarChart::validate` reports a legacy radar document's
validation error as a `&'static str`. Handle validation/layout/budget errors before
replacing the last valid display.

## Construction and prewarming

Construction and full Cartesian prewarming are separate operations:

| Entry point | Work performed |
| --- | --- |
| `Renderer::try_new` / `try_new_observed` | Validate device capabilities and create the initial column pool, shared layouts/sampler, shader modules, unit quad and axis pipeline. Optional resident data/style, arc/fit/contour-label and picking pipelines remain lazy. |
| Native `for_window` / `for_window_async` / `for_window_async_observed` | Create and retain instance/adapter/device/queue, configure a surface and optional MSAA target, then initialize the same base renderer resources. |
| Browser `for_window_async` / `for_window_async_observed` | In addition to window setup, await temporary JavaScript warmup pipelines for the Cartesian render entries. Those temporary pipelines are discarded; they do not populate the renderer's retained optional caches. |
| `for_window_minimal_async_observed` | Limit that browser render warmup to `PreparationFeature::Basic` entries. On native targets it has the same base initialization as the ordinary async constructor. It does not install a completed Basic preparation group. |
| `prewarm_all` / `prewarm_all_observed` | Compile and retain the resident Cartesian graphics/style pipelines, contour-label compute, arc scans, fit reduction and GPU picking machinery for the current renderer/target generation. |

Normal preparation creates missing pipelines lazily. To move that work before
first use, await `prewarm_all`, or compile and install the required detached
preparation groups described below. Repeating prewarm reuses installed caches
while they remain valid; changing the target format or sample count can require
new target pipelines. Shader-module creation and temporary browser warmup can
still involve driver compilation work during construction.

Full prewarm does not prepare future chart data, views, series styles, label
atlases or result buffers, and is not a guarantee of allocation-free or
compilation-free first drawing. The separate radial
renderer and nonresident matrix/tile streaming pipelines are outside this prewarm scope.

## Ownership and frame validity

`RendererDevice` holds `Arc` handles to a device and queue created together.
The host may share them across renderers. Their originating device must match;
wgpu exposes no general parent-device identity check for a queue. Window helpers
also retain the instance, adapter and surface needed for their own lifetime.

Cartesian `PreparedFrame` validity depends on captured renderer resources and
revisions. If painting returns `StalePreparedFrame`, rebuild the inputs and
prepare again. A bounded chart frame (`Arc<RadialFrame>`) owns an immutable GPU snapshot:
keeping it alive keeps its resources alive even after the current model changes.
Do not retain every historical frame in a live-edit loop.

## GPU memory accounting and budget scope

`Renderer::gpu_memory_usage()` combines the column pool's byte counts with the
renderer ledger's tracked allocations. Live and retired bytes both count.
Dropping the final tracked owner retires its resources; a submission boundary
captures them, and GPU completion releases their handles and byte credits.
`end_gpu_frame()` alone does not make those bytes available for reuse.

`set_memory_budget` sets an allocation policy used by checked allocation and
admission paths, including pool growth. It is **not a hard ceiling on all GPU
memory** or an interception of every device allocation. The initial pool is
created before a budget is set. Lowering the budget does not shrink or evict
resources: the setter applies the policy and returns `Some(current_total)` if
the tracked total already exceeds it, otherwise `None` (also when disabling it).

The regular ledger excludes the lazy `DrawStyle::Milkyway` / `DrawStyle::Constellation`
textures (PSF, planet atlas, blackbody LUT and ring strip); streaming export admission separately
checks their known payload size. CPU allocations, final RGBA/PNG bytes, host-owned
GPU targets, other renderers' resources and shader/pipeline/driver internals are
also outside this report. Counts describe requested tracked bytes, not measured
physical VRAM residency. Admission checks do not reserve future allocations.

For a shared device, combine renderer budgets with host allocations and leave
headroom for untracked resources. Device limits and a passing budget check do not
guarantee driver allocation success; handle allocation failure and device loss.

## Render targets, coordinates and output

Bounded chart frames require a target with the prepared physical size, the renderer's
format and sample count 1. `draw` sets viewport/scissor to that target. For a host
UI subregion or a multisampled host pass, draw to a matching offscreen texture and
composite it. Supported formats are RGBA8/BGRA8, linear or sRGB.

Bounded chart `prepare` takes logical size and scale; picking takes logical
coordinates. The radial chart draws the label strings of its slices'
`text` and formats nothing: write them with `RadialChart::format_labels`
after editing values or formats (`normalize_labels` fills a loaded document
that has none). Frame sizes and data geometry rectangles are physical pixels;
`frame.decorations()` exposes its hit bounds and selection boxes in logical pixels.
Prepare at the actual output scale to rasterize labels at that resolution.
Each module documents its accepted size/scale bounds; the Cartesian export scale
contract differs from the bounded chart contract.

RGBA export returns pixels; PNG encoding returns bytes. The caller decides where
to send or store them. Native blocking helpers and async variants are both
available. No project file persistence or application event loop is required.

## Compatibility policy

The public Rust surface includes struct literals and exhaustive enums. Removing
or changing fields, adding required fields or enum variants, changing trait
requirements, and adding `non_exhaustive` to existing types can break consumers.
Incompatible changes require a new 0.x minor version and migration notes; patch
updates must preserve the supported contract. Public low-level modules such as
`data_render`, `axis_render` and `raster` follow the same policy. Moving to a new
wgpu major changes public wgpu types such as the `RendererDevice` handles, so it
ships in a new 0.x minor.

figgy-renderer 0.18.1 / figgy-model 0.12.1 follow the crates.io releases
0.16.1 / 0.10.0 (the first, 2026-10-13), 0.17.0 / 0.11.0 (2026-10-16) and
0.18.0 / 0.12.0 (2026-10-20).
Published registry artifacts are immutable.

Version 0.16.0 / model 0.10.0 added plot-area decoration state and targets:
fully specified `DecorationLayout` literals need `data_area: None`, and exhaustive
`DecorationId` matches need `DataArea` and `DataAreaEdge`. See the
[interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#트레잇-상태-타입-마이그레이션-웹-연결)
for migration details. Version 0.17.0 / model 0.11.0 added `RichText.align`;
JSON without it parses as `Left`, but fully specified `RichText` literals need
`align: TextAlign::Left`.

Version 0.18.0 / model 0.12.0 is also an incompatible minor. JSON written by
0.17.0 still parses, but fully specified literals need new fields, exhaustive
matches need new enum variants, `RenderWarning::axis()` returns an `Option`,
and some low-level `data_render` signatures change.
[Migrating from 0.17.0](https://github.com/shim9610/figgy/blob/master/CHANGELOG.md#migrating-from-0170)
lists every change and the value to write for each new field. Version 0.18.1 /
model 0.12.1 is a patch: it adds items without changing a signature or a
serialized field.

The optional serde representation is not an application file-format version.
Serialized fields added since the first crates.io release (0.16.1 / 0.10.0) are
optional or defaulted and omitted at their defaults, so a document written by one
of those releases parses and re-serializes unchanged on a later one. Arbitrary
cross-version migrations are not promised: an older release ignores fields it
does not know and rejects enum variants it does not know.

## Detached Cartesian pipeline preparation

`Renderer::for_window_minimal_async_observed` limits temporary browser render
warmup as described above. A host using detached preparation to gate first use
must compile and install `PreparationFeature::Basic` (which also includes arc
scans and fit reduction), plus the groups needed by its charts. Construction
alone does not mark Basic ready.

`begin_preparation(feature)` returns an owned `PipelinePreparation`, or `None`
when that group is already ready. Its consuming `compile(observer).await` owns
GPU handles and the originating renderer's memory ledger, **not a renderer borrow**.
Frames and config edits may proceed on the renderer while it awaits compilation.
`install_preparation(result)` merges the actual prepared caches synchronously,
rejecting another renderer/device/target generation. It never publishes chart data,
config, selection, view ranges, or a render revision. Dropping an unfinished job
or an uninstalled result releases its handles. Hosts deduplicate concurrent requests
per renderer and feature, serialize installation with other renderer calls, and
ignore results after their renderer has been disposed.

`PreparationFeature::ALL`, `for_chart(config, series)` and `preparation_ready`
provide the host's feature inventory and gating contract. Background compilation
can start for all groups after Basic is installed. Before calling a render path,
the host must ensure the current declarations' required groups are ready; before
picking, also ensure Picking. The groups cover the same Cartesian resident
pipelines as `prewarm_all`. GPU-driver contention can still delay frames; detached
ownership is not a frame-time guarantee.

## Line picking

Line picking treats a midpoint difference within the projected coordinates' f32
rounding uncertainty as a tie and chooses the earlier endpoint, on both CPU and
GPU. Points outside that narrow interval still choose the nearer endpoint. This
makes exact midpoint selection stable across native compiler backends.
Interior line distance uses the perpendicular distance to the segment. Beyond an
endpoint it uses distance to that endpoint. This avoids reconstructing a rounded
closest point that can falsely reject an exact centerline hit at zero tolerance;
it does not add a pick radius or turn finite segments into infinite lines.

## Bounded chart decoration interaction

Radial frames expose `decorations()`, a hit map using the shared `HitMap` / `Selectable` / `Draggable` / `Resizable` contracts: a `HitMap<RadialChart>`, whose title, plot area and slice labels (an outside label with its leader line) place their parts from the chart through the one emit the label raster draws with. See [interaction workflow](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md) for selection, logical-pixel dragging, SSOT persistence and host integration.

## Pointer position in drags

Every drag takes a `&dyn MeasureText` (the renderer's `CpuTextMeasure::for_style(&config.draw_style)`): a move is contained on the element's drawn parts, so it is rejected when none of them would keep its centre inside the chart area, or when the measure cannot place the element. Hosts that know the pointer position pass it through `Draggable::drag_from(cfg, measure, at, dx, dy)`; the default forwards to `drag_by`, and only a polar chart's axes and tick labels read the position (the ray turns around the centre, the angle labels move along the spoke under the pointer). `Config::nudge_from(element, measure, Some(at), dx, dy)` is the model-level entry; `nudge` is `nudge_from` without a position.

## Annotation layer

Every chart family carries `annotations: AnnotationLayer` (reference lines,
spans, text boxes, arrows). Cartesian views rasterize it into two more
`ChartView` textures: `refresh_axis*` redoes them with the axis raster and
`Renderer::refresh_annotations(view, chart, panel_rect, selected)` redoes them
alone when `ChartRenderStamp::needs_annotation_raster_since` or the annotation
selection changed; `ChartView::annotation_map()` is the layer's hit map. Bounded
renderers return the layer as `prepare_annotations(chart, size, scale)` →
`Arc<BoundedAnnotationFrame>`, drawn as `draw_underlay`, the frame, `draw_overlay`
(an underlay needs `set_background(FrameBackground::HostClear)` and a clear with
the chart background; `export_rgba` handles that itself). Items the raster could
not place are `RenderWarning::AnnotationSkipped` on `ChartView::render_warnings()`
(Cartesian) or `BoundedAnnotationFrame::render_warnings()` (bounded). See the
[annotation section of the interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#annotation-layer)
and the [configuration reference](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#layerref--annotationlayer).
