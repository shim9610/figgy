# Rust API contracts

## Package names and features

The Cargo packages are `figgy-renderer` and `figgy-model`. Their Rust library
names remain `renderer` and `model`. Existing `use renderer::...` imports and
type identities are preserved. A dependency previously named `renderer` needs
`package = "figgy-renderer"` when upgrading to a source revision with the rename.
An old source revision continues to require its old package name.

The renderer re-exports model types. Enable its `serde` feature to serialize
those types; consumers do not need to name `figgy-model` merely to enable serde.
`egui_demo` enables optional GUI example dependencies. Default features do not
require a windowing or GUI framework. The `figgy` WASM wrapper is a separate
consumer and has `publish = false` for the Rust registry.

Rust 1.99 is the declared support floor, not a claim that earlier toolchains
cannot compile the code. The tested development toolchain and the support floor
are recorded separately. Direct host device/queue/render-pass sharing requires
wgpu 30; a host on another wgpu major must adapt or upgrade its integration.

For a complete headless Cartesian program, run the shipped `line` example.
The source repository's [example index](https://github.com/shim9610/figgy/blob/master/crates/renderer/examples/README.md)
maps each chart to its input, output and command. Gallery images are repository
assets; they are not runtime resources required by the crate.

macOS / Metal support is **experimental and not yet stable**. Version 0.14.0 uses
explicit FMA rounding boundaries for hi/lo coordinates and a shared CPU/GPU rule
for midpoint picking uncertainty. Physical Apple GPU coverage remains outstanding.
A separate precompiled Metal shader experiment does not change the default renderer
or establish a precision guarantee; no dependency patch or precompiled shader is required.
The experimental path is not a public Cargo feature. Existing regression checks
remain enabled; see the source repository's backend results for tested revisions.

## Entry points and input contracts

| Renderer | Input | Frame preparation |
| --- | --- | --- |
| `Renderer` | `Config`, ordered `SeriesConfig`, registered numeric columns | `prepare` or `prepare_registered` |
| `RadialRenderer` | `RadialChart`, bounded slice values | `prepare` |
| `CategoricalRenderer` | `CategoricalChart`, bounded category/series values | `prepare` |
| `BoxPlotRenderer` | `BoxPlotChart`, supplied summary statistics | `prepare` |

The bounded chart APIs are deliberately separate from `ColumnSource` and streaming.
They do not calculate raw-sample statistics. Chart-specific limits, missing values,
style inheritance and picking identities are documented in their modules.
Categorical/boxplot targets use category/series IDs; radial targets use group/index
positions, so a host that reorders slices must update interaction targets too.

`ColumnSource` supplies scalar GPU pairs; `HiLoColumnSource` preserves sub-f32
deltas for large coordinates. Native upload avoids an intermediate CPU conversion
vector but still copies staging data to GPU storage. It is not a promise of zero
driver copies. Use the hi/lo path explicitly when precision requires it.

Replayable streams must return the same encoded values for the same source
revision. Bounded renderer memory does not make a one-shot source replayable.
Initial fit previews can be approximate; completion denotes the final render,
not merely that all input primitives have been submitted. The host owns and
retains the replayable source for the operations it enables.

## Native edit, pick and export workflow

1. Create one `RendererDevice` from the host's matching wgpu device/queue handles.
   Use `Renderer::try_new` for Cartesian charts or the bounded renderer's `new`.
   Window helpers are optional; a headless application does not need a window.
2. For Cartesian charts, upload columns, build `Chart`/`SeriesConfig`, create a
   `ChartView` and a `ChartStyle` for each series, then use `prepare` +
   `paint_prepared`, or the panel export helpers. Renderer-managed chart
   IDs instead use `register_chart`, `set_chart_config`/`set_chart_series` and
   `prepare_registered`; editing an unrelated local `Config` does not edit that ID.
   For bounded charts, edit the `RadialChart`, `CategoricalChart` or `BoxPlotChart`
   passed to `prepare`. Normal edits do not require `clear_cache`.
3. Hit-test the frame currently displayed. Bounded data uses `frame.hit_test`;
   titles/axes/labels/legends/plot areas use `frame.decorations()`. Both take
   chart-local logical pixels. Cartesian `GpuPickRequest` instead carries a
   physical canvas position, physical display panel and display scale. Do not
   reuse one coordinate convention for both APIs.
4. Store the target in the model and edit its values/styles or decoration layout.
   For Cartesian charts, refresh the view's changed axes/decorations and rebuild
   affected caller-owned styles as described below, then prepare, draw and submit.
   Bounded renderers update their derived resources in `prepare`.
   A stored frame is not a live view of the model.
   For a resize that may fail layout validation, prepare a trial model before
   replacing the displayed model. The host requests frames and advances animations.
