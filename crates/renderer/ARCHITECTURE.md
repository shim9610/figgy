# Rendering architecture / 렌더링 구조

Native rendering, data ownership, GPU memory and frame invalidation of the
Cartesian `Renderer`. The bounded chart families (`RadialRenderer`,
`CategoricalRenderer`, `BoxPlotRenderer`, `RadarRenderer`) are separate
renderers: each returns immutable prepared frames, keeps its own GPU ledger
(`gpu_memory_usage()`) and budget (`set_memory_budget(bytes)`), and uses no
column pool, auto-fit jobs or streaming. A host that reports total GPU memory
adds every renderer's `gpu_memory_usage()`. See [RADIAL.md](RADIAL.md),
[CATEGORICAL.md](CATEGORICAL.md), [BOXPLOT.md](BOXPLOT.md) and
[RADAR.md](RADAR.md).
For runnable examples, start with the [README](https://github.com/shim9610/figgy/blob/master/README.md).

| Component | Role |
|---|---|
| `figgy-model` | Chart settings, data contracts and interaction policies; optional serde |
| `figgy-renderer` | GPU data drawing, CPU text/axes, picking, caching, resource ownership and export |
| `crates/web` | Example application adapter: wasm-bindgen, canvas events and frame scheduling |

The browser lifecycle descriptions below describe that example adapter, not a
JavaScript API for every renderer. [Browser example scope](https://github.com/shim9610/figgy/blob/master/crates/web/README.md).

- **Resident GPU columnar pool**: columns admitted to the resident path share a single GPU buffer with first-fit allocation. A removal schedules a ping-pong defragmentation that runs when the host calls `process_pending_maintenance()` (or `defragment()`); under `DefragPolicy::OnAllocFailure` an upload that hits `OutOfSpace` also defragments once and retries. The default policy is `Manual`. Logical values are stored as f32 hi/lo pairs when uploaded through `HiLoColumnSource`, preserving timestamp-sized offsets on the GPU. Upload caches scalar stats (min / max / smallest-positive) for auto-fit; per-point geometry such as the dashed-line arc-length prefix is computed on the GPU by a compute scan (`line_arc.wgsl`).
- **Nonresident rendering**: replayable columns are registered by logical ID, length, encoding, and revision without keeping their complete data in the GPU pool. The renderer requests bounded original ranges and accumulates the exact drawing; the host retains the source for replay after a view or output change. Contours, Milkyway lines, polar charts, line fills and step lines, vector series, point colour and size maps, and documents with overlay layers or subplots reject streamed sources with an explicit error. Admission, supported styles, cancellation, and completed-revision queries are described in the [web streaming contract](WASM.md#exact-streaming-en).
- **Layered compositing**: grid → annotation underlay → data → axis/label/legend → annotation overlay, so grid never covers the data. Axis raster can be produced as `Grid` and `Decoration` layers; `AxisLayerKind::All` remains a legacy single-pass helper.
- **Data fidelity contract**: renderer/web consume the model contract without silently changing original coordinates, provenance, or axis↔data correspondence. Explicit clipping, log-domain skips, NaN skips, and antialiasing limits are rendering contracts rather than data rewrites.
- **Headless PNG export**: GPU offscreen raster at `scale` 0.25–8.0 (about 24–768 DPI; `clamp_export_scale` clamps other values, and the scaled target must fit the device texture limit) → RGBA / PNG bytes in memory (async-first; blocking wrappers on native).
- **Interaction layer (opt-in)**: hit-testing, selection boxes, drag (axes constrained to their perpendicular, detached-axis `line_offset`), PPT-style 8-handle resize of the data area — all policy in `model`, fed by host pointer events; never runs if you don't wire it.
- **Data picking (opt-in)**: `pick_point` retains the point/line compatibility contract, while `pick_data` additionally returns tagged histogram-bin, canonical matrix-cell, and contour-level identities. Resident picking evaluates bar rectangles and field/contour geometry on the GPU from the same transform, pool, style, lattice, and level tables used to draw them. A completed chart-local packed view supports GPU point/line picking with original row indices and no source replay; other nonresident streams return `null`. Low-level WASM stream-pick replay entry points reject immediately. Hosts may still feed known stable refs through `Config.picked_data` or `Config.picked_points` for bounded selection highlighting.
- **Per-point style mapping (opt-in)**: precise scatter can bind `point_style_table` / `point_style_index_column` / `point_style_overrides`; precise errorbars can independently bind `error_bar_style_table` / `error_bar_style_index_column` / `error_bar_style_overrides`. Styled modes keep their own visual shaders and ignore these mappings.
- **Rich-text everywhere**: titles, tick labels, and the legend share one engine — per-segment bold/italic/underline/sub/superscript/greek, per-segment color & size overrides, `'\n'` line breaks, `'\t'` table columns, fixed-width legend symbol fields.
- **Hand-drawn sketch mode (opt-in)**: `draw_style: { mode: "sketch", amplitude_px, wavelength_px, seed }` renders the chart xkcd-style — axes/ticks/grid/legend wobble on the CPU raster, line wobble/dash phase uses arc-length-scan-driven GPU variants, markers/errorbars use dedicated GPU variants, and chart text automatically switches to the bundled handwritten face (Comic Neue, OFL) with per-character fallback for glyphs it lacks: the configured family, then registered fonts, the bundled Liberation Sans and (native only) installed system fonts, so CJK keeps your registered font. Deterministic (seeded), composes with dashes, and the field's absence means the precise path runs completely untouched. The mode draws line, scatter and errorbar series only: histogram, heatmap, contour and vector series are skipped without an error, and `draw_style_supports(style, render_type)` reports which combinations draw.
- **Milkyway mode (opt-in)**: `draw_style: { mode: "milkyway", ... }` renders the chart as an astrophotograph — lines become star chains over a series-colored nebula ribbon; scatter markers become ringed planets; errorbars become bipolar jets over a deep-space backdrop. Like sketch mode, it skips histogram, heatmap, contour and vector series without an error.
- **Constellation mode (opt-in)**: `draw_style: { mode: "constellation", ... }` supports `ScatterLine` series only: PSF-rendered stars sit at scatter data positions and a translucent line connects them. Parameter ranges ship as machine-readable metadata (`draw_style_param_specs`).
- **Single wgpu major (30)**: the renderer and active egui integration use wgpu 30. The retained iced integration source is not a build target because iced 0.14 still exposes wgpu 27 types.
- **WebAssembly-ready**: pure-Rust raster stack (tiny-skia + fontdb + swash), async init/export, runtime font registration (`text_render::register_font_bytes` in Rust, `register_font` on the browser example's `FiggyChart`) for CJK and custom families.
- **Example browser startup**: in a wasm browser, `create` / `create_with_progress` compile every render WGSL entry and await the first empty-chart frame; optional render, style and compute caches stay lazy until first use or `prewarm_all` / `prewarm_all_with_progress`. The web cold-start contract under [Memory accounting and release](#memory-accounting-and-release) gives the details.

## Internal memory data flow

![Renderer library overview: column-based and chart-specific paths, screen display and image export](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/architecture-state-flow-en.png)

The diagram omits the annotation underlay and overlay textures, overlay layers
and subplots. Its area 6 is out of date: arc prefixes serve dashed, sketch and
Milkyway lines (the Constellation style adds none), and each series keeps up to
eight immutable arc results rather than a copy-on-write scratch.

The renderer-owned chart registry is the persistent single source of truth
(SSoT) for chart state that the browser wrapper uses.
Low-level native hosts may still supply `ChartDrawItem` directly. In both paths,
`ChartDrawItem` is prepare-only input; paint consumes only the owned token.


### Rendering contracts

The overview separates the column-based `Renderer` path from the chart-specific
radial, categorical, boxplot and radar renderers. Their immutable frames do not
share the `PreparedFrame` or column-pool/streaming contract. Screen display and
image export are separate output paths; export renders at the requested size.
The table summarizes contracts by topic, rather than numbered areas in the image.

| Topic | Data flow and ownership contract |
|---|---|
| **Workspace responsibilities** | `model` defines chart declarations, layout and interaction policy without GPU resources. `renderer` implements the chart registry, GPU data processing and CPU raster integration. `web` adapts those APIs to canvas, input and browser scheduling. A low-level native host can also supply its own `ChartDrawItem` instead of using the registry. |
| **Column upload and maintenance** | Borrow `ColumnSource` only during native upload; write f32 pairs and smallest-positive statistics directly into mapped staging, then copy staging into the GPU pool. The pool keeps column slots and scalar metadata, not CPU point arrays. First-fit allocation and coalescing manage free regions. A reservation restores the free list on failure; upsert prepares fallible dependent state before publication. Allocation epochs identify content replacement, while layout generation identifies relocation such as defragmentation. |
| **Persistent and panel state** | `Renderer` holds registered Config/series, the pool, pipelines and shared device/queue. The host retains the returned `ChartView` and `ChartStyle`; views hold raster textures and transform resources, and styles retain GPU bindings. `WindowedRenderer` additionally retains surface/instance/adapter. CPU axis, text and grid rasterization can allocate image buffers; the column-upload contract does not prohibit those allocations. |
| **Frame invalidation** | A persistent host compares renderer-issued desired/raster revisions with its last successfully presented stamp. Raster or viewport changes require a raster refresh; host-only redraw state can require drawing without changing chart state. The browser's clean rAF skips surface acquisition and drawing, while required maintenance/completion processing can continue. Failed presentation must not advance the last-presented stamp. |
| **Prepare and record** | `prepare(&mut self)` resolves draw inputs and captures GPU handles, shared charges and validation stamps in `PreparedFrame`. `paint_prepared(&self)` validates that token before recording. It rejects changed views, captured columns, layouts, target pipelines or registered chart revisions. Once commands are recorded, the host must submit them before conflicting mutations; a token cannot inspect an external command buffer's later submission order. |
| **GPU-derived data** | Arc-prefix scans, picking and exact primitive extents use GPU columns. Arc scan uses bounded chunks plus a carry chain, rather than a fixed whole-series 16.7M-point ceiling. Picking and fitting return compact results instead of a CPU copy of all points. Immutable derived GPU results can be shared by cache entries and prepared tokens; cache eviction alone need not release the last owner. |
| **Screen and export** | Compose grid → annotation underlay → data → decoration → annotation overlay. Export prepares a scaled chart/view/style and offscreen target, then reads back padded row chunks and converts premultiplied RGBA to straight alpha. Row chunking bounds the readback buffer; the target, final RGBA image and PNG output still need their own memory. Resident export uses GPU columns, while streamed export replays original ranges. |
| **Fidelity and caches** | Resident redraw reads original GPU columns without a data image cache. Nonresident streaming retains an accumulation surface and may admit a chart-local cache of exact source rows for supported views. Neither path introduces LOD, sampling or decimation. Clipping, invalid/log-domain samples and antialiasing remain rendering rules; different backends need not produce byte-identical pixels. |
| **Accounting and retirement** | A GPU resource's shared charge survives as long as a Figgy owner retains it. After the last owner drops, retired bytes remain counted until an appropriate submission boundary and queue completion. `end_gpu_frame()` follows submission/discard of earlier commands; native hosts must service completions. The ledger describes tracked requested bytes, with the style-texture exceptions described below, rather than physical VRAM usage. |

### Resident column upload

![Native resident upload: borrowed source, mapped staging, GPU copy and retained CPU metadata](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/column-upload-en.png)

Zero-copy here means writing directly into mapped staging without a separate
CPU point array. The GPU copy still occurs; CPU axis/text image buffers are
outside this column-upload guarantee.

### Exact streaming and residency

![Column-based screen update loop: conditional CPU rasterization, resident GPU data, chunked streaming and composition](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/streaming-architecture-en.png)

The loop shows initial resident upload outside screen updates, conditional CPU
raster updates and reuse inside them, and chunked data drawing before composition.
Streaming is a Cartesian `Renderer` contract, not a bounded-chart renderer path.
Optional GPU caches and export replay are described below; the export inset
shows the separate output path. The native host
owns replayable original data and supplies requested ranges. Browser hosts use
stable TypedArrays or a `readRange` provider; the web facade handles
browser scheduling and requests only the ranges needed for the current job; it
does not own the stream cursor, chart state, or accumulated statistics. The
renderer owns `Config`, ordered series, source revisions, the cursor, cached
range summaries, and view-local residency admission. Automatic streaming never
promotes a connected whole-column closure into the global `ColumnPool`; a host
promotes one explicitly with `resident_admission_for_streamed_columns` (a check
without side effects) and `promote_streamed_columns_to_resident` (a
failure-atomic commit). It
draws through bounded GPU uploads and an offscreen accumulation surface, then
may retain the exact original rows needed by the view within the configured
working-set and total-GPU budgets. Both paths draw original
primitives, without LOD, sampling, or decimation. A resident chart and a
streamed chart can coexist on one page.

Streaming presents completed portions while the next ranges are supplied. An
unchanged completed revision reuses its visible result; decoration-only edits
keep the data cursor and accumulation. A view or physical-resolution change
redraws at the new transform. Eligible GPU caches supply retained original rows;
missing ranges are requested from the source provider. `job.cancel()` stops new
work and releases job-owned resources after submitted GPU work settles. GPU point/line picking uses the packed rows when
available and returns original row indices without source replay. Other streamed
charts do not support immediate picking. Scaled PNG export uses replayable
original ranges in a separate GPU path, so the source must remain available.
Dashed lines, histograms, heatmaps and supported styled streams can also reuse
an original-source GPU cache. This cache preserves ordering and adjacency, and
is admitted only when all referenced columns fit the per-chart working-set cap,
the device buffer limit and the total GPU budget. It copies uploaded pairs on
the GPU, without retaining CPU payloads. Resize, DPR, axis, grid and heatmap
colour-mapping edits recompute geometry and pixels from those pairs. Source
revision changes invalidate the cache; missing ranges and oversized sources use
ordinary streaming. This cache does not add immediate picking or change the
packed-view `view_residency` status. Export still requires the source provider.

Streaming does not enable auto-fit by itself. For a new chart with unknown
bounds, request fitting before the first chunk: each partial display uses the
bounds processed so far. Earlier pixels are rescaled while new chunks advance;
the final exact replay progressively replaces updated screen pixels while
retaining the preview elsewhere. Each pixel uses one image, without blending
both data images; decorations are drawn once. Unfinished overlapping geometry
in a replaced tile can change temporarily. On completion only the exact image
remains. If viewport-sized scratch resources do not fit the GPU budget, the
preview remains until the final swap. Preview widths and sharpness may change
temporarily. Completion commits the fitted range and tick
settings. Ordinary redraws then preserve zoomed or restored ranges. Request auto-fit again to show
all current data; restore saved axis ranges to return to a historical view.
See the [initial fitting and call order](WASM.md#streaming-fit-en)
for the facade, raw WASM, and native automatic-streaming APIs.

See the [web streaming contract](WASM.md#exact-streaming-en)
for the current API, support limits, and lifecycle details. Exact original-data
processing does not imply byte-identical antialiasing across GPU backends or
render-pass boundaries.

### Ownership and lifetime boundaries

`Renderer` owns the chart registry and GPU-side state: each chart's authoritative
`Config` and ordered `SeriesConfig`, the resident `ColumnPool`, nonresident
logical-source metadata, render/compute pipelines,
the shared picker pipeline bundle, at most one derived active-chart picker cache,
pending pool maintenance, renderer-level bind groups, and the shared
`Arc<wgpu::Device>` / `Arc<wgpu::Queue>`. It creates `ChartView` and `ChartStyle`
values and returns them to the host, which keeps them in its panel state (see
[the egui host](examples/egui_embed.rs)). A `PreparedFrame` holds
its own GPU handles and shared charges, so dropping a host view/style does not
necessarily release those resources. `WindowedRenderer` additionally retains
the surface, instance, adapter, and optional MSAA target. A `ChartId` is opaque
and bound to its issuing renderer.
`set_chart_state` atomically validates and replaces a chart's `Config` and
ordered series when one logical edit affects both.

Column upsert, removal, and defragmentation prepare all fallible pool, chart,
revision, and active-picker work before publishing the new authority state.
For returned synchronous errors, the previous pool/chart/picker state remains
intact; the final publication is allocation-free. Plain `remove_column`
cascade-removes every renderer-owned series that references the id and does not
rewrite any `Config::legend` document. A host that derives a legend update from
that cascade uses `remove_column_with_chart_config`, which publishes the pool,
all affected series, and that chart's replacement `Config` in the same
transaction. The web facade uses this combined boundary for its
auto-managed-versus-free-edited legend policy.

Exact GPU picking is chart-aware. Call
`enable_gpu_picking()` once, optionally call
`prepare_gpu_picking_for_chart(chart_id)` to make a chart first-pick-ready, and
submit through `pick_chart(chart_id, GpuPickRequest)` or
`WindowedRenderer::pick_chart_at`. The renderer derives axis transforms and the
data-area clip from its authoritative `Config`. Picking reads the GPU
column pool directly.
Use `pick_chart_data` / `WindowedRenderer::pick_chart_data_at` for the tagged
point, histogram-bin, matrix-cell, and contour-level result.

![Renderer frame preparation, referenced-state validation, stale-frame recovery and submission order](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/prepared-frame-en.png)

Every mutation — `Renderer::prepare` and the export prepare path — runs behind
an `&mut self` boundary. Final GPU owners hand retired handles and their exact
byte credits to an atomic queue. The host
submission boundary drains a bounded number of entries; completion destroys
those handles before crediting their bytes. Queue operations can spin/yield under
contention, so this does not promise wait-free progress. `Renderer::paint_prepared`
records through `&self` against an owned
`PreparedFrame` token, so host paint callbacks that only hand out shared access
need no wrapper lock (`Renderer` is `Send + Sync`).

The token captures resolved pipelines, bind groups, buffers, panel geometry,
column allocation epochs, pool layout generation, target-pipeline generation,
and each captured `ChartView` content revision. A mismatch fails before
recording with `FiggyError::StalePreparedFrame`; the host prepares again on the
next frame. Arc/star scratch and automatic contour placement are immutable
exact-key results: equal compute inputs share one result, while changed
geometry, data generation, or placement inputs allocate a new result and never
overwrite the old one. Their shared GPU charges live as long as the cache or a
prepared token owns the corresponding handles; after the last Figgy owner
drops, the charge becomes retired. After the host submits or discards all
previously recorded commands, `end_gpu_frame()` attaches a queue-completion
callback; the retired charge remains until that callback runs. Explicit contour placement is immutable too. This
guarantee does not replace host frame ordering: rewriting the same `ChartView`,
replacing a captured column, defragmenting the pool, or rebuilding target
pipelines deliberately makes the old token stale. Commands recorded from that
token must be submitted before one of those mutations.

For resident `add_column`, `ColumnSource` data is borrowed only during upload.
The long-lived records are the GPU-pool column and the scalar stats cached for
auto-fit (min / max / smallest-positive); source references and CPU-side
per-point geometry are not kept. Per-point geometry such as the arc prefixes of
dashed, sketch and Milkyway lines is derived from the GPU pool by compute scans.
Nonresident registration instead retains logical source metadata and bounded
GPU work; its host must keep the original range provider available for exact
replay. The renderer does not retain a full CPU copy of that source.

### Memory accounting and release

![Shared GPU resource ownership, final-reference retirement and reclamation after queue completion](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/gpu-resource-lifetime-en.png)

`gpu_memory_usage()` reports requested allocation bytes tracked by figgy, not
physical VRAM usage. Two lazy style-texture creation paths for Milkyway and
Constellation remain outside the regular ledger; each creates PSF, blackbody
LUT, planet atlas and ring strip textures. Streaming export admission separately checks their known
payload size. These exceptions must not be read as fully accounted resources.

`set_memory_budget` is a policy used by participating allocation/admission paths,
not a hard cap on every device allocation. It does not retroactively constrain
the constructor's initial pool or evict existing resources. Admission is not a
reservation. See the [budget contract](API.md#gpu-memory-accounting-and-budget-scope)
for return values, exclusions and shared-device host responsibilities.

Live resources and retired resources both count toward the tracked total.
Dropping a Rust handle does not prove that queued GPU work has finished, and
`end_gpu_frame()` does not itself free device memory. Native hosts must service
completion callbacks even while idle. Native host allocations, final RGBA/PNG
output buffers, and driver internals are outside this GPU-byte report. The
native upload contract above does not claim control over browser JS/wasm copies.

An in-flight `GpuPickTicket` owns its readback resources and an `Arc`-backed
identity mapping captured at submission. Later chart or pool mutations, and
even dropping the renderer, cannot remap that ticket's eventual
`source_id` / `series_id` result.

The browser public surface follows the same boundary. The `<figgy-chart>`
facade owns the shadow canvas, ready promise, rAF loop, ResizeObserver/DPR
handling, pointer mapping, async-operation busy gate, and id register/unregister
lifecycle. The raw `FiggyChart` wasm kernel remains available as an advanced
escape hatch.

Web cold-start and lifecycle contract (see also the
[native/browser constructor and prewarm comparison](API.md#construction-and-prewarming)):

In browsers, first-frame readiness awaits the actual
`GPUQueue.onSubmittedWorkDone()` Promise. A queue rejection or device loss
rejects creation/readiness; it cannot publish a successful first-frame event.

| Surface | Contract |
|---|---|
| Raw `FiggyChart` | In a wasm browser, `create` / `create_with_progress` warm every render WGSL entry on the same `GPUDevice` through Promise-based `createRenderPipelineAsync`, discard the temporary JS pipelines, then submit and await the first empty-chart frame. Production renderer-owned optional render/style and arc/fit/picker/contour compute caches remain lazy. `prewarm_all_with_progress(callback)` publishes those actual wgpu caches with `{ scope, stage, phase }` progress; `prewarm_all()` performs the same work without a callback. `warm_up()` is a first-frame compatibility alias, not full prewarm. Creation does not enable the production picker: `prewarm_gpu_picking()` explicitly enables it and prepares the current chart, while retries and `pick_point` / `pick_data` reuse the same renderer-owned path and sticky activation error. |
| `<figgy-chart>` startup | The `web.create / first frame / finished` progress event and `figgy-ready` are published before background picker prewarm begins. A prewarm failure emits `figgy-error` with `operation: "prewarm_gpu_picking"` and `recoverable: true`; the fulfilled `ready` promise and rendering loop remain valid. |
| Async serialization | One generation+kernel operation token covers connect/create, `prewarm_all_with_progress` / `prewarm_all` and picker prewarm, export, `first_frame_ready` / `warm_up`, extent preparation, resident async fit, and pick. The facade's two full-prewarm methods pass through this generation-aware operation gate. While `busy`, rAF drawing and pointer/proxy kernel access do not enter wasm; only the latest resize and a pending pointer release are retained and applied after settlement. Streaming `auto_fit_all()` instead requests renderer-owned fitting and awaits its stream job without holding this token: the fit itself does not set `busy`, so the scheduler can keep calling the kernel and reconcile the latest chart state. An independent operation may still set `busy`. |
| Disconnect/reconnect | Disconnect invalidates the generation and cancels its rAF/observer. A kernel borrowed by an active operation is freed only after that operation settles. Its stale settlement cannot clear, resize, release, or free the new generation's kernel. |

The example facade enforces that gate; wasm-bindgen itself checks borrows and
does not serialize or queue calls. Competing application calls to the facade
are rejected while busy, rather than queued. Raw kernel users must prevent
same-object access until an async mutable borrow settles, including on failure;
see the Korean-language [raw kernel call ownership](WASM.md#raw-kernel-call-ownership)
rules.

Web mutation API contracts:

| API | Contract |
|---|---|
| `auto_fit_colorbar(padding)` | Fits the shared colorbar z axis to the upload-metadata union of every matrix value column, scatter `point_color_map` column and vector colour column. A chart without a colorbar is unchanged. |
| `set_colorbar_axis(json)` | Replaces the existing colorbar's complete `AxisOptions` SSoT, covering tick style/direction/length, inversion, tick-label style/offsets, and title options. |
| `set_colorbar_title(text)` | Sets the colorbar title and shows it; an empty string hides it. Fails when `Config.colorbar` is absent. |
| `set_contour_nice_levels(series_id, target_count, use_colormap_colors)` | Uses the colorbar axis tick rules to replace one contour series' explicit levels, optionally assigns color-map colors, records the result in series SSoT, and returns the resulting level count. |
| `series_draw_info(series_id)` | Reports `{ drawn_count, cols, rows, truncated }`. The raw wasm `FiggyChart` returns a JSON string; the `<figgy-chart>` facade parses it and returns the object. |
| `pick_data(x, y, max_distance_px)` | Asynchronously returns a tagged point/bin/cell/contour identity or `null`; raw wasm returns its JSON string or `undefined`, and the facade parses it. |
| `set_picked_points(json)` | Accepts a JSON string encoding `PickedPointsConfig` or `null`. It replaces only renderer-owned `Config.picked_points`; `null` clears the overlay. References retain `series_id`, optional `source_id`, and `point_index`, not copied point coordinates. |
| `set_picked_data(json)` | Accepts `DataSelectionsConfig` or `null` and replaces only `Config.picked_data`. Refs retain stable indices and provenance; current geometry stays in the GPU-backed chart SSoT. |
| `set_clear_color(r, g, b, a)` | Accepts linear RGBA components, clamps each to `0..1`, and schedules a surface redraw. Clear color is host/surface state and does not modify Config JSON or force an axis-raster refresh. |

### Dashed-line arc scan (GPU)

The dash phase needs the cumulative pixel arc length at every point, which
depends on the live data→pixel transform. It is produced entirely on the GPU
for each distinct compute key; an exact key hit reuses the immutable result:

```
pool columns (x, y) ──┐                       Transform uniform (144 B write)
                      ▼                                   │
   seg_init           dst[i] = |px(pᵢ) − px(pᵢ₋₁)|   ◄────┘
   scan_block         256-block inclusive scans (Hillis–Steele, shared mem)
   scan_block/add     block-sum levels (dst → sums0 → sums1 → sums2)
   carry chain        chunks of min(dispatch limit × 256, 256³) points run
                      sequentially; a 1-element carry buffer folds each
                      chunk's total into the next — n is limited only by
                      u32 indexing (see below), with no readback at any size
                      ▼
   arc prefix buffer ──► line pipeline vertex slots 4/5 (dash phase)
```

The compute encoder is submitted before the host's render pass, so queue
order sequences it under every supported embedding (winit / egui / web) without
API changes. The exact key includes pool layout generation, x/y offsets and
allocation epochs, length, the first 64 bytes of the transform (axis bounds,
log flags, pixel and panel mapping), the line's step or polar-arc mode word, and
optional star pitch. Each series retains its eight most recent
immutable results; a miss dispatches into new buffers and never rewrites an old
result. The current arc-prefix scan is u32-addressable (`u32::MAX =
4,294,967,295`); if a series length or pool element offset cannot fit in
`u32`, the dashed arc prefix is skipped. As a runaway-churn backstop, adding a
new series id clears the arc cache when it already holds 256 series ids.

### Renderer-owned state and frame invalidation

Persistent hosts register a chart in `Renderer` and keep the last
`ChartRenderStamp` that was actually submitted and presented. The renderer is
the sole issuer of its checked revisions:

| State | Current trigger/handling |
|---|---|
| renderer `desired` revision | Any accepted visible chart edit, a referenced column replacement, or synchronized font registration requires a draw |
| renderer `raster` revision | A config edit that changes the axis chrome (chart area, axes, title, grid, legend, colourbar, draw style, `polar`, any `overlays` or `subplots` field), a series change, or a font registration requires `refresh_axis` before drawing. Selection (`picked_*`), `active_layer` and `annotations` edits and identical replacements do not touch it, except that a layer change which clears an element selection advances it; the classification is `raster_config_equal`, tested field by field in [`src/renderer/ssot_invalidation_tests.rs`](src/renderer/ssot_invalidation_tests.rs). `refresh_axis` also redoes the annotation rasters, because the anchor space follows the axes and the data area. A layered view refreshes only the panels whose internal config changed |
| renderer `annotation` revision | An edit of `annotations.items` requires `refresh_annotations` before drawing: it rewrites the annotation underlay and overlay textures and the annotation hit map only, never the grid or decoration texture (`ChartView::layer_stamps` counts each layer's writes). Toggling `annotations.visible` bumps neither this nor `raster`: the draw skips the two composites. Classification `annotation_config_revision`, tested by `annotation_edits` in the same test file |
| host `view_dirty` | Surface/DPR preview geometry changed; refresh raster and draw |
| host `redraw_pending` | Host-only surface state such as clear color changed; draw without duplicating chart state |

Browser frame flow, simplified from `FiggyChart::frame` in
[`crates/web/src/lib.rs`](https://github.com/shim9610/figgy/blob/master/crates/web/src/lib.rs):

```rust
renderer.sync_external_invalidations()?;
let stamp = renderer.chart_render_stamp(chart_id)?;
let draw = stamp.needs_draw_since(last_presented_stamp.as_ref());
let raster = stamp.needs_raster_since(last_presented_stamp.as_ref());
let annotations = stamp.needs_annotation_raster_since(last_presented_stamp.as_ref());

if !draw && !raster && !view_dirty && !redraw_pending {
    if renderer.has_pending_maintenance() {
        renderer.process_pending_maintenance()?;
    }
    return Ok(());
}
renderer.process_pending_maintenance()?;
let stamp = renderer.chart_render_stamp(chart_id)?;
if raster || view_dirty {
    // Also redoes the annotation rasters.
    renderer.refresh_axis_with_selection(&mut view, &display_chart, rect, &boxes)?;
} else if annotations {
    renderer.refresh_annotations(&mut view, &display_chart, rect, selected_annotation)?;
}
if renderer.is_streaming_chart(chart_id) {
    renderer.draw_registered(clear, &[RegisteredChartDrawItem { chart_id, view: &view }])?;
} else {
    renderer.draw(clear, &items)?;
}
last_presented_stamp = Some(stamp);
view_dirty = false;
redraw_pending = false;
```

The last-presented stamp and host flags advance only after a successful draw, so
a failed visual frame is retried. A clean web rAF skips the entire GPU surface
path. A resident redraw records data primitives again from the pool; that path
has no data-layer image cache. Nonresident rendering instead
keeps a bounded GPU accumulation surface for partial display and reuses an
unchanged completed revision.

The public `FiggyChart::load_demo()` call is compound failure-atomic. Its four
columns, final `Config` and ordered series, active picker state, host metadata,
and retained extent cache become visible together; any synchronous preparation
failure leaves the complete prior state visible. The transaction submits no
extent reduction, so invalidated extents are recreated by the normal lazy retry
path after commit. An accepted call temporarily allocates one full
column-pool-capacity GPU buffer plus four staging buffers. If a defragmentation
backup already exists, peak pool storage is primary + backup + that temporary
full-pool buffer.

The standalone `Chart::{data_dirty,raster_dirty}` booleans remain a compatibility
mechanism for low-level callers that own an external `Chart`. `prepare` does not
read or consume those booleans; it writes the transform whenever the caller has
decided to draw. External-`Chart` hosts remain responsible for consuming
`raster_dirty` and calling `refresh_axis`.

### Log scale on the GPU

When `AxisOptions.scale = Logarithmic`:
- Auto-fit uses the cached smallest-positive value when data contains zero or negative samples.
- Manual non-positive/non-finite range bounds are guarded in renderer/axis paths with `1e-12`; valid positive bounds, even below `1e-12`, are preserved.
- CPU: `scatter_transform_from_config` pre-converts the guarded range to log10 and sets the relevant `scale_log` axis flag.
- GPU line/scatter paths check finite coordinates and the positive log domain before drawing or picking. Invalid markers and line segments touching an invalid endpoint are omitted, leaving a gap; valid data is not rejected or connected across that gap. The same rule covers precise, sketch, ribbon/arc and star line paths. Histogram zero baselines remain clipped to the visible positive range; color-mapped non-positive z values use `nan_color`.

### Export pipeline

![Requested-resolution offscreen export, resident or replayed streaming input, row readback and PNG encoding](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/png-export-en.png)

```
export_panel_rgba_async(chart, &[SeriesConfig], scale).await:
    scale ← clamp_export_scale(scale)         // [MIN_EXPORT_SCALE, MAX_EXPORT_SCALE]
    chart.config().scaled(scale)               // every pixel dim scaled proportionally
        ↓
    temp ChartView (scaled axis textures)
    temp ChartStyles ← create_style_for_series_scaled(cfg, scale) per cfg
        ↓
    offscreen wgpu::Texture (fixed Rgba8Unorm, COPY_SRC, transparent clear)
    paint(items) — same compositing order (grid → annotation underlay → data → decoration → annotation overlay)
        ↓
    copy_texture_to_buffer in ROW CHUNKS (256-byte aligned padding; chunk
    height adapts to the device's max buffer size, so readback never needs a
    whole-image buffer; the target must still fit the device texture limit)
        ↓
    map_async (+ inline Wait poll on native, browser-yielding await on wasm)
        ↓
    premul→straight α conversion, padding rows removed (no channel swap —
    the target is RGBA already)
        ↓
    RasterImage { width, height, rgba: Vec<u8> }   ← API return
        ↓
    encode_png(&img) → Vec<u8>                      ← PNG bytes
        ↓
    Caller decides: std::fs::write / clipboard / network / ...
```

`export_document_rgba_async` follows the same pipeline for a layered document:
the one temporary `ChartView` holds a panel per layer, `Config::expand()` turns
the scaled document into one draw item per panel, and the panels composite in
layer order into the same offscreen texture. `export_panel_*` keeps only the
base panel of that expansion; for a document without overlays the two images
are byte-identical. A document with subplots exports the union of every chart
area (`Config::document_area`): the document is placed at the target's
origin (`place_document`) and each subplot's panels are drawn at its own
area, while `export_panel_*` keeps the parent's area.

---

---

아래 구조 설명은 직교 좌표 `Renderer`의 네이티브 렌더링, 데이터 소유권, GPU 메모리와 프레임 갱신 판단에 해당한다. 원형·범주형 막대·박스플롯·레이더 차트는 별도 렌더러(`RadialRenderer`, `CategoricalRenderer`, `BoxPlotRenderer`, `RadarRenderer`)가 그린다. 각 렌더러는 변경할 수 없는 준비 프레임을 반환하고 자체 GPU 집계(`gpu_memory_usage()`)와 예산(`set_memory_budget(bytes)`)을 관리하며, 컬럼 풀·자동 범위 맞춤 작업·스트리밍을 사용하지 않는다. 전체 GPU 메모리를 보고하는 호스트는 모든 렌더러의 `gpu_memory_usage()`를 더한다. 자세한 내용은 [RADIAL.md](RADIAL.md), [CATEGORICAL.md](CATEGORICAL.md), [BOXPLOT.md](BOXPLOT.md), [RADAR.md](RADAR.md)를 참고한다. 실행 가능한 예제는 [README](https://github.com/shim9610/figgy/blob/master/README.ko.md)에서 시작한다.

아래의 브라우저 수명 주기 설명은 `crates/web` 예제 어댑터의 동작이며, 모든 렌더러를 JavaScript API로 제공한다는 뜻은 아니다([브라우저 예제 범위](https://github.com/shim9610/figgy/blob/master/crates/web/README.md)).

주요 기능은 다음과 같다. 선택 기능은 해당 API나 옵션을 사용할 때만 동작한다.

- **상주 GPU 컬럼 풀**: 여러 컬럼이 하나의 GPU 버퍼를 공유하며, 충분한 크기의 첫 빈 공간에 할당한다. 컬럼을 제거하면 두 버퍼를 번갈아 쓰는 재배치를 예약하고, 호스트가 `process_pending_maintenance()`나 `defragment()`를 호출할 때 실행한다. `DefragPolicy::OnAllocFailure`를 지정하면 공간이 부족한 업로드도 한 번 재배치한 뒤 다시 시도한다. 기본 정책은 `Manual`이다. `HiLoColumnSource`로 올린 값은 두 f32의 hi/lo 쌍으로 저장하므로 큰 타임스탬프에서도 작은 값 차이를 보존한다. 업로드할 때 최소·최대·최소 양수 값을 저장해 자동 범위 맞춤에 사용한다. 점선의 누적 경로 길이처럼 점마다 필요한 값은 GPU 스캔 연산(`line_arc.wgsl`)으로 계산한다.
- **비상주 렌더링**: 컬럼의 ID·길이·인코딩·리비전만 등록하고 전체 데이터를 GPU 풀에 보관하지 않는다. 렌더러가 제한된 크기로 원본 구간을 요청해 빠짐없이 그린다. 화면이나 출력 조건이 바뀌면 호스트가 같은 원본을 다시 공급한다. 등고선, 은하수 모드의 선, 극좌표 차트, 선 채우기와 계단 보간, 벡터 시리즈, 점 색상·크기 매핑, 오버레이 레이어나 서브플롯이 있는 문서는 스트리밍 원본을 명시적인 오류로 거부한다. 실행 가능 조건, 지원 스타일, 취소와 완료 결과 조회는 [WASM 가이드](WASM.md#exact-streaming)를 참고한다.
- **레이어 합성**: 격자, 주석 아래층, 데이터, 축·라벨·범례, 주석 위층 순서로 그려 격자가 데이터를 가리지 않게 한다. 축 이미지는 기본적으로 `Grid`와 `Decoration` 레이어로 나눈다. 기존 단일 패스 방식은 `AxisLayerKind::All`로 사용할 수 있다.
- **원본 데이터 보존**: 렌더러와 웹 래퍼는 모델의 설정을 따르며 원본 좌표, 데이터 출처, 축과 데이터의 대응 관계를 임의로 바꾸지 않는다. 화면 밖 자르기, 로그축에서 0 이하 값 제외, NaN 제외, 안티앨리어싱은 표시 과정에만 적용한다.
- **창 없이 PNG 출력**: 배율 0.25~8.0(약 24~768 DPI)으로 GPU에서 그린 뒤 RGBA 또는 PNG 바이트를 반환한다. 범위를 벗어난 배율은 `clamp_export_scale`이 범위 안으로 제한하며, 배율을 적용한 렌더 타깃은 장치의 텍스처 크기 한도 안에 들어와야 한다. 비동기 API를 기본으로 제공하며 네이티브에서는 동기 래퍼도 쓸 수 있다.
- **선택·드래그·크기 조절**: 마우스 위치 판정, 선택 상자, 축에 수직인 방향으로 드래그, `line_offset`을 이용한 축 이동, 데이터 영역의 8개 조절점을 지원한다. 정책은 `model`에 정의돼 있으며 호스트가 포인터 이벤트를 전달할 때만 동작한다.
- **데이터 피킹**: `pick_point`는 점·선 선택을, `pick_data`는 여기에 히스토그램 구간·행렬 셀·등고선 레벨 선택을 더한다. 상주 데이터는 그리기와 같은 좌표 변환·풀·스타일·격자·레벨 표를 GPU에서 읽어 판정한다. 완료된 차트별 패킹 캐시는 원본을 다시 읽지 않고 점·선을 선택하며 원본 행 인덱스를 반환한다. 그 밖의 비상주 스트림은 `null`을 반환하고 저수준 WASM 스트림 피킹 호출은 즉시 거부한다. 이미 알고 있는 데이터 참조를 `Config.picked_data` 또는 `Config.picked_points`에 지정하면 필요한 행만 읽어 강조할 수 있다.
- **점별 스타일**: 정밀 모드의 산점도는 `point_style_table` / `point_style_index_column` / `point_style_overrides`로, 오차 막대는 별도의 `error_bar_style_table` / `error_bar_style_index_column` / `error_bar_style_overrides`로 스타일을 지정한다. 스케치·은하수·별자리 모드는 전용 셰이더를 사용하므로 이 매핑을 적용하지 않는다.
- **리치 텍스트**: 제목·눈금 라벨·범례가 같은 텍스트 엔진을 사용한다. 구간별 굵게·기울임·밑줄·위아래 첨자·그리스 문자, 색·크기 지정, `\n` 줄바꿈, `\t` 열 정렬, 고정 너비 범례 기호를 지원한다.
- **스케치 모드**: `draw_style: { mode: "sketch", amplitude_px, wavelength_px, seed }`로 축·장식과 선·산점도·오차 막대 시리즈를 손그림처럼 표현한다. 축·눈금·격자·범례는 CPU에서, 선의 흔들림과 점선 간격은 경로 길이를 계산한 GPU에서 처리한다. 점과 오차 막대도 전용 GPU 셰이더를 쓴다. 텍스트에는 내장 손글씨 폰트 Comic Neue(OFL)를 사용한다. 이 폰트에 없는 글자는 글자마다 지정한 글꼴, 등록된 폰트, 내장 Liberation Sans, 설치된 시스템 폰트(네이티브 전용) 순으로 찾아 그린다. 같은 시드와 입력은 같은 결과를 내며 점선과도 함께 사용할 수 있다. 이 옵션을 생략하면 기본 정밀 모드로 그린다. 히스토그램·히트맵·등고선·벡터 시리즈는 스케치용 셰이더가 없어 오류 없이 생략되므로, 그릴 수 있는 조합인지는 `draw_style_supports(style, render_type)`로 확인한다.
- **은하수 모드**: `draw_style: { mode: "milkyway", ... }`로 선·산점도·오차 막대 시리즈에 천체사진 같은 효과를 낸다. 선은 시리즈 색의 성운 띠와 별 무리로, 산점도 기호는 고리가 있는 행성으로, 오차 막대는 양방향으로 뻗는 제트로 표현한다. 스케치 모드와 마찬가지로 히스토그램·히트맵·등고선·벡터 시리즈는 오류 없이 생략한다.
- **별자리 모드**: `draw_style: { mode: "constellation", ... }`는 `ScatterLine` 시리즈만 지원한다. 데이터 점 위치에 점광원 분포 함수(PSF)로 별을 그리고 반투명 선으로 잇는다. 프로그램에서 옵션 범위를 조회할 수 있도록 `draw_style_param_specs` 메타데이터를 제공한다.
- **wgpu 30 통합**: 렌더러와 egui 통합이 같은 wgpu 주 버전을 사용한다. iced 0.14는 wgpu 27을 사용하므로 보관 중인 iced 통합 예제는 빌드 대상에서 제외한다.
- **WebAssembly**: 순수 Rust로 구현된 tiny-skia·fontdb·swash와 비동기 초기화·출력 API를 사용한다. Rust의 `text_render::register_font_bytes`나 브라우저 예제 `FiggyChart`의 `register_font`로 실행 중에 폰트를 등록하면 한중일 문자와 사용자 지정 글꼴을 사용할 수 있다.
- **웹 예제의 초기화**: 브라우저의 wasm에서 `create` / `create_with_progress`는 모든 렌더링용 WGSL 진입점을 미리 컴파일하고 빈 차트의 첫 프레임을 기다린다. 선택적인 렌더링·스타일·컴퓨트 캐시는 처음 사용하거나 `prewarm_all` / `prewarm_all_with_progress`를 호출할 때 만든다. 자세한 규칙은 [메모리 계상과 회수](#메모리-계상과-회수)의 웹 초기화 표를 참고한다.

<a id="3-내부-메모리-데이터-흐름"></a>

## 내부 구조와 메모리 흐름

![렌더러 라이브러리 구조: 컬럼 기반·전용 차트 경로와 화면·이미지 출력](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/architecture-state-flow-kr.png)

그림에는 주석 아래층·위층 텍스처, 오버레이 레이어, 서브플롯이 빠져 있다. 6번 영역의 설명도 현재 구현과 다르다. 누적 경로 길이는 점선과 스케치·은하수 선에 쓰이며 별자리 스타일은 따로 만들지 않는다. 시리즈마다 쓰기 시 복사(copy-on-write) 임시 버퍼 대신 불변 결과를 최대 8개 보관한다.

### 렌더링 계약

전체 구조도는 컬럼 기반 `Renderer`와 도넛·범주형·박스플롯·레이더의 전용 렌더러 경로를 구분한다. 전용 차트의 불변 프레임은 일반 `PreparedFrame`이나 컬럼풀·스트리밍 계약을 공유하지 않는다. 화면 표시와 이미지 출력은 별도 경로이며, 이미지는 요청한 크기로 다시 렌더링한다. 아래 표는 그림의 번호 대신 주제별 계약을 요약한다.

| 주제 | 데이터 흐름과 소유권 |
|---|---|
| **크레이트별 역할** | `model`은 차트 설정·레이아웃·상호작용 정책을 정의하고 GPU 자원을 갖지 않는다. `renderer`는 등록된 차트, GPU 연산, CPU 래스터 렌더링을 관리한다. `web`은 이를 캔버스·입력·브라우저 실행 흐름에 연결한다. 저수준 네이티브 호스트는 차트를 등록하는 대신 `ChartDrawItem`을 직접 전달할 수도 있다. |
| **컬럼 업로드와 정리** | 네이티브에서는 업로드하는 동안만 `ColumnSource`를 빌려 f32 쌍을 매핑된 스테이징 버퍼에 쓰고 최소 양수 값을 계산한다. GPU가 데이터를 풀로 복사한 뒤에는 슬롯과 스칼라 메타데이터만 남기며 CPU 점 배열은 보관하지 않는다. 충분한 첫 빈 공간에 할당하고 인접 빈 공간은 합친다. 실패하면 예약을 취소해 빈 공간 목록을 복구한다. 교체는 관련 상태를 모두 준비한 뒤 확정한다. allocation epoch는 내용 교체를, layout generation은 재배치로 인한 위치 변경을 구분한다. |
| **차트 상태와 패널 자원** | `Renderer`는 등록된 설정·시리즈, 풀, 파이프라인과 공유 장치·큐를 보관한다. 호스트가 보관하는 `ChartView`는 래스터 텍스처와 좌표 변환 자원을, `ChartStyle`은 GPU 바인딩을 갖는다. `WindowedRenderer`는 surface·instance·adapter도 유지한다. 축·텍스트·격자의 CPU 렌더링에는 이미지 버퍼가 필요할 수 있다. 컬럼 업로드의 중간 버퍼를 없앴다고 이 이미지 할당까지 없어지는 것은 아니다. |
| **프레임 갱신 판단** | 렌더러가 발급한 desired/raster 리비전과 마지막 표시 성공 기록을 비교한다. 래스터 설정이나 표시 영역이 바뀌면 축 이미지를 갱신한다. 배경색 같은 호스트 상태는 차트 설정을 바꾸지 않고 다시 그릴 수 있다. 웹에서는 변경이 없는 프레임의 surface 획득과 그리기를 생략하되 필요한 정리·완료 처리는 계속한다. 표시가 실패하면 마지막 성공 기록을 갱신하지 않는다. |
| **준비와 명령 기록** | `prepare(&mut self)`가 입력을 확정하고 GPU 핸들·공유 할당 정보·검증용 상태를 `PreparedFrame`에 담는다. `paint_prepared(&self)`는 기록 전에 이를 검증한다. 뷰, 참조 컬럼, 풀 배치, 출력 파이프라인 또는 등록 차트 리비전이 바뀌면 거부한다. 호스트는 이 변경 전에 기존 명령을 제출해야 한다. 토큰은 외부 명령 버퍼의 제출 순서까지 검사하지 않는다. |
| **GPU 파생 데이터** | 누적 경로 길이, 피킹, 실제 도형의 범위는 GPU 컬럼에서 계산한다. 경로 길이 스캔은 청크별 계산 결과를 이어받으므로 시리즈 전체에 1670만 점의 고정 상한을 두지 않는다. 피킹과 범위 계산은 전체 점을 CPU로 복사하지 않고 작은 결과만 반환한다. 캐시와 프레임 토큰이 같은 계산 결과를 공유할 수 있으므로 캐시에서 지워도 자원이 바로 해제되지는 않을 수 있다. |
| **화면 표시와 이미지 출력** | 격자·주석 아래층·데이터·장식·주석 위층 순서로 합성한다. 출력할 때는 배율을 적용한 차트·뷰·스타일과 화면 밖 렌더 타깃을 준비한다. 정렬용 여백이 있는 행을 청크 단위로 읽어 오고, 알파가 미리 곱해진 RGBA를 일반 RGBA로 바꾼다. 청크 처리는 읽기 버퍼의 크기를 제한하지만 렌더 타깃·최종 RGBA·PNG의 메모리까지 없애지는 않는다. 상주 출력은 GPU 컬럼을, 스트리밍 출력은 다시 공급받은 원본 구간을 사용한다. |
| **원본 보존과 캐시** | 상주 경로는 데이터 이미지 캐시 없이 GPU 컬럼에서 다시 그린다. 비상주 경로는 누적 이미지를 유지하고, 지원되는 차트에서는 현재 화면에 필요한 원본 행만 모은 GPU 패킹 캐시를 사용할 수 있다. 두 경로 모두 LOD·샘플링·데시메이션으로 데이터를 줄이지 않는다. 화면 밖 자르기, 잘못된 값·로그 범위 밖 값 제외, 안티앨리어싱은 표시 규칙이며 GPU 백엔드 간 픽셀값 일치를 보장하지 않는다. |
| **메모리 집계와 회수** | figgy에서 자원을 참조하는 동안 공유 할당 정보도 유지한다. 마지막 참조가 해제돼도 회수 대기 바이트는 제출 경계를 알리고 GPU 큐의 작업이 끝날 때까지 집계한다. 호스트는 기존 명령을 제출하거나 폐기한 뒤 `end_gpu_frame()`을 호출해야 하며, 네이티브에서는 완료 콜백도 처리해야 한다. 아래에 설명한 스타일 텍스처 등은 집계에서 제외되며 보고값은 실제 VRAM 측정값이 아니다. |

### 상주 컬럼 업로드

![네이티브 상주 업로드: 빌린 원본, 매핑된 스테이징, 실제 GPU 복사와 CPU 메타데이터](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/column-upload-kr.png)

여기서 제로카피는 별도 CPU 점 배열 없이 매핑된 스테이징에 직접 기록한다는 뜻이다. GPU 복사는 존재하며, 축·문자용 CPU 이미지 버퍼는 컬럼 업로드의 보장 범위와 별개다.

### 원본 데이터 스트리밍과 상주 전환

![컬럼 기반 화면 갱신 루프: 조건부 CPU 래스터화, 상주 GPU 데이터와 청크 스트리밍·합성](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/streaming-architecture-kr.png)

루프 그림은 최초 상주 업로드를 화면 갱신 밖에 두고, 필요한 CPU 래스터 갱신과 재사용, 청크별 데이터 그리기와 합성을 갱신 루프 안에 표시한다. 스트리밍은 직교좌표 `Renderer`의 계약이며 전용 차트 렌더러의 경로가 아니다. 선택적인 GPU 캐시와 원본 재공급은 아래 본문에서, 별도 출력 경로는 내보내기 삽화에서 설명한다. 네이티브 호스트는 재공급할 수 있는 원본을 보관하고 요청된 구간을 전달한다. 브라우저에서는 TypedArray나 `readRange` 공급자를 사용한다. 웹 래퍼는 실행 순서와 구간 요청을 관리하며, 차트 설정·시리즈 순서·소스 리비전·처리 위치·범위 통계·캐시 사용 여부는 렌더러가 관리한다.

자동 스트리밍은 연결된 컬럼 전체를 `ColumnPool`에 넣지 않는다. 호스트가 상주로 옮기려면 `resident_admission_for_streamed_columns`로 상태를 바꾸지 않고 가능 여부를 확인한 뒤, `promote_streamed_columns_to_resident`로 한 번에 반영한다. 이 반영이 실패하면 기존 상태가 그대로 남는다. 원본을 제한된 청크로 GPU에 올려 화면 밖 렌더 타깃에 누적한다. 지원되는 차트에서는 예산이 허용할 때 현재 화면에 필요한 원본 행만 GPU 캐시에 남긴다. 두 경로 모두 원본 도형을 그리며 LOD·샘플링·데시메이션을 적용하지 않는다. 한 페이지에서 상주 차트와 스트리밍 차트를 함께 사용할 수 있다.

스트리밍은 완료된 부분부터 화면에 표시한다. 입력이 바뀌지 않은 완료 결과는 재사용하고, 제목·축·범례 같은 장식만 바뀌면 데이터 처리 위치와 누적 이미지를 유지한다. 표시 범위나 물리 해상도가 바뀌면 새 좌표 변환으로 다시 그린다. 필요한 원본이 GPU 캐시에 있으면 재사용하고, 빠진 구간만 호스트에 다시 요청한다.

`job.cancel()`은 새 작업을 멈추고 이미 제출된 GPU 작업이 끝난 뒤 자원을 정리한다. 완료된 패킹 캐시에서는 GPU의 점·선을 선택하고 원본 행 인덱스를 반환할 수 있다. 그 밖의 비상주 스트림은 즉시 피킹을 지원하지 않는다. 배율을 지정한 PNG 출력은 화면 이미지를 늘리는 대신 원본을 다시 읽어 그린다. 따라서 호스트는 다시 그리기와 출력을 위해 원본을 유지해야 한다.

스트리밍 자체는 자동 맞춤을 켜지 않는다. 범위를 모르는 새 차트는 첫 청크 전에 맞춤을 요청한다. 앞서 그린 데이터 이미지를 새 범위로 리스케일하고 새 청크를 이어 그린다. 중간 화면의 선 두께·선명도는 잠시 달라질 수 있다. 전체 범위가 확정되면 원본을 한 번 더 읽어 정밀하게 그리며, 새 청크가 반영된 화면 구역부터 정밀 결과로 교체한다. 나머지 구역은 미리보기를 유지하고, 두 데이터 이미지를 겹쳐 합성하지 않는다. 같은 구역에 겹친 미처리 도형의 중간 모습은 잠시 달라질 수 있지만, 완료 시에는 정밀 결과만 사용한다. 화면 크기에 비례하는 임시 자원이 GPU 예산에 들어오지 않으면 미리보기를 유지했다가 완료 후 한꺼번에 교체한다. 축 범위와 눈금 설정은 완료 때 함께 확정한다. 이후 일반 재그리기는 확대·이동한 범위나 저장한 범위를 유지한다. 전체 데이터를 다시 보려면 자동 맞춤을 재요청하고, 과거의 특정 범위로 돌아가려면 저장한 축 범위를 복원한다. 웹 래퍼·저수준 WASM·네이티브의 호출 순서는 [범위 맞춤 사용법](WASM.md#streaming-fit)을 참고한다.

점선·히스토그램·히트맵과 지원되는 스타일 스트림은 원본 순서와 인접 관계를 보존하는 GPU 캐시도 사용할 수 있다. 참조 컬럼 전체의 hi/lo 인코딩 크기가 차트별 상주 한도·장치의 단일 버퍼 한도·전체 GPU 예산에 들어올 때만 사용한다. 청크를 올리면서 GPU 안에서 복사하며 CPU 데이터 복사본은 남기지 않는다. 크기·DPR·축·그리드·히트맵 색상 매핑이 바뀌면 새 설정으로 다시 계산한다. 데이터 리비전이 바뀌면 이전 캐시를 사용하지 않고, 빠진 구간이나 한도보다 큰 원본은 일반 스트리밍으로 처리한다. 이 캐시는 즉시 피킹을 추가하지 않으며 `view_residency`는 기존의 패킹된 뷰 상태를 나타낸다. 출력과 캐시 미적중에 대비해 원본 공급자는 계속 보관해야 한다.

지원 범위와 웹 API는 [WASM 가이드](WASM.md#exact-streaming)를 참고한다. 원본을 빠짐없이 처리하더라도 GPU 백엔드나 렌더 패스 경계에 따라 안티앨리어싱 픽셀값은 달라질 수 있다.

웹 래퍼의 차트 상태는 렌더러의 레지스트리를 기준으로 한다. 저수준 네이티브 호스트는 `ChartDrawItem`을 직접 전달할 수도 있다. 어느 경로든 `ChartDrawItem`은 준비 단계에서만 사용하며, 명령 기록 단계는 `PreparedFrame`에 보관된 입력을 사용한다.

### 소유권과 수명 경계

`Renderer`는 차트별 `Config`, 순서가 있는 `SeriesConfig`, 상주 `ColumnPool`, 비상주 소스 메타데이터, 렌더링·컴퓨트 파이프라인을 보관한다. 피킹 파이프라인은 공유하고, 현재 차트용 피킹 캐시는 최대 하나만 유지한다. 대기 중인 풀 정리 작업, 렌더러 공통 바인드 그룹, `Arc<wgpu::Device>`와 `Arc<wgpu::Queue>`도 관리한다.

생성해서 반환한 `ChartView`와 `ChartStyle`은 호스트가 패널 상태에 보관한다([egui 예제](examples/egui_embed.rs)). `PreparedFrame` 역시 GPU 핸들과 공유 할당 정보를 유지하므로 호스트가 뷰·스타일을 해제해도 자원이 즉시 사라지지는 않을 수 있다. `WindowedRenderer`는 추가로 surface·instance·adapter와 선택적인 MSAA 렌더 타깃을 보관한다. `ChartId`는 발급한 렌더러에서만 유효하다. 설정과 시리즈를 함께 편집하려면 `set_chart_state`로 둘을 검증한 뒤 한꺼번에 교체한다.

컬럼 교체·제거·재배치는 풀, 차트, 리비전, 피킹 캐시를 먼저 준비하고 성공했을 때만 반영한다. 동기 준비 과정에서 오류가 나면 기존 상태를 유지하며, 마지막 반영 단계에서는 새 메모리를 할당하지 않는다. `remove_column`은 해당 컬럼을 참조하는 모든 등록 시리즈를 함께 제거하지만 `Config::legend`는 바꾸지 않는다. 범례까지 함께 바꾸려면 `remove_column_with_chart_config`를 사용한다. 이 호출은 풀, 영향을 받는 모든 시리즈, 해당 차트의 새 `Config`를 하나의 트랜잭션으로 반영한다. 웹 래퍼는 자동 범례와 직접 편집한 범례를 구분해 이 API를 사용한다.

GPU 피킹은 차트 ID를 받는 API로 제공한다. `enable_gpu_picking()`을 한 번 호출하고, 필요하면 첫 선택 전에 `prepare_gpu_picking_for_chart(chart_id)`로 준비한다. 이후 `pick_chart(chart_id, GpuPickRequest)` 또는 `WindowedRenderer::pick_chart_at`을 호출한다. 축 변환과 데이터 영역 자르기는 렌더러가 `Config`에서 계산한다. 피킹은 GPU 컬럼 풀을 직접 읽는다. 점·막대·셀·등고선을 구분하는 결과가 필요하면 `pick_chart_data` / `WindowedRenderer::pick_chart_data_at`을 사용한다.

![Renderer의 프레임 준비, 참조 상태 검증, 오래된 프레임의 재준비와 명령 제출 순서](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/prepared-frame-kr.png)

`Renderer::prepare`와 출력 준비 과정의 상태 변경은 모두 `&mut self`에서 수행한다. `Renderer::paint_prepared`는 `&self`로 토큰의 명령을 기록하므로 공유 참조만 제공하는 paint 콜백에서도 별도 잠금이 필요하지 않다. `Renderer`는 `Send + Sync`다. GPU 자원 회수는 mutex 없이 원자적 큐로 핸들과 해당 바이트 수를 함께 전달한다. 제출 경계에서 모은 자원은 GPU 완료 후 파괴하고 집계에서 차감한다.

토큰은 확정된 파이프라인·바인드 그룹·버퍼·패널 배치와 함께 컬럼 할당 세대, 풀 배치 세대, 출력 파이프라인 세대, 각 `ChartView`의 내용 리비전을 보관한다. 하나라도 바뀌면 기록 전에 `FiggyError::StalePreparedFrame`을 반환하므로 다음 프레임에서 다시 준비해야 한다.

경로 길이·별 위치와 자동 등고선 라벨 배치 결과는 컴퓨트 입력의 비트값을 키로 삼는 불변 자원이다. 입력이 같을 때만 공유하고, 좌표 변환·데이터 세대·배치 입력이 다르면 새 결과를 만든다. 기존 결과를 덮어쓰지 않는다. 캐시나 토큰이 GPU 핸들을 보관하는 동안 공유 할당 정보도 유지한다. 마지막 figgy 참조가 해제되면 회수 대기 상태로 옮기고, `end_gpu_frame()`으로 제출 경계를 알린 뒤 GPU 큐의 완료 콜백이 실행될 때까지 메모리 집계에 포함한다. 호스트는 이 호출 전에 기존 명령을 모두 제출하거나 폐기해야 한다.

직접 지정한 라벨 배치도 불변 자원이다. 같은 `ChartView`를 다시 작성하거나 참조 컬럼을 교체하거나 풀·출력 파이프라인을 재구성하면 기존 토큰은 무효가 된다. 호스트는 이 변경 전에 토큰으로 기록한 명령 버퍼를 제출해야 한다.

상주 `add_column`은 업로드할 때만 `ColumnSource`를 빌린다. 이후에는 GPU 컬럼과 자동 범위 맞춤용 최소·최대·최소 양수 값만 보관한다. 원본 참조나 점별 CPU 도형 정보는 유지하지 않는다. 점선·스케치·은하수 선에 필요한 누적 경로 길이도 GPU 풀을 스캔해 계산한다. 비상주 경로에서는 소스 메타데이터와 제한된 GPU 작업 상태만 유지하며, 호스트가 원본을 다시 공급할 수 있어야 한다. 렌더러는 원본 전체의 CPU 복사본을 보관하지 않는다.

### 메모리 계상과 회수

![공유 GPU 자원의 참조, 마지막 참조 해제 후 회수 대기와 GPU 완료 후 집계 해제](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/gpu-resource-lifetime-kr.png)

`gpu_memory_usage()`는 figgy가 추적하는 GPU 자원의 요청 할당량을 바이트로 반환한다. 실제 VRAM 사용량을 측정한 값은 아니다. 은하수·별자리 스타일이 필요할 때 생성하는 PSF·아틀라스·띠 텍스처는 일반 집계에서 제외된다. 다만 스트리밍 출력의 실행 가능 여부를 검사할 때는 알려진 텍스처 크기를 별도로 고려한다.

사용 중인 자원(`live`)과 회수 대기 자원(`retired`)은 모두 합계에 포함한다. Rust 핸들을 해제해도 제출된 GPU 작업은 계속 실행될 수 있으며, `end_gpu_frame()` 호출만으로 장치 메모리가 반환되지는 않는다. 네이티브 호스트는 새 프레임을 그리지 않을 때도 완료 콜백을 처리해야 한다. CPU 할당, 최종 RGBA·PNG 버퍼, 드라이버 내부 메모리는 이 집계에 포함하지 않는다. 네이티브 업로드 규칙은 브라우저의 JS·WASM 경계에서 생기는 복사에는 적용되지 않는다.

진행 중인 `GpuPickTicket`은 결과 읽기용 자원과 제출 당시의 `Arc` 기반 식별자 매핑을 직접 보관한다. 차트·풀이 바뀌거나 렌더러가 해제돼도 해당 요청의 `source_id` / `series_id`가 다른 대상을 가리키게 되지는 않는다.

웹에서도 같은 소유권 경계를 따른다. `<figgy-chart>`는 내부 캔버스, `ready` Promise, rAF 루프, ResizeObserver·DPR 처리, 포인터 변환, 비동기 호출 직렬화, ID 등록·해제를 담당한다. 이 과정을 직접 제어해야 할 때만 저수준 WASM 클래스인 `FiggyChart`를 사용한다.

웹의 초기화와 객체 수명 관리 규칙은 다음과 같다([네이티브·브라우저 생성자와 사전 준비 비교](API.md#construction-and-prewarming)도 참고한다). 첫 프레임 준비는 실제 `GPUQueue.onSubmittedWorkDone()` Promise가 완료될 때까지 기다린다. 큐 요청이 거부되거나 장치가 소멸하면 생성·준비 오류로 전달하며 첫 프레임 성공 이벤트를 보내지 않는다.

| API·상태 | 동작 규칙 |
|---|---|
| 저수준 `FiggyChart` | `create` / `create_with_progress`는 같은 `GPUDevice`에서 모든 렌더링용 WGSL 진입점을 `createRenderPipelineAsync`로 미리 컴파일한다. 임시 JS 파이프라인을 해제하고 빈 차트의 첫 프레임을 제출한 뒤 완료를 기다린다. 선택적인 렌더링·스타일 자원과 경로 길이·범위·피킹·등고선 컴퓨트 캐시는 필요할 때 만든다. `prewarm_all_with_progress(callback)`은 이를 실제 wgpu 캐시에 생성하며 `{ scope, stage, phase }`를 알린다. `prewarm_all()`은 콜백 없이 같은 작업을 한다. `warm_up()`은 첫 프레임 대기용 호환 메서드다. 생성 시 피킹을 켜지는 않으며, `prewarm_gpu_picking()`으로 활성화하고 현재 차트를 준비한다. 재시도와 `pick_point` / `pick_data`도 저장된 활성화 오류를 포함한 같은 렌더러 상태를 사용한다. |
| `<figgy-chart>` 시작 | `web.create / first frame / finished`와 `figgy-ready`를 알린 뒤 백그라운드 피킹 준비를 시작한다. 실패하면 `operation: "prewarm_gpu_picking"`, `recoverable: true`인 `figgy-error`를 보낸다. 이미 완료된 `ready`와 렌더링 루프는 유지한다. |
| 비동기 호출 직렬화 | 연결 세대와 커널을 확인하는 작업 토큰으로 생성, 전체 사전 준비, 피킹 준비, 출력, 첫 프레임 대기, 범위 엔진 준비, 상주 자동 범위 맞춤과 피킹을 직렬화한다. 작업 중에는 rAF·포인터·대리 호출이 WASM에 접근하지 않으며 마지막 크기 변경과 포인터 해제만 보관했다가 종료 후 적용한다. 스트리밍 `auto_fit_all()`은 이 토큰 없이 렌더러에 범위 계산을 요청하고 스트림 완료를 기다린다. 스트리밍 실행기가 커널 호출을 계속하고 최신 차트 상태 요청을 반영한다. 범위 계산 자체는 busy를 켜지 않지만 별도 작업이 실행 중이면 busy일 수 있다. |
| 연결 해제·재연결 | 연결 해제 시 이전 세대를 무효화하고 rAF와 관찰자를 정리한다. 작업 중인 커널은 작업 종료 후 해제한다. 이전 세대의 완료 처리는 새 토큰·커널에 영향을 주거나 새 세대의 크기 변경·포인터 해제·객체 해제를 수행하지 않는다. |

이 직렬화는 예제 래퍼가 수행한다. wasm-bindgen은 대여 상태만 검사하며 호출을 직렬화하거나 대기열에 넣지 않는다. 래퍼가 작업 중일 때 애플리케이션의 경쟁 호출은 대기하지 않고 거부된다. 저수준 커널을 직접 쓰는 호스트는 비동기 가변 대여가 끝날 때까지(실패한 경우 포함) 같은 객체에 접근하지 않아야 한다. 자세한 내용은 [저수준 커널 호출 소유권](WASM.md#raw-kernel-call-ownership)을 참고한다.

웹 상태 변경 API는 다음과 같다.

| API | 동작 |
|---|---|
| `auto_fit_colorbar(padding)` | 행렬 값 컬럼, 산점도 `point_color_map` 컬럼, 벡터 색상 컬럼의 업로드 통계를 합쳐 공유 Z축 범위를 맞춘다. 색상 막대가 없으면 차트를 바꾸지 않는다. |
| `set_colorbar_axis(json)` | 기존 색상 막대의 전체 `AxisOptions`를 교체한다. 눈금 모양·방향·길이, 축 반전, 라벨 스타일·위치, 제목 옵션을 함께 설정한다. |
| `set_colorbar_title(text)` | 제목을 지정하고 표시한다. 빈 문자열이면 숨기며 `Config.colorbar`가 없으면 실패한다. |
| `set_contour_nice_levels(series_id, target_count, use_colormap_colors)` | 색상 막대 눈금 규칙으로 한 시리즈의 등고선 값을 만들고 필요하면 색상표의 색을 지정한다. 시리즈 설정에 반영한 뒤 레벨 수를 반환한다. |
| `series_draw_info(series_id)` | `{ drawn_count, cols, rows, truncated }`를 반환한다. 저수준 WASM은 JSON 문자열, 웹 래퍼는 객체를 반환한다. |
| `pick_data(x, y, max_distance_px)` | 점·구간·셀·등고선 레벨을 구분하는 식별자를 비동기로 반환한다. 저수준 WASM은 JSON 문자열 또는 `undefined`, 래퍼는 객체 또는 `null`이다. |
| `set_picked_points(json)` | `PickedPointsConfig` 또는 `null`을 담은 JSON 문자열로 `Config.picked_points`만 교체한다. `null`은 선택 표시를 지운다. 참조에는 좌표 대신 `series_id`, 선택적인 `source_id`, `point_index`를 저장한다. |
| `set_picked_data(json)` | `DataSelectionsConfig` 또는 `null`로 `Config.picked_data`만 교체한다. 참조는 출처와 인덱스만 보관하며 현재 도형은 차트의 GPU 자원에서 계산한다. |
| `set_clear_color(r, g, b, a)` | 선형 RGBA 각 성분을 0~1로 제한하고 다시 그리기를 요청한다. 화면 배경 상태이므로 Config JSON을 바꾸거나 축 이미지를 갱신하지 않는다. |

`FiggyChart::load_demo()`는 데모 상태 전체를 한 번에 교체한다. 컬럼 4개, 최종 `Config`와 시리즈 순서, 피킹 상태, 호스트 메타데이터, 유효한 범위 캐시를 함께 반영한다. 동기 준비 중 오류가 나면 기존 전체 상태를 유지한다. 트랜잭션 중에는 범위 계산 명령을 제출하지 않는다. 무효화된 캐시는 변경을 확정한 뒤 기존의 지연 생성·재시도 경로로 다시 만든다.

이 작업은 풀 용량과 같은 크기의 임시 GPU 버퍼 하나와 스테이징 버퍼 4개를 추가로 사용한다. 재배치용 백업이 남아 있으면 원본 풀·백업·임시 풀이 잠시 동시에 존재한다.

### 점선의 누적 경로 길이 계산 (GPU)

점선의 시작 위치와 간격을 맞추려면 각 점까지의 누적 경로 길이가 필요하다. 이 길이는 현재 데이터→픽셀 변환에 따라 달라진다. 계산 키가 달라지면 GPU에서 새로 계산하고, 키가 정확히 같으면 이전의 불변 결과를 재사용한다.

```
pool 컬럼 (x, y) ──┐                       Transform uniform (144 B write)
                   ▼                                   │
   seg_init        dst[i] = |px(pᵢ) − px(pᵢ₋₁)|   ◄────┘
   scan_block      256-블록 inclusive 스캔 (Hillis–Steele, 공유 메모리)
   scan_block/add  블록 합 레벨 (dst → sums0 → sums1 → sums2)
   carry 체인      min(디스패치 한계 × 256, 256³) 점 단위 청크를 순차 실행;
                   1-원소 carry 버퍼가 각 청크의 누계를 다음 청크에 전파 —
                   n의 상한은 u32 인덱스 범위(아래 참고)이며, 어떤 크기에서도 readback 없음
                   ▼
   누적 경로 길이 버퍼 ──► 라인 파이프라인 정점 슬롯 4/5 (dash 위상)
```

컴퓨트 명령은 호스트의 렌더 패스보다 먼저 GPU 큐에 제출한다. 따라서 지원하는 모든 통합 환경(winit·egui·웹)에서 API를 바꾸지 않아도 큐의 실행 순서에 따라 계산 결과가 준비된 뒤 그리게 된다.
계산 키에는 풀 배치 세대, X·Y 오프셋과 할당 세대, 길이, 좌표 변환의 앞 64바이트(축 범위·로그 플래그·픽셀과 패널 매핑), 선의 계단·극좌표 호 모드 값, 선택적인 별 배치 간격이 포함된다. 시리즈마다 최근 결과 8개를 보관하며, 없는 키는 새 버퍼에서 계산한다. 기존 결과는 덮어쓰지 않는다.
현재 누적 경로 길이 스캔은 `u32::MAX = 4,294,967,295`로 주소를 표현할 수 있는 범위에서 동작한다. 시리즈 길이나 풀 원소 오프셋이 `u32` 범위를 넘으면 점선용 누적 경로 계산을 생략한다. 새 시리즈 ID를 추가할 때 이미 256개 ID가 캐시에 있으면 무제한 증가를 막기 위해 경로 길이 캐시 전체를 비운다.

<a id="renderer-owned-상태와-frame-invalidation"></a>

### 렌더러 상태와 다시 그리기 판단

차트를 지속적으로 표시하는 호스트는 `Renderer`에 차트를 등록하고, GPU 제출과 화면 표시에 성공한 마지막 `ChartRenderStamp`를 보관한다. 검증된 리비전은 렌더러만 발급한다.

| 상태 | 다시 그리는 조건과 처리 |
|---|---|
| 렌더러의 `desired` 리비전 | 표시 상태 변경, 참조 컬럼 교체, 동기화된 폰트 등록 시 다시 그린다. |
| 렌더러의 `raster` 리비전 | 축 장식에 영향을 주는 설정 편집(차트 영역, 축, 제목, 그리드, 범례, 색상 막대, 그리기 스타일, `polar`, `overlays`·`subplots`의 모든 필드), 시리즈 변경, 폰트 등록이 있으면 그리기 전에 `refresh_axis`를 호출한다. 선택(`picked_*`), `active_layer`, `annotations` 편집과 동일한 설정 교체는 이 리비전을 바꾸지 않는다. 단, 레이어 변경이 요소 선택을 지우면 올린다. 분류는 `raster_config_equal`이며 [`src/renderer/ssot_invalidation_tests.rs`](src/renderer/ssot_invalidation_tests.rs)가 필드별로 검사한다. 앵커 공간이 축과 데이터 영역을 따르므로 `refresh_axis`는 주석 래스터도 함께 다시 만든다. 레이어가 있는 뷰는 내부 설정이 바뀐 패널만 다시 래스터한다. |
| 렌더러의 `annotation` 리비전 | `annotations.items` 편집이 있으면 그리기 전에 `refresh_annotations`를 호출한다. 주석 underlay·overlay 텍스처와 주석 히트 맵만 다시 쓰고 그리드·장식 텍스처는 건드리지 않는다(`ChartView::layer_stamps`가 층별 쓰기 횟수를 센다). `annotations.visible`을 바꾸면 이 리비전과 `raster` 리비전은 그대로이고, 그리기 단계에서 주석 텍스처 두 장의 합성만 생략한다. 분류는 `annotation_config_revision`이며 같은 테스트 파일의 `annotation_edits`가 검사한다. |
| 호스트의 `view_dirty` | 화면 크기·DPR로 미리보기 배치가 바뀌면 축 이미지를 갱신하고 다시 그린다. |
| 호스트의 `redraw_pending` | 배경색 등 화면 상태가 바뀌면 차트 설정을 복제하지 않고 다시 그린다. |

브라우저의 프레임 처리 순서는 다음과 같다. [`crates/web/src/lib.rs`](https://github.com/shim9610/figgy/blob/master/crates/web/src/lib.rs)의 `FiggyChart::frame`을 줄인 예시다.

```rust
renderer.sync_external_invalidations()?;
let stamp = renderer.chart_render_stamp(chart_id)?;
let draw = stamp.needs_draw_since(last_presented_stamp.as_ref());
let raster = stamp.needs_raster_since(last_presented_stamp.as_ref());
let annotations = stamp.needs_annotation_raster_since(last_presented_stamp.as_ref());

if !draw && !raster && !view_dirty && !redraw_pending {
    if renderer.has_pending_maintenance() {
        renderer.process_pending_maintenance()?;
    }
    return Ok(());
}
renderer.process_pending_maintenance()?;
let stamp = renderer.chart_render_stamp(chart_id)?;
if raster || view_dirty {
    // 주석 래스터도 함께 다시 만든다.
    renderer.refresh_axis_with_selection(&mut view, &display_chart, rect, &boxes)?;
} else if annotations {
    renderer.refresh_annotations(&mut view, &display_chart, rect, selected_annotation)?;
}
if renderer.is_streaming_chart(chart_id) {
    renderer.draw_registered(clear, &[RegisteredChartDrawItem { chart_id, view: &view }])?;
} else {
    renderer.draw(clear, &items)?;
}
last_presented_stamp = Some(stamp);
view_dirty = false;
redraw_pending = false;
```

마지막 표시 기록과 호스트 플래그는 그리기가 성공한 뒤에만 갱신한다. 실패하면 다음 프레임에서 다시 시도한다. 변경이 없는 웹 rAF에서는 GPU 화면 출력 경로 전체를 생략한다. 상주 경로는 다시 그릴 때 GPU 풀의 원본 도형에서 명령을 만들며 데이터 이미지 캐시를 두지 않는다. 비상주 경로는 부분 표시를 위한 GPU 누적 이미지를 유지하고, 입력이 바뀌지 않은 완료 결과를 재사용한다.

`Chart::{data_dirty,raster_dirty}`는 `Chart`를 직접 소유하는 저수준 호스트와의 호환성을 위해 남겨 둔 플래그다. `prepare`는 이 값을 읽거나 초기화하지 않고, 호출될 때 좌표 변환을 기록한다. 외부에서 `Chart`를 관리한다면 호스트가 `raster_dirty`를 확인·해제하고 `refresh_axis`를 호출해야 한다.

<a id="log-scale-gpu-처리"></a>

### GPU의 로그축 처리

`AxisOptions.scale = Logarithmic`이면 다음 규칙을 적용한다.

- 자동 범위 맞춤은 0·음수가 섞여 있어도 저장된 최소 양수 값을 하한으로 사용한다.
- 수동으로 지정한 경계가 0 이하이거나 유효하지 않으면 `1e-12`로 보정한다. 유효한 양수는 `1e-12`보다 작아도 유지한다.
- CPU의 `scatter_transform_from_config`가 보정된 범위를 log10으로 변환하고 해당 축의 `scale_log`를 설정한다.
- GPU의 선·산점도 경로는 그리기와 피킹 전에 좌표가 유한하고 로그축에서 양수인지 확인한다. 유효하지 않은 점과 그 점에 닿는 선분은 생략해 빈 구간으로 남기며, 나머지 데이터를 거부하거나 빈 구간을 선으로 잇지 않는다. 정밀·스케치·리본/호·별 선 경로에도 같은 규칙을 적용한다. 히스토그램의 0 기준선은 양수 표시 범위로 잘라 그리며, 로그 색상표의 0 이하 Z값은 `nan_color`를 사용한다.

<a id="export-파이프라인"></a>

### 이미지 출력 과정

![출력 해상도의 오프스크린 렌더링, 상주 또는 스트리밍 원본 입력, 행 청크 읽기와 PNG 인코딩](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/png-export-kr.png)

```
export_panel_rgba_async(chart, &[SeriesConfig], scale).await:
    scale ← clamp_export_scale(scale)         // [MIN_EXPORT_SCALE, MAX_EXPORT_SCALE]
    chart.config().scaled(scale)               // 모든 픽셀 치수를 같은 비율로 확대
        ↓
    임시 ChartView (확대한 축 텍스처)
    임시 ChartStyle ← cfg마다 create_style_for_series_scaled(cfg, scale)
        ↓
    offscreen wgpu::Texture (고정 Rgba8Unorm, COPY_SRC, transparent clear)
    paint(items) — 동일 합성 순서 (grid → annotation underlay → data → decoration → annotation overlay)
        ↓
    copy_texture_to_buffer를 행 단위 청크로 실행 (256바이트 정렬 패딩. 청크
    높이는 장치의 최대 버퍼 크기에 맞춰 조절하므로 읽기 버퍼가 이미지 전체
    크기일 필요가 없다. 렌더 타깃은 장치의 텍스처 크기 한도 안이어야 한다)
        ↓
    map_async (네이티브는 Wait로 바로 폴링, wasm은 브라우저에 양보하며 await)
        ↓
    premul→straight α 변환, 패딩 행 제거 (대상이 이미 RGBA이므로 채널 순서는
    바꾸지 않음)
        ↓
    RasterImage { width, height, rgba: Vec<u8> }   ← API 반환
    (export_document_rgba_async: 레이어마다 한 패널을 같은 텍스처에 순서대로 합성.
     export_panel_*는 기본 패널만 그리며, 오버레이가 없으면 두 결과는 바이트 단위로 같다.
     서브플롯이 있으면 문서 출력은 모든 차트 영역의 합집합 크기다)
        ↓
    encode_png(&img) → Vec<u8>                      ← PNG 바이트
        ↓
    호출자가 std::fs::write / clipboard / 네트워크 등 자유 처리
```

---