5. Export the same model state to RGBA/PNG. Selection and hover emphasis are model
   state; clearing them or exporting another copy changes the output. Hide only
   decoration handles by clearing `decorations.selected` in an export clone while
   preserving data selection, offsets and `data_area`. The caller stores PNG bytes.
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
| Axis range, ticks, grid, title, legend, decoration layout or panel size | Call `refresh_axis` with the current chart and panel rectangle. Use `refresh_axis_with_selection` when displaying selection boxes. |
| Series appearance, mapped styles or series identity | Recreate the affected `ChartStyle` with `create_style_for_series`, or its scaled variant at the display scale. Refresh decorations too if labels or legend appearance changed. |
| Column values | Finish the upload/upsert before preparing. Refresh axes/decorations if the edit also changes their config. |
| Renderer-owned chart ID | Commit edits with `set_chart_config` / `set_chart_series`, refresh the caller-owned view from that registered config, then call `prepare_registered`. This path maintains the series-style cache itself. |

`prepare` updates transforms and derived GPU data; it does **not** rerasterize
the view or recreate the `ChartStyle` values supplied by the caller.
`update_transform` alone also leaves axis labels and the grid unchanged.
Use the chart's dirty state or `ChartRenderStamp::needs_raster_since` to decide
when to refresh; unchanged frames can reuse the existing view and styles.

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
been refreshed. Apply the same model edits to both paths.

### Bounded chart example

This bounded example accepts the host GPU pair, selects from a displayed frame,
changes a slice color and exports the resulting model. It is compiled as a doctest;
the interactive editors demonstrate render-pass submission and pointer events.

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
    let displayed = renderer.prepare(&chart, (800, 600), 1.0)?;
    chart.interaction.selected = displayed.hit_test(pointer);
    chart.slices[0].color = Color::from_rgb8(50, 180, 150);
    // Export prepares the edited model at the requested resolution (1600 x 1200).
    let image = renderer.export_rgba_async(&chart, (800, 600), 2.0).await?;
    drop(displayed);
    renderer.end_frame();
    Ok(renderer::encode_png(&image)?)
}
```

Automatic streaming renders the registered range unless explicitly asked to fit.
Use `request_stream_auto_fit(chart_id, padding)` for the next automatic execution
when the initial range is unknown, or when the user requests a fresh fit. Do not
call it for every chunk, repaint, tab switch or aspect-ratio change. Manual streaming
and resident drawing do not consume that request. Inspect `StreamingStatus` and
`auto_fit_pending`; submitted input alone is not the final-image completion signal.

Cartesian operations return `FiggyError`; bounded renderers use `RadialError`,
`CategoricalError` and `BoxPlotError`. Handle validation/layout/budget errors before
replacing the last valid display. Device loss and external allocation failure still
need host recovery; a configured memory budget is not a driver success guarantee.

## Construction and prewarming

Construction and full Cartesian prewarming are separate operations:

| Entry point | Work performed |
| --- | --- |
| `Renderer::try_new` / `try_new_observed` | Validate device capabilities and create the initial column pool, shared layouts/sampler, shader modules, unit quad and axis pipeline. Optional resident data/style, arc/fit/contour-label and picking pipelines remain lazy. |
| Native `for_window` / `for_window_async` (including observed variants) | Create and retain instance/adapter/device/queue, configure a surface and optional MSAA target, then initialize the same base renderer resources. |
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
compilation-free first drawing. Separate radial/categorical/boxplot renderers
and nonresident matrix/tile streaming pipelines are outside this prewarm scope.
Browser warmup uses explicit production-compatible descriptors rather than
auto-layout guesses; temporary JS warmup and installed wgpu caches remain distinct.

## Ownership and frame validity

`RendererDevice` holds `Arc` handles to a device and queue created together.
The host may share them across renderers. Their originating device must match;
wgpu exposes no general parent-device identity check for a queue. Window helpers
also retain the instance, adapter and surface needed for their own lifetime.

Cartesian `PreparedFrame` validity depends on captured renderer resources and
revisions. If painting returns `StalePreparedFrame`, rebuild the inputs and
prepare again. A small-chart `Arc<...Frame>` owns an immutable GPU snapshot:
keeping it alive keeps its resources alive even after the current model changes.
Call `prepare` again after an edit; an old frame does not update itself.

After submitting or discarding recorded commands, custom native hosts call
`Renderer::end_gpu_frame` or the small renderer's `end_frame`. Poll the device as
required by the host loop so completion callbacks can retire resources. Do not
retain every historical frame in a live-edit loop. Window helpers manage their
documented submission boundaries.

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

The regular ledger excludes lazy Milkyway/Constellation style textures (PSF,
planet atlas, blackbody LUT and ring strip); streaming export admission separately
checks their known payload size. CPU allocations, final RGBA/PNG bytes, host-owned
GPU targets, other renderers' resources and shader/pipeline/driver internals are
also outside this report. Counts describe requested tracked bytes, not measured
physical VRAM residency. Admission checks do not reserve future allocations.

For a shared device, combine renderer budgets with host allocations and leave
headroom for untracked resources. Device limits and a passing budget check do not
guarantee driver allocation success; handle allocation failure and device loss.

## Render targets, coordinates and output

Small-chart frames require a target with the prepared physical size, the renderer's
format and sample count 1. `draw` sets viewport/scissor to that target. For a host
UI subregion or a multisampled host pass, draw to a matching offscreen texture and
composite it. Supported formats are RGBA8/BGRA8, linear or sRGB.

Small-chart `prepare` takes logical size and scale; picking takes logical
coordinates. Frame sizes and data geometry rectangles are physical pixels;
`frame.decorations()` exposes its hit bounds and selection boxes in logical pixels.
Prepare at the actual output scale to rasterize labels at that resolution.
Each module documents its accepted size/scale bounds; the Cartesian export scale
contract differs from the small-chart contract.

RGBA export returns pixels; PNG encoding returns bytes. The caller decides where
to send or store them. Native blocking helpers and async variants are both
available. No project file persistence or application event loop is required.

## Compatibility policy

Changing package identity requires a Cargo dependency declaration update; it does
not rename Rust items. Bounded chart modules are already part of the public source
release. Version 0.16.0 / model 0.10.0 added plot-area decoration state and targets:
fully specified `DecorationLayout` literals need `data_area: None`, and exhaustive
`DecorationId` matches need `DataArea` and `DataAreaEdge`. See the interaction guide
for migration details. The unreleased `AxisOptions.range_locked` field is additive
in JSON (serde default) but fully specified `AxisOptions` literals need
`range_locked: false`.
Packaging work does not change those contracts. Public
low-level modules remain available; they must not be removed silently during packaging.

The public Rust surface includes struct literals and exhaustive enums. Removing
or changing fields, adding required fields or enum variants, changing trait
requirements, and adding `non_exhaustive` to existing types can break consumers.
Incompatible changes require a new 0.x minor version and migration notes; patch
updates must preserve the supported contract. A new wgpu major needs the same
review. The published crates.io versions are figgy-renderer 0.16.1 and figgy-model 0.10.0.
Later source commits do not replace those immutable registry artifacts.

The optional serde representation is not an application file-format version.
Same-version round-trip tests do not promise arbitrary cross-version migrations.

## Detached Cartesian pipeline preparation

`Renderer::for_window_minimal_async_observed` limits temporary browser render
warmup as described above. A host using detached preparation to gate first use
must compile and install `PreparationFeature::Basic` (which also includes arc
scans and fit reduction), plus the groups needed by its charts. Construction
alone does not mark Basic ready. Ordinary `prepare` still supports lazy creation;
`prewarm_all` explicitly prepares all resident Cartesian groups.

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
picking, also ensure Picking. This opt-in API does not change legacy lazy creation.
The groups cover Cartesian resident pipelines, matching `prewarm_all`; nonresident
matrix/tile streaming and separate radial/categorical/boxplot renderers are not
part of this inventory. GPU-driver contention can still delay frames; detached
ownership is not a frame-time guarantee.


Line picking treats a midpoint difference within the projected coordinates' f32
rounding uncertainty as a tie and chooses the earlier endpoint, on both CPU and
GPU. Points outside that narrow interval still choose the nearer endpoint. This
makes exact midpoint selection stable across native compiler backends.
Interior line distance uses the perpendicular distance to the segment. Beyond an
endpoint it uses distance to that endpoint. This avoids reconstructing a rounded
closest point that can falsely reject an exact centerline hit at zero tolerance;
it does not add a pick radius or turn finite segments into infinite lines.

## Bounded chart decoration interaction

Radial, categorical and boxplot frames expose `decorations()`, a renderer-measured hit map using the existing `HitMap` / `Selectable` / `Draggable` contracts. See [interaction workflow](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md) for selection, logical-pixel dragging, SSOT persistence and host integration.
